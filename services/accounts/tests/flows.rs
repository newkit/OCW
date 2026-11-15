//! The sign-in flows end to end over HTTP on the loopback interface (spec
//! §9.1, §11.4): an app (forms from `ocw_auth::client`), the backend and
//! the fake Google (OIDC) and Strava (OAuth 2) servers, all in-process;
//! [`TestBrowser`] plays the user. No network beyond 127.0.0.1.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};

use ocw_accounts::config::Config;
use ocw_accounts::fake::{DevBackend, FakeGoogle, FakeStrava, TestBrowser};
use ocw_accounts::{AccountHooks, AuthService};
use ocw_auth::client::{self, Callback, PendingLogin, Poll, Session, TokenError};
use ocw_auth::oidc::{DeviceAuthorization, Discovery, LinkTicket, TokenResponse, UserInfo};
use ocw_auth::{CLIENT_DESKTOP, CLIENT_KIOSK, CLIENT_WEB, Jwks, Provider, Verifier, VerifyError};

const APP: &str = "http://127.0.0.1:5999/callback";

struct Env {
    _rt: tokio::runtime::Runtime,
    svc: Arc<AuthService>,
    issuer: String,
    google: FakeGoogle,
    strava: FakeStrava,
    clock: Arc<AtomicI64>,
    browser: TestBrowser,
}

fn real_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn env_with(tune: impl FnOnce(&mut Config)) -> Env {
    let rt = tokio::runtime::Runtime::new().unwrap();
    // The backend's clock starts at a day's first hour (Strava's 15-minute
    // and daily windows are stable through a test); the fakes run on the
    // real clock (their ID tokens are checked against it).
    let clock = Arc::new(AtomicI64::new(real_now() / 86_400 * 86_400 + 3600));
    let c = clock.clone();
    let (svc, issuer, google, strava) = rt.block_on(async move {
        let google = FakeGoogle::start().await.unwrap();
        let strava = FakeStrava::start().await.unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let issuer = format!("http://{}", listener.local_addr().unwrap());
        let mut cfg = Config::dev(&issuer);
        cfg.google = Some(google.config());
        cfg.strava = Some(strava.config());
        let mut clients = Config::default_clients();
        clients
            .iter_mut()
            .find(|c| c.id == CLIENT_WEB)
            .unwrap()
            .redirect_uris
            .push("https://app.example/ride/".into());
        cfg.clients = clients;
        cfg.rate_limit.per_minute = 1000;
        tune(&mut cfg);
        let svc = Arc::new(
            AuthService::with_clock(cfg, Arc::new(move || c.load(Ordering::SeqCst))).unwrap(),
        );
        let s = svc.clone();
        tokio::spawn(async move { ocw_accounts::serve(s, listener).await });
        (svc, issuer, google, strava)
    });
    Env {
        _rt: rt,
        svc,
        issuer,
        google,
        strava,
        clock,
        browser: TestBrowser::new(),
    }
}

fn env() -> Env {
    env_with(|_| {})
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .build()
        .into()
}

fn post(url: &str, body: &str, bearer: Option<&str>) -> (u16, String) {
    let mut req = agent()
        .post(url)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(t) = bearer {
        req = req.header("authorization", &format!("Bearer {t}"));
    }
    let mut r = req.send(body).unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
}

fn get(url: &str, bearer: Option<&str>) -> (u16, String) {
    let mut req = agent().get(url);
    if let Some(t) = bearer {
        req = req.header("authorization", &format!("Bearer {t}"));
    }
    let mut r = req.call().unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
}

fn delete(url: &str, bearer: &str) -> (u16, String) {
    let mut r = agent()
        .delete(url)
        .header("authorization", &format!("Bearer {bearer}"))
        .call()
        .unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
}

impl Env {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.issuer)
    }

    fn advance(&self, s: i64) {
        self.clock.fetch_add(s, Ordering::SeqCst);
    }

    fn now(&self) -> i64 {
        self.clock.load(Ordering::SeqCst)
    }

    fn pending(&self, provider: Option<Provider>) -> PendingLogin {
        let mut r = [0u8; 96];
        getrandom::fill(&mut r).unwrap();
        PendingLogin::new(&r, APP, provider)
    }

    /// Runs the browser part of a sign-in: the redirect back to the app.
    fn browse(&self, p: &PendingLogin, client_id: &str) -> String {
        let url = p.authorize_url(&self.url("/authorize"), client_id);
        self.browser.open_until(&url, APP).unwrap()
    }

    fn code_of(&self, p: &PendingLogin, redirect: &str) -> Callback {
        let target = redirect.strip_prefix("http://127.0.0.1:5999").unwrap();
        client::parse_callback(target, client::LOOPBACK_PATH, p, &self.issuer)
    }

    fn exchange(&self, p: &PendingLogin, code: &str) -> Result<TokenResponse, TokenError> {
        let (s, b) = post(
            &self.url("/token"),
            &p.code_form(CLIENT_DESKTOP, code),
            None,
        );
        client::parse_token_response(s, &b)
    }

    /// A full desktop sign-in with `provider`.
    fn sign_in(&self, provider: Provider) -> Session {
        let p = self.pending(Some(provider));
        let redirect = self.browse(&p, CLIENT_DESKTOP);
        let Callback::Code(code) = self.code_of(&p, &redirect) else {
            panic!("no code: {redirect}");
        };
        let t = self.exchange(&p, &code).unwrap();
        Session::from_response(
            &t,
            self.now(),
            &self.issuer,
            CLIENT_DESKTOP,
            Some(&p.nonce),
            None,
        )
        .unwrap()
    }

    fn refresh(&self, s: &Session) -> Result<Session, TokenError> {
        let rt = s.refresh_token.as_deref().unwrap();
        let (st, b) = post(
            &self.url("/token"),
            &client::refresh_form(CLIENT_DESKTOP, rt),
            None,
        );
        let t = client::parse_token_response(st, &b)?;
        Ok(
            Session::from_response(&t, self.now(), &self.issuer, CLIENT_DESKTOP, None, Some(s))
                .unwrap(),
        )
    }

    fn userinfo(&self, s: &Session) -> UserInfo {
        let (st, b) = get(&self.url("/userinfo"), Some(&s.access_token));
        assert_eq!(st, 200, "{b}");
        serde_json::from_str(&b).unwrap()
    }

    fn verifier(&self) -> Verifier {
        let (_, jwks) = get(&self.url("/jwks"), None);
        Verifier::new(
            &self.issuer,
            ocw_auth::DEFAULT_AUDIENCE,
            serde_json::from_str::<Jwks>(&jwks).unwrap(),
        )
    }
}

#[test]
fn discovery_is_oidc_shaped() {
    let e = env();
    let (s, b) = get(&e.url("/.well-known/openid-configuration"), None);
    assert_eq!(s, 200);
    let d: Discovery = serde_json::from_str(&b).unwrap();
    assert_eq!(d.issuer, e.issuer);
    assert_eq!(d.token_endpoint, e.url("/token"));
    assert_eq!(
        d.device_authorization_endpoint,
        e.url("/device_authorization")
    );
    assert_eq!(d.code_challenge_methods_supported, ["S256"]);
    assert_eq!(d.ocw_providers, [Provider::Google, Provider::Strava]);
    assert!(d.authorization_response_iss_parameter_supported);
}

#[test]
fn google_sign_in_with_pkce_state_and_nonce() {
    let e = env();
    let s = e.sign_in(Provider::Google);
    assert_eq!(s.name, "Marianne Vos");
    assert_eq!(s.providers, ["google"]);
    // The access token is good for the game server (signature, issuer,
    // audience) and names the account.
    let claims = e.verifier().verify(&s.access_token, e.now()).unwrap();
    assert_eq!(claims.sub, s.account_id);
    assert_eq!(claims.client_id, CLIENT_DESKTOP);
    let u = e.userinfo(&s);
    assert_eq!(u.email.as_deref(), Some("marianne@example.org"));
    // The same Google account signs in to the same OCW account.
    let again = e.sign_in(Provider::Google);
    assert_eq!(again.account_id, s.account_id);
    // Another Google user: another account.
    e.google
        .set_user("999", "Annemiek van Vleuten", "a@example.org");
    let other = e.sign_in(Provider::Google);
    assert_ne!(other.account_id, s.account_id);
    // The provider choice page when the app names none.
    let p = e.pending(None);
    let page = e
        .browser
        .open(&p.authorize_url(&e.url("/authorize"), CLIENT_DESKTOP))
        .unwrap();
    assert_eq!(page.status, 200);
    assert!(page.body.contains("provider=google") && page.body.contains("provider=strava"));
}

#[test]
fn state_and_nonce_mismatches_are_refused() {
    let e = env();
    // The app ignores a redirect with another state (CSRF).
    let p = e.pending(Some(Provider::Google));
    let redirect = e.browse(&p, CLIENT_DESKTOP);
    let other = e.pending(Some(Provider::Google));
    assert_eq!(e.code_of(&other, &redirect), Callback::Ignore);
    // The backend refuses a callback with an unknown state.
    let page = e
        .browser
        .open(&e.url("/callback/google?code=x&state=forged"))
        .unwrap();
    assert!(
        page.body.contains("expired or was already used"),
        "{}",
        page.body
    );
    // A replayed ID token (another nonce) is refused: error to the app.
    e.google.wrong_nonce(true);
    let p = e.pending(Some(Provider::Google));
    let redirect = e.browse(&p, CLIENT_DESKTOP);
    match e.code_of(&p, &redirect) {
        Callback::Denied(d) => assert!(d.contains("refused"), "{d}"),
        c => panic!("{c:?}"),
    }
    e.google.wrong_nonce(false);
    // The user cancels at Google.
    e.google.deny(true);
    let p = e.pending(Some(Provider::Google));
    assert!(matches!(
        e.code_of(&p, &e.browse(&p, CLIENT_DESKTOP)),
        Callback::Denied(_)
    ));
}

#[test]
fn authorization_requests_are_checked() {
    let e = env();
    let p = e.pending(Some(Provider::Google));
    let url = p.authorize_url(&e.url("/authorize"), CLIENT_DESKTOP);
    // An unregistered redirect URI: an error page, never a redirect.
    for (from, to) in [
        ("127.0.0.1%3A5999", "evil.example%3A5999"),
        ("http%3A%2F%2F127.0.0.1", "https%3A%2F%2F127.0.0.1"),
        ("%2Fcallback", "%2Fother"),
    ] {
        let bad = url.replace(from, to);
        assert_ne!(bad, url);
        let page = e.browser.open(&bad).unwrap();
        assert_eq!(page.status, 400, "{bad}");
        assert!(page.url.starts_with(&e.issuer));
    }
    // The browser app: only its exact page.
    let mut web = e.pending(Some(Provider::Google));
    web.redirect_uri = "https://app.example/ride/".into();
    let ok = e.browser.open_until(
        &web.authorize_url(&e.url("/authorize"), CLIENT_WEB),
        "https://app.example/ride/",
    );
    assert!(ok.unwrap().contains("code="));
    web.redirect_uri = "https://app.example/ride/?x=1".into();
    let page = e
        .browser
        .open(&web.authorize_url(&e.url("/authorize"), CLIENT_WEB))
        .unwrap();
    assert_eq!(page.status, 400);
    // No PKCE or plain PKCE: back to the app with an error.
    for (from, to) in [("S256", "plain"), ("code_challenge_method=S256&", "")] {
        let r = e.browser.open_until(&url.replace(from, to), APP).unwrap();
        assert!(r.contains("error=invalid_request"), "{r}");
    }
    // Repeated parameters.
    let page = e.browser.open(&format!("{url}&state=again")).unwrap();
    assert_eq!(page.status, 400);
    // Unknown client.
    let page = e
        .browser
        .open(&p.authorize_url(&e.url("/authorize"), "evil-app"))
        .unwrap();
    assert_eq!(page.status, 400);
}

#[test]
fn codes_are_single_use_and_bound_to_the_verifier() {
    let e = env();
    let p = e.pending(Some(Provider::Google));
    let Callback::Code(code) = e.code_of(&p, &e.browse(&p, CLIENT_DESKTOP)) else {
        panic!()
    };
    // Wrong verifier: refused (and the code is gone).
    let mut wrong = p.clone();
    wrong.verifier = ocw_auth::pkce::verifier(&[1; 32]);
    assert!(matches!(
        e.exchange(&wrong, &code),
        Err(TokenError::InvalidGrant(_))
    ));
    assert!(matches!(
        e.exchange(&p, &code),
        Err(TokenError::InvalidGrant(_))
    ));
    // A good exchange, then a replay of the code revokes its tokens.
    let p = e.pending(Some(Provider::Google));
    let Callback::Code(code) = e.code_of(&p, &e.browse(&p, CLIENT_DESKTOP)) else {
        panic!()
    };
    let t = e.exchange(&p, &code).unwrap();
    let s = Session::from_response(&t, e.now(), &e.issuer, CLIENT_DESKTOP, Some(&p.nonce), None)
        .unwrap();
    assert!(matches!(
        e.exchange(&p, &code),
        Err(TokenError::InvalidGrant(_))
    ));
    assert!(
        matches!(e.refresh(&s), Err(TokenError::InvalidGrant(_))),
        "revoked by the replay"
    );
    // A code expires after 60 s.
    let p = e.pending(Some(Provider::Google));
    let Callback::Code(code) = e.code_of(&p, &e.browse(&p, CLIENT_DESKTOP)) else {
        panic!()
    };
    e.advance(61);
    assert!(matches!(
        e.exchange(&p, &code),
        Err(TokenError::InvalidGrant(_))
    ));
}

#[test]
fn refresh_rotation_reuse_detection_and_revocation() {
    let e = env();
    let s1 = e.sign_in(Provider::Google);
    e.advance(30);
    let s2 = e.refresh(&s1).unwrap();
    assert_ne!(s2.refresh_token, s1.refresh_token, "rotated");
    assert_ne!(s2.access_token, s1.access_token);
    assert_eq!(s2.account_id, s1.account_id);
    let s3 = e.refresh(&s2).unwrap();
    // Reusing the rotated s1 token: the whole family dies (s3 too).
    assert!(matches!(e.refresh(&s1), Err(TokenError::InvalidGrant(_))));
    assert!(matches!(e.refresh(&s3), Err(TokenError::InvalidGrant(_))));
    // Revocation ends a sign-in; another one stays.
    let a = e.sign_in(Provider::Google);
    let b = e.sign_in(Provider::Google);
    let (st, _) = post(
        &e.url("/revoke"),
        &client::revoke_form(CLIENT_DESKTOP, a.refresh_token.as_deref().unwrap()),
        None,
    );
    assert_eq!(st, 200);
    assert!(matches!(e.refresh(&a), Err(TokenError::InvalidGrant(_))));
    assert!(e.refresh(&b).is_ok());
    // Unknown tokens revoke quietly (no oracle).
    let (st, _) = post(
        &e.url("/revoke"),
        &client::revoke_form(CLIENT_DESKTOP, "nothing"),
        None,
    );
    assert_eq!(st, 200);
    // Idle timeout (30 days).
    let c = e.sign_in(Provider::Google);
    e.advance(31 * 86_400);
    assert!(matches!(e.refresh(&c), Err(TokenError::InvalidGrant(_))));
    // Another client cannot use the token.
    let d = e.sign_in(Provider::Google);
    let (st, b) = post(
        &e.url("/token"),
        &client::refresh_form(CLIENT_WEB, d.refresh_token.as_deref().unwrap()),
        None,
    );
    assert!(matches!(
        client::parse_token_response(st, &b),
        Err(TokenError::InvalidGrant(_))
    ));
}

#[test]
fn strava_sign_in_keeps_the_athlete_and_encrypted_tokens() {
    let e = env();
    let s = e.sign_in(Provider::Strava);
    assert_eq!(s.name, "Eddy Merckx");
    let u = e.userinfo(&s);
    assert_eq!(u.email, None, "Strava gives no e-mail");
    assert!(u.providers[0].upload, "activity:write granted");
    let t = e
        .svc
        .provider_tokens(&s.account_id, Provider::Strava)
        .unwrap();
    assert!(t.scope.contains(&"activity:write".to_owned()));
    // The optional consent: the athlete unticks the upload.
    e.strava.grant_upload(false);
    e.strava.set_athlete(77, "Lizzie", "Deignan");
    let s = e.sign_in(Provider::Strava);
    assert!(!e.userinfo(&s).providers[0].upload);
}

#[test]
fn strava_rate_limits_are_respected() {
    let e = env();
    e.sign_in(Provider::Strava);
    let before = e.strava.requests();
    // Strava reports its 15-minute limit as used up.
    e.strava.set_usage(200, 300);
    e.strava.set_athlete(5, "A", "B");
    e.sign_in(Provider::Strava);
    let p = e.pending(Some(Provider::Strava));
    match e.code_of(&p, &e.browse(&p, CLIENT_DESKTOP)) {
        Callback::Denied(d) => assert!(d.contains("rate limit"), "{d}"),
        c => panic!("{c:?}"),
    }
    assert_eq!(
        e.strava.requests(),
        before + 1,
        "no request beyond the limit"
    );
    // The next window.
    e.advance(900);
    e.strava.set_usage(0, 300);
    e.sign_in(Provider::Strava);
}

#[test]
fn linking_and_unlinking_providers() {
    let e = env();
    let s = e.sign_in(Provider::Google);
    // Link Strava: a ticket bound to the next login's PKCE challenge.
    let mut p = e.pending(Some(Provider::Strava));
    let (st, b) = post(
        &e.url("/account/link"),
        &client::form(&[("code_challenge", &p.challenge())]),
        Some(&s.access_token),
    );
    assert_eq!(st, 200, "{b}");
    let ticket: LinkTicket = serde_json::from_str(&b).unwrap();
    p.link_ticket = Some(ticket.link_ticket.clone());
    let Callback::Code(code) = e.code_of(&p, &e.browse(&p, CLIENT_DESKTOP)) else {
        panic!()
    };
    let t = e.exchange(&p, &code).unwrap();
    let linked =
        Session::from_response(&t, e.now(), &e.issuer, CLIENT_DESKTOP, Some(&p.nonce), None)
            .unwrap();
    assert_eq!(linked.account_id, s.account_id);
    assert_eq!(linked.providers, ["google", "strava"]);
    // Signing in with Strava now reaches the same account.
    assert_eq!(e.sign_in(Provider::Strava).account_id, s.account_id);
    // The ticket was single-use.
    let mut again = e.pending(Some(Provider::Strava));
    again.link_ticket = Some(ticket.link_ticket);
    assert!(matches!(
        e.code_of(&again, &e.browse(&again, CLIENT_DESKTOP)),
        Callback::Denied(_)
    ));
    // A ticket used with another PKCE challenge is refused.
    let p2 = e.pending(Some(Provider::Strava));
    let (_, b) = post(
        &e.url("/account/link"),
        &client::form(&[("code_challenge", &p2.challenge())]),
        Some(&s.access_token),
    );
    let ticket: LinkTicket = serde_json::from_str(&b).unwrap();
    let mut thief = e.pending(Some(Provider::Strava));
    thief.link_ticket = Some(ticket.link_ticket);
    assert!(matches!(
        e.code_of(&thief, &e.browse(&thief, CLIENT_DESKTOP)),
        Callback::Denied(_)
    ));
    // A Strava athlete of another account cannot be linked here.
    e.strava.set_athlete(9001, "Other", "Rider");
    let other = e.sign_in(Provider::Strava);
    assert_ne!(other.account_id, s.account_id);
    e.google.set_user("g-other", "G Other", "o@example.org");
    let g2 = e.sign_in(Provider::Google);
    let mut p = e.pending(Some(Provider::Strava));
    let (_, b) = post(
        &e.url("/account/link"),
        &client::form(&[("code_challenge", &p.challenge())]),
        Some(&g2.access_token),
    );
    p.link_ticket = Some(serde_json::from_str::<LinkTicket>(&b).unwrap().link_ticket);
    match e.code_of(&p, &e.browse(&p, CLIENT_DESKTOP)) {
        Callback::Denied(d) => assert!(d.contains("another OCW account"), "{d}"),
        c => panic!("{c:?}"),
    }
    // Unlink: never the last sign-in.
    let (st, b) = delete(&e.url("/account/providers/google"), &linked.access_token);
    assert_eq!(st, 200, "{b}");
    let u: UserInfo = serde_json::from_str(&b).unwrap();
    assert_eq!(u.providers.len(), 1);
    let (st, b) = delete(&e.url("/account/providers/strava"), &linked.access_token);
    assert_eq!(st, 409, "{b}");
    assert!(
        e.svc
            .provider_tokens(&s.account_id, Provider::Strava)
            .is_some()
    );
}

#[test]
fn device_flow_pending_slow_down_approved_denied_expired() {
    let e = env();
    let (st, b) = post(
        &e.url("/device_authorization"),
        &client::device_start_form(CLIENT_KIOSK),
        None,
    );
    assert_eq!(st, 200, "{b}");
    let d: DeviceAuthorization = serde_json::from_str(&b).unwrap();
    assert_eq!(d.verification_uri, e.url("/device"));
    assert_eq!(d.user_code.len(), 9);
    let poll = || {
        let (st, b) = post(
            &e.url("/token"),
            &client::device_poll_form(CLIENT_KIOSK, &d.device_code),
            None,
        );
        client::parse_poll(st, &b)
    };
    assert_eq!(poll(), Poll::Pending);
    assert_eq!(poll(), Poll::SlowDown, "polled at once again");
    e.advance(11);
    assert_eq!(poll(), Poll::Pending, "after the longer interval");
    // The phone: the code page (lower case, without the dash works too).
    let typed = d.user_code.replace('-', "").to_lowercase();
    let page = e
        .browser
        .open(&format!("{}?user_code={typed}", d.verification_uri))
        .unwrap();
    assert!(page.body.contains(&d.user_code), "{}", page.body);
    assert!(page.body.contains("Open Cycling World kiosk"));
    let csrf = TestBrowser::field(&page.body, "csrf").unwrap();
    // Without the CSRF token (or with another one): refused.
    let confirm = e.url("/device/confirm");
    let bad = TestBrowser::new()
        .submit(
            &confirm,
            &[
                ("csrf", &csrf),
                ("user_code", &d.user_code),
                ("action", "google"),
            ],
        )
        .unwrap();
    assert_eq!(bad.status, 403);
    let bad = e
        .browser
        .submit(
            &confirm,
            &[
                ("csrf", "forged"),
                ("user_code", &d.user_code),
                ("action", "google"),
            ],
        )
        .unwrap();
    assert_eq!(bad.status, 403);
    // Confirmed with Strava.
    let done = e
        .browser
        .submit(
            &confirm,
            &[
                ("csrf", &csrf),
                ("user_code", &d.user_code),
                ("action", "strava"),
            ],
        )
        .unwrap();
    assert!(
        done.body.contains("Signed in as Eddy Merckx"),
        "{}",
        done.body
    );
    e.advance(16);
    let Poll::Tokens(t) = poll() else { panic!() };
    let s = Session::from_response(&t, e.now(), &e.issuer, CLIENT_KIOSK, None, None).unwrap();
    assert_eq!(s.name, "Eddy Merckx");
    assert!(e.verifier().verify(&s.access_token, e.now()).is_ok());
    // Used up.
    e.advance(30);
    assert_eq!(poll(), Poll::Expired);

    // Denied on the phone.
    let (_, b) = post(
        &e.url("/device_authorization"),
        &client::device_start_form(CLIENT_KIOSK),
        None,
    );
    let d2: DeviceAuthorization = serde_json::from_str(&b).unwrap();
    let page = e
        .browser
        .open(d2.verification_uri_complete.as_deref().unwrap())
        .unwrap();
    let csrf = TestBrowser::field(&page.body, "csrf").unwrap();
    e.browser
        .submit(
            &confirm,
            &[
                ("csrf", &csrf),
                ("user_code", &d2.user_code),
                ("action", "deny"),
            ],
        )
        .unwrap();
    let (st, b) = post(
        &e.url("/token"),
        &client::device_poll_form(CLIENT_KIOSK, &d2.device_code),
        None,
    );
    assert_eq!(client::parse_poll(st, &b), Poll::Denied);

    // Expired after ten minutes.
    let (_, b) = post(
        &e.url("/device_authorization"),
        &client::device_start_form(CLIENT_KIOSK),
        None,
    );
    let d3: DeviceAuthorization = serde_json::from_str(&b).unwrap();
    e.advance(601);
    let (st, b) = post(
        &e.url("/token"),
        &client::device_poll_form(CLIENT_KIOSK, &d3.device_code),
        None,
    );
    assert_eq!(client::parse_poll(st, &b), Poll::Expired);
    let page = e
        .browser
        .open(d3.verification_uri_complete.as_deref().unwrap())
        .unwrap();
    assert!(page.body.contains("unknown or has expired"));

    // The desktop app may not use the device flow.
    let (st, _) = post(
        &e.url("/device_authorization"),
        &client::device_start_form(CLIENT_DESKTOP),
        None,
    );
    assert_eq!(st, 400);
}

#[test]
fn jwks_rotation_keeps_old_tokens_valid_until_they_expire() {
    let e = env();
    let s = e.sign_in(Provider::Google);
    let old_kid = e.svc.jwks().keys[0].kid.clone();
    e.svc.rotate_keys().unwrap();
    let v = e.verifier();
    assert_eq!(v.keys().keys.len(), 2, "old and new key published");
    assert!(
        v.verify(&s.access_token, e.now()).is_ok(),
        "old token still good"
    );
    let s2 = e.refresh(&s).unwrap();
    assert!(v.verify(&s2.access_token, e.now()).is_ok());
    let p = ocw_auth::jwt::Parsed::parse(&s2.access_token).unwrap();
    assert_ne!(p.header.kid, old_kid, "signed with the new key");
    // A verifier with the old set only does not know the new key: it
    // refreshes its keys (the game server does this on UnknownKey).
    let stale = Verifier::new(
        &e.issuer,
        ocw_auth::DEFAULT_AUDIENCE,
        Jwks {
            keys: vec![v.keys().keys[0].clone()],
        },
    );
    let err = stale.verify(&s2.access_token, e.now()).unwrap_err();
    assert_eq!(err, VerifyError::UnknownKey);
    assert!(err.refresh_keys());
    // Past the grace time, a later rotation drops the old key.
    e.advance(2000);
    e.svc.rotate_keys().unwrap();
    let v = e.verifier();
    assert_eq!(v.keys().keys.len(), 2);
    assert!(v.keys().keys.iter().all(|k| k.kid != old_kid));
    // Expired tokens fail.
    assert_eq!(
        v.verify(&s2.access_token, e.now()),
        Err(VerifyError::Expired)
    );
}

struct Training;

impl AccountHooks for Training {
    fn name(&self) -> &str {
        "training"
    }
    fn export(&self, account: &str) -> serde_json::Value {
        serde_json::json!({ "rides": 3, "account": account })
    }
    fn delete(&self, _account: &str) {}
}

#[test]
fn export_and_deletion() {
    let e = env();
    e.svc.add_hooks(Arc::new(Training));
    let s = e.sign_in(Provider::Strava);
    let (st, b) = get(&e.url("/account/export"), Some(&s.access_token));
    assert_eq!(st, 200);
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert_eq!(v["account"]["display_name"], "Eddy Merckx");
    assert_eq!(v["modules"]["training"]["rides"], 3);
    assert!(
        !b.contains(&s.access_token) && !b.contains("refresh"),
        "no tokens in the export"
    );
    // Deletion wants a recent sign-in.
    e.advance(1000);
    let fresh = e.refresh(&s).unwrap();
    let (st, b) = delete(&e.url("/account"), &fresh.access_token);
    assert_eq!(st, 403, "{b}");
    let s = e.sign_in(Provider::Strava);
    let (st, _) = delete(&e.url("/account"), &s.access_token);
    assert_eq!(st, 204);
    assert!(
        e.svc
            .provider_tokens(&s.account_id, Provider::Strava)
            .is_none()
    );
    assert!(matches!(e.refresh(&s), Err(TokenError::InvalidGrant(_))));
    let (st, _) = get(&e.url("/userinfo"), Some(&s.access_token));
    assert_eq!(st, 401);
    // Signing in again makes a new account.
    assert_ne!(e.sign_in(Provider::Strava).account_id, s.account_id);
}

#[test]
fn token_endpoint_is_rate_limited() {
    let e = env_with(|c| c.rate_limit.per_minute = 5);
    let mut last = 0;
    for _ in 0..6 {
        last = post(
            &e.url("/token"),
            &client::refresh_form(CLIENT_DESKTOP, "x"),
            None,
        )
        .0;
    }
    assert_eq!(last, 429);
    e.advance(61);
    assert_eq!(
        post(
            &e.url("/token"),
            &client::refresh_form(CLIENT_DESKTOP, "x"),
            None
        )
        .0,
        400
    );
}

#[test]
fn bearer_checks() {
    let e = env();
    assert_eq!(get(&e.url("/userinfo"), None).0, 401);
    assert_eq!(get(&e.url("/userinfo"), Some("not-a-token")).0, 401);
    let s = e.sign_in(Provider::Google);
    e.advance(700);
    assert_eq!(
        get(&e.url("/userinfo"), Some(&s.access_token)).0,
        401,
        "expired"
    );
}

#[test]
fn dev_backend_runs_both_fakes() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let dev = rt
        .block_on(DevBackend::start(
            "127.0.0.1:0",
            Some(("http://127.0.0.1:1/".into(), "http://127.0.0.1:1".into())),
        ))
        .unwrap();
    let (s, b) = get(
        &format!("{}/.well-known/openid-configuration", dev.issuer),
        None,
    );
    assert_eq!(s, 200);
    assert!(b.contains("strava"));
    let _ = (&dev.google, &dev.strava, &dev.service);
}

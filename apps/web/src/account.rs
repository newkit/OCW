//! Sign-in in the browser (spec §9.1): the same Settings → Account page
//! and `account` command as the desktop, with the browser's redirect flow
//! (authorization code + PKCE S256, `state`, `nonce`, the `iss` check of
//! RFC 9207).
//!
//! - **Sign in**: the page keeps the pending login (state, nonce, PKCE
//!   verifier) and its own query in `sessionStorage` (this tab only) and
//!   goes to the backend; the backend's redirect brings the page back
//!   with `?code=…&state=…`. At the start the page takes its original
//!   query again (the ride's options), puts the original URL back into
//!   the address bar (the code never stays in the history) and exchanges
//!   the code with `fetch()` (CORS: the backend allows the page's origin).
//! - **Tokens** (decided here, a proposal in spec §9.1): the access token
//!   lives in memory (and for remote rides in `sessionStorage`: this tab,
//!   ten minutes); the **refresh token in `localStorage`**, so a signed-in
//!   rider stays signed in across visits. The backend rotates it on every
//!   use and revokes the whole sign-in when an old one comes back (reuse
//!   detection), its idle lifetime is 30 days. A secure HttpOnly cookie
//!   would hide it from scripts, but needs the backend on the page's own
//!   site (B8 hosting is open); the page loads no third-party scripts.
//! - The page option `auth=URL` names the accounts server (none: sign-in
//!   is off, everyone rides anonymously, Phase 10 Q5).

use bevy::prelude::*;
use ocw_auth::Provider;
use ocw_auth::client::{self, Callback, PendingLogin, Session};
use ocw_client::account::{AccountInfo, AccountState};
use serde::{Deserialize, Serialize};

/// `sessionStorage`: the sign-in on its way through the backend.
pub const PENDING_KEY: &str = "ocw.account.pending";
/// `localStorage`: the session without its access token.
pub const SESSION_KEY: &str = "ocw.account.session";
/// `sessionStorage`: the access token and its expiry (remote rides).
pub const ACCESS_KEY: &str = "ocw.account.access";

/// A sign-in the page started (kept across the redirect).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedLogin {
    pub issuer: String,
    pub login: PendingLogin,
    /// The page's query before it left (the ride's options).
    pub query: String,
    pub linking: bool,
}

/// What the page start found (`lib.rs` → the plugin).
#[derive(Resource, Debug, Clone, Default)]
pub struct AccountStart {
    /// The accounts server (`auth=`).
    pub issuer: Option<String>,
    /// The backend's redirect: the code (or the error) and its login.
    pub callback: Option<(Callback, SavedLogin)>,
    /// The session kept from an earlier visit (no access token).
    pub stored: Option<Session>,
}

/// The query to start with and the redirect it answers: a redirect of the
/// backend (state and issuer of the saved login) starts with the saved
/// query; anything else is the page's own query.
pub fn page_start(
    query: &str,
    saved: Option<SavedLogin>,
    page_path: &str,
) -> (String, Option<(Callback, SavedLogin)>) {
    let Some(saved) = saved else {
        return (query.to_owned(), None);
    };
    let q = query.trim_start_matches('?');
    if !q.split('&').any(|p| p.starts_with("state=")) {
        return (query.to_owned(), None);
    }
    match client::parse_callback(
        &format!("{page_path}?{q}"),
        page_path,
        &saved.login,
        &saved.issuer,
    ) {
        Callback::Ignore => (query.to_owned(), None),
        cb => (saved.query.clone(), Some((cb, saved))),
    }
}

/// The `auth=` page option: an `https://` URL (or `http://` on the
/// loopback address, for development).
pub fn issuer_from_query(query: &str) -> Option<String> {
    client::parse_form(query.trim_start_matches('?'))
        .into_iter()
        .find(|(k, _)| k == "auth")
        .map(|(_, v)| v.trim().trim_end_matches('/').to_owned())
        .filter(|v| {
            v.starts_with("https://")
                || v.starts_with("http://127.0.0.1")
                || v.starts_with("http://localhost")
                || v.starts_with("http://[::1]")
        })
}

/// What `localStorage` keeps: the session without the access token.
pub fn stored(s: &Session) -> Session {
    Session {
        access_token: String::new(),
        access_expires_at: 0,
        ..s.clone()
    }
}

/// The page's state of the sign-in.
#[derive(Resource, Debug, Default)]
pub struct WebAccount {
    pub issuer: Option<String>,
    pub session: Option<Session>,
    pub state: AccountState,
    pub message: Option<String>,
    /// Answers of the page's `fetch()` calls, handled next frame.
    events: std::sync::Arc<std::sync::Mutex<Vec<Event>>>,
    /// A token call is on its way.
    busy: bool,
}

#[derive(Debug)]
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
enum Event {
    Session(Session),
    Failed { message: String, sign_out: bool },
    SignedOut,
}

impl WebAccount {
    pub fn info(&self) -> AccountInfo {
        AccountInfo {
            issuer: self.issuer.clone(),
            state: self.state.clone(),
            message: self.message.clone(),
        }
    }

    fn signed_in_state(&self) -> AccountState {
        match &self.session {
            Some(s) => AccountState::SignedIn {
                name: s.name.clone(),
                providers: s
                    .providers
                    .iter()
                    .filter_map(|p| Provider::parse(p))
                    .collect(),
            },
            None => AccountState::SignedOut,
        }
    }
}

pub struct WebAccountPlugin;

impl Plugin for WebAccountPlugin {
    fn build(&self, app: &mut App) {
        use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
        let start = app
            .world()
            .get_resource::<AccountStart>()
            .cloned()
            .unwrap_or_default();
        let account = WebAccount {
            issuer: start.issuer.clone(),
            session: start.stored.clone(),
            state: if start.stored.is_some() {
                AccountState::Waiting {
                    provider: None,
                    linking: false,
                }
            } else {
                AccountState::SignedOut
            },
            ..WebAccount::default()
        };
        app.insert_resource(account)
            .add_systems(Startup, begin)
            .add_systems(Update, update);
        let provider = || clap::Arg::new("provider").value_parser(["google", "strava"]);
        app.console_command(ConsoleCommand::new(
            Permission::User,
            clap::Command::new("account")
                .about("Sign in with Google or Strava (spec 9.1): status, signin, link, unlink, signout")
                .subcommand(clap::Command::new("status"))
                .subcommand(clap::Command::new("signin").arg(provider()).arg(
                    clap::Arg::new("code").long("code").action(clap::ArgAction::SetTrue),
                ))
                .subcommand(clap::Command::new("link").arg(provider().required(true)))
                .subcommand(clap::Command::new("unlink").arg(provider().required(true)))
                .subcommand(clap::Command::new("signout"))
                .subcommand(clap::Command::new("cancel")),
            |world, m| {
                let p = |m: &clap::ArgMatches| m.get_one::<String>("provider").and_then(|p| Provider::parse(p));
                let mut a = world.resource_mut::<WebAccount>();
                if a.issuer.is_none() && !matches!(m.subcommand(), None | Some(("status", _))) {
                    return Err("sign-in is not set up on this page (page option auth=URL)".into());
                }
                match m.subcommand() {
                    None | Some(("status", _)) => Ok(a.info().status()),
                    Some(("signin", sub)) => {
                        if sub.get_flag("code") {
                            return Err("in the browser, sign in with Google or Strava".into());
                        }
                        a.state = AccountState::Waiting {
                            provider: p(sub),
                            linking: false,
                        };
                        go(&mut a, p(sub), false);
                        Ok("going to the sign-in …".into())
                    }
                    Some(("link", sub)) => {
                        let pr = p(sub).ok_or("which provider?")?;
                        if a.session.is_none() {
                            return Err("sign in first".into());
                        }
                        a.state = AccountState::Waiting {
                            provider: Some(pr),
                            linking: true,
                        };
                        go(&mut a, Some(pr), true);
                        Ok(format!("going to the {pr} sign-in to link it …"))
                    }
                    Some(("unlink", sub)) => {
                        let pr = p(sub).ok_or("which provider?")?;
                        unlink(&mut a, pr);
                        Ok(format!("unlinking {pr} …"))
                    }
                    Some(("signout", _)) => {
                        sign_out(&mut a);
                        Ok("signed out".into())
                    }
                    Some(("cancel", _)) => {
                        a.state = a.signed_in_state();
                        Ok("cancelled".into())
                    }
                    Some((other, _)) => Err(format!("unknown subcommand {other}")),
                }
            },
        ));
    }
}

/// At the start: the redirect's code is exchanged, a kept session
/// refreshed.
fn begin(start: Option<Res<AccountStart>>, mut a: ResMut<WebAccount>) {
    let Some(start) = start else {
        return;
    };
    match &start.callback {
        Some((Callback::Code(code), saved)) => {
            a.state = AccountState::Waiting {
                provider: saved.login.provider,
                linking: saved.linking,
            };
            exchange(&mut a, code.clone(), saved.clone());
        }
        Some((Callback::Denied(why), _)) => {
            a.state = a.signed_in_state();
            a.message = Some(format!("Not signed in: {why}"));
            if a.session.is_some() {
                refresh(&mut a);
            }
        }
        _ if a.session.is_some() => refresh(&mut a),
        _ => {}
    }
}

/// Handles the answers, refreshes the access token before it expires and
/// shows the state on the Account page.
fn update(mut a: ResMut<WebAccount>, menu: Option<ResMut<ocw_ui::menu::AccountMenu>>) {
    let events: Vec<Event> =
        std::mem::take(&mut *a.events.lock().unwrap_or_else(|e| e.into_inner()));
    for e in events {
        a.busy = false;
        match e {
            Event::Session(s) => {
                info!(account = %s.account_id, "signed in as {}", s.name);
                keep(&s);
                a.session = Some(s);
                a.message = None;
                a.state = a.signed_in_state();
            }
            Event::Failed { message, sign_out } => {
                warn!("account: {message}");
                if sign_out {
                    a.session = None;
                    forget();
                }
                a.message = Some(message);
                a.state = a.signed_in_state();
            }
            Event::SignedOut => {
                a.session = None;
                a.state = AccountState::SignedOut;
                a.message = Some("Signed out.".into());
            }
        }
    }
    if !a.busy
        && let Some(s) = &a.session
        && matches!(a.state, AccountState::SignedIn { .. })
        && s.needs_refresh(now_s())
    {
        refresh(&mut a);
    }
    if let Some(mut menu) = menu {
        let info = a.info();
        if menu.0 != info {
            menu.0 = info;
        }
    }
}

fn now_s() -> i64 {
    ocw_server::unix_now_ms() / 1000
}

#[cfg(not(target_arch = "wasm32"))]
mod net {
    //! Natively (the tests) there is no page: nothing is sent.
    use super::*;

    pub fn go(a: &mut WebAccount, _: Option<Provider>, _: bool) {
        a.message = Some("sign-in needs the browser".into());
    }
    pub fn exchange(_: &mut WebAccount, _: String, _: SavedLogin) {}
    pub fn refresh(_: &mut WebAccount) {}
    pub fn unlink(_: &mut WebAccount, _: Provider) {}
    pub fn sign_out(a: &mut WebAccount) {
        a.session = None;
        a.state = AccountState::SignedOut;
    }
    pub fn keep(_: &Session) {}
    pub fn forget() {}
}

#[cfg(target_arch = "wasm32")]
mod net {
    //! The page's side: storage, navigation and `fetch()`.
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    fn window() -> Option<web_sys::Window> {
        web_sys::window()
    }

    pub fn session_get(key: &str) -> Option<String> {
        window()?.session_storage().ok()??.get_item(key).ok()?
    }

    pub fn session_set(key: &str, value: &str) {
        if let Some(s) = window().and_then(|w| w.session_storage().ok().flatten()) {
            let _ = s.set_item(key, value);
        }
    }

    pub fn session_remove(key: &str) {
        if let Some(s) = window().and_then(|w| w.session_storage().ok().flatten()) {
            let _ = s.remove_item(key);
        }
    }

    fn local() -> Option<web_sys::Storage> {
        window()?.local_storage().ok()?
    }

    pub fn keep(s: &Session) {
        if let (Some(l), Ok(json)) = (local(), serde_json::to_string(&stored(s))) {
            let _ = l.set_item(SESSION_KEY, &json);
        }
        session_set(
            ACCESS_KEY,
            &format!("{} {}", s.access_expires_at, s.access_token),
        );
    }

    pub fn forget() {
        if let Some(l) = local() {
            let _ = l.remove_item(SESSION_KEY);
        }
        session_remove(ACCESS_KEY);
    }

    /// The session kept in `localStorage`.
    pub fn stored_session() -> Option<Session> {
        serde_json::from_str(&local()?.get_item(SESSION_KEY).ok()??).ok()
    }

    /// A still valid access token of this tab (for a remote ride's hello).
    pub fn tab_access_token() -> Option<String> {
        let v = session_get(ACCESS_KEY)?;
        let (exp, token) = v.split_once(' ')?;
        (exp.parse::<i64>().ok()? > now_s() + 30).then(|| token.to_owned())
    }

    fn random96() -> [u8; 96] {
        let mut b = [0u8; 96];
        if let Some(c) = window().and_then(|w| w.crypto().ok()) {
            let _ = c.get_random_values_with_u8_array(&mut b);
        }
        b
    }

    /// The page's own URL without its query (the redirect URI).
    pub fn page_url() -> (String, String) {
        let l = window().map(|w| w.location());
        let origin = l.as_ref().and_then(|l| l.origin().ok()).unwrap_or_default();
        let path = l
            .as_ref()
            .and_then(|l| l.pathname().ok())
            .unwrap_or_else(|| "/".into());
        (format!("{origin}{path}"), path)
    }

    /// Puts `url` into the address bar without loading it.
    pub fn replace_url(url: &str) {
        if let Some(h) = window().and_then(|w| w.history().ok()) {
            let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(url));
        }
    }

    async fn call(
        method: &str,
        url: &str,
        body: Option<&str>,
        bearer: Option<&str>,
    ) -> Result<(u16, String), String> {
        let js = |e: wasm_bindgen::JsValue| format!("{e:?}");
        let w = window().ok_or("no window")?;
        let init = web_sys::RequestInit::new();
        init.set_method(method);
        let headers = web_sys::Headers::new().map_err(js)?;
        if body.is_some() {
            headers
                .set("content-type", "application/x-www-form-urlencoded")
                .map_err(js)?;
        }
        if let Some(t) = bearer {
            headers
                .set("authorization", &format!("Bearer {t}"))
                .map_err(js)?;
        }
        init.set_headers(&headers);
        if let Some(b) = body {
            init.set_body(&wasm_bindgen::JsValue::from_str(b));
        }
        let req = web_sys::Request::new_with_str_and_init(url, &init).map_err(js)?;
        let resp: web_sys::Response = JsFuture::from(w.fetch_with_request(&req))
            .await
            .map_err(|_| "the account server cannot be reached".to_string())?
            .dyn_into()
            .map_err(js)?;
        let status = resp.status();
        let text = JsFuture::from(resp.text().map_err(js)?)
            .await
            .map_err(js)?
            .as_string()
            .unwrap_or_default();
        Ok((status, text))
    }

    /// Saves the login and goes to the backend (with a link ticket first
    /// when linking).
    pub fn go(a: &mut WebAccount, provider: Option<Provider>, linking: bool) {
        let Some(issuer) = a.issuer.clone() else {
            return;
        };
        let (redirect, _) = page_url();
        let mut login = PendingLogin::new(&random96(), &redirect, provider);
        let query = window()
            .and_then(|w| w.location().search().ok())
            .unwrap_or_default();
        let bearer = a.session.as_ref().map(|s| s.access_token.clone());
        let events = a.events.clone();
        a.busy = true;
        wasm_bindgen_futures::spawn_local(async move {
            let fail = |m: String| {
                events
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(Event::Failed {
                        message: m,
                        sign_out: false,
                    })
            };
            if linking {
                let form = client::form(&[("code_challenge", &login.challenge())]);
                match call(
                    "POST",
                    &format!("{issuer}/account/link"),
                    Some(&form),
                    bearer.as_deref(),
                )
                .await
                {
                    Ok((200, body)) => {
                        match serde_json::from_str::<ocw_auth::oidc::LinkTicket>(&body) {
                            Ok(t) => login.link_ticket = Some(t.link_ticket),
                            Err(e) => return fail(e.to_string()),
                        }
                    }
                    Ok((s, _)) => return fail(format!("linking refused (HTTP {s})")),
                    Err(e) => return fail(e),
                }
            }
            let saved = SavedLogin {
                issuer: issuer.clone(),
                login,
                query,
                linking,
            };
            match serde_json::to_string(&saved) {
                Ok(json) => session_set(PENDING_KEY, &json),
                Err(e) => return fail(e.to_string()),
            }
            let url = saved
                .login
                .authorize_url(&format!("{issuer}/authorize"), ocw_auth::CLIENT_WEB);
            if let Some(w) = window() {
                let _ = w.location().assign(&url);
            }
        });
    }

    pub fn exchange(a: &mut WebAccount, code: String, saved: SavedLogin) {
        let events = a.events.clone();
        a.busy = true;
        wasm_bindgen_futures::spawn_local(async move {
            let form = saved.login.code_form(ocw_auth::CLIENT_WEB, &code);
            let r = call(
                "POST",
                &format!("{}/token", saved.issuer),
                Some(&form),
                None,
            )
            .await;
            let e = match r {
                Ok((status, body)) => match client::parse_token_response(status, &body) {
                    Ok(t) => match Session::from_response(
                        &t,
                        now_s(),
                        &saved.issuer,
                        ocw_auth::CLIENT_WEB,
                        Some(&saved.login.nonce),
                        None,
                    ) {
                        Ok(s) => Event::Session(s),
                        Err(m) => Event::Failed {
                            message: m,
                            sign_out: false,
                        },
                    },
                    Err(e) => Event::Failed {
                        message: e.to_string(),
                        sign_out: false,
                    },
                },
                Err(m) => Event::Failed {
                    message: m,
                    sign_out: false,
                },
            };
            events.lock().unwrap_or_else(|e| e.into_inner()).push(e);
        });
    }

    pub fn refresh(a: &mut WebAccount) {
        let Some(s) = a.session.clone() else {
            return;
        };
        let Some(rt) = s.refresh_token.clone() else {
            return;
        };
        let events = a.events.clone();
        a.busy = true;
        wasm_bindgen_futures::spawn_local(async move {
            let form = client::refresh_form(&s.client_id, &rt);
            let e = match call("POST", &format!("{}/token", s.issuer), Some(&form), None).await {
                Ok((status, body)) => match client::parse_token_response(status, &body) {
                    Ok(t) => match Session::from_response(
                        &t,
                        now_s(),
                        &s.issuer,
                        &s.client_id,
                        None,
                        Some(&s),
                    ) {
                        Ok(n) => Event::Session(n),
                        Err(m) => Event::Failed {
                            message: m,
                            sign_out: true,
                        },
                    },
                    Err(client::TokenError::InvalidGrant(m)) => Event::Failed {
                        message: format!("Your sign-in ended ({m}): sign in again."),
                        sign_out: true,
                    },
                    Err(e) => Event::Failed {
                        message: e.to_string(),
                        sign_out: false,
                    },
                },
                // Offline: keep the session, try again later.
                Err(m) => Event::Failed {
                    message: m,
                    sign_out: false,
                },
            };
            events.lock().unwrap_or_else(|e| e.into_inner()).push(e);
        });
    }

    pub fn unlink(a: &mut WebAccount, p: Provider) {
        let Some(s) = a.session.clone() else {
            return;
        };
        let events = a.events.clone();
        a.busy = true;
        wasm_bindgen_futures::spawn_local(async move {
            let url = format!("{}/account/providers/{}", s.issuer, p.id());
            match call("DELETE", &url, None, Some(&s.access_token)).await {
                Ok((200, _)) => {
                    // New tokens name the providers now linked.
                    let form = client::refresh_form(
                        &s.client_id,
                        s.refresh_token.as_deref().unwrap_or_default(),
                    );
                    let e = match call("POST", &format!("{}/token", s.issuer), Some(&form), None)
                        .await
                    {
                        Ok((status, body)) => client::parse_token_response(status, &body)
                            .map_err(|e| e.to_string())
                            .and_then(|t| {
                                Session::from_response(
                                    &t,
                                    now_s(),
                                    &s.issuer,
                                    &s.client_id,
                                    None,
                                    Some(&s),
                                )
                            })
                            .map_or_else(
                                |m| Event::Failed {
                                    message: m,
                                    sign_out: false,
                                },
                                Event::Session,
                            ),
                        Err(m) => Event::Failed {
                            message: m,
                            sign_out: false,
                        },
                    };
                    events.lock().unwrap_or_else(|e| e.into_inner()).push(e);
                }
                Ok((_, body)) => {
                    events
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(Event::Failed {
                            message: serde_json::from_str::<ocw_auth::oidc::ErrorResponse>(&body)
                                .ok()
                                .and_then(|e| e.error_description)
                                .unwrap_or_else(|| "not unlinked".into()),
                            sign_out: false,
                        })
                }
                Err(m) => events
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(Event::Failed {
                        message: m,
                        sign_out: false,
                    }),
            }
        });
    }

    pub fn sign_out(a: &mut WebAccount) {
        let s = a.session.take();
        forget();
        a.state = AccountState::SignedOut;
        let events = a.events.clone();
        if let Some(s) = s
            && let Some(rt) = s.refresh_token.clone()
        {
            wasm_bindgen_futures::spawn_local(async move {
                let form = client::revoke_form(&s.client_id, &rt);
                let _ = call("POST", &format!("{}/revoke", s.issuer), Some(&form), None).await;
                events
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(Event::SignedOut);
            });
        }
    }
}

use net::{exchange, forget, go, keep, refresh, sign_out, unlink};

/// The page start (wasm): the redirect's code, the original query back
/// into the address bar, the kept session.
#[cfg(target_arch = "wasm32")]
pub fn start_page(query: &str) -> (String, AccountStart) {
    let saved =
        net::session_get(PENDING_KEY).and_then(|s| serde_json::from_str::<SavedLogin>(&s).ok());
    let (url, path) = net::page_url();
    let (query, callback) = page_start(query, saved, &path);
    if callback.is_some() {
        net::session_remove(PENDING_KEY);
        net::replace_url(&format!("{url}{query}"));
    }
    let issuer = issuer_from_query(&query);
    let stored = net::stored_session().filter(|s| Some(&s.issuer) == issuer.as_ref());
    (
        query,
        AccountStart {
            issuer,
            callback,
            stored,
        },
    )
}

/// The access token of this tab for a remote ride's hello (wasm).
#[cfg(target_arch = "wasm32")]
pub fn tab_access_token() -> Option<String> {
    net::tab_access_token()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn saved() -> SavedLogin {
        SavedLogin {
            issuer: "https://accounts.example.org".into(),
            login: PendingLogin::new(
                &[7; 96],
                "https://ride.example.org/",
                Some(Provider::Google),
            ),
            query: "?world=liechtenstein&auth=https%3A%2F%2Faccounts.example.org".into(),
            linking: false,
        }
    }

    #[test]
    fn the_redirect_brings_back_the_original_query() {
        let s = saved();
        let q = format!(
            "?code=c0de&state={}&iss=https%3A%2F%2Faccounts.example.org",
            s.login.state
        );
        let (query, cb) = page_start(&q, Some(s.clone()), "/");
        assert_eq!(query, s.query);
        assert_eq!(cb.unwrap().0, Callback::Code("c0de".into()));
        // Another state (a forged link) or issuer: the page's own query.
        for bad in [
            "?code=c0de&state=forged".to_owned(),
            format!(
                "?code=c0de&state={}&iss=https%3A%2F%2Fevil.example",
                s.login.state
            ),
        ] {
            let (query, cb) = page_start(&bad, Some(s.clone()), "/");
            assert_eq!(query, bad);
            assert!(cb.is_none());
        }
        // No login waiting: the query as it is.
        let (query, cb) = page_start("?world=x", None, "/");
        assert_eq!((query.as_str(), cb.is_none()), ("?world=x", true));
        // A denial comes back too.
        let q = format!("?error=access_denied&state={}", s.login.state);
        let (_, cb) = page_start(&q, Some(s), "/");
        assert!(matches!(cb.unwrap().0, Callback::Denied(_)));
    }

    #[test]
    fn the_accounts_server_option_and_the_stored_session() {
        assert_eq!(
            issuer_from_query("?world=x&auth=https%3A%2F%2Faccounts.example.org%2F").as_deref(),
            Some("https://accounts.example.org")
        );
        assert_eq!(
            issuer_from_query("auth=http://127.0.0.1:8480").as_deref(),
            Some("http://127.0.0.1:8480")
        );
        assert_eq!(
            issuer_from_query("auth=http://evil.example"),
            None,
            "plain http"
        );
        assert_eq!(issuer_from_query("world=x"), None);
        let s = Session {
            issuer: "i".into(),
            client_id: "ocw-web".into(),
            access_token: "secret-access".into(),
            access_expires_at: 99,
            refresh_token: Some("rt".into()),
            account_id: "acc".into(),
            name: "N".into(),
            providers: vec![],
        };
        let kept = stored(&s);
        assert!(kept.access_token.is_empty() && kept.refresh_token.is_some());
    }

    #[test]
    fn the_command_and_page_without_a_server() {
        use ocw_render::console::{ConsoleCorePlugin, ConsoleState};
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ocw_ui::menu::AccountMenu>()
            .add_plugins(WebAccountPlugin);
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("account signin google");
        app.update();
        let log = app.world().resource::<ConsoleState>().log().to_vec();
        assert!(log.last().unwrap().contains("not set up"), "{log:?}");
        let menu = app.world().resource::<ocw_ui::menu::AccountMenu>();
        assert!(menu.issuer.is_none());
        assert!(menu.status().contains("anonymously"));
    }
}

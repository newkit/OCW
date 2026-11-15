//! The apps' side of the sign-in (spec §9.1): the authorization request
//! with PKCE, `state` and `nonce`; the loopback redirect of the desktop
//! (RFC 8252 §7.3) and the browser's redirect; the token, refresh, device
//! (RFC 8628) and revocation forms; the token set an app keeps.
//!
//! Transport-free: the desktop sends the forms with ureq, the browser
//! with `fetch()`, the tests against an in-process backend.

use serde::{Deserialize, Serialize};

use crate::jwt::{IdClaims, Parsed};
use crate::oidc::{ErrorResponse, GRANT_DEVICE_CODE, Provider, TokenResponse};

/// The path of the desktop's loopback redirect (`http://127.0.0.1:PORT/callback`).
pub const LOOPBACK_PATH: &str = "/callback";

/// `application/x-www-form-urlencoded` of `pairs`.
pub fn form(pairs: &[(&str, &str)]) -> String {
    let mut s = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        s.append_pair(k, v);
    }
    s.finish()
}

/// Decodes a query string or form body into pairs.
pub fn parse_form(s: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(s.as_bytes())
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect()
}

/// One sign-in in progress: what the app must remember between opening
/// the browser and the redirect coming back.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingLogin {
    /// `None`: the backend asks which provider.
    pub provider: Option<Provider>,
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub redirect_uri: String,
    /// Linking another provider to the signed-in account (from `POST
    /// /account/link`, bound to this login's PKCE challenge).
    #[serde(default)]
    pub link_ticket: Option<String>,
}

impl std::fmt::Debug for PendingLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PendingLogin {{ provider: {:?}, redirect_uri: {}, link: {}, secrets: *** }}",
            self.provider,
            self.redirect_uri,
            self.link_ticket.is_some()
        )
    }
}

impl PendingLogin {
    /// A new login from 96 random bytes (state, nonce, PKCE verifier).
    pub fn new(random: &[u8; 96], redirect_uri: &str, provider: Option<Provider>) -> Self {
        let part = |i: usize| -> [u8; 32] { random[i * 32..(i + 1) * 32].try_into().unwrap() };
        Self {
            provider,
            state: crate::random_token(&part(0)),
            nonce: crate::random_token(&part(1)),
            verifier: crate::pkce::verifier(&part(2)),
            redirect_uri: redirect_uri.to_owned(),
            link_ticket: None,
        }
    }

    pub fn challenge(&self) -> String {
        crate::pkce::challenge(&self.verifier)
    }

    /// The URL the browser opens.
    pub fn authorize_url(&self, authorization_endpoint: &str, client_id: &str) -> String {
        let challenge = self.challenge();
        let mut pairs = vec![
            ("response_type", "code"),
            ("client_id", client_id),
            ("redirect_uri", self.redirect_uri.as_str()),
            ("scope", crate::oidc::APP_SCOPE),
            ("state", self.state.as_str()),
            ("nonce", self.nonce.as_str()),
            ("code_challenge", challenge.as_str()),
            ("code_challenge_method", crate::pkce::METHOD),
        ];
        if let Some(p) = self.provider {
            pairs.push(("provider", p.id()));
        }
        if let Some(t) = &self.link_ticket {
            pairs.push(("link_ticket", t.as_str()));
        }
        let sep = if authorization_endpoint.contains('?') {
            '&'
        } else {
            '?'
        };
        format!("{authorization_endpoint}{sep}{}", form(&pairs))
    }

    /// The token request for the code the redirect brought.
    pub fn code_form(&self, client_id: &str, code: &str) -> String {
        form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", &self.redirect_uri),
            ("client_id", client_id),
            ("code_verifier", &self.verifier),
        ])
    }
}

/// What a redirect to the app brought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    Code(String),
    /// The user declined, or the backend reported an error.
    Denied(String),
    /// Not our redirect (another path, a wrong `state`: CSRF, or the
    /// response of another issuer: mix-up, RFC 9207).
    Ignore,
}

/// Parses the redirect target (`/callback?code=…&state=…&iss=…`, or the
/// browser's page query) against the pending login.
pub fn parse_callback(target: &str, path: &str, pending: &PendingLogin, issuer: &str) -> Callback {
    let (p, query) = target.split_once('?').unwrap_or((target, ""));
    if p != path {
        return Callback::Ignore;
    }
    let pairs = parse_form(query);
    let get = |k: &str| pairs.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if !get("state").is_some_and(|s| crate::constant_time_eq(s, &pending.state)) {
        return Callback::Ignore;
    }
    if get("iss").is_some_and(|i| i != issuer) {
        return Callback::Ignore;
    }
    if let Some(e) = get("error") {
        return Callback::Denied(get("error_description").unwrap_or(e).to_owned());
    }
    match get("code") {
        Some(c) if !c.is_empty() => Callback::Code(c.to_owned()),
        _ => Callback::Denied("no authorization code".into()),
    }
}

pub fn refresh_form(client_id: &str, refresh_token: &str) -> String {
    form(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", client_id),
    ])
}

pub fn device_start_form(client_id: &str) -> String {
    form(&[("client_id", client_id), ("scope", crate::oidc::APP_SCOPE)])
}

pub fn device_poll_form(client_id: &str, device_code: &str) -> String {
    form(&[
        ("grant_type", GRANT_DEVICE_CODE),
        ("device_code", device_code),
        ("client_id", client_id),
    ])
}

pub fn revoke_form(client_id: &str, token: &str) -> String {
    form(&[
        ("token", token),
        ("token_type_hint", "refresh_token"),
        ("client_id", client_id),
    ])
}

/// Why a token request failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// The grant is gone (revoked, expired, reused): sign in again.
    #[error("sign-in expired: {0}")]
    InvalidGrant(String),
    #[error("{0}")]
    Other(String),
}

/// A token endpoint answer.
pub fn parse_token_response(status: u16, body: &str) -> Result<TokenResponse, TokenError> {
    if status == 200 {
        return serde_json::from_str(body)
            .map_err(|e| TokenError::Other(format!("bad token response: {e}")));
    }
    match serde_json::from_str::<ErrorResponse>(body) {
        Ok(e) if e.error == "invalid_grant" => Err(TokenError::InvalidGrant(
            e.error_description.unwrap_or(e.error),
        )),
        Ok(e) => Err(TokenError::Other(match e.error_description {
            Some(d) => format!("{}: {d}", e.error),
            None => e.error,
        })),
        Err(_) => Err(TokenError::Other(format!("HTTP {status}"))),
    }
}

/// One poll of the device flow (RFC 8628 §3.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Poll {
    /// The user has not confirmed yet: poll again after the interval.
    Pending,
    /// Polled too fast: add 5 s to the interval.
    SlowDown,
    Denied,
    Expired,
    Tokens(TokenResponse),
    Failed(String),
}

pub fn parse_poll(status: u16, body: &str) -> Poll {
    match parse_token_response(status, body) {
        Ok(t) => Poll::Tokens(t),
        Err(_) if status != 200 => match serde_json::from_str::<ErrorResponse>(body) {
            Ok(e) => match e.error.as_str() {
                "authorization_pending" => Poll::Pending,
                "slow_down" => Poll::SlowDown,
                "access_denied" => Poll::Denied,
                "expired_token" => Poll::Expired,
                _ => Poll::Failed(e.error_description.unwrap_or(e.error)),
            },
            Err(_) => Poll::Failed(format!("HTTP {status}")),
        },
        Err(e) => Poll::Failed(e.to_string()),
    }
}

/// The tokens an app keeps (the desktop in the OS keyring or a private
/// file, the browser the refresh token in `localStorage`).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub issuer: String,
    pub client_id: String,
    pub access_token: String,
    /// Unix seconds.
    pub access_expires_at: i64,
    #[serde(default)]
    pub refresh_token: Option<String>,
    pub account_id: String,
    pub name: String,
    #[serde(default)]
    pub providers: Vec<String>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Session {{ account: {}, name: {}, providers: {:?}, expires_at: {}, tokens: *** }}",
            self.account_id, self.name, self.providers, self.access_expires_at
        )
    }
}

/// Refresh this long before the access token expires.
pub const REFRESH_MARGIN_S: i64 = 60;

impl Session {
    /// The session from a token response. The ID token comes straight
    /// from the token endpoint over TLS (OpenID Connect Core §3.1.3.7), so
    /// its claims are read without the signature, but issuer, audience
    /// and nonce are checked. A refresh may come without an ID token: the
    /// identity of `previous` then stays.
    pub fn from_response(
        resp: &TokenResponse,
        now_s: i64,
        issuer: &str,
        client_id: &str,
        nonce: Option<&str>,
        previous: Option<&Session>,
    ) -> Result<Self, String> {
        if !resp.token_type.eq_ignore_ascii_case("bearer") {
            return Err(format!("unexpected token type {}", resp.token_type));
        }
        let (account_id, name, providers) = match &resp.id_token {
            Some(t) => {
                let c: IdClaims = Parsed::parse(t)
                    .and_then(|p| p.claims_unverified())
                    .map_err(|e| format!("bad ID token: {e}"))?;
                if c.iss != issuer {
                    return Err("ID token from another issuer".into());
                }
                if c.aud != client_id {
                    return Err("ID token for another app".into());
                }
                if let Some(n) = nonce
                    && c.nonce.as_deref() != Some(n)
                {
                    return Err("ID token nonce mismatch".into());
                }
                if let Some(p) = previous
                    && p.account_id != c.sub
                {
                    return Err("the refresh answered for another account".into());
                }
                (c.sub, c.name, c.providers)
            }
            None => match previous {
                Some(p) => (p.account_id.clone(), p.name.clone(), p.providers.clone()),
                None => return Err("no ID token".into()),
            },
        };
        Ok(Self {
            issuer: issuer.to_owned(),
            client_id: client_id.to_owned(),
            access_token: resp.access_token.clone(),
            access_expires_at: now_s + resp.expires_in,
            refresh_token: resp
                .refresh_token
                .clone()
                .or_else(|| previous.and_then(|p| p.refresh_token.clone())),
            account_id,
            name,
            providers,
        })
    }

    pub fn needs_refresh(&self, now_s: i64) -> bool {
        now_s + REFRESH_MARGIN_S >= self.access_expires_at
    }

    /// The access token is still good for a hello.
    pub fn access_valid(&self, now_s: i64) -> bool {
        now_s + 5 < self.access_expires_at
    }

    pub fn has(&self, p: Provider) -> bool {
        self.providers.iter().any(|x| x == p.id())
    }
}

/// Whether `uri` is a loopback redirect (RFC 8252 §7.3: `http`, an IP
/// literal of the loopback interface, any port) with this `path`.
pub fn is_loopback_redirect(uri: &str, path: &str) -> bool {
    let Some(rest) = uri.strip_prefix("http://") else {
        return false;
    };
    let (host_port, p) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, ""),
    };
    let port = if let Some(r) = host_port.strip_prefix("127.0.0.1:") {
        r
    } else if let Some(r) = host_port.strip_prefix("[::1]:") {
        r
    } else {
        return false;
    };
    !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit()) && p == path
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jwt::{TYP_ID, sign};

    fn pending() -> PendingLogin {
        let mut r = [0u8; 96];
        for (i, b) in r.iter_mut().enumerate() {
            *b = i as u8;
        }
        PendingLogin::new(&r, "http://127.0.0.1:5123/callback", Some(Provider::Google))
    }

    #[test]
    fn authorize_url_has_pkce_state_nonce() {
        let p = pending();
        assert_ne!(p.state, p.nonce);
        let url = p.authorize_url("https://id.example/authorize", "ocw-desktop");
        let (base, q) = url.split_once('?').unwrap();
        assert_eq!(base, "https://id.example/authorize");
        let q = parse_form(q);
        let get = |k: &str| q.iter().find(|(n, _)| n == k).unwrap().1.clone();
        assert_eq!(get("code_challenge_method"), "S256");
        assert_eq!(get("code_challenge"), crate::pkce::challenge(&p.verifier));
        assert_eq!(get("state"), p.state);
        assert_eq!(get("nonce"), p.nonce);
        assert_eq!(get("provider"), "google");
        assert_eq!(get("redirect_uri"), "http://127.0.0.1:5123/callback");
        assert!(
            !url.contains(&p.verifier),
            "the verifier never leaves the app"
        );
        let code = parse_form(&p.code_form("ocw-desktop", "c0de"));
        assert!(code.contains(&("code_verifier".into(), p.verifier.clone())));
        assert!(!format!("{p:?}").contains(&p.verifier));
    }

    #[test]
    fn callback_checks_state_and_issuer() {
        let p = pending();
        let iss = "https://id.example";
        let ok = format!(
            "/callback?code=abc&state={}&iss={}",
            p.state, "https%3A%2F%2Fid.example"
        );
        assert_eq!(
            parse_callback(&ok, LOOPBACK_PATH, &p, iss),
            Callback::Code("abc".into())
        );
        let no_iss = format!("/callback?code=abc&state={}", p.state);
        assert_eq!(
            parse_callback(&no_iss, LOOPBACK_PATH, &p, iss),
            Callback::Code("abc".into())
        );
        for bad in [
            "/callback?code=abc&state=evil".to_owned(),
            format!("/favicon.ico?state={}", p.state),
            format!(
                "/callback?code=abc&state={}&iss=https%3A%2F%2Fevil.example",
                p.state
            ),
        ] {
            assert_eq!(
                parse_callback(&bad, LOOPBACK_PATH, &p, iss),
                Callback::Ignore,
                "{bad}"
            );
        }
        let denied = format!("/callback?error=access_denied&state={}", p.state);
        assert_eq!(
            parse_callback(&denied, LOOPBACK_PATH, &p, iss),
            Callback::Denied("access_denied".into())
        );
    }

    #[test]
    fn loopback_redirects() {
        assert!(is_loopback_redirect(
            "http://127.0.0.1:5123/callback",
            "/callback"
        ));
        assert!(is_loopback_redirect(
            "http://[::1]:80/callback",
            "/callback"
        ));
        for bad in [
            "https://127.0.0.1:5123/callback",
            "http://localhost:5123/callback",
            "http://127.0.0.1/callback",
            "http://127.0.0.1:5123/other",
            "http://127.0.0.1:5123/callback/../x",
            "http://127.0.0.1.evil.example:5123/callback",
            "http://127.0.0.1:12a/callback",
        ] {
            assert!(!is_loopback_redirect(bad, "/callback"), "{bad}");
        }
    }

    fn id_token(sub: &str, aud: &str, nonce: Option<&str>) -> String {
        let key = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
        let c = IdClaims {
            iss: "https://id.example".into(),
            sub: sub.into(),
            aud: aud.into(),
            exp: 2000,
            iat: 1000,
            auth_time: 1000,
            nonce: nonce.map(str::to_owned),
            name: "Marianne Vos".into(),
            providers: vec!["strava".into()],
        };
        sign(&c, &key, "k", TYP_ID)
    }

    fn response(id: Option<String>, refresh: Option<&str>) -> TokenResponse {
        TokenResponse {
            access_token: "at".into(),
            token_type: "Bearer".into(),
            expires_in: 600,
            refresh_token: refresh.map(str::to_owned),
            id_token: id,
            scope: None,
        }
    }

    #[test]
    fn session_from_responses() {
        let iss = "https://id.example";
        let r = response(
            Some(id_token("acc_1", "ocw-desktop", Some("n"))),
            Some("rt1"),
        );
        let s = Session::from_response(&r, 1000, iss, "ocw-desktop", Some("n"), None).unwrap();
        assert_eq!(
            (s.account_id.as_str(), s.name.as_str()),
            ("acc_1", "Marianne Vos")
        );
        assert!(s.has(Provider::Strava) && !s.has(Provider::Google));
        assert_eq!(s.access_expires_at, 1600);
        assert!(!s.needs_refresh(1500) && s.needs_refresh(1540));
        assert!(!format!("{s:?}").contains("rt1"));
        // Nonce, audience, issuer.
        assert!(Session::from_response(&r, 0, iss, "ocw-desktop", Some("other"), None).is_err());
        assert!(Session::from_response(&r, 0, iss, "ocw-web", Some("n"), None).is_err());
        assert!(
            Session::from_response(&r, 0, "https://evil", "ocw-desktop", Some("n"), None).is_err()
        );
        // A refresh without ID token or new refresh token keeps both.
        let r2 = response(None, None);
        let s2 = Session::from_response(&r2, 1600, iss, "ocw-desktop", None, Some(&s)).unwrap();
        assert_eq!(s2.refresh_token.as_deref(), Some("rt1"));
        assert_eq!(s2.account_id, "acc_1");
        // Another account on refresh: refused.
        let r3 = response(Some(id_token("acc_2", "ocw-desktop", None)), Some("rt2"));
        assert!(Session::from_response(&r3, 0, iss, "ocw-desktop", None, Some(&s)).is_err());
        assert!(Session::from_response(&r2, 0, iss, "ocw-desktop", None, None).is_err());
    }

    #[test]
    fn token_and_poll_answers() {
        assert_eq!(
            parse_token_response(
                400,
                r#"{"error":"invalid_grant","error_description":"reused"}"#
            ),
            Err(TokenError::InvalidGrant("reused".into()))
        );
        assert!(matches!(
            parse_token_response(500, "oops"),
            Err(TokenError::Other(_))
        ));
        for (body, want) in [
            (r#"{"error":"authorization_pending"}"#, Poll::Pending),
            (r#"{"error":"slow_down"}"#, Poll::SlowDown),
            (r#"{"error":"access_denied"}"#, Poll::Denied),
            (r#"{"error":"expired_token"}"#, Poll::Expired),
        ] {
            assert_eq!(parse_poll(400, body), want, "{body}");
        }
        let ok = serde_json::to_string(&response(None, Some("r"))).unwrap();
        assert!(matches!(parse_poll(200, &ok), Poll::Tokens(_)));
        let f = parse_form(&device_poll_form("ocw-kiosk", "dc"));
        assert!(f.contains(&("grant_type".into(), GRANT_DEVICE_CODE.into())));
    }
}

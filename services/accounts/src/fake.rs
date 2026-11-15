//! Fake providers for development and tests (spec §9.1: no network): an
//! in-process **Google** (OpenID Connect: discovery, authorization with
//! PKCE S256 and nonce, a token endpoint with client authentication, ID
//! tokens signed RS256 like Google's, JWKS) and an in-process **Strava**
//! (OAuth 2: authorization, the token response with the athlete, the
//! rate-limit headers). Both approve every sign-in at once as their
//! configured user, unless told to refuse.
//!
//! [`TestBrowser`] plays the user's browser (follows redirects, keeps
//! cookies, submits forms); [`DevBackend`] wires the fakes and a backend
//! together for `ocw-server --auth-dev`, the screenshots and the browser
//! test. Never used with real providers: the fakes' signing key is a
//! public test key.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use openidconnect::core::{CoreJsonWebKeySet, CoreJwsSigningAlgorithm, CoreRsaPrivateSigningKey};
use openidconnect::{JsonWebKeyId, PrivateSigningKey};

use crate::config::{ClientConfig, Config, GoogleConfig, StravaConfig};
use crate::service::AuthService;

/// The fake Google's RSA key: a test key, public in the repository.
const TEST_RSA_PEM: &str = include_str!("fake-google-test-key.pem");

type Pairs = Vec<(String, String)>;

fn get<'a>(p: &'a Pairs, k: &str) -> Option<&'a str> {
    p.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str())
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn b64(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

fn found(url: &str) -> Response {
    let mut r = StatusCode::FOUND.into_response();
    if let Ok(v) = HeaderValue::from_str(url) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r
}

fn with_query(base: &str, pairs: &[(&str, &str)]) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{}", ocw_auth::client::form(pairs))
}

async fn spawn(router: Router) -> std::io::Result<String> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });
    Ok(format!("http://{addr}"))
}

/// Who the fake signs in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeUser {
    pub subject: String,
    pub name: String,
    pub email: String,
}

struct GoogleCode {
    redirect_uri: String,
    challenge: String,
    nonce: String,
    user: FakeUser,
}

struct GoogleState {
    base: String,
    client_id: String,
    client_secret: String,
    key: CoreRsaPrivateSigningKey,
    kid: String,
    user: Mutex<FakeUser>,
    codes: Mutex<HashMap<String, GoogleCode>>,
    deny: AtomicBool,
    wrong_nonce: AtomicBool,
}

/// The fake Google.
#[derive(Clone)]
pub struct FakeGoogle {
    pub base: String,
    state: Arc<GoogleState>,
}

impl FakeGoogle {
    /// Starts it on a free loopback port (inside a Tokio runtime).
    pub async fn start() -> std::io::Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let base = format!("http://{}", listener.local_addr()?);
        let kid = "fake-google-1".to_owned();
        let key =
            CoreRsaPrivateSigningKey::from_pem(TEST_RSA_PEM, Some(JsonWebKeyId::new(kid.clone())))
                .map_err(std::io::Error::other)?;
        let state = Arc::new(GoogleState {
            base: base.clone(),
            client_id: "fake-google-client.apps.example".into(),
            client_secret: "fake-google-secret".into(),
            key,
            kid,
            user: Mutex::new(FakeUser {
                subject: "108234567890123456789".into(),
                name: "Marianne Vos".into(),
                email: "marianne@example.org".into(),
            }),
            codes: Mutex::new(HashMap::new()),
            deny: AtomicBool::new(false),
            wrong_nonce: AtomicBool::new(false),
        });
        let router = Router::new()
            .route("/.well-known/openid-configuration", get(g_discovery))
            .route("/authorize", get(g_authorize))
            .route("/token", post(g_token))
            .route("/jwks", get(g_jwks))
            .with_state(state.clone());
        tokio::spawn(async move {
            let _ = axum::serve(listener, router).await;
        });
        Ok(Self { base, state })
    }

    pub fn config(&self) -> GoogleConfig {
        GoogleConfig {
            client_id: self.state.client_id.clone(),
            client_secret: self.state.client_secret.clone(),
            issuer: self.base.clone(),
        }
    }

    pub fn set_user(&self, subject: &str, name: &str, email: &str) {
        *self.state.user.lock().unwrap_or_else(|e| e.into_inner()) = FakeUser {
            subject: subject.into(),
            name: name.into(),
            email: email.into(),
        };
    }

    /// The user cancels at Google.
    pub fn deny(&self, on: bool) {
        self.state.deny.store(on, Ordering::SeqCst);
    }

    /// Issue ID tokens with another nonce (a replayed token).
    pub fn wrong_nonce(&self, on: bool) {
        self.state.wrong_nonce.store(on, Ordering::SeqCst);
    }
}

async fn g_discovery(State(s): State<Arc<GoogleState>>) -> Response {
    axum::Json(serde_json::json!({
        "issuer": s.base,
        "authorization_endpoint": format!("{}/authorize", s.base),
        "token_endpoint": format!("{}/token", s.base),
        "jwks_uri": format!("{}/jwks", s.base),
        "response_types_supported": ["code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": ["RS256"],
        "scopes_supported": ["openid", "email", "profile"],
        "code_challenge_methods_supported": ["S256"],
    }))
    .into_response()
}

async fn g_jwks(State(s): State<Arc<GoogleState>>) -> Response {
    axum::Json(CoreJsonWebKeySet::new(vec![s.key.as_verification_key()])).into_response()
}

async fn g_authorize(State(s): State<Arc<GoogleState>>, Query(q): Query<Pairs>) -> Response {
    let (Some(redirect_uri), Some(state)) = (get(&q, "redirect_uri"), get(&q, "state")) else {
        return (StatusCode::BAD_REQUEST, "redirect_uri and state required").into_response();
    };
    let scopes: Vec<&str> = get(&q, "scope").unwrap_or_default().split(' ').collect();
    if get(&q, "client_id") != Some(s.client_id.as_str())
        || get(&q, "response_type") != Some("code")
        || !scopes.contains(&"openid")
        || get(&q, "code_challenge_method") != Some("S256")
        || get(&q, "nonce").is_none()
    {
        return (StatusCode::BAD_REQUEST, "bad authorization request").into_response();
    }
    if s.deny.load(Ordering::SeqCst) {
        return found(&with_query(
            redirect_uri,
            &[("error", "access_denied"), ("state", state)],
        ));
    }
    let code = crate::random_token();
    s.codes.lock().unwrap_or_else(|e| e.into_inner()).insert(
        code.clone(),
        GoogleCode {
            redirect_uri: redirect_uri.into(),
            challenge: get(&q, "code_challenge").unwrap_or_default().into(),
            nonce: get(&q, "nonce").unwrap_or_default().into(),
            user: s.user.lock().unwrap_or_else(|e| e.into_inner()).clone(),
        },
    );
    found(&with_query(
        redirect_uri,
        &[("code", &code), ("state", state)],
    ))
}

fn oauth_error(error: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        axum::Json(serde_json::json!({ "error": error })),
    )
        .into_response()
}

/// Client credentials from HTTP Basic or the form.
fn client_credentials(headers: &HeaderMap, form: &Pairs) -> Option<(String, String)> {
    if let Some(basic) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Basic "))
    {
        use base64::Engine;
        let raw = base64::engine::general_purpose::STANDARD
            .decode(basic)
            .ok()?;
        let s = String::from_utf8(raw).ok()?;
        let (id, secret) = s.split_once(':')?;
        let dec = |x: &str| {
            form_urlencoded::parse(format!("x={x}").as_bytes())
                .next()
                .map(|(_, v)| v.into_owned())
                .unwrap_or_default()
        };
        return Some((dec(id), dec(secret)));
    }
    Some((
        get(form, "client_id")?.into(),
        get(form, "client_secret")?.into(),
    ))
}

async fn g_token(
    State(s): State<Arc<GoogleState>>,
    headers: HeaderMap,
    Form(f): Form<Pairs>,
) -> Response {
    match client_credentials(&headers, &f) {
        Some((id, secret)) if id == s.client_id && secret == s.client_secret => {}
        _ => {
            return (
                StatusCode::UNAUTHORIZED,
                axum::Json(serde_json::json!({ "error": "invalid_client" })),
            )
                .into_response();
        }
    }
    if get(&f, "grant_type") != Some("authorization_code") {
        return oauth_error("unsupported_grant_type");
    }
    let Some(code) =
        get(&f, "code").and_then(|c| s.codes.lock().unwrap_or_else(|e| e.into_inner()).remove(c))
    else {
        return oauth_error("invalid_grant");
    };
    if get(&f, "redirect_uri") != Some(code.redirect_uri.as_str())
        || !ocw_auth::pkce::matches(
            get(&f, "code_verifier").unwrap_or_default(),
            &code.challenge,
        )
    {
        return oauth_error("invalid_grant");
    }
    let now = now_s();
    let nonce = if s.wrong_nonce.load(Ordering::SeqCst) {
        "replayed-nonce".to_owned()
    } else {
        code.nonce
    };
    let header = serde_json::json!({ "alg": "RS256", "kid": s.kid, "typ": "JWT" });
    let claims = serde_json::json!({
        "iss": s.base,
        "sub": code.user.subject,
        "aud": s.client_id,
        "azp": s.client_id,
        "iat": now,
        "exp": now + 3600,
        "nonce": nonce,
        "name": code.user.name,
        "email": code.user.email,
        "email_verified": true,
    });
    let input = format!(
        "{}.{}",
        b64(header.to_string().as_bytes()),
        b64(claims.to_string().as_bytes())
    );
    let Ok(sig) = s.key.sign(
        &CoreJwsSigningAlgorithm::RsaSsaPkcs1V15Sha256,
        input.as_bytes(),
    ) else {
        return oauth_error("server_error");
    };
    axum::Json(serde_json::json!({
        "access_token": crate::random_token(),
        "token_type": "Bearer",
        "expires_in": 3599,
        "scope": "openid email profile",
        "id_token": format!("{input}.{}", b64(&sig)),
    }))
    .into_response()
}

struct StravaCode {
    redirect_uri: String,
    challenge: Option<String>,
    scope: String,
}

struct StravaState {
    client_id: String,
    client_secret: String,
    athlete: Mutex<(i64, String, String)>,
    codes: Mutex<HashMap<String, StravaCode>>,
    grant_upload: AtomicBool,
    requests: AtomicU32,
    usage: Mutex<(u32, u32)>,
}

/// The fake Strava.
#[derive(Clone)]
pub struct FakeStrava {
    pub base: String,
    state: Arc<StravaState>,
}

impl FakeStrava {
    pub async fn start() -> std::io::Result<Self> {
        let state = Arc::new(StravaState {
            client_id: "12345".into(),
            client_secret: "fake-strava-secret".into(),
            athlete: Mutex::new((4242, "Eddy".into(), "Merckx".into())),
            codes: Mutex::new(HashMap::new()),
            grant_upload: AtomicBool::new(true),
            requests: AtomicU32::new(0),
            usage: Mutex::new((0, 0)),
        });
        let router = Router::new()
            .route("/oauth/authorize", get(s_authorize))
            .route("/oauth/token", post(s_token))
            .with_state(state.clone());
        let base = spawn(router).await?;
        Ok(Self { base, state })
    }

    pub fn config(&self) -> StravaConfig {
        StravaConfig {
            client_id: self.state.client_id.clone(),
            client_secret: self.state.client_secret.clone(),
            authorize_url: format!("{}/oauth/authorize", self.base),
            token_url: format!("{}/oauth/token", self.base),
            ask_upload: true,
            pkce: true,
            limit_15min: 200,
            limit_day: 2000,
        }
    }

    pub fn set_athlete(&self, id: i64, first: &str, last: &str) {
        *self.state.athlete.lock().unwrap_or_else(|e| e.into_inner()) =
            (id, first.into(), last.into());
    }

    /// The athlete unticks the upload permission.
    pub fn grant_upload(&self, on: bool) {
        self.state.grant_upload.store(on, Ordering::SeqCst);
    }

    /// Token requests served.
    pub fn requests(&self) -> u32 {
        self.state.requests.load(Ordering::SeqCst)
    }

    /// The usage the rate-limit headers report.
    pub fn set_usage(&self, fifteen: u32, day: u32) {
        *self.state.usage.lock().unwrap_or_else(|e| e.into_inner()) = (fifteen, day);
    }
}

async fn s_authorize(State(s): State<Arc<StravaState>>, Query(q): Query<Pairs>) -> Response {
    let (Some(redirect_uri), Some(state)) = (get(&q, "redirect_uri"), get(&q, "state")) else {
        return (StatusCode::BAD_REQUEST, "bad request").into_response();
    };
    if get(&q, "client_id") != Some(s.client_id.as_str())
        || get(&q, "response_type") != Some("code")
    {
        return (StatusCode::BAD_REQUEST, "bad request").into_response();
    }
    let requested = get(&q, "scope").unwrap_or("read");
    let scope: Vec<&str> = requested
        .split(',')
        .filter(|x| *x != "activity:write" || s.grant_upload.load(Ordering::SeqCst))
        .collect();
    let scope = scope.join(",");
    let code = crate::random_token();
    s.codes.lock().unwrap_or_else(|e| e.into_inner()).insert(
        code.clone(),
        StravaCode {
            redirect_uri: redirect_uri.into(),
            challenge: get(&q, "code_challenge").map(str::to_owned),
            scope: scope.clone(),
        },
    );
    found(&with_query(
        redirect_uri,
        &[("state", state), ("code", &code), ("scope", &scope)],
    ))
}

async fn s_token(State(s): State<Arc<StravaState>>, Form(f): Form<Pairs>) -> Response {
    let n = s.requests.fetch_add(1, Ordering::SeqCst) + 1;
    let (u15, uday) = *s.usage.lock().unwrap_or_else(|e| e.into_inner());
    let mut r = if get(&f, "client_id") != Some(s.client_id.as_str())
        || get(&f, "client_secret") != Some(s.client_secret.as_str())
    {
        (
            StatusCode::UNAUTHORIZED,
            axum::Json(serde_json::json!({ "message": "Authorization Error" })),
        )
            .into_response()
    } else {
        match get(&f, "code")
            .and_then(|c| s.codes.lock().unwrap_or_else(|e| e.into_inner()).remove(c))
        {
            Some(code)
                if get(&f, "redirect_uri").is_none_or(|u| u == code.redirect_uri)
                    && code.challenge.as_deref().is_none_or(|c| {
                        ocw_auth::pkce::matches(get(&f, "code_verifier").unwrap_or_default(), c)
                    }) =>
            {
                let (id, first, last) = s.athlete.lock().unwrap_or_else(|e| e.into_inner()).clone();
                let now = now_s();
                let _ = code.scope;
                axum::Json(serde_json::json!({
                    "token_type": "Bearer",
                    "expires_at": now + 21_600,
                    "expires_in": 21_600,
                    "refresh_token": crate::random_token(),
                    "access_token": crate::random_token(),
                    "athlete": { "id": id, "firstname": first, "lastname": last },
                }))
                .into_response()
            }
            _ => oauth_error("invalid_grant"),
        }
    };
    let h = r.headers_mut();
    h.insert("x-ratelimit-limit", HeaderValue::from_static("200,2000"));
    if let Ok(v) = HeaderValue::from_str(&format!("{},{}", u15.max(n), uday.max(n))) {
        h.insert("x-ratelimit-usage", v);
    }
    r
}

/// A backend with both fakes, for `ocw-server --auth-dev`, the
/// screenshots and the browser test.
pub struct DevBackend {
    pub service: Arc<AuthService>,
    pub issuer: String,
    pub google: FakeGoogle,
    pub strava: FakeStrava,
}

impl DevBackend {
    /// Starts the fakes and the backend on `listen` (`127.0.0.1:0`: any
    /// port). `web` is the browser app's page and origin, allowed as its
    /// redirect URI.
    pub async fn start(listen: &str, web: Option<(String, String)>) -> Result<Self, String> {
        let google = FakeGoogle::start().await.map_err(|e| e.to_string())?;
        let strava = FakeStrava::start().await.map_err(|e| e.to_string())?;
        let listener = tokio::net::TcpListener::bind(listen)
            .await
            .map_err(|e| format!("{listen}: {e}"))?;
        let addr: SocketAddr = listener.local_addr().map_err(|e| e.to_string())?;
        let issuer = format!("http://{addr}");
        let mut cfg = Config::dev(&issuer);
        cfg.google = Some(google.config());
        cfg.strava = Some(strava.config());
        let mut clients = Config::default_clients();
        if let Some((page, origin)) = web
            && let Some(c) = clients.iter_mut().find(|c| c.id == ocw_auth::CLIENT_WEB)
        {
            c.redirect_uris.push(page);
            c.origins.push(origin);
        }
        cfg.clients = clients;
        cfg.rate_limit.per_minute = 600;
        let service = Arc::new(AuthService::new(cfg)?);
        let s = service.clone();
        tokio::spawn(async move {
            let _ = crate::serve(s, listener).await;
        });
        Ok(Self {
            service,
            issuer,
            google,
            strava,
        })
    }

    /// Adds a client (tests).
    pub fn client(id: &str, loopback: bool, device: bool) -> ClientConfig {
        ClientConfig {
            id: id.into(),
            name: id.into(),
            redirect_uris: Vec::new(),
            loopback_path: loopback.then(|| ocw_auth::client::LOOPBACK_PATH.into()),
            device,
            origins: Vec::new(),
        }
    }
}

/// The user's browser in tests: follows redirects, keeps cookies, submits
/// forms. Synchronous (call it outside the Tokio runtime's threads).
#[derive(Default)]
pub struct TestBrowser {
    cookies: Mutex<HashMap<String, String>>,
    /// Every URL visited, in order.
    pub visited: Mutex<Vec<String>>,
}

/// A page the browser ended on.
#[derive(Debug, Clone)]
pub struct Page {
    pub url: String,
    pub status: u16,
    pub body: String,
}

impl TestBrowser {
    pub fn new() -> Self {
        Self::default()
    }

    fn agent() -> ureq::Agent {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .build()
            .into()
    }

    fn cookie_header(&self) -> String {
        self.cookies
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn keep_cookies(&self, headers: &http::HeaderMap) {
        for v in headers.get_all(header::SET_COOKIE) {
            if let Some((k, v)) = v
                .to_str()
                .ok()
                .and_then(|s| s.split(';').next())
                .and_then(|kv| kv.split_once('='))
            {
                self.cookies
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(k.trim().into(), v.trim().into());
            }
        }
    }

    fn resolve(base: &str, location: &str) -> String {
        if location.starts_with("http://") || location.starts_with("https://") {
            return location.into();
        }
        let origin_end = base.find("://").map_or(0, |i| i + 3);
        let host_end = base[origin_end..]
            .find('/')
            .map_or(base.len(), |i| origin_end + i);
        if location.starts_with('/') {
            format!("{}{location}", &base[..host_end])
        } else {
            let path_end = base
                .rfind('/')
                .filter(|&i| i >= host_end)
                .unwrap_or(base.len());
            format!("{}/{location}", &base[..path_end])
        }
    }

    fn send(
        &self,
        method: &str,
        url: &str,
        form: Option<&str>,
        origin: Option<&str>,
    ) -> Result<(u16, Option<String>, String), String> {
        self.visited
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(url.to_owned());
        let mut b = http::Request::builder().method(method).uri(url);
        let cookies = self.cookie_header();
        if !cookies.is_empty() {
            b = b.header(header::COOKIE, cookies);
        }
        if let Some(o) = origin {
            b = b.header(header::ORIGIN, o);
        }
        let body = form.unwrap_or_default().as_bytes().to_vec();
        if form.is_some() {
            b = b.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let req = b.body(body).map_err(|e| e.to_string())?;
        let resp = Self::agent().run(req).map_err(|e| format!("{url}: {e}"))?;
        let (parts, mut body) = resp.into_parts();
        self.keep_cookies(&parts.headers);
        let location = parts
            .headers
            .get(header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|l| Self::resolve(url, l));
        let text = body.read_to_string().unwrap_or_default();
        Ok((parts.status.as_u16(), location, text))
    }

    fn follow(
        &self,
        mut status: u16,
        mut location: Option<String>,
        mut url: String,
        mut body: String,
    ) -> Result<Page, String> {
        for _ in 0..20 {
            match (status, location.take()) {
                (301..=303 | 307 | 308, Some(next)) => {
                    let (s, l, b) = self.send("GET", &next, None, None)?;
                    (status, location, body, url) = (s, l, b, next);
                }
                _ => return Ok(Page { url, status, body }),
            }
        }
        Err("too many redirects".into())
    }

    /// Opens `url` and follows the redirects until one goes to a URL
    /// starting with `stop` (the app's redirect URI): returns that URL.
    pub fn open_until(&self, url: &str, stop: &str) -> Result<String, String> {
        let mut next = url.to_owned();
        for _ in 0..20 {
            let (status, location, body) = self.send("GET", &next, None, None)?;
            match location {
                Some(l) if (300..400).contains(&status) => {
                    if l.starts_with(stop) {
                        return Ok(l);
                    }
                    next = l;
                }
                _ => {
                    return Err(format!(
                        "ended on {next} ({status}): {}",
                        body.chars().take(300).collect::<String>()
                    ));
                }
            }
        }
        Err("too many redirects".into())
    }

    /// Opens `url` and follows the redirects.
    pub fn open(&self, url: &str) -> Result<Page, String> {
        let (s, l, b) = self.send("GET", url, None, None)?;
        self.follow(s, l, url.to_owned(), b)
    }

    /// Submits a form like the page would (with the page's origin).
    pub fn submit(&self, url: &str, pairs: &[(&str, &str)]) -> Result<Page, String> {
        let origin_end = url.find("://").map_or(0, |i| i + 3);
        let host_end = url[origin_end..]
            .find('/')
            .map_or(url.len(), |i| origin_end + i);
        let (s, l, b) = self.send(
            "POST",
            url,
            Some(&ocw_auth::client::form(pairs)),
            Some(&url[..host_end]),
        )?;
        self.follow(s, l, url.to_owned(), b)
    }

    /// The value of a hidden form field on a page.
    pub fn field(body: &str, name: &str) -> Option<String> {
        let needle = format!("name=\"{name}\" value=\"");
        let start = body.find(&needle)? + needle.len();
        let end = body[start..].find('"')? + start;
        Some(body[start..end].to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_locations() {
        assert_eq!(
            TestBrowser::resolve("http://a:1/device", "device/confirm"),
            "http://a:1/device/confirm"
        );
        assert_eq!(TestBrowser::resolve("http://a:1/x/y", "/z"), "http://a:1/z");
        assert_eq!(
            TestBrowser::resolve("http://a:1", "http://b/c"),
            "http://b/c"
        );
        assert_eq!(
            TestBrowser::field(r#"<input type="hidden" name="csrf" value="abc">"#, "csrf")
                .as_deref(),
            Some("abc")
        );
    }
}

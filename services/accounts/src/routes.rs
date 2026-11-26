//! The HTTP side (axum): routes onto [`AuthService`].
//!
//! Security on the wire (RFC 9700, RFC 8252, RFC 8628 §5): every answer
//! with a token is `no-store`; pages are never framed, cached or sent with
//! a referrer; the device confirmation checks a CSRF token (a `SameSite=
//! Strict` cookie against the form field) and the `Origin`; the token,
//! device and sign-in endpoints are rate-limited per client address;
//! repeated OAuth parameters are refused; nothing logs a query string, a
//! code or a token.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

use axum::Router;
use axum::body::Bytes;
use axum::extract::{ConnectInfo, Form, Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use ocw_auth::Provider;
use ocw_auth::oidc::ErrorResponse;

use crate::pages;
use crate::service::{AuthService, Authorize, Callback, OAuthError};
use crate::workshop::{ApiFail, Workshop};
use ocw_account_api::workshop::Endpoint;

type Svc = State<Arc<AuthService>>;
type Pairs = Vec<(String, String)>;

/// The router of every endpoint.
pub fn router(svc: Arc<AuthService>) -> Router {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/jwks", get(jwks))
        .route("/authorize", get(authorize))
        .route("/callback/{provider}", get(callback))
        .route("/token", post(token).options(preflight))
        .route("/revoke", post(revoke).options(preflight))
        .route("/device_authorization", post(device_authorization))
        .route("/device", get(device_page))
        .route("/device/confirm", post(device_confirm))
        .route("/userinfo", get(userinfo).post(userinfo).options(preflight))
        .route("/account/link", post(link).options(preflight))
        .route(
            "/account/providers/{provider}",
            delete(unlink).options(preflight),
        )
        .route("/account/export", get(export).options(preflight))
        .route("/account", delete(delete_account).options(preflight))
        .route("/workshop", get(workshop_state).options(preflight))
        .route("/workshop/ledger", get(workshop_ledger).options(preflight))
        .route(
            "/workshop/purchase",
            post(workshop_purchase).options(preflight),
        )
        .route("/workshop/sell", post(workshop_sell).options(preflight))
        .route("/workshop/garage", post(workshop_garage).options(preflight))
        .route("/rides/credit", post(rides_credit))
        .route("/rides/summary", post(rides_summary).options(preflight))
        .route("/health", get(|| async { "ok" }))
        .with_state(svc)
}

/// Serves the router on `listener` until the task is dropped.
pub async fn serve(
    svc: Arc<AuthService>,
    listener: tokio::net::TcpListener,
) -> std::io::Result<()> {
    axum::serve(
        listener,
        router(svc).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}

fn client_ip(svc: &AuthService, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
    let trusted = svc.config().rate_limit.trusted_proxy.as_deref();
    if trusted.is_some_and(|p| p.parse::<IpAddr>().ok() == Some(peer.ip()))
        && let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.rsplit(',').next())
            .and_then(|s| s.trim().parse().ok())
    {
        return ip;
    }
    peer.ip()
}

fn too_many() -> Response {
    json_error((
        429,
        ErrorResponse::new("slow_down", "too many requests; wait a minute"),
    ))
}

fn limited(
    svc: &AuthService,
    peer: SocketAddr,
    headers: &HeaderMap,
    endpoint: &'static str,
) -> bool {
    !svc.limiter
        .allow(client_ip(svc, peer, headers), endpoint, svc.now())
}

/// OAuth parameters must not repeat (RFC 6749 §3.1).
fn has_duplicates(pairs: &Pairs) -> bool {
    let mut keys: Vec<&str> = pairs.iter().map(|(k, _)| k.as_str()).collect();
    keys.sort_unstable();
    keys.windows(2).any(|w| w[0] == w[1])
}

fn no_store(mut r: Response) -> Response {
    let h = r.headers_mut();
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    r
}

fn json_ok<T: serde::Serialize>(v: &T) -> Response {
    no_store(axum::Json(v).into_response())
}

fn json_error((status, body): OAuthError) -> Response {
    let mut r = no_store(
        (
            StatusCode::from_u16(status).unwrap_or(StatusCode::BAD_REQUEST),
            axum::Json(body),
        )
            .into_response(),
    );
    if status == 401 {
        r.headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
    }
    r
}

fn html(status: StatusCode, body: String) -> Response {
    let mut r = (status, body).into_response();
    let h = r.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    h.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    h.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; frame-ancestors 'none'",
        ),
    );
    r
}

fn redirect(url: &str) -> Response {
    match HeaderValue::from_str(url) {
        Ok(v) => {
            let mut r = StatusCode::FOUND.into_response();
            r.headers_mut().insert(header::LOCATION, v);
            r.headers_mut().insert(
                header::REFERRER_POLICY,
                HeaderValue::from_static("no-referrer"),
            );
            no_store(r)
        }
        Err(_) => html(
            StatusCode::BAD_REQUEST,
            pages::message("Sign-in failed", "Bad redirect."),
        ),
    }
}

/// CORS for the browser app's calls (allowed origins of the clients).
fn cors(svc: &AuthService, headers: &HeaderMap, mut r: Response) -> Response {
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok())
        && svc.cors_origin_allowed(origin)
        && let Ok(v) = HeaderValue::from_str(origin)
    {
        let h = r.headers_mut();
        h.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, v);
        h.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    r
}

async fn preflight(State(svc): Svc, headers: HeaderMap) -> Response {
    let mut r = StatusCode::NO_CONTENT.into_response();
    let h = r.headers_mut();
    h.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, DELETE"),
    );
    h.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("authorization, content-type, idempotency-key"),
    );
    h.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("600"),
    );
    cors(&svc, &headers, r)
}

async fn discovery(State(svc): Svc) -> Response {
    axum::Json(svc.discovery()).into_response()
}

async fn jwks(State(svc): Svc) -> Response {
    let mut r = axum::Json(svc.jwks()).into_response();
    r.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    r
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
    tokio::task::spawn_blocking(f).await.ok()
}

async fn authorize(
    State(svc): Svc,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<Pairs>,
) -> Response {
    if limited(&svc, peer, &headers, "authorize") {
        return html(
            StatusCode::TOO_MANY_REQUESTS,
            pages::message("Slow down", "Too many sign-ins from here; wait a minute."),
        );
    }
    if has_duplicates(&q) {
        return html(
            StatusCode::BAD_REQUEST,
            pages::message("Sign-in failed", "Repeated parameters."),
        );
    }
    let s = svc.clone();
    match blocking(move || s.authorize(&q)).await {
        Some(Authorize::Choose { client, links }) => {
            html(StatusCode::OK, pages::choose(&client, &links))
        }
        Some(Authorize::Redirect(url)) => redirect(&url),
        Some(Authorize::Page(text)) => html(
            StatusCode::BAD_REQUEST,
            pages::message("Sign-in failed", &text),
        ),
        None => html(
            StatusCode::INTERNAL_SERVER_ERROR,
            pages::message("Sign-in failed", "Internal error."),
        ),
    }
}

async fn callback(
    State(svc): Svc,
    Path(provider): Path<String>,
    Query(q): Query<Pairs>,
) -> Response {
    let Some(provider) = Provider::parse(&provider) else {
        return html(
            StatusCode::NOT_FOUND,
            pages::message("Not found", "Unknown provider."),
        );
    };
    let s = svc.clone();
    match blocking(move || s.callback(provider, &q)).await {
        Some(Callback::Redirect(url)) => redirect(&url),
        Some(Callback::Page { title, text }) => html(StatusCode::OK, pages::message(&title, &text)),
        None => html(
            StatusCode::INTERNAL_SERVER_ERROR,
            pages::message("Sign-in failed", "Internal error."),
        ),
    }
}

async fn token(
    State(svc): Svc,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<Pairs>,
) -> Response {
    let r = if limited(&svc, peer, &headers, "token") {
        too_many()
    } else if has_duplicates(&form) {
        json_error((
            400,
            ErrorResponse::new("invalid_request", "repeated parameters"),
        ))
    } else {
        match svc.token(&form) {
            Ok(t) => json_ok(&t),
            Err(e) => json_error(e),
        }
    };
    cors(&svc, &headers, r)
}

async fn revoke(State(svc): Svc, headers: HeaderMap, Form(form): Form<Pairs>) -> Response {
    svc.revoke(&form);
    cors(&svc, &headers, no_store(StatusCode::OK.into_response()))
}

async fn device_authorization(
    State(svc): Svc,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<Pairs>,
) -> Response {
    if limited(&svc, peer, &headers, "device") {
        return too_many();
    }
    match svc.device_authorization(&form) {
        Ok(d) => json_ok(&d),
        Err(e) => json_error(e),
    }
}

fn csrf_cookie_name(svc: &AuthService) -> &'static str {
    if svc.issuer().starts_with("https://") {
        "__Host-ocw_device_csrf"
    } else {
        "ocw_device_csrf"
    }
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_owned())
}

fn origin_of(url: &str) -> &str {
    let after = url.find("://").map_or(0, |i| i + 3);
    match url[after..].find('/') {
        Some(i) => &url[..after + i],
        None => url,
    }
}

async fn device_page(
    State(svc): Svc,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<Pairs>,
) -> Response {
    let code = q
        .iter()
        .find(|(k, _)| k == "user_code")
        .map(|(_, v)| v.clone());
    let Some(code) = code else {
        return html(StatusCode::OK, pages::device_enter(None));
    };
    if limited(&svc, peer, &headers, "device-page") {
        return html(
            StatusCode::TOO_MANY_REQUESTS,
            pages::device_enter(Some("Too many tries; wait a minute.")),
        );
    }
    let (Some(normal), Some(client)) = (
        AuthService::normalize_user_code(&code),
        svc.device_lookup(&code),
    ) else {
        return html(
            StatusCode::OK,
            pages::device_enter(Some("This code is unknown or has expired.")),
        );
    };
    let csrf = crate::random_token();
    let mut r = html(
        StatusCode::OK,
        pages::device_confirm(&client, &normal, &csrf, &svc.providers()),
    );
    let secure = if svc.issuer().starts_with("https://") {
        "; Secure"
    } else {
        ""
    };
    if let Ok(v) = HeaderValue::from_str(&format!(
        "{}={csrf}; Path=/; HttpOnly; SameSite=Strict; Max-Age=900{secure}",
        csrf_cookie_name(&svc)
    )) {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

async fn device_confirm(
    State(svc): Svc,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<Pairs>,
) -> Response {
    let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    if limited(&svc, peer, &headers, "device-page") {
        return html(
            StatusCode::TOO_MANY_REQUESTS,
            pages::device_enter(Some("Too many tries; wait a minute.")),
        );
    }
    // CSRF: the form's token must be the cookie's (SameSite=Strict), and
    // a browser's Origin must be ours.
    let origin_ok = headers
        .get(header::ORIGIN)
        .and_then(|o| o.to_str().ok())
        .is_none_or(|o| o == origin_of(svc.issuer()));
    let token_ok = match (cookie(&headers, csrf_cookie_name(&svc)), get("csrf")) {
        (Some(c), Some(f)) => !c.is_empty() && ocw_auth::constant_time_eq(&c, f),
        _ => false,
    };
    if !origin_ok || !token_ok {
        return html(
            StatusCode::FORBIDDEN,
            pages::message(
                "Not confirmed",
                "The page expired. Open the device link again.",
            ),
        );
    }
    let code = get("user_code").unwrap_or_default().to_owned();
    match get("action") {
        Some("deny") => {
            svc.device_deny(&code);
            html(
                StatusCode::OK,
                pages::message("Cancelled", "The device was not signed in."),
            )
        }
        Some(p) => {
            let Some(provider) = Provider::parse(p) else {
                return html(
                    StatusCode::BAD_REQUEST,
                    pages::message("Not confirmed", "Unknown provider."),
                );
            };
            let s = svc.clone();
            match blocking(move || s.device_start(&code, provider)).await {
                Some(Ok(url)) => {
                    let mut r = redirect(&url);
                    *r.status_mut() = StatusCode::SEE_OTHER;
                    r
                }
                Some(Err(e)) => html(StatusCode::OK, pages::device_enter(Some(&e))),
                None => html(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    pages::message("Sign-in failed", "Internal error."),
                ),
            }
        }
        None => html(
            StatusCode::BAD_REQUEST,
            pages::message("Not confirmed", "No choice made."),
        ),
    }
}

fn auth_header(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
}

async fn userinfo(State(svc): Svc, headers: HeaderMap) -> Response {
    let r = match svc.userinfo(auth_header(&headers)) {
        Ok(u) => json_ok(&u),
        Err(e) => json_error(e),
    };
    cors(&svc, &headers, r)
}

async fn link(State(svc): Svc, headers: HeaderMap, Form(form): Form<Pairs>) -> Response {
    let r = match svc.link_ticket(auth_header(&headers), &form) {
        Ok(t) => json_ok(&t),
        Err(e) => json_error(e),
    };
    cors(&svc, &headers, r)
}

async fn unlink(State(svc): Svc, headers: HeaderMap, Path(provider): Path<String>) -> Response {
    let r = match Provider::parse(&provider) {
        None => json_error((404, ErrorResponse::new("not_found", "unknown provider"))),
        Some(p) => match svc.unlink(auth_header(&headers), p) {
            Ok(u) => json_ok(&u),
            Err(e) => json_error(e),
        },
    };
    cors(&svc, &headers, r)
}

async fn export(State(svc): Svc, headers: HeaderMap) -> Response {
    let r = match svc.export(auth_header(&headers)) {
        Ok(v) => {
            let mut r = json_ok(&v);
            r.headers_mut().insert(
                header::CONTENT_DISPOSITION,
                HeaderValue::from_static("attachment; filename=\"ocw-account.json\""),
            );
            r
        }
        Err(e) => json_error(e),
    };
    cors(&svc, &headers, r)
}

async fn delete_account(State(svc): Svc, headers: HeaderMap) -> Response {
    let r = match svc.delete_account(auth_header(&headers)) {
        Ok(()) => no_store(StatusCode::NO_CONTENT.into_response()),
        Err(e) => json_error(e),
    };
    cors(&svc, &headers, r)
}

// ---- workshop and rides (spec 14 Phase 4, 4O) ---------------------------

fn api<T: serde::Serialize>(r: Option<Result<T, ApiFail>>) -> Response {
    match r {
        Some(Ok(v)) => json_ok(&v),
        Some(Err(e)) => json_error(e.into_oauth()),
        None => json_error((500, ErrorResponse::new("server_error", "internal error"))),
    }
}

fn owned_auth(headers: &HeaderMap) -> Option<String> {
    auth_header(headers).map(str::to_owned)
}

async fn workshop_state(State(svc): Svc, headers: HeaderMap) -> Response {
    let a = owned_auth(&headers);
    let s = svc.clone();
    let r = blocking(move || Workshop::new(s).state(a.as_deref())).await;
    cors(&svc, &headers, api(r))
}

async fn workshop_ledger(State(svc): Svc, headers: HeaderMap) -> Response {
    let a = owned_auth(&headers);
    let s = svc.clone();
    let r = blocking(move || Workshop::new(s).ledger(a.as_deref())).await;
    cors(&svc, &headers, api(r))
}

async fn workshop_action(
    svc: Arc<AuthService>,
    headers: HeaderMap,
    endpoint: Endpoint,
    body: Bytes,
) -> Response {
    let a = owned_auth(&headers);
    let key = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let s = svc.clone();
    let r =
        blocking(move || Workshop::new(s).act(a.as_deref(), endpoint, key.as_deref(), &body)).await;
    cors(&svc, &headers, api(r))
}

async fn workshop_purchase(State(svc): Svc, headers: HeaderMap, body: Bytes) -> Response {
    workshop_action(svc, headers, Endpoint::Purchase, body).await
}

async fn workshop_sell(State(svc): Svc, headers: HeaderMap, body: Bytes) -> Response {
    workshop_action(svc, headers, Endpoint::Sell, body).await
}

async fn workshop_garage(State(svc): Svc, headers: HeaderMap, body: Bytes) -> Response {
    workshop_action(svc, headers, Endpoint::Garage, body).await
}

async fn rides_credit(State(svc): Svc, body: Bytes) -> Response {
    let s = svc.clone();
    api(blocking(move || Workshop::new(s).credit(&body)).await)
}

async fn rides_summary(State(svc): Svc, headers: HeaderMap, body: Bytes) -> Response {
    let a = owned_auth(&headers);
    let s = svc.clone();
    let r = blocking(move || Workshop::new(s).summary(a.as_deref(), &body)).await;
    cors(&svc, &headers, api(r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(origin_of("https://id.example/auth"), "https://id.example");
        assert_eq!(origin_of("http://127.0.0.1:8480"), "http://127.0.0.1:8480");
        let dup: Pairs = vec![("a".into(), "1".into()), ("a".into(), "2".into())];
        assert!(has_duplicates(&dup));
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("x=1; ocw_device_csrf=abc"),
        );
        assert_eq!(cookie(&h, "ocw_device_csrf").as_deref(), Some("abc"));
        assert_eq!(cookie(&h, "y"), None);
    }
}

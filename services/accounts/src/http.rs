//! The HTTP client of the provider calls: ureq (rustls with *ring*, as
//! the rest of the server) for `openidconnect` and `oauth2`, whose own
//! reqwest client is left out. Synchronous: the service runs provider
//! calls on Tokio's blocking pool. No redirects are followed (RFC 9700
//! §4.11: a token endpoint never redirects), responses are capped.

use std::time::Duration;

/// Biggest provider response read (discovery, JWKS, tokens).
const MAX_BODY: u64 = 1 << 20;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct HttpError(pub String);

/// Response headers a caller wants to see (Strava's rate limits).
pub type HeaderHook<'a> = &'a (dyn Fn(&http::HeaderMap) + Sync);

/// One request.
pub fn call(
    req: oauth2::HttpRequest,
    on_headers: Option<HeaderHook<'_>>,
) -> Result<oauth2::HttpResponse, HttpError> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent("open-cycling-world-accounts")
        .build()
        .into();
    let resp = agent.run(req).map_err(|e| HttpError(e.to_string()))?;
    let (parts, mut body) = resp.into_parts();
    if let Some(h) = on_headers {
        h(&parts.headers);
    }
    let bytes = body
        .with_config()
        .limit(MAX_BODY)
        .read_to_vec()
        .map_err(|e| HttpError(e.to_string()))?;
    Ok(http::Response::from_parts(parts, bytes))
}

/// A plain client for `openidconnect` / `oauth2` (`SyncHttpClient`).
pub fn client(req: oauth2::HttpRequest) -> Result<oauth2::HttpResponse, HttpError> {
    call(req, None)
}

//! Strava HTTP API: request building and response parsing, behind the
//! [`Http`] trait (the real client is `ureq`; tests use a mock, no network).

use std::fmt;
use std::time::Duration;

use serde::Deserialize;

pub const AUTHORIZE_URL: &str = "https://www.strava.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://www.strava.com/oauth/token";
pub const UPLOADS_URL: &str = "https://www.strava.com/api/v3/uploads";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: Method,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

#[cfg(test)]
impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Without the headers and body: they hold tokens and secrets.
impl fmt::Debug for HttpRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:?} {} ({} bytes)",
            self.method,
            self.url,
            self.body.len()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpResponse {
    pub status: u16,
    pub body: Vec<u8>,
}

/// A blocking HTTP client. Transport errors (no network, TLS, timeout) are
/// `Err`; every HTTP status is an `Ok` response.
pub trait Http: Send + Sync {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String>;
}

/// The real client: `ureq` with rustls.
pub struct UreqHttp(ureq::Agent);

impl Default for UreqHttp {
    fn default() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(120)))
            .http_status_as_error(false)
            .user_agent(concat!("OpenCyclingWorld/", env!("CARGO_PKG_VERSION")))
            .build()
            .into();
        Self(agent)
    }
}

impl Http for UreqHttp {
    fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
        let result = match req.method {
            Method::Get => {
                let mut r = self.0.get(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.call()
            }
            Method::Post => {
                let mut r = self.0.post(&req.url);
                for (k, v) in &req.headers {
                    r = r.header(k, v);
                }
                r.send(&req.body[..])
            }
        };
        let resp = result.map_err(|e| e.to_string())?;
        let status = resp.status().as_u16();
        let body = resp
            .into_body()
            .with_config()
            .limit(1 << 20)
            .read_to_vec()
            .map_err(|e| e.to_string())?;
        Ok(HttpResponse { status, body })
    }
}

/// How a request failed, which decides what happens next.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401: the token is invalid or revoked; reconnect (`strava.connect`).
    Unauthorized,
    /// 429: rate limit; retry later.
    RateLimited,
    /// Network problem or server error; retry later.
    Transient(String),
    /// The request itself is wrong (e.g. a broken file); do not retry.
    Permanent(String),
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unauthorized => write!(f, "not authorized (run strava.connect)"),
            Self::RateLimited => write!(f, "Strava rate limit reached"),
            Self::Transient(e) => write!(f, "{e}"),
            Self::Permanent(e) => write!(f, "{e}"),
        }
    }
}

impl ApiError {
    pub fn is_retryable(&self) -> bool {
        matches!(self, Self::RateLimited | Self::Transient(_))
    }
}

/// Sends `req` and maps transport errors and error statuses to [`ApiError`].
pub fn call(http: &dyn Http, req: &HttpRequest) -> Result<HttpResponse, ApiError> {
    let resp = http.send(req).map_err(ApiError::Transient)?;
    match resp.status {
        200..=299 => Ok(resp),
        401 => Err(ApiError::Unauthorized),
        429 => Err(ApiError::RateLimited),
        s @ 500..=599 => Err(ApiError::Transient(format!("Strava server error {s}"))),
        s => Err(ApiError::Permanent(format!(
            "Strava error {s}: {}",
            error_message(&resp.body)
        ))),
    }
}

/// The `message` of a Strava error body (never the whole body: it could
/// echo request data).
fn error_message(body: &[u8]) -> String {
    #[derive(Deserialize)]
    struct Fault {
        message: Option<String>,
    }
    serde_json::from_slice::<Fault>(body)
        .ok()
        .and_then(|f| f.message)
        .unwrap_or_else(|| "unexpected response".into())
}

/// `application/x-www-form-urlencoded` / URL query encoding.
pub fn form_encode(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = b
                    .get(i + 1..i + 3)
                    .filter(|h| h.iter().all(u8::is_ascii_hexdigit))
                    .and_then(|h| std::str::from_utf8(h).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok());
                match hex {
                    Some(v) => {
                        out.push(v);
                        i += 2;
                    }
                    None => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn form_post(url: &str, pairs: &[(&str, &str)]) -> HttpRequest {
    HttpRequest {
        method: Method::Post,
        url: url.into(),
        headers: vec![(
            "Content-Type".into(),
            "application/x-www-form-urlencoded".into(),
        )],
        body: form_encode(pairs).into_bytes(),
    }
}

/// Exchanges an authorization code for tokens.
pub fn token_exchange_request(client_id: &str, client_secret: &str, code: &str) -> HttpRequest {
    form_post(
        TOKEN_URL,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("grant_type", "authorization_code"),
        ],
    )
}

/// Gets a new access token with the refresh token.
pub fn token_refresh_request(
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> HttpRequest {
    form_post(
        TOKEN_URL,
        &[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
        ],
    )
}

/// Token endpoint answer (exchange and refresh).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub expires_at: i64,
    #[serde(default)]
    pub athlete: Option<Athlete>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Athlete {
    #[serde(default)]
    pub firstname: Option<String>,
    #[serde(default)]
    pub lastname: Option<String>,
}

pub fn parse_token(resp: &HttpResponse) -> Result<TokenResponse, ApiError> {
    serde_json::from_slice(&resp.body)
        .map_err(|e| ApiError::Transient(format!("bad token response: {e}")))
}

/// What an upload ride carries besides the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UploadMeta {
    pub name: String,
    pub description: String,
    /// Our file name; Strava shows it and rejects reuse.
    pub external_id: String,
}

/// `POST /uploads` (multipart): a FIT file as a virtual trainer ride.
pub fn upload_request(
    access_token: &str,
    file_name: &str,
    fit: &[u8],
    meta: &UploadMeta,
    boundary: &str,
) -> HttpRequest {
    let mut body = Vec::with_capacity(fit.len() + 1024);
    let mut text = |name: &str, value: &str| {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    };
    text("data_type", "fit");
    text("activity_type", "virtualride");
    text("sport_type", "VirtualRide");
    text("trainer", "1");
    text("name", &meta.name);
    text("description", &meta.description);
    text("external_id", &meta.external_id);
    let safe_name: String = file_name
        .chars()
        .map(|c| if c == '"' || c.is_control() { '_' } else { c })
        .collect();
    body.extend_from_slice(
        format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{safe_name}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(fit);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    HttpRequest {
        method: Method::Post,
        url: UPLOADS_URL.into(),
        headers: vec![
            ("Authorization".into(), format!("Bearer {access_token}")),
            (
                "Content-Type".into(),
                format!("multipart/form-data; boundary={boundary}"),
            ),
        ],
        body,
    }
}

/// `GET /uploads/{id}`.
pub fn upload_status_request(access_token: &str, upload_id: u64) -> HttpRequest {
    HttpRequest {
        method: Method::Get,
        url: format!("{UPLOADS_URL}/{upload_id}"),
        headers: vec![("Authorization".into(), format!("Bearer {access_token}"))],
        body: Vec::new(),
    }
}

/// State of an upload as Strava reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadStatus {
    /// Still being processed.
    Processing { upload_id: u64 },
    /// Done: the ride is an activity now.
    Ready { activity_id: u64 },
    /// Strava already has this ride (counts as uploaded).
    Duplicate { activity_id: Option<u64> },
    /// Strava rejected the file.
    Failed { error: String },
}

pub fn parse_upload(resp: &HttpResponse) -> Result<UploadStatus, ApiError> {
    #[derive(Deserialize)]
    struct Upload {
        id: Option<u64>,
        error: Option<String>,
        activity_id: Option<u64>,
    }
    let u: Upload = serde_json::from_slice(&resp.body)
        .map_err(|e| ApiError::Transient(format!("bad upload response: {e}")))?;
    Ok(match (u.activity_id, u.error) {
        (Some(activity_id), _) => UploadStatus::Ready { activity_id },
        (None, Some(error)) if error.contains("duplicate of") => UploadStatus::Duplicate {
            activity_id: duplicate_activity_id(&error),
        },
        (None, Some(error)) if !error.is_empty() => UploadStatus::Failed { error },
        _ => match u.id {
            Some(upload_id) => UploadStatus::Processing { upload_id },
            None => return Err(ApiError::Transient("upload without id".into())),
        },
    })
}

/// "... duplicate of <a href='/activities/123'>123</a>" → 123.
fn duplicate_activity_id(error: &str) -> Option<u64> {
    let rest = &error[error.find("duplicate of")? + "duplicate of".len()..];
    let digits: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

#[cfg(test)]
pub mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;

    use super::*;

    /// Answers requests from a queue and remembers them.
    #[derive(Default)]
    pub struct MockHttp {
        pub responses: Mutex<VecDeque<Result<HttpResponse, String>>>,
        pub requests: Mutex<Vec<HttpRequest>>,
    }

    impl MockHttp {
        pub fn reply(&self, status: u16, body: &str) -> &Self {
            self.responses.lock().unwrap().push_back(Ok(HttpResponse {
                status,
                body: body.as_bytes().to_vec(),
            }));
            self
        }
        pub fn fail(&self, error: &str) -> &Self {
            self.responses
                .lock()
                .unwrap()
                .push_back(Err(error.to_string()));
            self
        }
        pub fn urls(&self) -> Vec<String> {
            self.requests
                .lock()
                .unwrap()
                .iter()
                .map(|r| r.url.clone())
                .collect()
        }
    }

    impl Http for MockHttp {
        fn send(&self, req: &HttpRequest) -> Result<HttpResponse, String> {
            self.requests.lock().unwrap().push(req.clone());
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err("mock: no response queued".into()))
        }
    }

    #[test]
    fn form_and_percent_encoding() {
        assert_eq!(
            form_encode(&[("a", "b c"), ("scope", "read,activity:write")]),
            "a=b%20c&scope=read%2Cactivity%3Awrite"
        );
        assert_eq!(percent_decode("a%20b+c%3A"), "a b c:");
        assert_eq!(percent_decode("bad%zz%4"), "bad%zz%4");
        assert_eq!(percent_decode(&percent_encode("ä/?&=")), "ä/?&=");
    }

    #[test]
    fn token_requests() {
        let r = token_refresh_request("123", "s3cret", "refresh-1");
        assert_eq!(r.method, Method::Post);
        assert_eq!(r.url, TOKEN_URL);
        let body = String::from_utf8(r.body.clone()).unwrap();
        assert_eq!(
            body,
            "client_id=123&client_secret=s3cret&grant_type=refresh_token&refresh_token=refresh-1"
        );
        // Debug output never shows the secret.
        assert!(!format!("{r:?}").contains("s3cret"));
        let r = token_exchange_request("123", "s3cret", "abc");
        assert!(
            String::from_utf8(r.body)
                .unwrap()
                .contains("code=abc&grant_type=authorization_code")
        );
    }

    #[test]
    fn upload_request_is_multipart_fit_virtual_ride() {
        let meta = UploadMeta {
            name: "Open Cycling World: Loop".into(),
            description: "Virtual ride".into(),
            external_id: "2026-11-11_12-00-00.fit".into(),
        };
        let fit = [0x0E, 0x20, 0x00, 0xFF];
        let r = upload_request("tok", "2026-11-11_12-00-00.fit", &fit, &meta, "XyZ");
        assert_eq!(r.url, "https://www.strava.com/api/v3/uploads");
        assert_eq!(r.header("authorization"), Some("Bearer tok"));
        assert_eq!(
            r.header("content-type"),
            Some("multipart/form-data; boundary=XyZ")
        );
        let body = String::from_utf8_lossy(&r.body);
        for (name, value) in [
            ("data_type", "fit"),
            ("activity_type", "virtualride"),
            ("sport_type", "VirtualRide"),
            ("trainer", "1"),
            ("name", "Open Cycling World: Loop"),
            ("external_id", "2026-11-11_12-00-00.fit"),
        ] {
            assert!(
                body.contains(&format!(
                    "--XyZ\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )),
                "{name} missing in {body}"
            );
        }
        assert!(body.contains("name=\"file\"; filename=\"2026-11-11_12-00-00.fit\""));
        // The file bytes are embedded unchanged, then the closing boundary.
        let pos = r.body.windows(4).position(|w| w == fit).unwrap();
        assert_eq!(&r.body[pos + 4..], b"\r\n--XyZ--\r\n");
    }

    #[test]
    fn upload_status_parsing() {
        let resp = |body: &str| HttpResponse {
            status: 200,
            body: body.into(),
        };
        assert_eq!(
            parse_upload(&resp(
                r#"{"id":7,"id_str":"7","status":"Your activity is still being processed.","error":null,"activity_id":null}"#
            )),
            Ok(UploadStatus::Processing { upload_id: 7 })
        );
        assert_eq!(
            parse_upload(&resp(
                r#"{"id":7,"status":"Your activity is ready.","error":null,"activity_id":42}"#
            )),
            Ok(UploadStatus::Ready { activity_id: 42 })
        );
        assert_eq!(
            parse_upload(&resp(
                r#"{"id":7,"error":"2026.fit duplicate of <a href='/activities/99'>99</a>","activity_id":null}"#
            )),
            Ok(UploadStatus::Duplicate {
                activity_id: Some(99)
            })
        );
        assert_eq!(
            parse_upload(&resp(
                r#"{"id":7,"error":"Improperly formatted data.","activity_id":null}"#
            )),
            Ok(UploadStatus::Failed {
                error: "Improperly formatted data.".into()
            })
        );
        assert!(parse_upload(&resp("<html>")).is_err());
    }

    #[test]
    fn statuses_map_to_errors() {
        let http = MockHttp::default();
        http.reply(401, "{}")
            .reply(429, "{}")
            .reply(503, "")
            .reply(400, r#"{"message":"Bad Request","errors":[]}"#)
            .fail("connection refused")
            .reply(201, "{}");
        let req = upload_status_request("t", 1);
        assert_eq!(call(&http, &req), Err(ApiError::Unauthorized));
        assert_eq!(call(&http, &req), Err(ApiError::RateLimited));
        assert!(matches!(call(&http, &req), Err(ApiError::Transient(_))));
        assert_eq!(
            call(&http, &req),
            Err(ApiError::Permanent("Strava error 400: Bad Request".into()))
        );
        let e = call(&http, &req).unwrap_err();
        assert!(e.is_retryable(), "{e:?}");
        assert_eq!(call(&http, &req).unwrap().status, 201);
        assert_eq!(http.urls()[0], "https://www.strava.com/api/v3/uploads/1");
    }
}

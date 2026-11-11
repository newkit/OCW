//! Strava OAuth 2 (authorization code with a localhost redirect), the app
//! credentials and the stored tokens.

use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use super::api::{
    self, AUTHORIZE_URL, ApiError, Http, form_encode, parse_token, percent_decode,
    token_exchange_request, token_refresh_request,
};

pub const ENV_CLIENT_ID: &str = "OCW_STRAVA_CLIENT_ID";
pub const ENV_CLIENT_SECRET: &str = "OCW_STRAVA_CLIENT_SECRET";
/// Scopes: uploading needs `activity:write`.
pub const SCOPE: &str = "read,activity:write";
/// Refresh this long before the access token expires.
pub const REFRESH_MARGIN_S: i64 = 300;

/// The Strava API application (client id and secret), from the environment
/// or `<config dir>/ocw/strava.toml`.
#[derive(Clone, PartialEq, Eq, Deserialize)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
}

impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Credentials {{ client_id: {}, client_secret: *** }}",
            self.client_id
        )
    }
}

/// `strava.toml`: the credentials (unless in the environment) and options.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct StravaConfig {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    /// Upload every saved ride automatically (default true).
    #[serde(default)]
    pub auto_upload: Option<bool>,
}

impl StravaConfig {
    /// Reads the file; a missing file is an empty configuration.
    pub fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read_to_string(path) {
            Ok(s) => toml::from_str(&s).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Credentials: the environment variables win over the file. `None`
    /// means not configured (the feature stays off).
    pub fn credentials(&self, env: impl Fn(&str) -> Option<String>) -> Option<Credentials> {
        let pick = |var: &str, file: &Option<String>| {
            env(var)
                .or_else(|| file.clone())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        };
        Some(Credentials {
            client_id: pick(ENV_CLIENT_ID, &self.client_id)?,
            client_secret: pick(ENV_CLIENT_SECRET, &self.client_secret)?,
        })
    }
}

/// Stored tokens of the linked athlete.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Token {
    pub access_token: String,
    pub refresh_token: String,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub athlete: Option<String>,
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Token {{ expires_at: {}, athlete: {:?}, tokens: *** }}",
            self.expires_at, self.athlete
        )
    }
}

impl Token {
    pub fn needs_refresh(&self, now_unix_s: i64) -> bool {
        now_unix_s + REFRESH_MARGIN_S >= self.expires_at
    }

    fn from_response(r: api::TokenResponse, previous_athlete: Option<String>) -> Self {
        let athlete = r
            .athlete
            .map(|a| {
                [a.firstname, a.lastname]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .filter(|s| !s.is_empty())
            .or(previous_athlete);
        Self {
            access_token: r.access_token,
            refresh_token: r.refresh_token,
            expires_at: r.expires_at,
            athlete,
        }
    }
}

/// The token file (`<config dir>/ocw/strava-token.toml`, mode 0600).
#[derive(Debug, Clone)]
pub struct TokenStore {
    pub path: PathBuf,
}

impl TokenStore {
    pub fn load(&self) -> Result<Option<Token>, String> {
        match std::fs::read_to_string(&self.path) {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| format!("{}: {e}", self.path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("{}: {e}", self.path.display())),
        }
    }

    /// Writes the tokens readable only by the user (through a temporary
    /// file, so a crash never leaves a broken token file).
    pub fn save(&self, token: &Token) -> Result<(), String> {
        let err = |e: std::io::Error| format!("{}: {e}", self.path.display());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(err)?;
        }
        let text = toml::to_string_pretty(token).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("toml.part");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(err)?;
        #[cfg(unix)]
        {
            // The mode only applies to new files.
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(std::fs::Permissions::from_mode(0o600))
                .map_err(err)?;
        }
        f.write_all(text.as_bytes()).map_err(err)?;
        f.sync_all().map_err(err)?;
        drop(f);
        std::fs::rename(&tmp, &self.path).map_err(err)
    }

    pub fn delete(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(format!("{}: {e}", self.path.display()))
            }
            _ => Ok(()),
        }
    }
}

/// A valid access token: the stored one, refreshed (and saved) when it is
/// about to expire. `Ok(None)`: not connected.
pub fn valid_token(
    http: &dyn Http,
    creds: &Credentials,
    store: &TokenStore,
    now_unix_s: i64,
) -> Result<Option<Token>, ApiError> {
    let Some(token) = store.load().map_err(ApiError::Permanent)? else {
        return Ok(None);
    };
    if !token.needs_refresh(now_unix_s) {
        return Ok(Some(token));
    }
    let req = token_refresh_request(&creds.client_id, &creds.client_secret, &token.refresh_token);
    let resp = match api::call(http, &req) {
        Ok(r) => r,
        // A refused refresh token (revoked access, 400/401) needs a new login.
        Err(ApiError::Permanent(_)) => return Err(ApiError::Unauthorized),
        Err(e) => return Err(e),
    };
    let new = Token::from_response(parse_token(&resp)?, token.athlete);
    store.save(&new).map_err(ApiError::Permanent)?;
    Ok(Some(new))
}

/// Exchanges the authorization code and saves the tokens.
pub fn exchange_code(
    http: &dyn Http,
    creds: &Credentials,
    store: &TokenStore,
    code: &str,
) -> Result<Token, ApiError> {
    let req = token_exchange_request(&creds.client_id, &creds.client_secret, code);
    let token = Token::from_response(parse_token(&api::call(http, &req)?)?, None);
    store.save(&token).map_err(ApiError::Permanent)?;
    Ok(token)
}

/// The page the user approves the link on.
pub fn authorize_url(client_id: &str, redirect_uri: &str, state: &str) -> String {
    format!(
        "{AUTHORIZE_URL}?{}",
        form_encode(&[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("response_type", "code"),
            ("approval_prompt", "auto"),
            ("scope", SCOPE),
            ("state", state),
        ])
    )
}

/// An unguessable `state` value (CSRF protection of the redirect).
pub fn random_state() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..2u64 {
        // RandomState is seeded from the OS's random source.
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out
}

/// What the redirect to our localhost listener brought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    Code(String),
    /// The user declined, or Strava reported an error.
    Denied(String),
    /// Not the callback (e.g. `/favicon.ico`) or a wrong `state`.
    Ignore,
}

/// Parses the request line `GET /strava/callback?state=..&code=..&scope=.. HTTP/1.1`.
pub fn parse_callback(request_line: &str, expected_state: &str) -> Callback {
    let mut parts = request_line.split_whitespace();
    let (Some("GET"), Some(target)) = (parts.next(), parts.next()) else {
        return Callback::Ignore;
    };
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    if path != CALLBACK_PATH {
        return Callback::Ignore;
    }
    let mut code = None;
    let mut state = None;
    let mut error = None;
    let mut scope = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let v = percent_decode(v);
        match k {
            "code" => code = Some(v),
            "state" => state = Some(v),
            "error" => error = Some(v),
            "scope" => scope = Some(v),
            _ => {}
        }
    }
    if state.as_deref() != Some(expected_state) {
        return Callback::Ignore;
    }
    if let Some(e) = error {
        return Callback::Denied(e);
    }
    // The user can untick the upload permission on Strava's page.
    if scope.is_some_and(|s| !s.split(',').any(|s| s == "activity:write")) {
        return Callback::Denied("upload permission (activity:write) not granted".into());
    }
    match code {
        Some(c) if !c.is_empty() => Callback::Code(c),
        _ => Callback::Denied("no authorization code".into()),
    }
}

pub const CALLBACK_PATH: &str = "/strava/callback";

/// Listens on a free localhost port for the redirect.
pub fn listen() -> std::io::Result<(TcpListener, String)> {
    // Strava allows "localhost" and "127.0.0.1" as callback domains.
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    Ok((listener, format!("http://127.0.0.1:{port}{CALLBACK_PATH}")))
}

/// Waits for the browser's redirect and answers it with a short page.
pub fn wait_for_callback(
    listener: &TcpListener,
    state: &str,
    timeout: Duration,
) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let (stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
            Err(e) => return Err(e.to_string()),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        let mut reader = BufReader::new(&stream);
        if reader.read_line(&mut line).is_err() {
            continue;
        }
        let result = parse_callback(&line, state);
        let (status, text) = match &result {
            Callback::Code(_) => (
                "200 OK",
                "Strava is linked with Open Cycling World. You can close this tab.",
            ),
            Callback::Denied(_) => (
                "200 OK",
                "Strava was not linked. You can close this tab and try again with strava.connect.",
            ),
            Callback::Ignore => ("404 Not Found", "Not found."),
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>Open Cycling World</title><p>{text}</p>"
        );
        let mut stream = &stream;
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        match result {
            Callback::Code(c) => return Ok(c),
            Callback::Denied(e) => return Err(format!("not linked: {e}")),
            Callback::Ignore => {}
        }
    }
    Err("timed out waiting for the browser".into())
}

/// Opens `url` in the default browser.
pub fn open_browser(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut cmd = {
        // `start` treats the first quoted argument as the window title.
        let mut c = std::process::Command::new("cmd");
        c.args(["/C", "start", ""]).arg(url);
        c
    };
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = std::process::Command::new("open");
        c.arg(url);
        c
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut cmd = {
        let mut c = std::process::Command::new("xdg-open");
        c.arg(url);
        c
    };
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot open a browser: {e}"))
}

#[cfg(test)]
mod tests {
    use super::super::api::tests::MockHttp;
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-strava-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn creds() -> Credentials {
        Credentials {
            client_id: "123".into(),
            client_secret: "s3cret".into(),
        }
    }

    fn token(expires_at: i64) -> Token {
        Token {
            access_token: "old-access".into(),
            refresh_token: "old-refresh".into(),
            expires_at,
            athlete: Some("Eddy Merckx".into()),
        }
    }

    #[test]
    fn credentials_from_env_or_file() {
        let none = |_: &str| None;
        assert_eq!(StravaConfig::default().credentials(none), None);
        let file = StravaConfig {
            client_id: Some("1".into()),
            client_secret: Some("file-secret".into()),
            auto_upload: None,
        };
        assert_eq!(
            file.credentials(none),
            Some(Credentials {
                client_id: "1".into(),
                client_secret: "file-secret".into()
            })
        );
        let env = |k: &str| (k == ENV_CLIENT_SECRET).then(|| "env-secret".to_string());
        assert_eq!(file.credentials(env).unwrap().client_secret, "env-secret");
        // An id alone is not a configuration.
        let half = StravaConfig {
            client_id: Some("1".into()),
            ..StravaConfig::default()
        };
        assert_eq!(half.credentials(none), None);
        // Secrets never appear in debug output.
        assert!(!format!("{:?}", file.credentials(none)).contains("file-secret"));
        assert!(!format!("{:?}", token(0)).contains("old-"));
        let parsed: StravaConfig =
            toml::from_str("client_id = \"9\"\nclient_secret = \"x\"\nauto_upload = false\n")
                .unwrap();
        assert_eq!(parsed.auto_upload, Some(false));
    }

    #[test]
    fn token_file_round_trip_is_private() {
        let dir = temp_dir("token");
        let store = TokenStore {
            path: dir.join("nested/strava-token.toml"),
        };
        assert_eq!(store.load(), Ok(None));
        store.save(&token(1000)).unwrap();
        assert_eq!(store.load(), Ok(Some(token(1000))));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&store.path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.delete().unwrap();
        store.delete().unwrap();
        assert_eq!(store.load(), Ok(None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fresh_token_is_used_without_refresh() {
        let dir = temp_dir("fresh");
        let store = TokenStore {
            path: dir.join("t.toml"),
        };
        let http = MockHttp::default();
        assert_eq!(valid_token(&http, &creds(), &store, 0), Ok(None));
        store.save(&token(10_000)).unwrap();
        assert_eq!(
            valid_token(&http, &creds(), &store, 10_000 - REFRESH_MARGIN_S - 1),
            Ok(Some(token(10_000)))
        );
        assert!(http.urls().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn expiring_token_is_refreshed_and_saved() {
        let dir = temp_dir("refresh");
        let store = TokenStore {
            path: dir.join("t.toml"),
        };
        store.save(&token(10_000)).unwrap();
        let http = MockHttp::default();
        http.reply(
            200,
            r#"{"token_type":"Bearer","access_token":"new-access","refresh_token":"new-refresh","expires_at":30000,"expires_in":21600}"#,
        );
        let t = valid_token(&http, &creds(), &store, 9_800)
            .unwrap()
            .unwrap();
        assert_eq!(t.access_token, "new-access");
        assert_eq!(t.refresh_token, "new-refresh");
        assert_eq!(t.expires_at, 30_000);
        assert_eq!(t.athlete.as_deref(), Some("Eddy Merckx"), "kept");
        assert_eq!(store.load().unwrap(), Some(t));
        let req = &http.requests.lock().unwrap()[0];
        assert_eq!(req.url, api::TOKEN_URL);
        assert!(
            String::from_utf8_lossy(&req.body)
                .contains("grant_type=refresh_token&refresh_token=old-refresh")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn refused_or_failed_refresh() {
        let dir = temp_dir("refused");
        let store = TokenStore {
            path: dir.join("t.toml"),
        };
        store.save(&token(0)).unwrap();
        let http = MockHttp::default();
        http.reply(400, r#"{"message":"Bad Request"}"#)
            .fail("network down");
        assert_eq!(
            valid_token(&http, &creds(), &store, 100),
            Err(ApiError::Unauthorized)
        );
        assert!(matches!(
            valid_token(&http, &creds(), &store, 100),
            Err(ApiError::Transient(_))
        ));
        // The old token stays until a refresh succeeds.
        assert_eq!(store.load().unwrap(), Some(token(0)));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn code_exchange_stores_the_athlete() {
        let dir = temp_dir("exchange");
        let store = TokenStore {
            path: dir.join("t.toml"),
        };
        let http = MockHttp::default();
        http.reply(
            200,
            r#"{"access_token":"a","refresh_token":"r","expires_at":5,"athlete":{"id":1,"firstname":"Marianne","lastname":"Vos"}}"#,
        );
        let t = exchange_code(&http, &creds(), &store, "the-code").unwrap();
        assert_eq!(t.athlete.as_deref(), Some("Marianne Vos"));
        assert_eq!(store.load().unwrap(), Some(t));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn authorize_url_and_callback() {
        let url = authorize_url("123", "http://127.0.0.1:5000/strava/callback", "st");
        assert_eq!(
            url,
            "https://www.strava.com/oauth/authorize?client_id=123&redirect_uri=http%3A%2F%2F127.0.0.1%3A5000%2Fstrava%2Fcallback&response_type=code&approval_prompt=auto&scope=read%2Cactivity%3Awrite&state=st"
        );
        assert_eq!(
            parse_callback(
                "GET /strava/callback?state=st&code=abc%2D1&scope=read,activity:write HTTP/1.1\r\n",
                "st"
            ),
            Callback::Code("abc-1".into())
        );
        assert_eq!(
            parse_callback(
                "GET /strava/callback?state=st&error=access_denied HTTP/1.1",
                "st"
            ),
            Callback::Denied("access_denied".into())
        );
        assert!(matches!(
            parse_callback(
                "GET /strava/callback?state=st&code=abc&scope=read HTTP/1.1",
                "st"
            ),
            Callback::Denied(_)
        ));
        // Wrong state (CSRF), other paths and methods are ignored.
        for line in [
            "GET /strava/callback?state=evil&code=abc HTTP/1.1",
            "GET /favicon.ico HTTP/1.1",
            "POST /strava/callback?state=st&code=abc HTTP/1.1",
            "",
        ] {
            assert_eq!(parse_callback(line, "st"), Callback::Ignore, "{line}");
        }
        let (a, b) = (random_state(), random_state());
        assert_eq!(a.len(), 32);
        assert_ne!(a, b);
    }
}

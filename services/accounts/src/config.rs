//! The backend's configuration (`config/accounts.toml`, git-ignored; the
//! template is `services/accounts/accounts.example.toml`). Client secrets live only
//! here, never in the repository or the apps (spec §9.1).

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// Strava's endpoints (https://developers.strava.com/docs/authentication/).
pub const STRAVA_AUTHORIZE_URL: &str = "https://www.strava.com/oauth/authorize";
pub const STRAVA_TOKEN_URL: &str = "https://www.strava.com/oauth/token";
/// Google's issuer (discovery at `/.well-known/openid-configuration`).
pub const GOOGLE_ISSUER: &str = "https://accounts.google.com";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// The public base URL of the backend, without a trailing slash: the
    /// `iss` of our tokens and the base of every endpoint (a public HTTPS
    /// domain for real sign-ins; `http://127.0.0.1:PORT` in development).
    pub issuer: String,
    /// Where the HTTP server listens (behind a TLS reverse proxy in
    /// production).
    #[serde(default = "default_listen")]
    pub listen: String,
    /// The `aud` of access tokens: what the game servers check.
    #[serde(default = "default_audience")]
    pub audience: String,
    #[serde(default)]
    pub tokens: Lifetimes,
    #[serde(default)]
    pub keys: KeyConfig,
    #[serde(default)]
    pub storage: StorageConfig,
    /// The apps (public clients with PKCE). Empty: the built-in three
    /// ([`Config::default_clients`]).
    #[serde(default)]
    pub clients: Vec<ClientConfig>,
    #[serde(default)]
    pub google: Option<GoogleConfig>,
    #[serde(default)]
    pub strava: Option<StravaConfig>,
    #[serde(default)]
    pub rate_limit: RateLimit,
    /// Who may credit hearts (spec 14 Phase 4, 4O Q19).
    #[serde(default)]
    pub hearts: HeartsConfig,
}

/// The trust of ride credits (Q19 (a)): game servers whose signing key
/// one of these roots certified (the key hierarchy of spec 4.7) credit
/// hearts directly; every other ride counts through the uploaded summary.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HeartsConfig {
    /// Root public key files (`*.pub.json` of `world-builder keys
    /// init-root`): the project's root key. Empty: no server credits
    /// directly.
    pub roots: Vec<PathBuf>,
    /// Revoked key ids (16 hex digits): roots or servers' signing keys.
    pub revoked: Vec<String>,
}

fn default_listen() -> String {
    "127.0.0.1:8480".into()
}

fn default_audience() -> String {
    ocw_auth::DEFAULT_AUDIENCE.into()
}

/// Token lifetimes in seconds (short access tokens, RFC 9700 §2.2).
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Lifetimes {
    pub access_s: i64,
    pub id_s: i64,
    /// A refresh token unused this long expires (idle timeout).
    pub refresh_idle_s: i64,
    /// A refresh token family ends this long after the sign-in.
    pub refresh_max_s: i64,
    pub code_s: i64,
    /// A sign-in at a provider must come back within this time.
    pub login_s: i64,
    pub device_s: i64,
    pub device_interval_s: i64,
    pub link_ticket_s: i64,
    /// Account deletion wants a sign-in at most this old.
    pub recent_auth_s: i64,
}

impl Default for Lifetimes {
    fn default() -> Self {
        Self {
            access_s: 600,
            id_s: 600,
            refresh_idle_s: 30 * 86_400,
            refresh_max_s: 180 * 86_400,
            code_s: 60,
            login_s: 600,
            device_s: 600,
            device_interval_s: 5,
            link_ticket_s: 300,
            recent_auth_s: 900,
        }
    }
}

/// The signing keys: kept in `dir` (mode 0600), rotated every
/// `rotate_days`; the previous key stays in the JWKS until no token it
/// signed can be valid any more.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct KeyConfig {
    /// `None`: keys in memory only (tests, `--auth-dev`).
    pub dir: Option<PathBuf>,
    pub rotate_days: i64,
}

impl Default for KeyConfig {
    fn default() -> Self {
        Self {
            dir: None,
            rotate_days: 30,
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StorageConfig {
    /// `memory` (tests, development), `file` (one JSON file, 0600) or
    /// `postgres` (the hosted backend, 4O; also the workshop and hearts).
    pub kind: StorageKind,
    pub path: Option<PathBuf>,
    /// `postgres`: the connection URL (`postgres://user:password@host/db`).
    /// It holds the password, so it is better left out here and given by
    /// the environment variable `url_env` names (Fly.io sets
    /// `DATABASE_URL`).
    pub url: Option<String>,
    /// `postgres`: the environment variable with the URL (default
    /// `DATABASE_URL`) when `url` is not set.
    pub url_env: Option<String>,
    /// `postgres`: the most connections the backend opens (default 8).
    pub max_connections: Option<u32>,
    /// The 32-byte key that encrypts the stored provider tokens (Strava),
    /// created on first start (0600). `None`: a key in memory (tokens die
    /// with the process).
    pub secrets_key: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageKind {
    #[default]
    Memory,
    File,
    Postgres,
}

/// Never prints the database URL (it holds the password).
impl fmt::Debug for StorageConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StorageConfig")
            .field("kind", &self.kind)
            .field("path", &self.path)
            .field("url", &self.url.as_ref().map(|_| "***"))
            .field("url_env", &self.url_env)
            .field("max_connections", &self.max_connections)
            .field("secrets_key", &self.secrets_key)
            .finish()
    }
}

impl StorageConfig {
    /// The PostgreSQL URL: `url`, else the environment variable `url_env`
    /// (default `DATABASE_URL`).
    pub fn database_url(&self) -> Result<String, String> {
        if let Some(u) = &self.url {
            return Ok(u.clone());
        }
        let var = self.url_env.as_deref().unwrap_or("DATABASE_URL");
        std::env::var(var).map_err(|_| {
            format!(
                "storage.kind = \"postgres\" needs storage.url or the environment variable {var}"
            )
        })
    }
}

/// One app.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClientConfig {
    pub id: String,
    /// Shown on the device page ("Open Cycling World kiosk").
    pub name: String,
    /// Exact redirect URIs (the browser app's page).
    #[serde(default)]
    pub redirect_uris: Vec<String>,
    /// Loopback redirects on any port (RFC 8252 §7.3) with this path.
    #[serde(default)]
    pub loopback_path: Option<String>,
    /// The device flow (kiosk, VR headset).
    #[serde(default)]
    pub device: bool,
    /// Origins allowed to call the token endpoint from a page (CORS).
    #[serde(default)]
    pub origins: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoogleConfig {
    pub client_id: String,
    pub client_secret: String,
    /// The OpenID provider (tests: the fake's URL).
    #[serde(default = "google_issuer")]
    pub issuer: String,
}

fn google_issuer() -> String {
    GOOGLE_ISSUER.into()
}

impl fmt::Debug for GoogleConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "GoogleConfig {{ client_id: {}, issuer: {}, client_secret: *** }}",
            self.client_id, self.issuer
        )
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StravaConfig {
    pub client_id: String,
    pub client_secret: String,
    #[serde(default = "strava_authorize")]
    pub authorize_url: String,
    #[serde(default = "strava_token")]
    pub token_url: String,
    /// Ask for `activity:write` at sign-in (optional consent: the athlete
    /// can untick it), so the later upload (spec §2.8) needs no second
    /// authorization.
    #[serde(default = "yes")]
    pub ask_upload: bool,
    /// Send PKCE to Strava too (spec §9.1: PKCE everywhere; Strava
    /// ignores unknown parameters).
    #[serde(default = "yes")]
    pub pkce: bool,
    /// Strava's default application limits (requests per 15 minutes and
    /// per day); the backend stays below them.
    #[serde(default = "strava_15min")]
    pub limit_15min: u32,
    #[serde(default = "strava_day")]
    pub limit_day: u32,
}

fn strava_authorize() -> String {
    STRAVA_AUTHORIZE_URL.into()
}
fn strava_token() -> String {
    STRAVA_TOKEN_URL.into()
}
fn yes() -> bool {
    true
}
fn strava_15min() -> u32 {
    200
}
fn strava_day() -> u32 {
    2000
}

impl fmt::Debug for StravaConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "StravaConfig {{ client_id: {}, authorize_url: {}, token_url: {}, ask_upload: {}, client_secret: *** }}",
            self.client_id, self.authorize_url, self.token_url, self.ask_upload
        )
    }
}

/// Requests per minute and client address on the token, device and
/// sign-in endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RateLimit {
    pub per_minute: u32,
    /// Trust `X-Forwarded-For` from this proxy address (the TLS reverse
    /// proxy in front); otherwise the peer address counts.
    pub trusted_proxy: Option<String>,
}

impl Default for RateLimit {
    fn default() -> Self {
        Self {
            per_minute: 30,
            trusted_proxy: None,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let cfg: Self = toml::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// A development configuration: memory storage, the built-in clients.
    pub fn dev(issuer: &str) -> Self {
        Self {
            issuer: issuer.trim_end_matches('/').to_owned(),
            listen: default_listen(),
            audience: default_audience(),
            tokens: Lifetimes::default(),
            keys: KeyConfig::default(),
            storage: StorageConfig::default(),
            clients: Vec::new(),
            google: None,
            strava: None,
            rate_limit: RateLimit::default(),
            hearts: HeartsConfig::default(),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.issuer.ends_with('/') {
            return Err("issuer must not end with '/'".into());
        }
        let local = self.issuer.starts_with("http://127.0.0.1")
            || self.issuer.starts_with("http://localhost")
            || self.issuer.starts_with("http://[::1]");
        if !self.issuer.starts_with("https://") && !local {
            return Err("issuer must be https:// (http only on the loopback address)".into());
        }
        if self.storage.kind == StorageKind::File && self.storage.path.is_none() {
            return Err("storage.kind = \"file\" needs storage.path".into());
        }
        for c in self.clients() {
            for u in &c.redirect_uris {
                if u.contains('#') || u.contains('*') {
                    return Err(format!("client {}: redirect URI {u} must be exact", c.id));
                }
            }
        }
        Ok(())
    }

    /// The configured apps, or the built-in desktop, browser and kiosk
    /// clients (the browser's redirect URI is the issuer's `/app/` page
    /// only in development).
    pub fn clients(&self) -> Vec<ClientConfig> {
        if !self.clients.is_empty() {
            return self.clients.clone();
        }
        Self::default_clients()
    }

    pub fn default_clients() -> Vec<ClientConfig> {
        vec![
            ClientConfig {
                id: ocw_auth::CLIENT_DESKTOP.into(),
                name: "Open Cycling World".into(),
                redirect_uris: Vec::new(),
                loopback_path: Some(ocw_auth::client::LOOPBACK_PATH.into()),
                device: false,
                origins: Vec::new(),
            },
            ClientConfig {
                id: ocw_auth::CLIENT_KIOSK.into(),
                name: "Open Cycling World kiosk".into(),
                redirect_uris: Vec::new(),
                loopback_path: None,
                device: true,
                origins: Vec::new(),
            },
            ClientConfig {
                id: ocw_auth::CLIENT_WEB.into(),
                name: "Open Cycling World in the browser".into(),
                redirect_uris: Vec::new(),
                loopback_path: None,
                device: false,
                origins: Vec::new(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The field names a serde struct reads (its derive's list).
    fn fields<T: serde::de::DeserializeOwned>() -> &'static [&'static str] {
        use serde::de::{Error, Visitor};
        struct Probe<'a>(&'a mut &'static [&'static str]);
        impl<'de> serde::Deserializer<'de> for Probe<'_> {
            type Error = serde::de::value::Error;
            fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Self::Error> {
                Err(Error::custom("probe"))
            }
            fn deserialize_struct<V: Visitor<'de>>(
                self,
                _: &'static str,
                fields: &'static [&'static str],
                _: V,
            ) -> Result<V::Value, Self::Error> {
                *self.0 = fields;
                Err(Error::custom("probe"))
            }
            serde::forward_to_deserialize_any! {
                bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string
                bytes byte_buf option unit unit_struct newtype_struct seq tuple
                tuple_struct map enum identifier ignored_any
            }
        }
        let mut f: &'static [&'static str] = &[];
        let _ = T::deserialize(Probe(&mut f));
        assert!(!f.is_empty(), "{}", std::any::type_name::<T>());
        f
    }

    /// The rule (CLAUDE.md, spec 3.2.4): every key of the config has an
    /// example in `accounts.example.toml`, set or commented out, in its
    /// section – a new field fails here until it is documented there.
    #[test]
    fn every_config_key_is_in_the_example() {
        let text = include_str!("../accounts.example.toml");
        // (section, key) of every `key = ` line, set or commented out.
        let mut found: Vec<(String, String)> = Vec::new();
        let mut section = String::new();
        for line in text.lines() {
            let l = line.trim_start().trim_start_matches('#').trim_start();
            if let Some(h) = l.strip_prefix('[') {
                section = h.trim_matches(|c| c == '[' || c == ']').to_string();
            } else if let Some((k, _)) = l.split_once(" = ")
                && !k.is_empty()
                && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            {
                found.push((section.clone(), k.to_string()));
            }
        }
        let mut lacking = Vec::new();
        let sections: [(&str, &[&str]); 9] = [
            ("", fields::<Config>()),
            ("tokens", fields::<Lifetimes>()),
            ("keys", fields::<KeyConfig>()),
            ("storage", fields::<StorageConfig>()),
            ("clients", fields::<ClientConfig>()),
            ("google", fields::<GoogleConfig>()),
            ("strava", fields::<StravaConfig>()),
            ("rate_limit", fields::<RateLimit>()),
            ("hearts", fields::<HeartsConfig>()),
        ];
        for (s, keys) in sections {
            for k in keys {
                let is_section = s.is_empty() && sections.iter().any(|(n, _)| n == k);
                if !is_section && !found.iter().any(|(fs, fk)| fs == s && fk == k) {
                    lacking.push(format!("[{s}] {k}"));
                }
            }
        }
        assert!(
            lacking.is_empty(),
            "not in accounts.example.toml: {lacking:?}"
        );
    }

    #[test]
    fn example_config_parses() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("accounts.example.toml");
        let cfg = Config::load(&path).unwrap();
        assert!(cfg.google.is_some() && cfg.strava.is_some());
        assert_eq!(cfg.clients().len(), 3);
        // Secrets never in debug output.
        let d = format!("{cfg:?}");
        assert!(
            !d.contains("GOOGLE-CLIENT-SECRET") && !d.contains("STRAVA-CLIENT-SECRET"),
            "{d}"
        );
    }

    #[test]
    fn validation() {
        let mut c = Config::dev("http://127.0.0.1:8480");
        assert!(c.validate().is_ok());
        c.issuer = "http://accounts.example.org".into();
        assert!(c.validate().is_err(), "plain http outside the loopback");
        c.issuer = "https://accounts.example.org/".into();
        assert!(c.validate().is_err());
        let mut c = Config::dev("https://accounts.example.org");
        c.storage.kind = StorageKind::File;
        assert!(c.validate().is_err());
        assert_eq!(Lifetimes::default().access_s, 600);
    }

    #[test]
    fn postgres_storage_never_prints_its_url() {
        let cfg: Config = toml::from_str(
            r#"
issuer = "http://127.0.0.1:8480"
[storage]
kind = "postgres"
url = "postgres://ocw:not-a-real-password@127.0.0.1/ocw"
[hearts]
roots = ["keys/ocw-root.pub.json"]
"#,
        )
        .unwrap();
        assert_eq!(cfg.storage.kind, StorageKind::Postgres);
        assert!(cfg.storage.database_url().unwrap().contains("not-a-real"));
        assert!(!format!("{cfg:?}").contains("not-a-real"));
        assert_eq!(cfg.hearts.roots.len(), 1);
        let none = StorageConfig {
            kind: StorageKind::Postgres,
            url_env: Some("OCW_TEST_NO_SUCH_VARIABLE".into()),
            ..StorageConfig::default()
        };
        assert!(none.database_url().is_err());
    }
}

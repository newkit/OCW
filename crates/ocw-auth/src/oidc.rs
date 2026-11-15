//! The wire types of the backend's OIDC-shaped endpoints (spec §9.1):
//! discovery (OpenID Connect Discovery 1.0), token and error responses
//! (RFC 6749 §5), the device authorization (RFC 8628 §3.2), userinfo, and
//! the account API's answers.

use serde::{Deserialize, Serialize};

/// The device-code grant type (RFC 8628 §3.4).
pub const GRANT_DEVICE_CODE: &str = "urn:ietf:params:oauth:grant-type:device_code";
/// What the apps ask for: an ID token, the profile, a refresh token.
pub const APP_SCOPE: &str = "openid profile offline_access";

/// The providers one can sign in with (spec §9.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Google,
    Strava,
}

impl Provider {
    pub const ALL: [Provider; 2] = [Provider::Google, Provider::Strava];

    pub fn id(self) -> &'static str {
        match self {
            Self::Google => "google",
            Self::Strava => "strava",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Google => "Google",
            Self::Strava => "Strava",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.id() == s)
    }
}

impl std::fmt::Display for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// `/.well-known/openid-configuration`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Discovery {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub jwks_uri: String,
    pub userinfo_endpoint: String,
    pub revocation_endpoint: String,
    pub device_authorization_endpoint: String,
    #[serde(default)]
    pub response_types_supported: Vec<String>,
    #[serde(default)]
    pub grant_types_supported: Vec<String>,
    #[serde(default)]
    pub subject_types_supported: Vec<String>,
    #[serde(default)]
    pub id_token_signing_alg_values_supported: Vec<String>,
    #[serde(default)]
    pub code_challenge_methods_supported: Vec<String>,
    #[serde(default)]
    pub token_endpoint_auth_methods_supported: Vec<String>,
    #[serde(default)]
    pub scopes_supported: Vec<String>,
    #[serde(default)]
    pub claims_supported: Vec<String>,
    /// RFC 9207: the redirect carries `iss` (mix-up defence).
    #[serde(default)]
    pub authorization_response_iss_parameter_supported: bool,
    /// OCW extension: the providers this backend offers.
    #[serde(default)]
    pub ocw_providers: Vec<Provider>,
    /// OCW extension: the account API (link, unlink, export, delete).
    #[serde(default)]
    pub ocw_account_endpoint: Option<String>,
}

impl Discovery {
    /// The document of a backend at `issuer` (no trailing slash).
    pub fn for_issuer(issuer: &str, providers: Vec<Provider>) -> Self {
        let e = |p: &str| format!("{issuer}{p}");
        let v = |a: &[&str]| a.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        Self {
            issuer: issuer.to_owned(),
            authorization_endpoint: e("/authorize"),
            token_endpoint: e("/token"),
            jwks_uri: e("/jwks"),
            userinfo_endpoint: e("/userinfo"),
            revocation_endpoint: e("/revoke"),
            device_authorization_endpoint: e("/device_authorization"),
            response_types_supported: v(&["code"]),
            grant_types_supported: v(&["authorization_code", "refresh_token", GRANT_DEVICE_CODE]),
            subject_types_supported: v(&["public"]),
            id_token_signing_alg_values_supported: v(&[crate::jwt::ALG]),
            code_challenge_methods_supported: v(&[crate::pkce::METHOD]),
            token_endpoint_auth_methods_supported: v(&["none"]),
            scopes_supported: v(&["openid", "profile", "offline_access"]),
            claims_supported: v(&["sub", "name", "providers", "auth_time"]),
            authorization_response_iss_parameter_supported: true,
            ocw_providers: providers,
            ocw_account_endpoint: Some(e("/account")),
        }
    }
}

/// A successful token response.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print tokens (spec §9.1: no tokens in logs).
        write!(
            f,
            "TokenResponse {{ expires_in: {}, refresh: {}, id_token: {}, tokens: *** }}",
            self.expires_in,
            self.refresh_token.is_some(),
            self.id_token.is_some()
        )
    }
}

/// An OAuth error (RFC 6749 §5.2, RFC 8628 §3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

impl ErrorResponse {
    pub fn new(error: &str, description: impl Into<String>) -> Self {
        Self {
            error: error.into(),
            error_description: Some(description.into()),
        }
    }
}

/// The device authorization response (RFC 8628 §3.2).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceAuthorization {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_uri_complete: Option<String>,
    pub expires_in: i64,
    #[serde(default = "default_interval")]
    pub interval: i64,
}

fn default_interval() -> i64 {
    5
}

impl std::fmt::Debug for DeviceAuthorization {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DeviceAuthorization {{ user_code: {}, uri: {}, expires_in: {}, interval: {}, device_code: *** }}",
            self.user_code, self.verification_uri, self.expires_in, self.interval
        )
    }
}

/// A provider linked to the account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkedProvider {
    pub provider: Provider,
    /// The name at the provider (Google's name, the Strava athlete's).
    pub name: String,
    pub linked_at: i64,
    /// Strava: uploads allowed (`activity:write` granted, spec §2.8).
    #[serde(default)]
    pub upload: bool,
}

/// `userinfo` (OpenID Connect Core §5.3) with our claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserInfo {
    pub sub: String,
    pub name: String,
    #[serde(default)]
    pub providers: Vec<LinkedProvider>,
    /// Google's address, when Google is linked (Strava gives none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// The answer of `POST /account/link`: a one-time ticket for the
/// authorization request that links another provider.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkTicket {
    pub link_ticket: String,
    pub expires_in: i64,
}

impl std::fmt::Debug for LinkTicket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LinkTicket {{ expires_in: {}, ticket: *** }}",
            self.expires_in
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_round_trip_and_providers() {
        let d = Discovery::for_issuer("https://id.example", vec![Provider::Google]);
        assert_eq!(d.token_endpoint, "https://id.example/token");
        assert_eq!(d.code_challenge_methods_supported, ["S256"]);
        let json = serde_json::to_string(&d).unwrap();
        assert!(json.contains(r#""ocw_providers":["google"]"#), "{json}");
        assert_eq!(serde_json::from_str::<Discovery>(&json).unwrap(), d);
        assert_eq!(Provider::parse("strava"), Some(Provider::Strava));
        assert_eq!(Provider::parse("facebook"), None);
    }

    #[test]
    fn secrets_stay_out_of_debug_output() {
        let t = TokenResponse {
            access_token: "AT-secret".into(),
            token_type: "Bearer".into(),
            expires_in: 600,
            refresh_token: Some("RT-secret".into()),
            id_token: None,
            scope: None,
        };
        let d = DeviceAuthorization {
            device_code: "DC-secret".into(),
            user_code: "BCDF-GHJK".into(),
            verification_uri: "https://id.example/device".into(),
            verification_uri_complete: None,
            expires_in: 600,
            interval: 5,
        };
        let l = LinkTicket {
            link_ticket: "LT-secret".into(),
            expires_in: 300,
        };
        for s in [format!("{t:?}"), format!("{d:?}"), format!("{l:?}")] {
            assert!(!s.contains("secret"), "{s}");
        }
        let parsed: DeviceAuthorization = serde_json::from_str(
            r#"{"device_code":"d","user_code":"u","verification_uri":"v","expires_in":9}"#,
        )
        .unwrap();
        assert_eq!(parsed.interval, 5, "RFC 8628 default");
    }
}

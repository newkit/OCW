//! The accounts backend of Open Cycling World (spec §9.1): sign-in with
//! **Google** (OpenID Connect) and **Strava** (OAuth 2), brokered by our
//! own token issuer – no separate identity server.
//!
//! The endpoints are shaped like an OpenID Connect provider
//! (`/.well-known/openid-configuration`, `/authorize`, `/token`, `/jwks`,
//! `/device_authorization`, `/revoke`, `/userinfo`), so Keycloak or
//! Authentik can replace this broker later without changing the apps. The
//! apps are public clients with PKCE: the desktop through the system
//! browser and a loopback redirect (RFC 8252), the browser app with a
//! redirect, kiosks and headsets with the device flow (RFC 8628). The game
//! server checks our access tokens against the JWKS (`ocw_auth::Verifier`).
//!
//! - [`service::AuthService`]: the flows, transport-free,
//! - [`routes`]: the axum router and [`serve`],
//! - [`store`]: the account storage trait, in memory or a private JSON
//!   file (PostgreSQL is the documented next step),
//! - [`keys`]: Ed25519 signing keys with rotation, [`secrets`]: provider
//!   tokens encrypted at rest, [`limits`]: rate limits,
//! - [`providers`]: Google and Strava, [`fake`]: in-process fake Google and
//!   Strava servers for the tests and `ocw-server --auth-dev`.
//!
//! Secrets (client secrets, keys, tokens) never appear in logs or debug
//! output.

pub mod config;
pub mod fake;
pub mod http;
pub mod keys;
pub mod limits;
pub mod pages;
pub mod providers;
pub mod routes;
pub mod secrets;
pub mod service;
pub mod store;

use std::sync::Arc;

pub use config::Config;
pub use routes::{router, serve};
pub use service::AuthService;

/// The service's clock (Unix seconds); tests move it.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// What another backend module adds to an account's export and deletion
/// (spec §12: training data, the workshop's hearts and parts …).
pub trait AccountHooks: Send + Sync {
    /// The module's name in the export.
    fn name(&self) -> &str;
    /// The module's data of the account (JSON).
    fn export(&self, account: &str) -> serde_json::Value;
    /// Deletes the module's data of the account.
    fn delete(&self, account: &str);
}

/// `N` bytes from the OS's random source.
pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the OS random source works");
    b
}

/// An opaque 256-bit random token (base64url).
pub fn random_token() -> String {
    ocw_auth::random_token(&random_bytes::<32>())
}

//! The sign-in broker (spec §9.1), transport-free: the axum routes in
//! [`crate::routes`] only translate HTTP to these calls.
//!
//! - **authorize**: an app's request (exact redirect URI or a loopback one,
//!   PKCE S256, `state`, `nonce`) → the provider's sign-in (our own `state`,
//!   `nonce` and PKCE towards it) → the callback creates or finds the OCW
//!   account → a one-time code (60 s, bound to client, redirect URI and
//!   challenge) back to the app, with `iss` (RFC 9207).
//! - **token**: code + verifier → access token (JWT, 10 min), ID token,
//!   refresh token (rotating; a reused one revokes its whole family; a
//!   reused code revokes what it gave); refresh; the device code grant.
//! - **device** (RFC 8628): `device_authorization`, the code page,
//!   pending / slow down / approved / denied / expired.
//! - **account**: userinfo, link tickets (linking another provider is a
//!   normal sign-in bound to the app's PKCE challenge, committed only when
//!   the app redeems the code), unlink (never the last sign-in), export
//!   and deletion with the hooks of the other modules (spec §12).

use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use ocw_auth::jwt::{AccessClaims, IdClaims, Jwks, TYP_ACCESS, TYP_ID, sign};
use ocw_auth::oidc::{
    DeviceAuthorization, Discovery, ErrorResponse, GRANT_DEVICE_CODE, LinkTicket, LinkedProvider,
    TokenResponse, UserInfo,
};
use ocw_auth::{Provider, Verifier, pkce, token_hash};

use crate::config::{ClientConfig, Config, StorageKind};
use crate::keys::KeyRing;
use crate::limits::RateLimiter;
use crate::providers::{Google, ProviderIdentity, StartRequest, Strava, Upstream};
use crate::secrets::SecretBox;
use crate::store::{Account, Identity, MemStore, RefreshRecord, Store, StoreError};
use crate::{AccountHooks, Clock, random_token};

/// An OAuth error answer: HTTP status and body.
pub type OAuthError = (u16, ErrorResponse);

fn err(status: u16, error: &str, description: impl Into<String>) -> OAuthError {
    (status, ErrorResponse::new(error, description))
}

fn invalid_grant(d: &str) -> OAuthError {
    err(400, "invalid_grant", d)
}

/// The user-code alphabet (RFC 8628 §6.1: no vowels, no look-alikes).
const USER_CODE_CHARS: &[u8] = b"BCDFGHJKLMNPQRSTVWXZ";

/// A validated authorization request of an app.
#[derive(Debug, Clone)]
struct AppRequest {
    client_id: String,
    redirect_uri: String,
    state: String,
    nonce: Option<String>,
    challenge: String,
    scope: String,
    /// Linking a provider to this account.
    link: Option<String>,
}

#[derive(Debug, Clone)]
enum Target {
    App(AppRequest),
    /// A device code being confirmed (its hash).
    Device(String),
}

/// A sign-in at a provider in progress (by our `state` towards it).
struct PendingProvider {
    provider: Provider,
    target: Target,
    nonce: String,
    verifier: String,
    created: i64,
}

struct AuthCode {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    nonce: Option<String>,
    account: String,
    scope: String,
    auth_time: i64,
    created: i64,
    /// A link to commit on redemption.
    link: Option<ProviderIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DeviceStatus {
    Pending,
    Approved { account: String, auth_time: i64 },
    Denied,
}

struct DeviceGrant {
    client_id: String,
    scope: String,
    user_code: String,
    expires_at: i64,
    interval: i64,
    last_poll: Option<i64>,
    status: DeviceStatus,
}

struct TicketRec {
    account: String,
    client_id: String,
    challenge: String,
    expires_at: i64,
}

#[derive(Default)]
struct Ephemeral {
    logins: HashMap<String, PendingProvider>,
    codes: HashMap<String, AuthCode>,
    /// Redeemed codes → the refresh family they started (reuse revokes it).
    redeemed: HashMap<String, (String, i64)>,
    devices: HashMap<String, DeviceGrant>,
    user_codes: HashMap<String, String>,
    tickets: HashMap<String, TicketRec>,
}

impl Ephemeral {
    fn purge(&mut self, now: i64, cfg: &Config) {
        let t = &cfg.tokens;
        self.logins.retain(|_, p| now - p.created < t.login_s);
        self.codes.retain(|_, c| now - c.created < t.code_s);
        self.redeemed.retain(|_, (_, at)| now - *at < 3600);
        self.devices.retain(|_, d| now < d.expires_at + 60);
        let devices = &self.devices;
        self.user_codes.retain(|_, h| devices.contains_key(h));
        self.tickets.retain(|_, k| now < k.expires_at);
    }
}

/// What `/authorize` answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Authorize {
    /// Show the provider choice: links to these URLs.
    Choose {
        client: String,
        links: Vec<(Provider, String)>,
    },
    /// Redirect the browser (to the provider, or back to the app with an
    /// error).
    Redirect(String),
    /// A request we cannot redirect back (unknown client, bad redirect
    /// URI): an error page, never a redirect (RFC 6749 §4.1.2.1).
    Page(String),
}

/// What a provider's callback answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    Redirect(String),
    /// A page with a title and text (the device flow's end, errors).
    Page {
        title: String,
        text: String,
    },
}

pub struct AuthService {
    cfg: Config,
    clients: Vec<ClientConfig>,
    store: Arc<dyn Store>,
    keys: KeyRing,
    upstreams: Vec<Arc<dyn Upstream>>,
    eph: Mutex<Ephemeral>,
    pub(crate) limiter: RateLimiter,
    secrets: SecretBox,
    hooks: RwLock<Vec<Arc<dyn AccountHooks>>>,
    clock: Clock,
}

fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl AuthService {
    /// The service of `cfg` on the system clock.
    pub fn new(cfg: Config) -> Result<Self, String> {
        Self::with_clock(cfg, Arc::new(now_s))
    }

    pub fn with_clock(cfg: Config, clock: Clock) -> Result<Self, String> {
        cfg.validate()?;
        let now = clock();
        let rotate_s = cfg.keys.rotate_days.max(1) * 86_400;
        let grace_s = cfg.tokens.access_s.max(cfg.tokens.id_s) + 300;
        let keys = match &cfg.keys.dir {
            Some(dir) => KeyRing::open(dir.clone(), now, rotate_s, grace_s)?,
            None => KeyRing::memory(now, rotate_s, grace_s),
        };
        let store: Arc<dyn Store> = match cfg.storage.kind {
            StorageKind::Memory => Arc::new(MemStore::memory()),
            StorageKind::File => Arc::new(
                MemStore::open(cfg.storage.path.clone().expect("validated"))
                    .map_err(|e| e.to_string())?,
            ),
        };
        let secrets = match &cfg.storage.secrets_key {
            Some(p) => SecretBox::open(p)?,
            None => SecretBox::ephemeral(),
        };
        let mut upstreams: Vec<Arc<dyn Upstream>> = Vec::new();
        if let Some(g) = &cfg.google {
            upstreams.push(Arc::new(Google::new(g.clone(), &cfg.issuer)));
        }
        if let Some(s) = &cfg.strava {
            upstreams.push(Arc::new(Strava::new(s.clone(), &cfg.issuer)));
        }
        Ok(Self {
            clients: cfg.clients(),
            limiter: RateLimiter::new(cfg.rate_limit.per_minute),
            store,
            keys,
            upstreams,
            eph: Mutex::new(Ephemeral::default()),
            secrets,
            hooks: RwLock::new(Vec::new()),
            clock,
            cfg,
        })
    }

    /// Uses another store (tests; PostgreSQL later).
    pub fn with_store(mut self, store: Arc<dyn Store>) -> Self {
        self.store = store;
        self
    }

    /// Another module's export and deletion hook (spec §12).
    pub fn add_hooks(&self, hooks: Arc<dyn AccountHooks>) {
        self.hooks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .push(hooks);
    }

    pub fn config(&self) -> &Config {
        &self.cfg
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    pub fn now(&self) -> i64 {
        (self.clock)()
    }

    pub fn issuer(&self) -> &str {
        &self.cfg.issuer
    }

    pub fn providers(&self) -> Vec<Provider> {
        self.upstreams.iter().map(|u| u.provider()).collect()
    }

    fn upstream(&self, p: Provider) -> Option<Arc<dyn Upstream>> {
        self.upstreams.iter().find(|u| u.provider() == p).cloned()
    }

    fn client(&self, id: &str) -> Option<&ClientConfig> {
        self.clients.iter().find(|c| c.id == id)
    }

    /// The origins allowed to call the token endpoint from a page.
    pub fn cors_origin_allowed(&self, origin: &str) -> bool {
        self.clients
            .iter()
            .any(|c| c.origins.iter().any(|o| o == origin))
    }

    fn eph(&self) -> std::sync::MutexGuard<'_, Ephemeral> {
        let mut e = self.eph.lock().unwrap_or_else(|e| e.into_inner());
        e.purge(self.now(), &self.cfg);
        e
    }

    pub fn discovery(&self) -> Discovery {
        Discovery::for_issuer(&self.cfg.issuer, self.providers())
    }

    pub fn jwks(&self) -> Jwks {
        self.keys.jwks()
    }

    /// Rotates the signing key when it is due (called periodically).
    pub fn maybe_rotate_keys(&self) -> Result<bool, String> {
        self.keys.maybe_rotate(self.now())
    }

    /// Rotates now (tests, an operator after a suspected leak).
    pub fn rotate_keys(&self) -> Result<(), String> {
        self.keys.rotate(self.now())
    }

    /// A verifier for our own access tokens with the current keys.
    pub fn verifier(&self) -> Verifier {
        Verifier::new(&self.cfg.issuer, &self.cfg.audience, self.jwks())
    }

    // ---- authorize -------------------------------------------------------

    fn redirect_allowed(client: &ClientConfig, uri: &str) -> bool {
        client.redirect_uris.iter().any(|u| u == uri)
            || client
                .loopback_path
                .as_deref()
                .is_some_and(|p| ocw_auth::client::is_loopback_redirect(uri, p))
    }

    fn back_to_app(&self, redirect_uri: &str, pairs: &[(&str, &str)]) -> String {
        let mut all: Vec<(&str, &str)> = pairs.to_vec();
        all.push(("iss", &self.cfg.issuer));
        let sep = if redirect_uri.contains('?') { '&' } else { '?' };
        format!("{redirect_uri}{sep}{}", ocw_auth::client::form(&all))
    }

    fn error_to_app(&self, req: &AppRequest, error: &str, description: &str) -> String {
        self.back_to_app(
            &req.redirect_uri,
            &[
                ("error", error),
                ("error_description", description),
                ("state", &req.state),
            ],
        )
    }

    /// Handles `/authorize` (blocking: the provider's discovery).
    pub fn authorize(&self, query: &[(String, String)]) -> Authorize {
        let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let Some(client) = get("client_id").and_then(|id| self.client(id)) else {
            return Authorize::Page("Unknown app (client_id).".into());
        };
        let Some(redirect_uri) = get("redirect_uri").filter(|u| Self::redirect_allowed(client, u))
        else {
            return Authorize::Page("This redirect URI is not registered for the app.".into());
        };
        let mut req = AppRequest {
            client_id: client.id.clone(),
            redirect_uri: redirect_uri.to_owned(),
            state: get("state").unwrap_or_default().to_owned(),
            nonce: get("nonce").map(str::to_owned),
            challenge: get("code_challenge").unwrap_or_default().to_owned(),
            scope: get("scope").unwrap_or("openid").to_owned(),
            link: None,
        };
        if get("response_type") != Some("code") {
            return Authorize::Redirect(self.error_to_app(
                &req,
                "unsupported_response_type",
                "only the authorization code flow",
            ));
        }
        if req.state.is_empty() || req.state.len() > 512 {
            return Authorize::Redirect(self.error_to_app(
                &req,
                "invalid_request",
                "state is required",
            ));
        }
        if get("code_challenge_method") != Some(pkce::METHOD)
            || !pkce::valid_challenge(&req.challenge)
        {
            return Authorize::Redirect(self.error_to_app(
                &req,
                "invalid_request",
                "PKCE with S256 is required",
            ));
        }
        let provider = match get("provider") {
            None => {
                // The choice page: the same request with each provider.
                let links = self
                    .providers()
                    .into_iter()
                    .map(|p| {
                        let mut q: Vec<(&str, &str)> = query
                            .iter()
                            .filter(|(k, _)| k != "provider")
                            .map(|(k, v)| (k.as_str(), v.as_str()))
                            .collect();
                        q.push(("provider", p.id()));
                        (
                            p,
                            format!(
                                "{}/authorize?{}",
                                self.cfg.issuer,
                                ocw_auth::client::form(&q)
                            ),
                        )
                    })
                    .collect();
                return Authorize::Choose {
                    client: client.name.clone(),
                    links,
                };
            }
            Some(p) => match Provider::parse(p).and_then(|p| self.upstream(p)) {
                Some(u) => u,
                None => {
                    return Authorize::Redirect(self.error_to_app(
                        &req,
                        "invalid_request",
                        "this sign-in provider is not available",
                    ));
                }
            },
        };
        let now = self.now();
        if let Some(ticket) = get("link_ticket") {
            let rec = self.eph().tickets.remove(&token_hash(ticket));
            match rec {
                Some(t)
                    if t.client_id == req.client_id
                        && t.challenge == req.challenge
                        && now < t.expires_at =>
                {
                    req.link = Some(t.account);
                }
                _ => {
                    return Authorize::Redirect(self.error_to_app(
                        &req,
                        "invalid_request",
                        "the link request expired; try again",
                    ));
                }
            }
        }
        let name = provider.provider();
        match self.start_provider(provider, Target::App(req.clone())) {
            Ok(url) => Authorize::Redirect(url),
            Err(e) => {
                tracing::warn!(provider = %name, error = %e, "provider sign-in failed to start");
                Authorize::Redirect(self.error_to_app(&req, "temporarily_unavailable", &e))
            }
        }
    }

    fn start_provider(
        &self,
        upstream: Arc<dyn Upstream>,
        target: Target,
    ) -> Result<String, String> {
        let (state, nonce, verifier) = (random_token(), random_token(), random_token());
        let now = self.now();
        let url = upstream.start(
            &StartRequest {
                state: &state,
                nonce: &nonce,
                verifier: &verifier,
            },
            now,
        )?;
        self.eph().logins.insert(
            state,
            PendingProvider {
                provider: upstream.provider(),
                target,
                nonce,
                verifier,
                created: now,
            },
        );
        Ok(url)
    }

    /// Handles `/callback/{provider}` (blocking: the code exchange).
    pub fn callback(&self, provider: Provider, query: &[(String, String)]) -> Callback {
        let get = |k: &str| query.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let page = |title: &str, text: &str| Callback::Page {
            title: title.into(),
            text: text.into(),
        };
        let Some(pending) = get("state").and_then(|s| self.eph().logins.remove(s)) else {
            return page(
                "Sign-in expired",
                "This sign-in has expired or was already used. Start it again from the app.",
            );
        };
        if pending.provider != provider {
            return page("Sign-in failed", "The answer came from another provider.");
        }
        let fail = |text: &str| match &pending.target {
            Target::App(req) => Callback::Redirect(self.error_to_app(req, "access_denied", text)),
            Target::Device(_) => page("Not signed in", text),
        };
        if get("error").is_some() {
            return fail(&format!("{provider} sign-in was cancelled."));
        }
        let Some(code) = get("code") else {
            return fail("No authorization code.");
        };
        let Some(upstream) = self.upstream(provider) else {
            return fail("This provider is not available.");
        };
        let start = StartRequest {
            state: "",
            nonce: &pending.nonce,
            verifier: &pending.verifier,
        };
        let identity = match upstream.finish(code, &start, get("scope"), self.now()) {
            Ok(i) => i,
            Err(e) => {
                tracing::warn!(%provider, error = %e, "provider sign-in failed");
                return fail(&format!("{provider} sign-in failed: {e}"));
            }
        };
        let auth_time = self.now();
        match pending.target {
            Target::App(req) => {
                let (account, link) = match &req.link {
                    Some(acc) => match self.store.find_identity(provider, &identity.subject) {
                        Some(other) if &other != acc => {
                            return Callback::Redirect(self.error_to_app(
                                &req,
                                "access_denied",
                                &StoreError::LinkedElsewhere(provider).to_string(),
                            ));
                        }
                        _ => (acc.clone(), Some(identity)),
                    },
                    None => match self.sign_in(&identity) {
                        Ok(a) => (a, None),
                        Err(e) => {
                            return Callback::Redirect(self.error_to_app(
                                &req,
                                "server_error",
                                &e.to_string(),
                            ));
                        }
                    },
                };
                let code = random_token();
                let mut eph = self.eph();
                eph.codes.insert(
                    token_hash(&code),
                    AuthCode {
                        client_id: req.client_id.clone(),
                        redirect_uri: req.redirect_uri.clone(),
                        challenge: req.challenge.clone(),
                        nonce: req.nonce.clone(),
                        account,
                        scope: req.scope.clone(),
                        auth_time,
                        created: self.now(),
                        link,
                    },
                );
                drop(eph);
                Callback::Redirect(
                    self.back_to_app(&req.redirect_uri, &[("code", &code), ("state", &req.state)]),
                )
            }
            Target::Device(hash) => {
                let account = match self.sign_in(&identity) {
                    Ok(a) => a,
                    Err(e) => return page("Not signed in", &e.to_string()),
                };
                let name = self
                    .store
                    .account(&account)
                    .map(|a| a.display_name)
                    .unwrap_or_default();
                let mut eph = self.eph();
                match eph.devices.get_mut(&hash) {
                    Some(d) if d.status == DeviceStatus::Pending && self.now() < d.expires_at => {
                        d.status = DeviceStatus::Approved { account, auth_time };
                        page(
                            "Device connected",
                            &format!("Signed in as {name}. You can go back to your device now."),
                        )
                    }
                    _ => page(
                        "Code expired",
                        "The code has expired. Ask the device for a new one.",
                    ),
                }
            }
        }
    }

    /// The account of a provider identity: found (its name refreshed) or
    /// created. Strava's tokens are kept encrypted.
    fn sign_in(&self, identity: &ProviderIdentity) -> Result<String, StoreError> {
        let now = self.now();
        let ident = Identity {
            provider: identity.provider,
            subject: identity.subject.clone(),
            name: identity.name.clone(),
            email: identity.email.clone(),
            linked_at: now,
            upload: identity.upload,
        };
        let account = match self
            .store
            .find_identity(identity.provider, &identity.subject)
        {
            Some(id) => {
                let linked_at = self
                    .store
                    .account(&id)
                    .and_then(|a| {
                        a.identities
                            .into_iter()
                            .find(|i| i.provider == identity.provider)
                    })
                    .map_or(now, |i| i.linked_at);
                self.store.link(&id, Identity { linked_at, ..ident })?;
                id
            }
            None => {
                let id = format!("acc_{}", &token_hash(&random_token())[..24]);
                let name = if identity.name.trim().is_empty() {
                    "Rider".to_owned()
                } else {
                    identity.name.trim().chars().take(40).collect()
                };
                self.store.create_account(Account {
                    id: id.clone(),
                    display_name: name,
                    created_at: now,
                    identities: vec![ident],
                })?;
                tracing::info!(account = %id, provider = %identity.provider, "account created");
                id
            }
        };
        self.keep_tokens(&account, identity)?;
        Ok(account)
    }

    fn keep_tokens(&self, account: &str, identity: &ProviderIdentity) -> Result<(), StoreError> {
        if let Some(t) = &identity.tokens {
            self.store.put_secret(
                account,
                identity.provider,
                self.secrets.seal(account, identity.provider, t),
            )?;
        }
        Ok(())
    }

    /// The provider tokens kept for an account (the later Strava upload).
    pub fn provider_tokens(
        &self,
        account: &str,
        provider: Provider,
    ) -> Option<crate::secrets::ProviderTokens> {
        let s = self.store.secret(account, provider)?;
        self.secrets.open_sealed(account, provider, &s)
    }

    // ---- tokens ----------------------------------------------------------

    fn issue(
        &self,
        account: &Account,
        client_id: &str,
        scope: &str,
        auth_time: i64,
        nonce: Option<String>,
        family: Option<(String, i64)>,
    ) -> Result<TokenResponse, OAuthError> {
        let now = self.now();
        let t = &self.cfg.tokens;
        let providers: Vec<String> = account
            .identities
            .iter()
            .map(|i| i.provider.id().to_owned())
            .collect();
        let (kid, key) = self.keys.active();
        let access = AccessClaims {
            iss: self.cfg.issuer.clone(),
            sub: account.id.clone(),
            aud: self.cfg.audience.clone(),
            exp: now + t.access_s,
            iat: now,
            jti: random_token(),
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
            name: account.display_name.clone(),
            auth_time,
            providers: providers.clone(),
        };
        let id_token = scope.split(' ').any(|s| s == "openid").then(|| {
            sign(
                &IdClaims {
                    iss: self.cfg.issuer.clone(),
                    sub: account.id.clone(),
                    aud: client_id.to_owned(),
                    exp: now + t.id_s,
                    iat: now,
                    auth_time,
                    nonce,
                    name: account.display_name.clone(),
                    providers,
                },
                &key,
                &kid,
                TYP_ID,
            )
        });
        let refresh = random_token();
        let (family, family_created) = family.unwrap_or_else(|| (random_token(), now));
        let store = |e: StoreError| err(500, "server_error", e.to_string());
        self.store
            .put_refresh(RefreshRecord {
                hash: token_hash(&refresh),
                family,
                account: account.id.clone(),
                client_id: client_id.to_owned(),
                scope: scope.to_owned(),
                auth_time,
                family_created,
                created_at: now,
                used: false,
                revoked: false,
            })
            .map_err(store)?;
        self.store
            .purge_refresh(now - t.refresh_idle_s)
            .map_err(store)?;
        Ok(TokenResponse {
            access_token: sign(&access, &key, &kid, TYP_ACCESS),
            token_type: "Bearer".into(),
            expires_in: t.access_s,
            refresh_token: Some(refresh),
            id_token,
            scope: Some(scope.to_owned()),
        })
    }

    /// Handles `/token`.
    pub fn token(&self, form: &[(String, String)]) -> Result<TokenResponse, OAuthError> {
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let client_id = get("client_id").unwrap_or_default();
        if self.client(client_id).is_none() {
            return Err(err(401, "invalid_client", "unknown client"));
        }
        match get("grant_type") {
            Some("authorization_code") => self.redeem_code(
                client_id,
                get("code").unwrap_or_default(),
                get("redirect_uri").unwrap_or_default(),
                get("code_verifier").unwrap_or_default(),
            ),
            Some("refresh_token") => {
                self.refresh(client_id, get("refresh_token").unwrap_or_default())
            }
            Some(GRANT_DEVICE_CODE) => {
                self.poll_device(client_id, get("device_code").unwrap_or_default())
            }
            _ => Err(err(400, "unsupported_grant_type", "unsupported grant_type")),
        }
    }

    fn redeem_code(
        &self,
        client_id: &str,
        code: &str,
        redirect_uri: &str,
        verifier: &str,
    ) -> Result<TokenResponse, OAuthError> {
        let hash = token_hash(code);
        let removed = self.eph().codes.remove(&hash);
        let Some(rec) = removed else {
            // A code used twice: revoke what the first use gave (RFC 6749
            // §4.1.2, RFC 9700 §4.5).
            let first = self.eph().redeemed.remove(&hash);
            if let Some((family, _)) = first {
                tracing::warn!("authorization code reused: tokens revoked");
                let _ = self.store.revoke_family(&family);
            }
            return Err(invalid_grant("unknown or used code"));
        };
        if rec.client_id != client_id || rec.redirect_uri != redirect_uri {
            return Err(invalid_grant(
                "the code belongs to another client or redirect URI",
            ));
        }
        if self.now() - rec.created >= self.cfg.tokens.code_s {
            return Err(invalid_grant("code expired"));
        }
        if !pkce::matches(verifier, &rec.challenge) {
            return Err(invalid_grant("PKCE verification failed"));
        }
        if let Some(identity) = &rec.link {
            let now = self.now();
            self.store
                .link(
                    &rec.account,
                    Identity {
                        provider: identity.provider,
                        subject: identity.subject.clone(),
                        name: identity.name.clone(),
                        email: identity.email.clone(),
                        linked_at: now,
                        upload: identity.upload,
                    },
                )
                .map_err(|e| invalid_grant(&e.to_string()))?;
            self.keep_tokens(&rec.account, identity)
                .map_err(|e| err(500, "server_error", e.to_string()))?;
            tracing::info!(account = %rec.account, provider = %identity.provider, "provider linked");
        }
        let account = self
            .store
            .account(&rec.account)
            .ok_or_else(|| invalid_grant("no account"))?;
        let family = random_token();
        let resp = self.issue(
            &account,
            client_id,
            &rec.scope,
            rec.auth_time,
            rec.nonce,
            Some((family.clone(), self.now())),
        )?;
        self.eph().redeemed.insert(hash, (family, self.now()));
        Ok(resp)
    }

    fn refresh(&self, client_id: &str, token: &str) -> Result<TokenResponse, OAuthError> {
        let store = |e: StoreError| err(500, "server_error", e.to_string());
        let Some(rec) = self.store.use_refresh(&token_hash(token)).map_err(store)? else {
            return Err(invalid_grant("unknown refresh token"));
        };
        if rec.client_id != client_id {
            return Err(invalid_grant("the refresh token belongs to another client"));
        }
        if rec.revoked {
            return Err(invalid_grant("refresh token revoked"));
        }
        if rec.used {
            // Reuse of a rotated token: someone has a copy. End the sign-in.
            tracing::warn!(account = %rec.account, "refresh token reuse detected: sign-in revoked");
            self.store.revoke_family(&rec.family).map_err(store)?;
            return Err(invalid_grant("refresh token reused; sign in again"));
        }
        let now = self.now();
        let t = &self.cfg.tokens;
        if now - rec.created_at > t.refresh_idle_s || now - rec.family_created > t.refresh_max_s {
            return Err(invalid_grant("refresh token expired"));
        }
        let account = self
            .store
            .account(&rec.account)
            .ok_or_else(|| invalid_grant("no account"))?;
        self.issue(
            &account,
            client_id,
            &rec.scope,
            rec.auth_time,
            None,
            Some((rec.family, rec.family_created)),
        )
    }

    /// Handles `/revoke` (RFC 7009): a refresh token ends its sign-in.
    /// Always succeeds (no oracle).
    pub fn revoke(&self, form: &[(String, String)]) {
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let token = get("token").unwrap_or_default();
        if let Some(rec) = self.store.refresh(&token_hash(token))
            && get("client_id") == Some(rec.client_id.as_str())
        {
            let _ = self.store.revoke_family(&rec.family);
        }
    }

    // ---- device flow -----------------------------------------------------

    /// Handles `/device_authorization`.
    pub fn device_authorization(
        &self,
        form: &[(String, String)],
    ) -> Result<DeviceAuthorization, OAuthError> {
        let get = |k: &str| form.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
        let client = get("client_id")
            .and_then(|id| self.client(id))
            .ok_or_else(|| err(401, "invalid_client", "unknown client"))?;
        if !client.device {
            return Err(err(
                400,
                "unauthorized_client",
                "this app does not use the device flow",
            ));
        }
        let device_code = random_token();
        let now = self.now();
        let t = &self.cfg.tokens;
        let mut eph = self.eph();
        let user_code = loop {
            let r = crate::random_bytes::<8>();
            let c: String = r
                .iter()
                .map(|b| USER_CODE_CHARS[*b as usize % USER_CODE_CHARS.len()] as char)
                .collect();
            let c = format!("{}-{}", &c[..4], &c[4..]);
            if !eph.user_codes.contains_key(&c) {
                break c;
            }
        };
        let hash = token_hash(&device_code);
        eph.user_codes.insert(user_code.clone(), hash.clone());
        eph.devices.insert(
            hash,
            DeviceGrant {
                client_id: client.id.clone(),
                scope: get("scope").unwrap_or("openid").to_owned(),
                user_code: user_code.clone(),
                expires_at: now + t.device_s,
                interval: t.device_interval_s,
                last_poll: None,
                status: DeviceStatus::Pending,
            },
        );
        let uri = format!("{}/device", self.cfg.issuer);
        Ok(DeviceAuthorization {
            device_code,
            verification_uri_complete: Some(format!("{uri}?user_code={user_code}")),
            user_code,
            verification_uri: uri,
            expires_in: t.device_s,
            interval: t.device_interval_s,
        })
    }

    fn poll_device(&self, client_id: &str, device_code: &str) -> Result<TokenResponse, OAuthError> {
        let now = self.now();
        let hash = token_hash(device_code);
        let mut eph = self.eph();
        let Some(d) = eph.devices.get_mut(&hash) else {
            return Err(err(400, "expired_token", "unknown or expired device code"));
        };
        if d.client_id != client_id {
            return Err(invalid_grant("the device code belongs to another client"));
        }
        if now >= d.expires_at {
            let user_code = d.user_code.clone();
            eph.devices.remove(&hash);
            eph.user_codes.remove(&user_code);
            return Err(err(400, "expired_token", "the code expired"));
        }
        if let Some(last) = d.last_poll
            && now - last < d.interval
        {
            d.interval += 5;
            d.last_poll = Some(now);
            return Err(err(
                400,
                "slow_down",
                format!("poll every {} s", d.interval),
            ));
        }
        d.last_poll = Some(now);
        match d.status.clone() {
            DeviceStatus::Pending => Err(err(
                400,
                "authorization_pending",
                "waiting for the confirmation",
            )),
            DeviceStatus::Denied => {
                let user_code = d.user_code.clone();
                eph.devices.remove(&hash);
                eph.user_codes.remove(&user_code);
                Err(err(400, "access_denied", "the sign-in was cancelled"))
            }
            DeviceStatus::Approved { account, auth_time } => {
                let (user_code, scope) = (d.user_code.clone(), d.scope.clone());
                eph.devices.remove(&hash);
                eph.user_codes.remove(&user_code);
                drop(eph);
                let account = self
                    .store
                    .account(&account)
                    .ok_or_else(|| invalid_grant("no account"))?;
                self.issue(&account, client_id, &scope, auth_time, None, None)
            }
        }
    }

    /// `XXXX-XXXX` from what the user typed (case, spaces and dashes
    /// ignored), if it can be a code.
    pub fn normalize_user_code(input: &str) -> Option<String> {
        let c: String = input
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_uppercase())
            .collect();
        (c.len() == 8 && c.bytes().all(|b| USER_CODE_CHARS.contains(&b)))
            .then(|| format!("{}-{}", &c[..4], &c[4..]))
    }

    /// The app waiting for this code (its name), if the code is pending.
    pub fn device_lookup(&self, user_code: &str) -> Option<String> {
        let code = Self::normalize_user_code(user_code)?;
        let now = self.now();
        let eph = self.eph();
        let d = eph.devices.get(eph.user_codes.get(&code)?)?;
        (d.status == DeviceStatus::Pending && now < d.expires_at).then(|| {
            self.client(&d.client_id)
                .map_or_else(|| d.client_id.clone(), |c| c.name.clone())
        })
    }

    /// The user confirmed the code with a provider: its sign-in URL.
    pub fn device_start(&self, user_code: &str, provider: Provider) -> Result<String, String> {
        let code = Self::normalize_user_code(user_code).ok_or("not a code")?;
        let hash = self
            .eph()
            .user_codes
            .get(&code)
            .cloned()
            .ok_or("unknown or expired code")?;
        let upstream = self
            .upstream(provider)
            .ok_or("this provider is not available")?;
        self.start_provider(upstream, Target::Device(hash))
    }

    /// The user cancelled on the device page.
    pub fn device_deny(&self, user_code: &str) {
        let Some(code) = Self::normalize_user_code(user_code) else {
            return;
        };
        let mut eph = self.eph();
        if let Some(hash) = eph.user_codes.get(&code).cloned()
            && let Some(d) = eph.devices.get_mut(&hash)
            && d.status == DeviceStatus::Pending
        {
            d.status = DeviceStatus::Denied;
        }
    }

    // ---- account API -----------------------------------------------------

    /// The claims of a bearer access token for our own API.
    pub fn bearer(&self, authorization: Option<&str>) -> Result<AccessClaims, OAuthError> {
        let token = authorization
            .and_then(|h| {
                h.strip_prefix("Bearer ")
                    .or_else(|| h.strip_prefix("bearer "))
            })
            .ok_or_else(|| err(401, "invalid_token", "a bearer token is required"))?;
        let claims = self
            .verifier()
            .verify(token.trim(), self.now())
            .map_err(|e| err(401, "invalid_token", e.to_string()))?;
        if self.store.account(&claims.sub).is_none() {
            return Err(err(401, "invalid_token", "the account no longer exists"));
        }
        Ok(claims)
    }

    fn user_info(&self, account: &Account) -> UserInfo {
        UserInfo {
            sub: account.id.clone(),
            name: account.display_name.clone(),
            providers: account
                .identities
                .iter()
                .map(|i| LinkedProvider {
                    provider: i.provider,
                    name: i.name.clone(),
                    linked_at: i.linked_at,
                    upload: i.upload,
                })
                .collect(),
            email: account.identities.iter().find_map(|i| {
                (i.provider == Provider::Google)
                    .then(|| i.email.clone())
                    .flatten()
            }),
        }
    }

    pub fn userinfo(&self, authorization: Option<&str>) -> Result<UserInfo, OAuthError> {
        let c = self.bearer(authorization)?;
        let a = self
            .store
            .account(&c.sub)
            .ok_or_else(|| err(401, "invalid_token", "no account"))?;
        Ok(self.user_info(&a))
    }

    /// `POST /account/link`: a ticket for linking another provider, bound
    /// to the app's PKCE challenge of the sign-in that follows.
    pub fn link_ticket(
        &self,
        authorization: Option<&str>,
        form: &[(String, String)],
    ) -> Result<LinkTicket, OAuthError> {
        let c = self.bearer(authorization)?;
        let challenge = form
            .iter()
            .find(|(k, _)| k == "code_challenge")
            .map(|(_, v)| v.as_str())
            .filter(|ch| pkce::valid_challenge(ch))
            .ok_or_else(|| err(400, "invalid_request", "code_challenge (S256) is required"))?;
        let ticket = random_token();
        let ttl = self.cfg.tokens.link_ticket_s;
        self.eph().tickets.insert(
            token_hash(&ticket),
            TicketRec {
                account: c.sub,
                client_id: c.client_id,
                challenge: challenge.to_owned(),
                expires_at: self.now() + ttl,
            },
        );
        Ok(LinkTicket {
            link_ticket: ticket,
            expires_in: ttl,
        })
    }

    /// `DELETE /account/providers/{provider}`.
    pub fn unlink(
        &self,
        authorization: Option<&str>,
        provider: Provider,
    ) -> Result<UserInfo, OAuthError> {
        let c = self.bearer(authorization)?;
        self.store.unlink(&c.sub, provider).map_err(|e| match e {
            StoreError::LastIdentity(_) | StoreError::NotLinked(_) => {
                err(409, "conflict", e.to_string())
            }
            e => err(500, "server_error", e.to_string()),
        })?;
        tracing::info!(account = %c.sub, %provider, "provider unlinked");
        let a = self
            .store
            .account(&c.sub)
            .ok_or_else(|| err(401, "invalid_token", "no account"))?;
        Ok(self.user_info(&a))
    }

    /// `GET /account/export` (spec §12): the account and every module's
    /// data. Never tokens.
    pub fn export(&self, authorization: Option<&str>) -> Result<serde_json::Value, OAuthError> {
        let c = self.bearer(authorization)?;
        let a = self
            .store
            .account(&c.sub)
            .ok_or_else(|| err(401, "invalid_token", "no account"))?;
        let mut modules = serde_json::Map::new();
        for h in self.hooks.read().unwrap_or_else(|e| e.into_inner()).iter() {
            modules.insert(h.name().to_owned(), h.export(&a.id));
        }
        Ok(serde_json::json!({
            "format": "ocw-account-export/1",
            "exported_at": self.now(),
            "account": {
                "id": a.id,
                "display_name": a.display_name,
                "created_at": a.created_at,
                "identities": a.identities.iter().map(|i| serde_json::json!({
                    "provider": i.provider,
                    "subject": i.subject,
                    "name": i.name,
                    "email": i.email,
                    "linked_at": i.linked_at,
                    "upload": i.upload,
                })).collect::<Vec<_>>(),
            },
            "modules": modules,
        }))
    }

    /// `DELETE /account` (spec §12): wants a recent sign-in; deletes the
    /// account, its identities, tokens and secrets, and tells every module.
    pub fn delete_account(&self, authorization: Option<&str>) -> Result<(), OAuthError> {
        let c = self.bearer(authorization)?;
        if self.now() - c.auth_time > self.cfg.tokens.recent_auth_s {
            return Err(err(
                403,
                "login_required",
                "sign in again to delete the account",
            ));
        }
        for h in self.hooks.read().unwrap_or_else(|e| e.into_inner()).iter() {
            h.delete(&c.sub);
        }
        self.store
            .delete_account(&c.sub)
            .map_err(|e| err(500, "server_error", e.to_string()))?;
        tracing::info!(account = %c.sub, "account deleted");
        Ok(())
    }
}

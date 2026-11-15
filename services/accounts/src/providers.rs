//! The upstream providers (spec §9.1): Google through `openidconnect`
//! (discovery, authorization code with PKCE and nonce, ID-token
//! validation) and Strava through `oauth2` (authorization code; the token
//! response carries the athlete, no e-mail). Synchronous HTTP: the service
//! calls them on Tokio's blocking pool.

use std::sync::Mutex;

use ocw_auth::Provider;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    AuthorizationCode, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse as _,
};
use serde::{Deserialize, Serialize};

use crate::config::{GoogleConfig, StravaConfig};
use crate::limits::StravaBudget;
use crate::secrets::ProviderTokens;

/// Who signed in at a provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderIdentity {
    pub provider: Provider,
    /// Google `sub`, Strava athlete id.
    pub subject: String,
    pub name: String,
    pub email: Option<String>,
    /// Strava's tokens (kept encrypted for the later upload, §2.8).
    pub tokens: Option<ProviderTokens>,
    /// Strava: `activity:write` granted.
    pub upload: bool,
}

/// What a provider sign-in needs from the service.
pub struct StartRequest<'a> {
    /// Our `state` towards the provider.
    pub state: &'a str,
    pub nonce: &'a str,
    /// Our PKCE verifier towards the provider.
    pub verifier: &'a str,
}

/// One upstream provider.
pub trait Upstream: Send + Sync {
    fn provider(&self) -> Provider;
    /// The provider's authorization URL.
    fn start(&self, req: &StartRequest<'_>, now: i64) -> Result<String, String>;
    /// Exchanges the code; `scope` is what the callback's query said was
    /// granted (Strava).
    fn finish(
        &self,
        code: &str,
        req: &StartRequest<'_>,
        scope: Option<&str>,
        now: i64,
    ) -> Result<ProviderIdentity, String>;
}

/// Google's provider metadata (with its keys) is fetched once an hour.
const DISCOVERY_TTL_S: i64 = 3600;

pub struct Google {
    cfg: GoogleConfig,
    redirect: String,
    metadata: Mutex<Option<(CoreProviderMetadata, i64)>>,
}

impl Google {
    pub fn new(cfg: GoogleConfig, issuer: &str) -> Self {
        Self {
            cfg,
            redirect: format!("{issuer}/callback/google"),
            metadata: Mutex::new(None),
        }
    }

    fn metadata(&self, now: i64, force: bool) -> Result<CoreProviderMetadata, String> {
        let mut m = self.metadata.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((meta, at)) = &*m
            && !force
            && now - at < DISCOVERY_TTL_S
        {
            return Ok(meta.clone());
        }
        let issuer = IssuerUrl::new(self.cfg.issuer.clone()).map_err(|e| e.to_string())?;
        let meta = CoreProviderMetadata::discover(&issuer, &crate::http::client)
            .map_err(|e| format!("Google discovery failed: {e}"))?;
        *m = Some((meta.clone(), now));
        Ok(meta)
    }

    fn client(
        &self,
        meta: CoreProviderMetadata,
    ) -> Result<
        CoreClient<
            openidconnect::EndpointSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointMaybeSet,
            openidconnect::EndpointMaybeSet,
        >,
        String,
    > {
        Ok(CoreClient::from_provider_metadata(
            meta,
            ClientId::new(self.cfg.client_id.clone()),
            Some(ClientSecret::new(self.cfg.client_secret.clone())),
        )
        .set_redirect_uri(RedirectUrl::new(self.redirect.clone()).map_err(|e| e.to_string())?))
    }
}

impl Upstream for Google {
    fn provider(&self) -> Provider {
        Provider::Google
    }

    fn start(&self, req: &StartRequest<'_>, now: i64) -> Result<String, String> {
        let client = self.client(self.metadata(now, false)?)?;
        let (state, nonce) = (req.state.to_owned(), req.nonce.to_owned());
        let challenge = PkceCodeChallenge::from_code_verifier_sha256(&PkceCodeVerifier::new(
            req.verifier.into(),
        ));
        let (url, _, _) = client
            .authorize_url(
                CoreAuthenticationFlow::AuthorizationCode,
                move || CsrfToken::new(state),
                move || Nonce::new(nonce),
            )
            .add_scope(Scope::new("profile".into()))
            .add_scope(Scope::new("email".into()))
            .set_pkce_challenge(challenge)
            .url();
        Ok(url.to_string())
    }

    fn finish(
        &self,
        code: &str,
        req: &StartRequest<'_>,
        _scope: Option<&str>,
        now: i64,
    ) -> Result<ProviderIdentity, String> {
        let client = self.client(self.metadata(now, false)?)?;
        let token = client
            .exchange_code(AuthorizationCode::new(code.into()))
            .map_err(|e| e.to_string())?
            .set_pkce_verifier(PkceCodeVerifier::new(req.verifier.into()))
            .request(&crate::http::client)
            .map_err(|e| format!("Google token exchange failed: {e}"))?;
        let id_token = token.id_token().ok_or("Google sent no ID token")?;
        let nonce = Nonce::new(req.nonce.into());
        let claims = match id_token.claims(&client.id_token_verifier(), &nonce) {
            Ok(c) => c.clone(),
            // Google may have rotated its keys since the last discovery:
            // check the same token once more with fresh metadata.
            Err(_) => {
                let fresh = self.client(self.metadata(now, true)?)?;
                id_token
                    .claims(&fresh.id_token_verifier(), &nonce)
                    .map_err(|e| format!("Google ID token refused: {e}"))?
                    .clone()
            }
        };
        let name = claims
            .name()
            .and_then(|n| n.get(None))
            .map(|n| n.as_str().to_owned())
            .or_else(|| {
                claims
                    .given_name()
                    .and_then(|n| n.get(None))
                    .map(|n| n.as_str().to_owned())
            })
            .unwrap_or_default();
        let email = claims
            .email()
            .filter(|_| claims.email_verified().unwrap_or(false))
            .map(|e| e.as_str().to_owned());
        Ok(ProviderIdentity {
            provider: Provider::Google,
            subject: claims.subject().as_str().to_owned(),
            name,
            email,
            tokens: None,
            upload: false,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Athlete {
    pub id: i64,
    #[serde(default)]
    pub firstname: Option<String>,
    #[serde(default)]
    pub lastname: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StravaExtra {
    #[serde(default)]
    pub athlete: Option<Athlete>,
    #[serde(default)]
    pub expires_at: Option<i64>,
}

impl oauth2::ExtraTokenFields for StravaExtra {}

type StravaTokenResponse =
    oauth2::StandardTokenResponse<StravaExtra, oauth2::basic::BasicTokenType>;
type StravaClient = oauth2::Client<
    oauth2::basic::BasicErrorResponse,
    StravaTokenResponse,
    oauth2::basic::BasicTokenIntrospectionResponse,
    oauth2::StandardRevocableToken,
    oauth2::basic::BasicRevocationErrorResponse,
    oauth2::EndpointSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointNotSet,
    oauth2::EndpointSet,
>;

/// Strava's scope separator is a comma.
pub const STRAVA_SCOPE: &str = "read";
pub const STRAVA_UPLOAD_SCOPE: &str = "activity:write";

pub struct Strava {
    cfg: StravaConfig,
    redirect: String,
    budget: Mutex<StravaBudget>,
}

impl Strava {
    pub fn new(cfg: StravaConfig, issuer: &str) -> Self {
        let budget = StravaBudget::new(cfg.limit_15min, cfg.limit_day);
        Self {
            cfg,
            redirect: format!("{issuer}/callback/strava"),
            budget: Mutex::new(budget),
        }
    }

    fn client(&self) -> Result<StravaClient, String> {
        Ok(
            StravaClient::new(oauth2::ClientId::new(self.cfg.client_id.clone()))
                .set_client_secret(oauth2::ClientSecret::new(self.cfg.client_secret.clone()))
                .set_auth_uri(
                    oauth2::AuthUrl::new(self.cfg.authorize_url.clone())
                        .map_err(|e| e.to_string())?,
                )
                .set_token_uri(
                    oauth2::TokenUrl::new(self.cfg.token_url.clone()).map_err(|e| e.to_string())?,
                )
                .set_redirect_uri(
                    oauth2::RedirectUrl::new(self.redirect.clone()).map_err(|e| e.to_string())?,
                )
                // Strava wants the credentials in the form body.
                .set_auth_type(oauth2::AuthType::RequestBody),
        )
    }

    fn scope(&self) -> String {
        if self.cfg.ask_upload {
            format!("{STRAVA_SCOPE},{STRAVA_UPLOAD_SCOPE}")
        } else {
            STRAVA_SCOPE.into()
        }
    }
}

impl Upstream for Strava {
    fn provider(&self) -> Provider {
        Provider::Strava
    }

    fn start(&self, req: &StartRequest<'_>, _now: i64) -> Result<String, String> {
        let state = req.state.to_owned();
        let mut auth = self
            .client()?
            .authorize_url(move || oauth2::CsrfToken::new(state))
            .add_scope(oauth2::Scope::new(self.scope()))
            .add_extra_param("approval_prompt", "auto");
        if self.cfg.pkce {
            auth = auth.set_pkce_challenge(oauth2::PkceCodeChallenge::from_code_verifier_sha256(
                &oauth2::PkceCodeVerifier::new(req.verifier.into()),
            ));
        }
        Ok(auth.url().0.to_string())
    }

    fn finish(
        &self,
        code: &str,
        req: &StartRequest<'_>,
        scope: Option<&str>,
        now: i64,
    ) -> Result<ProviderIdentity, String> {
        self.budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take(now)
            .map_err(|wait| {
                format!(
                    "Strava is busy (rate limit): try again in {} min",
                    wait.div_ceil(60)
                )
            })?;
        let hook = |h: &http::HeaderMap| {
            let get = |k: &str| h.get(k).and_then(|v| v.to_str().ok());
            self.budget
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .observe(get("x-ratelimit-limit"), get("x-ratelimit-usage"), now);
        };
        let hook_ref: crate::http::HeaderHook<'_> = &hook;
        let http_client = |r: oauth2::HttpRequest| crate::http::call(r, Some(hook_ref));
        let mut exchange = self
            .client()?
            .exchange_code(oauth2::AuthorizationCode::new(code.into()));
        if self.cfg.pkce {
            exchange =
                exchange.set_pkce_verifier(oauth2::PkceCodeVerifier::new(req.verifier.into()));
        }
        let token = exchange
            .request(&http_client)
            .map_err(|e| format!("Strava token exchange failed: {e}"))?;
        use oauth2::TokenResponse as _;
        let athlete = token
            .extra_fields()
            .athlete
            .clone()
            .ok_or("Strava sent no athlete")?;
        let name = [athlete.firstname.clone(), athlete.lastname.clone()]
            .into_iter()
            .flatten()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let granted: Vec<String> = scope
            .unwrap_or_default()
            .split([',', ' '])
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        let upload = granted.iter().any(|s| s == STRAVA_UPLOAD_SCOPE);
        let expires_at = token
            .extra_fields()
            .expires_at
            .unwrap_or_else(|| now + token.expires_in().map_or(21_600, |d| d.as_secs() as i64));
        Ok(ProviderIdentity {
            provider: Provider::Strava,
            subject: athlete.id.to_string(),
            name,
            email: None,
            tokens: Some(ProviderTokens {
                access_token: token.access_token().secret().clone(),
                refresh_token: token
                    .refresh_token()
                    .map(|t| t.secret().clone())
                    .unwrap_or_default(),
                expires_at,
                scope: granted,
            }),
            upload,
        })
    }
}

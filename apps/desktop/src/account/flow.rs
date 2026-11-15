//! The desktop's side of the sign-in (spec §9.1), Bevy-free: the system
//! browser and a loopback redirect with PKCE (RFC 8252 §7.3: an ephemeral
//! port on 127.0.0.1, any port allowed by the backend), the device flow
//! for kiosks (RFC 8628), refresh, revocation and the account API. HTTP
//! with ureq; the forms and checks come from `ocw_auth::client`.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use ocw_auth::Provider;
use ocw_auth::client::{self, Callback, PendingLogin, Poll, Session, TokenError};
use ocw_auth::oidc::{DeviceAuthorization, Discovery, LinkTicket, UserInfo};

/// How long the browser part of a sign-in may take.
pub const BROWSER_TIMEOUT: Duration = Duration::from_secs(300);

/// One accounts backend, as an app client.
pub struct AuthClient {
    pub issuer: String,
    /// The browser sign-in's client (`ocw-desktop`).
    pub client_id: String,
    /// The device flow's client (`ocw-kiosk`): a kiosk signs in with a code.
    pub device_client_id: String,
    agent: ureq::Agent,
    discovery: Option<Discovery>,
}

/// What a call to the backend can end with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlowError {
    /// The sign-in is gone (refresh token revoked, expired, reused): the
    /// user signs in again.
    SignInAgain(String),
    /// The user cancelled, or the flow timed out.
    Cancelled(String),
    /// Anything else (network, server).
    Failed(String),
}

impl std::fmt::Display for FlowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SignInAgain(m) | Self::Cancelled(m) | Self::Failed(m) => f.write_str(m),
        }
    }
}

impl From<TokenError> for FlowError {
    fn from(e: TokenError) -> Self {
        match e {
            TokenError::InvalidGrant(m) => Self::SignInAgain(m),
            TokenError::Other(m) => Self::Failed(m),
        }
    }
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("the OS random source works");
    b
}

/// Seconds since the epoch.
pub fn now_s() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl AuthClient {
    pub fn new(issuer: &str, client_id: &str) -> Self {
        Self {
            issuer: issuer.trim_end_matches('/').to_owned(),
            client_id: client_id.to_owned(),
            device_client_id: ocw_auth::CLIENT_KIOSK.to_owned(),
            agent: ureq::Agent::config_builder()
                .http_status_as_error(false)
                .timeout_global(Some(Duration::from_secs(20)))
                .build()
                .into(),
            discovery: None,
        }
    }

    fn read(
        r: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<(u16, String), FlowError> {
        let mut r =
            r.map_err(|e| FlowError::Failed(format!("the account server cannot be reached: {e}")))?;
        let status = r.status().as_u16();
        let body = r
            .body_mut()
            .read_to_string()
            .map_err(|e| FlowError::Failed(e.to_string()))?;
        Ok((status, body))
    }

    fn post(
        &self,
        url: &str,
        form: &str,
        bearer: Option<&str>,
    ) -> Result<(u16, String), FlowError> {
        let mut req = self
            .agent
            .post(url)
            .header("content-type", "application/x-www-form-urlencoded");
        if let Some(t) = bearer {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        Self::read(req.send(form))
    }

    fn get(&self, url: &str, bearer: Option<&str>) -> Result<(u16, String), FlowError> {
        let mut req = self.agent.get(url);
        if let Some(t) = bearer {
            req = req.header("authorization", &format!("Bearer {t}"));
        }
        Self::read(req.call())
    }

    fn delete(&self, url: &str, bearer: &str) -> Result<(u16, String), FlowError> {
        Self::read(
            self.agent
                .delete(url)
                .header("authorization", &format!("Bearer {bearer}"))
                .call(),
        )
    }

    /// The backend's discovery document (fetched once).
    pub fn discovery(&mut self) -> Result<Discovery, FlowError> {
        if let Some(d) = &self.discovery {
            return Ok(d.clone());
        }
        let (status, body) = self.get(
            &format!("{}/.well-known/openid-configuration", self.issuer),
            None,
        )?;
        if status != 200 {
            return Err(FlowError::Failed(format!(
                "the account server answered HTTP {status}"
            )));
        }
        let d: Discovery = serde_json::from_str(&body)
            .map_err(|e| FlowError::Failed(format!("bad discovery document: {e}")))?;
        if d.issuer != self.issuer {
            return Err(FlowError::Failed(format!(
                "the account server names another issuer ({})",
                d.issuer
            )));
        }
        self.discovery = Some(d.clone());
        Ok(d)
    }

    fn account_url(&mut self, path: &str) -> Result<String, FlowError> {
        let d = self.discovery()?;
        let base = d
            .ocw_account_endpoint
            .unwrap_or_else(|| format!("{}/account", self.issuer));
        Ok(format!("{base}{path}"))
    }

    /// Signs in (or, with `link`, links `provider` to that account) in the
    /// system browser: `open` opens the URL, the redirect comes back to a
    /// loopback listener. `cancel` ends the wait.
    pub fn sign_in(
        &mut self,
        provider: Option<Provider>,
        link: Option<&Session>,
        open: &dyn Fn(&str) -> Result<(), String>,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> Result<Session, FlowError> {
        let d = self.discovery()?;
        let listener =
            TcpListener::bind(("127.0.0.1", 0)).map_err(|e| FlowError::Failed(e.to_string()))?;
        let port = listener
            .local_addr()
            .map_err(|e| FlowError::Failed(e.to_string()))?
            .port();
        let redirect = format!("http://127.0.0.1:{port}{}", client::LOOPBACK_PATH);
        let mut pending = PendingLogin::new(&random::<96>(), &redirect, provider);
        if let Some(s) = link {
            let (status, body) = self.post(
                &self.account_url("/link")?,
                &client::form(&[("code_challenge", &pending.challenge())]),
                Some(&s.access_token),
            )?;
            if status != 200 {
                return Err(FlowError::Failed(format!(
                    "linking refused (HTTP {status})"
                )));
            }
            let t: LinkTicket =
                serde_json::from_str(&body).map_err(|e| FlowError::Failed(e.to_string()))?;
            pending.link_ticket = Some(t.link_ticket);
        }
        let url = pending.authorize_url(&d.authorization_endpoint, &self.client_id);
        open(&url).map_err(FlowError::Failed)?;
        let code = wait_for_code(&listener, &pending, &self.issuer, cancel, timeout)?;
        let (status, body) = self.post(
            &d.token_endpoint,
            &pending.code_form(&self.client_id, &code),
            None,
        )?;
        let t = client::parse_token_response(status, &body)?;
        Session::from_response(
            &t,
            now_s(),
            &self.issuer,
            &self.client_id,
            Some(&pending.nonce),
            None,
        )
        .map_err(FlowError::Failed)
    }

    /// Starts the device flow (kiosk, headset).
    pub fn device_start(&mut self) -> Result<DeviceAuthorization, FlowError> {
        let d = self.discovery()?;
        let (status, body) = self.post(
            &d.device_authorization_endpoint,
            &client::device_start_form(&self.device_client_id),
            None,
        )?;
        if status != 200 {
            return Err(FlowError::Failed(format!(
                "the device sign-in was refused (HTTP {status})"
            )));
        }
        serde_json::from_str(&body).map_err(|e| FlowError::Failed(e.to_string()))
    }

    /// Polls until the code is confirmed (or denied, expired, cancelled).
    pub fn device_wait(
        &mut self,
        auth: &DeviceAuthorization,
        cancel: &AtomicBool,
        sleep: &dyn Fn(Duration),
    ) -> Result<Session, FlowError> {
        let d = self.discovery()?;
        let mut interval = auth.interval.max(1);
        let deadline = now_s() + auth.expires_in;
        loop {
            // Sleep in short steps so a cancel is quick.
            for _ in 0..interval * 4 {
                if cancel.load(Ordering::Relaxed) {
                    return Err(FlowError::Cancelled("sign-in cancelled".into()));
                }
                sleep(Duration::from_millis(250));
            }
            if now_s() > deadline + 5 {
                return Err(FlowError::Cancelled("the code expired".into()));
            }
            let (status, body) = self.post(
                &d.token_endpoint,
                &client::device_poll_form(&self.device_client_id, &auth.device_code),
                None,
            )?;
            match client::parse_poll(status, &body) {
                Poll::Pending => {}
                Poll::SlowDown => interval += 5,
                Poll::Denied => {
                    return Err(FlowError::Cancelled(
                        "the sign-in was cancelled on the phone".into(),
                    ));
                }
                Poll::Expired => return Err(FlowError::Cancelled("the code expired".into())),
                Poll::Failed(e) => return Err(FlowError::Failed(e)),
                Poll::Tokens(t) => {
                    return Session::from_response(
                        &t,
                        now_s(),
                        &self.issuer,
                        &self.device_client_id,
                        None,
                        None,
                    )
                    .map_err(FlowError::Failed);
                }
            }
        }
    }

    /// New tokens for a session (the refresh token rotates).
    pub fn refresh(&mut self, s: &Session) -> Result<Session, FlowError> {
        let d = self.discovery()?;
        let rt = s
            .refresh_token
            .as_deref()
            .ok_or_else(|| FlowError::SignInAgain("no refresh token".into()))?;
        // The session's own client (a kiosk's code sign-in is `ocw-kiosk`).
        let (status, body) = self.post(
            &d.token_endpoint,
            &client::refresh_form(&s.client_id, rt),
            None,
        )?;
        let t = client::parse_token_response(status, &body)?;
        Session::from_response(&t, now_s(), &self.issuer, &s.client_id, None, Some(s))
            .map_err(FlowError::Failed)
    }

    /// Ends the sign-in at the backend (the refresh token's family).
    pub fn revoke(&mut self, s: &Session) -> Result<(), FlowError> {
        let d = self.discovery()?;
        if let Some(rt) = &s.refresh_token {
            self.post(
                &d.revocation_endpoint,
                &client::revoke_form(&s.client_id, rt),
                None,
            )?;
        }
        Ok(())
    }

    pub fn userinfo(&mut self, s: &Session) -> Result<UserInfo, FlowError> {
        let d = self.discovery()?;
        let (status, body) = self.get(&d.userinfo_endpoint, Some(&s.access_token))?;
        match status {
            200 => serde_json::from_str(&body).map_err(|e| FlowError::Failed(e.to_string())),
            401 => Err(FlowError::SignInAgain(
                "the account server refused the token".into(),
            )),
            s => Err(FlowError::Failed(format!("userinfo: HTTP {s}"))),
        }
    }

    /// Unlinks a provider (never the last one: the backend refuses).
    pub fn unlink(&mut self, s: &Session, p: Provider) -> Result<UserInfo, FlowError> {
        let url = self.account_url(&format!("/providers/{}", p.id()))?;
        let (status, body) = self.delete(&url, &s.access_token)?;
        match status {
            200 => serde_json::from_str(&body).map_err(|e| FlowError::Failed(e.to_string())),
            _ => Err(FlowError::Failed(error_text(status, &body))),
        }
    }

    /// The account's data export (spec §12), as JSON text.
    pub fn export(&mut self, s: &Session) -> Result<String, FlowError> {
        let url = self.account_url("/export")?;
        let (status, body) = self.get(&url, Some(&s.access_token))?;
        match status {
            200 => Ok(body),
            _ => Err(FlowError::Failed(error_text(status, &body))),
        }
    }

    /// Deletes the account (spec §12; wants a recent sign-in).
    pub fn delete_account(&mut self, s: &Session) -> Result<(), FlowError> {
        let url = self.account_url("")?;
        let (status, body) = self.delete(&url, &s.access_token)?;
        match status {
            204 | 200 => Ok(()),
            403 => Err(FlowError::SignInAgain(
                "sign in again first (deleting wants a fresh sign-in)".into(),
            )),
            _ => Err(FlowError::Failed(error_text(status, &body))),
        }
    }
}

fn error_text(status: u16, body: &str) -> String {
    serde_json::from_str::<ocw_auth::oidc::ErrorResponse>(body)
        .ok()
        .and_then(|e| e.error_description.or(Some(e.error)))
        .unwrap_or_else(|| format!("HTTP {status}"))
}

/// The page the browser shows after the redirect.
fn answer_page(ok: bool) -> &'static str {
    if ok {
        "Signed in to Open Cycling World. You can close this tab and go back to the app."
    } else {
        "Not signed in. You can close this tab and try again in the app."
    }
}

/// Waits for the browser's redirect on the loopback listener and answers
/// it with a short page; ignores other requests (a favicon, a forged
/// `state`).
pub fn wait_for_code(
    listener: &TcpListener,
    pending: &PendingLogin,
    issuer: &str,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<String, FlowError> {
    listener
        .set_nonblocking(true)
        .map_err(|e| FlowError::Failed(e.to_string()))?;
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cancel.load(Ordering::Relaxed) {
            return Err(FlowError::Cancelled("sign-in cancelled".into()));
        }
        let (stream, _) = match listener.accept() {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(e) => return Err(FlowError::Failed(e.to_string())),
        };
        let _ = stream.set_nonblocking(false);
        let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
        let mut line = String::new();
        if BufReader::new(&stream).read_line(&mut line).is_err() {
            continue;
        }
        let mut parts = line.split_whitespace();
        let result = match (parts.next(), parts.next()) {
            (Some("GET"), Some(target)) => {
                client::parse_callback(target, client::LOOPBACK_PATH, pending, issuer)
            }
            _ => Callback::Ignore,
        };
        let (status, text) = match &result {
            Callback::Code(_) => ("200 OK", answer_page(true)),
            Callback::Denied(_) => ("200 OK", answer_page(false)),
            Callback::Ignore => ("404 Not Found", "Not found."),
        };
        let page = format!(
            "<!doctype html><meta charset=utf-8><title>Open Cycling World</title><p style=\"font:20px system-ui;margin:3em\">{text}</p>"
        );
        let mut s = &stream;
        let _ = write!(
            s,
            "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}",
            page.len()
        );
        match result {
            Callback::Code(c) => return Ok(c),
            Callback::Denied(e) => return Err(FlowError::Cancelled(format!("not signed in: {e}"))),
            Callback::Ignore => {}
        }
    }
    Err(FlowError::Cancelled(
        "timed out waiting for the browser".into(),
    ))
}

/// Opens `url` in the system browser (`OCW_OPEN_URL` names another command,
/// e.g. for tests and headless kiosks).
pub fn open_browser(url: &str) -> Result<(), String> {
    if let Ok(cmd) = std::env::var("OCW_OPEN_URL")
        && !cmd.trim().is_empty()
    {
        let mut words = cmd.split_whitespace();
        let prog = words.next().unwrap_or_default();
        return std::process::Command::new(prog)
            .args(words)
            .arg(url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|e| format!("cannot run OCW_OPEN_URL ({prog}): {e}"));
    }
    crate::strava::auth::open_browser(url)
}

#[cfg(test)]
mod tests {
    //! The desktop client against the real backend and the fake providers,
    //! all in-process (spec §11.4: no network beyond the loopback).

    use super::*;
    use ocw_accounts::fake::{DevBackend, TestBrowser};

    struct Env {
        _rt: tokio::runtime::Runtime,
        dev: DevBackend,
    }

    fn env() -> Env {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dev = rt.block_on(DevBackend::start("127.0.0.1:0", None)).unwrap();
        Env { _rt: rt, dev }
    }

    /// The "system browser": the fake providers approve at once.
    fn browser(url: &str) -> Result<(), String> {
        let url = url.to_owned();
        std::thread::spawn(move || {
            let b = TestBrowser::new();
            let _ = b.open(&url);
        });
        Ok(())
    }

    #[test]
    fn desktop_signs_in_links_refreshes_and_signs_out() {
        let e = env();
        let mut c = AuthClient::new(&e.dev.issuer, ocw_auth::CLIENT_DESKTOP);
        let no = AtomicBool::new(false);
        let s = c
            .sign_in(
                Some(Provider::Google),
                None,
                &browser,
                &no,
                Duration::from_secs(20),
            )
            .unwrap();
        assert_eq!(s.name, "Marianne Vos");
        assert_eq!(s.providers, ["google"]);
        // The game server would accept it.
        assert!(
            e.dev
                .service
                .verifier()
                .verify(&s.access_token, now_s())
                .is_ok()
        );
        // Link Strava to the same account.
        let linked = c
            .sign_in(
                Some(Provider::Strava),
                Some(&s),
                &browser,
                &no,
                Duration::from_secs(20),
            )
            .unwrap();
        assert_eq!(linked.account_id, s.account_id);
        assert_eq!(linked.providers, ["google", "strava"]);
        let u = c.userinfo(&linked).unwrap();
        assert_eq!(u.providers.len(), 2);
        // Refresh rotates.
        let r = c.refresh(&linked).unwrap();
        assert_ne!(r.refresh_token, linked.refresh_token);
        // Unlink, export, sign out (revoke): the old refresh token is dead.
        let u = c.unlink(&r, Provider::Google).unwrap();
        assert_eq!(u.providers.len(), 1);
        assert!(c.unlink(&r, Provider::Strava).is_err(), "never the last");
        assert!(c.export(&r).unwrap().contains("Marianne Vos"));
        c.revoke(&r).unwrap();
        assert!(matches!(c.refresh(&r), Err(FlowError::SignInAgain(_))));
    }

    #[test]
    fn kiosk_signs_in_with_a_code() {
        let e = env();
        let mut c = AuthClient::new(&e.dev.issuer, ocw_auth::CLIENT_DESKTOP);
        let d = c.device_start().unwrap();
        // The phone confirms with Strava.
        let complete = d.verification_uri_complete.clone().unwrap();
        let code = d.user_code.clone();
        std::thread::spawn(move || {
            let b = TestBrowser::new();
            let page = b.open(&complete).unwrap();
            let csrf = TestBrowser::field(&page.body, "csrf").unwrap();
            let confirm = format!("{}/confirm", complete.split('?').next().unwrap());
            b.submit(
                &confirm,
                &[("csrf", &csrf), ("user_code", &code), ("action", "strava")],
            )
            .unwrap();
        });
        let no = AtomicBool::new(false);
        // Real sleeps: the backend answers slow_down to faster polls.
        let s = c.device_wait(&d, &no, &std::thread::sleep).unwrap();
        assert_eq!(s.name, "Eddy Merckx");
        assert_eq!(s.client_id, ocw_auth::CLIENT_KIOSK);
        // Its refresh uses the kiosk's client.
        assert!(c.refresh(&s).is_ok());
        // A cancelled wait ends at once.
        let d = c.device_start().unwrap();
        let yes = AtomicBool::new(true);
        assert!(matches!(
            c.device_wait(&d, &yes, &|_| {}),
            Err(FlowError::Cancelled(_))
        ));
    }

    #[test]
    fn a_forged_redirect_is_ignored_and_a_cancel_ends_the_wait() {
        let e = env();
        let mut c = AuthClient::new(&e.dev.issuer, ocw_auth::CLIENT_DESKTOP);
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        // The "browser" only sends a redirect with a forged state, then the
        // user cancels in the app.
        let forge = move |url: &str| -> Result<(), String> {
            let redirect = client::parse_form(url.split_once('?').unwrap().1)
                .into_iter()
                .find(|(k, _)| k == "redirect_uri")
                .unwrap()
                .1;
            let flag = flag.clone();
            std::thread::spawn(move || {
                let _ = TestBrowser::new().open(&format!("{redirect}?code=evil&state=forged"));
                flag.store(true, Ordering::Relaxed);
            });
            Ok(())
        };
        let r = c.sign_in(
            Some(Provider::Google),
            None,
            &forge,
            &cancel,
            Duration::from_secs(20),
        );
        assert!(matches!(r, Err(FlowError::Cancelled(_))), "{r:?}");
    }
}

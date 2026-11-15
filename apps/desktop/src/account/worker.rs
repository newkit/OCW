//! The `ocw-account` thread: every network call of the sign-in, the
//! stored session and its refresh in the background. The Bevy side sends
//! [`Command`]s, reads [`Shared`] (the Account page, the token for the
//! hello) and prints the messages.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ocw_auth::Provider;
use ocw_auth::client::Session;
use ocw_client::account::{AccountInfo, AccountState};

use super::flow::{AuthClient, BROWSER_TIMEOUT, FlowError, now_s};
use super::store::TokenStore;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    SignIn(Option<Provider>),
    /// The device flow (kiosk).
    Code,
    Link(Provider),
    Unlink(Provider),
    SignOut,
    Export(PathBuf),
    Delete,
}

/// What the app reads.
#[derive(Debug, Default)]
pub struct Shared {
    pub info: AccountInfo,
    /// The access token for the next hello and its expiry (Unix s).
    pub access: Option<(String, i64)>,
}

pub struct Worker {
    tx: Sender<Command>,
    pub shared: Arc<Mutex<Shared>>,
    pub messages: Mutex<Receiver<String>>,
    /// Ends a waiting sign-in (browser or code).
    pub cancel: Arc<AtomicBool>,
}

pub struct WorkerConfig {
    pub issuer: String,
    pub client_id: String,
    pub store: TokenStore,
    pub open_url: fn(&str) -> Result<(), String>,
}

impl Worker {
    pub fn send(&self, cmd: Command) {
        self.cancel.store(false, Ordering::Relaxed);
        let _ = self.tx.send(cmd);
    }

    pub fn spawn(cfg: WorkerConfig) -> std::io::Result<Self> {
        let (tx, rx) = channel();
        let (msg_tx, msg_rx) = channel();
        let shared = Arc::new(Mutex::new(Shared {
            info: AccountInfo {
                issuer: Some(cfg.issuer.clone()),
                ..AccountInfo::default()
            },
            access: None,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let mut t = Thread {
            client: AuthClient::new(&cfg.issuer, &cfg.client_id),
            store: cfg.store,
            open_url: cfg.open_url,
            session: None,
            shared: shared.clone(),
            messages: msg_tx,
            cancel: cancel.clone(),
        };
        std::thread::Builder::new()
            .name("ocw-account".into())
            .spawn(move || t.run(&rx))?;
        Ok(Self {
            tx,
            shared,
            messages: Mutex::new(msg_rx),
            cancel,
        })
    }
}

struct Thread {
    client: AuthClient,
    store: TokenStore,
    open_url: fn(&str) -> Result<(), String>,
    session: Option<Session>,
    shared: Arc<Mutex<Shared>>,
    messages: Sender<String>,
    cancel: Arc<AtomicBool>,
}

impl Thread {
    fn say(&self, m: impl Into<String>) {
        let _ = self.messages.send(m.into());
    }

    fn publish(&self, state: AccountState, message: Option<String>) {
        let mut sh = self.shared.lock().unwrap_or_else(|e| e.into_inner());
        sh.info.state = state;
        sh.info.message = message;
        sh.access = self
            .session
            .as_ref()
            .map(|s| (s.access_token.clone(), s.access_expires_at));
    }

    fn signed_in_state(&self) -> AccountState {
        match &self.session {
            Some(s) => AccountState::SignedIn {
                name: s.name.clone(),
                providers: s
                    .providers
                    .iter()
                    .filter_map(|p| Provider::parse(p))
                    .collect(),
            },
            None => AccountState::SignedOut,
        }
    }

    fn keep(&mut self, s: Session) {
        match self.store.save(&s) {
            Ok(_) => {}
            Err(e) => self.say(format!("account: the sign-in is not kept ({e})")),
        }
        self.session = Some(s);
    }

    fn forget(&mut self, why: Option<String>) {
        self.session = None;
        self.store.delete();
        self.publish(AccountState::SignedOut, why);
    }

    fn run(&mut self, rx: &Receiver<Command>) {
        // A stored sign-in: refreshed before it is used.
        if let Some(s) = self.store.load()
            && s.issuer == self.client.issuer
        {
            self.session = Some(s);
            self.publish(self.signed_in_state(), None);
            self.refresh_if_due(true);
        }
        loop {
            match rx.recv_timeout(Duration::from_secs(5)) {
                Ok(cmd) => self.handle(cmd),
                Err(RecvTimeoutError::Timeout) => self.refresh_if_due(false),
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }

    fn refresh_if_due(&mut self, force: bool) {
        let Some(s) = self.session.clone() else {
            return;
        };
        if !force && !s.needs_refresh(now_s()) {
            return;
        }
        match self.client.refresh(&s) {
            Ok(new) => {
                self.keep(new);
                self.publish(self.signed_in_state(), None);
            }
            Err(FlowError::SignInAgain(e)) => {
                self.say(format!(
                    "account: signed out ({e}); sign in again in Settings -> Account"
                ));
                self.forget(Some("Your sign-in expired: sign in again.".into()));
            }
            // Offline: keep the session, try again later.
            Err(e) => {
                tracing::debug!(%e, "token refresh failed; retrying later");
            }
        }
    }

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::SignIn(p) => {
                self.publish(
                    AccountState::Waiting {
                        provider: p,
                        linking: false,
                    },
                    None,
                );
                let r = self
                    .client
                    .sign_in(p, None, &self.open_url, &self.cancel, BROWSER_TIMEOUT);
                self.finish(r, "signed in");
            }
            Command::Link(p) => {
                self.refresh_if_due(false);
                let Some(s) = self.session.clone() else {
                    self.say("account: sign in first");
                    return;
                };
                self.publish(
                    AccountState::Waiting {
                        provider: Some(p),
                        linking: true,
                    },
                    None,
                );
                let r = self.client.sign_in(
                    Some(p),
                    Some(&s),
                    &self.open_url,
                    &self.cancel,
                    BROWSER_TIMEOUT,
                );
                self.finish(r, "linked");
            }
            Command::Code => match self.client.device_start() {
                Ok(d) => {
                    self.say(format!(
                        "account: on your phone open {} and enter {}",
                        d.verification_uri, d.user_code
                    ));
                    self.publish(
                        AccountState::DeviceCode {
                            user_code: d.user_code.clone(),
                            uri: d.verification_uri.clone(),
                            expires_at: now_s() + d.expires_in,
                        },
                        None,
                    );
                    let r = self
                        .client
                        .device_wait(&d, &self.cancel, &std::thread::sleep);
                    self.finish(r, "signed in");
                }
                Err(e) => {
                    self.say(format!("account: {e}"));
                    self.publish(self.signed_in_state(), Some(e.to_string()));
                }
            },
            Command::Unlink(p) => {
                self.refresh_if_due(false);
                let Some(s) = self.session.clone() else {
                    return;
                };
                match self.client.unlink(&s, p) {
                    Ok(_) => {
                        // New tokens name the providers now linked.
                        self.refresh_if_due(true);
                        self.say(format!("account: {p} unlinked"));
                        self.publish(self.signed_in_state(), Some(format!("{p} unlinked.")));
                    }
                    Err(e) => {
                        self.say(format!("account: {e}"));
                        self.publish(self.signed_in_state(), Some(e.to_string()));
                    }
                }
            }
            Command::SignOut => {
                if let Some(s) = self.session.clone()
                    && let Err(e) = self.client.revoke(&s)
                {
                    tracing::debug!(%e, "revocation failed (signed out here anyway)");
                }
                self.say("account: signed out");
                self.forget(Some("Signed out.".into()));
            }
            Command::Export(path) => {
                let Some(s) = self.session.clone() else {
                    self.say("account: sign in first");
                    return;
                };
                self.refresh_if_due(false);
                let s = self.session.clone().unwrap_or(s);
                match self.client.export(&s).and_then(|json| {
                    std::fs::write(&path, json)
                        .map_err(|e| FlowError::Failed(format!("{}: {e}", path.display())))
                }) {
                    Ok(()) => self.say(format!("account: data exported to {}", path.display())),
                    Err(e) => self.say(format!("account: export failed: {e}")),
                }
            }
            Command::Delete => {
                let Some(s) = self.session.clone() else {
                    self.say("account: sign in first");
                    return;
                };
                match self.client.delete_account(&s) {
                    Ok(()) => {
                        self.say("account: the account and its data are deleted");
                        self.forget(Some("Account deleted.".into()));
                    }
                    Err(e) => {
                        self.say(format!("account: not deleted: {e}"));
                        self.publish(self.signed_in_state(), Some(e.to_string()));
                    }
                }
            }
        }
    }

    fn finish(&mut self, r: Result<Session, FlowError>, what: &str) {
        match r {
            Ok(s) => {
                let name = s.name.clone();
                self.keep(s);
                self.say(format!("account: {what} as {name}"));
                self.publish(self.signed_in_state(), None);
            }
            Err(e) => {
                self.say(format!("account: {e}"));
                self.publish(self.signed_in_state(), Some(e.to_string()));
            }
        }
    }
}

//! Sign-in on the desktop (spec §9.1): Settings → Account and the
//! `account` console command. The system browser signs in at the
//! accounts backend (Google or Strava) and comes back to a loopback
//! redirect; kiosks use a code confirmed on a phone. The session is kept
//! in the OS keyring (else a private file) and refreshed in the
//! background; the access token goes into the hello of a shared ride.
//!
//! The backend comes from `--account-server URL` or `OCW_ACCOUNT_SERVER`;
//! without one, sign-in is off and everyone rides anonymously (Phase 10
//! Q5) – until OCW's public server exists.
//!
//! Network calls run on the `ocw-account` thread ([`worker`]); [`flow`]
//! is the Bevy-free client, [`store`] the token storage.

pub mod flow;
pub mod store;
pub mod worker;

use std::path::PathBuf;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_auth::Provider;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_ui::menu::AccountMenu;

use worker::{Command as Cmd, Worker, WorkerConfig};

/// The environment variable naming the accounts backend.
pub const ENV_SERVER: &str = "OCW_ACCOUNT_SERVER";

#[derive(Default)]
pub struct AccountPlugin {
    /// The backend (overrides `--account-server` and the environment).
    pub issuer: Option<String>,
    /// Keep the session in this file instead of the keyring (tests).
    pub file: Option<PathBuf>,
    /// Opens the sign-in URL (default: the system browser; tests).
    pub open_url: Option<fn(&str) -> Result<(), String>>,
}

/// The sign-in; `worker` is `None` when no backend is configured.
#[derive(Resource)]
pub struct Account {
    worker: Option<Worker>,
}

impl Account {
    /// The access token for a hello, if signed in and still valid.
    pub fn access_token(&self) -> Option<String> {
        let w = self.worker.as_ref()?;
        let sh = w.shared.lock().unwrap_or_else(|e| e.into_inner());
        let (token, exp) = sh.access.clone()?;
        (flow::now_s() + 5 < exp).then_some(token)
    }
}

/// The access token for the hello of a shared ride (spec §9.1).
pub fn access_token(world: &World) -> Option<String> {
    world.get_resource::<Account>()?.access_token()
}

impl Plugin for AccountPlugin {
    fn build(&self, app: &mut App) {
        let issuer = self
            .issuer
            .clone()
            .or_else(|| {
                app.world()
                    .get_resource::<crate::Args>()
                    .and_then(|a| a.account_server.clone())
            })
            .or_else(|| std::env::var(ENV_SERVER).ok())
            .map(|s| s.trim().trim_end_matches('/').to_owned())
            .filter(|s| !s.is_empty());
        let worker = issuer.and_then(|issuer| {
            let file = self.file.clone();
            let store = match (file, crate::strava::default_config_dir()) {
                (Some(f), _) => store::TokenStore::File(f),
                (None, Some(dir)) => store::TokenStore::keyring(&issuer, dir.join("account.json")),
                (None, None) => store::TokenStore::None,
            };
            info!(%issuer, "sign-in: accounts server");
            Worker::spawn(WorkerConfig {
                issuer,
                client_id: ocw_auth::CLIENT_DESKTOP.into(),
                store,
                open_url: self.open_url.unwrap_or(flow::open_browser),
            })
            .map_err(|e| warn!("sign-in: cannot start its thread: {e}"))
            .ok()
        });
        app.insert_resource(Account { worker })
            .add_systems(Update, (print_messages, refresh_menu));
        register_commands(app);
    }
}

fn print_messages(account: Res<Account>, mut console: ResMut<ConsoleState>) {
    if let Some(w) = &account.worker {
        for m in w
            .messages
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_iter()
        {
            info!("{m}");
            console.print(m);
        }
    }
}

/// The Account page follows the worker's state.
fn refresh_menu(account: Res<Account>, menu: Option<ResMut<AccountMenu>>) {
    let Some(mut menu) = menu else {
        return;
    };
    let info = match &account.worker {
        Some(w) => w
            .shared
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .info
            .clone(),
        None => Default::default(),
    };
    if menu.0 != info {
        menu.0 = info;
    }
}

fn worker(world: &World) -> Result<&Worker, String> {
    world.resource::<Account>().worker.as_ref().ok_or_else(|| {
        format!("sign-in is not set up: start with --account-server URL or set {ENV_SERVER}")
    })
}

fn provider_arg() -> Arg {
    Arg::new("provider").value_parser(["google", "strava"])
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("account")
            .about("Sign in with Google or Strava (spec 9.1): status, signin, link, unlink, signout, cancel, export, delete")
            .subcommand_required(false)
            .subcommand(Command::new("status").about("Show who is signed in"))
            .subcommand(
                Command::new("signin")
                    .about("Sign in in the browser (or with a code on a kiosk)")
                    .arg(provider_arg())
                    .arg(
                        Arg::new("code")
                            .long("code")
                            .action(ArgAction::SetTrue)
                            .help("Sign in with a code confirmed on a phone (kiosk, headset)"),
                    ),
            )
            .subcommand(Command::new("link").about("Link the other provider").arg(provider_arg().required(true)))
            .subcommand(
                Command::new("unlink")
                    .about("Unlink a provider (never the last one)")
                    .arg(provider_arg().required(true)),
            )
            .subcommand(Command::new("signout").about("Sign out (ends the sign-in at the server)"))
            .subcommand(Command::new("cancel").about("Stop waiting for the browser or the code"))
            .subcommand(
                Command::new("export")
                    .about("Save your account's data (spec 12) as JSON")
                    .arg(Arg::new("path").default_value("ocw-account.json")),
            )
            .subcommand(
                Command::new("delete")
                    .about("Delete your account and its data (needs a fresh sign-in)")
                    .arg(Arg::new("yes").long("yes").action(ArgAction::SetTrue).required(true)),
            ),
        |world, m| {
            let provider = |m: &clap::ArgMatches| {
                m.get_one::<String>("provider").and_then(|p| Provider::parse(p))
            };
            match m.subcommand() {
                None | Some(("status", _)) => {
                    Ok(world
                        .get_resource::<AccountMenu>()
                        .map(|a| a.status())
                        .unwrap_or_else(|| ocw_client::account::AccountInfo::default().status()))
                }
                Some(("signin", sub)) => {
                    let w = worker(world)?;
                    if sub.get_flag("code") {
                        w.send(Cmd::Code);
                        Ok("asking the account server for a code …".into())
                    } else {
                        w.send(Cmd::SignIn(provider(sub)));
                        Ok("opening the sign-in in your browser …".into())
                    }
                }
                Some(("link", sub)) => {
                    let p = provider(sub).ok_or("which provider?")?;
                    worker(world)?.send(Cmd::Link(p));
                    Ok(format!("opening the {p} sign-in in your browser to link it …"))
                }
                Some(("unlink", sub)) => {
                    let p = provider(sub).ok_or("which provider?")?;
                    worker(world)?.send(Cmd::Unlink(p));
                    Ok(format!("unlinking {p} …"))
                }
                Some(("signout", _)) => {
                    worker(world)?.send(Cmd::SignOut);
                    Ok("signing out …".into())
                }
                Some(("cancel", _)) => {
                    let w = worker(world)?;
                    w.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    Ok("cancelling the sign-in".into())
                }
                Some(("export", sub)) => {
                    let path = PathBuf::from(sub.get_one::<String>("path").cloned().unwrap_or_default());
                    worker(world)?.send(Cmd::Export(path));
                    Ok("exporting …".into())
                }
                Some(("delete", _)) => {
                    worker(world)?.send(Cmd::Delete);
                    Ok("deleting the account …".into())
                }
                Some((other, _)) => Err(format!("unknown subcommand {other}")),
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use ocw_accounts::fake::{DevBackend, TestBrowser};
    use ocw_client::account::AccountState;
    use ocw_render::console::ConsoleCorePlugin;

    use super::*;

    /// The "system browser" of the tests: follows the redirects (the
    /// fake providers approve at once).
    fn test_browser(url: &str) -> Result<(), String> {
        let url = url.to_owned();
        std::thread::spawn(move || {
            let _ = TestBrowser::new().open(&url);
        });
        Ok(())
    }

    fn app(issuer: Option<String>, file: PathBuf) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<AccountMenu>()
            .add_plugins(AccountPlugin {
                issuer,
                file: Some(file),
                open_url: Some(test_browser),
            });
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world()
            .resource::<ConsoleState>()
            .log()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    #[test]
    fn without_a_server_sign_in_is_off() {
        let dir = std::env::temp_dir().join(format!("ocw-account-off-{}", std::process::id()));
        let mut a = app(None, dir.join("a.json"));
        // (The environment may name a server on a developer's machine.)
        if std::env::var(ENV_SERVER).is_ok() {
            return;
        }
        assert!(run(&mut a, "account signin google").contains("not set up"));
        assert!(access_token(a.world()).is_none());
        assert!(!dir.exists());
    }

    /// End to end: the desktop's Account page and console against the
    /// backend and the fake Google, in-process (spec §9.1 tests).
    #[test]
    fn signs_in_from_the_console_and_shows_it_on_the_page() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let dev = rt.block_on(DevBackend::start("127.0.0.1:0", None)).unwrap();
        let dir = std::env::temp_dir().join(format!("ocw-account-e2e-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut a = app(Some(dev.issuer.clone()), dir.join("account.json"));
        let wait = |a: &mut App, done: &dyn Fn(&AccountState) -> bool| {
            for _ in 0..400 {
                a.update();
                if done(&a.world().resource::<AccountMenu>().state) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            panic!("timed out: {:?}", a.world().resource::<AccountMenu>().0);
        };
        run(&mut a, "account signin google");
        wait(&mut a, &|s| matches!(s, AccountState::SignedIn { .. }));
        assert_eq!(
            a.world().resource::<AccountMenu>().status(),
            "Signed in as Marianne Vos (Google)"
        );
        let token = access_token(a.world()).expect("a token for the hello");
        assert_eq!(
            dev.service
                .verifier()
                .verify(&token, flow::now_s())
                .unwrap()
                .name,
            "Marianne Vos"
        );
        // Kept in the file (the tests' store), private.
        assert!(dir.join("account.json").exists());
        run(&mut a, "account link strava");
        wait(
            &mut a,
            &|s| matches!(s, AccountState::SignedIn { providers, .. } if providers.len() == 2),
        );
        run(&mut a, "account signout");
        wait(&mut a, &|s| *s == AccountState::SignedOut);
        assert!(access_token(a.world()).is_none());
        assert!(!dir.join("account.json").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

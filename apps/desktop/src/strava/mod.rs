//! Strava upload and sync of saved rides (spec §2.8).
//!
//! Interim desktop path until the backend exists (spec §2.8, §14 Phase 5):
//! the user registers their own Strava API application and configures its
//! client id and secret (`OCW_STRAVA_CLIENT_ID` / `OCW_STRAVA_CLIENT_SECRET`
//! or `<config dir>/ocw/strava.toml`); nothing is built in. Without them
//! the feature is off.
//!
//! Everything that touches the network runs on the `ocw-strava` thread
//! ([`worker`]); the Bevy side only sends commands and prints messages.

pub mod api;
pub mod auth;
pub mod sync;
pub mod worker;

use std::path::PathBuf;
use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use auth::{ENV_CLIENT_ID, ENV_CLIENT_SECRET, StravaConfig, TokenStore};
use sync::SyncPaths;
use worker::{Command as Cmd, Worker, WorkerConfig};

/// `<config dir>/ocw`; `OCW_CONFIG_DIR` overrides it.
pub fn default_config_dir() -> Option<PathBuf> {
    match std::env::var_os("OCW_CONFIG_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => Some(dirs::config_dir()?.join("ocw")),
    }
}

#[derive(Default)]
pub struct StravaPlugin {
    /// Overrides [`default_config_dir`].
    pub config_dir: Option<PathBuf>,
    /// Ignore the environment variables (tests).
    pub ignore_env: bool,
}

/// The Strava link; `worker` is `None` when not configured.
#[derive(Resource)]
pub struct Strava {
    worker: Option<Worker>,
    /// Why it is off.
    off_reason: String,
}

impl Strava {
    /// Tells the sync about a newly saved ride.
    pub fn ride_saved(&self) {
        if let Some(w) = &self.worker {
            w.send(Cmd::RideSaved);
        }
    }

    pub fn status_text(&self) -> String {
        let Some(w) = &self.worker else {
            return self.off_reason.clone();
        };
        let s = w.status();
        let mut out = if s.connected {
            format!(
                "Strava: linked{} (automatic upload {})",
                s.athlete.map(|a| format!(" as {a}")).unwrap_or_default(),
                if s.auto_upload { "on" } else { "off" }
            )
        } else {
            "Strava: configured, not linked (run strava.connect)".to_string()
        };
        let c = &s.counts;
        out.push_str(&format!(
            "\nrides: {} uploaded, {} pending, {} failed, {} rejected",
            c.uploaded, c.pending, c.failed, c.rejected
        ));
        if let Some(at) = c.next_retry {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs() as i64);
            out.push_str(&format!("; next retry in {} s", (at - now).max(0)));
        }
        if let Some(b) = s.busy {
            out.push_str(&format!("\nnow: {b}"));
        }
        if let Some(l) = s.last {
            out.push_str(&format!("\nlast: {l}"));
        }
        out
    }
}

impl Plugin for StravaPlugin {
    fn build(&self, app: &mut App) {
        let rides_dir = app
            .world()
            .get_resource::<crate::recording::Recording>()
            .and_then(|r| r.dir.clone())
            .or_else(crate::recording::default_rides_dir);
        let config_dir = self.config_dir.clone().or_else(default_config_dir);
        let strava = match (config_dir, rides_dir) {
            (Some(config_dir), Some(rides_dir)) => start(&config_dir, rides_dir, self.ignore_env),
            _ => Strava {
                worker: None,
                off_reason: "Strava: off (no config or data directory)".into(),
            },
        };
        if strava.worker.is_none() {
            info!("{}", strava.off_reason.lines().next().unwrap_or_default());
        }
        app.insert_resource(strava)
            .add_systems(Update, print_messages);
        register_commands(app);
    }
}

fn start(config_dir: &std::path::Path, rides_dir: PathBuf, ignore_env: bool) -> Strava {
    let config_file = config_dir.join("strava.toml");
    let config = match StravaConfig::load(&config_file) {
        Ok(c) => c,
        Err(e) => {
            return Strava {
                worker: None,
                off_reason: format!("Strava: off, {e}"),
            };
        }
    };
    let env = |k: &str| {
        if ignore_env {
            None
        } else {
            std::env::var(k).ok()
        }
    };
    let Some(creds) = config.credentials(env) else {
        return Strava {
            worker: None,
            off_reason: format!(
                "Strava: not configured. Register an API application at https://www.strava.com/settings/api \
                 (authorization callback domain: localhost), then set {ENV_CLIENT_ID} and {ENV_CLIENT_SECRET} \
                 or put client_id and client_secret into {}",
                config_file.display()
            ),
        };
    };
    let cfg = WorkerConfig {
        creds,
        paths: SyncPaths {
            state_file: rides_dir.join("strava-sync.toml"),
            rides_dir,
            token: TokenStore {
                path: config_dir.join("strava-token.toml"),
            },
        },
        auto_upload: config.auto_upload.unwrap_or(true),
        http: Arc::new(api::UreqHttp::default()),
        open_url: auth::open_browser,
        sleep: std::thread::sleep,
    };
    match Worker::spawn(cfg) {
        Ok(w) => Strava {
            worker: Some(w),
            off_reason: String::new(),
        },
        Err(e) => Strava {
            worker: None,
            off_reason: format!("Strava: off, cannot start its thread: {e}"),
        },
    }
}

/// Shows the background thread's messages in the console.
fn print_messages(strava: Res<Strava>, mut console: ResMut<ConsoleState>) {
    if let Some(w) = &strava.worker {
        for m in w.messages.lock().expect("not poisoned").try_iter() {
            console.print(m);
        }
    }
}

fn worker(world: &World) -> Result<&Worker, String> {
    let s = world.resource::<Strava>();
    s.worker.as_ref().ok_or_else(|| s.off_reason.clone())
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.connect")
            .about("Link your Strava account (opens the Strava login in the browser)"),
        |world, _| {
            worker(world)?.send(Cmd::Connect);
            Ok("opening the Strava login in the browser…".into())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.sync")
            .about("Upload every saved ride that is not on Strava yet (in the background)")
            .arg(
                Arg::new("retry")
                    .long("retry")
                    .action(ArgAction::SetTrue)
                    .help("Also retry failed and rejected rides now"),
            ),
        |world, m| {
            let retry = m.get_flag("retry");
            worker(world)?.send(Cmd::Sync { retry });
            Ok("Strava sync started".into())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.status").about("Show the Strava link and upload state"),
        |world, _| Ok(world.resource::<Strava>().status_text()),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("strava.disconnect").about("Unlink Strava (deletes the stored tokens)"),
        |world, _| {
            worker(world)?.send(Cmd::Disconnect);
            Ok("disconnecting Strava…".into())
        },
    ));
}

#[cfg(test)]
mod tests {
    use ocw_render::console::ConsoleCorePlugin;

    use super::*;

    #[test]
    fn without_configuration_strava_stays_off() {
        let dir = std::env::temp_dir().join(format!("ocw-strava-off-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .add_plugins(StravaPlugin {
                config_dir: Some(dir.clone()),
                ignore_env: true,
            });
        for cmd in ["strava.status", "strava.connect", "strava.sync"] {
            app.world_mut().resource_mut::<ConsoleState>().queue(cmd);
            app.update();
            let log = app.world().resource::<ConsoleState>().log().to_vec();
            assert!(
                log.last().unwrap().contains("not configured"),
                "{cmd}: {log:?}"
            );
        }
        assert!(app.world().resource::<Strava>().worker.is_none());
        // Nothing was written.
        assert!(!dir.exists());
    }
}

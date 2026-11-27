//! Ride → Join → *Public server* on the desktop (spec 11.5 *The public
//! server as the default*, 3.2.1 *Join*): the server every app offers
//! first, for now the one on Fly.io (`ocw_client::public`).
//!
//! `join public` (the page's first row, the start choice, `--join public`)
//! fetches the server's join info in the background (10 s, at most 64 kB),
//! checks its world as the kiosk does – an installed package of the
//! server's id whose SHA-256 is the server's, else its id and version –
//! and then joins its desktop link like any `join LINK`. A missing world
//! is said on the page before any connection is opened; when the join info
//! names a download (`world_manifest`) the page offers *Download the world
//! and join* (`join.public --download`), else it says what to install.
//! `net.public_server` (default / off / another join info URL) changes or
//! clears the server without a rebuild; off removes the rows.

use std::path::Path;
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use ocw_client::menu::join::{PublicJoin, PublicState};
use ocw_client::public::{self, Installed, JoinInfo, WorldMatch};
use ocw_render::console::ConsoleState;
use ocw_ui::join::JoinUi;

use super::RideCatalog;
use crate::Args;

/// How the join info is fetched: HTTPS with ureq; tests put in their own
/// (no test touches the network).
#[derive(Resource, Clone)]
pub struct PublicFetcher(pub Arc<FetchFn>);

/// GET a URL as text.
pub type FetchFn = dyn Fn(&str) -> Result<String, String> + Send + Sync;

impl Default for PublicFetcher {
    fn default() -> Self {
        Self(Arc::new(fetch_https))
    }
}

/// GET `url` (https only), at most [`public::MAX_JOIN_INFO_BYTES`].
fn fetch_https(url: &str) -> Result<String, String> {
    use std::io::Read;
    if !url.starts_with("https://") {
        return Err(format!("{url}: not an https:// URL"));
    }
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(
            public::FETCH_TIMEOUT_S,
        )))
        .user_agent(concat!("OpenCyclingWorld/", env!("CARGO_PKG_VERSION")))
        .build()
        .into();
    let resp = agent
        .get(url)
        .header("Cache-Control", "no-store")
        .call()
        .map_err(|e| format!("join info {url} not fetched: {e}"))?;
    let mut text = String::new();
    resp.into_body()
        .into_reader()
        .take(public::MAX_JOIN_INFO_BYTES)
        .read_to_string(&mut text)
        .map_err(|e| format!("join info {url}: {e}"))?;
    Ok(text)
}

/// What the background step found.
enum Outcome {
    /// Join this link (the world is here, or none is needed).
    Join(String),
    Missing {
        why: String,
        download: Option<String>,
        hint: String,
    },
    Failed(String),
}

/// The background step in progress.
#[derive(Resource)]
struct Task(Mutex<Receiver<Outcome>>);

/// The public server's join info URL (`net.public_server`; without the
/// settings – tests – the built-in default).
pub fn url(world: &World) -> Option<String> {
    let v = world
        .get_resource::<ocw_ui::settings::UserSettings>()
        .map_or_else(
            || "default".to_owned(),
            |s| s.value(public::SETTING).to_string(),
        );
    public::resolve_or_default(&v)
}

pub fn build(app: &mut App) {
    app.init_resource::<PublicFetcher>()
        .add_systems(Update, (follow_setting, pump));
}

/// Keeps the Join page's public row in step with the setting.
fn follow_setting(world: &mut World) {
    let url = url(world);
    let Some(mut ui) = world.get_resource_mut::<JoinUi>() else {
        return;
    };
    let now = ui.info.public.as_ref().map(|p| p.url.clone());
    if now != url {
        ui.info.public = url.as_deref().map(PublicJoin::new);
    }
}

/// The installed rideable packages (the World list's).
fn installed(world: &mut World) -> Vec<Installed> {
    super::refresh_catalog(world);
    world
        .resource::<RideCatalog>()
        .0
        .worlds
        .iter()
        .flat_map(ocw_worldstore::discover::WorldEntry::members)
        .filter(|e| e.is_rideable())
        .map(|e| Installed {
            spec: e.spec.clone(),
            id: e.id.clone(),
            version: e.version,
            path: e.path.clone(),
        })
        .collect()
}

/// The SHA-256 of a file (hex), read in pieces.
fn sha256_file(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// The join info → what to do (the background step's core, without the
/// network: `fetch` gets the text).
fn resolve(
    url: &str,
    fetch: &dyn Fn(&str) -> Result<String, String>,
    installed: &[Installed],
) -> Outcome {
    let info = match fetch(url)
        .and_then(|t| JoinInfo::parse(&t).map_err(|e| format!("join info {url}: {e}")))
    {
        Ok(i) => i,
        Err(e) => return Outcome::Failed(e),
    };
    match public::match_world(&info, installed, &mut sha256_file) {
        WorldMatch::NotNeeded | WorldMatch::Installed(_) => Outcome::Join(info.link),
        WorldMatch::Missing { why, download } => Outcome::Missing {
            why,
            download,
            hint: public::install_hint(&info.world_spec().unwrap_or_default()),
        },
    }
}

/// `join public` / `join.public [--download]`.
pub fn start(world: &mut World, download: bool) -> Result<String, String> {
    let url = url(world).ok_or(
        "no public server: net.public_server is off (set net.public_server default to offer it again)",
    )?;
    if world.contains_resource::<Task>() {
        return Err("already finding the public server".into());
    }
    if world.contains_resource::<super::host::Hosting>() {
        return Err("you host a shared ride: stop hosting before joining another".into());
    }
    if world.resource::<JoinUi>().info.flow.busy() {
        return Err("a join is already in progress (cancel it first)".into());
    }
    let manifest = match world
        .resource::<JoinUi>()
        .info
        .public
        .as_ref()
        .map(|p| &p.state)
    {
        Some(PublicState::Missing {
            download: Some(m), ..
        }) => Some(m.clone()),
        _ => None,
    };
    if download && manifest.is_none() {
        return Err("the public server names no download for its world".into());
    }
    let installed = installed(world);
    let fetch = world.resource::<PublicFetcher>().0.clone();
    let opts = if download {
        Some(crate::worlds::load_options(world.resource::<Args>())?)
    } else {
        None
    };
    let allow_unsigned = world.resource::<Args>().allow_unsigned;
    let now = world
        .get_resource::<Time>()
        .map_or(0, |t| t.elapsed_secs_f64() as u64);
    let (tx, rx) = channel();
    let task_url = url.clone();
    std::thread::Builder::new()
        .name("ocw-public-join".into())
        .spawn(move || {
            if let (Some(m), Some(opts)) = (&manifest, &opts) {
                let policy = if allow_unsigned {
                    ocw_world::manifest::UnsignedPolicy::Warn
                } else {
                    ocw_world::manifest::UnsignedPolicy::Refuse
                };
                let transport = ocw_worldstore::fetch::transport_for(m);
                if let Err(e) = ocw_worldstore::fetch(
                    m,
                    &opts.cache,
                    &opts.trust,
                    policy,
                    opts.now_unix_s,
                    &*transport,
                    &mut |_, _, _| {},
                ) {
                    let _ = tx.send(Outcome::Failed(format!(
                        "the world was not downloaded: {m}: {e}"
                    )));
                    return;
                }
                // The package the server named is in the cache now (the
                // join checks its road graph against the server's).
                let _ = tx.send(match fetch(&task_url).and_then(|t| JoinInfo::parse(&t)) {
                    Ok(i) => Outcome::Join(i.link),
                    Err(e) => Outcome::Failed(format!("join info {task_url}: {e}")),
                });
                return;
            }
            let _ = tx.send(resolve(&task_url, &*fetch, &installed));
        })
        .map_err(|e| format!("cannot start: {e}"))?;
    world.insert_resource(Task(Mutex::new(rx)));
    let host = public::host_of(&url).to_owned();
    if let Some(p) = world.resource_mut::<JoinUi>().info.public.as_mut() {
        p.state = if download {
            PublicState::Downloading {
                world: "the server's world".into(),
                since_s: now,
            }
        } else {
            PublicState::Fetching { since_s: now }
        };
    }
    info!(server = %host, "finding the public server");
    Ok(format!("finding the public server ({host}) …"))
}

/// Each frame: the background step's answer.
fn pump(world: &mut World) {
    let Some(task) = world.get_resource::<Task>() else {
        return;
    };
    let got = task.0.lock().expect("not poisoned").try_recv();
    let outcome = match got {
        Ok(o) => o,
        Err(std::sync::mpsc::TryRecvError::Empty) => return,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
            Outcome::Failed("finding the public server stopped".into())
        }
    };
    world.remove_resource::<Task>();
    let state = match outcome {
        Outcome::Join(link) => {
            if let Some(p) = world.resource_mut::<JoinUi>().info.public.as_mut() {
                p.state = PublicState::Idle;
            }
            info!("the public server's world is here: joining");
            let msg = match ocw_ui::join::join_line(world, &link, "") {
                Ok(m) => m,
                Err(e) => format!("not joined: {e}"),
            };
            world.resource_mut::<ConsoleState>().print(msg);
            return;
        }
        Outcome::Missing {
            why,
            download,
            hint,
        } => {
            warn!("public server: {why}");
            PublicState::Missing {
                why,
                download,
                hint,
            }
        }
        Outcome::Failed(e) => {
            warn!("public server: {e}");
            PublicState::Failed(e)
        }
    };
    let line = PublicJoin {
        url: String::new(),
        state: state.clone(),
    }
    .status_line(0.0)
    .unwrap_or_default();
    if let Some(p) = world.resource_mut::<JoinUi>().info.public.as_mut() {
        p.state = state;
    }
    world.resource_mut::<ConsoleState>().print(line);
}

#[cfg(test)]
mod tests {
    use super::*;

    const FP: &str = "0ae1fb51a5eacb8fb1beb577b107b5fa5f7b177f89ccd4cae299c8e23a86ff77";

    #[test]
    fn the_join_info_becomes_a_join_or_a_missing_world() {
        let info = format!(
            r#"{{"join": "ocw://203.0.113.7?cert={FP}&world=liechtenstein@10",
                "world": "liechtenstein", "world_version": 10}}"#
        );
        let fetch = |u: &str| {
            assert_eq!(u, public::PUBLIC_JOIN_INFO);
            Ok(info.clone())
        };
        let here = [Installed {
            spec: "liechtenstein@10".into(),
            id: "liechtenstein".into(),
            version: 10,
            path: None,
        }];
        match resolve(public::PUBLIC_JOIN_INFO, &fetch, &here) {
            Outcome::Join(l) => assert!(l.starts_with("ocw://203.0.113.7?cert=")),
            _ => panic!("expected a join"),
        }
        match resolve(public::PUBLIC_JOIN_INFO, &fetch, &[]) {
            Outcome::Missing {
                why,
                download,
                hint,
            } => {
                assert!(why.contains("liechtenstein@10") && download.is_none());
                assert!(hint.contains("install the package liechtenstein@10"));
            }
            _ => panic!("expected the world missing"),
        }
        let down = |_: &str| Err("join info not fetched: timed out".to_string());
        assert!(matches!(
            resolve(public::PUBLIC_JOIN_INFO, &down, &here),
            Outcome::Failed(e) if e.contains("timed out")
        ));
    }

    #[test]
    fn the_file_hash_is_the_sha256() {
        let dir = std::env::temp_dir().join(format!("ocw-public-sha-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.ocwpkg");
        std::fs::write(&p, b"abc").unwrap();
        assert_eq!(
            sha256_file(&p).as_deref(),
            Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
        );
        assert_eq!(sha256_file(&dir.join("missing")), None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

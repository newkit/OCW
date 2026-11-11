//! World packages in the desktop app (spec §4.7, §12): `--world PATH | ID`,
//! the local cache, `--fetch-world`, the trust flags and the console
//! commands `worlds.list` and `worlds.verify`. The logic lives in
//! `ocw-worldstore`; this is the glue.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_world::PackageIdentity;
use ocw_world::manifest::UnsignedPolicy;
use ocw_worldstore::trust::TrustConfig;
use ocw_worldstore::{LoadOptions, LoadedWorld, WorldCache, load_world};

use crate::Args;

/// The loaded world package: what it is and whether it is trusted.
#[derive(Resource, Debug, Clone)]
pub struct WorldInfo {
    /// What `--world` said (path or id).
    pub spec: String,
    pub identity: PackageIdentity,
    /// One line: identity, trust, file.
    pub description: String,
    pub trusted: bool,
    /// Credits (spec §13).
    pub attributions: Vec<String>,
}

impl WorldInfo {
    pub fn of(spec: &str, w: &LoadedWorld) -> Self {
        Self {
            spec: spec.into(),
            identity: w.identity.clone(),
            description: w.describe(),
            trusted: w.trust.is_trusted(),
            attributions: w.attributions(),
        }
    }
}

pub fn cache(args: &Args) -> WorldCache {
    WorldCache::new(
        args.world_cache
            .clone()
            .unwrap_or_else(WorldCache::default_root),
    )
}

pub fn load_options(args: &Args) -> Result<LoadOptions, String> {
    let trust = TrustConfig {
        dev_roots: cfg!(debug_assertions) || args.trust_dev_root,
        root_files: args.trust_root.clone(),
        ..TrustConfig::for_build()
    }
    .trust_store()?;
    Ok(LoadOptions {
        allow_unsigned: args.allow_unsigned,
        require_signed: args.require_signed,
        ..LoadOptions::new(trust, cache(args))
    })
}

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("{msg}");
    std::process::exit(1);
}

/// `--list-worlds` and `--fetch-world`; exits when there is nothing to
/// ride afterwards.
pub fn handle_cli(args: &Args) {
    if args.list_worlds {
        println!("{}", list(args));
        std::process::exit(0);
    }
    let Some(url) = &args.fetch_world else {
        return;
    };
    let opts = load_options(args).unwrap_or_else(|e| fail(e));
    let policy = if args.allow_unsigned {
        UnsignedPolicy::Warn
    } else {
        UnsignedPolicy::Refuse
    };
    let transport = ocw_worldstore::fetch::transport_for(url);
    let mut last = 0u64;
    let fetched = ocw_worldstore::fetch(
        url,
        &opts.cache,
        &opts.trust,
        policy,
        opts.now_unix_s,
        &*transport,
        &mut |file, done, total| {
            // Every 10 %.
            let step = (total / 10).max(1);
            if done / step != last / step || done == total {
                println!(
                    "{file}: {:.1} / {:.1} MB",
                    done as f64 / 1e6,
                    total as f64 / 1e6
                );
            }
            last = done;
        },
    )
    .unwrap_or_else(|e| fail(format!("{url}: {e}")));
    println!(
        "installed {}@{} in {} ({:.1} MB downloaded) [{}]",
        fetched.entry.id,
        fetched.entry.version,
        fetched.entry.dir.display(),
        fetched.downloaded as f64 / 1e6,
        fetched.trust
    );
    if args.world.is_none() {
        println!(
            "ride it with: ocw-desktop --world {} --free-ride",
            fetched.entry.id
        );
        std::process::exit(0);
    }
}

/// Loads `--world`, verified; exits with the reason if it is refused.
pub fn load(args: &Args, spec: &str) -> LoadedWorld {
    let opts = load_options(args).unwrap_or_else(|e| fail(e));
    let w = load_world(spec, &opts).unwrap_or_else(|e| {
        fail(format!(
            "{e}\n(world packages: spec §4.7; local builds load with a warning, cached ones must be signed unless --allow-unsigned)"
        ))
    });
    if w.trust.is_trusted() {
        info!("world package {}", w.describe());
    } else {
        warn!("world package {}", w.describe());
        eprintln!("warning: world package {}", w.describe());
    }
    w
}

/// The cache contents, one line per package version.
fn list(args: &Args) -> String {
    let opts = match load_options(args) {
        Ok(o) => o,
        Err(e) => return e,
    };
    let entries = opts.cache.list();
    if entries.is_empty() {
        return format!(
            "no world packages in {} (install one with --fetch-world URL)",
            opts.cache.root().display()
        );
    }
    let mut s = format!("world packages in {}:", opts.cache.root().display());
    for e in entries {
        let line = match e.manifest() {
            Ok(m) => {
                let size: u64 = m.manifest.files.iter().map(|f| f.size).sum();
                let trust = match m.verify_signature(&opts.trust, opts.now_unix_s) {
                    Ok(_) => "signed, trusted".to_string(),
                    Err(err) => format!("NOT TRUSTED: {err}"),
                };
                format!(
                    "{}@{}  \"{}\"  {:.1} MB  {trust}",
                    e.id,
                    e.version,
                    m.manifest.name,
                    size as f64 / 1e6
                )
            }
            Err(err) => format!("{}@{}  broken: {err}", e.id, e.version),
        };
        s.push_str("\n  ");
        s.push_str(&line);
    }
    s
}

pub struct WorldsPlugin;

impl Plugin for WorldsPlugin {
    fn build(&self, app: &mut App) {
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("worlds.info")
                .about("The loaded world package: id, version, content hash, trust, credits"),
            |world, _| {
                let w = world
                    .get_resource::<WorldInfo>()
                    .ok_or("no world package loaded")?;
                Ok(format!(
                    "{}\nidentity: {} (sha256 {})\ntrusted: {}\n{}",
                    w.description,
                    w.identity,
                    w.identity.content_hash,
                    if w.trusted { "yes" } else { "NO" },
                    w.attributions.join("\n")
                ))
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("worlds.list").about("List the world packages in the local cache"),
            |world, _| Ok(list(world.resource::<Args>())),
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("worlds.verify")
                .about("Verify a world package (hashes and signature): the loaded one, or a path / cache id")
                .arg(Arg::new("world").required(false)),
            |world, m| {
                let spec = match m.get_one::<String>("world") {
                    Some(s) => s.clone(),
                    None => world
                        .get_resource::<WorldInfo>()
                        .map(|w| w.spec.clone())
                        .ok_or("no world package loaded; give a path or id")?,
                };
                let opts = load_options(world.resource::<Args>())?;
                let w = load_world(&spec, &opts).map_err(|e| format!("REFUSED: {e}"))?;
                Ok(format!(
                    "{}\n{}",
                    w.describe(),
                    w.attributions().join("\n")
                ))
            },
        ));
    }
}

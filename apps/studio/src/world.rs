//! Opening a world package in the studio (spec §14 Phase 16, 16A, 16C):
//! loaded and verified exactly as the game loads it (`ocw_worldstore`:
//! signature and trust, the installed neighbours), handed to the renderer
//! as the game's [`PackageWorld`] (shared `ocw_ui::worlds`), plus what the
//! World workspace shows and the 2D map draws.
//!
//! **A package signed by an unknown user key** (spec 4.7 *User keys*;
//! 14 Phase 16, 16H Q14 (a)): [`open`] asks first with the game's trust
//! dialog (`ocw_ui::trust`, the shared menu page `ConfirmTrust`: *Trust
//! this key* / *Load once* / *Cancel* focused, the fingerprint large, the
//! claimed name "(unverified name)", the world and its data sources) and
//! opens once the user agreed. The trusted keys are the game's list
//! (`ocw_worldstore::userkeys`: `$OCW_TRUSTED_KEYS`, else `<data
//! dir>/ocw/trusted-keys.json`, the same path in both apps), so a key
//! trusted here counts in the game and the other way round; *Load once*
//! lasts until the studio quits. A damaged package or a bad signature is
//! refused, never asked about.

use std::path::PathBuf;

use bevy::prelude::*;
use ocw_render::RideView;
use ocw_render::package::PackageWorld;
use ocw_worldstore::details::WorldDetails;
use ocw_worldstore::trust::TrustConfig;
use ocw_worldstore::userkeys::TrustRequest;
use ocw_worldstore::{LoadError, LoadOptions, WorldCache};

use crate::map::MapData;

/// How packages are loaded and trusted (the game's options).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct LoadSettings {
    pub allow_unsigned: bool,
    pub trust_dev_root: bool,
    pub trust_root: Vec<PathBuf>,
    pub no_neighbours: bool,
    /// The trusted user keys' file; `None`: the game's
    /// ([`ocw_worldstore::userkeys::default_path`]).
    pub keys_file: Option<PathBuf>,
}

impl LoadSettings {
    /// The trusted user keys' file: the one the game reads and writes.
    pub fn keys_path(&self) -> Option<PathBuf> {
        self.keys_file
            .clone()
            .or_else(ocw_worldstore::userkeys::default_path)
    }

    /// What the studio trusts: the game's built-in and extra roots, the
    /// user's trusted keys and the keys accepted with *Load once* in this
    /// run.
    pub fn trust_config(&self) -> TrustConfig {
        TrustConfig {
            dev_roots: cfg!(debug_assertions) || self.trust_dev_root,
            root_files: self.trust_root.clone(),
            user_keys_file: self.keys_path(),
            ..TrustConfig::for_build()
        }
    }

    pub fn options(&self) -> Result<LoadOptions, String> {
        let trust = self.trust_config().trust_store()?;
        Ok(LoadOptions {
            allow_unsigned: self.allow_unsigned,
            ..LoadOptions::new(trust, WorldCache::new(WorldCache::default_root()))
        })
    }
}

/// The trusted world keys of the trust dialog and `world trust` (the
/// game's file, [`LoadSettings::keys_path`]; a corrupt one trusts none and
/// is not overwritten until the list changes).
pub fn trusted_keys(settings: &LoadSettings) -> ocw_ui::trust::TrustedKeys {
    let builtin = settings.trust_config().builtin_store().unwrap_or_default();
    let store = settings
        .keys_path()
        .map(|p| Box::new(ocw_ui::settings::FileStore(p)) as Box<dyn ocw_ui::settings::TextStore>);
    let keys = ocw_ui::trust::TrustedKeys::load(store, builtin);
    if let Some(w) = &keys.warning {
        warn!("{w}");
    }
    keys
}

/// Whether `spec` needs the user's consent before it opens:
/// `Ok(Some(request))` for an intact package signed by a user key not
/// trusted here (the trust dialog), `Ok(None)` to load it (or let the
/// loader say why not), `Err` for a damaged package, a bad signature or a
/// revoked key – refused, never asked about.
pub fn consent(spec: &str, settings: &LoadSettings) -> Result<Option<TrustRequest>, String> {
    let opts = settings.options()?;
    match ocw_worldstore::consent_needed(spec, &opts) {
        Ok(r) => Ok(r),
        Err(LoadError::NotFound(..)) => Ok(None),
        Err(e) => Err(format!("refused: {e}")),
    }
}

/// The game's trust dialog in the studio (spec 16H Q14 (a)): the shared
/// menu (`ocw_ui::menu`, keys / pad / mouse / touch) – only for the
/// dialog, Esc and Start do not open the game's menu here – the trusted
/// keys of `settings` and `world trust list | add | remove | show |
/// accept | once | decline` in the console.
pub struct TrustDialogPlugin(pub LoadSettings);

impl Plugin for TrustDialogPlugin {
    fn build(&self, app: &mut App) {
        use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
        app.insert_resource(ocw_ui::menu::MenuHooks {
            // The studio has no game menu: Esc is its own.
            escape: |_| true,
            ..Default::default()
        })
        .add_plugins(ocw_ui::menu::MenuPlugin)
        .insert_resource(trusted_keys(&self.0))
        .console_command(ConsoleCommand::new(
            Permission::User,
            clap::Command::new("world")
                .about("World keys (spec 4.7 User keys): world trust …")
                .subcommand_required(true)
                .subcommand(ocw_ui::trust::command()),
            |world, m| match m.subcommand() {
                Some(("trust", t)) => ocw_ui::trust::run(world, t),
                _ => Err("world trust list | add | remove | show | accept | once | decline".into()),
            },
        ));
    }
}

/// The console line that opens `spec` (run again after the dialog).
pub fn open_line(spec: &str) -> String {
    format!("studio open \"{spec}\"")
}

/// A package loaded for the studio.
pub struct Loaded {
    pub spec: String,
    pub render: PackageWorld,
    pub details: WorldDetails,
    /// The World workspace's rows ([`crate::info::package_rows`]).
    pub rows: Vec<(String, String)>,
    pub map: MapData,
    /// The border and the neighbours on the map (16H Q15).
    pub surround: crate::surround::Surround,
    /// Where the camera starts: above the middle of the package.
    pub start: [f64; 3],
    /// The package file and its manifest (the patch goes next to it).
    pub file: PathBuf,
    pub manifest: Option<PathBuf>,
}

/// Loads `spec` (a path or an id) with its installed neighbours.
pub fn load(spec: &str, settings: &LoadSettings) -> Result<Loaded, String> {
    let opts = settings.options()?;
    let world = ocw_worldstore::load_world(spec, &opts).map_err(|e| e.to_string())?;
    let hood = ocw_worldstore::neighbourhood(&world, spec, &opts, !settings.no_neighbours);
    for line in &hood.log {
        info!("{line}");
    }
    let details = WorldDetails::of(&world, &hood.worlds);
    let sections = std::fs::read(&world.path)
        .map(|b| ocw_world::package::section_sizes(&b))
        .unwrap_or_default();
    let pkg = &world.package;
    let rows = crate::info::package_rows(&details, pkg, &sections);
    let map = crate::map::map_data(pkg);
    // The map's neighbours are the skeletons just joined (no further
    // loading); one by a user key not trusted here only as an outline.
    let unknown = if settings.no_neighbours {
        Vec::new()
    } else {
        ocw_worldstore::neighbours::unknown_key_neighbours(&world, &hood, &opts)
    };
    for u in &unknown {
        info!(
            "neighbour {}: signed by an unknown key ('{}'): drawn as an outline, not opened",
            u.spec, u.claimed_signer
        );
    }
    let regions: Vec<_> = hood
        .worlds
        .iter()
        .map(|w| w.package.region.clone())
        .collect();
    let surround =
        crate::surround::surround(pkg, (map.min, map.max), &hood.set, &regions, &unknown);
    let mid = [
        (map.min[0] + map.max[0]) * 0.5,
        (map.min[1] + map.max[1]) * 0.5,
    ];
    let ground = pkg.surface_height(mid[0], mid[1]).unwrap_or(0.0);
    let start = [mid[0], ground + crate::flight::START_HEIGHT_M, mid[1]];
    info!("world package {}", world.describe());
    let file = world.path.clone();
    let manifest = world.manifest_path.clone();
    drop(world);
    Ok(Loaded {
        spec: spec.into(),
        render: ocw_ui::worlds::package_world(hood, Some(opts)),
        details,
        rows,
        map,
        surround,
        start,
        file,
        manifest,
    })
}

/// The open world of the studio (what the panels and the map show).
#[derive(Resource)]
pub struct OpenWorld {
    pub spec: String,
    pub details: WorldDetails,
    pub rows: Vec<(String, String)>,
    pub map: MapData,
    /// The map's land-use texture (made by the UI in the context that
    /// draws the map, again when that context changes).
    pub land_texture: crate::map::LandTexture,
    pub map_view: Option<crate::map::MapView>,
    /// The border and the neighbours (16H Q15).
    pub surround: crate::surround::Surround,
    /// The aerial-photo layer, where the package has an orthophoto
    /// corridor (decoded only while the layer is on).
    pub aerial: Option<crate::aerial::AerialLayer>,
    /// The next fit (no `map_view`) takes the neighbours in too.
    pub fit_all: bool,
}

/// Shows `loaded` in the studio: the renderer's world, the camera above
/// it, the World workspace's document. `rebuild`: the scene is there
/// already (an open while running); at start-up the renderer spawns it.
pub fn show(world: &mut World, loaded: Loaded, now_s: f64, rebuild: bool) {
    let Loaded {
        spec,
        render,
        details,
        rows,
        map,
        surround,
        start,
        file,
        manifest,
    } = loaded;
    let aerial = crate::aerial::AerialLayer::of(render.home().clone());
    let package = crate::patch::StudioPackage::new(render.home().clone(), file, manifest);
    world.insert_resource(render);
    if let Some(mut view) = world.get_resource_mut::<RideView>() {
        // No route, no riders: the package alone, as in a free ride.
        view.free_ride = true;
        view.riders.clear();
    }
    let model = world.resource::<crate::flight::StudioFlight>().flight.model;
    world.insert_resource(crate::flight::StudioFlight::at(start, 0.0, model));
    let start_view = world.resource::<crate::flight::StudioFlight>().view();
    world.resource_mut::<RideView>().flight = Some(start_view);
    if rebuild {
        ocw_render::rebuild_scene(world);
    }
    crate::edit::open_world_doc(world, &spec, &details, package, now_s);
    world.insert_resource(OpenWorld {
        spec,
        details,
        rows,
        map,
        land_texture: Default::default(),
        map_view: None,
        surround,
        aerial,
        fit_all: false,
    });
}

/// What [`prepare`] found.
pub enum Prepared {
    /// Loaded: show it.
    Loaded(Box<Loaded>),
    /// The trust dialog asks first (the console's line); the open runs
    /// again once the user trusted the key or chose *Load once*.
    Asked(String),
}

/// The trust check and the loading of an open: a package by a user key
/// not trusted here asks with the trust dialog ([`consent`]); a damaged
/// one is refused.
pub fn prepare(world: &mut World, spec: &str) -> Result<Prepared, String> {
    let settings = world
        .get_resource::<LoadSettings>()
        .cloned()
        .unwrap_or_default();
    if let Some(request) = consent(spec, &settings)? {
        let msg = ocw_ui::trust::ask(world, request, Some(open_line(spec)), None, false);
        return Ok(Prepared::Asked(msg));
    }
    Ok(Prepared::Loaded(Box::new(load(spec, &settings)?)))
}

/// Opens `spec` now (File → Open, `studio open`, `--world`); the message
/// for the status line.
pub fn open(world: &mut World, spec: &str) -> Result<String, String> {
    let loaded = match prepare(world, spec)? {
        Prepared::Asked(msg) => return Ok(msg),
        Prepared::Loaded(l) => *l,
    };
    let msg = format!(
        "opened {} ({}); beside it: {}",
        loaded.details.region,
        loaded.details.id,
        loaded.surround.summary()
    );
    let now = world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    show(world, loaded, now, true);
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use ocw_client::menu::model::{Action, Page};
    use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleCorePlugin, ConsoleState};
    use ocw_render::menu::MenuView;
    use ocw_ui::menu::MenuState;
    use ocw_ui::trust::{TrustDialog, TrustedKeys};
    use ocw_world::manifest::Fingerprint;
    use ocw_worldstore::userkeys;

    /// The worlds the test's `studio open` loaded.
    #[derive(Resource, Default)]
    struct Opened(Vec<String>);

    fn temp(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("ocw-studio-trust-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A world package signed by a throwaway user key (test only;
    /// fictional names), written into `dir`; its key's fingerprint.
    fn user_signed_package(dir: &Path, signer: &str, seed: u8) -> (PathBuf, Fingerprint) {
        use ocw_world::manifest::{
            Attribution, Manifest, RootKeyPair, SignedManifest, SigningKeyPair,
        };
        use ocw_world::package::{Tile, TileKey};
        let now = ocw_worldstore::now_unix_s();
        let root = RootKeyPair::from_seed(signer, [seed; 32]);
        let cert = root.certify(
            signer,
            SigningKeyPair::public_key_of([seed ^ 0x5a; 32]),
            now - 3600,
            now + 3600,
        );
        let key = SigningKeyPair::new([seed ^ 0x5a; 32], cert)
            .unwrap()
            .with_root(root.public())
            .unwrap();
        let tile = TileKey { x: 0, z: 0 };
        let bytes = ocw_world::WorldPackage {
            name: "Vaduz Fixes".into(),
            origin: (47.14, 9.52),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: [(
                tile,
                Tile {
                    key: tile,
                    heights: vec![450.0; 4],
                    roads: Vec::new(),
                },
            )]
            .into(),
            graph: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            landuse: None,
            buildings: None,
            bridges: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
        }
        .to_bytes();
        let mut m = Manifest::new(
            "user/ada/vaduz",
            "Vaduz Fixes",
            2,
            now,
            "test",
            vec![Attribution::openstreetmap(), Attribution::srtm()],
        )
        .unwrap();
        m.add_file("vaduz.ocwpkg", &bytes).unwrap();
        let p = dir.join("vaduz.ocwpkg");
        std::fs::write(&p, &bytes).unwrap();
        std::fs::write(
            dir.join("vaduz.ocwpkg.manifest.json"),
            SignedManifest::signed(&m, &key).to_json(),
        )
        .unwrap();
        (p, root.public().fingerprint())
    }

    /// The studio's trust dialog and open path, headless, with the trusted
    /// keys in `keys` (the game's file format).
    fn app(keys: &Path) -> App {
        let settings = LoadSettings {
            no_neighbours: true,
            keys_file: Some(keys.to_path_buf()),
            ..Default::default()
        };
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Opened>()
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .insert_resource(settings.clone())
            .add_plugins(TrustDialogPlugin(settings));
        // `studio open` as in the studio, without drawing the scene.
        app.console_command(ConsoleCommand::new(
            ocw_render::console::Permission::User,
            clap::Command::new("studio")
                .subcommand(clap::Command::new("open").arg(clap::Arg::new("path").required(true))),
            |world, m| {
                let spec = m
                    .subcommand_matches("open")
                    .and_then(|a| a.get_one::<String>("path"))
                    .cloned()
                    .unwrap_or_default();
                match prepare(world, &spec)? {
                    Prepared::Asked(msg) => Ok(msg),
                    Prepared::Loaded(l) => {
                        world.resource_mut::<Opened>().0.push(l.spec.clone());
                        Ok(format!("opened {}", l.details.id))
                    }
                }
            },
        ));
        app.update();
        app
    }

    fn frames(app: &mut App, n: usize) {
        for _ in 0..n {
            app.update();
        }
    }

    /// Runs a console line; the log lines it added.
    fn run(app: &mut App, line: &str) -> String {
        let before = app.world().resource::<ConsoleState>().log().len();
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue(line.to_string());
        frames(app, 2);
        app.world().resource::<ConsoleState>().log()[before..].join("\n")
    }

    fn menu(app: &mut App, actions: &[Action]) {
        for a in actions {
            app.world_mut().resource_mut::<MenuState>().queue(*a);
            frames(app, 1);
        }
        frames(app, 3);
    }

    fn opened(app: &App) -> &[String] {
        &app.world().resource::<Opened>().0
    }

    fn pending(app: &App) -> bool {
        app.world().resource::<TrustDialog>().is_pending()
    }

    /// Spec 16H Q14 (a): an unknown user key asks with the game's dialog;
    /// *Trust this key* remembers it in the game's list and opens.
    #[test]
    fn an_unknown_key_asks_and_trust_remembers_it_in_the_shared_list() {
        let d = temp("trust");
        let keys = d.join("trusted-keys.json");
        let (pkg, fp) = user_signed_package(&d, "Ada Rider", 141);
        let spec = pkg.display().to_string();
        let mut app = app(&keys);
        let log = run(&mut app, &open_line(&spec));
        assert!(
            log.contains("signed by 'Ada Rider' (unverified name)"),
            "{log}"
        );
        assert!(opened(&app).is_empty(), "not opened before the answer");
        frames(&mut app, 3);
        // The game's dialog: the shared menu page, Cancel focused, the
        // fingerprint large, the claimed name, world, id, version, data.
        let page = app.world().resource::<MenuState>().menu.page();
        assert_eq!(page, Some(Page::ConfirmTrust));
        let v = app.world().resource::<MenuView>().clone();
        assert!(v.open);
        assert_eq!(v.items, ["Trust this key", "Load once", "Cancel"]);
        assert_eq!(v.focused, 2, "Cancel");
        assert_eq!(v.emphasis.join(" "), fp.to_string());
        assert_eq!(v.status, "Signed by 'Ada Rider' (unverified name):");
        let text = v.text.join("\n");
        assert!(text.contains("Vaduz Fixes (user/ada/vaduz v2"), "{text}");
        assert!(
            text.contains("OpenStreetMap") && text.contains("ODbL"),
            "{text}"
        );
        // Trust this key: up twice, Enter.
        menu(&mut app, &[Action::Up, Action::Up, Action::Confirm]);
        assert!(!pending(&app));
        assert_eq!(opened(&app), std::slice::from_ref(&spec));
        assert!(!app.world().resource::<MenuView>().open);
        // Remembered in the shared file, in the game's format.
        let saved = std::fs::read_to_string(&keys).unwrap();
        let (list, warning) = userkeys::UserKeys::load(Ok(Some(saved)));
        assert!(warning.is_none());
        assert_eq!(list.keys.len(), 1);
        assert_eq!(list.keys[0].fingerprint(), fp);
        assert_eq!(list.keys[0].name, "Ada Rider");
        // The next open asks no more.
        let log = run(&mut app, &open_line(&spec));
        assert!(log.contains("opened user/ada/vaduz"), "{log}");
        assert!(!pending(&app));
        // The studio's default list is the game's file.
        assert_eq!(
            LoadSettings::default().keys_path(),
            userkeys::default_path()
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    /// *Load once* opens without storing; *Cancel* (and Esc) does not open.
    #[test]
    fn load_once_opens_without_storing_and_cancel_does_not_open() {
        let d = temp("once");
        let keys = d.join("trusted-keys.json");
        let (pkg, fp) = user_signed_package(&d, "Bea Climber", 142);
        let spec = pkg.display().to_string();
        let mut app = app(&keys);
        // Cancel (focused): Enter.
        run(&mut app, &open_line(&spec));
        frames(&mut app, 3);
        assert!(pending(&app));
        menu(&mut app, &[Action::Confirm]);
        assert!(!pending(&app));
        assert!(opened(&app).is_empty());
        assert!(!keys.exists(), "nothing written");
        assert!(
            app.world()
                .resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l.contains("did not trust"))
        );
        // Back (Esc, B) on the dialog declines too.
        run(&mut app, &open_line(&spec));
        frames(&mut app, 3);
        menu(&mut app, &[Action::Back]);
        assert!(!pending(&app) && opened(&app).is_empty());
        // Load once: a click on the second item.
        run(&mut app, &open_line(&spec));
        frames(&mut app, 3);
        menu(&mut app, &[Action::Click(1)]);
        assert_eq!(opened(&app), std::slice::from_ref(&spec));
        assert!(!keys.exists(), "Load once is not remembered");
        let id = userkeys::once_roots()
            .iter()
            .find(|k| k.fingerprint() == fp)
            .map(|k| k.key_id)
            .expect("accepted until the studio quits");
        // Until the studio quits: the next open in this run asks no more.
        let log = run(&mut app, &open_line(&spec));
        assert!(log.contains("opened"), "{log}");
        assert!(!pending(&app));
        userkeys::forget_once(id);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// A damaged package or a bad signature is refused, never asked about;
    /// a corrupt list trusts none and is not overwritten.
    #[test]
    fn damaged_packages_are_refused_without_a_dialog() {
        let d = temp("damaged");
        let keys = d.join("trusted-keys.json");
        std::fs::write(&keys, "{ broken").unwrap();
        let (pkg, _) = user_signed_package(&d, "Cy Sprinter", 143);
        let spec = pkg.display().to_string();
        let mut app = app(&keys);
        assert!(app.world().resource::<TrustedKeys>().warning.is_some());
        // The file does not match its manifest.
        let mut bytes = std::fs::read(&pkg).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 0xff;
        std::fs::write(&pkg, &bytes).unwrap();
        let log = run(&mut app, &open_line(&spec));
        assert!(log.contains("refused"), "{log}");
        frames(&mut app, 3);
        assert!(!pending(&app));
        assert!(!app.world().resource::<MenuView>().open);
        assert!(opened(&app).is_empty());
        // A bad signature: the manifest changed after signing.
        let (pkg, _) = user_signed_package(&d, "Cy Sprinter", 143);
        let mp = d.join("vaduz.ocwpkg.manifest.json");
        let text = std::fs::read_to_string(&mp).unwrap();
        std::fs::write(&mp, text.replace("Vaduz Fixes", "Vaduz Fixed")).unwrap();
        let log = run(&mut app, &open_line(&pkg.display().to_string()));
        assert!(log.contains("refused"), "{log}");
        frames(&mut app, 3);
        assert!(!pending(&app) && opened(&app).is_empty());
        // The corrupt list was left as it was.
        assert_eq!(std::fs::read_to_string(&keys).unwrap(), "{ broken");
        // An intact package by an unknown key still asks (none trusted).
        let (pkg, _) = user_signed_package(&d, "Cy Sprinter", 143);
        run(&mut app, &open_line(&pkg.display().to_string()));
        assert!(pending(&app));
        assert_eq!(std::fs::read_to_string(&keys).unwrap(), "{ broken");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// `world trust …` in the studio's console, on the shared list; a
    /// built-in root cannot be added.
    #[test]
    fn the_console_has_world_trust() {
        let d = temp("console");
        let keys = d.join("trusted-keys.json");
        let (pkg, fp) = user_signed_package(&d, "Dee Descender", 144);
        let mut app = app(&keys);
        let log = run(&mut app, &format!("world trust add \"{}\"", pkg.display()));
        assert!(log.contains("trusted 'Dee Descender'"), "{log}");
        assert!(
            std::fs::read_to_string(&keys)
                .unwrap()
                .contains(&fp.to_string())
        );
        let log = run(&mut app, "world trust list");
        assert!(log.contains("'Dee Descender'"), "{log}");
        let log = run(&mut app, &format!("world trust remove {}", fp.hex()));
        assert!(log.contains("no longer trusted"), "{log}");
        // A development root is built in (debug builds trust it): the list
        // cannot take it.
        if let Some(dev) = ocw_worldstore::trust::dev_roots().first() {
            let mut k = app.world_mut().resource_mut::<TrustedKeys>();
            let e = k.trust(dev, None, None, 0).unwrap_err();
            assert!(e.contains("built-in"), "{e}");
        }
        let _ = std::fs::remove_dir_all(&d);
    }
}

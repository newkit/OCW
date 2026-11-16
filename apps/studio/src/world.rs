//! Opening a world package in the studio (spec §14 Phase 16, 16A, 16C):
//! loaded and verified exactly as the game loads it (`ocw_worldstore`:
//! signature and trust, the installed neighbours), handed to the renderer
//! as the game's [`PackageWorld`] (shared `ocw_ui::worlds`), plus what the
//! World workspace shows and the 2D map draws.

use std::path::PathBuf;

use bevy::prelude::*;
use ocw_render::RideView;
use ocw_render::package::PackageWorld;
use ocw_worldstore::details::WorldDetails;
use ocw_worldstore::trust::TrustConfig;
use ocw_worldstore::{LoadOptions, WorldCache};

use crate::map::MapData;

/// How packages are loaded and trusted (the game's options).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct LoadSettings {
    pub allow_unsigned: bool,
    pub trust_dev_root: bool,
    pub trust_root: Vec<PathBuf>,
    pub no_neighbours: bool,
}

impl LoadSettings {
    pub fn options(&self) -> Result<LoadOptions, String> {
        let trust = TrustConfig {
            dev_roots: cfg!(debug_assertions) || self.trust_dev_root,
            root_files: self.trust_root.clone(),
            ..TrustConfig::for_build()
        }
        .trust_store()?;
        Ok(LoadOptions {
            allow_unsigned: self.allow_unsigned,
            ..LoadOptions::new(trust, WorldCache::new(WorldCache::default_root()))
        })
    }
}

/// A package loaded for the studio.
pub struct Loaded {
    pub spec: String,
    pub render: PackageWorld,
    pub details: WorldDetails,
    /// The World workspace's rows ([`crate::info::package_rows`]).
    pub rows: Vec<(String, String)>,
    pub map: MapData,
    /// Where the camera starts: above the middle of the package.
    pub start: [f64; 3],
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
    let map = MapData::of(pkg);
    let mid = [
        (map.min[0] + map.max[0]) * 0.5,
        (map.min[1] + map.max[1]) * 0.5,
    ];
    let ground = pkg.surface_height(mid[0], mid[1]).unwrap_or(0.0);
    let start = [mid[0], ground + crate::flight::START_HEIGHT_M, mid[1]];
    info!("world package {}", world.describe());
    drop(world);
    Ok(Loaded {
        spec: spec.into(),
        render: ocw_ui::worlds::package_world(hood, Some(opts)),
        details,
        rows,
        map,
        start,
    })
}

/// The open world of the studio (what the panels and the map show).
#[derive(Resource)]
pub struct OpenWorld {
    pub spec: String,
    pub details: WorldDetails,
    pub rows: Vec<(String, String)>,
    pub map: MapData,
    /// The map's land-use texture (made by the UI on first use).
    pub land_texture: Option<bevy_egui::egui::TextureHandle>,
    pub map_view: Option<crate::map::MapView>,
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
        start,
    } = loaded;
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
    crate::edit::open_world_doc(world, &spec, &details, now_s);
    world.insert_resource(OpenWorld {
        spec,
        details,
        rows,
        map,
        land_texture: None,
        map_view: None,
    });
}

/// Opens `spec` now (File → Open, `studio open`, `--world`); the message
/// for the status line.
pub fn open(world: &mut World, spec: &str) -> Result<String, String> {
    let settings = world
        .get_resource::<LoadSettings>()
        .cloned()
        .unwrap_or_default();
    let loaded = load(spec, &settings)?;
    let msg = format!("opened {} ({})", loaded.details.region, loaded.details.id);
    let now = world
        .get_resource::<Time<Real>>()
        .map_or(0.0, |t| t.elapsed_secs_f64());
    show(world, loaded, now, true);
    Ok(msg)
}

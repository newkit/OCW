//! God mode's inspection (spec §14 Phase 13D, milestone G3): the panel
//! (I: position in package metres, latitude / longitude, tile, local
//! time), the crosshair that names what it points at (a ray from the
//! camera into the world, [`ocw_world::inspect`]), and the pins (P: kept
//! for the ride; `god.pins` lists them with a ready-to-copy `--start X,Z`).
//!
//! The texts and the pins are pure ([`panel_lines`], [`Pins`]); the Bevy
//! system [`inspect`] casts the ray a few times a second.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_world::TileKey;
use ocw_world::geo::LocalProjection;
use ocw_world::inspect::{self as world_inspect, Inspection};

use crate::connection::ClientState;

/// The crosshair and the panel are refreshed this often (Hz).
pub const INSPECT_HZ: f64 = 5.0;

/// A pin (P): where and what was there.
#[derive(Debug, Clone, PartialEq)]
pub struct Pin {
    pub number: u32,
    /// Local metres (x, y, z): on the ground (or roof) the crosshair hit.
    pub pos: [f64; 3],
    /// What the crosshair named there (its first line).
    pub label: String,
}

/// The pins of the ride.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Pins {
    pins: Vec<Pin>,
}

impl Pins {
    /// Puts a pin; returns its number.
    pub fn add(&mut self, pos: [f64; 3], label: impl Into<String>) -> u32 {
        let number = self.pins.last().map_or(1, |p| p.number + 1);
        self.pins.push(Pin {
            number,
            pos,
            label: label.into(),
        });
        number
    }

    pub fn list(&self) -> &[Pin] {
        &self.pins
    }

    pub fn clear(&mut self) {
        self.pins.clear();
    }

    /// One line per pin: number, coordinates, latitude / longitude (with a
    /// geographic origin), what was there and the `--start X,Z` to ride
    /// from it (free ride).
    pub fn lines(&self, geo_origin: Option<(f64, f64)>) -> Vec<String> {
        if self.pins.is_empty() {
            return vec![
                "no pins yet (P in unicorn mode puts one where the crosshair points)".into(),
            ];
        }
        self.pins
            .iter()
            .map(|p| {
                let geo = geo_origin
                    .map(|o| {
                        let (lat, lon) =
                            LocalProjection::new(o.0, o.1).unproject([p.pos[0], p.pos[2]]);
                        format!("  {}", lat_lon(lat, lon))
                    })
                    .unwrap_or_default();
                format!(
                    "pin {}: x {:.0} z {:.0}, {:.0} m{geo}  {}  --start {:.0},{:.0}",
                    p.number, p.pos[0], p.pos[2], p.pos[1], p.label, p.pos[0], p.pos[2]
                )
            })
            .collect()
    }
}

/// "47.14123° N 9.52151° E".
pub fn lat_lon(lat: f64, lon: f64) -> String {
    format!(
        "{:.5}° {} {:.5}° {}",
        lat.abs(),
        if lat >= 0.0 { "N" } else { "S" },
        lon.abs(),
        if lon >= 0.0 { "E" } else { "W" }
    )
}

/// What the panel shows besides the flight line (altitude, speed, factor
/// and heading are in the big line, `hud::flight_line`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PanelFacts {
    /// The unicorn (local metres).
    pub pos: [f64; 3],
    pub height_m: f64,
    pub speed_mps: f64,
    pub scale: f64,
    /// Compass heading (degrees, 0 = north).
    pub heading_deg: f64,
    pub geo_origin: Option<(f64, f64)>,
    /// The package and tile under the unicorn.
    pub tile: Option<(String, TileKey)>,
    /// The local time of the place (Phase 9): hour, minute, UTC offset (s).
    pub local_time: Option<(u32, u32, i32)>,
    pub pins: usize,
}

/// The inspection panel's lines (spec §14 Phase 13D).
pub fn panel_lines(f: &PanelFacts) -> Vec<String> {
    let mut out = vec![
        format!(
            "Altitude {:.0} m ASL · {:.0} m above ground",
            f.pos[1],
            f.height_m.max(0.0)
        ),
        format!(
            "Speed {:.0} km/h · speed factor x{:.2} · heading {:03.0}° {}",
            f.speed_mps * 3.6,
            f.scale,
            f.heading_deg,
            ocw_render::hud::compass(f.heading_deg)
        ),
        format!("Position x {:.0} z {:.0} m", f.pos[0], f.pos[2]),
    ];
    if let Some((lat0, lon0)) = f.geo_origin {
        let (lat, lon) = LocalProjection::new(lat0, lon0).unproject([f.pos[0], f.pos[2]]);
        out.push(lat_lon(lat, lon));
    }
    if let Some((pkg, key)) = &f.tile {
        out.push(format!("Tile {}, {} of {pkg}", key.x, key.z));
    }
    if let Some((h, m, offset)) = f.local_time {
        let sign = if offset < 0 { '-' } else { '+' };
        let o = offset.unsigned_abs();
        out.push(format!(
            "Local time {h:02}:{m:02} (UTC{sign}{:02}:{:02})",
            o / 3600,
            (o / 60) % 60
        ));
    }
    out.push(format!(
        "Pins: {} · P: pin · I: hide · god.pins lists them",
        f.pins
    ));
    out
}

/// The inspection: shown or not (I), the last crosshair result, the pins.
#[derive(Resource, Debug, Clone)]
pub struct Inspector {
    pub shown: bool,
    pub crosshair: Option<Inspection>,
    pub panel: Vec<String>,
    pub pins: Pins,
    last_s: f64,
}

impl Default for Inspector {
    fn default() -> Self {
        Self {
            shown: true,
            crosshair: None,
            panel: Vec::new(),
            pins: Pins::default(),
            last_s: f64::NEG_INFINITY,
        }
    }
}

/// The package and tile at `p` (home metres).
pub fn tile_at(set: &ocw_world::WorldSet, p: [f64; 2]) -> Option<(String, TileKey)> {
    let i = set.owner(p)?;
    let m = &set.members()[i];
    let pkg = m.package()?;
    Some((
        m.id.clone(),
        TileKey::containing(m.frame.invert(p), f64::from(pkg.tile_size_m)),
    ))
}

/// What the world offers to the ray: terrain and buildings (the camera's
/// solid obstacles), the water of the packages.
type RayWorld<'w> = (
    Option<Res<'w, ocw_render::Ground>>,
    Res<'w, ocw_render::camera::CameraObstacles>,
    Option<Res<'w, ocw_render::package::PackageWorld>>,
);

/// Casts the crosshair's ray from the camera along the view; returns the
/// hit and what is there.
pub fn cast(
    client: &crate::client::ClientCore,
    (ground, obstacles, packages): &(
        Option<&ocw_render::Ground>,
        &ocw_render::camera::CameraObstacles,
        Option<&ocw_render::package::PackageWorld>,
    ),
) -> Option<Inspection> {
    let g = client.god();
    let f = g.flight();
    let solid = |x: f64, z: f64| ocw_render::camera::solid_height(*ground, obstacles, x, z);
    let view = ocw_render::FlightView {
        pos: f.pos,
        yaw: f.view_yaw(),
        pitch: f.view_pitch(),
        first_person: g.first_person(),
        ..Default::default()
    };
    let (eye, target) = ocw_render::camera::flight_eye(&view, &solid);
    let dir = (target - eye).normalize_or_zero();
    let set = packages.map(|p| p.set.as_ref());
    let surface = |x: f64, z: f64| {
        let water = set.and_then(|s| world_inspect::set_water_level(s, [x, z]));
        match (solid(x, z), water) {
            (Some(a), Some(w)) => Some(a.max(w)),
            (a, w) => a.or(w),
        }
    };
    let hit = world_inspect::raycast(
        [f64::from(eye.x), f64::from(eye.y), f64::from(eye.z)],
        [f64::from(dir.x), f64::from(dir.y), f64::from(dir.z)],
        world_inspect::MAX_RAY_M,
        &surface,
    )?;
    let heading = {
        let h = f64::from(dir.x).hypot(f64::from(dir.z));
        (h > 1e-6).then(|| [f64::from(dir.x) / h, f64::from(dir.z) / h])
    };
    Some(match set {
        Some(s) => world_inspect::inspect(s, [hit[0], hit[2]], hit[1], heading),
        None => Inspection {
            point: hit,
            elevation_m: ground.and_then(|g| (g.0)(hit[0], hit[2])),
            ..Default::default()
        },
    })
}

/// I toggles the inspection, P puts a pin; a few times a second the ray
/// and the panel are refreshed.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub(crate) fn inspect(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    (ground, obstacles, packages): RayWorld<'_>,
    view: Res<RideView>,
    client: Res<ClientState>,
    mut inspector: ResMut<Inspector>,
    mut console: ResMut<ConsoleState>,
) {
    let c = &client.0;
    if !c.god().flying() {
        inspector.crosshair = None;
        inspector.panel.clear();
        return;
    }
    let world = (ground.as_deref(), &*obstacles, packages.as_deref());
    let now = time.elapsed_secs_f64();
    if ocw_render::menu::ride_input(context) {
        if keys.just_pressed(KeyCode::KeyI) {
            inspector.shown = !inspector.shown;
            inspector.last_s = f64::NEG_INFINITY;
        }
        if keys.just_pressed(KeyCode::KeyP) {
            console.print(put_pin(c, &world, &mut inspector));
        }
    }
    if !inspector.shown {
        inspector.crosshair = None;
        inspector.panel.clear();
        return;
    }
    if now - inspector.last_s < 1.0 / INSPECT_HZ {
        return;
    }
    inspector.last_s = now;
    inspector.crosshair = cast(c, &world);
    let g = c.god();
    let f = g.flight();
    let facts = PanelFacts {
        pos: f.pos,
        height_m: g.height_m(),
        speed_mps: f.speed(),
        scale: f.scale,
        heading_deg: ocw_sim::god::heading_deg(f.yaw),
        geo_origin: c.geo_origin(),
        tile: packages
            .as_deref()
            .and_then(|p| tile_at(&p.set, [f.pos[0], f.pos[2]])),
        local_time: view
            .sky
            .as_ref()
            .map(|s| (s.sky.local.hour, s.sky.local.minute, s.sky.local.offset_s)),
        pins: inspector.pins.list().len(),
    };
    inspector.panel = panel_lines(&facts);
}

/// Puts a pin where the crosshair points (below the unicorn when it
/// points at the sky).
fn put_pin(
    c: &crate::client::ClientCore,
    world: &(
        Option<&ocw_render::Ground>,
        &ocw_render::camera::CameraObstacles,
        Option<&ocw_render::package::PackageWorld>,
    ),
    inspector: &mut Inspector,
) -> String {
    let (pos, label) = match cast(c, world) {
        Some(i) => (i.point, i.lines().first().cloned().unwrap_or_default()),
        None => {
            let p = c.god().flight().pos;
            let y = world.0.and_then(|g| (g.0)(p[0], p[2])).unwrap_or(p[1]);
            ([p[0], y, p[2]], "below the unicorn".into())
        }
    };
    let n = inspector.pins.add(pos, label);
    inspector.last_s = f64::NEG_INFINITY;
    let line = inspector
        .pins
        .lines(c.geo_origin())
        .pop()
        .unwrap_or_default();
    info!("unicorn mode: {line}");
    format!("pin {n} put: {line}")
}

/// The panel, the crosshair and the pins for the renderer.
pub(crate) fn update_view(inspector: Res<Inspector>, mut view: ResMut<RideView>) {
    let pins: Vec<ocw_render::PinView> = inspector
        .pins
        .list()
        .iter()
        .map(|p| ocw_render::PinView {
            number: p.number,
            pos: p.pos,
        })
        .collect();
    if view.pins != pins {
        view.pins = pins;
    }
    if let Some(f) = view.flight.as_mut() {
        f.panel.clone_from(&inspector.panel);
        f.crosshair = (!inspector.panel.is_empty()).then(|| match &inspector.crosshair {
            Some(i) => i.lines(),
            None => vec!["Sky".into()],
        });
    }
}

pub(crate) fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.pins")
            .about("Unicorn mode: list the pins (P) with coordinates and a ready-to-copy --start X,Z; 'god.pins clear' removes them")
            .arg(Arg::new("action").value_parser(["list", "clear"]).default_value("list")),
        |world, m| {
            if m.get_one::<String>("action").is_some_and(|a| a == "clear") {
                world.resource_mut::<Inspector>().pins.clear();
                return Ok("pins removed".into());
            }
            let origin = world.resource::<ClientState>().0.geo_origin();
            Ok(world.resource::<Inspector>().pins.lines(origin).join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.pin").about("Unicorn mode: put a pin where the crosshair points (P)"),
        |world, _| {
            if !world.resource::<ClientState>().0.god().flying() {
                return Err("not flying (god on first)".into());
            }
            let line = world.resource_scope(|world, mut inspector: Mut<Inspector>| {
                let ground = world.get_resource::<ocw_render::Ground>();
                let obstacles = world.resource::<ocw_render::camera::CameraObstacles>();
                let packages = world.get_resource::<ocw_render::package::PackageWorld>();
                let c = &world.resource::<ClientState>().0;
                put_pin(c, &(ground, obstacles, packages), &mut inspector)
            });
            Ok(line)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.inspect")
            .about("Unicorn mode: the inspection panel and the crosshair (I); without an argument: what the crosshair points at")
            .arg(Arg::new("state").value_parser(["on", "off"])),
        |world, m| {
            if let Some(s) = m.get_one::<String>("state") {
                let on = s == "on";
                let mut i = world.resource_mut::<Inspector>();
                i.shown = on;
                i.last_s = f64::NEG_INFINITY;
                return Ok(format!("inspection {}", if on { "on" } else { "off" }));
            }
            let i = world.resource::<Inspector>();
            match &i.crosshair {
                Some(c) => Ok(c.lines().join("\n")),
                None => Err("nothing under the crosshair (flying with the inspection on?)".into()),
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_are_numbered_and_listed_with_a_start() {
        let mut p = Pins::default();
        assert!(p.lines(None)[0].starts_with("no pins"));
        assert_eq!(p.add([1234.4, 455.0, -567.6], "Landstrasse · way 4711"), 1);
        assert_eq!(p.add([10.0, 400.0, 20.0], "Area: forest"), 2);
        let lines = p.lines(Some((47.14, 9.52)));
        assert_eq!(lines.len(), 2);
        assert!(
            lines[0].starts_with("pin 1: x 1234 z -568, 455 m"),
            "{}",
            lines[0]
        );
        assert!(lines[0].contains("Landstrasse · way 4711"), "{}", lines[0]);
        assert!(lines[0].ends_with("--start 1234,-568"), "{}", lines[0]);
        assert!(
            lines[0].contains("° N") && lines[0].contains("° E"),
            "{}",
            lines[0]
        );
        assert!(lines[1].ends_with("--start 10,20"));
        p.clear();
        assert!(p.list().is_empty());
        assert_eq!(p.add([0.0; 3], ""), 1, "numbers start again");
    }

    #[test]
    fn the_panel_shows_position_geo_tile_and_local_time() {
        let f = PanelFacts {
            pos: [1500.0, 1234.4, -2500.0],
            height_m: 380.0,
            speed_mps: 100.0,
            scale: 2.0,
            heading_deg: 45.0,
            geo_origin: Some((47.14, 9.52)),
            tile: Some(("li-vaduz".into(), TileKey { x: 1, z: -3 })),
            local_time: Some((14, 5, 3600)),
            pins: 2,
        };
        let l = panel_lines(&f);
        assert_eq!(l[0], "Altitude 1234 m ASL · 380 m above ground");
        assert_eq!(
            l[1],
            "Speed 360 km/h · speed factor x2.00 · heading 045° NE"
        );
        assert_eq!(l[2], "Position x 1500 z -2500 m");
        // 2.5 km north (−z) and 1.5 km east of the origin.
        assert!(
            l[3].starts_with("47.162") && l[3].contains("° N 9.53"),
            "{}",
            l[3]
        );
        assert_eq!(l[4], "Tile 1, -3 of li-vaduz");
        assert_eq!(l[5], "Local time 14:05 (UTC+01:00)");
        assert!(l[6].starts_with("Pins: 2"));
        // Without a geographic origin, package or clock: those lines go.
        let bare = panel_lines(&PanelFacts {
            local_time: Some((23, 0, -3 * 3600 - 1800)),
            ..Default::default()
        });
        assert_eq!(bare.len(), 5);
        assert_eq!(bare[3], "Local time 23:00 (UTC-03:30)");
        assert_eq!(lat_lon(-33.5, -70.25), "33.50000° S 70.25000° W");
    }
}

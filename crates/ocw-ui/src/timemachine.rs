//! World time and the time machine in both apps (spec §14 Phase 9, S1 /
//! S3; shared since Phase 10 B6): the session clock from the server
//! (predicted by `ClientCore`), the time zone of the rider's place in the
//! world (decision Q1), the sky for the renderer
//! ([`ocw_render::RideView::sky`]), the HUD clock, the console command
//! `time` and the World → Time page ([`TimeMachine`]; its rows:
//! `ocw_client::menu::time`).
//!
//! The zone of the place: the world package's `TZID` section; for older
//! packages and route files the zone of their origin
//! (`ocw_sky::zone::zone_for`: a region table, else the longitude's
//! nautical zone); rides without a geographic origin (the built-in test
//! routes) are in Vaduz (`ocw_sky::zone::DEFAULT_PLACE`, `Europe/Vaduz`).
//!
//! The server owns the clock and checks who may change it (§6.1): the
//! client asks with a `ClockRequest`; anyone on a local server, the host in
//! a shared ride.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::client::ClientCore;
pub use ocw_client::menu::time::*;
pub use ocw_client::sky::{hud_label, place_zone};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::sky::SkySettings;
use ocw_sky::Zone;
use ocw_sky::machine::{self, ClockChange, TimeCommand};

pub struct TimeMachinePlugin;

impl Plugin for TimeMachinePlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<TimeMachine>()
            .init_resource::<WorldPlace>()
            .add_systems(Update, update_sky.in_set(crate::ViewExtrasSet));
        register_commands(app);
    }
}

/// What the Time machine page shows, as a resource (every frame).
#[derive(Resource, Debug, Clone, Default, PartialEq, Deref, DerefMut)]
pub struct TimeMachine(pub TimeMachineInfo);

/// The time zone of the ridden place, resolved once per ride.
#[derive(Resource, Debug, Clone)]
pub struct WorldPlace {
    pub zone: Zone,
    /// Where the zone came from ("package", "origin", "default").
    pub source: &'static str,
    /// Package zone id and origin it was resolved for.
    key: (Option<String>, Option<(i64, i64)>),
}

impl Default for WorldPlace {
    fn default() -> Self {
        let (zone, source) = place_zone(None, None);
        Self {
            zone,
            source,
            key: (None, None),
        }
    }
}

fn update_sky(world: &mut World) {
    let now = crate::now(world);
    let package = world.get_resource::<ocw_render::package::PackageWorld>();
    let pkg_zone = package.and_then(|p| p.home().time_zone.clone());
    let origin = package
        .map(|p| p.home().origin)
        .or_else(|| crate::client(world).and_then(|c| c.geo_origin()));
    let key = (
        pkg_zone.clone(),
        origin.map(|(a, b)| ((a * 1e4) as i64, (b * 1e4) as i64)),
    );
    if world.resource::<WorldPlace>().key != key {
        let (zone, source) = place_zone(pkg_zone.as_deref(), origin);
        info!(zone = zone.id(), source, "world time zone");
        *world.resource_mut::<WorldPlace>() = WorldPlace { zone, source, key };
    }
    let floor = world
        .get_resource::<SkySettings>()
        .map_or(ocw_sky::light::DEFAULT_NIGHT_FLOOR, |s| s.night_floor);
    let zone = world.resource::<WorldPlace>().zone.clone();
    let prev = world.resource::<TimeMachine>().0.clone();
    let latlon = match (world.get_resource::<RideView>(), crate::client(world)) {
        (Some(v), Some(c)) => place_latlon(v, c),
        _ => ocw_sky::zone::DEFAULT_PLACE,
    };
    let out = {
        let Some(mut client) = crate::client_mut(world) else {
            return;
        };
        refresh(&mut client, now, &zone, latlon, floor, &prev)
    };
    let Some((sky, info)) = out else {
        if let Some(mut view) = world.get_resource_mut::<RideView>()
            && view.sky.is_some()
        {
            view.sky = None;
        }
        return;
    };
    if let Some(mut view) = world.get_resource_mut::<RideView>() {
        view.sky = Some(sky);
    }
    let mut tm = world.resource_mut::<TimeMachine>();
    if tm.0 != info {
        tm.0 = info;
    }
}

/// Runs `time ARGS`: shows the clock or asks the server for a change.
fn run_time(world: &mut World, args: &[&str]) -> Result<String, String> {
    let cmd = machine::parse(args)?;
    let now = crate::now(world);
    let (zone, zone_from) = {
        let p = world.resource::<WorldPlace>();
        (p.zone.clone(), p.source)
    };
    let latlon = {
        let view = world.resource::<RideView>();
        let client = crate::client(world).ok_or("not riding")?;
        place_latlon(view, client)
    };
    let (ms, clock, may) = {
        let mut c = crate::client_mut(world).ok_or("not riding")?;
        let ms = c.world_ms(now);
        (ms, c.clock_info().copied(), c.may_change_clock())
    };
    let Some(ms) = ms else {
        return Err("no world clock yet (the server sends it when you join)".into());
    };
    if cmd == TimeCommand::Show {
        let sun = ocw_sky::sun::position(ms, latlon.0, latlon.1);
        let moon = ocw_sky::moon::position(ms, latlon.0, latlon.1);
        return Ok(format!(
            "{} ({}, zone from the {}), {}; sun {:.1}° high, azimuth {:.0}°; moon {:.0} % ({}) {:.1}° high, azimuth {:.0}°; set by {}",
            long_time(&zone, ms),
            zone.id(),
            zone_from,
            clock.map_or("real, x1".into(), |c| {
                let speed = ocw_sky::clock::speed_label(c.speed, c.paused);
                format!("{}, {speed}", c.mode.name())
            }),
            sun.elevation_deg,
            sun.azimuth_deg,
            moon.illuminated * 100.0,
            moon.phase_name(),
            moon.elevation_deg,
            moon.azimuth_deg,
            control_label(clock.as_ref(), may)
        ));
    }
    if !may {
        return Err(format!(
            "{} sets the time of this ride",
            control_label(clock.as_ref(), false)
        ));
    }
    let change = machine::resolve(&cmd, ms, &zone, latlon.0, latlon.1)?.expect("not Show");
    if let TimeCommand::Preset(p) = cmd {
        world.resource_mut::<TimeMachine>().preset = Some(p);
    }
    crate::send(world, ClientCore::clock_message(change));
    Ok(match change {
        ClockChange::Fixed(at) => format!("time: {}", long_time(&zone, at)),
        _ => String::new(),
    })
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        // A user command (spec §14 Phase 9, 9D): the server decides who may
        // change the time (solo: you; a shared ride: the host).
        Permission::User,
        Command::new("time")
            .about("World time at your place in the world, and the time machine")
            .long_about(machine::USAGE)
            .arg(
                Arg::new("args")
                    .num_args(0..)
                    .allow_hyphen_values(true)
                    .action(ArgAction::Append),
            ),
        |world, m| {
            let args: Vec<&str> = m
                .get_many::<String>("args")
                .map(|v| v.map(String::as_str).collect())
                .unwrap_or_default();
            let out = run_time(world, &args);
            if let Err(e) = &out {
                world
                    .resource_mut::<ConsoleState>()
                    .print(format!("time: {e}"));
                return Err(e.clone());
            }
            out
        },
    ));
}

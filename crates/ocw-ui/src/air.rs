//! The simulated aircraft in both apps (spec 14 Phase 18, 18C, T8): each
//! frame the sky of `ocw_client::air` from the session's air seed, the
//! world clock and the weather, seen and heard from the camera – the
//! unicorn's in god mode – into [`ocw_render::RideView::air`]; the
//! renderer draws it (`ocw_render::aircraft`), the audio feed plays it.
//! Everywhere (Q9 (b)): desktop, browser and the headsets, the number by
//! the quality level.
//!
//! Commands: `air` (what flies now, the aerodromes, the upper air and the
//! contrails), `air.spot [KIND|MISSION] [METRES] [SECONDS]` (unicorn mode:
//! wait beside the path of the nearest such aircraft, where it will be in
//! SECONDS – scenes and videos).

use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::air::{AirClient, AirFrame};
use ocw_render::RideView;
use ocw_render::camera::FollowCamera;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::package::PackageWorld;
use ocw_sim::air::upper::UpperAir;
use ocw_sim::air::{AirSpace, Flight, heading_of, mix};

use crate::traffic::TrafficSettings;

/// The client's sky.
#[derive(Resource, Default)]
pub struct AirState(pub AirClient);

pub struct AirPlugin;

impl Plugin for AirPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<AirState>()
            .add_systems(Update, update_air.after(crate::ViewExtrasSet));
        register_commands(app);
    }
}

/// The world's key and how to build its airspace: the home package, else
/// the route's origin.
fn world_space(world: &World) -> Option<(u64, Box<dyn FnOnce() -> AirSpace>)> {
    if let Some(p) = world.get_resource::<PackageWorld>() {
        let home = p.home().clone();
        // By name and place: a streamed package's resident versions are
        // the same world.
        let key = home.name.bytes().fold(
            mix(home.origin.0.to_bits() ^ home.origin.1.to_bits().rotate_left(17)),
            |h, b| mix(h ^ u64::from(b)),
        );
        return Some((key, Box::new(move || AirSpace::from_package(&home))));
    }
    let origin = crate::client(world)?.geo_origin()?;
    let key = mix(origin.0.to_bits() ^ origin.1.to_bits().rotate_left(17));
    Some((
        key,
        Box::new(move || {
            AirSpace::new(
                origin,
                None,
                ocw_sim::air::relief::Relief::default(),
                |_, _| None,
            )
        }),
    ))
}

/// The camera's eye and its forward direction (x, z).
fn eye(world: &mut World) -> Option<([f64; 3], [f64; 2])> {
    let mut q = world.query_filtered::<&GlobalTransform, With<FollowCamera>>();
    let t = q.iter(world).next()?;
    let p = t.translation();
    let f = t.forward();
    Some((
        [f64::from(p.x), f64::from(p.y), f64::from(p.z)],
        [f64::from(f.x), f64::from(f.z)],
    ))
}

fn frame_inputs(world: &mut World) -> Option<AirFrame<'static>> {
    let settings = world
        .get_resource::<TrafficSettings>()
        .copied()
        .unwrap_or_default();
    let seed = crate::client(world)
        .filter(|c| c.rider_id().is_some())?
        .traffic()
        .air_seed();
    let (eye, forward) = eye(world)?;
    let view = world.get_resource::<RideView>()?;
    let clock = view
        .sky
        .as_ref()
        .map(|s| (s.sky.instant, s.sky.local.offset_s));
    let ground_m = world
        .get_resource::<ocw_render::Ground>()
        .and_then(|g| (g.0)(eye[0], eye[2]))
        .unwrap_or(0.0);
    let level = world
        .get_resource::<ocw_render::quality::RenderQuality>()
        .map_or(Default::default(), |q| q.0.level);
    let (max_aircraft, contrails) = ocw_render::aircraft::budget(level);
    Some(AirFrame {
        choice: settings.view,
        seed,
        clock,
        weather: None,
        eye,
        forward,
        ground_m,
        max_aircraft,
        contrails,
    })
}

fn update_air(world: &mut World) {
    let Some(f) = frame_inputs(world) else {
        if let Some(mut v) = world.get_resource_mut::<RideView>()
            && v.air.is_some()
        {
            v.air = None;
        }
        return;
    };
    if AirClient::shown(&f.choice)
        && let Some((key, build)) = world_space(world)
    {
        world.resource_mut::<AirState>().0.set_space(key, build);
    }
    let weather = world
        .get_resource::<RideView>()
        .and_then(|v| v.weather.as_ref().map(|w| w.state));
    let frame = AirFrame {
        weather: weather.as_ref(),
        ..f
    };
    let air = world.resource_mut::<AirState>().0.frame(&frame);
    if let Some(mut v) = world.get_resource_mut::<RideView>() {
        v.air = air;
    }
}

/// The flights alive now, the clock (s), the eye and the upper air.
type Now = (Vec<Arc<Flight>>, f64, [f64; 3], UpperAir);

/// The flights alive now near the camera, with the inputs (the commands).
fn flights_now(world: &mut World) -> Result<Now, String> {
    let f = frame_inputs(world).ok_or("not riding")?;
    if !AirClient::shown(&f.choice) {
        return Err("aircraft are off (traffic.air off, or traffic.enabled off)".into());
    }
    let weather = world
        .get_resource::<RideView>()
        .and_then(|v| v.weather.as_ref().map(|w| w.state))
        .unwrap_or_default();
    let fallback = weather;
    let inp = AirClient::inputs(&f, &fallback).ok_or("the world clock is not known yet")?;
    let mut st = world.resource_mut::<AirState>();
    let space = st.0.space().cloned().ok_or("no airspace yet")?;
    let flights = st.0.sky_mut().flights(&space, &inp);
    Ok((
        flights,
        inp.now_s,
        f.eye,
        UpperAir::from_weather(&weather, f.ground_m),
    ))
}

fn summary(world: &mut World) -> Result<String, String> {
    let note = match world
        .get_resource::<TrafficSettings>()
        .map(|s| s.view.air_shown())
    {
        Some(ocw_core::traffic::AirTraffic::Live) => {
            " (live aircraft are not available yet: simulated)"
        }
        _ => "",
    };
    let (mut flights, now, eye, upper) = flights_now(world)?;
    let st = world.resource::<AirState>();
    let space = st.0.space().cloned().ok_or("no airspace yet")?;
    let mut out = vec![format!(
        "aircraft: simulated{note}; {} aerodromes, {} sources of flights",
        space.fields().count(),
        space.generator_count()
    )];
    let fields: Vec<String> = space
        .fields()
        .take(8)
        .map(|(n, p, e)| format!("{n} ({:.0}, {:.0}, {e:.0} m)", p[0], p[1]))
        .collect();
    if !fields.is_empty() {
        out.push(format!("aerodromes: {}", fields.join(", ")));
    }
    let cruise = 10_700.0;
    out.push(format!(
        "upper air at {cruise:.0} m: {:.0} °C, {:.0} % over ice – contrails {:?}",
        upper.temperature_c(cruise),
        upper.humidity_ice(cruise) * 100.0,
        upper.contrail(cruise)
    ));
    let d = |f: &Flight| {
        let p = f.state(now).pos;
        ((p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2)).sqrt()
    };
    flights.sort_by(|a, b| d(a).total_cmp(&d(b)));
    out.push(format!(
        "{} flying now within {:.0} km:",
        flights.len(),
        ocw_client::air::SKY_RADIUS_M / 1000.0
    ));
    for f in flights.iter().take(10) {
        let s = f.state(now);
        let speed = s.vel[0].hypot(s.vel[2]);
        out.push(format!(
            "  {} {} – {:.1} km away, {:.0} m high at ({:.0}, {:.0}), heading {:.0}°, {:.0} km/h{}",
            f.kind.key(),
            f.mission.key(),
            d(f) / 1000.0,
            s.pos[1],
            s.pos[0],
            s.pos[2],
            heading_of([s.vel[0], s.vel[2]]),
            speed * 3.6,
            f.runway
                .map(|r| format!(", runway {:.0}°", r.heading_deg))
                .unwrap_or_default()
        ));
    }
    Ok(out.join("\n"))
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("air").about(
            "The simulated aircraft (traffic.air): what flies now near the camera, the aerodromes, the upper air and its contrails",
        ),
        |world, _| summary(world),
    ));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("air.spot")
            .about("Unicorn mode: wait beside the path of the nearest aircraft of a kind or mission (airliner, small, helicopter, glider, balloon; arrival, departure, cruise, circuit, rescue, soaring, drift …), METRES to its side, where it will be in SECONDS")
            .arg(Arg::new("what").default_value("any"))
            .arg(Arg::new("metres").value_parser(value_parser!(f64)).default_value("150").allow_negative_numbers(true))
            .arg(Arg::new("seconds").value_parser(value_parser!(f64)).default_value("20")),
        |world, m| {
            let what = m.get_one::<String>("what").expect("default").to_lowercase();
            let side = *m.get_one::<f64>("metres").expect("default");
            let ahead = *m.get_one::<f64>("seconds").expect("default");
            let (flights, now, eye, _) = flights_now(world)?;
            let matches = |f: &Flight| {
                what == "any"
                    || f.kind.key().starts_with(what.as_str())
                    || f.mission.key().starts_with(what.as_str())
            };
            let f = flights
                .iter()
                .filter(|f| matches(f) && f.alive(now + ahead))
                .min_by(|a, b| {
                    let d = |f: &Flight| {
                        let p = f.state(now).pos;
                        (p[0] - eye[0]).hypot(p[2] - eye[2])
                    };
                    d(a).total_cmp(&d(b))
                })
                .ok_or(format!("no {what} flying now"))?;
            let at = f.state(now + ahead);
            let dir = {
                let l = at.vel[0].hypot(at.vel[2]).max(1e-6);
                [at.vel[0] / l, at.vel[2] / l]
            };
            let right = [-dir[1], dir[0]];
            let spot = [at.pos[0] + right[0] * side, at.pos[1] + 15.0, at.pos[2] + right[1] * side];
            // Look back along the path, toward where it comes from.
            let now_pos = f.state(now).pos;
            let look = [now_pos[0] - spot[0], now_pos[1] - spot[1], now_pos[2] - spot[2]];
            let yaw = heading_of([look[0], look[2]]);
            let pitch = look[1].atan2(look[0].hypot(look[2]));
            let moved = crate::client_mut(world).is_some_and(|mut c| {
                c.god_mut()
                    .teleport(spot, Some(-yaw.to_radians()), Some(pitch))
            });
            if !moved {
                return Err("not flying (god on first)".into());
            }
            Ok(format!(
                "unicorn at ({:.0}, {:.0}, {:.0}) m beside the {} {} – it passes in {ahead:.0} s",
                spot[0],
                spot[1],
                spot[2],
                f.kind.key(),
                f.mission.key()
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_commands_need_a_ride() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(ocw_render::console::ConsoleCorePlugin)
            .add_plugins(AirPlugin);
        app.update();
        let w = app.world_mut();
        assert!(summary(w).is_err());
        assert!(w.resource::<AirState>().0.space().is_none());
    }
}

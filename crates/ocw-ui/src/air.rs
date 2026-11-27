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
//! SECONDS – scenes and videos), `air.follow …` (a camera and the unicorn
//! flying with one aircraft for close-up flyby videos:
//! [`ocw_client::air_follow`]).

use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::air::{AirClient, AirFrame};
use ocw_client::air_follow::{FollowKey, FollowPath, Formation, first_below};
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
            .init_resource::<AirFollow>()
            .add_systems(
                Update,
                (update_air, follow_air).chain().after(crate::ViewExtrasSet),
            );
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
    // A route without a place (the test routes): the default place's sky.
    let origin = crate::client(world)?
        .geo_origin()
        .unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
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

/// `air.follow`: the aircraft followed, the camera's keys and the
/// unicorn's place beside it.
#[derive(Resource, Default)]
pub struct AirFollow {
    pub flight: Option<Arc<Flight>>,
    pub path: FollowPath,
    /// The game time of `air.follow play`.
    pub since: Option<f64>,
    pub unicorn: Option<Formation>,
    /// Whether this put the camera on its way (and clears it again).
    guiding: bool,
}

/// The world clock now (s), as the sky is drawn.
fn clock_s(world: &World) -> Option<f64> {
    let v = world.get_resource::<RideView>()?;
    v.sky.as_ref().map(|s| s.sky.instant as f64 / 1000.0)
}

/// Each frame: the camera and the unicorn with the aircraft followed, from
/// the same world clock as the aircraft drawn.
fn follow_air(world: &mut World) {
    let flight = world.resource::<AirFollow>().flight.clone();
    let clock = clock_s(world);
    let (Some(f), Some(clock)) = (flight, clock) else {
        if std::mem::take(&mut world.resource_mut::<AirFollow>().guiding)
            && let Some(mut v) = world.get_resource_mut::<RideView>()
        {
            v.guided_camera = None;
        }
        return;
    };
    let (now, dt) = world
        .get_resource::<Time>()
        .map_or((0.0, 0.0), |t| (t.elapsed_secs_f64(), t.delta_secs_f64()));
    let st = world.resource::<AirFollow>();
    let t = st.since.map_or(0.0, |s| now - s);
    let state = |w: f64| f.state(w.clamp(f.start_s, f.end_s()));
    let cam = st.path.camera(t, clock, state);
    // The unicorn is drawn from the next frame's view: where the aircraft
    // will be then.
    let unicorn = st.unicorn.map(|u| u.place(&state(clock + dt)));
    if let Some((eye, look)) = cam {
        let v = state(clock).vel;
        world.resource_mut::<AirFollow>().guiding = true;
        if let Some(mut view) = world.get_resource_mut::<RideView>() {
            view.guided_camera = Some(ocw_render::GuidedCamera {
                eye,
                look,
                velocity: [v[0], v[2]],
            });
        }
    }
    if let Some((pos, heading)) = unicorn
        && let Some(mut c) = crate::client_mut(world)
    {
        c.god_mut()
            .teleport(pos, Some(-heading.to_radians()), Some(0.0));
    }
}

/// Whether a flight matches every comma-separated term of `what` (a kind
/// or a mission, by its start: `airliner,arrival`).
fn matches_terms(f: &Flight, what: &str) -> bool {
    what == "any"
        || what
            .split(',')
            .filter(|w| !w.is_empty())
            .all(|w| f.kind.key().starts_with(w) || f.mission.key().starts_with(w))
}

fn follow_command() -> Command {
    Command::new("air.follow")
        .about("A camera and the unicorn flying with one aircraft (close-up flyby videos): pick WHAT [--below M --in S] (the nearest flight of a kind / mission, e.g. airliner,arrival; --below steps the world clock so that it is M metres above its field S seconds from now), key T RIGHT UP AHEAD [AT_RIGHT AT_UP AT_AHEAD [LAG]] (the eye T s after play in the aircraft's frame, looking at a point of it, LAG s behind it), unicorn RIGHT UP AHEAD | off, play, stop, list")
        .arg(
            Arg::new("action")
                .required(true)
                .value_parser(["pick", "key", "unicorn", "play", "stop", "list"]),
        )
        .arg(
            Arg::new("args")
                .num_args(0..)
                .allow_negative_numbers(true),
        )
        .arg(
            Arg::new("below")
                .long("below")
                .value_parser(value_parser!(f64)),
        )
        .arg(
            Arg::new("in")
                .long("in")
                .value_parser(value_parser!(f64))
                .default_value("5"),
        )
}

fn numbers(args: &[String]) -> Result<Vec<f64>, String> {
    args.iter()
        .map(|a| {
            a.parse::<f64>()
                .map_err(|_| format!("'{a}' is not a number"))
        })
        .collect()
}

fn follow_action(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    let action = m.get_one::<String>("action").expect("required").as_str();
    let args: Vec<String> = m
        .get_many::<String>("args")
        .map(|v| v.cloned().collect())
        .unwrap_or_default();
    match action {
        "pick" => {
            let what = args
                .first()
                .map_or_else(|| "any".to_string(), |w| w.to_lowercase());
            let lead = *m.get_one::<f64>("in").expect("default");
            let below = m.get_one::<f64>("below").copied();
            let (flights, now, eye, _) = flights_now(world)?;
            let cands = flights.iter().filter(|f| matches_terms(f, &what));
            let picked = match below {
                // The flight that comes down to M soonest from now (or was
                // there most recently): the least step of the clock.
                Some(h) => cands
                    .filter_map(|f| {
                        let t = first_below(|t| f.state(t), f.start_s, f.end_s(), f.field_m, h)?;
                        Some((f.clone(), t - lead - now))
                    })
                    .min_by(|a, b| a.1.abs().total_cmp(&b.1.abs())),
                None => cands
                    .filter(|f| f.alive(now + lead))
                    .min_by(|a, b| {
                        let d = |f: &Flight| {
                            let p = f.state(now).pos;
                            (p[0] - eye[0]).hypot(p[2] - eye[2])
                        };
                        d(a).total_cmp(&d(b))
                    })
                    .map(|f| (f.clone(), 0.0)),
            };
            let (f, step_s) = picked
                .ok_or_else(|| format!("no {what} flying now ({} flights)", flights.len()))?;
            let ms = (step_s * 1000.0).round() as i64;
            if ms != 0 {
                crate::send(
                    world,
                    ocw_client::client::ClientCore::clock_message(ocw_sky::ClockChange::Step(ms)),
                );
            }
            let line = format!(
                "following the {} {} (flight {}){}",
                f.kind.key(),
                f.mission.key(),
                f.id,
                if ms != 0 {
                    format!(", the clock stepped {step_s:+.0} s")
                } else {
                    String::new()
                }
            );
            world.resource_mut::<AirFollow>().flight = Some(f);
            Ok(line)
        }
        "key" => {
            let v = numbers(&args)?;
            let (t, eye, target, lag) = match v[..] {
                [t, r, u, a] => (t, [r, u, a], [0.0; 3], 0.0),
                [t, r, u, a, tr, tu, ta] => (t, [r, u, a], [tr, tu, ta], 0.0),
                [t, r, u, a, tr, tu, ta, lag] => (t, [r, u, a], [tr, tu, ta], lag),
                _ => {
                    return Err(
                        "key T RIGHT UP AHEAD [AT_RIGHT AT_UP AT_AHEAD [LAG]] (s, m)".into(),
                    );
                }
            };
            let mut st = world.resource_mut::<AirFollow>();
            st.path.add(FollowKey {
                t,
                eye,
                target,
                lag,
            });
            Ok(format!("key at {t:.1} s ({} keys)", st.path.keys().len()))
        }
        "unicorn" => {
            let mut st = world.resource_mut::<AirFollow>();
            if args.first().is_some_and(|a| a == "off") {
                st.unicorn = None;
                return Ok("the unicorn flies on its own".into());
            }
            let [r, u, a] = numbers(&args)?[..] else {
                return Err("unicorn RIGHT UP AHEAD (m) | off".into());
            };
            st.unicorn = Some(Formation { offset: [r, u, a] });
            Ok(format!(
                "the unicorn flies {r:.0} m right, {u:.0} m up, {a:.0} m ahead of the aircraft"
            ))
        }
        "play" => {
            let now = crate::now(world);
            let mut st = world.resource_mut::<AirFollow>();
            if st.flight.is_none() {
                return Err("no aircraft (air.follow pick first)".into());
            }
            st.since = Some(now);
            Ok(format!(
                "playing {} keys over {:.1} s",
                st.path.keys().len(),
                st.path.duration_s()
            ))
        }
        "stop" => {
            let mut st = world.resource_mut::<AirFollow>();
            st.flight = None;
            st.since = None;
            st.unicorn = None;
            st.path.clear();
            Ok("stopped following".into())
        }
        "list" => {
            let st = world.resource::<AirFollow>();
            let mut out = vec![match &st.flight {
                Some(f) => format!(
                    "following the {} {} (flight {})",
                    f.kind.key(),
                    f.mission.key(),
                    f.id
                ),
                None => "following nothing".to_string(),
            }];
            for k in st.path.keys() {
                out.push(format!(
                    "  {:.1} s: eye {:?}, at {:?}, lag {:.1} s",
                    k.t, k.eye, k.target, k.lag
                ));
            }
            if let Some(u) = st.unicorn {
                out.push(format!("  unicorn at {:?}", u.offset));
            }
            Ok(out.join("\n"))
        }
        _ => unreachable!("clap checks the actions"),
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        follow_command(),
        follow_action,
    ));
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
                .ok_or_else(|| {
                    let mut kinds: Vec<&str> = flights.iter().map(|f| f.mission.key()).collect();
                    kinds.sort_unstable();
                    kinds.dedup();
                    format!(
                        "no {what} flying now ({} flights: {})",
                        flights.len(),
                        kinds.join(", ")
                    )
                })?;
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
    fn follow_parses_its_options_after_the_words() {
        // The positional words must not swallow --below / --in, and
        // negative offsets are numbers.
        let m = follow_command()
            .try_get_matches_from("air.follow pick airliner,arrival --below 340 --in 10".split(' '))
            .unwrap();
        assert_eq!(m.get_one::<f64>("below"), Some(&340.0));
        assert_eq!(m.get_one::<f64>("in"), Some(&10.0));
        let args: Vec<&String> = m.get_many::<String>("args").unwrap().collect();
        assert_eq!(args, ["airliner,arrival"]);
        let m = follow_command()
            .try_get_matches_from("air.follow key 36 -38 16 -70 0 0 0 6".split(' '))
            .unwrap();
        let v: Vec<String> = m.get_many::<String>("args").unwrap().cloned().collect();
        assert_eq!(
            numbers(&v).unwrap(),
            [36.0, -38.0, 16.0, -70.0, 0.0, 0.0, 0.0, 6.0]
        );
    }

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

//! The UFOs in both apps (spec 14 Phase 18, 18C *UFOs*, 18E T9; Q7):
//! each frame the sightings, scenes and the unicorn's visitor of
//! `ocw_client::ufo` from the session's air seed, the world clock and the
//! weather, seen and heard from the camera, into
//! [`ocw_render::RideView::ufo`]; the renderer draws them
//! (`ocw_render::ufo`), the audio feed plays the hum and the whoosh.
//! Everywhere (Q9 (b)): desktop, browser and the headsets. Never in a race
//! or event (the rider an entrant of one, or an organiser's traffic rule).
//!
//! Commands: `ufo` (the setting, what is on now near the camera, the crop
//! circles, the unicorn's visits), `ufo.visit` (unicorn mode: the visitor
//! comes now – scenes and videos), `ufo.next KIND [SECONDS]` (steps the
//! world clock to the next sighting of a kind – night, day, circle, cow –
//! near the camera, so it begins in SECONDS) and `ufo.spot [METRES]`
//! (unicorn mode: hover METRES from the nearest sighting, looking at it).

use std::sync::Arc;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::ufo::{PackageGround, RouteGround, SharedGround, UfoClient, UfoFrame};
use ocw_render::RideView;
use ocw_render::camera::FollowCamera;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::package::PackageWorld;
use ocw_sim::air::{heading_of, mix};
use ocw_sim::ufo::curiosity::Unicorn;
use ocw_sim::ufo::{CELL_M, SLOT_S, Sighting, SightingKind, cells_near, sighting};

use crate::traffic::TrafficSettings;

/// The client's UFOs.
#[derive(Resource, Default)]
pub struct UfoState(pub UfoClient);

pub struct UfoPlugin;

impl Plugin for UfoPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<UfoState>()
            .add_systems(Update, update_ufos.after(crate::ViewExtrasSet));
        register_commands(app);
    }
}

/// How to build the UFOs' ground of a world.
type BuildGround = Box<dyn FnOnce() -> SharedGround>;

/// The world's key, its origin and how to build the UFOs' ground.
fn world_ground(world: &World) -> Option<(u64, (f64, f64), BuildGround)> {
    if let Some(p) = world.get_resource::<PackageWorld>() {
        let home = p.home().clone();
        let key = home.name.bytes().fold(
            mix(home.origin.0.to_bits() ^ home.origin.1.to_bits().rotate_left(17)),
            |h, b| mix(h ^ u64::from(b)),
        );
        let origin = home.origin;
        return Some((key, origin, Box::new(move || Arc::new(PackageGround(home)))));
    }
    let origin = crate::client(world)?
        .geo_origin()
        .unwrap_or(ocw_sky::zone::DEFAULT_PLACE);
    let view = world.get_resource::<RideView>()?;
    let points: Vec<[f64; 2]> = view.route.points().iter().map(|p| [p[0], p[2]]).collect();
    let height = world
        .get_resource::<ocw_render::Ground>()
        .map(|g| g.0.clone());
    let key = mix(origin.0.to_bits() ^ origin.1.to_bits().rotate_left(17) ^ points.len() as u64);
    Some((
        key,
        origin,
        Box::new(move || {
            Arc::new(RouteGround {
                height: height.unwrap_or_else(|| Arc::new(|_, _| Some(0.0))),
                points,
            })
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

/// Whether the rider is in a race or event (no UFOs then, 18C).
fn in_event(world: &World) -> bool {
    crate::client(world).is_some_and(|c| {
        c.traffic().by_event()
            || c.event_info().is_some_and(|i| {
                i.phase != ocw_sim::EventPhase::Finished
                    && i.standings.iter().any(|s| Some(s.id) == c.rider_id())
            })
    })
}

/// The frame's inputs (without the weather, borrowed from the view).
fn frame_inputs(world: &mut World) -> Option<(UfoFrame<'static>, u64, BuildGround)> {
    let settings = world
        .get_resource::<TrafficSettings>()
        .copied()
        .unwrap_or_default();
    let seed = crate::client(world)
        .filter(|c| c.rider_id().is_some())?
        .traffic()
        .air_seed();
    let in_event = in_event(world);
    let (eye, forward) = eye(world)?;
    let (key, origin, ground) = world_ground(world)?;
    let view = world.get_resource::<RideView>()?;
    let clock_ms = view.sky.as_ref().map(|s| s.sky.instant);
    let unicorn = view.flight.as_ref().map(|f| Unicorn {
        pos: f.pos,
        vel: [f.velocity[0], f.speed_mps * f.pitch.sin(), f.velocity[1]],
    });
    Some((
        UfoFrame {
            choice: settings.view,
            in_event,
            seed,
            clock_ms,
            weather: None,
            origin,
            eye,
            forward,
            unicorn,
        },
        key,
        ground,
    ))
}

fn update_ufos(world: &mut World) {
    let Some((f, key, ground)) = frame_inputs(world) else {
        if let Some(mut v) = world.get_resource_mut::<RideView>()
            && v.ufo.is_some()
        {
            v.ufo = None;
        }
        return;
    };
    let level = f.choice.ufos_shown(f.in_event);
    if level != ocw_core::traffic::Ufos::Off {
        world.resource_mut::<UfoState>().0.set_ground(key, ground);
    }
    let weather = world
        .get_resource::<RideView>()
        .and_then(|v| v.weather.as_ref().map(|w| w.state));
    let frame = UfoFrame {
        weather: weather.as_ref(),
        ..f
    };
    let ufo = world.resource_mut::<UfoState>().0.frame(&frame);
    if let Some(mut v) = world.get_resource_mut::<RideView>() {
        v.ufo = ufo;
    }
}

/// The sightings near the camera from `from_s` on (the commands): up to
/// `hours` ahead.
fn upcoming(world: &mut World, hours: f64) -> Result<(Vec<Sighting>, f64, [f64; 3]), String> {
    let (f, key, ground) = frame_inputs(world).ok_or("not riding")?;
    let level = f.choice.ufos_shown(f.in_event);
    if level == ocw_core::traffic::Ufos::Off {
        return Err(if f.in_event {
            "no UFOs in races and events".into()
        } else {
            "UFOs are off (traffic.ufo off, or traffic.enabled off)".into()
        });
    }
    let weather = world
        .get_resource::<RideView>()
        .and_then(|v| v.weather.as_ref().map(|w| w.state))
        .unwrap_or_default();
    let frame = UfoFrame {
        weather: Some(&weather),
        ..f
    };
    let fallback = weather;
    let inp = UfoClient::inputs(&frame, &fallback).ok_or("the world clock is not known yet")?;
    let mut st = world.resource_mut::<UfoState>();
    st.0.set_ground(key, ground);
    let g = st.0.ground().cloned().ok_or("no ground")?;
    let now = inp.now_s;
    let k0 = (now / SLOT_S).floor() as i64 - 1;
    let k1 = ((now + hours * 3600.0) / SLOT_S).ceil() as i64;
    let mut out = Vec::new();
    for cell in cells_near(f.eye) {
        // Only the cells within half a view of the camera: seen from here.
        let c = [
            (f64::from(cell.0) + 0.5) * CELL_M,
            (f64::from(cell.1) + 0.5) * CELL_M,
        ];
        if (c[0] - f.eye[0]).hypot(c[1] - f.eye[2]) > 4000.0 {
            continue;
        }
        for k in k0..=k1 {
            if let Some(s) = sighting(g.as_ref(), &inp, cell, k)
                && s.end_s > now
            {
                out.push(s);
            }
        }
    }
    out.sort_by(|a, b| a.start_s.total_cmp(&b.start_s));
    Ok((out, now, f.eye))
}

fn summary(world: &mut World) -> Result<String, String> {
    let s = world
        .get_resource::<TrafficSettings>()
        .copied()
        .unwrap_or_default();
    let mut out = vec![format!("UFOs: {}", s.view.ufo.key())];
    let (list, now, eye) = upcoming(world, 6.0)?;
    let d = |p: [f64; 3]| (p[0] - eye[0]).hypot(p[2] - eye[2]);
    let on: Vec<String> = list
        .iter()
        .filter(|s| s.alive(now))
        .map(|s| format!("{} {:.1} km away", s.kind.key(), d(s.pos(now)) / 1000.0))
        .collect();
    out.push(format!(
        "now: {}",
        if on.is_empty() {
            "none".into()
        } else {
            on.join(", ")
        }
    ));
    for s in list.iter().filter(|s| s.start_s > now).take(5) {
        out.push(format!(
            "  next: {} in {:.0} min, {:.1} km away",
            s.kind.key(),
            (s.start_s - now) / 60.0,
            d(s.pos(s.start_s + 5.0)) / 1000.0
        ));
    }
    let st = world.resource::<UfoState>();
    let c = st.0.curiosity();
    out.push(format!(
        "crop circles seen this session: {}; the unicorn's visits: {} ({} chased away){}",
        world
            .get_resource::<RideView>()
            .and_then(|v| v.ufo.as_ref())
            .map_or(0, |u| u.circles.len()),
        c.visits,
        c.chased,
        c.phase()
            .map(|p| format!(", now: {p:?}"))
            .unwrap_or_default()
    ));
    Ok(out.join("\n"))
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ufo").about(
            "The UFOs (traffic.ufo): what is on near the camera, the next sightings, the crop circles, the unicorn's visits",
        ),
        |world, _| summary(world),
    ));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ufo.visit")
            .about("Unicorn mode: a curious UFO comes to the unicorn now (scenes and videos; traffic.ufo rare or often)"),
        |world, _| {
            let flying = world
                .get_resource::<RideView>()
                .is_some_and(|v| v.flight.is_some());
            if !flying {
                return Err("not flying (god on first)".into());
            }
            world.resource_mut::<UfoState>().0.visit_now();
            Ok("a UFO is coming to look at the unicorn".into())
        },
    ));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ufo.next")
            .about("Step the world clock to the next UFO sighting of a kind near the camera (night, day, circle, cow, any), so that it begins in SECONDS")
            .arg(Arg::new("kind").default_value("any"))
            .arg(Arg::new("seconds").value_parser(value_parser!(f64)).default_value("5")),
        |world, m| {
            let what = m.get_one::<String>("kind").expect("default").to_lowercase();
            let lead = *m.get_one::<f64>("seconds").expect("default");
            let kind = if what == "any" {
                None
            } else {
                Some(SightingKind::parse(&what).ok_or("night, day, circle, cow or any")?)
            };
            // Up to two days ahead.
            let (list, now, eye) = upcoming(world, 48.0)?;
            let s = list
                .iter()
                .find(|s| s.start_s - lead > now && kind.is_none_or(|k| s.kind == k))
                .ok_or_else(|| format!("no {what} sighting near here in the next two days (clear weather needed)"))?;
            let step = s.start_s - lead - now;
            let ms = (step * 1000.0).round() as i64;
            crate::send(
                world,
                ocw_client::client::ClientCore::clock_message(ocw_sky::ClockChange::Step(ms)),
            );
            let p = s.pos(s.start_s + 5.0);
            Ok(format!(
                "a {} sighting in {lead:.0} s at ({:.0}, {:.0}, {:.0}), {:.1} km away – the clock stepped {:+.0} min",
                s.kind.key(),
                p[0],
                p[1],
                p[2],
                (p[0] - eye[0]).hypot(p[2] - eye[2]) / 1000.0,
                step / 60.0
            ))
        },
    ));
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ufo.spot")
            .about("Unicorn mode: hover METRES from the UFO sighting on now near the camera (else the next one), UP metres over its ground, from ANGLE degrees (0 = north), looking at it")
            .arg(Arg::new("metres").value_parser(value_parser!(f64)).default_value("120"))
            .arg(Arg::new("up").value_parser(value_parser!(f64)).default_value("25").allow_negative_numbers(true))
            .arg(Arg::new("angle").value_parser(value_parser!(f64)).default_value("200").allow_negative_numbers(true)),
        |world, m| {
            let dist = *m.get_one::<f64>("metres").expect("default");
            let up = *m.get_one::<f64>("up").expect("default");
            let angle = m.get_one::<f64>("angle").expect("default").to_radians();
            let (list, now, _) = upcoming(world, 1.0)?;
            // The one on now, else the next (`ufo.next` stepped the clock to it).
            let s = list
                .iter()
                .find(|s| s.kind != SightingKind::Day && s.end_s > now)
                .ok_or("no sighting near here within the hour")?;
            // Where to look and where from: behind the cow, over the circle,
            // else at the saucer where it hovers first.
            let (spot, target) = if let Some(c) = s.cow {
                // Behind the cow's field, looking along its way: the lift
                // close by, the carry going away, the setting down beyond.
                let d = [c.b[0] - c.a[0], c.b[2] - c.a[2]];
                let l = d[0].hypot(d[1]).max(1.0);
                let back = [-d[0] / l, -d[1] / l];
                (
                    [c.a[0] + back[0] * dist, c.a[1] + up, c.a[2] + back[1] * dist],
                    [c.a[0], c.a[1] + 12.0, c.a[2]],
                )
            } else {
                let (at, target) = match &s.circle {
                    Some(c) => (
                        [c.centre[0], c.ground_m, c.centre[1]],
                        [c.centre[0], c.ground_m, c.centre[1]],
                    ),
                    None => {
                        let p = s.pos(s.start_s + 5.0);
                        ([p[0], p[1] - 60.0, p[2]], p)
                    }
                };
                (
                    [
                        at[0] + dist * angle.sin(),
                        at[1] + up,
                        at[2] - dist * angle.cos(),
                    ],
                    target,
                )
            };
            let look = [target[0] - spot[0], target[1] - spot[1], target[2] - spot[2]];
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
                "unicorn at ({:.0}, {:.0}, {:.0}) m beside the {} sighting",
                spot[0],
                spot[1],
                spot[2],
                s.kind.key()
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
            .add_plugins(UfoPlugin);
        app.update();
        let w = app.world_mut();
        assert!(summary(w).is_err());
        assert!(w.resource::<UfoState>().0.ground().is_none());
    }
}

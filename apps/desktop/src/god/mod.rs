//! God mode (spec §14 Phase 13): the rider leaves the road and flies
//! through the world as a unicorn.
//!
//! [`state`] is the Bevy-free client side (state machine, flight, poses,
//! glides, the others' unicorns; inside [`ClientCore`]), [`input`] the key
//! and gamepad mapping; this module is the Bevy glue: the keys (G, Shift +
//! G, C, the gamepad chord), the flight each frame (terrain, buildings and
//! the world set's limits from the renderer's resources), the poses to the
//! server, the pointer capture (Esc releases it first, then opens the
//! menu), the [`RideView`]'s flight and unicorns, and the console commands
//! `god`, `god.pose`, `god.cruise`, `god.view`, `god.trail`, `god.invert`.

pub mod input;
pub mod state;

use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use clap::{Arg, Command, value_parser};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_sim::god::{Bounds, FlightInput};

use self::state::Phase;
use crate::client::ClientCore;
use crate::connection::{ClientState, Link};
use input::{FlightKeys, GodKey, PadFlight};

/// Open question Q6 (spec §14 Phase 13H): the name in the menu and the
/// HUD. The proposal: "God mode" (the console command stays `god`).
pub const MENU_NAME: &str = "God mode";

/// The keys line of the flight HUD.
pub const KEYS: &str = "arrows: fly, +/-: up/down, mouse: look (Esc frees the pointer), Shift: boost, Ctrl: precise, wheel: speed, G: land, Shift+G: fly back, C: first person | ` or F12: console";

pub struct GodPlugin;

impl Plugin for GodPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FlightPointer>()
            .init_resource::<FlightSettings>()
            .init_resource::<Cruise>()
            .add_systems(
                Update,
                (
                    god_keys,
                    fly,
                    grab_pointer,
                    update_view.after(crate::connection::update_view),
                )
                    .chain(),
            );
        register_commands(app);
    }
}

/// Whether the pointer is captured for looking around (spec §14 Phase
/// 13A): on when the flight begins, released by Esc (the menu's first
/// Esc in god mode), captured again by a click.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlightPointer {
    pub captured: bool,
}

/// Flight options: invert Y (Settings → Controls).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct FlightSettings {
    pub invert_y: bool,
}

/// A steady flight input from the console (`god.cruise`, for scenes and
/// videos): forward, climb, turn rate (rad/s).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct Cruise(pub Option<(f64, f64, f64)>);

/// Run condition: not flying (the riding keys do nothing in god mode).
pub fn not_flying(client: Option<Res<ClientState>>) -> bool {
    client.is_none_or(|c| !c.0.god().active())
}

/// What a god-mode request asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    On,
    Off,
    Toggle,
    Back,
}

/// Asks for god mode on the client: the message to send, if any, or why
/// not. Leaving first looks up the landing spot below the unicorn.
pub fn ask(c: &mut ClientCore, what: Ask, now: f64) -> Result<Option<Vec<u8>>, String> {
    let phase = c.god().phase();
    let flying = c.god().flying();
    match what {
        Ask::On | Ask::Toggle if phase == Phase::Riding => {
            if c.rider_id().is_none() {
                return Err("not joined yet".into());
            }
            if !c.may_use_god() {
                return Err("god mode: only the host of this shared ride may use it".into());
            }
            Ok(c.god_mut().request_enter(now))
        }
        Ask::Off | Ask::Toggle | Ask::Back if flying => {
            let p = c.god().flight().pos;
            let spot = c.landing_spot(p[0], p[2]);
            c.god_mut().set_landing(spot);
            Ok(c.god_mut().request_leave(what == Ask::Back, now))
        }
        Ask::On => Err("already in god mode".into()),
        _ if matches!(phase, Phase::Asking(_) | Phase::Leaving { .. }) => Ok(None),
        _ => Err("not in god mode".into()),
    }
}

fn send_ask(world: &mut World, what: Ask) -> Result<String, String> {
    let now = world.resource::<Time>().elapsed_secs_f64();
    let msg = ask(&mut world.resource_mut::<ClientState>().0, what, now)?;
    if let Some(m) = msg {
        world.resource::<Link>().send_bytes(m);
    }
    // The server answers (printed when it does).
    Ok(String::new())
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn god_keys(
    keys: Res<ButtonInput<KeyCode>>,
    pads: Query<&Gamepad>,
    time: Res<Time>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
) {
    if !ocw_render::menu::ride_input(context) {
        return;
    }
    let shift = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    let select = pads.iter().any(|p| p.pressed(GamepadButton::Select));
    let pressed = |b| pads.iter().any(|p| p.just_pressed(b));
    let key = input::god_key(
        keys.just_pressed(KeyCode::KeyG),
        shift,
        select,
        pressed(GamepadButton::North),
        pressed(GamepadButton::West),
    );
    let now = time.elapsed_secs_f64();
    let what = match key {
        GodKey::None => None,
        GodKey::Toggle => Some(Ask::Toggle),
        GodKey::FlyBack => client.0.god().flying().then_some(Ask::Back),
    };
    if let Some(what) = what {
        match ask(&mut client.0, what, now) {
            Ok(Some(m)) => link.send_bytes(m),
            Ok(None) => {}
            Err(e) => console.print(e),
        }
    }
    if keys.just_pressed(KeyCode::KeyC) && client.0.god().active() {
        let fp = client.0.god_mut().toggle_first_person();
        info!(
            "god mode: {} view",
            if fp { "first-person" } else { "flight" }
        );
    }
}

/// The keys of the flight.
fn flight_keys(keys: &ButtonInput<KeyCode>) -> FlightKeys {
    let any = |k: &[KeyCode]| keys.any_pressed(k.iter().copied());
    let wasd = input::WASD_FLIES;
    FlightKeys {
        up: any(&[KeyCode::ArrowUp]) || (wasd && any(&[KeyCode::KeyW])),
        down: any(&[KeyCode::ArrowDown]) || (wasd && any(&[KeyCode::KeyS])),
        left: any(&[KeyCode::ArrowLeft]) || (wasd && any(&[KeyCode::KeyA])),
        right: any(&[KeyCode::ArrowRight]) || (wasd && any(&[KeyCode::KeyD])),
        plus: any(&[KeyCode::Equal, KeyCode::NumpadAdd]),
        minus: any(&[KeyCode::Minus, KeyCode::NumpadSubtract]),
        shift: any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        ctrl: any(&[KeyCode::ControlLeft, KeyCode::ControlRight]),
    }
}

/// A gamepad's flight controls; the triggers are read above their resting
/// value ([`crate::input::trigger_pull`]: pads differ in what a released
/// trigger reads).
fn pad_flight(p: &Gamepad, rest: &mut [Option<f32>; 2]) -> PadFlight {
    let axis = |a| p.get(a).unwrap_or(0.0);
    let button = |b| p.get(b).unwrap_or(0.0);
    PadFlight {
        left_x: axis(GamepadAxis::LeftStickX),
        left_y: axis(GamepadAxis::LeftStickY),
        right_x: axis(GamepadAxis::RightStickX),
        right_y: axis(GamepadAxis::RightStickY),
        right_trigger: crate::input::trigger_pull(
            button(GamepadButton::RightTrigger2),
            &mut rest[0],
        ),
        left_trigger: crate::input::trigger_pull(button(GamepadButton::LeftTrigger2), &mut rest[1]),
        south: p.pressed(GamepadButton::South),
    }
}

/// The world set's area, else the route's (grown by 3 km).
fn flight_bounds(
    packages: Option<&ocw_render::package::PackageWorld>,
    view: &RideView,
) -> Option<Bounds> {
    if let Some(w) = packages {
        let (min, max) = w.set.bounds();
        if min[0] <= max[0] && min[1] <= max[1] {
            return Some(Bounds { min, max });
        }
    }
    let (min, max) = ocw_render::route_bounds(&view.route);
    (min[0] <= max[0]).then(|| Bounds { min, max }.grown(3_000.0))
}

/// What the flight stays above and within: terrain, buildings, the world
/// set, the ride view.
type WorldParams<'w> = (
    Option<Res<'w, ocw_render::Ground>>,
    Res<'w, ocw_render::camera::CameraObstacles>,
    Option<Res<'w, ocw_render::package::PackageWorld>>,
    Res<'w, RideView>,
);

/// The resting values of each gamepad's triggers.
type TriggerRest = std::collections::HashMap<Entity, [Option<f32>; 2]>;

/// The flight, each frame: input (only in the ride's input context),
/// look, wheel, the step over the terrain and the buildings, the landing
/// spot (4 times a second), the poses to the server.
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    pads: Query<(Entity, &Gamepad)>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    (pointer, settings, cruise): (Res<FlightPointer>, Res<FlightSettings>, Res<Cruise>),
    (ground, obstacles, packages, view): WorldParams<'_>,
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
    (mut last_spot, mut trigger_rest): (Local<f64>, Local<TriggerRest>),
) {
    let c = &mut client.0;
    if let Some(e) = c.take_god_notice() {
        console.print(match e {
            self::state::GodEvent::Landed => "god mode ended by the server".into(),
            other => format!("{other:?}"),
        });
    }
    if !c.god().active() && !matches!(c.god().phase(), Phase::Asking(_)) {
        return;
    }
    let now = time.elapsed_secs_f64();
    let dt = time.delta_secs_f64();
    let riding_keys = ocw_render::menu::ride_input(context);
    trigger_rest.retain(|e, _| pads.contains(*e));
    let pad_list: Vec<PadFlight> = if riding_keys {
        pads.iter()
            .map(|(e, p)| pad_flight(p, trigger_rest.entry(e).or_default()))
            .collect()
    } else {
        Vec::new()
    };
    let mut flight = if riding_keys {
        input::flight_input(&flight_keys(&keys), &pad_list)
    } else {
        FlightInput::default()
    };
    let mouse = if riding_keys && (pointer.captured || buttons.pressed(MouseButton::Left)) {
        (motion.delta.x, motion.delta.y)
    } else {
        (0.0, 0.0)
    };
    let mut look = input::look_delta(mouse, &pad_list, settings.invert_y, dt);
    if let Some((fw, climb, turn)) = cruise.0
        && flight == FlightInput::default()
    {
        flight = FlightInput {
            forward: fw,
            climb,
            ..Default::default()
        };
        look.0 += turn * dt;
    }
    let wheel = if riding_keys {
        f64::from(match scroll.unit {
            MouseScrollUnit::Line => scroll.delta.y,
            MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
        })
    } else {
        0.0
    };
    let surface =
        |x: f64, z: f64| ocw_render::camera::solid_height(ground.as_deref(), &obstacles, x, z);
    let bounds = flight_bounds(packages.as_deref(), &view);
    if let Some(e) = c
        .god_mut()
        .step(&flight, look, wheel, dt, now, &surface, bounds)
        && let self::state::GodEvent::Refused(why) = e
    {
        console.print(format!("god mode: {why}"));
    }
    if c.god().flying() && now - *last_spot >= 0.25 {
        *last_spot = now;
        let p = c.god().flight().pos;
        let spot = c.landing_spot(p[0], p[2]);
        c.god_mut().set_landing(spot);
    }
    if let Some(m) = c.god_mut().pose_message(now) {
        link.send_bytes(m);
    }
}

/// Captures the pointer when the flight begins and frees it when it ends
/// (or Esc freed it, [`crate::menu`]); a click captures it again.
fn grab_pointer(
    client: Res<ClientState>,
    buttons: Res<ButtonInput<MouseButton>>,
    context: Option<Res<ocw_render::menu::InputContext>>,
    mut pointer: ResMut<FlightPointer>,
    mut was_active: Local<bool>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let active = client.0.god().flying();
    if active && !*was_active {
        pointer.captured = true;
    } else if !active && *was_active {
        pointer.captured = false;
    } else if active
        && !pointer.captured
        && buttons.just_pressed(MouseButton::Left)
        && ocw_render::menu::ride_input(context)
    {
        pointer.captured = true;
    }
    *was_active = active;
    let (visible, grab) = if pointer.captured {
        (false, CursorGrabMode::Locked)
    } else {
        (true, CursorGrabMode::None)
    };
    for mut c in &mut windows {
        if c.visible != visible || c.grab_mode != grab {
            c.visible = visible;
            c.grab_mode = grab;
        }
    }
}

/// The flight and the unicorns for the renderer.
fn update_view(time: Res<Time>, client: Res<ClientState>, mut view: ResMut<RideView>) {
    let now = time.elapsed_secs_f64();
    let c = &client.0;
    view.unicorns = c.unicorns(now);
    let g = c.god();
    view.flight = g.unicorn(now).map(|u| {
        let f = g.flight();
        ocw_render::FlightView {
            pos: u.pose.pos,
            yaw: if u.leaving { u.pose.yaw } else { f.yaw },
            pitch: if u.leaving {
                f.pitch.min(-0.15)
            } else {
                f.pitch
            },
            speed_mps: u.pose.speed,
            height_m: g.height_m(),
            scale: f.scale,
            first_person: g.first_person(),
            landing: g.landing(),
            leaving: u.leaving,
            keys: KEYS.into(),
        }
    });
}

fn on_off(m: &clap::ArgMatches) -> bool {
    m.get_one::<String>("state").is_none_or(|s| s == "on")
}

fn register_commands(app: &mut App) {
    let state = || {
        Arg::new("state")
            .value_parser(["on", "off"])
            .default_value("on")
    };
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god")
            .about("God mode: fly over the world as a unicorn (G; Shift+G flies back)")
            .arg(
                Arg::new("action")
                    .value_parser(["on", "off", "toggle", "back", "status"])
                    .default_value("toggle"),
            ),
        |world, m| {
            let what = match m.get_one::<String>("action").map(String::as_str) {
                Some("on") => Ask::On,
                Some("off") => Ask::Off,
                Some("back") => Ask::Back,
                Some("status") => {
                    let c = &world.resource::<ClientState>().0;
                    let g = c.god();
                    let f = g.flight();
                    return Ok(match g.phase() {
                        Phase::Riding => format!(
                            "riding ({})",
                            if c.may_use_god() {
                                "G enters god mode"
                            } else {
                                "god mode is not allowed here"
                            }
                        ),
                        Phase::Asking(_) => "asking the server ...".into(),
                        Phase::Flying { .. } => format!(
                            "flying at x {:.0} z {:.0}, {:.0} m ({:.0} m above ground), {:.0} m/s, scale x{:.2}{}",
                            f.pos[0],
                            f.pos[2],
                            f.pos[1],
                            g.height_m(),
                            f.speed(),
                            f.scale,
                            g.landing()
                                .map(|l| format!(", lands at x {:.0} z {:.0}", l[0], l[2]))
                                .unwrap_or_default()
                        ),
                        Phase::Leaving { .. } => "landing ...".into(),
                    });
                }
                _ => Ask::Toggle,
            };
            send_ask(world, what)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("god.pose")
            .about("God mode: move the unicorn to x z (local metres), HEIGHT m above the ground, looking along YAW (deg, 0 = north, 90 = east) and PITCH (deg, up)")
            .arg(Arg::new("x").required(true).allow_negative_numbers(true).value_parser(value_parser!(f64)))
            .arg(Arg::new("z").required(true).allow_negative_numbers(true).value_parser(value_parser!(f64)))
            .arg(Arg::new("height").required(true).value_parser(value_parser!(f64)))
            .arg(Arg::new("yaw").allow_negative_numbers(true).value_parser(value_parser!(f64)))
            .arg(Arg::new("pitch").allow_negative_numbers(true).value_parser(value_parser!(f64))),
        |world, m| {
            let x = *m.get_one::<f64>("x").expect("required");
            let z = *m.get_one::<f64>("z").expect("required");
            let h = *m.get_one::<f64>("height").expect("required");
            // Compass heading → the flight's yaw (counter-clockwise from north).
            let yaw = m.get_one::<f64>("yaw").map(|d| -d.to_radians());
            let pitch = m.get_one::<f64>("pitch").map(|d| d.to_radians());
            let ground = world
                .get_resource::<ocw_render::Ground>()
                .and_then(|g| (g.0)(x, z))
                .unwrap_or(0.0);
            let y = ground + h.max(ocw_sim::god::CLEARANCE_M);
            let moved = world
                .resource_mut::<ClientState>()
                .0
                .god_mut()
                .teleport([x, y, z], yaw, pitch);
            if moved {
                Ok(format!("unicorn at x {x:.0} z {z:.0}, {y:.0} m"))
            } else {
                Err("not flying (god on first)".into())
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("god.cruise")
            .about("God mode: fly by itself (scenes, videos) – FORWARD and CLIMB (-1 … 1), TURN (deg/s, positive = left); 'god.cruise 0' stops")
            .arg(Arg::new("forward").required(true).allow_negative_numbers(true).value_parser(value_parser!(f64)))
            .arg(Arg::new("climb").allow_negative_numbers(true).value_parser(value_parser!(f64)))
            .arg(Arg::new("turn").allow_negative_numbers(true).value_parser(value_parser!(f64))),
        |world, m| {
            let fw = m.get_one::<f64>("forward").copied().unwrap_or(0.0).clamp(-1.0, 1.0);
            let climb = m.get_one::<f64>("climb").copied().unwrap_or(0.0).clamp(-1.0, 1.0);
            let turn = m.get_one::<f64>("turn").copied().unwrap_or(0.0).to_radians();
            let on = fw != 0.0 || climb != 0.0 || turn != 0.0;
            world.resource_mut::<Cruise>().0 = on.then_some((fw, climb, turn));
            Ok(if on { "cruising".into() } else { "cruise off".into() })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.view")
            .about("God mode: the flight camera behind the unicorn, or first-person from its head (C)")
            .arg(Arg::new("view").value_parser(["flight", "first"]).required(true)),
        |world, m| {
            let first = m.get_one::<String>("view").is_some_and(|v| v == "first");
            let mut c = world.resource_mut::<ClientState>();
            if c.0.god().first_person() != first {
                c.0.god_mut().toggle_first_person();
            }
            Ok(format!("god mode view: {}", if first { "first-person" } else { "flight" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.trail")
            .about("God mode: the rainbow trail behind the unicorns (Settings → Display → Effects)")
            .arg(state()),
        |world, m| {
            let on = on_off(m);
            world.insert_resource(ocw_render::unicorn::UnicornTrail(on));
            Ok(format!("unicorn trail {}", if on { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.invert")
            .about("God mode: invert the mouse's up / down look (Settings → Controls)")
            .arg(state()),
        |world, m| {
            let on = on_off(m);
            world.resource_mut::<FlightSettings>().invert_y = on;
            Ok(format!("invert Y {}", if on { "on" } else { "off" }))
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;

    use super::*;

    fn app() -> App {
        let argv = ["ocw-desktop", "--bots", "0", "--power", "150", "--virtual"];
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(crate::Args::parse_from(argv))
            .insert_resource(RideView::new(Route::test_loop()))
            .insert_resource(crate::connection::Ghosts(None))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<ocw_render::camera::CameraObstacles>()
            .add_plugins((
                crate::connection::ConnectionPlugin {
                    route: Route::test_loop(),
                },
                crate::devices::DevicesPlugin,
                GodPlugin,
            ));
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().to_vec()
    }

    fn update_until(app: &mut App, done: impl Fn(&ClientCore) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4) {
            app.update();
            if done(&app.world().resource::<ClientState>().0) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// G1 end to end on the local server (spec §14 Phase 13F): enter, the
    /// view and the pointer, fly, land on the route, standing.
    #[test]
    fn god_toggles_flies_and_lands_on_the_local_server() {
        let mut app = app();
        // Dev commands (god.cruise): the local server's session.
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        let phase = |app: &App| app.world().resource::<ClientState>().0.god().phase();
        assert!(update_until(&mut app, |c| c.rider_id().is_some()));
        let out = run(&mut app, "god on");
        assert!(out.iter().all(|l| !l.starts_with("error")), "{out:?}");
        assert!(
            update_until(&mut app, |c| c.god().flying()),
            "{:?}",
            phase(&app)
        );
        app.update();
        assert!(
            app.world().resource::<FlightPointer>().captured,
            "pointer captured"
        );
        let view = app.world().resource::<RideView>();
        assert!(view.flight.is_some());
        assert_eq!(view.unicorns.len(), 1);
        assert!(view.unicorns[0].is_local);
        // The landing ring is on the route below.
        assert!(update_until(&mut app, |c| c.god().landing().is_some()));
        // Cruise forward (dev command on the local server), then land.
        let out = run(&mut app, "god.cruise 1");
        assert!(out.iter().any(|l| l.contains("cruising")), "{out:?}");
        let start = app.world().resource::<ClientState>().0.god().flight().pos;
        assert!(update_until(&mut app, |c| {
            let p = c.god().flight().pos;
            (p[0] - start[0]).hypot(p[2] - start[2]) > 20.0
        }));
        run(&mut app, "god.cruise 0");
        run(&mut app, "god off");
        assert!(
            update_until(&mut app, |c| c.god().phase() == Phase::Riding),
            "{:?}",
            phase(&app)
        );
        app.update();
        assert!(!app.world().resource::<FlightPointer>().captured);
        assert!(app.world().resource::<RideView>().flight.is_none());
        let out = run(&mut app, "god status");
        assert!(out.iter().any(|l| l.starts_with("riding")), "{out:?}");
        let s = app.world().resource::<ClientState>().0.state();
        assert!(s.speed < 3.0, "landed standing: {s:?}");
    }
}

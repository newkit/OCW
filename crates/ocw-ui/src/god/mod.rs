//! God mode – "Unicorn mode" in the UI (spec §14 Phase 13): the rider
//! leaves the road and flies through the world as a unicorn. Shared by the
//! desktop and the browser app since Phase 13 G5 (v3.37; the way Phase 10
//! B6 shared the menus): both apps add [`GodPlugin`] and reach their ride
//! through [`crate::RideAccess`].
//!
//! [`state`] is the Bevy-free client side (state machine, flight, poses,
//! glides, the others' unicorns; inside `ClientCore`), [`input`] the key
//! and gamepad mapping, [`touch`] the touch controls; this module is the
//! Bevy glue: the keys (G, Shift + G, C, the gamepad chord), the flight
//! each frame (terrain, buildings and the world set's limits from the
//! renderer's resources), the poses to the server, the pointer capture
//! (Esc releases it first, then opens the menu; the browser's Pointer Lock
//! through [`PointerLockWatch`]), the [`RideView`]'s flight and unicorns,
//! and the console commands `god`, `god.pose`, `god.cruise`, `god.view`,
//! `god.trail`, `god.invert`, `god.flight`, `ride.riders`.
//! [`inspect`] is the inspection (G3: the panel and the crosshair on I,
//! the pins on P, `god.pins`).
//!
//! The server decides who may fly (spec §14 Phase 13A / G4: anyone on the
//! local servers, the host in a shared ride, nobody in an organizer's
//! session) – on the desktop and in the browser alike.

pub mod input;
pub mod inspect;
pub use ocw_client::god_state as state;
pub mod touch;

use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use clap::{Arg, Command, value_parser};
use ocw_client::client::ClientCore;
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::InputContext;
use ocw_sim::god::{Bounds, FlightInput, FlightModel};

use self::state::Phase;
use input::{FlightKeys, GodKey, KeyRole, LayoutKeys, PadFlight};

/// The name in the menu and the HUD (spec §14 Phase 13H, Q6: decided;
/// the console command stays `god`).
pub const MENU_NAME: &str = "Unicorn mode";

/// The keys line of the flight HUD.
pub const KEYS: &str = "Up/Down: faster/slower, Left/Right: bank, +/-: climb/descend, mouse: look around (L: ahead; Esc frees the pointer), Shift: boost, Ctrl: precise, wheel: speed, G: land, Shift+G: fly back, C: first person, I: inspection, P: pin | ` or F12: console";

/// The keys line of the hover model (`controls.god_flight hover`).
pub const KEYS_HOVER: &str = "arrows: fly, +/-: up/down, mouse: look (Esc frees the pointer), Shift: boost, Ctrl: precise, wheel: speed, G: land, Shift+G: fly back, C: first person, I: inspection, P: pin | ` or F12: console";

pub struct GodPlugin;

impl Plugin for GodPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.add_message::<KeyboardInput>()
            .init_resource::<FlightPointer>()
            .init_resource::<FlightSettings>()
            .init_resource::<Cruise>()
            .init_resource::<FlightTouch>()
            .init_resource::<FlightLayout>()
            .init_resource::<FlightFrame>()
            .init_resource::<GodStatus>()
            .init_resource::<LandingTimer>()
            .init_resource::<inspect::Inspector>()
            // After the app's view (the flight and the unicorns go into
            // it), before the menu reads it.
            .add_systems(
                Update,
                (
                    layout_keys,
                    god_keys,
                    gather_flight,
                    fly,
                    inspect::inspect,
                    grab_pointer,
                    update_view,
                    inspect::update_view,
                )
                    .chain()
                    .in_set(crate::ViewExtrasSet),
            );
        register_commands(app);
        inspect::register_commands(app);
    }
}

/// Whether the pointer is captured for looking around (spec §14 Phase
/// 13A): on when the flight begins, released by Esc (the menu's first
/// Esc in god mode; in the browser also when the page loses its Pointer
/// Lock), captured again by a click.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FlightPointer {
    pub captured: bool,
}

/// Flight options (Settings → Controls): invert Y, plane or hover.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct FlightSettings {
    pub invert_y: bool,
    pub model: FlightModel,
}

/// The + / − keys of the keyboard's layout ([`LayoutKeys`]).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct FlightLayout(pub LayoutKeys);

/// Where god mode is this frame (refreshed by the god-mode systems; for
/// the systems that do not reach the client).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GodStatus {
    /// Flying, or landing / taking off (`GodCore::active`).
    pub active: bool,
    /// Flying (the unicorn is steered).
    pub flying: bool,
    /// Asked the server to enter.
    pub asking: bool,
}

impl GodStatus {
    fn of(c: &ClientCore) -> Self {
        let g = c.god();
        Self {
            active: g.active(),
            flying: g.flying(),
            asking: matches!(g.phase(), Phase::Asking(_)),
        }
    }
}

/// When the landing spot below the unicorn was looked up last (s).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
struct LandingTimer(f64);

/// The frame's flight input from keys, pads, mouse and touch
/// ([`gather_flight`]), applied to the client by [`fly`].
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct FlightFrame {
    pub input: FlightInput,
    /// The view's turn (yaw, pitch; rad).
    pub look: (f64, f64),
    /// Wheel steps of the speed scale.
    pub wheel: f64,
    /// L or the middle button: look ahead again.
    pub recentre: bool,
}

/// Learns the + / − keys from what they type (spec §14 Phase 13B: the
/// German `+` is the US `]`; in the browser the logical key is the
/// `KeyboardEvent.key`), every frame, also outside the flight.
fn layout_keys(mut events: MessageReader<KeyboardInput>, mut layout: ResMut<FlightLayout>) {
    for e in events.read() {
        layout
            .0
            .on_key(e.key_code, &e.logical_key, e.state.is_pressed());
    }
}

/// When the last touch was (s, game time): the touch controls are shown
/// for a while after it (spec §3.2.2).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct FlightTouch {
    pub last_s: Option<f64>,
}

/// A steady flight input from the console (`god.cruise`, for scenes and
/// videos): forward, climb, turn rate (rad/s).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct Cruise(pub Option<(f64, f64, f64)>);

/// Run condition: not flying (the riding keys do nothing in god mode).
pub fn not_flying(world: &World) -> bool {
    crate::client(world).is_none_or(|c| !c.god().active())
}

/// The ride's keys count (the menu, console and workshop are closed).
fn ride_keys(world: &World) -> bool {
    world
        .get_resource::<InputContext>()
        .is_none_or(|c| *c == InputContext::Ride)
}

/// Prints a line to the console (if there is one).
fn print(world: &mut World, line: impl Into<String>) {
    if let Some(mut c) = world.get_resource_mut::<ConsoleState>() {
        c.print(line);
    }
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
                return Err("unicorn mode: only the host of this shared ride may use it".into());
            }
            Ok(c.god_mut().request_enter(now))
        }
        Ask::Off | Ask::Toggle | Ask::Back if flying => {
            let p = c.god().flight().pos;
            let spot = c.landing_spot(p[0], p[2]);
            c.god_mut().set_landing(spot);
            Ok(c.god_mut().request_leave(what == Ask::Back, now))
        }
        Ask::On => Err("already in unicorn mode".into()),
        _ if matches!(phase, Phase::Asking(_) | Phase::Leaving { .. }) => Ok(None),
        _ => Err("not in unicorn mode".into()),
    }
}

/// Asks on the app's client and sends the request.
fn send_ask(world: &mut World, what: Ask) -> Result<String, String> {
    let now = crate::now(world);
    crate::with_client(world, |c| Ok(((), ask(c, what, now)?)))?;
    // The server answers (printed when it does).
    Ok(String::new())
}

/// G toggles, Shift + G flies back, the gamepad's Select + North / West;
/// C switches to first person.
fn god_keys(world: &mut World) {
    if !ride_keys(world) {
        return;
    }
    // G and C, or their bound keys (spec §3.2.1 M9); the pad's chords
    // stay fixed.
    let bindings = world.get_resource::<crate::bindings::ActiveBindings>();
    let (g, shift, c_key) = match world.get_resource::<ButtonInput<KeyCode>>() {
        Some(k) => (
            crate::bindings::key_just_pressed(bindings, ocw_client::bindings::Action::GodMode, k),
            k.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]),
            crate::bindings::key_just_pressed(bindings, ocw_client::bindings::Action::Camera, k),
        ),
        None => (false, false, false),
    };
    let (select, north, west) = {
        let mut q = world.query::<&Gamepad>();
        let pads: Vec<&Gamepad> = q.iter(world).collect();
        (
            pads.iter().any(|p| p.pressed(GamepadButton::Select)),
            pads.iter().any(|p| p.just_pressed(GamepadButton::North)),
            pads.iter().any(|p| p.just_pressed(GamepadButton::West)),
        )
    };
    let Some(c) = crate::client(world) else {
        return;
    };
    let what = match input::god_key(g, shift, select, north, west) {
        GodKey::None => None,
        GodKey::Toggle => Some(Ask::Toggle),
        GodKey::FlyBack => c.god().flying().then_some(Ask::Back),
    };
    if let Some(what) = what
        && let Err(e) = send_ask(world, what)
    {
        print(world, e);
    }
    if c_key
        && let Some(mut c) = crate::client_mut(world)
        && c.god().active()
    {
        let fp = c.god_mut().toggle_first_person();
        info!(
            "unicorn mode: {} view",
            if fp { "first-person" } else { "flight" }
        );
    }
    if let Some(c) = crate::client(world) {
        let s = GodStatus::of(c);
        world.resource_mut::<GodStatus>().set_if_neq(s);
    }
}

/// The keys of the flight: the arrows (and W A S D, Q3) by position, + / −
/// by what they type.
fn flight_keys(keys: &ButtonInput<KeyCode>, layout: &LayoutKeys) -> FlightKeys {
    let any = |k: &[KeyCode]| keys.any_pressed(k.iter().copied());
    let pressed = |k: KeyCode| keys.pressed(k);
    let wasd = input::WASD_FLIES;
    FlightKeys {
        up: any(&[KeyCode::ArrowUp]) || (wasd && any(&[KeyCode::KeyW])),
        down: any(&[KeyCode::ArrowDown]) || (wasd && any(&[KeyCode::KeyS])),
        left: any(&[KeyCode::ArrowLeft]) || (wasd && any(&[KeyCode::KeyA])),
        right: any(&[KeyCode::ArrowRight]) || (wasd && any(&[KeyCode::KeyD])),
        plus: layout.held(KeyRole::Plus, &pressed),
        minus: layout.held(KeyRole::Minus, &pressed),
        shift: any(&[KeyCode::ShiftLeft, KeyCode::ShiftRight]),
        ctrl: any(&[KeyCode::ControlLeft, KeyCode::ControlRight]),
    }
}

/// A gamepad's flight controls; the triggers are read above their resting
/// value ([`ocw_client::input::trigger_pull`]: pads differ in what a
/// released trigger reads).
fn pad_flight(p: &Gamepad, rest: &mut [Option<f32>; 2]) -> PadFlight {
    use ocw_client::input::trigger_pull;
    let axis = |a| p.get(a).unwrap_or(0.0);
    let button = |b| p.get(b).unwrap_or(0.0);
    PadFlight {
        left_x: axis(GamepadAxis::LeftStickX),
        left_y: axis(GamepadAxis::LeftStickY),
        right_x: axis(GamepadAxis::RightStickX),
        right_y: axis(GamepadAxis::RightStickY),
        right_trigger: trigger_pull(button(GamepadButton::RightTrigger2), &mut rest[0]),
        left_trigger: trigger_pull(button(GamepadButton::LeftTrigger2), &mut rest[1]),
        south: p.pressed(GamepadButton::South),
    }
}

/// The world set's area, else the route's (grown by 3 km).
fn flight_bounds(
    packages: Option<&ocw_render::package::PackageWorld>,
    view: Option<&RideView>,
) -> Option<Bounds> {
    if let Some(w) = packages {
        let (min, max) = w.set.bounds();
        if min[0] <= max[0] && min[1] <= max[1] {
            return Some(Bounds { min, max });
        }
    }
    let (min, max) = ocw_render::route_bounds(&view?.route);
    (min[0] <= max[0]).then(|| Bounds { min, max }.grown(3_000.0))
}

/// The input devices of the flight.
type InputParams<'w> = (
    Option<Res<'w, ButtonInput<KeyCode>>>,
    Option<Res<'w, ButtonInput<MouseButton>>>,
    Option<Res<'w, AccumulatedMouseMotion>>,
    Option<Res<'w, AccumulatedMouseScroll>>,
);

/// The fingers on the screen and the window's size (touch flight).
type TouchParams<'w, 's> = (
    Option<Res<'w, bevy::input::touch::Touches>>,
    Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    ResMut<'w, FlightTouch>,
);

/// The resting values of each gamepad's triggers.
type TriggerRest = std::collections::HashMap<Entity, [Option<f32>; 2]>;

/// The flight's input this frame, only in the ride's input context: keys
/// (+ / − by what they type), gamepads, the mouse (while the pointer is
/// captured or the left button held), the wheel, touch (spec §3.2.2: the
/// left half steers, the right half looks, the buttons climb).
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn gather_flight(
    (keys, buttons, motion, scroll): InputParams<'_>,
    pads: Query<(Entity, &Gamepad)>,
    context: Option<Res<InputContext>>,
    (pointer, settings, layout, status): (
        Res<FlightPointer>,
        Res<FlightSettings>,
        Res<FlightLayout>,
        Res<GodStatus>,
    ),
    (touches, windows, mut flight_touch): TouchParams,
    time: Res<Time>,
    mut frame: ResMut<FlightFrame>,
    mut trigger_rest: Local<TriggerRest>,
) {
    let mut out = FlightFrame::default();
    if !status.active && !status.asking {
        frame.set_if_neq(out);
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
    if riding_keys && let Some(k) = keys.as_deref() {
        out.input = input::flight_input(&flight_keys(k, &layout.0), &pad_list);
    }
    // Touch (spec §3.2.2): the left half steers, the right half looks, the
    // buttons climb.
    let fingers: Vec<touch::Finger> = match touches.as_deref() {
        Some(t) if riding_keys => t
            .iter()
            .map(|f| touch::Finger {
                id: f.id(),
                pos: f.position(),
                start: f.start_position(),
                prev: f.previous_position(),
            })
            .collect(),
        _ => Vec::new(),
    };
    if !fingers.is_empty() {
        flight_touch.last_s = Some(now);
    }
    let size = windows.single().map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let by_touch = touch::touch_flight(&fingers, size, settings.invert_y);
    let i = &mut out.input;
    i.forward = (i.forward + by_touch.input.forward).clamp(-1.0, 1.0);
    i.side = (i.side + by_touch.input.side).clamp(-1.0, 1.0);
    i.climb = (i.climb + by_touch.input.climb).clamp(-1.0, 1.0);
    // A touch also arrives as an emulated mouse (Windows, browsers): not
    // twice.
    let left_held = buttons
        .as_deref()
        .is_some_and(|b| b.pressed(MouseButton::Left));
    let mouse = match motion.as_deref() {
        Some(m) if riding_keys && fingers.is_empty() && (pointer.captured || left_held) => {
            (m.delta.x, m.delta.y)
        }
        _ => (0.0, 0.0),
    };
    let mut look = input::look_delta(mouse, &pad_list, settings.invert_y, dt);
    look.0 += by_touch.look.0;
    look.1 += by_touch.look.1;
    out.look = look;
    // L or the middle mouse button: look ahead again (plane).
    out.recentre = riding_keys
        && (keys
            .as_deref()
            .is_some_and(|k| k.just_pressed(KeyCode::KeyL))
            || buttons
                .as_deref()
                .is_some_and(|b| b.just_pressed(MouseButton::Middle)));
    out.wheel = match scroll.as_deref() {
        Some(s) if riding_keys => f64::from(match s.unit {
            MouseScrollUnit::Line => s.delta.y,
            MouseScrollUnit::Pixel => s.delta.y / 40.0,
        }),
        _ => 0.0,
    } + by_touch.wheel;
    frame.set_if_neq(out);
}

/// The flight, each frame: the frame's input ([`FlightFrame`]) or the
/// cruise, the step over the terrain and the buildings, the landing spot
/// (4 times a second), the poses to the server.
fn fly(world: &mut World) {
    let now = crate::now(world);
    let dt = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.delta_secs_f64());
    let settings = *world.resource::<FlightSettings>();
    let cruise = world.resource::<Cruise>().0;
    let frame = *world.resource::<FlightFrame>();
    let last_spot = world.resource::<LandingTimer>().0;
    // Terrain and buildings (the camera's obstacles) and the tree crowns
    // (only the flight's: the camera sees through trees); cheap clones
    // (shared functions), so the client can be borrowed mutably below.
    let ground = world.get_resource::<ocw_render::Ground>().cloned();
    let obstacles = world
        .get_resource::<ocw_render::camera::CameraObstacles>()
        .cloned()
        .unwrap_or_default();
    let trees = world
        .get_resource::<ocw_render::flight_world::FlightObstacles>()
        .cloned();
    let bounds = flight_bounds(
        world.get_resource::<ocw_render::package::PackageWorld>(),
        world.get_resource::<RideView>(),
    );
    let surface = |x: f64, z: f64| {
        let solid = ocw_render::camera::solid_height(ground.as_ref(), &obstacles, x, z);
        let tree = trees.as_ref().and_then(|t| t.0.top_at(x, z));
        match (solid, tree) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    };
    let mut lines = Vec::new();
    let mut spot_at = last_spot;
    let (msg, status) = {
        let Some(mut c) = crate::client_mut(world) else {
            return;
        };
        c.god_mut().set_model(settings.model);
        if let Some(e) = c.take_god_notice() {
            lines.push(match e {
                self::state::GodEvent::Landed => "unicorn mode ended by the server".into(),
                other => format!("{other:?}"),
            });
        }
        let mut msg = None;
        if c.god().active() || matches!(c.god().phase(), Phase::Asking(_)) {
            if frame.recentre {
                c.god_mut().recentre_look();
            }
            let mut flight = frame.input;
            let mut look = frame.look;
            if let Some((fw, climb, turn)) = cruise
                && flight == FlightInput::default()
            {
                flight = cruise_input(c.god().flight(), fw, climb, turn);
                if settings.model == FlightModel::Hover {
                    look.0 += turn * dt;
                }
            }
            if let Some(self::state::GodEvent::Refused(why)) =
                c.god_mut()
                    .step(&flight, look, frame.wheel, dt, now, &surface, bounds)
            {
                lines.push(format!("unicorn mode: {why}"));
            }
            if c.god().flying() && now - last_spot >= 0.25 {
                spot_at = now;
                let p = c.god().flight().pos;
                let spot = c.landing_spot(p[0], p[2]);
                c.god_mut().set_landing(spot);
            }
            msg = c.god_mut().pose_message(now);
        }
        (msg, GodStatus::of(&c))
    };
    world.resource_mut::<LandingTimer>().0 = spot_at;
    world.resource_mut::<GodStatus>().set_if_neq(status);
    for l in lines {
        print(world, l);
    }
    if let Some(m) = msg {
        crate::send(world, m);
    }
}

/// The flight input of `god.cruise FORWARD CLIMB TURN` (scenes, videos).
/// Plane: FORWARD sets the throttle, TURN (rad/s, positive = left) banks
/// as far as that turn needs at the speed now; hover: FORWARD moves, the
/// caller turns the view.
pub fn cruise_input(f: &ocw_sim::god::Flight, fw: f64, climb: f64, turn: f64) -> FlightInput {
    use ocw_sim::god;
    match f.model {
        FlightModel::Plane => FlightInput {
            set_throttle: Some(fw.abs().clamp(god::MIN_THROTTLE, 1.0)),
            side: god::bank_for_turn(turn, f.horizontal_speed()) / god::MAX_BANK,
            climb,
            ..Default::default()
        },
        FlightModel::Hover => FlightInput {
            forward: fw,
            climb,
            ..Default::default()
        },
    }
}

/// God mode's part of Esc (spec §14 Phase 13A; the shared menu calls it
/// first): a captured pointer is freed, and that Esc opens no menu.
pub fn free_pointer(world: &mut World) -> bool {
    match world.get_resource_mut::<FlightPointer>() {
        Some(mut f) if f.captured => {
            f.captured = false;
            true
        }
        _ => false,
    }
}

/// Captures the pointer when the flight begins and frees it when it ends
/// (or Esc freed it, [`free_pointer`]); a click captures it again. In the
/// browser a captured pointer is the page's Pointer Lock (Bevy asks for
/// it on the canvas; the key or click that started it is the user
/// gesture it needs).
fn grab_pointer(
    status: Res<GodStatus>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    context: Option<Res<InputContext>>,
    mut pointer: ResMut<FlightPointer>,
    mut was_active: Local<bool>,
    mut windows: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let active = status.flying;
    if active && !*was_active {
        pointer.captured = true;
    } else if !active && *was_active {
        pointer.captured = false;
    } else if active
        && !pointer.captured
        && buttons.is_some_and(|b| b.just_pressed(MouseButton::Left))
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

/// How long the browser may take to grant the Pointer Lock (s).
pub const POINTER_LOCK_GRACE_S: f64 = 1.0;

/// The browser's side of [`FlightPointer`] (pure; fed by the web app with
/// `document.pointerLockElement`): the browser itself ends a Pointer Lock
/// on Esc (the page may never see that key) or refuses one without a user
/// gesture, so a captured pointer whose lock is gone – or never came
/// within [`POINTER_LOCK_GRACE_S`] – is freed. The next Esc then opens the
/// menu, and a click asks for the lock again.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct PointerLockWatch {
    /// When the capture began (s).
    since: Option<f64>,
    /// The page had the lock during this capture.
    had_lock: bool,
}

impl PointerLockWatch {
    /// Whether the pointer stays captured, given the page's lock now.
    pub fn keep(&mut self, captured: bool, locked: bool, now_s: f64) -> bool {
        if !captured {
            *self = Self::default();
            return false;
        }
        let since = *self.since.get_or_insert(now_s);
        if locked {
            self.had_lock = true;
            return true;
        }
        if self.had_lock || now_s - since > POINTER_LOCK_GRACE_S {
            *self = Self::default();
            return false;
        }
        true
    }
}

/// The flight and the unicorns for the renderer.
fn update_view(world: &mut World) {
    let now = crate::now(world);
    let touch = *world.resource::<FlightTouch>();
    let Some(c) = crate::client(world) else {
        return;
    };
    let unicorns = c.unicorns(now);
    let active = c.god().active();
    let g = c.god();
    let flight = g.unicorn(now).map(|u| {
        let f = g.flight();
        ocw_render::FlightView {
            pos: u.pose.pos,
            yaw: if u.leaving { u.pose.yaw } else { f.view_yaw() },
            pitch: if u.leaving {
                f.view_pitch().min(-0.15)
            } else {
                f.view_pitch()
            },
            roll: if u.leaving { 0.0 } else { f.camera_roll() },
            speed_mps: u.pose.speed,
            height_m: g.height_m(),
            scale: f.scale,
            first_person: g.first_person(),
            landing: g.landing(),
            leaving: u.leaving,
            keys: match f.model {
                FlightModel::Plane => KEYS,
                FlightModel::Hover => KEYS_HOVER,
            }
            .into(),
            velocity: if u.leaving {
                [0.0, 0.0]
            } else {
                [f.vel[0], f.vel[2]]
            },
            touch: touch::overlay_shown(touch.last_s, now),
            ..Default::default()
        }
    });
    let Some(mut view) = world.get_resource_mut::<RideView>() else {
        return;
    };
    view.unicorns = unicorns;
    if active {
        // No junction overlay over the flight (the road below is frozen).
        view.junction = None;
    }
    view.flight = flight;
}

/// The rider list of the session (console `ride.riders`): one line per
/// rider, the own one marked, who flies in god mode marked too (spec §14
/// Phase 13E: a host sees who is in god mode).
pub fn rider_list(view: &RideView) -> Vec<String> {
    if view.riders.is_empty() {
        return vec!["no riders yet".into()];
    }
    let flying = view.unicorns.len();
    let mut out = vec![format!(
        "{} riders, {flying} in {}",
        view.riders.len(),
        MENU_NAME.to_lowercase()
    )];
    for r in &view.riders {
        let god = view.unicorns.iter().any(|u| u.id == r.id);
        out.push(format!(
            "{:>4}  {}{}{}",
            r.id.0,
            r.name,
            if r.is_local { " (you)" } else { "" },
            if god { "  - flying (unicorn mode)" } else { "" }
        ));
    }
    out
}

fn on_off(m: &clap::ArgMatches) -> bool {
    m.get_one::<String>("state").is_none_or(|s| s == "on")
}

/// The `god status` line.
fn status_line(c: &ClientCore) -> String {
    let g = c.god();
    let f = g.flight();
    match g.phase() {
        Phase::Riding => format!(
            "riding ({})",
            if c.may_use_god() {
                "G enters unicorn mode"
            } else {
                "unicorn mode is not allowed here"
            }
        ),
        Phase::Asking(_) => "asking the server ...".into(),
        Phase::Flying { .. } => format!(
            "flying ({}) at x {:.0} z {:.0}, {:.0} m ({:.0} m above ground), {:.0} m/s, scale x{:.2}{}",
            g.model().name(),
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
    }
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
            .about("Unicorn mode: fly over the world as a unicorn (G; Shift+G flies back)")
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
                    let c = crate::client(world).ok_or("not riding")?;
                    return Ok(status_line(c));
                }
                _ => Ask::Toggle,
            };
            send_ask(world, what)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("god.pose")
            .about("Unicorn mode: move the unicorn to x z (local metres; 'here' keeps the current one), HEIGHT m above the ground, looking along YAW (deg, 0 = north, 90 = east; 'here' keeps it) and PITCH (deg, up)")
            .arg(Arg::new("x").required(true).allow_negative_numbers(true))
            .arg(Arg::new("z").required(true).allow_negative_numbers(true))
            .arg(Arg::new("height").required(true).value_parser(value_parser!(f64)))
            .arg(Arg::new("yaw").allow_negative_numbers(true))
            .arg(Arg::new("pitch").allow_negative_numbers(true).value_parser(value_parser!(f64))),
        |world, m| {
            let here = crate::client(world).ok_or("not riding")?.god().flight().pos;
            let coord = |name: &str, current: f64| -> Result<f64, String> {
                match m.get_one::<String>(name).map(String::as_str) {
                    Some("here") | None => Ok(current),
                    Some(v) => v.parse().map_err(|_| format!("{name}: a number or 'here', not {v:?}")),
                }
            };
            let x = coord("x", here[0])?;
            let z = coord("z", here[2])?;
            let h = *m.get_one::<f64>("height").expect("required");
            // Compass heading → the flight's yaw (counter-clockwise from north).
            let yaw = match m.get_one::<String>("yaw").map(String::as_str) {
                None | Some("here") => None,
                Some(v) => Some(
                    -v.parse::<f64>()
                        .map_err(|_| format!("yaw: degrees or 'here', not {v:?}"))?
                        .to_radians(),
                ),
            };
            let pitch = m.get_one::<f64>("pitch").map(|d| d.to_radians());
            let ground = world
                .get_resource::<ocw_render::Ground>()
                .and_then(|g| (g.0)(x, z))
                .unwrap_or(0.0);
            let y = ground + h.max(ocw_sim::god::CLEARANCE_M);
            let moved = crate::client_mut(world)
                .is_some_and(|mut c| c.god_mut().teleport([x, y, z], yaw, pitch));
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
            .about("Unicorn mode: fly by itself (scenes, videos) – FORWARD (plane: the throttle 0 … 1; hover: -1 … 1) and CLIMB (-1 … 1), TURN (deg/s, positive = left; the plane banks into it); 'god.cruise 0' stops")
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
            .about("Unicorn mode: the flight camera behind the unicorn, or first-person from its head (C)")
            .arg(Arg::new("view").value_parser(["flight", "first"]).required(true)),
        |world, m| {
            let first = m.get_one::<String>("view").is_some_and(|v| v == "first");
            let mut c = crate::client_mut(world).ok_or("not riding")?;
            if c.god().first_person() != first {
                c.god_mut().toggle_first_person();
            }
            Ok(format!("unicorn mode view: {}", if first { "first-person" } else { "flight" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.trail")
            .about("Unicorn mode: the rainbow trail behind the unicorns (Settings → Display → Effects)")
            .arg(state()),
        |world, m| {
            let on = on_off(m);
            world.insert_resource(ocw_render::unicorn::UnicornTrail(on));
            Ok(format!("unicorn trail {}", if on { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ride.riders")
            .about("The riders of this ride, and who of them flies in unicorn mode"),
        |world, _| {
            let view = world.get_resource::<RideView>().ok_or("not riding")?;
            Ok(rider_list(view).join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.flight")
            .about("Unicorn mode: fly like a plane (always forward; arrows: throttle and bank) or hover (arrows: move, mouse: turn) – Settings → Controls")
            .arg(Arg::new("model").value_parser(["plane", "hover"])),
        |world, m| {
            let mut s = world.resource_mut::<FlightSettings>();
            if let Some(model) = m
                .get_one::<String>("model")
                .and_then(|v| FlightModel::parse(v))
            {
                s.model = model;
            }
            Ok(format!("unicorn mode flight: {}", s.model.name()))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("god.invert")
            .about("Unicorn mode: invert the mouse's up / down look (Settings → Controls)")
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
    use std::time::Duration;

    use bevy::time::TimeUpdateStrategy;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_server::local::FrameServer;
    use ocw_world::Route;

    use super::*;

    #[test]
    fn the_rider_list_shows_who_flies() {
        let mut v = RideView::new(Route::test_loop());
        assert_eq!(rider_list(&v), vec!["no riders yet".to_string()]);
        let rider = |id: u32, name: &str, local: bool| ocw_render::RiderView {
            id: ocw_core::RiderId(id),
            name: name.into(),
            state: Default::default(),
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: local,
            bike: Default::default(),
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
            cue: Default::default(),
        };
        v.riders = vec![rider(1, "Host", true), rider(2, "Ada", false)];
        v.unicorns.push(ocw_render::UnicornView {
            id: ocw_core::RiderId(2),
            name: "Ada".into(),
            is_local: false,
            pos: [0.0; 3],
            yaw: 0.0,
            pitch: 0.0,
            speed_mps: 0.0,
            climb_mps: 0.0,
            turn_rate: 0.0,
            accel: 0.0,
            age_s: 1.0,
            landing: false,
            colour: ocw_core::Rgb(0),
        });
        let l = rider_list(&v);
        assert_eq!(l[0], "2 riders, 1 in unicorn mode");
        assert!(l[1].ends_with("Host (you)"), "{}", l[1]);
        assert!(l[2].ends_with("Ada  - flying (unicorn mode)"), "{}", l[2]);
    }

    /// The browser's Pointer Lock: kept while the page has it, freed when
    /// the browser ends it (Esc) or never grants it (no user gesture).
    #[test]
    fn the_pointer_lock_watch_frees_a_lost_or_refused_lock() {
        let mut w = PointerLockWatch::default();
        assert!(!w.keep(false, false, 0.0), "not captured");
        // Granted after a few frames, then ended by the browser's Esc.
        assert!(w.keep(true, false, 1.0));
        assert!(w.keep(true, true, 1.1));
        assert!(w.keep(true, true, 5.0));
        assert!(!w.keep(true, false, 5.1), "the browser ended the lock");
        // A click asks again: refused (no gesture) – freed after the grace.
        assert!(w.keep(true, false, 6.0));
        assert!(w.keep(true, false, 6.0 + POINTER_LOCK_GRACE_S - 0.01));
        assert!(!w.keep(true, false, 6.0 + POINTER_LOCK_GRACE_S + 0.01));
        // Freed by the app (Esc in the app, landing): starts over.
        assert!(w.keep(true, true, 10.0));
        assert!(!w.keep(false, true, 10.1));
        assert!(w.keep(true, false, 10.2), "a new capture waits again");
    }

    /// A ride on the in-memory local server (the browser's `FrameServer`),
    /// reached through [`crate::RideAccess`] as the apps do.
    #[derive(Resource)]
    struct Ride {
        client: ClientCore,
        server: FrameServer,
    }

    fn step_ride(time: Res<Time>, mut ride: ResMut<Ride>) {
        let now = time.elapsed_secs_f64();
        let Ride { client, server } = &mut *ride;
        server.advance(time.delta_secs_f64());
        for m in server.drain() {
            let _ = client.on_server_message(&m, now);
        }
        client.predict_step();
    }

    fn app() -> App {
        let route = Route::test_loop();
        let client = ClientCore::new(route.clone(), "Uni", Default::default());
        let mut server = FrameServer::new(route.clone(), 0, 0.0);
        server.send(&client.hello());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .insert_resource(crate::UiPlatform(crate::Platform::Browser))
            .insert_resource(RideView::new(route))
            .insert_resource(Ride { client, server })
            .insert_resource(crate::RideAccess {
                client: |w| w.get_resource::<Ride>().map(|r| &r.client),
                client_mut: |w| {
                    w.get_resource_mut::<Ride>()
                        .map(|r| r.map_unchanged(|r| &mut r.client))
                },
                send: |w, bytes| {
                    if let Some(mut r) = w.get_resource_mut::<Ride>() {
                        r.server.send(&bytes);
                    }
                },
            })
            .add_systems(Update, step_ride.in_set(crate::RideViewSet))
            .add_plugins((
                crate::settings::SettingsPlugin,
                crate::weather::WeatherPlugin,
                crate::timemachine::TimeMachinePlugin,
                crate::riders_page::RidersPagePlugin,
                crate::menu::MenuPlugin,
                GodPlugin,
            ));
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        app
    }

    fn client(app: &App) -> &ClientCore {
        &app.world().resource::<Ride>().client
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().to_vec()
    }

    fn update_until(app: &mut App, done: impl Fn(&ClientCore) -> bool) -> bool {
        for _ in 0..400 {
            app.update();
            if done(client(app)) {
                return true;
            }
        }
        false
    }

    fn frames(app: &mut App, n: usize) {
        for _ in 0..n {
            app.update();
        }
    }

    /// A key as the browser's `KeyboardEvent` arrives: its position
    /// (`code`) and what it types (`key`).
    fn key(app: &mut App, code: KeyCode, typed: &str, down: bool) {
        app.world_mut().write_message(KeyboardInput {
            key_code: code,
            logical_key: bevy::input::keyboard::Key::Character(typed.into()),
            state: if down {
                bevy::input::ButtonState::Pressed
            } else {
                bevy::input::ButtonState::Released
            },
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        if down {
            keys.press(code);
        } else {
            keys.release(code);
        }
    }

    fn tap(app: &mut App, code: KeyCode) {
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(code);
        app.update();
        let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
        keys.release(code);
        keys.clear();
        app.update();
    }

    /// Unicorn mode through the shared plugin over a ride reached by
    /// `RideAccess` (the browser's setup, spec §14 Phase 13 G5): G enters, the
    /// plus key climbs (by what it types), the inspection's panel and crosshair,
    /// a pin, Esc frees the pointer first and then opens the menu, G
    /// lands.
    #[test]
    fn unicorn_mode_flies_inspects_and_lands_through_ride_access() {
        let mut app = app();
        assert!(update_until(&mut app, |c| c.rider_id().is_some()));
        frames(&mut app, 10);
        assert!(not_flying_now(&app));
        tap(&mut app, KeyCode::KeyG);
        assert!(
            update_until(&mut app, |c| c.god().flying()),
            "{:?}",
            client(&app).god().phase()
        );
        app.update();
        assert!(app.world().resource::<FlightPointer>().captured);
        assert!(app.world().resource::<GodStatus>().flying);
        assert!(!not_flying_now(&app));
        let view = app.world().resource::<RideView>();
        assert!(view.flight.is_some());
        assert_eq!(view.unicorns.len(), 1);
        // + climbs: the US `=` key typing "+" (with Shift: also a boost).
        let y0 = client(&app).god().flight().pos[1];
        key(&mut app, KeyCode::Equal, "+", true);
        frames(&mut app, 60);
        key(&mut app, KeyCode::Equal, "+", false);
        app.update();
        let y1 = client(&app).god().flight().pos[1];
        assert!(y1 > y0 + 2.0, "+ climbs: {y0} → {y1}");
        // − descends (the German `-`, the US `/` position).
        key(&mut app, KeyCode::Slash, "-", true);
        frames(&mut app, 60);
        key(&mut app, KeyCode::Slash, "-", false);
        app.update();
        let y2 = client(&app).god().flight().pos[1];
        assert!(y2 < y1 - 1.0, "- descends: {y1} → {y2}");
        // G3: the panel and the crosshair; a pin.
        frames(&mut app, 15);
        let f = app.world().resource::<RideView>().flight.clone().unwrap();
        assert!(
            f.panel.iter().any(|l| l.starts_with("Position x")),
            "{:?}",
            f.panel
        );
        assert!(f.crosshair.is_some());
        let out = run(&mut app, "god.pin");
        assert!(out.iter().any(|l| l.contains("pin 1 put")), "{out:?}");
        app.update();
        assert_eq!(app.world().resource::<RideView>().pins.len(), 1);
        tap(&mut app, KeyCode::KeyI);
        frames(&mut app, 2);
        let f = app.world().resource::<RideView>().flight.clone().unwrap();
        assert!(f.panel.is_empty() && f.crosshair.is_none(), "I hides it");
        tap(&mut app, KeyCode::KeyI);
        // Esc: first the pointer, then the menu (World → Unicorn mode).
        tap(&mut app, KeyCode::Escape);
        assert!(!app.world().resource::<FlightPointer>().captured);
        let menu = app.world().resource::<ocw_render::menu::MenuView>();
        assert!(!menu.open, "the first Esc frees the pointer only");
        tap(&mut app, KeyCode::Escape);
        assert!(app.world().resource::<ocw_render::menu::MenuView>().open);
        run(&mut app, "menu close");
        frames(&mut app, 2);
        // G lands on the route below.
        tap(&mut app, KeyCode::KeyG);
        assert!(
            update_until(&mut app, |c| c.god().phase() == Phase::Riding),
            "{:?}",
            client(&app).god().phase()
        );
        frames(&mut app, 2);
        assert!(!app.world().resource::<FlightPointer>().captured);
        assert!(app.world().resource::<RideView>().flight.is_none());
        let out = run(&mut app, "god status");
        assert!(out.iter().any(|l| l.starts_with("riding")), "{out:?}");
        // The test's view has no riders (the apps fill them).
        let out = run(&mut app, "ride.riders");
        assert!(out.iter().any(|l| l.contains("no riders yet")), "{out:?}");
    }

    fn not_flying_now(app: &App) -> bool {
        not_flying(app.world())
    }

    /// The flight settings apply in the browser too (spec §14 Phase 13 G5:
    /// the platform filter no longer hides them).
    #[test]
    fn the_flight_settings_apply_through_their_commands() {
        let mut app = app();
        app.update();
        run(&mut app, "set controls.invert_y on");
        run(&mut app, "set controls.god_flight hover");
        app.update();
        let s = *app.world().resource::<FlightSettings>();
        assert!(s.invert_y);
        assert_eq!(s.model, FlightModel::Hover);
        run(&mut app, "set display.unicorn_trail off");
        app.update();
        assert_eq!(
            app.world()
                .get_resource::<ocw_render::unicorn::UnicornTrail>()
                .map(|t| t.0),
            Some(false)
        );
    }
}

//! The 3D view's camera: the **unicorn flight controls** of Phase 13
//! (spec §14 Phase 16, 16A) – the same input mapping as the game's god
//! mode (`ocw_ui::god::input`: arrows, + / −, Shift, Ctrl, the wheel, the
//! gamepads) and the same flight model (`ocw_sim::god::Flight`, over the
//! terrain and the buildings, inside the world's bounds), without the
//! unicorn: the renderer's first-person flight camera ([`RideView::flight`]
//! with `first_person`), and no unicorn in [`RideView::unicorns`]. The
//! mouse looks around while a button is held over the 3D view.

use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use ocw_render::RideView;
use ocw_sim::god::{Bounds, Flight, FlightModel};
use ocw_ui::god::input::{FlightKeys, LayoutKeys, PadFlight, flight_input, look_delta};
use ocw_ui::god::{flight_keys, pad_flight};

/// The keys line of the studio's flight HUD.
pub const KEYS_HOVER: &str = "arrows: fly, +/-: up/down, drag in the view: look, Shift: boost, Ctrl: precise, wheel: speed, double-click the map: fly there | ` or F12: console";
pub const KEYS_PLANE: &str = "Up/Down: faster/slower, Left/Right: bank, +/-: climb/descend, drag in the view: look, L: ahead, Shift: boost, Ctrl: precise, wheel: speed | ` or F12: console";

/// The height above the ground a fresh view starts at (m).
pub const START_HEIGHT_M: f64 = 250.0;
/// The start view's pitch (rad, down).
pub const START_PITCH: f64 = -0.45;

/// How far behind (south of) and above a selection the camera looks at it
/// from (m).
pub const LOOK_BACK_M: f64 = 140.0;
pub const LOOK_UP_M: f64 = 90.0;

/// A camera that looks north at `target` (x, height, z) from
/// [`LOOK_BACK_M`] south of it and [`LOOK_UP_M`] above it: (eye, yaw,
/// pitch).
pub fn look_at(target: [f64; 3]) -> ([f64; 3], f64, f64) {
    (
        [target[0], target[1] + LOOK_UP_M, target[2] + LOOK_BACK_M],
        0.0,
        -(LOOK_UP_M.atan2(LOOK_BACK_M)),
    )
}

/// The studio camera's flight.
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct StudioFlight {
    pub flight: Flight,
    /// Height above the ground below (m), from the last step.
    pub height_m: f64,
}

impl Default for StudioFlight {
    fn default() -> Self {
        Self::at([0.0, START_HEIGHT_M, 0.0], 0.0, FlightModel::Hover)
    }
}

impl StudioFlight {
    /// A still flight at `pos` looking along `yaw`, a little down.
    pub fn at(pos: [f64; 3], yaw: f64, model: FlightModel) -> Self {
        let mut flight = Flight::hover(pos, yaw);
        flight.model = model;
        flight.pitch = START_PITCH;
        Self {
            flight,
            height_m: START_HEIGHT_M,
        }
    }

    /// Jumps to `pos` (a saved view, a map double-click), still.
    pub fn jump(&mut self, pos: [f64; 3], yaw: f64, pitch: f64) {
        self.flight.pos = pos;
        self.flight.yaw = yaw;
        self.flight.pitch = pitch;
        self.flight.stop();
    }

    /// What the renderer draws: the first-person flight camera.
    pub fn view(&self) -> ocw_render::FlightView {
        let f = &self.flight;
        ocw_render::FlightView {
            pos: f.pos,
            yaw: f.view_yaw(),
            pitch: f.view_pitch(),
            roll: f.camera_roll(),
            speed_mps: f.speed(),
            height_m: self.height_m,
            scale: f.scale,
            first_person: true,
            keys: match f.model {
                FlightModel::Hover => KEYS_HOVER,
                FlightModel::Plane => KEYS_PLANE,
            }
            .into(),
            velocity: [f.vel[0], f.vel[2]],
            ..Default::default()
        }
    }
}

/// Whether the UI has the keyboard or the pointer this frame (written by
/// the panels; the flight then ignores them).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct UiFocus {
    /// A text field has the keyboard.
    pub keyboard: bool,
    /// The pointer is over the 3D view (not over a panel).
    pub over_view: bool,
}

/// The `+` / `−` keys of the keyboard's layout (the god mode's
/// [`LayoutKeys`]).
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct StudioLayout(pub LayoutKeys);

pub struct FlightPlugin;

impl Plugin for FlightPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StudioFlight>()
            .init_resource::<UiFocus>()
            .init_resource::<StudioLayout>()
            .add_systems(Update, (layout_keys, fly).chain());
    }
}

fn layout_keys(mut events: MessageReader<KeyboardInput>, mut layout: ResMut<StudioLayout>) {
    for e in events.read() {
        layout
            .0
            .on_key(e.key_code, &e.logical_key, e.state.is_pressed());
    }
}

/// The world's bounds for the flight's soft stop: the world set's tiles.
fn bounds(world: Option<&ocw_render::package::PackageWorld>) -> Option<Bounds> {
    let (min, max) = world?.set.bounds();
    (min[0] <= max[0] && min[1] <= max[1]).then_some(Bounds { min, max })
}

/// What the flight flies over: terrain, buildings, tree crowns, bounds.
type WorldParams<'w> = (
    Option<Res<'w, ocw_render::Ground>>,
    Option<Res<'w, ocw_render::camera::CameraObstacles>>,
    Option<Res<'w, ocw_render::flight_world::FlightObstacles>>,
    Option<Res<'w, ocw_render::package::PackageWorld>>,
);

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn fly(
    time: Res<Time>,
    keys: Option<Res<ButtonInput<KeyCode>>>,
    buttons: Option<Res<ButtonInput<MouseButton>>>,
    motion: Option<Res<AccumulatedMouseMotion>>,
    scroll: Option<Res<AccumulatedMouseScroll>>,
    pads: Query<(Entity, &Gamepad)>,
    mut trigger_rest: Local<bevy::platform::collections::HashMap<Entity, [Option<f32>; 2]>>,
    (focus, layout, console): (
        Res<UiFocus>,
        Res<StudioLayout>,
        Option<Res<ocw_render::console::ConsoleState>>,
    ),
    (ground, obstacles, trees, package): WorldParams,
    mut state: ResMut<StudioFlight>,
    mut view: ResMut<RideView>,
) {
    let dt = time.delta_secs_f64();
    let typing = focus.keyboard || console.is_some_and(|c| c.is_open());
    trigger_rest.retain(|e, _| pads.contains(*e));
    let pad_list: Vec<PadFlight> = pads
        .iter()
        .map(|(e, p)| pad_flight(p, trigger_rest.entry(e).or_default()))
        .collect();
    let input = match keys.as_deref() {
        Some(k) if !typing => flight_input(&flight_keys(k, &layout.0), &pad_list),
        _ => flight_input(&FlightKeys::default(), &pad_list),
    };
    let held = buttons
        .as_deref()
        .is_some_and(|b| b.pressed(MouseButton::Left) || b.pressed(MouseButton::Right));
    let mouse = match motion.as_deref() {
        Some(m) if held && focus.over_view => (m.delta.x, m.delta.y),
        _ => (0.0, 0.0),
    };
    let look = look_delta(mouse, &pad_list, false, dt);
    let wheel = match scroll.as_deref() {
        Some(s) if focus.over_view => f64::from(match s.unit {
            MouseScrollUnit::Line => s.delta.y,
            MouseScrollUnit::Pixel => s.delta.y / 40.0,
        }),
        _ => 0.0,
    };
    let empty = ocw_render::camera::CameraObstacles::default();
    let obstacles = obstacles.as_deref().unwrap_or(&empty);
    let surface = |x: f64, z: f64| {
        let solid = ocw_render::camera::solid_height(ground.as_deref(), obstacles, x, z);
        let tree = trees
            .as_deref()
            .and_then(|t| ocw_render::camera::solid_height(None, &t.0, x, z));
        match (solid, tree) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    };
    let f = &mut state.flight;
    if !typing
        && keys
            .as_deref()
            .is_some_and(|k| k.just_pressed(KeyCode::KeyL))
    {
        f.recentre_look();
    }
    f.look(look.0, look.1);
    if wheel != 0.0 {
        f.scroll(wheel);
    }
    f.step(&input, dt, &surface, bounds(package.as_deref()));
    let p = f.pos;
    state.height_m = surface(p[0], p[2]).map_or(p[1], |h| p[1] - h);
    view.flight = Some(state.view());
    // No unicorn: the studio flies the camera only.
    view.unicorns.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_view_is_the_first_person_camera_without_a_unicorn() {
        let s = StudioFlight::at([10.0, 500.0, -20.0], 1.0, FlightModel::Hover);
        let v = s.view();
        assert!(v.first_person);
        assert_eq!(v.pos, [10.0, 500.0, -20.0]);
        assert!((v.pitch - START_PITCH).abs() < 1e-9);
        assert_eq!(v.keys, KEYS_HOVER);
        assert_eq!(v.landing, None);
        let p = StudioFlight::at([0.0; 3], 0.0, FlightModel::Plane).view();
        assert_eq!(p.keys, KEYS_PLANE);
    }

    #[test]
    fn the_camera_looks_at_a_selection_from_the_south() {
        let (eye, yaw, pitch) = look_at([100.0, 400.0, 50.0]);
        assert_eq!(eye, [100.0, 490.0, 190.0]);
        assert_eq!(yaw, 0.0, "north");
        // The view's line goes through the target.
        let drop = -pitch.tan() * LOOK_BACK_M;
        assert!((drop - LOOK_UP_M).abs() < 1e-9);
    }

    #[test]
    fn a_jump_stops_the_flight() {
        let mut s = StudioFlight::default();
        s.flight.vel = [5.0, 1.0, 2.0];
        s.jump([1.0, 2.0, 3.0], 0.5, -0.2);
        assert_eq!(s.flight.pos, [1.0, 2.0, 3.0]);
        assert_eq!(s.flight.speed(), 0.0);
        assert_eq!(s.view().yaw, 0.5);
    }

    #[test]
    fn the_arrow_keys_fly_like_the_unicorn() {
        let mut k = ButtonInput::<KeyCode>::default();
        k.press(KeyCode::ArrowUp);
        k.press(KeyCode::ShiftLeft);
        let keys = flight_keys(&k, &LayoutKeys::default());
        assert!(keys.up && keys.shift && !keys.down);
        let input = flight_input(&keys, &[]);
        assert_eq!(input.forward, 1.0);
        assert!(input.boost);
        // Hover: forward flies along the view.
        let mut s = StudioFlight::at([0.0, 300.0, 0.0], 0.0, FlightModel::Hover);
        for _ in 0..60 {
            s.flight.step(&input, 1.0 / 30.0, &|_, _| Some(0.0), None);
        }
        assert!(s.flight.pos[2] < -1.0, "north is -z: {:?}", s.flight.pos);
    }
}

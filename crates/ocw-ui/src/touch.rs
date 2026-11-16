//! Touch steering in a ride for both apps (spec §3.2.3): Bevy's touches
//! (the browser's touch pointers through winit, the desktop's touch
//! screens on Windows / Linux) → the pure state machine
//! `ocw_client::input::touch::TouchSteer` → the ride's client (line or
//! lean), the junction fan's exit (a tap on its arrow), the chase camera
//! (two fingers: drag orbits, pinch sets the distance). The hints' presence
//! (`TouchPresence`) follows the last input device; the hints are drawn by
//! `ocw_render::touch_hud` from [`TouchHud`].
//!
//! Active only in the ride's input context (not while the menu, the
//! console or the workshop is open), not in flight (unicorn mode has its
//! own touch, `crate::god::touch`) and not with `controls.touch_steer
//! off`. A finger already down when the zones become active counts only
//! after it was lifted.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ocw_client::input::touch::{
    Fan, Pos, Rect, Side, TouchEvent, TouchFrame, TouchPresence, TouchSetting, TouchSteer,
    TouchZones, ZoneHeight,
};
use ocw_render::RideView;
use ocw_render::camera::CameraOrbit;
use ocw_render::menu::InputContext;
use ocw_render::touch_hud::TouchHud;

use crate::settings::UserSettings;

pub struct TouchSteerPlugin;

impl Plugin for TouchSteerPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<RideTouch>()
            .init_resource::<TouchHud>()
            .init_resource::<CameraOrbit>()
            .add_systems(Update, touch_steer.after(crate::RideViewSet));
    }
}

/// How long a tap's chevron flashes (s).
pub const FLASH_S: f64 = 0.15;
/// The first-ride tip.
pub const TIP: &str =
    "Tap or hold the left / right side to steer \u{2013} two fingers move the camera";

/// The ride's touch state (shared by both apps).
#[derive(Resource, Default)]
pub struct RideTouch {
    pub steer: TouchSteer,
    presence: Option<TouchPresence>,
    flash: Option<(Side, f64)>,
    /// The last frame's result and whether the zones were active.
    pub last: TouchFrame,
    pub active: bool,
    pub shown: bool,
    /// The last exit chosen by a tap on the fan (taken by the client) …
    pub last_exit: Option<usize>,
    /// … and the last exit arrow tapped.
    pub tapped_exit: Option<usize>,
}

/// The setting `controls.touch_steer` and the zones' height.
fn touch_settings(world: &World) -> (TouchSetting, ZoneHeight, bool) {
    let Some(s) = world.get_resource::<UserSettings>() else {
        return (TouchSetting::Auto, ZoneHeight::Full, false);
    };
    let word = |k| s.settings.get(k).map(|v| v.to_string()).unwrap_or_default();
    (
        TouchSetting::parse(&word("controls.touch_steer")),
        ZoneHeight::parse(&word("controls.touch_zones")),
        word("controls.touch_tip_seen") == "on",
    )
}

/// The window's size (logical px) and the HUD's scale.
fn window(world: &mut World) -> (Vec2, f32) {
    let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
    let size = q.single(world).map_or(Vec2::new(1280.0, 720.0), |w| {
        Vec2::new(w.width(), w.height())
    });
    let scale = world.get_resource::<UiScale>().map_or(1.0, |s| s.0);
    (size, scale)
}

fn pos(v: Vec2) -> Pos {
    Pos::new(v.x, v.y)
}

/// The frame's touch events from Bevy's [`Touches`].
fn touch_events(t: &Touches) -> Vec<TouchEvent> {
    let mut out: Vec<TouchEvent> = t
        .iter_just_pressed()
        .map(|f| TouchEvent::Start {
            id: f.id(),
            pos: pos(f.position()),
        })
        .collect();
    out.extend(t.iter().map(|f| TouchEvent::Move {
        id: f.id(),
        pos: pos(f.position()),
    }));
    out.extend(t.iter_just_released().map(|f| TouchEvent::End {
        id: f.id(),
        pos: pos(f.position()),
    }));
    out.extend(
        t.iter_just_canceled()
            .map(|f| TouchEvent::Cancel { id: f.id() }),
    );
    out
}

/// Whether another device than touch was used this frame (a key, a
/// click, a gamepad), and the mouse's move (px) – for the hints.
fn other_input(world: &mut World) -> (bool, bool, f32) {
    let key = world
        .get_resource::<ButtonInput<KeyCode>>()
        .is_some_and(|k| k.get_pressed().next().is_some() || k.get_just_pressed().next().is_some());
    let click = world
        .get_resource::<ButtonInput<MouseButton>>()
        .is_some_and(|b| b.get_just_pressed().next().is_some());
    let moved = world
        .get_resource::<AccumulatedMouseMotion>()
        .map_or(0.0, |m| m.delta.length());
    let mut q = world.query::<&Gamepad>();
    let pad = q.iter(world).any(|p| {
        p.get_just_pressed().next().is_some()
            || p.left_stick().length() > ocw_client::input::STICK_DEAD_ZONE * 2.0
            || p.right_stick().length() > ocw_client::input::STICK_DEAD_ZONE * 2.0
    });
    (key || pad, click, moved)
}

fn cursor(world: &mut World) -> Option<Pos> {
    let mut q = world.query_filtered::<&Window, With<PrimaryWindow>>();
    q.single(world).ok()?.cursor_position().map(pos)
}

/// The frame's touch steering (see the module docs).
fn touch_steer(world: &mut World) {
    let now = crate::now(world);
    let dt = world
        .get_resource::<Time>()
        .map_or(0.0, |t| t.delta_secs_f64());
    let (setting, zone_height, tip_seen) = touch_settings(world);
    let (size, ui_scale) = window(world);
    let events = world
        .get_resource::<Touches>()
        .map(touch_events)
        .unwrap_or_default();
    let first_touch = events.iter().find_map(|e| match e {
        TouchEvent::Start { pos, .. } => Some(*pos),
        _ => None,
    });
    let (other, click, moved) = other_input(world);
    let cursor = cursor(world);
    let ride_input = world
        .get_resource::<InputContext>()
        .is_none_or(|c| *c == InputContext::Ride);
    let active = crate::riding(world)
        && ride_input
        && crate::god::not_flying(world)
        && setting != TouchSetting::Off;
    // The zones, without the junction fan (its arrows are tap targets).
    let mut zones = TouchZones::new(size.x, size.y);
    zones.zone_height = zone_height;
    let junction = world
        .get_resource::<RideView>()
        .and_then(|v| v.junction.clone());
    let fan = junction.as_ref().map(|_| {
        let g = ocw_render::junction::fan_geometry(size, ui_scale);
        Fan {
            rect: Rect::new(g.min.x, g.min.y, g.max.x, g.max.y),
            pivot: pos(g.pivot),
            arrow_len: g.arrow_len,
        }
    });
    if let Some(f) = &fan {
        zones.excluded.push(f.rect);
    }
    let angles: Vec<f64> = junction
        .as_ref()
        .map(|j| j.exits.iter().map(|e| e.angle_deg).collect())
        .unwrap_or_default();
    // The state machine.
    let mut touch = std::mem::take(&mut *world.resource_mut::<RideTouch>());
    let presence = touch.presence.get_or_insert_with(|| {
        let facts = world
            .get_resource::<crate::quality::QualityContext>()
            .map(|q| q.device.touch_facts())
            .unwrap_or_default();
        TouchPresence::new(facts)
    });
    for e in events {
        touch.steer.event(e, now);
    }
    let frame = if active {
        let fan_arg = fan.as_ref().map(|f| (f, angles.as_slice()));
        match crate::client_mut(world) {
            Some(mut client) => touch.steer.update(&zones, fan_arg, &mut *client, dt),
            None => TouchFrame::default(),
        }
    } else {
        touch.steer.suspend();
        TouchFrame::default()
    };
    touch.tapped_exit = frame.exit.or(touch.tapped_exit);
    if let Some(i) = frame.exit
        && let Some(mut client) = crate::client_mut(world)
        && let Some(said) = client.choose_exit(i)
    {
        info!("touch: {said}");
        touch.last_exit = Some(i);
    }
    if frame.camera_drag != (0.0, 0.0) || frame.camera_zoom != 0.0 {
        world.resource_mut::<CameraOrbit>().apply(
            Vec2::new(frame.camera_drag.0, frame.camera_drag.1),
            frame.camera_zoom as f32,
        );
    }
    // The hints follow the last input device.
    if let Some(p) = first_touch {
        presence.touch(now, p);
    }
    if other {
        presence.key();
    }
    if click {
        presence.click(now, cursor);
    }
    if first_touch.is_none() && touch.steer.fingers() == 0 {
        presence.mouse_move(now, moved, cursor);
    }
    if let Some(side) = frame.tap {
        touch.flash = Some((side, now + FLASH_S));
    }
    let flash = touch
        .flash
        .filter(|(_, until)| now < *until)
        .map(|(s, _)| s);
    let shown = active && presence.shown(setting);
    let steered = frame.left_held || frame.right_held || frame.tap.is_some();
    if steered && !tip_seen {
        mark_tip_seen(world);
    }
    let hud = TouchHud {
        shown,
        dimmed: presence.dimmed(now) && setting != TouchSetting::On,
        left_held: frame.left_held && !frame.right_held,
        right_held: frame.right_held && !frame.left_held,
        flash_left: flash == Some(Side::Left),
        flash_right: flash == Some(Side::Right),
        tip: (shown && !tip_seen && !steered).then(|| TIP.to_string()),
        zone_top: zones.top() / size.y.max(1.0),
    };
    {
        let mut h = world.resource_mut::<TouchHud>();
        if *h != hud {
            *h = hud;
        }
    }
    touch.last = frame;
    touch.active = active;
    touch.shown = shown;
    *world.resource_mut::<RideTouch>() = touch;
}

/// The tip is shown once per device: the first touch steer ends it.
fn mark_tip_seen(world: &mut World) {
    crate::settings::save_setting(world, "controls.touch_tip_seen", "on");
}

/// The touch state as JSON for the browser test (`window.ocw.steer()`):
/// the line, the lean, autodrive, the zones and the hints.
pub fn state_json(world: &World) -> String {
    let c = crate::client(world);
    let t = world.get_resource::<RideTouch>();
    let hud = world.get_resource::<TouchHud>();
    let (setting, zones, tip_seen) = touch_settings(world);
    serde_json_lite(&[
        ("riding", c.is_some().to_string()),
        (
            "line_m",
            c.and_then(|c| c.line_target())
                .map_or("null".into(), |v| format!("{v:.4}")),
        ),
        (
            "lean_rad",
            c.and_then(|c| c.lean())
                .map_or("null".into(), |v| format!("{v:.4}")),
        ),
        ("autodrive", c.is_some_and(|c| c.autodrive()).to_string()),
        ("active", t.is_some_and(|t| t.active).to_string()),
        ("shown", t.is_some_and(|t| t.shown).to_string()),
        ("fingers", t.map_or(0, |t| t.steer.fingers()).to_string()),
        ("left_held", t.is_some_and(|t| t.last.left_held).to_string()),
        (
            "right_held",
            t.is_some_and(|t| t.last.right_held).to_string(),
        ),
        (
            "last_exit",
            t.and_then(|t| t.last_exit)
                .map_or("null".into(), |i| i.to_string()),
        ),
        ("tip", hud.is_some_and(|h| h.tip.is_some()).to_string()),
        ("tip_seen", tip_seen.to_string()),
        (
            "setting",
            format!(
                "\"{}\"",
                match setting {
                    TouchSetting::Auto => "auto",
                    TouchSetting::On => "on",
                    TouchSetting::Off => "off",
                }
            ),
        ),
        (
            "zones",
            format!(
                "\"{}\"",
                match zones {
                    ZoneHeight::Full => "full",
                    ZoneHeight::Lower => "lower",
                }
            ),
        ),
    ])
}

/// A flat JSON object from already-encoded values.
fn serde_json_lite(fields: &[(&str, String)]) -> String {
    let body: Vec<String> = fields
        .iter()
        .map(|(k, v)| format!("\"{k}\": {v}"))
        .collect();
    format!("{{{}}}", body.join(", "))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::input::touch::{TouchInput, TouchPhase};
    use bevy::time::TimeUpdateStrategy;
    use ocw_client::client::ClientCore;
    use ocw_client::input::touch::{TAP_LEAN_STEP_RAD, TAP_LINE_STEP_M};
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_server::local::FrameServer;
    use ocw_world::Route;

    use super::*;

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

    /// A ride on the in-memory server with the shared touch steering, at
    /// 50 frames a second.
    fn app() -> App {
        let route = Route::test_loop();
        let core = ClientCore::new(route.clone(), "Ada Rider", Default::default());
        let mut server = FrameServer::new(route.clone(), 0, 0.0);
        server.send(&core.hello());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, ConsoleCorePlugin))
            .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_millis(
                20,
            )))
            .insert_resource(RideView::new(route))
            .insert_resource(Ride {
                client: core,
                server,
            })
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
            .add_plugins((crate::settings::SettingsPlugin, TouchSteerPlugin));
        for _ in 0..200 {
            app.update();
            if client(&app).rider_id().is_some() {
                break;
            }
        }
        assert!(client(&app).rider_id().is_some(), "joined");
        app.world_mut().resource_mut::<Ride>().client.steer_to(0.0);
        app
    }

    fn client(app: &App) -> &ClientCore {
        &app.world().resource::<Ride>().client
    }

    fn touch(app: &mut App, id: u64, phase: TouchPhase, x: f32, y: f32) {
        let window = Entity::PLACEHOLDER;
        app.world_mut().write_message(TouchInput {
            phase,
            position: Vec2::new(x, y),
            window,
            force: None,
            id,
        });
    }

    /// Holds a finger at (x, y) for `frames` frames (20 ms each).
    fn hold(app: &mut App, id: u64, x: f32, y: f32, frames: usize) {
        touch(app, id, TouchPhase::Started, x, y);
        for _ in 0..frames {
            app.update();
        }
        touch(app, id, TouchPhase::Ended, x, y);
        app.update();
    }

    fn line(app: &App) -> f64 {
        client(app).line_target().unwrap_or(0.0)
    }

    #[test]
    fn a_tap_and_a_hold_steer_the_line_in_autodrive() {
        let mut app = app();
        assert!(client(&app).autodrive());
        let l0 = line(&app);
        // A tap on the right half (one frame): +0.5 m.
        hold(&mut app, 1, 1000.0, 500.0, 1);
        assert!(
            (line(&app) - l0 - TAP_LINE_STEP_M).abs() < 1e-6,
            "{}",
            line(&app)
        );
        assert!(client(&app).autodrive(), "a touch never ends autodrive");
        // Held for 0.5 s on the left: 2 m/s × 0.5 s back (as the key).
        let l1 = line(&app);
        hold(&mut app, 2, 200.0, 500.0, 25);
        assert!((l1 - line(&app) - 1.0).abs() < 1e-6, "{}", line(&app));
        // The hints showed after the touch (the desktop: no facts).
        assert!(
            app.world()
                .resource::<RideTouch>()
                .presence
                .unwrap()
                .shown(TouchSetting::Auto)
        );
        let json = state_json(app.world());
        assert!(json.contains("\"autodrive\": true"), "{json}");
        assert!(json.contains("\"tip_seen\": true"), "{json}");
    }

    #[test]
    fn a_tap_leans_one_step_with_manual_steering() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<Ride>()
            .client
            .set_autodrive(false);
        hold(&mut app, 1, 1000.0, 500.0, 1);
        let lean = client(&app).lean().unwrap();
        assert!((lean - TAP_LEAN_STEP_RAD).abs() < 1e-9, "{lean}");
        // The lean stays.
        for _ in 0..10 {
            app.update();
        }
        assert_eq!(client(&app).lean(), Some(lean));
    }

    #[test]
    fn nothing_steers_with_the_setting_off_or_in_the_menu() {
        let mut app = app();
        crate::settings::save_setting(app.world_mut(), "controls.touch_steer", "off");
        let l0 = line(&app);
        hold(&mut app, 1, 1000.0, 500.0, 5);
        assert_eq!(line(&app), l0);
        assert!(!app.world().resource::<TouchHud>().shown);
        crate::settings::save_setting(app.world_mut(), "controls.touch_steer", "on");
        app.insert_resource(InputContext::Menu);
        // Down while the menu is open, still down after it closed:
        // ignored until lifted.
        touch(&mut app, 2, TouchPhase::Started, 1000.0, 500.0);
        app.update();
        app.insert_resource(InputContext::Ride);
        for _ in 0..5 {
            app.update();
        }
        touch(&mut app, 2, TouchPhase::Ended, 1000.0, 500.0);
        app.update();
        assert_eq!(line(&app), l0);
        assert!(app.world().resource::<TouchHud>().shown, "on: the hints");
    }

    #[test]
    fn two_fingers_orbit_the_camera_and_pinch_zooms() {
        let mut app = app();
        let l0 = line(&app);
        let o0 = *app.world().resource::<CameraOrbit>();
        touch(&mut app, 1, TouchPhase::Started, 300.0, 400.0);
        touch(&mut app, 2, TouchPhase::Started, 900.0, 400.0);
        app.update();
        // Dragged together 40 px to the right, then spread apart.
        touch(&mut app, 1, TouchPhase::Moved, 340.0, 400.0);
        touch(&mut app, 2, TouchPhase::Moved, 940.0, 400.0);
        app.update();
        let o1 = *app.world().resource::<CameraOrbit>();
        assert!(o1.yaw != o0.yaw, "orbited");
        touch(&mut app, 1, TouchPhase::Moved, 240.0, 400.0);
        touch(&mut app, 2, TouchPhase::Moved, 1040.0, 400.0);
        app.update();
        let o2 = *app.world().resource::<CameraOrbit>();
        assert!(o2.distance < o1.distance, "spread: closer");
        touch(&mut app, 1, TouchPhase::Ended, 240.0, 400.0);
        touch(&mut app, 2, TouchPhase::Ended, 1040.0, 400.0);
        app.update();
        assert_eq!(line(&app), l0, "left + right: no steering");
    }

    #[test]
    fn a_tap_on_a_fan_arrow_chooses_that_exit() {
        let mut app = app();
        app.world_mut().resource_mut::<RideView>().junction = Some(ocw_render::JunctionView {
            distance_m: 80.0,
            exits: [-90.0, 0.0, 90.0]
                .into_iter()
                .map(|a| ocw_render::ExitView {
                    angle_deg: a,
                    name: "Test road".into(),
                    dead_end: false,
                    unpaved: false,
                    steep: false,
                })
                .collect(),
            selected: 1,
            default: 1,
            locked: false,
            route: None,
        });
        // The right arrow (the test route has no junction: the client
        // says no, but the tap is taken by the fan and never steers).
        let g = ocw_render::junction::fan_geometry(Vec2::new(1280.0, 720.0), 1.0);
        let l0 = line(&app);
        hold(&mut app, 1, g.pivot.x + 60.0, g.pivot.y - 2.0, 1);
        assert_eq!(line(&app), l0);
        let t = app.world().resource::<RideTouch>();
        assert_eq!((t.tapped_exit, t.last.tap), (Some(2), None));
    }
}

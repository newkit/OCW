//! Touch steering in a ride for both apps (spec §3.2.3): Bevy's touches
//! (the browser's touch pointers through winit, the desktop's touch
//! screens on Windows / Linux) → the pure state machine
//! `ocw_client::input::touch::TouchSteer` → the ride's client (line or
//! lean), the junction fan's exit (a tap on its arrow), the chase camera
//! (two fingers: drag orbits, pinch sets the distance), and the on-screen
//! controls of spec 3.2.3 *The on-screen controls*:
//!
//! - the **menu button** (☰, top left): a tap opens the menu, as Esc /
//!   gamepad Start, and closes it again;
//! - the **hide button** (the eye beside it): hides / shows the chevrons,
//!   the zone marks and the tips (`controls.touch_controls`, saved) –
//!   hidden controls are not drawn, every touch area still works; the
//!   first time a note says so;
//! - the **faster / slower zones** (they replaced the + / − / stop
//!   buttons): the top quarter of the screen is ↑, the bottom quarter
//!   takes power away, while the power is the virtual trainer's
//!   (`ocw_client::input::power_zones`; [`TouchPower`], read by both
//!   apps' power keys). Steering keeps the half between them.
//!
//! The hints' presence (`TouchPresence`) follows the last input device;
//! everything is drawn by `ocw_render::touch_hud` from [`TouchHud`].
//!
//! Steering and the zones are active only in the ride's input context
//! (not while the menu, the console or the workshop is open), not in
//! flight (unicorn mode has its own touch, `crate::god::touch`) and not
//! with `controls.touch_steer off`; the zones also not while the page
//! shows a panel or a text field over the ride ([`TouchBlocked`]). A
//! finger already down when the zones become active counts only after it
//! was lifted.

use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::input::touch::Touches;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use ocw_client::input::ThrottleKeys;
use ocw_client::input::power_zones::{PowerDir, PowerTouch, PowerZones};
use ocw_client::input::touch::{
    Fan, Insets, Pos, Rect, Side, TouchEvent, TouchFrame, TouchPresence, TouchSetting, TouchSteer,
    TouchZones, ZoneHeight,
};
use ocw_render::RideView;
use ocw_render::camera::CameraOrbit;
use ocw_render::menu::InputContext;
use ocw_render::safe_area::SafeArea;
use ocw_render::touch_hud::{CornerButton, PowerEdge, TouchHud, corner_buttons};

use crate::settings::UserSettings;

pub struct TouchSteerPlugin;

impl Plugin for TouchSteerPlugin {
    fn build(&self, app: &mut App) {
        crate::configure_sets(app);
        app.init_resource::<RideTouch>()
            .init_resource::<TouchHud>()
            .init_resource::<TouchPower>()
            .init_resource::<TouchBlocked>()
            .init_resource::<CameraOrbit>()
            .add_systems(Update, touch_steer.after(crate::RideViewSet));
    }
}

/// How long a tap's chevron flashes (s).
pub const FLASH_S: f64 = 0.15;
/// How long a faster / slower step's feedback shows (s).
pub const POWER_FLASH_S: f64 = 0.9;
/// How long the note of the hide button shows (s).
pub const TOAST_S: f64 = 4.0;
/// The first-ride tip (the second line while the power is the virtual
/// trainer's).
pub const TIP: &str =
    "Tap or hold the left / right side to steer \u{2013} two fingers move the camera";
pub const TIP_POWER: &str = "Top of the screen: faster \u{2013} bottom: slower";
/// The note when the controls are hidden the first time.
pub const HIDDEN_NOTE: &str = "Controls hidden \u{2013} they still work";

/// The page shows something over the ride that takes the touches or the
/// rider's attention (the browser's page log, its Devices panel, a text
/// field with the on-screen keyboard): the faster / slower zones are off
/// meanwhile. Set by the app.
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TouchBlocked(pub bool);

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
    /// The faster / slower zones' finger.
    pub power_touch: PowerTouch,
    /// The zones were on in the last frame, and where the top zone ends
    /// and the bottom zone starts (window px, logical).
    pub power: bool,
    pub power_band: Option<(f32, f32)>,
    /// The last step's feedback: the edge, where across the window (px)
    /// and until when.
    power_flash: Option<(PowerEdge, f32, f64)>,
    /// The menu and hide buttons showed in the last frame (their taps
    /// count) …
    pub buttons: bool,
    /// … where they are (window px, logical) …
    pub buttons_at: Vec<(CornerButton, Rect)>,
    /// … and the fingers that went down on one.
    button_fingers: Vec<(u64, CornerButton)>,
    /// The hide button's note, until when.
    toast: Option<f64>,
    /// The menu button's taps so far (each opened or closed the menu).
    pub menu_taps: u32,
}

/// The faster / slower zones' keys (spec 3.2.3): a tap at the top is ↑
/// pressed (25 W), held there ↑ held (the ramp); a tap at the bottom
/// takes 10 W (and stops once none is left), held there the power ramps
/// down. Taken by the app's power keys each frame ([`TouchPower::take`])
/// and fed with the keyboard's into the shared `KeyThrottle`.
#[derive(Resource, Debug, Default)]
pub struct TouchPower {
    /// Presses since the app last took them.
    pending: ThrottleKeys,
    /// The zone held now.
    pub held: Option<PowerDir>,
}

impl TouchPower {
    /// One step of `dir` (a tap, or the start of a hold at the top).
    pub fn step(&mut self, dir: PowerDir) {
        match dir {
            PowerDir::Faster => self.pending.up_pressed = true,
            PowerDir::Slower => self.pending.slower_pressed = true,
        }
    }

    /// The keys of this frame: the steps since the last call, and the
    /// ramp of the zone a finger stays on.
    pub fn take(&mut self) -> ThrottleKeys {
        let mut k = std::mem::take(&mut self.pending);
        k.up_held |= self.held == Some(PowerDir::Faster);
        k.slower_held |= self.held == Some(PowerDir::Slower);
        k
    }
}

/// The menu and hide buttons' rectangles in window px (logical), from the
/// shared layout in UI px.
fn button_rects(
    ui_off: Vec2,
    ui_size: Vec2,
    ui_scale: f32,
    boxes: Option<&ocw_render::hud::HudBoxes>,
) -> Vec<(CornerButton, Rect)> {
    let k = ui_scale.max(0.1);
    let avoid = boxes
        .map(|b| ocw_render::touch_hud::top_boxes(b, ui_off, k))
        .unwrap_or_default();
    corner_buttons(ui_size / k, k, &avoid)
        .into_iter()
        .map(|(b, r)| {
            let (min, max) = (r.min * k + ui_off, r.max * k + ui_off);
            (b, Rect::new(min.x, min.y, max.x, max.y))
        })
        .collect()
}

/// The touch settings of this frame.
struct TouchSettings {
    steer: TouchSetting,
    zones: ZoneHeight,
    tip_seen: bool,
    /// `controls.touch_controls hidden`.
    hidden: bool,
    hidden_hint_seen: bool,
}

fn touch_settings(world: &World) -> TouchSettings {
    let Some(s) = world.get_resource::<UserSettings>() else {
        return TouchSettings {
            steer: TouchSetting::Auto,
            zones: ZoneHeight::Full,
            tip_seen: false,
            hidden: false,
            hidden_hint_seen: false,
        };
    };
    let word = |k| s.settings.get(k).map(|v| v.to_string()).unwrap_or_default();
    TouchSettings {
        steer: TouchSetting::parse(&word("controls.touch_steer")),
        zones: ZoneHeight::parse(&word("controls.touch_zones")),
        tip_seen: word("controls.touch_tip_seen") == "on",
        hidden: word("controls.touch_controls") == "hidden",
        hidden_hint_seen: word("controls.touch_hidden_hint_seen") == "on",
    }
}

/// Opens the menu, or closes it (the menu button; as Esc / Start).
fn toggle_menu(world: &mut World) {
    if let Some(mut m) = world.get_resource_mut::<crate::menu::MenuState>() {
        let action = if m.menu.is_open() {
            ocw_client::menu::model::Action::Close
        } else {
            ocw_client::menu::model::Action::Open
        };
        m.queue(action);
    }
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
    let TouchSettings {
        steer: setting,
        zones: zone_height,
        tip_seen,
        mut hidden,
        hidden_hint_seen,
    } = touch_settings(world);
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
    let context = world.get_resource::<InputContext>().copied();
    let ride_input = context.is_none_or(|c| c == InputContext::Ride);
    let menu_open = context == Some(InputContext::Menu);
    let active = crate::riding(world)
        && ride_input
        && crate::god::not_flying(world)
        && setting != TouchSetting::Off;
    // The safe area (spec §3.2.3: notch, home bar; the page's
    // `env(safe-area-inset-*)`): the zones keep out of it, and the HUD –
    // the fan too – is laid out inside it.
    let safe = world
        .get_resource::<SafeArea>()
        .copied()
        .unwrap_or_default()
        .usable(size);
    let (ui_off, ui_size) = safe.inner(size);
    // The zones, without the junction fan (its arrows are tap targets).
    let mut zones = TouchZones::new(size.x, size.y);
    zones.zone_height = zone_height;
    zones.insets = Insets {
        top: safe.top,
        right: safe.right,
        bottom: safe.bottom,
        left: safe.left,
    };
    let junction = world
        .get_resource::<RideView>()
        .and_then(|v| v.junction.clone());
    let fan = junction.as_ref().map(|_| {
        let g = ocw_render::junction::fan_geometry(ui_size, ui_scale);
        let (min, max) = (g.min + ui_off, g.max + ui_off);
        Fan {
            rect: Rect::new(min.x, min.y, max.x, max.y),
            pivot: pos(g.pivot + ui_off),
            arrow_len: g.arrow_len,
        }
    });
    if let Some(f) = &fan {
        zones.excluded.push(f.rect);
    }
    // The menu and hide buttons are tap targets, not steering: where
    // they were drawn in the last frame.
    let buttons_on = world.resource::<RideTouch>().buttons;
    let boxes = world.get_resource::<ocw_render::hud::HudBoxes>().cloned();
    let buttons_at = if buttons_on {
        button_rects(ui_off, ui_size, ui_scale, boxes.as_ref())
    } else {
        Vec::new()
    };
    zones.excluded.extend(buttons_at.iter().map(|(_, r)| *r));
    // The faster / slower zones (the top and bottom quarter of the safe
    // area) while the power is the virtual trainer's and the page shows
    // nothing over the ride; steering keeps the half between them.
    let virtual_power = crate::client(world).is_some_and(|c| c.virtual_device());
    let blocked = world.get_resource::<TouchBlocked>().is_some_and(|b| b.0);
    let power_zones = (active && virtual_power && !blocked).then(|| PowerZones {
        width: size.x,
        height: size.y,
        insets: zones.insets,
        excluded: zones.excluded.clone(),
    });
    if let Some(z) = &power_zones {
        zones.band = Some(z.band());
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
    let mut power = std::mem::take(&mut *world.resource_mut::<TouchPower>());
    // The menu and hide buttons: a finger that goes down and lifts on
    // one. The menu button also over the open menu (it closes it).
    let mut pressed: Vec<CornerButton> = Vec::new();
    for e in &events {
        match *e {
            TouchEvent::Start { id, pos } if ride_input || menu_open => {
                if let Some((b, _)) = buttons_at.iter().find(|(_, r)| r.contains(pos))
                    && (ride_input || *b == CornerButton::Menu)
                {
                    touch.button_fingers.push((id, *b));
                }
            }
            TouchEvent::End { id, pos } => {
                if let Some(i) = touch.button_fingers.iter().position(|(f, _)| *f == id) {
                    let (_, b) = touch.button_fingers.remove(i);
                    if buttons_at.iter().any(|(k, r)| *k == b && r.contains(pos)) {
                        pressed.push(b);
                    }
                }
            }
            TouchEvent::Cancel { id } => {
                touch.button_fingers.retain(|(f, _)| *f != id);
            }
            _ => {}
        }
    }
    if !(ride_input || menu_open) {
        touch.button_fingers.clear();
    }
    for b in pressed {
        match b {
            CornerButton::Menu => {
                toggle_menu(world);
                touch.menu_taps += 1;
            }
            CornerButton::Hide => {
                hidden = !hidden;
                let word = if hidden { "hidden" } else { "shown" };
                crate::settings::save_setting(world, "controls.touch_controls", word);
                // The first time: say that the touch areas still work.
                touch.toast = (hidden && !hidden_hint_seen).then_some(now + TOAST_S);
                if hidden && !hidden_hint_seen {
                    crate::settings::save_setting(world, "controls.touch_hidden_hint_seen", "on");
                }
                info!("touch controls {word}");
            }
        }
    }
    // The faster / slower zones.
    for e in &events {
        touch.power_touch.event(*e, now);
    }
    let power_frame = touch.power_touch.update(power_zones.as_ref(), now);
    // A tap is a step; a hold at the top starts with one (↑ pressed,
    // then held), a hold at the bottom only ramps down – it never stops.
    let hold_step = power_frame.hold_start.filter(|d| *d == PowerDir::Faster);
    for dir in [power_frame.tap, hold_step].into_iter().flatten() {
        power.step(dir);
    }
    power.held = power_frame.held;
    let edge = |d: PowerDir| match d {
        PowerDir::Faster => PowerEdge::Top,
        PowerDir::Slower => PowerEdge::Bottom,
    };
    if let (Some(dir), Some(at)) = (power_frame.tap.or(power_frame.held), power_frame.at) {
        touch.power_flash = Some((edge(dir), at.x, now + POWER_FLASH_S));
    }
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
    // The menu and hide buttons: on a touch device (or after a touch),
    // in the ride – in flight and with the steering *off* too – and the
    // menu button over the open menu.
    let touch_device = presence.shown(if setting == TouchSetting::Off {
        TouchSetting::Auto
    } else {
        setting
    });
    let buttons = crate::riding(world) && (ride_input || menu_open) && touch_device;
    let steered = frame.left_held || frame.right_held || frame.tap.is_some();
    if steered && !tip_seen {
        mark_tip_seen(world);
    }
    // The step's feedback: the power after it ("▲ 220 W"), or the stop.
    touch.power_flash = touch
        .power_flash
        .filter(|(_, _, until)| now < *until && power_zones.is_some());
    let power_flash = touch.power_flash.map(|(e, x, _)| {
        let c = crate::client(world);
        let text = if e == PowerEdge::Bottom && c.is_some_and(|c| c.braking()) {
            format!("{} stop", e.arrow())
        } else {
            format!("{} {:.0} W", e.arrow(), c.map_or(0.0, |c| c.power_w()))
        };
        (
            e,
            text,
            ((x - ui_off.x) / ui_size.x.max(1.0)).clamp(0.0, 1.0),
        )
    });
    touch.toast = touch.toast.filter(|until| now < *until && hidden);
    let share = |y: f32| ((y - ui_off.y) / ui_size.y.max(1.0)).clamp(0.0, 1.0);
    let power_band = power_zones.as_ref().map(|z| z.band());
    let tip = if power_zones.is_some() {
        format!("{TIP}\n{TIP_POWER}")
    } else {
        TIP.to_string()
    };
    let hud = TouchHud {
        shown,
        hidden,
        dimmed: presence.dimmed(now) && setting != TouchSetting::On,
        left_held: frame.left_held && !frame.right_held,
        right_held: frame.right_held && !frame.left_held,
        flash_left: flash == Some(Side::Left),
        flash_right: flash == Some(Side::Right),
        tip: (shown && !tip_seen && !steered).then_some(tip),
        // Shares of the HUD's (the safe area's) height.
        zone_top: share(zones.top()),
        zone_bottom: share(zones.bottom()),
        buttons,
        menu_open,
        button_held: touch.button_fingers.last().map(|(_, b)| *b),
        power: power_band.map(|(a, b)| (share(a), share(b))),
        power_held: power.held.map(edge),
        power_flash,
        toast: touch.toast.map(|_| HIDDEN_NOTE.to_string()),
    };
    touch.power = power_zones.is_some();
    touch.power_band = power_band;
    touch.buttons = buttons;
    touch.buttons_at = if buttons {
        button_rects(ui_off, ui_size, ui_scale, boxes.as_ref())
    } else {
        Vec::new()
    };
    *world.resource_mut::<TouchPower>() = power;
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

/// A text field of the menu is being typed into (Ride → Join's fields,
/// the ANT+ network key): on a touch device the browser's page shows a
/// text bar that raises the on-screen keyboard and sends its keys here.
pub fn typing(world: &World) -> bool {
    world
        .get_resource::<crate::join::JoinUi>()
        .is_some_and(|u| u.info.typing.is_some())
        || world
            .get_resource::<crate::key_entry::KeyEntry>()
            .is_some_and(|k| k.typing)
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
    let TouchSettings {
        steer: setting,
        zones,
        tip_seen,
        hidden,
        ..
    } = touch_settings(world);
    let rect = |r: &Rect| {
        format!(
            "[{:.1}, {:.1}, {:.1}, {:.1}]",
            r.min.x, r.min.y, r.max.x, r.max.y
        )
    };
    serde_json_lite(&[
        ("riding", c.is_some().to_string()),
        // A text field of the menu is typed into (the page offers its
        // on-screen keyboard bar on a touch device).
        ("typing", typing(world).to_string()),
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
        // The faster / slower zones (the top / bottom of the screen; no
        // power buttons since v7.09): on, where the top zone ends and
        // the bottom zone starts (logical px from the top), the zone
        // held, the feedback's text.
        ("power", t.is_some_and(|t| t.power).to_string()),
        (
            "power_zones",
            t.and_then(|t| t.power_band)
                .map_or("null".into(), |(a, b)| format!("[{a:.1}, {b:.1}]")),
        ),
        (
            "power_held",
            hud.and_then(|h| h.power_held).map_or("null".into(), |e| {
                match e {
                    PowerEdge::Top => "\"faster\"",
                    PowerEdge::Bottom => "\"slower\"",
                }
                .into()
            }),
        ),
        (
            "power_flash",
            hud.and_then(|h| h.power_flash.as_ref())
                .map_or("null".into(), |(_, text, _)| format!("\"{text}\"")),
        ),
        ("power_buttons", "[]".into()),
        ("braking", c.is_some_and(|c| c.braking()).to_string()),
        (
            "power_w",
            c.map_or("null".into(), |c| format!("{:.1}", c.power_w())),
        ),
        // The menu and hide buttons ([x0, y0, x1, y1], logical px: ☰,
        // then the eye), whether the controls are hidden, the note, the
        // menu button's taps.
        ("buttons", t.is_some_and(|t| t.buttons).to_string()),
        (
            "menu_button",
            t.and_then(|t| t.buttons_at.first())
                .map_or("null".into(), |(_, r)| rect(r)),
        ),
        (
            "hide_button",
            t.and_then(|t| t.buttons_at.get(1))
                .map_or("null".into(), |(_, r)| rect(r)),
        ),
        ("controls_hidden", hidden.to_string()),
        ("hints_drawn", hud.is_some_and(|h| h.hints()).to_string()),
        (
            "toast",
            hud.and_then(|h| h.toast.as_ref())
                .map_or("null".into(), |t| format!("\"{t}\"")),
        ),
        ("menu_taps", t.map_or(0, |t| t.menu_taps).to_string()),
        (
            "key_power_w",
            c.map_or("null".into(), |c| format!("{:.1}", c.key_power_w())),
        ),
        // The safe-area insets the zones and the HUD keep clear of
        // (top, right, bottom, left; logical px).
        ("insets", {
            let i = world
                .get_resource::<SafeArea>()
                .copied()
                .unwrap_or_default();
            format!("[{}, {}, {}, {}]", i.top, i.right, i.bottom, i.left)
        }),
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
    fn the_safe_area_keeps_the_zones_off_the_notch() {
        let mut app = app();
        // A notch on the left and a home bar (logical px), as the page
        // passes them.
        app.insert_resource(SafeArea {
            top: 0.0,
            right: 59.0,
            bottom: 21.0,
            left: 59.0,
        });
        let l0 = line(&app);
        // Behind the notch and on the home bar: nothing.
        hold(&mut app, 1, 40.0, 500.0, 1);
        hold(&mut app, 2, 1240.0, 500.0, 1);
        hold(&mut app, 3, 200.0, 710.0, 1);
        assert_eq!(line(&app), l0);
        // Inside the safe area each half steers.
        hold(&mut app, 4, 1000.0, 500.0, 1);
        assert!((line(&app) - l0 - TAP_LINE_STEP_M).abs() < 1e-6);
        hold(&mut app, 5, 200.0, 500.0, 1);
        assert!((line(&app) - l0).abs() < 1e-6);
        let json = state_json(app.world());
        assert!(json.contains("\"insets\": [0, 59, 21, 59]"), "{json}");
    }

    fn take(app: &mut App) -> ThrottleKeys {
        app.world_mut().resource_mut::<TouchPower>().take()
    }

    /// A ride on the virtual trainer with the hints on (a phone).
    fn virtual_app() -> App {
        let mut app = app();
        crate::settings::save_setting(app.world_mut(), "controls.touch_steer", "on");
        app.world_mut()
            .resource_mut::<Ride>()
            .client
            .set_virtual_device(true);
        app.update();
        app
    }

    /// The faster / slower zones (they replaced the + / − / stop
    /// buttons): on while the power is the virtual trainer's; a tap in
    /// the top quarter is ↑ pressed when it lifts, held there ↑ pressed
    /// and held (the ramp); a tap in the bottom quarter is one step
    /// slower, held there the ramp down – none of them steers, and the
    /// half between still steers.
    #[test]
    fn the_top_of_the_screen_is_faster_and_the_bottom_slower() {
        let mut app = app();
        crate::settings::save_setting(app.world_mut(), "controls.touch_steer", "on");
        app.update();
        assert!(app.world().resource::<TouchHud>().shown);
        assert_eq!(
            app.world().resource::<TouchHud>().power,
            None,
            "not with a power device"
        );
        // With a power device the halves take the height: the bottom
        // steers, the top band does nothing.
        let l0 = line(&app);
        hold(&mut app, 1, 1000.0, 650.0, 1);
        assert!((line(&app) - l0 - TAP_LINE_STEP_M).abs() < 1e-6);
        hold(&mut app, 2, 200.0, 650.0, 1);
        hold(&mut app, 3, 640.0, 60.0, 1);
        assert_eq!(take(&mut app), ThrottleKeys::default());
        app.world_mut()
            .resource_mut::<Ride>()
            .client
            .set_virtual_device(true);
        app.update();
        // 1280 × 720: the top zone ends at 180, the bottom starts at 540.
        assert_eq!(app.world().resource::<TouchHud>().power, Some((0.25, 0.75)));
        assert_eq!(
            app.world().resource::<RideTouch>().power_band,
            Some((180.0, 540.0))
        );
        let l0 = line(&app);
        take(&mut app);
        // A tap at the top: one step, when the finger lifts.
        touch(&mut app, 1, TouchPhase::Started, 900.0, 100.0);
        app.update();
        assert_eq!(take(&mut app), ThrottleKeys::default(), "not on touch-down");
        touch(&mut app, 1, TouchPhase::Ended, 900.0, 100.0);
        app.update();
        let k = take(&mut app);
        assert!(k.up_pressed && !k.up_held && !k.slower_pressed, "{k:?}");
        // The feedback: at the top edge, with the power.
        let hud = app.world().resource::<TouchHud>().clone();
        let (edge, text, x) = hud.power_flash.expect("the feedback");
        assert_eq!(edge, PowerEdge::Top);
        assert!(
            text.starts_with('\u{25b2}') && text.ends_with(" W"),
            "{text}"
        );
        assert!((x - 900.0 / 1280.0).abs() < 1e-3);
        // It fades.
        for _ in 0..60 {
            app.update();
        }
        assert_eq!(app.world().resource::<TouchHud>().power_flash, None);
        // Held at the top (20 ms frames): after 250 ms ↑ pressed once,
        // then held – the ramp – until the finger lifts.
        take(&mut app);
        touch(&mut app, 2, TouchPhase::Started, 300.0, 100.0);
        for _ in 0..10 {
            app.update();
        }
        assert_eq!(take(&mut app), ThrottleKeys::default(), "not yet a hold");
        for _ in 0..5 {
            app.update();
        }
        let k = take(&mut app);
        assert!(k.up_pressed && k.up_held, "{k:?}");
        assert_eq!(
            app.world().resource::<TouchHud>().power_held,
            Some(PowerEdge::Top)
        );
        app.update();
        let k = take(&mut app);
        assert!(!k.up_pressed && k.up_held, "held: the ramp, {k:?}");
        touch(&mut app, 2, TouchPhase::Ended, 300.0, 100.0);
        app.update();
        assert_eq!(take(&mut app), ThrottleKeys::default());
        assert_eq!(app.world().resource::<TouchHud>().power_held, None);
        // A tap at the bottom: one step slower; held: the ramp down.
        hold(&mut app, 3, 640.0, 650.0, 2);
        let k = take(&mut app);
        assert!(k.slower_pressed && !k.slower_held && !k.up_pressed, "{k:?}");
        let (edge, text, _) = app
            .world()
            .resource::<TouchHud>()
            .power_flash
            .clone()
            .expect("the feedback");
        assert_eq!(edge, PowerEdge::Bottom);
        assert!(text.starts_with('\u{25bc}'), "{text}");
        touch(&mut app, 4, TouchPhase::Started, 640.0, 650.0);
        for _ in 0..15 {
            app.update();
        }
        let k = take(&mut app);
        assert!(
            k.slower_held && !k.slower_pressed && !k.down_pressed,
            "{k:?}"
        );
        touch(&mut app, 4, TouchPhase::Ended, 640.0, 650.0);
        app.update();
        take(&mut app);
        assert_eq!(line(&app), l0, "no zone steered");
        // The half between still steers (and changes no power).
        hold(&mut app, 5, 1000.0, 400.0, 1);
        assert!((line(&app) - l0 - TAP_LINE_STEP_M).abs() < 1e-6);
        hold(&mut app, 6, 200.0, 400.0, 1);
        assert_eq!(take(&mut app), ThrottleKeys::default());
        // … but no longer above or below it.
        hold(&mut app, 7, 1000.0, 600.0, 1);
        assert!(
            (line(&app) - l0).abs() < 1e-6,
            "the bottom zone is not steering"
        );
        take(&mut app);
        let json = state_json(app.world());
        assert!(json.contains("\"power\": true"), "{json}");
        assert!(json.contains("\"power_zones\": [180.0, 540.0]"), "{json}");
        assert!(json.contains("\"power_buttons\": []"), "{json}");
    }

    /// The zones never fire by accident: not for a drag or a pinch, not
    /// in the safe-area insets, not while the menu is open or the page
    /// shows a panel or a text field, not with the steering *off*.
    #[test]
    fn the_zones_ignore_gestures_insets_the_menu_and_page_panels() {
        let mut app = virtual_app();
        take(&mut app);
        // A drag across the top (a scroll attempt).
        touch(&mut app, 1, TouchPhase::Started, 600.0, 100.0);
        app.update();
        touch(&mut app, 1, TouchPhase::Moved, 660.0, 100.0);
        for _ in 0..20 {
            app.update();
        }
        touch(&mut app, 1, TouchPhase::Ended, 660.0, 100.0);
        app.update();
        // A pinch with one finger in each zone, and the two-finger camera.
        let o0 = *app.world().resource::<CameraOrbit>();
        touch(&mut app, 2, TouchPhase::Started, 400.0, 100.0);
        touch(&mut app, 3, TouchPhase::Started, 800.0, 650.0);
        app.update();
        touch(&mut app, 2, TouchPhase::Moved, 440.0, 100.0);
        touch(&mut app, 3, TouchPhase::Moved, 840.0, 650.0);
        for _ in 0..20 {
            app.update();
        }
        assert!(
            app.world().resource::<CameraOrbit>().yaw != o0.yaw,
            "two fingers still move the camera"
        );
        touch(&mut app, 2, TouchPhase::Ended, 440.0, 100.0);
        touch(&mut app, 3, TouchPhase::Ended, 840.0, 650.0);
        app.update();
        assert_eq!(take(&mut app), ThrottleKeys::default(), "gestures: no step");
        // The notch and the home bar (a phone in portrait would have
        // them at the top and bottom: here 59 / 34 px).
        app.insert_resource(SafeArea {
            top: 59.0,
            right: 0.0,
            bottom: 34.0,
            left: 0.0,
        });
        app.update();
        hold(&mut app, 4, 640.0, 40.0, 1);
        hold(&mut app, 5, 640.0, 700.0, 1);
        assert_eq!(take(&mut app), ThrottleKeys::default(), "the insets");
        hold(&mut app, 6, 640.0, 80.0, 1);
        assert!(take(&mut app).up_pressed, "inside the safe area");
        app.insert_resource(SafeArea::default());
        // The page shows its log panel / a text field: the zones are off
        // (steering stays).
        app.insert_resource(TouchBlocked(true));
        app.update();
        assert_eq!(app.world().resource::<TouchHud>().power, None);
        let l0 = line(&app);
        hold(&mut app, 7, 640.0, 100.0, 1);
        hold(&mut app, 8, 1000.0, 650.0, 1);
        assert_eq!(take(&mut app), ThrottleKeys::default(), "blocked");
        assert!((line(&app) - l0 - TAP_LINE_STEP_M).abs() < 1e-6);
        app.insert_resource(TouchBlocked(false));
        // The menu is open: nothing, and a finger from then is ignored.
        app.insert_resource(InputContext::Menu);
        touch(&mut app, 9, TouchPhase::Started, 640.0, 100.0);
        app.update();
        app.insert_resource(InputContext::Ride);
        app.update();
        touch(&mut app, 9, TouchPhase::Ended, 640.0, 100.0);
        app.update();
        assert_eq!(take(&mut app), ThrottleKeys::default(), "the menu");
        // The steering off: no zones.
        crate::settings::save_setting(app.world_mut(), "controls.touch_steer", "off");
        app.update();
        hold(&mut app, 10, 640.0, 100.0, 1);
        assert_eq!(take(&mut app), ThrottleKeys::default(), "off");
    }

    fn button(app: &App, b: CornerButton) -> (f32, f32) {
        let t = app.world().resource::<RideTouch>();
        let (_, r) = t
            .buttons_at
            .iter()
            .find(|(k, _)| *k == b)
            .expect("the button shows");
        ((r.min.x + r.max.x) / 2.0, (r.min.y + r.max.y) / 2.0)
    }

    /// The hide button: the controls are not drawn (no chevrons, zone
    /// marks or tip) but every touch area works as before – steering,
    /// faster, slower; the state is saved, a note shows the first time
    /// only, and a second tap shows them again.
    #[test]
    fn hidden_controls_are_not_drawn_but_still_work() {
        let mut app = virtual_app();
        let hud = app.world().resource::<TouchHud>().clone();
        assert!(hud.buttons && hud.hints() && !hud.hidden, "{hud:?}");
        let (x, y) = button(&app, CornerButton::Hide);
        assert!(x < 200.0 && y < 100.0, "top left: {x} {y}");
        let l0 = line(&app);
        take(&mut app);
        hold(&mut app, 1, x, y, 2);
        app.update();
        let hud = app.world().resource::<TouchHud>().clone();
        assert!(hud.hidden && !hud.hints() && hud.shown, "{hud:?}");
        assert!(hud.buttons, "the two buttons stay");
        assert_eq!(hud.toast.as_deref(), Some(HIDDEN_NOTE));
        let settings = &app.world().resource::<UserSettings>().settings;
        assert_eq!(
            settings.get("controls.touch_controls").unwrap().to_string(),
            "hidden"
        );
        assert_eq!(line(&app), l0, "the button never steers");
        assert_eq!(
            take(&mut app),
            ThrottleKeys::default(),
            "… nor changes the power"
        );
        // Hidden: each area still does what it did.
        hold(&mut app, 2, 1000.0, 400.0, 1);
        assert!((line(&app) - l0 - TAP_LINE_STEP_M).abs() < 1e-6);
        hold(&mut app, 3, 200.0, 400.0, 1);
        assert!((line(&app) - l0).abs() < 1e-6);
        hold(&mut app, 4, 640.0, 100.0, 1);
        assert!(take(&mut app).up_pressed);
        hold(&mut app, 5, 640.0, 650.0, 1);
        assert!(take(&mut app).slower_pressed);
        // The feedback of a touch still shows.
        assert!(app.world().resource::<TouchHud>().power_flash.is_some());
        let json = state_json(app.world());
        assert!(json.contains("\"controls_hidden\": true"), "{json}");
        assert!(json.contains("\"hints_drawn\": false"), "{json}");
        assert!(json.contains("\"power\": true"), "{json}");
        // The note goes after a few seconds.
        for _ in 0..(TOAST_S / 0.02) as usize + 5 {
            app.update();
        }
        assert_eq!(app.world().resource::<TouchHud>().toast, None);
        // Shown again, hidden again: no second note.
        hold(&mut app, 6, x, y, 2);
        app.update();
        let hud = app.world().resource::<TouchHud>().clone();
        assert!(!hud.hidden && hud.hints(), "{hud:?}");
        hold(&mut app, 7, x, y, 2);
        app.update();
        let hud = app.world().resource::<TouchHud>().clone();
        assert!(hud.hidden && hud.toast.is_none(), "{hud:?}");
        // A finger that slides off the button does nothing.
        touch(&mut app, 8, TouchPhase::Started, x, y);
        app.update();
        touch(&mut app, 8, TouchPhase::Ended, x + 300.0, y + 300.0);
        app.update();
        app.update();
        assert!(app.world().resource::<TouchHud>().hidden);
    }

    /// The menu button: always there on a touch device, a tap opens the
    /// menu the Esc key opens, a tap on it over the open menu closes it;
    /// it never steers or changes the power.
    #[test]
    fn the_menu_button_opens_and_closes_the_menu() {
        let mut app = App::new();
        // The ride of `app()` with the menu.
        let route = Route::test_loop();
        let core = ClientCore::new(route.clone(), "Ada Rider", Default::default());
        let mut server = FrameServer::new(route.clone(), 0, 0.0);
        server.send(&core.hello());
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
            .add_plugins((
                crate::settings::SettingsPlugin,
                crate::weather::WeatherPlugin,
                crate::timemachine::TimeMachinePlugin,
                crate::riders_page::RidersPagePlugin,
                crate::menu::MenuPlugin,
                TouchSteerPlugin,
            ));
        for _ in 0..200 {
            app.update();
            if client(&app).rider_id().is_some() {
                break;
            }
        }
        app.world_mut()
            .resource_mut::<Ride>()
            .client
            .set_virtual_device(true);
        let open = |app: &App| app.world().resource::<ocw_render::menu::MenuView>().open;
        let settle = |app: &mut App| {
            for _ in 0..3 {
                app.update();
            }
        };
        // A desktop without a touch: no button yet (Esc opens the menu).
        app.update();
        assert!(!app.world().resource::<TouchHud>().buttons);
        // The first touch shows it (a phone shows it from the start).
        hold(&mut app, 1, 640.0, 400.0, 1);
        assert!(app.world().resource::<TouchHud>().buttons);
        let (x, y) = button(&app, CornerButton::Menu);
        assert!(x < 100.0 && y < 100.0, "top left: {x} {y}");
        let l0 = line(&app);
        take(&mut app);
        assert!(!open(&app));
        hold(&mut app, 2, x, y, 2);
        settle(&mut app);
        assert!(open(&app), "the tap opened the menu");
        assert_eq!(*app.world().resource::<InputContext>(), InputContext::Menu);
        let title = app
            .world()
            .resource::<ocw_render::menu::MenuView>()
            .title
            .clone();
        let hud = app.world().resource::<TouchHud>().clone();
        assert!(
            hud.buttons && hud.menu_open,
            "it stays over the menu: {hud:?}"
        );
        // Over the menu the zones are off; the button closes it.
        hold(&mut app, 3, 640.0, 100.0, 1);
        assert_eq!(take(&mut app), ThrottleKeys::default());
        hold(&mut app, 4, x, y, 2);
        settle(&mut app);
        assert!(!open(&app), "the tap closed the menu");
        assert_eq!(*app.world().resource::<InputContext>(), InputContext::Ride);
        assert_eq!(app.world().resource::<RideTouch>().menu_taps, 2);
        assert_eq!(line(&app), l0, "the button never steers");
        assert_eq!(take(&mut app), ThrottleKeys::default());
        // The same page Esc / Start (the menu's Open action) opens.
        app.world_mut()
            .resource_mut::<crate::menu::MenuState>()
            .queue(ocw_client::menu::model::Action::Open);
        settle(&mut app);
        assert!(open(&app));
        assert_eq!(
            app.world().resource::<ocw_render::menu::MenuView>().title,
            title
        );
        // With the steering by touch off the menu button is still there.
        hold(&mut app, 5, x, y, 2);
        settle(&mut app);
        assert!(!open(&app));
        crate::settings::save_setting(app.world_mut(), "controls.touch_steer", "off");
        hold(&mut app, 6, 640.0, 400.0, 1);
        assert!(app.world().resource::<TouchHud>().buttons);
        assert!(!app.world().resource::<TouchHud>().shown);
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

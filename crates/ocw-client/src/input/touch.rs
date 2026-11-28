//! Touch steering in a ride (spec §3.2.3), pure and shared by both apps:
//!
//! - [`TouchZones`]: the left and right steering zones of the view – the
//!   full height below the top band (or only the lower half, setting
//!   `controls.touch_zones lower`), without the dead band in the centre,
//!   the 16 px window edges, the safe-area insets and the HUD's tap
//!   targets (the junction fan);
//! - [`TouchSteer`]: the per-frame state machine. A finger in a zone is
//!   the arrow key of that side held down (line steering in autodrive,
//!   [`super::lean_ramp`] otherwise); a tap (lifted within
//!   [`TAP_MAX_S`], moved less than [`TAP_MAX_MOVE_PX`]) is topped up to
//!   one step ([`TAP_LINE_STEP_M`], [`TAP_LEAN_STEP_RAD`]) at any frame
//!   rate. Both sides at once steer nothing (as ← + →). A touch never
//!   switches autodrive off. Two fingers that move together are the
//!   camera (drag: orbit, pinch: chase distance); one finger only steers.
//!   A tap on an exit arrow of the junction fan chooses that exit
//!   ([`fan_hit`]); the halves never choose exits (Q5);
//! - [`TouchPresence`]: whether the hints (chevrons, edge glow, tip) are
//!   shown – at the start on a touch-first device, after the first real
//!   touch on any device, hidden again by a key, a click, a mouse move of
//!   4 px or a gamepad (the emulated mouse of a touch does not count).
//!
//! Positions are logical pixels from the window's top left. The Bevy glue
//! is `ocw_ui::touch`.

use std::collections::BTreeMap;

use super::{LINE_KEY_RATE_MPS, lean_ramp};

/// A tap is lifted within this time (s; spec §3.2.3 Q1: 250 ms).
pub const TAP_MAX_S: f64 = 0.25;
/// … and moved less than this (logical px).
pub const TAP_MAX_MOVE_PX: f32 = 10.0;
/// A tap moves the autodrive line at least this far (m).
pub const TAP_LINE_STEP_M: f64 = 0.5;
/// A tap changes the lean at least this much (rad, about 6°).
pub const TAP_LEAN_STEP_RAD: f64 = 0.1;
/// The top band without steering: this share of the view's height.
pub const TOP_BAND_FRAC: f32 = 0.15;
/// The dead band in the centre: this share of the width …
pub const DEAD_BAND_FRAC: f32 = 0.12;
/// … and at least this wide (px).
pub const DEAD_BAND_MIN_PX: f32 = 48.0;
/// The window edges kept free for the system's edge swipes (px).
pub const EDGE_PX: f32 = 16.0;
/// Two fingers become the camera once their centre or their distance
/// moved this far (px); until then they are two steering fingers.
pub const CAMERA_START_PX: f32 = 12.0;
/// Mouse events this soon after a touch, near it, are the system's
/// emulated mouse (s, px).
pub const EMULATED_MOUSE_S: f64 = 0.5;
pub const EMULATED_MOUSE_PX: f32 = 32.0;
/// A mouse move of this much hides the hints (px; less is jitter).
pub const MOUSE_MOVE_PX: f32 = 4.0;
/// The chevrons dim after this long without a touch (s).
pub const HINT_DIM_S: f64 = 8.0;

/// A point (logical px from the window's top left).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Pos {
    pub x: f32,
    pub y: f32,
}

impl Pos {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance(self, o: Pos) -> f32 {
        ((self.x - o.x).powi(2) + (self.y - o.y).powi(2)).sqrt()
    }

    fn mid(self, o: Pos) -> Pos {
        Pos::new((self.x + o.x) / 2.0, (self.y + o.y) / 2.0)
    }
}

/// An axis-aligned rectangle (px).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub min: Pos,
    pub max: Pos,
}

impl Rect {
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self {
            min: Pos::new(x0, y0),
            max: Pos::new(x1, y1),
        }
    }

    pub fn contains(&self, p: Pos) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }
}

/// A steering side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// −1 left, +1 right (as [`super::steer_direction`]).
    pub fn sign(self) -> f64 {
        match self {
            Side::Left => -1.0,
            Side::Right => 1.0,
        }
    }
}

/// How tall the zones are (setting `controls.touch_zones`, spec §3.2.3
/// Q8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ZoneHeight {
    /// The full height below the top band (the default).
    #[default]
    Full,
    /// Only the lower half of the view (the upper half stays free).
    Lower,
}

impl ZoneHeight {
    pub fn parse(word: &str) -> Self {
        if word == "lower" {
            Self::Lower
        } else {
            Self::Full
        }
    }
}

/// The safe-area insets (notch, home indicator; px).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Insets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

/// Insets above this are not believed (px; a broken page value).
pub const MAX_INSET_PX: f32 = 400.0;

impl Insets {
    /// From the page's JSON (`{"top": 59, "right": 0, "bottom": 34,
    /// "left": 0}`, CSS px = logical px: `env(safe-area-inset-*)`, spec
    /// §3.2.3 *Safe areas*); missing, negative or broken values are 0,
    /// values above [`MAX_INSET_PX`] are capped.
    pub fn from_json(text: &str) -> Self {
        let v: serde_json::Value = serde_json::from_str(text).unwrap_or_default();
        let px = |k: &str| {
            v[k].as_f64()
                .filter(|x| x.is_finite())
                .map_or(0.0, |x| (x as f32).clamp(0.0, MAX_INSET_PX))
        };
        Self {
            top: px("top"),
            right: px("right"),
            bottom: px("bottom"),
            left: px("left"),
        }
    }

    /// No inset on any side (desktops, phones without a notch).
    pub fn is_zero(&self) -> bool {
        self.top <= 0.0 && self.right <= 0.0 && self.bottom <= 0.0 && self.left <= 0.0
    }
}

/// The two steering zones of a view.
#[derive(Debug, Clone, PartialEq)]
pub struct TouchZones {
    /// The view's size (px).
    pub width: f32,
    pub height: f32,
    pub insets: Insets,
    pub zone_height: ZoneHeight,
    /// The HUD's tap targets: never a steer.
    pub excluded: Vec<Rect>,
    /// The faster / slower zones are on (the virtual trainer, spec 3.2.3
    /// *Faster / slower zones*): steering only between these two heights
    /// (px from the top; `super::power_zones::PowerZones::band`).
    pub band: Option<(f32, f32)>,
}

impl TouchZones {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            insets: Insets::default(),
            zone_height: ZoneHeight::Full,
            excluded: Vec::new(),
            band: None,
        }
    }

    /// The dead band's half width (px).
    pub fn dead_half(&self) -> f32 {
        (self.width * DEAD_BAND_FRAC).max(DEAD_BAND_MIN_PX) / 2.0
    }

    /// Where the zones start (px from the top).
    pub fn top(&self) -> f32 {
        let band = (self.height * TOP_BAND_FRAC)
            .max(self.insets.top + EDGE_PX)
            .max(self.band.map_or(0.0, |b| b.0));
        match self.zone_height {
            ZoneHeight::Full => band,
            ZoneHeight::Lower => band.max(self.height / 2.0),
        }
    }

    /// Where the zones end (px from the top).
    pub fn bottom(&self) -> f32 {
        let edge = self.height - EDGE_PX.max(self.insets.bottom);
        self.band.map_or(edge, |b| edge.min(b.1))
    }

    /// The zone's rectangle of `side` (the dead band, edges and insets
    /// taken off; excluded targets are not cut out).
    pub fn rect(&self, side: Side) -> Rect {
        let mid = self.width / 2.0;
        let top = self.top();
        let bottom = self.bottom();
        match side {
            Side::Left => Rect::new(
                EDGE_PX.max(self.insets.left),
                top,
                mid - self.dead_half(),
                bottom,
            ),
            Side::Right => Rect::new(
                mid + self.dead_half(),
                top,
                self.width - EDGE_PX.max(self.insets.right),
                bottom,
            ),
        }
    }

    /// The zone at `p`, if any.
    pub fn zone_at(&self, p: Pos) -> Option<Side> {
        if self.excluded.iter().any(|r| r.contains(p)) {
            return None;
        }
        [Side::Left, Side::Right]
            .into_iter()
            .find(|s| self.rect(*s).contains(p))
    }

    /// Whether `p` is on one of the HUD's tap targets.
    pub fn on_target(&self, p: Pos) -> bool {
        self.excluded.iter().any(|r| r.contains(p))
    }
}

/// The junction fan's geometry on screen (px): its rectangle, the arrows'
/// pivot, and how long the arrows are.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Fan {
    pub rect: Rect,
    pub pivot: Pos,
    pub arrow_len: f32,
}

/// The exit whose arrow a touch at `p` hits: inside the fan, at least a
/// fifth of an arrow from the pivot, the arrow nearest in direction (each
/// exit's turn angle in degrees, clockwise = right, 0 = straight up) and
/// within half the gap to its neighbours (at most 45°).
pub fn fan_hit(fan: &Fan, angles_deg: &[f64], p: Pos) -> Option<usize> {
    if !fan.rect.contains(p) || angles_deg.is_empty() {
        return None;
    }
    let (dx, dy) = (p.x - fan.pivot.x, p.y - fan.pivot.y);
    if (dx * dx + dy * dy).sqrt() < fan.arrow_len * 0.2 {
        return None;
    }
    // Clockwise from straight up.
    let at = f64::from(dx).atan2(f64::from(-dy)).to_degrees();
    let (i, diff) = angles_deg
        .iter()
        .enumerate()
        .map(|(i, a)| (i, (at - a).abs()))
        .min_by(|a, b| a.1.total_cmp(&b.1))?;
    let mut reach: f64 = 45.0;
    for j in [i.wrapping_sub(1), i + 1] {
        if let Some(n) = angles_deg.get(j) {
            reach = reach.min((angles_deg[i] - n).abs() / 2.0);
        }
    }
    (diff < reach.max(8.0)).then_some(i)
}

/// What a finger is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Steers by the zone it is in now.
    Steer,
    /// Came down on a tap target (the fan): never steers.
    Target,
    /// Part of a two-finger camera gesture (until it is lifted).
    Camera,
    /// Down from before the zones were active, or a third finger: ignored
    /// until lifted.
    Ignored,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Finger {
    role: Role,
    start: Pos,
    pos: Pos,
    prev: Pos,
    down_s: f64,
    moved_px: f32,
    /// Line (m) or lean (rad) steered while this finger was the only side
    /// held (for the tap top-up).
    steered: f64,
    /// The lean when it came down (lean steering).
    lean0: f64,
    /// The other side was held at some point: a tap does nothing (Q4).
    blocked: bool,
}

/// One touch event (from Bevy's `TouchInput` / `Touches`, or the tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TouchEvent {
    Start { id: u64, pos: Pos },
    Move { id: u64, pos: Pos },
    End { id: u64, pos: Pos },
    Cancel { id: u64 },
}

/// What the steering acts on (the ride's client; a mock in tests).
pub trait Steerable {
    /// Autodrive: the zones move the line; else they lean.
    fn autodrive(&self) -> bool;
    /// Moves the autodrive line by `delta_m` (positive = right).
    fn steer_by(&mut self, delta_m: f64);
    /// The lean (rad, positive = right).
    fn lean(&self) -> f64;
    fn set_lean(&mut self, rad: f64);
}

impl Steerable for crate::client::ClientCore {
    fn autodrive(&self) -> bool {
        crate::client::ClientCore::autodrive(self)
    }
    fn steer_by(&mut self, delta_m: f64) {
        crate::client::ClientCore::steer_by(self, delta_m);
    }
    fn lean(&self) -> f64 {
        crate::client::ClientCore::lean(self).unwrap_or(0.0)
    }
    fn set_lean(&mut self, rad: f64) {
        crate::client::ClientCore::set_lean(self, rad);
    }
}

/// What the fingers did this frame besides steering.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TouchFrame {
    /// The camera's two-finger drag (px, as a mouse drag) …
    pub camera_drag: (f32, f32),
    /// … and pinch, as wheel steps (positive: spread = closer).
    pub camera_zoom: f64,
    /// The junction exit tapped (leftmost = 0).
    pub exit: Option<usize>,
    /// The sides held now (both: nothing steers).
    pub left_held: bool,
    pub right_held: bool,
    /// A tap step on this side this frame (the chevron flashes).
    pub tap: Option<Side>,
    /// A real touch arrived this frame (the hints' presence).
    pub touched: bool,
}

/// The ride's touch steering state machine (spec §3.2.3).
#[derive(Debug, Clone, Default)]
pub struct TouchSteer {
    fingers: BTreeMap<u64, Finger>,
    events: Vec<(TouchEvent, f64)>,
    /// The upright pause of [`lean_ramp`] (as the keys').
    upright_pause: f64,
}

impl TouchSteer {
    /// Queues a touch event at game time `now` (s).
    pub fn event(&mut self, e: TouchEvent, now: f64) {
        self.events.push((e, now));
    }

    /// Fingers down now.
    pub fn fingers(&self) -> usize {
        self.fingers.len()
    }

    /// While the zones are not active (menu, console, flight, setting
    /// *off*): every finger down now is ignored until it is lifted.
    pub fn suspend(&mut self) {
        for (e, _) in std::mem::take(&mut self.events) {
            match e {
                TouchEvent::Start { id, pos } => {
                    self.fingers
                        .insert(id, finger(Role::Ignored, pos, 0.0, 0.0));
                }
                TouchEvent::End { id, .. } | TouchEvent::Cancel { id } => {
                    self.fingers.remove(&id);
                }
                TouchEvent::Move { .. } => {}
            }
        }
        for f in self.fingers.values_mut() {
            f.role = Role::Ignored;
        }
        self.upright_pause = 0.0;
    }

    /// Applies the frame's events and steers `target` for `dt` s.
    /// `fan`: the junction fan and its exits' angles while it shows.
    pub fn update(
        &mut self,
        zones: &TouchZones,
        fan: Option<(&Fan, &[f64])>,
        target: &mut dyn Steerable,
        dt: f64,
    ) -> TouchFrame {
        let mut out = TouchFrame::default();
        let autodrive = target.autodrive();
        for (e, now) in std::mem::take(&mut self.events) {
            match e {
                TouchEvent::Start { id, pos } => {
                    out.touched = true;
                    let busy = self
                        .fingers
                        .values()
                        .filter(|f| f.role != Role::Ignored)
                        .count();
                    let role = if busy >= 2 {
                        Role::Ignored
                    } else if let Some(i) = fan.and_then(|(f, a)| fan_hit(f, a, pos)) {
                        out.exit = Some(i);
                        Role::Target
                    } else if zones.on_target(pos) {
                        Role::Target
                    } else if self.fingers.values().any(|f| f.role == Role::Camera) {
                        Role::Camera
                    } else {
                        Role::Steer
                    };
                    self.fingers
                        .insert(id, finger(role, pos, now, target.lean()));
                }
                TouchEvent::Move { id, pos } => {
                    if let Some(f) = self.fingers.get_mut(&id) {
                        f.pos = pos;
                        f.moved_px = f.moved_px.max(pos.distance(f.start));
                    }
                }
                TouchEvent::End { id, pos } => {
                    out.touched = true;
                    if let Some(mut f) = self.fingers.remove(&id) {
                        f.pos = pos;
                        f.moved_px = f.moved_px.max(pos.distance(f.start));
                        if let Some(side) = self.tap_side(&f, zones, now) {
                            self.top_up(&f, side, autodrive, target);
                            out.tap = Some(side);
                        }
                    }
                }
                TouchEvent::Cancel { id } => {
                    self.fingers.remove(&id);
                }
            }
        }
        // Two fingers that move together are the camera.
        let pair: Vec<u64> = self
            .fingers
            .iter()
            .filter(|(_, f)| matches!(f.role, Role::Steer | Role::Camera))
            .map(|(id, _)| *id)
            .collect();
        if let [a, b] = pair[..] {
            let (fa, fb) = (self.fingers[&a], self.fingers[&b]);
            let camera = fa.role == Role::Camera
                || fb.role == Role::Camera
                || fa.pos.mid(fb.pos).distance(fa.start.mid(fb.start)) >= CAMERA_START_PX
                || (fa.pos.distance(fb.pos) - fa.start.distance(fb.start)).abs() >= CAMERA_START_PX;
            if camera {
                for id in [a, b] {
                    if let Some(f) = self.fingers.get_mut(&id) {
                        f.role = Role::Camera;
                    }
                }
                let (now_mid, prev_mid) = (fa.pos.mid(fb.pos), fa.prev.mid(fb.prev));
                out.camera_drag = (now_mid.x - prev_mid.x, now_mid.y - prev_mid.y);
                let (d, d_prev) = (fa.pos.distance(fb.pos), fa.prev.distance(fb.prev));
                if d > 1.0 && d_prev > 1.0 {
                    // The orbit's wheel: one step scales the distance by
                    // e^0.12; spreading the fingers comes closer.
                    out.camera_zoom = f64::from(d / d_prev).ln() / 0.12;
                }
            }
        }
        for f in self.fingers.values_mut() {
            f.prev = f.pos;
        }
        // The sides held now: a steering finger in a zone (two on one
        // side count as one).
        for f in self.fingers.values() {
            if f.role == Role::Steer {
                match zones.zone_at(f.pos) {
                    Some(Side::Left) => out.left_held = true,
                    Some(Side::Right) => out.right_held = true,
                    None => {}
                }
            }
        }
        let both = out.left_held && out.right_held;
        let dir = super::steer_direction(out.left_held, out.right_held);
        let before = target.lean();
        if autodrive {
            if dir != 0.0 {
                target.steer_by(dir * LINE_KEY_RATE_MPS * dt);
            }
        } else {
            let next = lean_ramp(before, dir, &mut self.upright_pause, dt);
            if dir != 0.0 && next != before {
                target.set_lean(next);
            }
        }
        let done = if autodrive {
            dir * LINE_KEY_RATE_MPS * dt
        } else {
            target.lean() - before
        };
        for f in self.fingers.values_mut() {
            if f.role != Role::Steer {
                continue;
            }
            if both {
                f.blocked = true;
            } else if let Some(s) = zones.zone_at(f.pos) {
                f.steered += done * s.sign();
            }
        }
        out
    }

    /// The side a lifted finger tapped, if it was a tap that counts.
    fn tap_side(&self, f: &Finger, zones: &TouchZones, now: f64) -> Option<Side> {
        let tap = now - f.down_s < TAP_MAX_S && f.moved_px < TAP_MAX_MOVE_PX;
        if f.role != Role::Steer || !tap || f.blocked {
            return None;
        }
        let side = zones.zone_at(f.pos)?;
        // Another finger holds the other side: nothing (Q4).
        let other = self
            .fingers
            .values()
            .any(|o| o.role == Role::Steer && zones.zone_at(o.pos) == Some(opposite(side)));
        (!other).then_some(side)
    }

    /// Tops a tap up to one step (the steering it did while held counts).
    fn top_up(&mut self, f: &Finger, side: Side, autodrive: bool, target: &mut dyn Steerable) {
        let s = side.sign();
        if autodrive {
            let rest = TAP_LINE_STEP_M - f.steered;
            if rest > 0.0 {
                target.steer_by(s * rest);
            }
            return;
        }
        // The lean: one step from where it was when the finger came down,
        // stopping at upright on the way through (as `lean_ramp`).
        let max = ocw_sim::physics::MAX_STEER_LEAN;
        let from = f.lean0;
        let mut want = (from + s * TAP_LEAN_STEP_RAD).clamp(-max, max);
        if from != 0.0 && (want == 0.0 || want.signum() != from.signum()) {
            want = 0.0;
        }
        let now = target.lean();
        // Only ever further in the tap's direction.
        if (want - now) * s > 0.0 {
            target.set_lean(want);
        }
    }
}

fn opposite(s: Side) -> Side {
    match s {
        Side::Left => Side::Right,
        Side::Right => Side::Left,
    }
}

fn finger(role: Role, pos: Pos, now: f64, lean: f64) -> Finger {
    Finger {
        role,
        start: pos,
        pos,
        prev: pos,
        down_s: now,
        moved_px: 0.0,
        steered: 0.0,
        lean0: lean,
        blocked: false,
    }
}

/// The page's facts about touch (the browser; the desktop knows none).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TouchFacts {
    /// `navigator.maxTouchPoints`.
    pub max_touch_points: u32,
    /// The primary pointer is coarse (`(pointer: coarse)`).
    pub coarse_pointer: bool,
    /// An iPad that reports a Mac (`MacIntel` with `maxTouchPoints > 1`).
    pub ipad_as_mac: bool,
}

impl TouchFacts {
    /// A phone or tablet: the hints show from the start.
    pub fn touch_first(&self) -> bool {
        (self.coarse_pointer && self.max_touch_points > 0) || self.ipad_as_mac
    }
}

/// The setting `controls.touch_steer`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TouchSetting {
    /// Hints on touch-first devices and after a touch (the default).
    #[default]
    Auto,
    /// Always the hints.
    On,
    /// No zones and no hints (taps reach only the HUD's buttons).
    Off,
}

impl TouchSetting {
    pub const CHOICES: [&'static str; 3] = ["auto", "on", "off"];

    pub fn parse(word: &str) -> Self {
        match word {
            "on" => Self::On,
            "off" => Self::Off,
            _ => Self::Auto,
        }
    }
}

/// Whether the touch hints are shown (spec §3.2.3 *Detection*).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct TouchPresence {
    shown: bool,
    last_touch: Option<(f64, Pos)>,
    mouse_moved_px: f32,
}

impl TouchPresence {
    /// At the start: shown on a touch-first device.
    pub fn new(facts: TouchFacts) -> Self {
        Self {
            shown: facts.touch_first(),
            ..Self::default()
        }
    }

    /// A real touch at `pos`.
    pub fn touch(&mut self, now: f64, pos: Pos) {
        self.shown = true;
        self.last_touch = Some((now, pos));
        self.mouse_moved_px = 0.0;
    }

    /// The system's emulated mouse after a touch: soon and near it.
    fn emulated(&self, now: f64, pos: Option<Pos>) -> bool {
        self.last_touch.is_some_and(|(t, p)| {
            now - t <= EMULATED_MOUSE_S && pos.is_none_or(|q| q.distance(p) <= EMULATED_MOUSE_PX)
        })
    }

    /// A key or a gamepad button / stick beyond its dead zone.
    pub fn key(&mut self) {
        self.shown = false;
    }

    /// A mouse click at `pos` (if known).
    pub fn click(&mut self, now: f64, pos: Option<Pos>) {
        if !self.emulated(now, pos) {
            self.shown = false;
        }
    }

    /// The mouse moved by `delta_px` (to `pos`, if known).
    pub fn mouse_move(&mut self, now: f64, delta_px: f32, pos: Option<Pos>) {
        if delta_px <= 0.0 || self.emulated(now, pos) {
            return;
        }
        self.mouse_moved_px += delta_px;
        if self.mouse_moved_px >= MOUSE_MOVE_PX {
            self.shown = false;
            self.mouse_moved_px = 0.0;
        }
    }

    /// Whether the hints show with `setting`.
    pub fn shown(&self, setting: TouchSetting) -> bool {
        match setting {
            TouchSetting::Auto => self.shown,
            TouchSetting::On => true,
            TouchSetting::Off => false,
        }
    }

    /// The chevrons are dimmed: no touch for [`HINT_DIM_S`].
    pub fn dimmed(&self, now: f64) -> bool {
        self.last_touch.is_none_or(|(t, _)| now - t >= HINT_DIM_S)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;
    const LEFT: Pos = Pos::new(200.0, 500.0);
    const RIGHT: Pos = Pos::new(1080.0, 500.0);

    /// A steering target: the line or the lean.
    #[derive(Debug, Default)]
    struct Rider {
        autodrive: bool,
        line: f64,
        lean: f64,
    }

    impl Steerable for Rider {
        fn autodrive(&self) -> bool {
            self.autodrive
        }
        fn steer_by(&mut self, d: f64) {
            self.line += d;
        }
        fn lean(&self) -> f64 {
            self.lean
        }
        fn set_lean(&mut self, r: f64) {
            let m = ocw_sim::physics::MAX_STEER_LEAN;
            self.lean = r.clamp(-m, m);
        }
    }

    fn line_rider() -> Rider {
        Rider {
            autodrive: true,
            ..Rider::default()
        }
    }

    /// Runs frames of `dt` from `t0` for `secs` s with the events at
    /// their times (s from `t0`).
    struct Run {
        steer: TouchSteer,
        zones: TouchZones,
        t: f64,
    }

    impl Run {
        fn new() -> Self {
            Self {
                steer: TouchSteer::default(),
                zones: TouchZones::new(W, H),
                t: 0.0,
            }
        }
        /// One frame of `dt`: the events come at its start.
        fn frame(&mut self, r: &mut Rider, dt: f64, events: &[TouchEvent]) -> TouchFrame {
            for e in events {
                self.steer.event(*e, self.t);
            }
            self.t += dt;
            self.steer.update(&self.zones, None, r, dt)
        }
        /// A touch at `p` held for `hold` s (frames of `dt`), then lifted.
        fn press(&mut self, r: &mut Rider, id: u64, p: Pos, hold: f64, dt: f64) -> TouchFrame {
            let start = self.t;
            self.frame(r, dt.min(hold), &[TouchEvent::Start { id, pos: p }]);
            while self.t - start < hold - 1e-9 {
                let step = dt.min(hold - (self.t - start));
                self.frame(r, step, &[]);
            }
            self.frame(r, dt, &[TouchEvent::End { id, pos: p }])
        }
    }

    #[test]
    fn the_zones_are_the_halves_without_centre_top_edges_and_targets() {
        let mut z = TouchZones::new(W, H);
        assert_eq!(z.zone_at(LEFT), Some(Side::Left));
        assert_eq!(z.zone_at(RIGHT), Some(Side::Right));
        // The dead band: 12 % of 1280 = 153.6 px.
        assert_eq!(z.zone_at(Pos::new(W / 2.0 - 70.0, 500.0)), None);
        assert_eq!(
            z.zone_at(Pos::new(W / 2.0 + 80.0, 500.0)),
            Some(Side::Right)
        );
        // At least 48 px on a narrow view.
        let narrow = TouchZones::new(300.0, 600.0);
        assert_eq!(narrow.dead_half(), 24.0);
        assert_eq!(narrow.zone_at(Pos::new(150.0 - 23.0, 400.0)), None);
        assert_eq!(
            narrow.zone_at(Pos::new(150.0 - 25.0, 400.0)),
            Some(Side::Left)
        );
        // The top band (15 % of 720 = 108 px).
        assert_eq!(z.zone_at(Pos::new(200.0, 100.0)), None);
        assert_eq!(z.zone_at(Pos::new(200.0, 110.0)), Some(Side::Left));
        // The 16 px edges.
        assert_eq!(z.zone_at(Pos::new(10.0, 400.0)), None);
        assert_eq!(z.zone_at(Pos::new(W - 10.0, 400.0)), None);
        assert_eq!(z.zone_at(Pos::new(400.0, H - 10.0)), None);
        // The safe-area insets.
        z.insets = Insets {
            left: 44.0,
            ..Insets::default()
        };
        assert_eq!(z.zone_at(Pos::new(30.0, 400.0)), None);
        assert_eq!(z.zone_at(Pos::new(50.0, 400.0)), Some(Side::Left));
        // A HUD tap target is never a steer.
        z.excluded.push(Rect::new(500.0, 560.0, 800.0, 714.0));
        assert_eq!(z.zone_at(Pos::new(510.0, 600.0)), None);
        assert!(z.on_target(Pos::new(510.0, 600.0)));
    }

    #[test]
    fn the_page_s_safe_insets_are_read_and_checked() {
        // An iPhone 15 in landscape: the notch side and the home bar.
        let i = Insets::from_json(r#"{"top": 0, "right": 59, "bottom": 21, "left": 59}"#);
        assert_eq!(
            i,
            Insets {
                top: 0.0,
                right: 59.0,
                bottom: 21.0,
                left: 59.0
            }
        );
        assert!(!i.is_zero());
        // Missing, broken, negative, too large.
        assert!(Insets::from_json("").is_zero());
        assert!(Insets::from_json("nonsense").is_zero());
        assert!(Insets::from_json(r#"{"top": "x", "left": -5}"#).is_zero());
        assert_eq!(
            Insets::from_json(r#"{"top": 1e9}"#).top,
            MAX_INSET_PX,
            "capped"
        );
        assert_eq!(Insets::from_json(r#"{"bottom": 34.5}"#).bottom, 34.5);
    }

    #[test]
    fn the_page_s_insets_keep_the_zones_off_the_notch_and_the_home_bar() {
        // iPhone 15 landscape (852 × 393): the insets as the page passes them.
        let mut z = TouchZones::new(852.0, 393.0);
        z.insets = Insets::from_json(r#"{"top": 0, "right": 59, "bottom": 21, "left": 59}"#);
        assert_eq!(z.zone_at(Pos::new(40.0, 250.0)), None, "behind the notch");
        assert_eq!(z.zone_at(Pos::new(812.0, 250.0)), None, "the other side");
        assert_eq!(z.zone_at(Pos::new(200.0, 380.0)), None, "on the home bar");
        assert_eq!(z.zone_at(Pos::new(200.0, 250.0)), Some(Side::Left));
        assert_eq!(z.zone_at(Pos::new(650.0, 250.0)), Some(Side::Right));
        // Portrait (393 × 852): the notch pushes the top band down.
        let mut p = TouchZones::new(393.0, 852.0);
        p.insets = Insets::from_json(r#"{"top": 59, "bottom": 34}"#);
        assert_eq!(p.top(), (852.0 * TOP_BAND_FRAC).max(59.0 + EDGE_PX));
        assert_eq!(p.zone_at(Pos::new(80.0, 830.0)), None, "on the home bar");
        assert_eq!(p.zone_at(Pos::new(80.0, 600.0)), Some(Side::Left));
        assert_eq!(p.zone_at(Pos::new(320.0, 600.0)), Some(Side::Right));
    }

    #[test]
    fn the_lower_half_setting_frees_the_upper_half() {
        let mut z = TouchZones::new(W, H);
        assert_eq!(z.zone_at(Pos::new(200.0, 200.0)), Some(Side::Left));
        z.zone_height = ZoneHeight::Lower;
        assert_eq!(z.zone_at(Pos::new(200.0, 200.0)), None);
        assert_eq!(z.zone_at(Pos::new(200.0, H / 2.0 - 1.0)), None);
        assert_eq!(z.zone_at(Pos::new(200.0, H / 2.0 + 1.0)), Some(Side::Left));
        assert_eq!(ZoneHeight::parse("lower"), ZoneHeight::Lower);
        assert_eq!(ZoneHeight::parse("full"), ZoneHeight::Full);
    }

    #[test]
    fn a_tap_moves_the_line_half_a_metre_at_any_frame_rate() {
        for fps in [30.0, 60.0, 144.0] {
            let dt = 1.0 / fps;
            for hold in [dt, 0.1, 0.249] {
                let mut r = line_rider();
                let mut run = Run::new();
                let f = run.press(&mut r, 1, RIGHT, hold, dt);
                assert_eq!(f.tap, Some(Side::Right));
                assert!(
                    (r.line - 0.5).abs() < 1e-9,
                    "{fps} fps {hold} s: {}",
                    r.line
                );
                let f = run.press(&mut r, 2, LEFT, hold, dt);
                assert_eq!(f.tap, Some(Side::Left));
                assert!(r.line.abs() < 1e-9, "{fps} fps back: {}", r.line);
            }
        }
    }

    #[test]
    fn a_hold_moves_the_line_like_the_held_key() {
        let mut r = line_rider();
        let mut run = Run::new();
        let f = run.press(&mut r, 1, LEFT, 1.0, 1.0 / 60.0);
        assert_eq!(f.tap, None);
        assert!((r.line + 2.0).abs() < 1e-6, "{}", r.line);
        // Steering starts on touch-down, with no wait.
        let mut r = line_rider();
        let mut run = Run::new();
        let f = run.frame(
            &mut r,
            1.0 / 60.0,
            &[TouchEvent::Start { id: 1, pos: RIGHT }],
        );
        assert!(f.right_held);
        assert!((r.line - 2.0 / 60.0).abs() < 1e-9, "{}", r.line);
    }

    #[test]
    fn tap_vs_hold_at_the_thresholds() {
        let dt = 1.0 / 100.0;
        // 0.249 s: a tap; 0.251 s: a hold (only what it steered).
        let mut r = line_rider();
        Run::new().press(&mut r, 1, RIGHT, 0.249, dt);
        assert!((r.line - 0.5).abs() < 1e-9);
        let mut r = line_rider();
        let f = Run::new().press(&mut r, 1, RIGHT, 0.251, dt);
        assert_eq!(f.tap, None);
        assert!((r.line - 0.502).abs() < 1e-6, "{}", r.line);
        // Moved 9 px: a tap; 11 px: not.
        for (moved, tap) in [(9.0, true), (11.0, false)] {
            let mut r = line_rider();
            let mut run = Run::new();
            run.frame(&mut r, dt, &[TouchEvent::Start { id: 1, pos: RIGHT }]);
            let to = Pos::new(RIGHT.x + moved, RIGHT.y);
            run.frame(&mut r, dt, &[TouchEvent::Move { id: 1, pos: to }]);
            let f = run.frame(&mut r, dt, &[TouchEvent::End { id: 1, pos: to }]);
            assert_eq!(f.tap.is_some(), tap, "{moved} px");
        }
    }

    #[test]
    fn a_tap_leans_a_tenth_of_a_radian_and_the_lean_stays() {
        for fps in [30.0, 60.0, 144.0] {
            let dt = 1.0 / fps;
            let mut r = Rider::default();
            let mut run = Run::new();
            run.press(&mut r, 1, RIGHT, dt, dt);
            assert!(
                (r.lean - TAP_LEAN_STEP_RAD).abs() < 1e-9,
                "{fps}: {}",
                r.lean
            );
            // Stays after the finger lifted (Q2).
            run.frame(&mut r, 1.0, &[]);
            assert!((r.lean - TAP_LEAN_STEP_RAD).abs() < 1e-9);
            run.press(&mut r, 2, RIGHT, 0.2, dt);
            assert!((r.lean - 0.2).abs() < 1e-9, "{fps}: {}", r.lean);
        }
    }

    #[test]
    fn a_tap_toward_upright_stops_there_and_the_next_leans_on() {
        let dt = 1.0 / 60.0;
        let mut r = Rider {
            lean: 0.05,
            ..Rider::default()
        };
        let mut run = Run::new();
        run.press(&mut r, 1, LEFT, 0.1, dt);
        assert_eq!(r.lean, 0.0);
        run.press(&mut r, 2, LEFT, 0.1, dt);
        assert!((r.lean + TAP_LEAN_STEP_RAD).abs() < 1e-9, "{}", r.lean);
        // A held side leans like the held key (0.4 rad/s).
        let mut r = Rider::default();
        Run::new().press(&mut r, 1, RIGHT, 0.5, dt);
        assert!((r.lean - 0.2).abs() < 1e-6, "{}", r.lean);
    }

    #[test]
    fn both_sides_at_once_do_nothing_and_one_lifted_resumes_the_other() {
        let dt = 1.0 / 60.0;
        let mut r = line_rider();
        let mut run = Run::new();
        run.frame(&mut r, dt, &[TouchEvent::Start { id: 1, pos: LEFT }]);
        let held = r.line;
        let f = run.frame(&mut r, dt, &[TouchEvent::Start { id: 2, pos: RIGHT }]);
        assert!(f.left_held && f.right_held);
        assert_eq!(r.line, held, "both: nothing");
        for _ in 0..10 {
            run.frame(&mut r, dt, &[]);
        }
        assert_eq!(r.line, held);
        // The right one lifted (a tap while the left is held: nothing).
        let f = run.frame(&mut r, dt, &[TouchEvent::End { id: 2, pos: RIGHT }]);
        assert_eq!(f.tap, None);
        assert!(r.line < held, "the left resumes: {}", r.line);
        // Two fingers on one side count as one.
        let mut r = line_rider();
        let mut run = Run::new();
        run.frame(
            &mut r,
            dt,
            &[
                TouchEvent::Start { id: 1, pos: LEFT },
                TouchEvent::Start {
                    id: 2,
                    pos: Pos::new(LEFT.x + 100.0, LEFT.y),
                },
            ],
        );
        assert!((r.line + 2.0 * dt).abs() < 1e-9, "{}", r.line);
    }

    #[test]
    fn the_zone_follows_the_finger() {
        let dt = 1.0 / 60.0;
        let mut r = line_rider();
        let mut run = Run::new();
        run.frame(&mut r, dt, &[TouchEvent::Start { id: 1, pos: LEFT }]);
        let a = r.line;
        // Into the dead band: paused.
        let centre = Pos::new(W / 2.0, 500.0);
        run.frame(&mut r, dt, &[TouchEvent::Move { id: 1, pos: centre }]);
        assert_eq!(r.line, a);
        // Across: the other way.
        run.frame(&mut r, dt, &[TouchEvent::Move { id: 1, pos: RIGHT }]);
        assert!(r.line > a);
        // Lifted after moving: not a tap.
        let f = run.frame(&mut r, dt, &[TouchEvent::End { id: 1, pos: RIGHT }]);
        assert_eq!(f.tap, None);
    }

    #[test]
    fn cancel_releases_and_fingers_from_before_are_ignored() {
        let dt = 1.0 / 60.0;
        let mut r = line_rider();
        let mut run = Run::new();
        run.frame(&mut r, dt, &[TouchEvent::Start { id: 1, pos: RIGHT }]);
        let f = run.frame(&mut r, dt, &[TouchEvent::Cancel { id: 1 }]);
        assert!(!f.right_held && f.tap.is_none());
        let a = r.line;
        run.frame(&mut r, dt, &[]);
        assert_eq!(r.line, a);
        // Down while the menu was open: ignored until lifted.
        let mut r = line_rider();
        let mut run = Run::new();
        run.steer
            .event(TouchEvent::Start { id: 7, pos: RIGHT }, 0.0);
        run.steer.suspend();
        let f = run.frame(&mut r, dt, &[]);
        assert!(!f.right_held);
        let f = run.frame(&mut r, dt, &[TouchEvent::End { id: 7, pos: RIGHT }]);
        assert_eq!((r.line, f.tap), (0.0, None));
        // The next one steers.
        run.press(&mut r, 8, RIGHT, dt, dt);
        assert!((r.line - 0.5).abs() < 1e-9);
    }

    #[test]
    fn a_touch_never_switches_autodrive_off() {
        let mut r = line_rider();
        let mut run = Run::new();
        run.press(&mut r, 1, LEFT, 0.6, 1.0 / 60.0);
        run.press(&mut r, 2, RIGHT, 0.1, 1.0 / 60.0);
        assert!(r.autodrive);
        assert_eq!(r.lean, 0.0, "the line moved, no lean");
    }

    #[test]
    fn two_fingers_moving_together_are_the_camera_not_the_steering() {
        let dt = 1.0 / 60.0;
        let mut r = line_rider();
        let mut run = Run::new();
        let (a, b) = (Pos::new(300.0, 400.0), Pos::new(500.0, 400.0));
        run.frame(
            &mut r,
            dt,
            &[
                TouchEvent::Start { id: 1, pos: a },
                TouchEvent::Start { id: 2, pos: b },
            ],
        );
        let steered = r.line;
        // Dragged right by 20 px each: the camera, 20 px.
        let f = run.frame(
            &mut r,
            dt,
            &[
                TouchEvent::Move {
                    id: 1,
                    pos: Pos::new(320.0, 400.0),
                },
                TouchEvent::Move {
                    id: 2,
                    pos: Pos::new(520.0, 400.0),
                },
            ],
        );
        assert_eq!(f.camera_drag, (20.0, 0.0));
        assert!(!f.left_held, "camera fingers do not steer");
        let f = run.frame(
            &mut r,
            dt,
            &[
                TouchEvent::Move {
                    id: 1,
                    pos: Pos::new(270.0, 400.0),
                },
                TouchEvent::Move {
                    id: 2,
                    pos: Pos::new(570.0, 400.0),
                },
            ],
        );
        // Spread from 200 to 300 px: closer (positive wheel steps).
        assert!((f.camera_zoom - (1.5f64).ln() / 0.12).abs() < 1e-4, "{f:?}");
        assert_eq!(f.camera_drag, (0.0, 0.0));
        // One lifted: the other stays camera until it is lifted.
        let f = run.frame(
            &mut r,
            dt,
            &[TouchEvent::End {
                id: 1,
                pos: Pos::new(270.0, 400.0),
            }],
        );
        assert!(!f.left_held && f.tap.is_none());
        assert!(
            (r.line - steered).abs() < 1e-9,
            "only the first frame steered"
        );
        // One finger only steers (no camera).
        let mut r = line_rider();
        let mut run = Run::new();
        run.frame(&mut r, dt, &[TouchEvent::Start { id: 1, pos: LEFT }]);
        let f = run.frame(
            &mut r,
            dt,
            &[TouchEvent::Move {
                id: 1,
                pos: Pos::new(LEFT.x + 60.0, LEFT.y),
            }],
        );
        assert_eq!(f.camera_drag, (0.0, 0.0));
        assert!(f.left_held);
    }

    fn fan() -> Fan {
        // The overlay of `ocw_render::junction` at 1280 × 720.
        Fan {
            rect: Rect::new(490.0, 554.0, 790.0, 714.0),
            pivot: Pos::new(640.0, 662.0),
            arrow_len: 82.0,
        }
    }

    fn arrow_at(angle_deg: f32, r: f32) -> Pos {
        let a = angle_deg.to_radians();
        Pos::new(640.0 + r * a.sin(), 662.0 - r * a.cos())
    }

    #[test]
    fn the_fan_arrows_are_tap_targets() {
        let angles = [-90.0, 0.0, 60.0];
        let f = fan();
        assert_eq!(fan_hit(&f, &angles, arrow_at(-88.0, 60.0)), Some(0));
        assert_eq!(fan_hit(&f, &angles, arrow_at(5.0, 60.0)), Some(1));
        assert_eq!(fan_hit(&f, &angles, arrow_at(55.0, 50.0)), Some(2));
        // Between two arrows (far from both), at the pivot, outside: none.
        assert_eq!(fan_hit(&f, &angles, arrow_at(-45.0, 60.0)), None);
        assert_eq!(fan_hit(&f, &angles, Pos::new(641.0, 660.0)), None);
        assert_eq!(fan_hit(&f, &angles, Pos::new(300.0, 600.0)), None);
        // Close arrows split the gap.
        let close = [-10.0, 10.0];
        assert_eq!(fan_hit(&f, &close, arrow_at(-3.0, 60.0)), Some(0));
        assert_eq!(fan_hit(&f, &close, arrow_at(4.0, 60.0)), Some(1));
    }

    #[test]
    fn a_fan_tap_chooses_the_exit_and_never_steers() {
        let dt = 1.0 / 60.0;
        let mut r = line_rider();
        let mut steer = TouchSteer::default();
        let mut zones = TouchZones::new(W, H);
        let f = fan();
        zones.excluded.push(f.rect);
        let angles = [-90.0, 0.0, 90.0];
        // Lower right of the fan (the right exit), inside the right zone's
        // area by x.
        let p = arrow_at(88.0, 80.0);
        steer.event(TouchEvent::Start { id: 1, pos: p }, 0.0);
        let out = steer.update(&zones, Some((&f, &angles)), &mut r, dt);
        assert_eq!(out.exit, Some(2));
        assert!(!out.right_held);
        steer.event(TouchEvent::End { id: 1, pos: p }, 0.05);
        let out = steer.update(&zones, Some((&f, &angles)), &mut r, dt);
        assert_eq!((out.tap, r.line), (None, 0.0));
        // The halves keep steering while the fan shows (Q5).
        steer.event(TouchEvent::Start { id: 2, pos: RIGHT }, 0.1);
        let out = steer.update(&zones, Some((&f, &angles)), &mut r, dt);
        assert_eq!(out.exit, None);
        assert!(out.right_held && r.line > 0.0);
    }

    #[test]
    fn presence_follows_the_last_input_device() {
        let phone = TouchFacts {
            max_touch_points: 5,
            coarse_pointer: true,
            ipad_as_mac: false,
        };
        assert!(TouchPresence::new(phone).shown(TouchSetting::Auto));
        let ipad = TouchFacts {
            ipad_as_mac: true,
            ..TouchFacts::default()
        };
        assert!(ipad.touch_first());
        // A hybrid laptop: a fine primary pointer with a touch screen.
        let laptop = TouchFacts {
            max_touch_points: 10,
            coarse_pointer: false,
            ipad_as_mac: false,
        };
        let mut p = TouchPresence::new(laptop);
        assert!(!p.shown(TouchSetting::Auto));
        p.touch(1.0, Pos::new(100.0, 100.0));
        assert!(p.shown(TouchSetting::Auto));
        // The emulated mouse (within 500 ms, near): still shown.
        p.click(1.2, Some(Pos::new(102.0, 101.0)));
        p.mouse_move(1.3, 10.0, Some(Pos::new(105.0, 100.0)));
        assert!(p.shown(TouchSetting::Auto));
        // 3 px of jitter later: shown; 4 px: hidden.
        p.mouse_move(3.0, 3.0, None);
        assert!(p.shown(TouchSetting::Auto));
        p.mouse_move(3.1, 1.0, None);
        assert!(!p.shown(TouchSetting::Auto));
        // A touch shows them again, a key hides them, a click too.
        p.touch(4.0, Pos::new(10.0, 10.0));
        p.key();
        assert!(!p.shown(TouchSetting::Auto));
        p.touch(5.0, Pos::new(10.0, 10.0));
        p.click(6.0, Some(Pos::new(10.0, 10.0)));
        assert!(!p.shown(TouchSetting::Auto));
        // The setting overrides.
        assert!(p.shown(TouchSetting::On));
        p.touch(7.0, Pos::new(10.0, 10.0));
        assert!(!p.shown(TouchSetting::Off));
        // The chevrons dim after 8 s without a touch.
        assert!(!p.dimmed(14.0));
        assert!(p.dimmed(15.0));
        assert_eq!(TouchSetting::parse("off"), TouchSetting::Off);
        assert_eq!(TouchSetting::parse("auto"), TouchSetting::Auto);
    }
}

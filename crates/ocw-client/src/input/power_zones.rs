//! Faster / slower by touching the top / bottom of the screen (spec 3.2.3
//! *Faster / slower zones*; they replaced the touch + / − / stop buttons),
//! pure and shared by both apps:
//!
//! - [`PowerZones`]: the **top quarter** of the safe area is *faster*, the
//!   **bottom quarter** *slower* – the full width, without the 16 px
//!   window edges, the safe-area insets (notch, home indicator) and the
//!   on-screen buttons and tap targets ([`PowerZones::excluded`]: the
//!   menu and hide buttons, the junction fan). The half in between keeps
//!   steering ([`PowerZones::band`] limits `TouchZones`) and stays free
//!   for the camera;
//! - [`PowerTouch`]: the state machine. **One finger alone** counts: a
//!   **tap** (lifted before [`POWER_HOLD_S`], moved less than
//!   `TAP_MAX_MOVE_PX`) is one step when it lifts; a finger **held** that
//!   long repeats (the ramp) while it stays in its zone. A second finger
//!   (a pinch, the two-finger camera), a drag before the hold began, a
//!   cancelled touch or the zones going off (the menu, a page panel, a
//!   power device) end it without a step; fingers already down when the
//!   zones come on count only after all were lifted.
//!
//! What a step is (`ThrottleKeys`): faster = ↑ (25 W, held: the ramp),
//! slower = 10 W less, and a stop once no keyboard power is left
//! (`KeyThrottle`). The Bevy glue is `ocw_ui::touch`.

use std::collections::BTreeSet;

use super::touch::{EDGE_PX, Insets, Pos, Rect, TAP_MAX_MOVE_PX, TAP_MAX_S, TouchEvent};

/// The share of the safe area's height each zone takes (top: faster,
/// bottom: slower).
pub const POWER_ZONE_FRAC: f32 = 0.25;
/// A finger down this long (s) without moving is a hold: it repeats.
pub const POWER_HOLD_S: f64 = TAP_MAX_S;

/// What a zone does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerDir {
    /// The top of the screen.
    Faster,
    /// The bottom of the screen.
    Slower,
}

/// The faster / slower zones of a view (logical px).
#[derive(Debug, Clone, PartialEq)]
pub struct PowerZones {
    pub width: f32,
    pub height: f32,
    pub insets: Insets,
    /// On-screen buttons and tap targets: never faster / slower.
    pub excluded: Vec<Rect>,
}

impl PowerZones {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width,
            height,
            insets: Insets::default(),
            excluded: Vec::new(),
        }
    }

    /// Where the faster zone ends and the slower zone starts (px from the
    /// top): a quarter of the safe area's height from its top and bottom.
    /// Steering happens only in between.
    pub fn band(&self) -> (f32, f32) {
        let safe = (self.height - self.insets.top - self.insets.bottom).max(0.0);
        let zone = safe * POWER_ZONE_FRAC;
        (
            self.insets.top + zone,
            self.height - self.insets.bottom - zone,
        )
    }

    /// The zone's rectangle (edges and insets taken off; excluded targets
    /// are not cut out).
    pub fn rect(&self, dir: PowerDir) -> Rect {
        let (faster_end, slower_start) = self.band();
        let left = EDGE_PX.max(self.insets.left);
        let right = self.width - EDGE_PX.max(self.insets.right);
        match dir {
            PowerDir::Faster => Rect::new(left, EDGE_PX.max(self.insets.top), right, faster_end),
            PowerDir::Slower => Rect::new(
                left,
                slower_start,
                right,
                self.height - EDGE_PX.max(self.insets.bottom),
            ),
        }
    }

    /// The zone at `p`, if any.
    pub fn zone_at(&self, p: Pos) -> Option<PowerDir> {
        if self.excluded.iter().any(|r| r.contains(p)) {
            return None;
        }
        // The band's lines themselves belong to the steering.
        let (faster_end, slower_start) = self.band();
        if p.y >= faster_end && p.y <= slower_start {
            return None;
        }
        [PowerDir::Faster, PowerDir::Slower]
            .into_iter()
            .find(|d| self.rect(*d).contains(p))
    }
}

/// What the zones' finger did this frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct PowerFrame {
    /// A tap lifted: one step.
    pub tap: Option<PowerDir>,
    /// A hold began (faster: the first step of the ramp).
    pub hold_start: Option<PowerDir>,
    /// The zone held now (the ramp).
    pub held: Option<PowerDir>,
    /// Where the tap or the held finger is (for the feedback).
    pub at: Option<Pos>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Finger {
    id: u64,
    dir: PowerDir,
    start: Pos,
    pos: Pos,
    down_s: f64,
    holding: bool,
}

/// The faster / slower zones' state machine (see the module docs).
#[derive(Debug, Clone, Default)]
pub struct PowerTouch {
    /// Every finger down (any role: a second one makes it a gesture).
    down: BTreeSet<u64>,
    finger: Option<Finger>,
    events: Vec<(TouchEvent, f64)>,
}

impl PowerTouch {
    /// Queues a touch event at game time `now` (s).
    pub fn event(&mut self, e: TouchEvent, now: f64) {
        self.events.push((e, now));
    }

    /// Applies the frame's events at game time `now`. `zones`: `None`
    /// while the zones are off (not riding, the menu or a page panel
    /// open, a power device connected, the setting *off*).
    pub fn update(&mut self, zones: Option<&PowerZones>, now: f64) -> PowerFrame {
        let mut out = PowerFrame::default();
        for (e, at) in std::mem::take(&mut self.events) {
            match e {
                TouchEvent::Start { id, pos } => {
                    let alone = self.down.is_empty();
                    self.down.insert(id);
                    // A second finger: a gesture (pinch, camera), never a
                    // step – for the first finger too.
                    self.finger = match zones.and_then(|z| z.zone_at(pos)) {
                        Some(dir) if alone => Some(Finger {
                            id,
                            dir,
                            start: pos,
                            pos,
                            down_s: at,
                            holding: false,
                        }),
                        _ => None,
                    };
                }
                TouchEvent::Move { id, pos } => {
                    if let Some(f) = self.finger.as_mut().filter(|f| f.id == id) {
                        f.pos = pos;
                        let dragged = !f.holding && pos.distance(f.start) >= TAP_MAX_MOVE_PX;
                        let left = zones.is_none_or(|z| z.zone_at(pos) != Some(f.dir));
                        if dragged || left {
                            self.finger = None;
                        }
                    }
                }
                TouchEvent::End { id, pos } => {
                    self.down.remove(&id);
                    if let Some(f) = self.finger.take_if(|f| f.id == id) {
                        let tap = !f.holding
                            && pos.distance(f.start) < TAP_MAX_MOVE_PX
                            && zones.is_some_and(|z| z.zone_at(pos) == Some(f.dir));
                        if tap {
                            out.tap = Some(f.dir);
                            out.at = Some(pos);
                        }
                    }
                }
                TouchEvent::Cancel { id } => {
                    self.down.remove(&id);
                    self.finger.take_if(|f| f.id == id);
                }
            }
        }
        if zones.is_none() {
            self.finger = None;
        }
        if let Some(f) = self.finger.as_mut() {
            if !f.holding && now - f.down_s >= POWER_HOLD_S {
                f.holding = true;
                out.hold_start = Some(f.dir);
            }
            if f.holding {
                out.held = Some(f.dir);
                out.at = Some(f.pos);
            }
        }
        out
    }

    /// The zone held now.
    pub fn held(&self) -> Option<PowerDir> {
        self.finger.filter(|f| f.holding).map(|f| f.dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn start(id: u64, x: f32, y: f32) -> TouchEvent {
        TouchEvent::Start {
            id,
            pos: Pos::new(x, y),
        }
    }
    fn mv(id: u64, x: f32, y: f32) -> TouchEvent {
        TouchEvent::Move {
            id,
            pos: Pos::new(x, y),
        }
    }
    fn end(id: u64, x: f32, y: f32) -> TouchEvent {
        TouchEvent::End {
            id,
            pos: Pos::new(x, y),
        }
    }

    /// An iPhone in portrait (430 × 932, notch 59, home bar 34) and turned
    /// (932 × 430, notch left / right 59, home bar 21), a tablet both ways
    /// and a desktop window: the top quarter of the safe area is faster,
    /// the bottom quarter slower, the half between neither; nothing in
    /// the insets or the 16 px edges.
    #[test]
    fn the_zones_are_the_top_and_bottom_quarter_of_the_safe_area() {
        let cases = [
            (430.0, 932.0, (59.0, 0.0, 34.0, 0.0)),
            (932.0, 430.0, (0.0, 59.0, 21.0, 59.0)),
            (820.0, 1180.0, (24.0, 0.0, 20.0, 0.0)),
            (1180.0, 820.0, (24.0, 0.0, 20.0, 0.0)),
            (1280.0, 720.0, (0.0, 0.0, 0.0, 0.0)),
        ];
        for (w, h, (top, right, bottom, left)) in cases {
            let mut z = PowerZones::new(w, h);
            z.insets = Insets {
                top,
                right,
                bottom,
                left,
            };
            let safe = h - top - bottom;
            let (a, b) = z.band();
            assert!((a - (top + safe / 4.0)).abs() < 1e-3, "{w}x{h}");
            assert!((b - (h - bottom - safe / 4.0)).abs() < 1e-3, "{w}x{h}");
            let at = |x: f32, y: f32| z.zone_at(Pos::new(x, y));
            let mid = w / 2.0;
            // Inside each zone, over the whole width.
            for x in [left.max(16.0) + 1.0, mid, w - right.max(16.0) - 1.0] {
                assert_eq!(at(x, a - 2.0), Some(PowerDir::Faster), "{w}x{h} {x}");
                assert_eq!(at(x, top.max(16.0) + 1.0), Some(PowerDir::Faster));
                assert_eq!(at(x, b + 2.0), Some(PowerDir::Slower), "{w}x{h} {x}");
                assert_eq!(at(x, h - bottom.max(16.0) - 1.0), Some(PowerDir::Slower));
            }
            // The half between: free (steering, the camera).
            for y in [a, a + 1.0, h / 2.0, b - 1.0, b] {
                assert_eq!(at(mid, y), None, "{w}x{h} {y}");
            }
            // The notch, the home bar and the window edges.
            assert_eq!(at(mid, top.max(16.0) - 1.0), None, "{w}x{h}: the notch");
            assert_eq!(at(mid, h - bottom.max(16.0) + 1.0), None, "{w}x{h}");
            assert_eq!(at(left.max(16.0) - 1.0, a - 2.0), None, "{w}x{h}");
            assert_eq!(at(w - right.max(16.0) + 1.0, b + 2.0), None, "{w}x{h}");
            // A zone is tall enough to hit (at least 80 px on these).
            for d in [PowerDir::Faster, PowerDir::Slower] {
                let r = z.rect(d);
                assert!(r.max.y - r.min.y >= 80.0, "{w}x{h} {d:?}");
            }
        }
    }

    #[test]
    fn buttons_and_tap_targets_are_cut_out_of_the_zones() {
        let mut z = PowerZones::new(430.0, 932.0);
        // The menu and hide buttons, the junction fan.
        z.excluded.push(Rect::new(12.0, 150.0, 116.0, 198.0));
        z.excluded.push(Rect::new(140.0, 760.0, 290.0, 900.0));
        assert_eq!(z.zone_at(Pos::new(30.0, 170.0)), None);
        assert_eq!(
            z.zone_at(Pos::new(200.0, 170.0)),
            Some(PowerDir::Faster),
            "beside the buttons"
        );
        assert_eq!(z.zone_at(Pos::new(200.0, 800.0)), None, "the fan");
        assert_eq!(z.zone_at(Pos::new(60.0, 800.0)), Some(PowerDir::Slower));
        // The steering zones end where the faster / slower zones begin.
        let mut steer = super::super::touch::TouchZones::new(430.0, 932.0);
        steer.band = Some(z.band());
        use super::super::touch::Side;
        assert_eq!(steer.zone_at(Pos::new(60.0, 200.0)), None);
        assert_eq!(steer.zone_at(Pos::new(60.0, 800.0)), None);
        assert_eq!(steer.zone_at(Pos::new(60.0, 466.0)), Some(Side::Left));
        assert_eq!(steer.zone_at(Pos::new(370.0, 600.0)), Some(Side::Right));
        // Without the band (a power device) the halves take the height.
        steer.band = None;
        assert_eq!(steer.zone_at(Pos::new(60.0, 800.0)), Some(Side::Left));
    }

    #[test]
    fn a_tap_is_one_step_when_it_lifts() {
        let z = PowerZones::new(430.0, 932.0);
        let mut t = PowerTouch::default();
        t.event(start(1, 200.0, 100.0), 1.0);
        let f = t.update(Some(&z), 1.0);
        assert_eq!(f, PowerFrame::default(), "nothing on touch-down");
        t.event(end(1, 203.0, 102.0), 1.1);
        let f = t.update(Some(&z), 1.1);
        assert_eq!(f.tap, Some(PowerDir::Faster));
        assert_eq!(f.at, Some(Pos::new(203.0, 102.0)));
        assert_eq!((f.hold_start, f.held), (None, None));
        // The bottom: slower. Down and up within one frame counts too.
        t.event(start(2, 200.0, 850.0), 2.0);
        t.event(end(2, 200.0, 850.0), 2.01);
        assert_eq!(t.update(Some(&z), 2.02).tap, Some(PowerDir::Slower));
        // The middle: nothing.
        t.event(start(3, 200.0, 466.0), 3.0);
        t.event(end(3, 200.0, 466.0), 3.05);
        assert_eq!(t.update(Some(&z), 3.05), PowerFrame::default());
    }

    #[test]
    fn a_hold_repeats_until_it_lifts_or_leaves_the_zone() {
        let z = PowerZones::new(430.0, 932.0);
        let mut t = PowerTouch::default();
        t.event(start(1, 200.0, 100.0), 1.0);
        assert_eq!(t.update(Some(&z), 1.0).held, None);
        assert_eq!(t.update(Some(&z), 1.24).held, None, "not yet a hold");
        let f = t.update(Some(&z), 1.25);
        assert_eq!(
            (f.hold_start, f.held),
            (Some(PowerDir::Faster), Some(PowerDir::Faster))
        );
        let f = t.update(Some(&z), 1.5);
        assert_eq!((f.hold_start, f.held), (None, Some(PowerDir::Faster)));
        assert_eq!(t.held(), Some(PowerDir::Faster));
        // A held finger may wander inside its zone …
        t.event(mv(1, 260.0, 120.0), 1.6);
        assert_eq!(t.update(Some(&z), 1.6).held, Some(PowerDir::Faster));
        // … lifting ends the hold without another step.
        t.event(end(1, 260.0, 120.0), 2.0);
        let f = t.update(Some(&z), 2.0);
        assert_eq!((f.tap, f.held), (None, None));
        // Sliding out of the zone ends it too.
        t.event(start(2, 200.0, 850.0), 3.0);
        t.update(Some(&z), 3.0);
        assert_eq!(t.update(Some(&z), 3.3).held, Some(PowerDir::Slower));
        t.event(mv(2, 200.0, 600.0), 3.4);
        assert_eq!(t.update(Some(&z), 3.4).held, None);
        t.event(end(2, 200.0, 600.0), 3.5);
        assert_eq!(t.update(Some(&z), 3.5), PowerFrame::default());
    }

    #[test]
    fn drags_and_second_fingers_are_gestures_not_steps() {
        let z = PowerZones::new(932.0, 430.0);
        let mut t = PowerTouch::default();
        // A drag (a scroll attempt, the start of a camera move): 10 px.
        t.event(start(1, 400.0, 60.0), 1.0);
        t.event(mv(1, 409.0, 60.0), 1.05);
        t.update(Some(&z), 1.05);
        t.event(mv(1, 411.0, 60.0), 1.1);
        t.update(Some(&z), 1.1);
        assert_eq!(t.update(Some(&z), 1.6).held, None, "dragged: no hold");
        t.event(end(1, 400.0, 60.0), 1.7);
        assert_eq!(t.update(Some(&z), 1.7).tap, None, "dragged: no tap");
        // A pinch: the second finger ends the first's tap and hold.
        t.event(start(2, 300.0, 60.0), 2.0);
        t.update(Some(&z), 2.0);
        t.event(start(3, 600.0, 380.0), 2.1);
        t.update(Some(&z), 2.1);
        assert_eq!(t.update(Some(&z), 2.6).held, None);
        t.event(end(2, 300.0, 60.0), 2.7);
        assert_eq!(t.update(Some(&z), 2.7).tap, None);
        // The second finger lifted alone in the other zone: nothing.
        t.event(end(3, 600.0, 380.0), 2.8);
        assert_eq!(t.update(Some(&z), 2.8).tap, None);
        // A finger while another (steering) one is down: nothing.
        t.event(start(4, 200.0, 215.0), 3.0);
        t.event(start(5, 300.0, 60.0), 3.1);
        t.event(end(5, 300.0, 60.0), 3.2);
        assert_eq!(t.update(Some(&z), 3.2).tap, None);
        t.event(end(4, 200.0, 215.0), 3.3);
        t.update(Some(&z), 3.3);
        // All lifted: the next tap counts again.
        t.event(start(6, 300.0, 60.0), 4.0);
        t.event(end(6, 300.0, 60.0), 4.1);
        assert_eq!(t.update(Some(&z), 4.1).tap, Some(PowerDir::Faster));
        // A cancelled touch (the system took it): nothing.
        t.event(start(7, 300.0, 60.0), 5.0);
        t.event(TouchEvent::Cancel { id: 7 }, 5.1);
        assert_eq!(t.update(Some(&z), 5.1), PowerFrame::default());
    }

    #[test]
    fn nothing_while_the_zones_are_off_and_no_carry_over() {
        let z = PowerZones::new(430.0, 932.0);
        let mut t = PowerTouch::default();
        // Down while the menu is open, lifted after it closed: nothing.
        t.event(start(1, 200.0, 100.0), 1.0);
        assert_eq!(t.update(None, 1.0), PowerFrame::default());
        t.event(end(1, 200.0, 100.0), 1.1);
        assert_eq!(t.update(Some(&z), 1.1), PowerFrame::default());
        // Down in a zone, then the menu opens: the hold ends, no tap.
        t.event(start(2, 200.0, 100.0), 2.0);
        t.update(Some(&z), 2.0);
        assert_eq!(t.update(Some(&z), 2.3).held, Some(PowerDir::Faster));
        assert_eq!(t.update(None, 2.4).held, None);
        t.event(end(2, 200.0, 100.0), 2.5);
        assert_eq!(t.update(Some(&z), 2.5), PowerFrame::default());
        // A finger from before the zones came on blocks until lifted.
        t.event(start(3, 200.0, 500.0), 3.0);
        t.update(None, 3.0);
        t.event(start(4, 200.0, 100.0), 3.1);
        t.event(end(4, 200.0, 100.0), 3.2);
        assert_eq!(t.update(Some(&z), 3.2).tap, None);
    }
}

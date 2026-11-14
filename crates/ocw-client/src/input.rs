//! Pure keyboard and gamepad logic of the ride controls (spec §3.2.2):
//! power ramps, steering and lean ramps, junction keys, trigger rest
//! values, the keyboard power ([`KeyThrottle`]). The Bevy systems that
//! read the keys live in the apps (`apps/desktop/src/input.rs`,
//! `apps/web`); this module is shared.

use crate::client::ClientCore;

/// Free ride: what the junction keys do this frame (spec §2.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JunctionKey {
    /// Not a junction key: steer as usual.
    None,
    /// Move the exit selection by this many exits (negative = left).
    Select(i32),
}

/// Maps the keys to a junction action: Q / E always; the arrows only in
/// autodrive while a junction is active (then they do not move the line).
/// Returns the action and whether the arrows are taken by the junction.
pub fn junction_key(
    q: bool,
    e: bool,
    left: bool,
    right: bool,
    autodrive: bool,
    junction_active: bool,
) -> (JunctionKey, bool) {
    let arrows = autodrive && junction_active;
    let mut steps = i32::from(e) - i32::from(q);
    if arrows {
        steps += i32::from(right) - i32::from(left);
    }
    let key = if steps == 0 {
        JunctionKey::None
    } else {
        JunctionKey::Select(steps)
    };
    (key, arrows)
}
/// Power added by each press of ↑.
pub const POWER_STEP_W: f64 = 25.0;
/// Power rise per second while ↑ is held.
pub const POWER_RAMP_W_PER_S: f64 = 250.0;
/// Highest keyboard power reachable with ↑.
pub const MAX_KEY_POWER_W: f64 = 1000.0;
/// Minimum time between power updates while ↑ is held.
pub const KEY_SEND_INTERVAL_S: f64 = 0.1;

/// New power target after `dt` seconds: `pressed` = ↑ went down this frame
/// (a step), `held` = ↑ is down (a ramp).
pub fn ramp(watts: f64, pressed: bool, held: bool, dt: f64) -> f64 {
    let step = if pressed { POWER_STEP_W } else { 0.0 };
    let rise = if held { POWER_RAMP_W_PER_S * dt } else { 0.0 };
    if step + rise == 0.0 {
        return watts;
    }
    (watts + step + rise).min(MAX_KEY_POWER_W.max(watts))
}

/// How fast the autodrive line moves while an arrow key is held, in m/s.
pub const LINE_KEY_RATE_MPS: f64 = 2.0;
/// Gamepad stick values below this are treated as centred.
pub const STICK_DEAD_ZONE: f32 = 0.08;

/// Lean (rad) for a gamepad stick deflection (−1 … 1): proportional, so
/// a centred stick rides upright.
pub fn stick_lean(x: f32) -> f64 {
    let x = if x.abs() < STICK_DEAD_ZONE {
        0.0
    } else {
        x.clamp(-1.0, 1.0)
    };
    f64::from(x) * ocw_sim::physics::MAX_STEER_LEAN
}

/// How fast the lean changes while an arrow key is held, in rad/s (~23°/s).
pub const LEAN_KEY_RATE: f64 = 0.4;

/// How long a held arrow key keeps the rider upright when the lean comes
/// back through it, before it leans on to the other side (s).
pub const LEAN_UPRIGHT_PAUSE_S: f64 = 0.3;

/// New lean (rad) after `dt` with the arrow key direction `dir`. Bringing
/// the lean back through upright stops there for [`LEAN_UPRIGHT_PAUSE_S`]
/// (`pause`: the time left), so straightening up is easy; held longer, it
/// leans on to the other side. Releasing the keys ends the pause.
pub fn lean_ramp(lean: f64, dir: f64, pause: &mut f64, dt: f64) -> f64 {
    if dir == 0.0 {
        *pause = 0.0;
        return lean;
    }
    if *pause > 0.0 {
        *pause -= dt;
        return lean;
    }
    let max = ocw_sim::physics::MAX_STEER_LEAN;
    let next = (lean + dir * LEAN_KEY_RATE * dt).clamp(-max, max);
    if lean != 0.0 && (next == 0.0 || next.signum() != lean.signum()) {
        *pause = LEAN_UPRIGHT_PAUSE_S;
        return 0.0;
    }
    next
}

/// Steering direction from the arrow keys: −1 left, +1 right, 0 none.
pub fn steer_direction(left: bool, right: bool) -> f64 {
    f64::from(i8::from(right) - i8::from(left))
}

/// Power change in watts from the keys pressed this frame.
pub fn power_delta(up: bool, down: bool) -> f64 {
    10.0 * f64::from(i8::from(up) - i8::from(down))
}

/// Analog trigger / stick values above this count as pressed.
pub const PAD_PRESS: f32 = 0.35;

/// "Faster" and "stop" from one gamepad: D-pad up / down, left stick up /
/// down, right / left trigger. Triggers and stick are read as analog values
/// (many pads report the triggers as axes, so their button state alone is
/// not reliable).
pub fn pad_throttle(
    stick_y: f32,
    dpad_up: bool,
    dpad_down: bool,
    rt: f32,
    lt: f32,
) -> (bool, bool) {
    let up = dpad_up || stick_y > PAD_PRESS + 0.15 || rt > PAD_PRESS;
    let down = dpad_down || stick_y < -(PAD_PRESS + 0.15) || lt > PAD_PRESS;
    (up && !down, down)
}

/// How far an analog trigger is pulled beyond its resting value. Pads and
/// drivers differ in what a released trigger reads (0, 0.5, −1 …); the
/// lowest value seen is taken as rest, so a released trigger never counts as
/// pressed. Regression: a pad whose released left trigger read ~0.5 kept the
/// brake on (power 0) from the start, so the trainer's power had no effect.
pub fn trigger_pull(value: f32, rest: &mut Option<f32>) -> f32 {
    let r = rest.map_or(value, |r| r.min(value));
    *rest = Some(r);
    value - r
}

/// The power keys of one frame (keyboard and gamepad together).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ThrottleKeys {
    /// ↑ went down this frame (a step).
    pub up_pressed: bool,
    /// ↑ is down (a ramp).
    pub up_held: bool,
    /// ↓ went down this frame: stop.
    pub down_pressed: bool,
    pub page_up: bool,
    pub page_down: bool,
}

/// Keyboard power (↑ / ↓, PageUp / PageDown): added to the rider's device
/// power (virtual or real trainer), so the keys work with any device. The
/// same rules in the desktop and the browser app.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct KeyThrottle {
    pub watts: f64,
    pub last_sent_s: f64,
}

impl KeyThrottle {
    /// Applies one frame's keys (`dt` s long, at game time `now`) to the
    /// client: ↓ stops (no keyboard power, brake), ↑ releases the brake
    /// and ramps the power. Returns the messages for the server and
    /// whether ↓ was pressed (the app then also sets the virtual trainer's
    /// power to 0).
    pub fn update(
        &mut self,
        client: &mut ClientCore,
        keys: ThrottleKeys,
        dt: f64,
        now: f64,
    ) -> (Vec<Vec<u8>>, bool) {
        let mut out = Vec::new();
        if keys.down_pressed {
            self.watts = 0.0;
            out.extend(client.set_brake(true));
        }
        if keys.up_pressed {
            out.extend(client.set_brake(false));
        }
        self.watts = ramp(self.watts, keys.up_pressed, keys.up_held, dt);
        self.watts = (self.watts + power_delta(keys.page_up, keys.page_down)).clamp(0.0, 2500.0);
        // Send the rider's new power at most every KEY_SEND_INTERVAL_S
        // while it changes, and at once when it reached 0.
        let changed = (self.watts - client.key_power_w()).abs() > 1e-9;
        if changed && (now - self.last_sent_s >= KEY_SEND_INTERVAL_S || self.watts == 0.0) {
            self.last_sent_s = now;
            out.extend(client.set_key_power(self.watts));
        }
        (out, keys.down_pressed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_released_trigger_never_counts_as_pressed() {
        // Regression: a pad whose released trigger reads 0.5 (centred axis
        // mapping) kept "stop" pressed and the brake on.
        let mut rest = None;
        assert_eq!(trigger_pull(0.5, &mut rest), 0.0);
        assert_eq!(
            pad_throttle(0.0, false, false, 0.0, trigger_pull(0.5, &mut rest)),
            (false, false)
        );
        // Pulled fully: pressed.
        assert!(trigger_pull(1.0, &mut rest) > PAD_PRESS);
        // A pad reading −1 at rest, +1 pulled.
        let mut rest = None;
        assert_eq!(trigger_pull(-1.0, &mut rest), 0.0);
        assert!(trigger_pull(1.0, &mut rest) > PAD_PRESS);
        // A normal pad (0 at rest): half pulled counts.
        let mut rest = None;
        trigger_pull(0.0, &mut rest);
        assert!(trigger_pull(0.5, &mut rest) > PAD_PRESS);
        // Pulled at the first reading: counts once the trigger was released.
        let mut rest = None;
        assert_eq!(trigger_pull(1.0, &mut rest), 0.0);
        trigger_pull(0.0, &mut rest);
        assert!(trigger_pull(1.0, &mut rest) > PAD_PRESS);
    }

    #[test]
    fn gamepad_faster_and_stop_from_dpad_stick_and_triggers() {
        // Nothing pressed, small stick and trigger noise: nothing.
        assert_eq!(pad_throttle(0.2, false, false, 0.1, 0.1), (false, false));
        // Faster: D-pad up, stick up, right trigger half pressed.
        assert_eq!(pad_throttle(0.0, true, false, 0.0, 0.0), (true, false));
        assert_eq!(pad_throttle(0.9, false, false, 0.0, 0.0), (true, false));
        assert_eq!(pad_throttle(0.0, false, false, 0.5, 0.0), (true, false));
        // Stop: D-pad down, stick down, left trigger; stop wins over faster.
        assert_eq!(pad_throttle(0.0, false, true, 0.0, 0.0), (false, true));
        assert_eq!(pad_throttle(-0.9, false, false, 0.0, 0.0), (false, true));
        assert_eq!(pad_throttle(0.0, false, false, 0.0, 0.6), (false, true));
        assert_eq!(pad_throttle(0.0, true, false, 0.0, 0.9), (false, true));
    }

    #[test]
    fn junction_keys_take_the_arrows_only_in_autodrive_near_a_junction() {
        use JunctionKey::*;
        // Q / E always.
        assert_eq!(
            junction_key(true, false, false, false, false, false),
            (Select(-1), false)
        );
        assert_eq!(
            junction_key(false, true, false, false, true, false),
            (Select(1), false)
        );
        // Arrows: only with autodrive and an active junction.
        assert_eq!(
            junction_key(false, false, true, false, true, true),
            (Select(-1), true)
        );
        assert_eq!(
            junction_key(false, false, false, true, true, false),
            (None, false)
        );
        // Manual steering keeps the arrows for leaning.
        assert_eq!(
            junction_key(false, false, false, true, false, true),
            (None, false)
        );
        // Taken even when nothing is pressed (the line does not move).
        assert_eq!(
            junction_key(false, false, false, false, true, true),
            (None, true)
        );
    }

    #[test]
    fn key_mapping() {
        assert_eq!(steer_direction(true, false), -1.0);
        assert_eq!(steer_direction(false, true), 1.0);
        assert_eq!(steer_direction(true, true), 0.0);
        assert_eq!(steer_direction(false, false), 0.0);
    }

    #[test]
    fn stick_leans_proportionally_with_a_dead_zone() {
        assert_eq!(stick_lean(0.05), 0.0);
        assert_eq!(stick_lean(-0.05), 0.0);
        assert!((stick_lean(0.5) - 0.5 * ocw_sim::physics::MAX_STEER_LEAN).abs() < 1e-6);
        assert_eq!(stick_lean(-3.0), -ocw_sim::physics::MAX_STEER_LEAN);
    }

    #[test]
    fn lean_keys_ramp_hold_and_pause_at_upright() {
        let frame = 1.0 / 60.0;
        let mut pause = 0.0;
        // Hold right for half a second: leaning right.
        let mut lean = 0.0;
        for _ in 0..30 {
            lean = lean_ramp(lean, 1.0, &mut pause, frame);
        }
        assert!((lean - 0.2).abs() < 1e-9, "{lean}");
        // Released: the lean stays.
        assert_eq!(lean_ramp(lean, 0.0, &mut pause, 1.0), lean);
        // Left straightens up (0.5 s) and pauses at upright while held.
        for _ in 0..(30 + 17) {
            lean = lean_ramp(lean, -1.0, &mut pause, frame);
        }
        assert_eq!(lean, 0.0);
        // Released and pressed again: it leans left at once.
        lean = lean_ramp(lean, 0.0, &mut pause, frame);
        lean = lean_ramp(lean, -1.0, &mut pause, 0.5);
        assert!(lean < 0.0);
        // Never beyond the limit.
        lean = lean_ramp(lean, -1.0, &mut pause, 100.0);
        assert_eq!(lean, -ocw_sim::physics::MAX_STEER_LEAN);
        assert_eq!(power_delta(true, false), 10.0);
        assert_eq!(power_delta(false, true), -10.0);
        assert_eq!(power_delta(true, true), 0.0);
    }

    /// Regression: holding the opposite key used to stop the lean at
    /// upright until the key was released, so a rider leaning into the
    /// road edge could not steer back out by holding it.
    #[test]
    fn held_opposite_key_leans_on_to_the_other_side() {
        let frame = 1.0 / 60.0;
        let mut pause = 0.0;
        let mut lean = ocw_sim::physics::MAX_STEER_LEAN;
        let mut upright_s = 0.0;
        // Hold left for 3 s: through upright, a short pause, then left.
        for _ in 0..180 {
            lean = lean_ramp(lean, -1.0, &mut pause, frame);
            if lean == 0.0 {
                upright_s += frame;
            }
        }
        assert!(
            (upright_s - LEAN_UPRIGHT_PAUSE_S).abs() < 2.5 * frame,
            "{upright_s}"
        );
        assert!(lean < -0.3, "{lean}");
    }

    #[test]
    fn up_ramps_the_power_and_stops_at_the_cap() {
        assert_eq!(ramp(200.0, false, false, 1.0), 200.0);
        // A short tap (one frame at 144 FPS) adds a full step.
        let tap = ramp(200.0, true, true, 1.0 / 144.0);
        assert!((tap - 226.7).abs() < 0.1, "{tap}");
        // Holding: +250 W/s.
        assert_eq!(ramp(200.0, false, true, 0.4), 300.0);
        assert_eq!(ramp(990.0, true, true, 1.0), MAX_KEY_POWER_W);
        // Above the cap (set via the console) the key does not lower it.
        assert_eq!(ramp(1500.0, false, true, 1.0), 1500.0);
    }

    #[test]
    fn throttle_keys_add_power_and_down_stops() {
        let mut client = ClientCore::new(
            ocw_world::Route::test_loop(),
            "K",
            ocw_core::RiderProfile::default(),
        );
        let mut t = KeyThrottle::default();
        let up = ThrottleKeys {
            up_pressed: true,
            up_held: true,
            ..ThrottleKeys::default()
        };
        // A tap: a step, sent at once.
        let (msgs, stop) = t.update(&mut client, up, 0.0, 1.0);
        assert!(!stop);
        assert_eq!(msgs.len(), 1);
        assert_eq!(client.key_power_w(), POWER_STEP_W);
        // Held: ramps, but sends at most every KEY_SEND_INTERVAL_S.
        let held = ThrottleKeys {
            up_held: true,
            ..ThrottleKeys::default()
        };
        let (msgs, _) = t.update(&mut client, held, 0.02, 1.02);
        assert!(msgs.is_empty());
        assert!(t.watts > POWER_STEP_W);
        let (msgs, _) = t.update(&mut client, held, 0.1, 1.2);
        assert_eq!(msgs.len(), 1);
        // Down: no keyboard power and the brake, at once.
        let down = ThrottleKeys {
            down_pressed: true,
            ..ThrottleKeys::default()
        };
        let (msgs, stop) = t.update(&mut client, down, 0.01, 1.21);
        assert!(stop && client.braking());
        assert_eq!(client.key_power_w(), 0.0);
        assert_eq!(msgs.len(), 2);
        // Page Up adds 10 W and up releases the brake.
        let page = ThrottleKeys {
            page_up: true,
            ..ThrottleKeys::default()
        };
        t.update(&mut client, page, 0.01, 2.0);
        assert_eq!(client.key_power_w(), 10.0);
        t.update(&mut client, up, 0.0, 3.0);
        assert!(!client.braking());
    }
}

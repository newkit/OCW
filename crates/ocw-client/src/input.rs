//! Pure keyboard and gamepad logic of the ride controls (spec §3.2.2):
//! power ramps, steering and lean ramps, junction keys, trigger rest
//! values, the keyboard power ([`KeyThrottle`]). The Bevy systems that
//! read the keys live in the apps (`apps/desktop/src/input.rs`,
//! `apps/web`); this module is shared. [`touch`]: touch steering by the
//! screen's halves (spec §3.2.3; its Bevy glue is `ocw_ui::touch`).

use crate::client::ClientCore;

pub mod power_zones;
pub mod touch;

/// Free ride: what the junction keys do this frame (spec 2.2.1, 3.2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JunctionKey {
    /// Not a junction key: steer as usual.
    None,
    /// Move the exit selection by this many exits (negative = left).
    Select(i32),
}

/// The junction keys of one frame: the exit to the left (default Delete
/// or Q, the D-pad left) and to the right (Page Down or E, the D-pad
/// right), at any time and in every steering mode. ← / → are never
/// junction keys: they only steer (spec 3.2.5, 2026-11-21: "the cursor
/// keys are double used").
pub fn junction_key(left: bool, right: bool) -> JunctionKey {
    match i32::from(right) - i32::from(left) {
        0 => JunctionKey::None,
        steps => JunctionKey::Select(steps),
    }
}

/// Power added by each press of ↑.
pub const POWER_STEP_W: f64 = 25.0;
/// Power rise per second while ↑ is held.
pub const POWER_RAMP_W_PER_S: f64 = 250.0;
/// Highest keyboard power reachable with ↑.
pub const MAX_KEY_POWER_W: f64 = 1000.0;
/// Power taken by a tap on the screen's bottom zone (as the former touch
/// − button).
pub const SLOWER_STEP_W: f64 = 10.0;
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
    /// +10 W / −10 W: no key changes the power by 10 W since v5.18 (Page
    /// Up / Page Down were, spec 3.2.5); the touch − button did until
    /// the screen zones replaced it ([`Self::slower_pressed`]).
    pub plus_10: bool,
    pub minus_10: bool,
    /// A tap on the screen's bottom zone (spec 3.2.3 *Faster / slower
    /// zones*): [`SLOWER_STEP_W`] less keyboard power; with none left it
    /// stops (as ↓: the brake, power 0).
    pub slower_pressed: bool,
    /// A finger held on the bottom zone: the keyboard power ramps down
    /// (to 0, never the brake).
    pub slower_held: bool,
}

impl ThrottleKeys {
    /// Both inputs together (the keys and the touch zones).
    pub fn or(self, o: Self) -> Self {
        Self {
            up_pressed: self.up_pressed || o.up_pressed,
            up_held: self.up_held || o.up_held,
            down_pressed: self.down_pressed || o.down_pressed,
            plus_10: self.plus_10 || o.plus_10,
            minus_10: self.minus_10 || o.minus_10,
            slower_pressed: self.slower_pressed || o.slower_pressed,
            slower_held: self.slower_held || o.slower_held,
        }
    }
}

/// Keyboard power (↑ / ↓; the touch zones' steps): added to the rider's device
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
        // The bottom zone's tap: a step down; with no keyboard power left
        // it is ↓ (stop) – "slower" ends in a stop.
        let mut stop = keys.down_pressed;
        if keys.slower_pressed {
            if self.watts > 0.0 {
                self.watts = (self.watts - SLOWER_STEP_W).max(0.0);
            } else if !keys.up_pressed && !client.braking() {
                stop = true;
            }
        }
        if keys.slower_held && !keys.up_held {
            self.watts = (self.watts - POWER_RAMP_W_PER_S * dt).max(0.0);
        }
        if stop {
            self.watts = 0.0;
            out.extend(client.set_brake(true));
        }
        if keys.up_pressed {
            out.extend(client.set_brake(false));
        }
        self.watts = ramp(self.watts, keys.up_pressed, keys.up_held, dt);
        self.watts = (self.watts + power_delta(keys.plus_10, keys.minus_10)).clamp(0.0, 2500.0);
        // Send the rider's new power at most every KEY_SEND_INTERVAL_S
        // while it changes, and at once when it reached 0.
        let changed = (self.watts - client.key_power_w()).abs() > 1e-9;
        if changed && (now - self.last_sent_s >= KEY_SEND_INTERVAL_S || self.watts == 0.0) {
            self.last_sent_s = now;
            out.extend(client.set_key_power(self.watts));
        }
        (out, stop)
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
    fn junction_keys_choose_the_exit_and_never_take_the_arrows() {
        use JunctionKey::*;
        assert_eq!(junction_key(true, false), Select(-1));
        assert_eq!(junction_key(false, true), Select(1));
        assert_eq!(junction_key(true, true), None, "both: no change");
        assert_eq!(junction_key(false, false), None);
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
        // The touch +10 W adds 10 W and up releases the brake.
        let page = ThrottleKeys {
            plus_10: true,
            ..ThrottleKeys::default()
        };
        t.update(&mut client, page, 0.01, 2.0);
        assert_eq!(client.key_power_w(), 10.0);
        t.update(&mut client, up, 0.0, 3.0);
        assert!(!client.braking());
    }

    /// The screen's bottom zone (spec 3.2.3): a tap takes 10 W of the
    /// keyboard power, with none left it stops (brake, as ↓); held it
    /// ramps down to 0 and never brakes; ↑ rides on.
    #[test]
    fn slower_steps_down_then_stops_and_a_hold_ramps_down() {
        let mut client = ClientCore::new(
            ocw_world::Route::test_loop(),
            "K",
            ocw_core::RiderProfile::default(),
        );
        let mut t = KeyThrottle {
            watts: 25.0,
            ..KeyThrottle::default()
        };
        let slower = ThrottleKeys {
            slower_pressed: true,
            ..ThrottleKeys::default()
        };
        for (i, want) in [15.0, 5.0, 0.0].into_iter().enumerate() {
            let (_, stop) = t.update(&mut client, slower, 0.01, 1.0 + i as f64);
            assert!(!stop && !client.braking());
            assert_eq!(t.watts, want);
        }
        // Nothing left: the next tap stops.
        let (_, stop) = t.update(&mut client, slower, 0.01, 5.0);
        assert!(stop && client.braking());
        // Stopped already: another tap is nothing new.
        let (msgs, stop) = t.update(&mut client, slower, 0.01, 6.0);
        assert!(!stop && msgs.is_empty() && client.braking());
        // ↑ (the top zone) rides on.
        let up = ThrottleKeys {
            up_pressed: true,
            ..ThrottleKeys::default()
        };
        t.update(&mut client, up, 0.0, 7.0);
        assert!(!client.braking());
        assert_eq!(t.watts, POWER_STEP_W);
        // Held: 250 W/s down, to 0 and no further – no brake.
        t.watts = 300.0;
        let held = ThrottleKeys {
            slower_held: true,
            ..ThrottleKeys::default()
        };
        t.update(&mut client, held, 0.4, 8.0);
        assert_eq!(t.watts, 200.0);
        let (_, stop) = t.update(&mut client, held, 2.0, 9.0);
        assert!(!stop && !client.braking());
        assert_eq!(t.watts, 0.0);
        assert_eq!(client.key_power_w(), 0.0);
    }
}

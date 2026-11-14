//! Hearts (♥), the workshop's unit (spec §14 Phase 4, 4E): earned only by
//! riding, measured by the server from the power it accepted.
//!
//! Decided rates (4L Q3): **1 ♥ per 10 kJ** of mechanical work and **1 ♥
//! per 20 m** of climbing, no daily cap, 500 ♥ at the start. Never earned
//! with virtual power, in god mode, while down after a crash, by bots, in
//! the kiosk / show mode or in captures; the server ([`HeartsMeter`]) and
//! the client's wallet leave those out.

/// Work that earns one heart (J).
pub const WORK_J_PER_HEART: f64 = 10_000.0;
/// Climbing that earns one heart (m).
pub const CLIMB_M_PER_HEART: f64 = 20.0;
/// Hearts every rider starts with.
pub const START_HEARTS: u32 = 500;

/// Hearts for `work_j` joules of work and `climb_m` metres of climbing:
/// whole hearts of each, added (rounded down: a part heart waits for more).
pub fn earned(work_j: f64, climb_m: f64) -> u32 {
    let w = (work_j.max(0.0) / WORK_J_PER_HEART).floor();
    let c = (climb_m.max(0.0) / CLIMB_M_PER_HEART).floor();
    (w + c).min(f64::from(u32::MAX)) as u32
}

/// Why a tick earns nothing (or `None`: it counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoHearts {
    /// Keyboard power, `sim.power`, the virtual trainer.
    VirtualPower,
    /// Flying as the unicorn (spec §14 Phase 13).
    GodMode,
    /// Down after a crash (spec §14 Phase 12).
    Down,
    /// A server-side bot or the Echo rider.
    Bot,
}

/// A rider's work and climbing so far in a session, as the server counts
/// them, and the hearts they are worth.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HeartsMeter {
    pub work_j: f64,
    pub climb_m: f64,
}

impl HeartsMeter {
    /// One physics step: `power_w` for `dt` s, `rise_m` metres up (a
    /// descent counts nothing); `blocked` says why it does not count.
    pub fn add(&mut self, power_w: f64, dt: f64, rise_m: f64, blocked: Option<NoHearts>) {
        if blocked.is_some() {
            return;
        }
        if power_w.is_finite() && dt.is_finite() {
            self.work_j += power_w.max(0.0) * dt.max(0.0);
        }
        if rise_m.is_finite() && rise_m > 0.0 {
            self.climb_m += rise_m;
        }
    }

    /// The hearts earned so far.
    pub fn hearts(&self) -> u32 {
        earned(self.work_j, self.climb_m)
    }
}

/// `1,239 ♥`: thousands separated by commas (spec §14 Phase 4, 4E).
pub fn format(hearts: i64) -> String {
    let digits = hearts.unsigned_abs().to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    format!("{}{out} ♥", if hearts < 0 { "-" } else { "" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_heart_per_10_kj_and_per_20_m() {
        assert_eq!(earned(0.0, 0.0), 0);
        assert_eq!(earned(9_999.0, 19.9), 0);
        assert_eq!(earned(10_000.0, 0.0), 1);
        assert_eq!(earned(0.0, 20.0), 1);
        // An hour at 200 W: 720 kJ = 72 ♥; 600 m of climbing = 30 ♥.
        assert_eq!(earned(200.0 * 3600.0, 600.0), 102);
        assert_eq!(earned(-5.0, -3.0), 0);
    }

    #[test]
    fn the_meter_counts_only_real_riding() {
        let mut m = HeartsMeter::default();
        let dt = 0.02;
        for _ in 0..(3600 * 50) {
            m.add(250.0, dt, 0.0, None);
        }
        // 900 kJ, no daily cap.
        assert_eq!(m.hearts(), 90);
        let before = m;
        for why in [
            NoHearts::VirtualPower,
            NoHearts::GodMode,
            NoHearts::Down,
            NoHearts::Bot,
        ] {
            m.add(1000.0, 10.0, 50.0, Some(why));
        }
        assert_eq!(m, before, "nothing for virtual power, god mode, down, bots");
        m.add(0.0, dt, 20.0, None);
        m.add(0.0, dt, -100.0, None);
        assert_eq!(m.hearts(), 91, "climbing counts, descents do not");
        m.add(f64::NAN, dt, f64::INFINITY, None);
        assert_eq!(m.hearts(), 91);
    }

    #[test]
    fn hearts_are_shown_with_separators() {
        assert_eq!(format(0), "0 ♥");
        assert_eq!(format(500), "500 ♥");
        assert_eq!(format(1239), "1,239 ♥");
        assert_eq!(format(15_000), "15,000 ♥");
        assert_eq!(format(1_234_567), "1,234,567 ♥");
        assert_eq!(format(-164), "-164 ♥");
    }
}

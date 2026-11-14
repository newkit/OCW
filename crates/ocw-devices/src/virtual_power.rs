//! The virtual trainer's logic, without a thread (spec §3.4): the power
//! the user set (or the ERG target) and a plausible cadence. The threaded
//! [`VirtualTrainer`](crate::VirtualTrainer) (native) and the frame-driven
//! [`FrameVirtualTrainer`] (the browser app, spec §14 Phase 10, B1: wasm
//! has no threads) share it.

use std::time::Duration;

use crate::{DeviceCommand, DeviceEvent};

/// The virtual trainer's power: set by the user, or the ERG target.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VirtualPower {
    set_w: f64,
    erg_w: Option<f64>,
}

impl VirtualPower {
    pub fn new(set_w: f64) -> Self {
        Self {
            set_w: set_w.max(0.0),
            erg_w: None,
        }
    }

    pub fn command(&mut self, cmd: DeviceCommand) {
        match cmd {
            DeviceCommand::SetVirtualPower(w) => self.set_w = w.max(0.0),
            DeviceCommand::SetTargetPower(w) => self.erg_w = Some(w.max(0.0)),
            DeviceCommand::SetSimulation(_) => self.erg_w = None,
            DeviceCommand::Shutdown => {}
        }
    }

    /// The power set by the user (W).
    pub fn set_w(&self) -> f64 {
        self.set_w
    }

    /// ERG: the target while the virtual rider pedals; otherwise the set power.
    pub fn reported(&self) -> f64 {
        match self.erg_w {
            Some(t) if self.set_w > 0.0 => t,
            _ => self.set_w,
        }
    }
}

/// Cadence a rider would plausibly use at a given power.
pub fn cadence_for(power_w: f64) -> Option<f64> {
    (power_w > 0.0).then(|| (70.0 + power_w / 20.0).min(105.0))
}

/// The virtual trainer on the caller's frame loop: [`advance`](Self::advance)
/// gets the frame time and reports a sample at the trainer's rate (4 Hz,
/// like the threaded one; the first at once).
#[derive(Debug, Clone, PartialEq)]
pub struct FrameVirtualTrainer {
    power: VirtualPower,
    period: Duration,
    elapsed: Duration,
    next: Duration,
}

impl FrameVirtualTrainer {
    /// Samples per second (real FTMS trainers report at roughly 1–4 Hz).
    pub const RATE_HZ: f64 = 4.0;

    pub fn new(initial_power_w: f64) -> Self {
        Self {
            power: VirtualPower::new(initial_power_w),
            period: Duration::from_secs_f64(1.0 / Self::RATE_HZ),
            elapsed: Duration::ZERO,
            next: Duration::ZERO,
        }
    }

    /// Takes a command (virtual power, ERG target, SIM).
    pub fn command(&mut self, cmd: DeviceCommand) {
        self.power.command(cmd);
    }

    /// The power the user set (W).
    pub fn set_power_w(&self) -> f64 {
        self.power.set_w()
    }

    /// The power reported now (W; the ERG target in ERG mode).
    pub fn reported_w(&self) -> f64 {
        self.power.reported()
    }

    /// Advances by `dt_s` seconds: the sample that became due, if any (one
    /// at most: after a long frame the newest value is all that matters).
    pub fn advance(&mut self, dt_s: f64) -> Option<DeviceEvent> {
        self.elapsed += Duration::from_secs_f64(dt_s.max(0.0));
        if self.elapsed < self.next {
            return None;
        }
        while self.next <= self.elapsed {
            self.next += self.period;
        }
        let watts = self.power.reported();
        Some(DeviceEvent::Power {
            watts,
            cadence_rpm: cadence_for(watts),
            timestamp: self.elapsed,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_trainer_reports_at_four_hertz() {
        let mut t = FrameVirtualTrainer::new(200.0);
        // The first sample at once.
        let first = t.advance(0.0).expect("first sample");
        assert_eq!(
            first,
            DeviceEvent::Power {
                watts: 200.0,
                cadence_rpm: Some(80.0),
                timestamp: Duration::ZERO,
            }
        );
        // 60 Hz frames for 2 s: 8 more samples.
        let n = (0..120).filter_map(|_| t.advance(1.0 / 60.0)).count();
        assert_eq!(n, 8);
        // A long frame gives one sample, not a burst.
        assert!(t.advance(3.0).is_some());
        assert!(t.advance(0.01).is_none());
    }

    #[test]
    fn frame_trainer_follows_commands() {
        let mut t = FrameVirtualTrainer::new(200.0);
        t.command(DeviceCommand::SetVirtualPower(250.0));
        assert_eq!(t.set_power_w(), 250.0);
        let Some(DeviceEvent::Power { watts, .. }) = t.advance(0.0) else {
            panic!("no sample");
        };
        assert_eq!(watts, 250.0);
        // ERG mode reports the target while the rider pedals.
        t.command(DeviceCommand::SetTargetPower(180.0));
        assert_eq!(t.reported_w(), 180.0);
        t.command(DeviceCommand::SetVirtualPower(-5.0));
        assert_eq!((t.set_power_w(), t.reported_w()), (0.0, 0.0));
    }
}

use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::virtual_power::{VirtualPower, cadence_for};
use crate::{DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle};

/// A trainer without hardware: reports a power value set by the user.
///
/// Behaves like a real trainer from the app's point of view: it runs on its
/// own thread and sends samples at a fixed rate (real FTMS trainers report at
/// roughly 1–4 Hz). In ERG mode it acts like an ideal ERG trainer with a
/// rider who keeps pedalling: it reports the target power (while the set
/// power is above 0, i.e. the virtual rider pedals) until a SIM command
/// ends ERG mode.
pub struct VirtualTrainer {
    initial_power_w: f64,
    rate_hz: f64,
}

impl VirtualTrainer {
    pub fn new(initial_power_w: f64) -> Self {
        Self {
            initial_power_w,
            rate_hz: 4.0,
        }
    }
}

impl DeviceBackend for VirtualTrainer {
    fn name(&self) -> &str {
        "Virtual trainer"
    }

    fn start(self) -> Result<DeviceHandle, DeviceError> {
        let (event_tx, events) = mpsc::channel();
        let (commands, command_rx) = mpsc::channel();
        let period = Duration::from_secs_f64(1.0 / self.rate_hz);
        let mut power = VirtualPower::new(self.initial_power_w);
        thread::Builder::new()
            .name("ocw-virtual-trainer".into())
            .spawn(move || {
                let start = Instant::now();
                let mut next = start;
                loop {
                    let wait = next.saturating_duration_since(Instant::now());
                    match command_rx.recv_timeout(wait) {
                        Ok(DeviceCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                        Ok(cmd) => {
                            power.command(cmd);
                            continue;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    let watts = power.reported();
                    let event = DeviceEvent::Power {
                        watts,
                        cadence_rpm: cadence_for(watts),
                        timestamp: start.elapsed(),
                    };
                    if event_tx.send(event).is_err() {
                        break;
                    }
                    next += period;
                }
            })
            .map_err(|e| DeviceError::Start(e.to_string()))?;
        Ok(DeviceHandle {
            name: "Virtual trainer".into(),
            events,
            commands,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_power_and_follows_commands() {
        let h = VirtualTrainer::new(150.0).start().unwrap();
        let first = h.events.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(matches!(first, DeviceEvent::Power { watts, .. } if watts == 150.0));
        h.commands
            .send(DeviceCommand::SetVirtualPower(300.0))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let e = h.events.recv_timeout(Duration::from_secs(2)).unwrap();
            if let DeviceEvent::Power {
                watts, cadence_rpm, ..
            } = e
                && watts == 300.0
            {
                assert_eq!(cadence_rpm, Some(85.0));
                break;
            }
            assert!(Instant::now() < deadline, "power never changed");
        }
        h.commands.send(DeviceCommand::Shutdown).unwrap();
    }

    #[test]
    fn erg_target_replaces_the_set_power_until_sim() {
        let mut p = VirtualPower::new(150.0);
        assert_eq!(p.reported(), 150.0);
        p.command(DeviceCommand::SetTargetPower(240.0));
        assert_eq!(p.reported(), 240.0);
        // The virtual rider stops: no power, also in ERG.
        p.command(DeviceCommand::SetVirtualPower(0.0));
        assert_eq!(p.reported(), 0.0);
        p.command(DeviceCommand::SetVirtualPower(100.0));
        assert_eq!(p.reported(), 240.0);
        p.command(DeviceCommand::SetSimulation(crate::SimulationParameters {
            grade: 0.0,
            wind_speed_mps: 0.0,
            crr: 0.004,
            cw_kg_per_m: 0.4,
            mass_kg: 80.0,
        }));
        assert_eq!(p.reported(), 100.0);
        p.command(DeviceCommand::SetTargetPower(-5.0));
        assert_eq!(p.reported(), 0.0);
    }

    #[test]
    fn no_cadence_when_not_pedaling() {
        assert_eq!(cadence_for(0.0), None);
    }
}

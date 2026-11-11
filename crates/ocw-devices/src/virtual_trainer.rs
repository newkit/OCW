use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant};

use crate::{DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle};

/// A trainer without hardware: reports a power value set by the user.
///
/// Behaves like a real trainer from the app's point of view: it runs on its
/// own thread and sends samples at a fixed rate (real FTMS trainers report at
/// roughly 1–4 Hz).
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

/// Cadence a rider would plausibly use at a given power.
fn cadence_for(power_w: f64) -> Option<f64> {
    (power_w > 0.0).then(|| (70.0 + power_w / 20.0).min(105.0))
}

impl DeviceBackend for VirtualTrainer {
    fn name(&self) -> &str {
        "Virtual trainer"
    }

    fn start(self) -> Result<DeviceHandle, DeviceError> {
        let (event_tx, events) = mpsc::channel();
        let (commands, command_rx) = mpsc::channel();
        let period = Duration::from_secs_f64(1.0 / self.rate_hz);
        let mut power = self.initial_power_w;
        thread::Builder::new()
            .name("ocw-virtual-trainer".into())
            .spawn(move || {
                let start = Instant::now();
                let mut next = start;
                loop {
                    let wait = next.saturating_duration_since(Instant::now());
                    match command_rx.recv_timeout(wait) {
                        Ok(DeviceCommand::SetVirtualPower(w)) => {
                            power = w.max(0.0);
                            continue;
                        }
                        Ok(DeviceCommand::Shutdown) | Err(RecvTimeoutError::Disconnected) => break,
                        Ok(other) => {
                            tracing::debug!(?other, "virtual trainer ignores command");
                            continue;
                        }
                        Err(RecvTimeoutError::Timeout) => {}
                    }
                    let event = DeviceEvent::Power {
                        watts: power,
                        cadence_rpm: cadence_for(power),
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
    fn no_cadence_when_not_pedaling() {
        assert_eq!(cadence_for(0.0), None);
    }
}

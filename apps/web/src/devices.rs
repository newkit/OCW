//! Devices of the browser app (spec §14 Phase 10 B4), without Bevy: the
//! Web Bluetooth trainers and sensors the rider connected (their
//! `DeviceHandle`s – the backends run the shared `ocw-devices` protocol on
//! the page's event loop, `browser_devices` starts them), the virtual
//! trainer as the fallback, the remembered devices of this origin, and the
//! SIM / ERG control with workouts – the same `TrainerControl` and
//! `WorkoutSession` as the desktop.
//!
//! Rules (as on the desktop, `apps/desktop/src/devices.rs`):
//! - Power comes from the virtual trainer until a trainer or power meter
//!   is connected; from then on only from the devices (while one
//!   reconnects the rider gets no power, as on the desktop). Forgetting the
//!   last power device brings the virtual trainer back. A heart-rate strap
//!   alone keeps the virtual power.
//! - Every SIM / ERG command goes to every device (each ignores what it
//!   does not support) and to the virtual trainer (ERG: it reports the
//!   target).
//! - Browsers without Web Bluetooth (Firefox, Safari, iOS; spec §14
//!   Phase 10 Q1) ride with virtual power and say so.

use ocw_client::client::ClientCore;
use ocw_client::trainer::{TrainerControl, TrainerMode};
use ocw_client::workout::WorkoutSession;
use ocw_core::RiderProfile;
use ocw_devices::ble::web::{DeviceKind, Remembered, RememberedDevice};
use std::sync::Mutex;
use std::sync::mpsc::{Receiver, Sender};

use ocw_devices::{DeviceCommand, DeviceEvent, DeviceHandle};

/// What the browser offers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bluetooth {
    /// Not known yet (the check is asynchronous).
    Unknown,
    /// `navigator.bluetooth` is missing: Firefox, Safari, iOS.
    Missing,
    /// Web Bluetooth, but no adapter (`getAvailability()` false).
    NoAdapter,
    Available,
}

/// The message for browsers without Web Bluetooth (the page overlay and
/// the console show it).
pub const NO_WEB_BLUETOOTH: &str = "This browser has no Web Bluetooth (Firefox, Safari and every browser on iOS): riding with virtual power (↑ / ↓, Page Up / Down). Trainers, power meters and heart-rate straps connect in Chrome or Edge on Windows, macOS, Linux, ChromeOS and Android.";
/// The message when the browser has Web Bluetooth but no adapter.
pub const NO_ADAPTER: &str =
    "No Bluetooth adapter found (or Bluetooth is off): riding with virtual power.";

/// A connection's state as the events tell it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    Connecting,
    /// Connected and subscribed: what the device is.
    Connected(&'static str),
    Reconnecting,
}

/// One running Bluetooth device (its backend's channels; the receiver in a
/// mutex, so the session can be a Bevy resource).
pub struct Device {
    pub name: String,
    events: Mutex<Receiver<DeviceEvent>>,
    commands: Sender<DeviceCommand>,
    pub kind: DeviceKind,
    /// The browser's id (per origin).
    pub id: String,
    pub state: LinkState,
    /// Last power (W) and heart rate (bpm) it reported.
    pub power_w: Option<f64>,
    pub heart_rate_bpm: Option<f64>,
    /// Power samples so far.
    pub samples: u64,
}

impl Device {
    fn delivers_power(&self) -> bool {
        matches!(self.kind, DeviceKind::Trainer | DeviceKind::PowerMeter)
    }
}

/// The devices of the ride and the trainer control.
pub struct WebDevices {
    pub devices: Vec<Device>,
    pub remembered: Remembered,
    pub bluetooth: Bluetooth,
    pub trainer: TrainerControl,
    pub workout: WorkoutSession,
    pub ftp_w: f64,
    /// The last command sent to the devices (for the HUD and the test).
    pub last_command: Option<DeviceCommand>,
    /// SIM / ERG commands sent so far.
    pub commands_sent: u64,
    /// Lines for the console (connects, drops, workout steps).
    messages: Vec<String>,
}

impl WebDevices {
    pub fn new(profile: RiderProfile, ftp_w: f64) -> Self {
        Self {
            devices: Vec::new(),
            remembered: Remembered::default(),
            bluetooth: Bluetooth::Unknown,
            trainer: TrainerControl::new(profile),
            workout: WorkoutSession::default(),
            ftp_w,
            last_command: None,
            commands_sent: 0,
            messages: Vec::new(),
        }
    }

    /// A started backend (`WebBluetoothBackend::start`) for `kind`; it is
    /// remembered for this origin. Returns the console line.
    pub fn add(&mut self, handle: DeviceHandle, kind: DeviceKind, id: String) -> String {
        let name = handle.name.clone();
        self.devices.retain(|d| {
            let again = d.id == id;
            if again {
                let _ = d.commands.send(DeviceCommand::Shutdown);
            }
            !again
        });
        // The latest target at once (the backend applies it when connected).
        if let Some(cmd) = self.last_command {
            let _ = handle.commands.send(cmd);
        }
        self.devices.push(Device {
            name: name.clone(),
            events: Mutex::new(handle.events),
            commands: handle.commands,
            kind,
            id: id.clone(),
            state: LinkState::Connecting,
            power_w: None,
            heart_rate_bpm: None,
            samples: 0,
        });
        self.remembered.remember(RememberedDevice {
            kind,
            id,
            name: name.clone(),
        });
        format!("{name}: connecting ({})", kind.label())
    }

    /// Stops and forgets devices by id or name (any case), or `all`.
    /// Returns how many were stopped or forgotten.
    pub fn forget(&mut self, which: &str) -> usize {
        let mut n = 0;
        self.devices.retain(|d| {
            let hit = which == "all" || d.id == which || d.name.eq_ignore_ascii_case(which);
            if hit {
                let _ = d.commands.send(DeviceCommand::Shutdown);
                n += 1;
            }
            !hit
        });
        n.max(self.remembered.forget(which))
    }

    /// Whether the virtual trainer gives the power (no trainer or power
    /// meter connected).
    pub fn uses_virtual(&self) -> bool {
        !self.devices.iter().any(Device::delivers_power)
    }

    /// Reads every device's events: the samples for the server (power
    /// events) in order; connects and drops go to the console.
    pub fn poll(&mut self, client: &mut ClientCore) -> Vec<Vec<u8>> {
        let mut samples = Vec::new();
        for d in &mut self.devices {
            let events: Vec<DeviceEvent> =
                d.events.lock().expect("not poisoned").try_iter().collect();
            for event in events {
                match event {
                    DeviceEvent::Connected { kind } => {
                        d.state = LinkState::Connected(kind);
                        self.messages
                            .push(format!("{}: connected ({kind})", d.name));
                    }
                    DeviceEvent::Disconnected => {
                        if d.state != LinkState::Reconnecting {
                            self.messages
                                .push(format!("{}: disconnected, reconnecting", d.name));
                        }
                        d.state = LinkState::Reconnecting;
                        d.power_w = None;
                    }
                    DeviceEvent::Power { watts, .. } => {
                        d.power_w = Some(watts);
                        d.samples += 1;
                    }
                    DeviceEvent::HeartRate { bpm, .. } => d.heart_rate_bpm = Some(bpm),
                }
                // Device power earns hearts, the virtual trainer's does
                // not (spec §14 Phase 4, 4E): the samples say which.
                if matches!(event, DeviceEvent::Power { .. }) {
                    client.set_virtual_device(false);
                }
                if let Some(s) = client.on_device_event(event) {
                    samples.push(s);
                }
            }
        }
        samples
    }

    /// One frame of trainer control (`dt_s` of game time, at `now_s`):
    /// the workout (it sets the mode) then SIM / ERG from the road. The
    /// command for every device and the virtual trainer, if one is due.
    pub fn control(&mut self, dt_s: f64, now_s: f64, client: &ClientCore) -> Option<DeviceCommand> {
        if client.rider_id().is_some() {
            let lines = self
                .workout
                .update(dt_s, client.power_w(), self.ftp_w, &mut self.trainer);
            self.messages.extend(lines);
        }
        let cmd = self.trainer.follow(now_s, client)?;
        for d in &self.devices {
            let _ = d.commands.send(cmd);
        }
        self.last_command = Some(cmd);
        self.commands_sent += 1;
        Some(cmd)
    }

    /// The console lines since the last call.
    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }

    /// The devices part of the HUD's status line.
    pub fn status(&self, virtual_w: f64) -> String {
        let mut parts: Vec<String> = self
            .devices
            .iter()
            .map(|d| match (d.state, d.power_w, d.heart_rate_bpm) {
                (LinkState::Connecting, ..) => format!("{}: connecting …", d.name),
                (LinkState::Reconnecting, ..) => format!("{}: reconnecting …", d.name),
                (LinkState::Connected(_), Some(w), _) => format!("{} {w:.0} W", d.name),
                (LinkState::Connected(_), None, Some(hr)) => {
                    format!("{} {hr:.0} bpm", d.name)
                }
                (LinkState::Connected(_), None, None) => d.name.clone(),
            })
            .collect();
        if self.uses_virtual() {
            let why = match self.bluetooth {
                Bluetooth::Missing => " (no Web Bluetooth here)",
                Bluetooth::NoAdapter => " (no Bluetooth adapter)",
                _ => "",
            };
            parts.insert(0, format!("Virtual trainer {virtual_w:.0} W{why}"));
        }
        parts.push(self.trainer.status());
        parts.join(" | ")
    }

    /// `devices.list`.
    pub fn list(&self) -> String {
        let mut out = format!(
            "Web Bluetooth: {}",
            match self.bluetooth {
                Bluetooth::Unknown => "checking",
                Bluetooth::Missing => "not in this browser (virtual power)",
                Bluetooth::NoAdapter => "no adapter (virtual power)",
                Bluetooth::Available => "available",
            }
        );
        out.push_str("\nrunning:");
        if self.devices.is_empty() {
            out.push_str(" none (virtual trainer)");
        }
        for d in &self.devices {
            let state = match d.state {
                LinkState::Connecting => "connecting".to_string(),
                LinkState::Connected(k) => format!("connected, {k}"),
                LinkState::Reconnecting => "reconnecting".to_string(),
            };
            out.push_str(&format!(
                "\n  {} ({}, {state}, {} samples)",
                d.name,
                d.kind.label(),
                d.samples
            ));
        }
        out.push_str("\nremembered on this site:");
        if self.remembered.devices.is_empty() {
            out.push_str(" none");
        }
        for d in &self.remembered.devices {
            out.push_str(&format!("\n  {} ({}, id {})", d.name, d.kind.label(), d.id));
        }
        out.push_str(&format!("\ntrainer: {}", self.trainer.status()));
        out
    }

    /// The devices as JSON for the page's overlay and the browser test
    /// (`window.ocw.devices()`).
    pub fn json(&self, virtual_w: f64) -> serde_json::Value {
        let devices: Vec<serde_json::Value> = self
            .devices
            .iter()
            .map(|d| {
                let (state, kind) = match d.state {
                    LinkState::Connecting => ("connecting", None),
                    LinkState::Connected(k) => ("connected", Some(k)),
                    LinkState::Reconnecting => ("reconnecting", None),
                };
                serde_json::json!({
                    "name": d.name,
                    "id": d.id,
                    "as": d.kind.key(),
                    "state": state,
                    "kind": kind,
                    "power_w": d.power_w,
                    "heart_rate_bpm": d.heart_rate_bpm,
                    "samples": d.samples,
                })
            })
            .collect();
        let remembered: Vec<serde_json::Value> = self
            .remembered
            .devices
            .iter()
            .map(|d| serde_json::json!({"name": d.name, "id": d.id, "as": d.kind.key()}))
            .collect();
        let command = self.last_command.map(|c| match c {
            DeviceCommand::SetSimulation(p) => serde_json::json!({
                "sim": {"grade": p.grade, "crr": p.crr, "cw": p.cw_kg_per_m,
                        "wind_mps": p.wind_speed_mps, "mass_kg": p.mass_kg}
            }),
            DeviceCommand::SetTargetPower(w) => serde_json::json!({"erg_w": w}),
            other => serde_json::json!(format!("{other:?}")),
        });
        serde_json::json!({
            "bluetooth": format!("{:?}", self.bluetooth),
            "virtual": self.uses_virtual(),
            "virtual_w": virtual_w,
            "devices": devices,
            "remembered": remembered,
            "trainer": self.trainer.status(),
            "mode": self.trainer.mode().to_string(),
            "last_command": command,
            "commands_sent": self.commands_sent,
        })
    }

    /// `trainer.mode sim|erg|off` (ERG keeps its target, else 150 W).
    pub fn set_mode(&mut self, mode: &str) -> Result<String, String> {
        let mode = match mode {
            "sim" => TrainerMode::Sim,
            "off" => TrainerMode::Off,
            "erg" => match self.trainer.mode() {
                erg @ TrainerMode::Erg { .. } => erg,
                _ => TrainerMode::Erg { target_w: 150.0 },
            },
            other => return Err(format!("unknown mode {other} (sim, erg, off)")),
        };
        self.trainer.set_mode(mode);
        Ok(format!("trainer mode {mode}"))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::time::Duration;

    use ocw_server::local::FrameServer;
    use ocw_world::Route;

    use super::*;

    /// A backend's two ends: the app's handle, and the device side.
    fn backend(name: &str) -> (DeviceHandle, Sender<DeviceEvent>, Receiver<DeviceCommand>) {
        let (ev_tx, ev_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        (
            DeviceHandle {
                name: name.into(),
                events: ev_rx,
                commands: cmd_tx,
            },
            ev_tx,
            cmd_rx,
        )
    }

    fn power(w: f64) -> DeviceEvent {
        DeviceEvent::Power {
            watts: w,
            cadence_rpm: Some(88.0),
            timestamp: Duration::from_millis(250),
        }
    }

    fn joined_client() -> ClientCore {
        let route = Route::test_hills();
        let mut client = ClientCore::new(route.clone(), "Web", RiderProfile::default());
        let course: ocw_server::Course = route.into();
        let mut server = FrameServer::new(course, 0, 0.0);
        server.send(&client.hello());
        server.advance(0.1);
        for m in server.drain() {
            client.on_server_message(&m, 0.1);
        }
        assert!(client.rider_id().is_some());
        client
    }

    #[test]
    fn devices_replace_the_virtual_trainer_and_report_their_state() {
        let mut d = WebDevices::new(RiderProfile::default(), 200.0);
        let mut client = joined_client();
        assert!(d.uses_virtual());
        assert!(
            d.status(200.0)
                .starts_with("Virtual trainer 200 W | trainer SIM")
        );
        // A heart-rate strap alone keeps the virtual power.
        let (hr, hr_tx, _hr_cmds) = backend("TICKR");
        d.add(hr, DeviceKind::HeartRate, "hr-1".into());
        assert!(d.uses_virtual());
        let (kickr, tx, cmds) = backend("KICKR");
        assert_eq!(
            d.add(kickr, DeviceKind::Trainer, "k-1".into()),
            "KICKR: connecting (trainer)"
        );
        assert!(!d.uses_virtual());
        assert!(
            d.status(200.0)
                .starts_with("TICKR: connecting … | KICKR: connecting …")
        );
        tx.send(DeviceEvent::Connected {
            kind: "smart trainer (FTMS)",
        })
        .unwrap();
        tx.send(power(234.0)).unwrap();
        hr_tx
            .send(DeviceEvent::Connected {
                kind: "heart-rate sensor",
            })
            .unwrap();
        hr_tx
            .send(DeviceEvent::HeartRate {
                bpm: 131.0,
                timestamp: Duration::ZERO,
            })
            .unwrap();
        let samples = d.poll(&mut client);
        assert_eq!(samples.len(), 1);
        assert_eq!(client.power_w(), 234.0);
        assert!(!client.virtual_power());
        assert_eq!(
            d.take_messages(),
            [
                "TICKR: connected (heart-rate sensor)",
                "KICKR: connected (smart trainer (FTMS))"
            ]
        );
        let status = d.status(200.0);
        assert!(
            status.starts_with("TICKR 131 bpm | KICKR 234 W | trainer"),
            "{status}"
        );
        // SIM commands reach every device.
        let cmd = d.control(0.02, 0.2, &client).expect("first SIM");
        assert!(matches!(cmd, DeviceCommand::SetSimulation(_)));
        assert_eq!(cmds.try_iter().collect::<Vec<_>>(), [cmd]);
        // A drop: reconnecting, said once.
        tx.send(DeviceEvent::Disconnected).unwrap();
        tx.send(DeviceEvent::Disconnected).unwrap();
        d.poll(&mut client);
        assert_eq!(d.take_messages(), ["KICKR: disconnected, reconnecting"]);
        assert!(d.status(200.0).contains("KICKR: reconnecting …"));
        assert!(!d.uses_virtual(), "no virtual power while it reconnects");
        // Both remembered, newest first; forgetting the trainer stops it.
        assert_eq!(d.remembered.devices[0].name, "KICKR");
        assert!(
            d.list()
                .contains("KICKR (trainer, reconnecting, 1 samples)")
        );
        assert_eq!(d.forget("kickr"), 1);
        assert_eq!(cmds.try_recv(), Ok(DeviceCommand::Shutdown));
        assert!(d.uses_virtual());
        let json = d.json(200.0);
        assert_eq!(json["virtual"], true);
        assert_eq!(json["devices"][0]["name"], "TICKR");
        assert!(json["last_command"]["sim"]["grade"].is_number());
    }

    #[test]
    fn erg_and_workouts_drive_the_trainer() {
        let mut d = WebDevices::new(RiderProfile::default(), 250.0);
        let client = joined_client();
        let (kickr, _tx, cmds) = backend("KICKR");
        d.add(kickr, DeviceKind::Trainer, "k".into());
        assert_eq!(d.set_mode("erg").unwrap(), "trainer mode ERG 150 W");
        assert_eq!(
            d.control(0.02, 0.1, &client),
            Some(DeviceCommand::SetTargetPower(150.0))
        );
        assert!(d.set_mode("fast").is_err());
        d.set_mode("sim").unwrap();
        // A workout: ERG with its first target (of the FTP), SIM after stop.
        d.workout
            .load(ocw_client::workout::example("short-demo").unwrap())
            .unwrap();
        d.workout.start().unwrap();
        d.control(0.02, 0.2, &client);
        assert!(
            matches!(d.trainer.mode(), TrainerMode::Erg { .. }),
            "{}",
            d.trainer.mode()
        );
        let sent: Vec<DeviceCommand> = cmds.try_iter().collect();
        assert!(
            sent.iter()
                .any(|c| matches!(c, DeviceCommand::SetTargetPower(_)))
        );
        d.workout.stop(&mut d.trainer).unwrap();
        assert_eq!(d.trainer.mode(), TrainerMode::Sim);
        // A device added later gets the latest target at once.
        let (late, _ltx, late_cmds) = backend("Late");
        d.add(late, DeviceKind::PowerMeter, "p".into());
        assert_eq!(late_cmds.try_recv().ok(), d.last_command);
        assert_eq!(d.json(0.0)["commands_sent"], d.commands_sent);
    }

    #[test]
    fn without_web_bluetooth_the_virtual_trainer_says_why() {
        let mut d = WebDevices::new(RiderProfile::default(), 200.0);
        d.bluetooth = Bluetooth::Missing;
        assert!(
            d.status(180.0)
                .starts_with("Virtual trainer 180 W (no Web Bluetooth here)")
        );
        assert!(d.list().starts_with("Web Bluetooth: not in this browser"));
        d.bluetooth = Bluetooth::NoAdapter;
        assert!(d.status(180.0).contains("(no Bluetooth adapter)"));
        assert!(NO_WEB_BLUETOOTH.contains("Firefox") && NO_WEB_BLUETOOTH.contains("Chrome"));
    }
}

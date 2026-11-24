//! The bridge's messages (spec 14 Phase 10, 10D.2 *Built (local bridge)*):
//! JSON objects, one per frame (a 4-byte little-endian length, then the
//! bytes), on the one bidirectional WebTransport stream the browser opens.
//! A stream keeps the order and loses nothing; on the local network at 4
//! samples a second there is no head-of-line delay worth datagrams.

use ocw_devices::{DeviceCommand, SimulationParameters};
use serde::{Deserialize, Serialize};

/// The bridge protocol's version (in both hellos).
pub const VERSION: u32 = 1;
/// The longest frame either side accepts.
pub const MAX_FRAME: usize = 64 * 1024;

/// Browser → desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToHost {
    /// The first message: the protocol, the proof that the browser has the
    /// code ([`crate::proof`]), the page's session (a reconnect of the same
    /// page replaces its old connection) and what the browser is.
    Hello {
        v: u32,
        proof: String,
        session: String,
        client: String,
    },
    /// Trainer control from the browser's ride.
    Control { id: u64, control: Control },
    /// Clock and latency: the browser's time (ms).
    Ping { id: u64, c: f64 },
    /// The page stops the bridge.
    Bye,
}

/// Desktop → browser.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ToBrowser {
    /// Paired: the desktop's name, its run (`epoch`: a new one after a
    /// restart, where the sample numbers start again) and its devices.
    Welcome {
        v: u32,
        host: String,
        epoch: u64,
        devices: Vec<DeviceInfo>,
    },
    /// Not paired (why), then the desktop closes the connection.
    Refused { why: String },
    /// Power (W) and cadence from the desktop's trainer or power meter;
    /// `h`: the desktop's time of the reading (ms since its bridge
    /// started), `seq` counts every relayed reading.
    Power {
        seq: u64,
        h: f64,
        w: f64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rpm: Option<f64>,
        /// The desktop's virtual trainer gives it (earns no hearts).
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        virt: bool,
    },
    HeartRate { seq: u64, h: f64, bpm: f64 },
    /// One of the desktop's devices connected, dropped or failed.
    Device { name: String, state: DeviceState },
    /// A control reached the trainer.
    Applied { id: u64 },
    /// The answer to a ping, with the desktop's time.
    Pong { id: u64, c: f64, h: f64 },
    /// Another page of the same browser took over, or the desktop stopped
    /// sharing.
    Closed { why: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub state: DeviceState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceState {
    Connected,
    Reconnecting,
    Failed,
}

/// Trainer control: the shared [`DeviceCommand`]s a ride sends.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Control {
    /// SIM: the road.
    Sim {
        grade: f64,
        wind_mps: f64,
        crr: f64,
        cw: f64,
        mass_kg: f64,
        wind_grade: f64,
    },
    /// ERG: the target power (W).
    Erg(f64),
}

impl Control {
    /// The control of a command (`None`: nothing for a trainer).
    pub fn of(cmd: DeviceCommand) -> Option<Self> {
        match cmd {
            DeviceCommand::SetSimulation(p) => Some(Self::Sim {
                grade: p.grade,
                wind_mps: p.wind_speed_mps,
                crr: p.crr,
                cw: p.cw_kg_per_m,
                mass_kg: p.mass_kg,
                wind_grade: p.wind_grade,
            }),
            DeviceCommand::SetTargetPower(w) => Some(Self::Erg(w)),
            DeviceCommand::SetVirtualPower(_) | DeviceCommand::Shutdown => None,
        }
    }

    /// The command for the desktop's devices (values out of any sane range
    /// are clamped: the browser is not trusted with the trainer's limits).
    pub fn command(self) -> DeviceCommand {
        let finite = |v: f64, lo: f64, hi: f64, d: f64| {
            if v.is_finite() { v.clamp(lo, hi) } else { d }
        };
        match self {
            Self::Sim {
                grade,
                wind_mps,
                crr,
                cw,
                mass_kg,
                wind_grade,
            } => DeviceCommand::SetSimulation(SimulationParameters {
                grade: finite(grade, -0.4, 0.4, 0.0),
                wind_speed_mps: finite(wind_mps, -40.0, 40.0, 0.0),
                crr: finite(crr, 0.0, 0.05, 0.004),
                cw_kg_per_m: finite(cw, 0.0, 2.0, 0.51),
                mass_kg: finite(mass_kg, 20.0, 250.0, 80.0),
                wind_grade: finite(wind_grade, -0.4, 0.4, 0.0),
            }),
            Self::Erg(w) => DeviceCommand::SetTargetPower(finite(w, 0.0, 2000.0, 150.0)),
        }
    }
}

/// One message as a frame.
pub fn frame<T: Serialize>(msg: &T) -> Vec<u8> {
    let body = serde_json::to_vec(msg).expect("serialisable");
    let mut out = (body.len() as u32).to_le_bytes().to_vec();
    out.extend(body);
    out
}

/// A frame that is not a message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    TooLarge(usize),
    Bad(String),
}

impl std::fmt::Display for FrameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLarge(n) => write!(f, "a frame of {n} bytes (at most {MAX_FRAME})"),
            Self::Bad(e) => write!(f, "not a bridge message: {e}"),
        }
    }
}

/// Collects stream bytes into messages (any split of the bytes).
#[derive(Debug, Default)]
pub struct Frames {
    buf: Vec<u8>,
}

impl Frames {
    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// The next whole message, if there is one.
    pub fn next_message<T: for<'de> Deserialize<'de>>(&mut self) -> Option<Result<T, FrameError>> {
        if self.buf.len() < 4 {
            return None;
        }
        let n = u32::from_le_bytes(self.buf[..4].try_into().expect("4 bytes")) as usize;
        if n > MAX_FRAME {
            self.buf.clear();
            return Some(Err(FrameError::TooLarge(n)));
        }
        if self.buf.len() < 4 + n {
            return None;
        }
        let body: Vec<u8> = self.buf.drain(..4 + n).skip(4).collect();
        Some(serde_json::from_slice(&body).map_err(|e| FrameError::Bad(e.to_string())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip_through_frames_in_any_split() {
        let msgs = vec![
            ToBrowser::Welcome {
                v: VERSION,
                host: "OCW desktop".into(),
                epoch: 7,
                devices: vec![DeviceInfo {
                    name: "KICKR".into(),
                    state: DeviceState::Connected,
                }],
            },
            ToBrowser::Power {
                seq: 1,
                h: 1250.5,
                w: 212.0,
                rpm: Some(88.0),
                virt: false,
            },
            ToBrowser::HeartRate {
                seq: 2,
                h: 1300.0,
                bpm: 131.0,
            },
            ToBrowser::Pong {
                id: 3,
                c: 10.0,
                h: 20.0,
            },
        ];
        let bytes: Vec<u8> = msgs.iter().flat_map(frame).collect();
        for split in [1, 3, 7, bytes.len()] {
            let mut f = Frames::default();
            let mut got = Vec::new();
            for chunk in bytes.chunks(split) {
                f.push(chunk);
                while let Some(m) = f.next_message::<ToBrowser>() {
                    got.push(m.unwrap());
                }
            }
            assert_eq!(got, msgs, "split {split}");
        }
        // The JSON is small and readable.
        let p = String::from_utf8(frame(&msgs[1])[4..].to_vec()).unwrap();
        assert_eq!(p, r#"{"t":"power","seq":1,"h":1250.5,"w":212.0,"rpm":88.0}"#);
    }

    #[test]
    fn bad_frames_are_errors() {
        let mut f = Frames::default();
        f.push(&(MAX_FRAME as u32 + 1).to_le_bytes());
        assert!(matches!(f.next_message::<ToHost>(), Some(Err(FrameError::TooLarge(_)))));
        let mut f = Frames::default();
        f.push(&frame(&serde_json::json!({"t": "nonsense"})));
        assert!(matches!(f.next_message::<ToHost>(), Some(Err(FrameError::Bad(_)))));
    }

    #[test]
    fn controls_map_to_commands_and_are_clamped() {
        let p = SimulationParameters {
            grade: 0.05,
            wind_speed_mps: 2.0,
            crr: 0.004,
            cw_kg_per_m: 0.4,
            mass_kg: 82.0,
            wind_grade: 0.06,
        };
        let c = Control::of(DeviceCommand::SetSimulation(p)).unwrap();
        assert_eq!(c.command(), DeviceCommand::SetSimulation(p));
        assert_eq!(
            Control::of(DeviceCommand::SetTargetPower(230.0)).unwrap().command(),
            DeviceCommand::SetTargetPower(230.0)
        );
        assert_eq!(Control::of(DeviceCommand::Shutdown), None);
        assert_eq!(Control::of(DeviceCommand::SetVirtualPower(100.0)), None);
        // A wild browser cannot push the trainer beyond sane limits.
        assert_eq!(Control::Erg(1e9).command(), DeviceCommand::SetTargetPower(2000.0));
        assert_eq!(Control::Erg(f64::NAN).command(), DeviceCommand::SetTargetPower(150.0));
        let DeviceCommand::SetSimulation(s) = (Control::Sim {
            grade: 3.0,
            wind_mps: 0.0,
            crr: 0.004,
            cw: 0.4,
            mass_kg: 80.0,
            wind_grade: f64::INFINITY,
        })
        .command() else {
            panic!()
        };
        assert_eq!((s.grade, s.wind_grade), (0.4, 0.0));
        // On the wire.
        let m = ToHost::Control { id: 4, control: Control::Erg(200.0) };
        let back: ToHost = serde_json::from_slice(&frame(&m)[4..]).unwrap();
        assert_eq!(back, m);
    }
}

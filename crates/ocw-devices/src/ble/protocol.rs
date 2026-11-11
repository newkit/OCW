//! The trainer/sensor protocol as a pure state machine ("sans-IO").
//!
//! Input: the characteristics a device offers, notification values, app
//! commands and clock ticks. Output: [`DeviceEvent`]s for the app and the
//! bytes to write to the FTMS control point. The async runner only moves
//! bytes between this and the Bluetooth stack, so all decisions are testable
//! without hardware.
//!
//! Rules:
//! - Power: FTMS Indoor Bike Data once it has reported power, otherwise the
//!   Cycling Power Service.
//! - Cadence: FTMS cadence if present, otherwise crank revolutions (Cycling
//!   Power or CSC).
//! - Heart rate: Heart Rate Service, otherwise the FTMS heart-rate field.
//! - Control (FTMS): the first SIM/ERG command requests control; once granted
//!   the trainer is started and the latest command is sent. Unanswered
//!   control requests are retried, then given up.
//! - Control (Wahoo, trainers without FTMS): unlock once, then SIM mode
//!   parameters (mass, crr, cw; again after ERG or when they change) and the
//!   gradient, or the ERG target.

use std::time::Duration;

use crate::gatt::cadence::{CadenceCalculator, CrankData};
use crate::gatt::ftms::{self, ControlRequest, ControlResponse, Features, IndoorBikeData};
use crate::gatt::{csc, cycling_power, heart_rate, uuids, wahoo};
use crate::{DeviceCommand, DeviceEvent};

/// How long a control write may stay unanswered before the next one is sent.
pub const WRITE_TIMEOUT: Duration = Duration::from_millis(1500);
/// How long to wait for the answer to a control request before retrying.
pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(3);
/// Control requests sent before giving up.
pub const CONTROL_ATTEMPTS: u32 = 3;

/// What the protocol wants done.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Output {
    pub events: Vec<DeviceEvent>,
    /// Values to write to the FTMS control point, in order.
    pub control_writes: Vec<Vec<u8>>,
    /// Problems worth logging (malformed data, refused requests).
    pub warnings: Vec<String>,
}

impl Output {
    fn warn(&mut self, msg: impl Into<String>) {
        self.warnings.push(msg.into());
    }
}

/// State of trainer control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlState {
    /// The device has no FTMS control point.
    Unavailable,
    /// Not requested yet.
    Idle,
    /// Control requested at `since`; `attempt` requests sent so far.
    Requested {
        since: Duration,
        attempt: u32,
    },
    Granted,
    /// The trainer refused or never answered.
    Refused,
}

/// State of the Wahoo control path.
#[derive(Debug, Clone, Default)]
struct WahooState {
    unlocked: bool,
    /// SIM mode parameters last sent (mass, crr, cw); `None` = not in SIM mode.
    sim: Option<(f64, f64, f64)>,
    wind_mps: f64,
}

pub struct TrainerProtocol {
    available: Vec<u16>,
    features: Option<Features>,
    control: ControlState,
    /// Latest SIM/ERG command not yet sent (waiting for control).
    pending: Option<DeviceCommand>,
    /// Op code of the last request sent, awaiting its response.
    awaiting: Option<u8>,
    ftms_reports_power: bool,
    ftms_reports_cadence: bool,
    cadence: CadenceCalculator,
    crank_cadence: Option<f64>,
    /// Set when the trainer is controlled through Wahoo's control point.
    wahoo: Option<WahooState>,
    /// Control writes waiting to be sent: trainers handle one control
    /// procedure at a time (a second write before the answer fails).
    queue: std::collections::VecDeque<Vec<u8>>,
    /// Op code and send time of the write awaiting its answer.
    in_flight: Option<(u8, Duration)>,
}

impl TrainerProtocol {
    /// `available`: the SIG characteristic numbers the device offers;
    /// `features`: the parsed Fitness Machine Feature, if readable.
    pub fn new(available: &[u16], features: Option<Features>) -> Self {
        let ftms = available.contains(&uuids::FITNESS_MACHINE_CONTROL_POINT);
        // FTMS is preferred; Wahoo's own control point only without it.
        let wahoo =
            (!ftms && available.contains(&uuids::WAHOO_TRAINER_CONTROL)).then(WahooState::default);
        let control = if ftms {
            ControlState::Idle
        } else if wahoo.is_some() {
            ControlState::Granted
        } else {
            ControlState::Unavailable
        };
        Self {
            available: available.to_vec(),
            features,
            control,
            pending: None,
            awaiting: None,
            ftms_reports_power: false,
            ftms_reports_cadence: false,
            cadence: CadenceCalculator::default(),
            crank_cadence: None,
            wahoo,
            queue: Default::default(),
            in_flight: None,
        }
    }

    pub fn control_state(&self) -> ControlState {
        self.control
    }

    /// Characteristics to subscribe to (notifications or indications).
    pub fn subscriptions(&self) -> Vec<u16> {
        [
            uuids::INDOOR_BIKE_DATA,
            uuids::CYCLING_POWER_MEASUREMENT,
            uuids::CSC_MEASUREMENT,
            uuids::HEART_RATE_MEASUREMENT,
            uuids::FITNESS_MACHINE_CONTROL_POINT,
            uuids::WAHOO_TRAINER_CONTROL,
        ]
        .into_iter()
        .filter(|c| self.available.contains(c))
        .collect()
    }

    /// Whether this device is a controllable trainer, a power meter or a sensor.
    /// Moves new control writes into the queue and releases the next one
    /// when nothing is awaiting an answer. A newer target (gradient, power)
    /// replaces a queued one with the same op code.
    fn flush(&mut self, out: &mut Output, now: Duration) {
        let replaceable: &[u8] = if self.wahoo.is_some() {
            &[
                wahoo::op::SET_ERG,
                wahoo::op::SET_GRADE,
                wahoo::op::SET_WIND_SPEED,
            ]
        } else {
            &[
                ftms::op::SET_TARGET_RESISTANCE,
                ftms::op::SET_TARGET_POWER,
                ftms::op::SET_INDOOR_BIKE_SIMULATION,
            ]
        };
        for w in out.control_writes.drain(..) {
            match self
                .queue
                .iter_mut()
                .find(|q| q[0] == w[0] && replaceable.contains(&w[0]))
            {
                Some(q) => *q = w,
                None => self.queue.push_back(w),
            }
        }
        if let Some((_, since)) = self.in_flight
            && now.saturating_sub(since) >= WRITE_TIMEOUT
        {
            out.warn("control write not answered in time");
            self.in_flight = None;
        }
        if self.in_flight.is_none()
            && let Some(w) = self.queue.pop_front()
        {
            self.in_flight = Some((w[0], now));
            out.control_writes.push(w);
        }
    }

    fn answered(&mut self, op_code: u8) {
        if self.in_flight.is_some_and(|(op, _)| op == op_code) {
            self.in_flight = None;
        }
    }

    /// The characteristic that `Output::control_writes` go to.
    pub fn control_characteristic(&self) -> u16 {
        if self.wahoo.is_some() {
            uuids::WAHOO_TRAINER_CONTROL
        } else {
            uuids::FITNESS_MACHINE_CONTROL_POINT
        }
    }

    pub fn describe(&self) -> &'static str {
        if self.wahoo.is_some() {
            "smart trainer (Wahoo control)"
        } else if self.control != ControlState::Unavailable {
            "smart trainer (FTMS)"
        } else if self.available.contains(&uuids::INDOOR_BIKE_DATA) {
            "trainer (FTMS, read-only)"
        } else if self.available.contains(&uuids::CYCLING_POWER_MEASUREMENT) {
            "power meter"
        } else if self.available.contains(&uuids::HEART_RATE_MEASUREMENT) {
            "heart-rate sensor"
        } else if self.available.contains(&uuids::CSC_MEASUREMENT) {
            "speed/cadence sensor"
        } else {
            "unsupported device"
        }
    }

    fn has(&self, characteristic: u16) -> bool {
        self.available.contains(&characteristic)
    }

    /// A notification or indication arrived at time `now`.
    pub fn on_notification(&mut self, characteristic: u16, value: &[u8], now: Duration) -> Output {
        let mut out = Output::default();
        match characteristic {
            uuids::INDOOR_BIKE_DATA => match IndoorBikeData::parse(value) {
                Ok(d) => {
                    if let Some(rpm) = d.cadence_rpm {
                        self.ftms_reports_cadence = true;
                        self.crank_cadence = Some(rpm);
                    }
                    if let Some(w) = d.power_w {
                        self.ftms_reports_power = true;
                        out.events.push(DeviceEvent::Power {
                            watts: f64::from(w.max(0)),
                            cadence_rpm: self.cadence_now(now),
                            timestamp: now,
                        });
                    }
                    if let (Some(bpm), false) =
                        (d.heart_rate_bpm, self.has(uuids::HEART_RATE_MEASUREMENT))
                    {
                        out.events.push(DeviceEvent::HeartRate {
                            bpm: f64::from(bpm),
                            timestamp: now,
                        });
                    }
                }
                Err(e) => out.warn(format!("indoor bike data: {e}")),
            },
            uuids::CYCLING_POWER_MEASUREMENT => match cycling_power::Measurement::parse(value) {
                Ok(m) => {
                    if let Some(crank) = m.crank {
                        self.feed_crank(crank, now);
                    }
                    if !self.ftms_reports_power {
                        out.events.push(DeviceEvent::Power {
                            watts: f64::from(m.power_w.max(0)),
                            cadence_rpm: self.cadence_now(now),
                            timestamp: now,
                        });
                    }
                }
                Err(e) => out.warn(format!("cycling power measurement: {e}")),
            },
            uuids::CSC_MEASUREMENT => match csc::Measurement::parse(value) {
                Ok(m) => {
                    if let Some(crank) = m.crank {
                        self.feed_crank(crank, now);
                    }
                }
                Err(e) => out.warn(format!("CSC measurement: {e}")),
            },
            uuids::HEART_RATE_MEASUREMENT => match heart_rate::Measurement::parse(value) {
                Ok(m) => out.events.push(DeviceEvent::HeartRate {
                    bpm: f64::from(m.bpm),
                    timestamp: now,
                }),
                Err(e) => out.warn(format!("heart rate measurement: {e}")),
            },
            uuids::WAHOO_TRAINER_CONTROL => match wahoo::Response::parse(value) {
                Ok(r) => {
                    self.answered(r.op_code);
                    if !r.is_ok() {
                        out.warn(format!(
                            "Wahoo trainer rejected request 0x{:02x} (result {})",
                            r.op_code, r.result
                        ));
                    }
                }
                Err(e) => out.warn(format!("Wahoo control point: {e}")),
            },
            uuids::FITNESS_MACHINE_CONTROL_POINT => match ControlResponse::parse(value) {
                Ok(r) => self.on_control_response(&r, &mut out),
                Err(e) => out.warn(format!("control point: {e}")),
            },
            _ => {}
        }
        self.flush(&mut out, now);
        out
    }

    fn feed_crank(&mut self, crank: CrankData, now: Duration) {
        if !self.ftms_reports_cadence {
            self.crank_cadence = self.cadence.update(crank, now);
        }
    }

    fn cadence_now(&mut self, now: Duration) -> Option<f64> {
        if self.ftms_reports_cadence {
            self.crank_cadence
        } else {
            self.cadence.current(now).or(self.crank_cadence)
        }
    }

    fn on_control_response(&mut self, r: &ControlResponse, out: &mut Output) {
        self.answered(r.request_op_code);
        if self.awaiting == Some(r.request_op_code) {
            self.awaiting = None;
        }
        if r.request_op_code == ftms::op::REQUEST_CONTROL {
            if r.is_success() {
                self.control = ControlState::Granted;
                self.send(ControlRequest::StartOrResume, out);
                if let Some(cmd) = self.pending.take() {
                    self.send_command(cmd, out);
                }
            } else {
                self.control = ControlState::Refused;
                self.pending = None;
                out.warn(format!("trainer refused control: {:?}", r.result));
            }
        } else if !r.is_success() {
            out.warn(format!(
                "trainer rejected request 0x{:02x}: {:?}",
                r.request_op_code, r.result
            ));
            if r.result == ftms::ResultCode::ControlNotPermitted {
                // The trainer lost our control (e.g. another app took it):
                // queued targets are stale; the next command requests
                // control first.
                self.control = ControlState::Idle;
                self.queue.clear();
                self.in_flight = None;
            }
        }
    }

    fn send(&mut self, req: ControlRequest, out: &mut Output) {
        self.awaiting = Some(req.op_code());
        out.control_writes.push(req.encode());
    }

    fn send_command(&mut self, cmd: DeviceCommand, out: &mut Output) {
        match cmd {
            DeviceCommand::SetSimulation(p) => self.send(
                ControlRequest::SetSimulation {
                    wind_speed_mps: p.wind_speed_mps,
                    grade: p.grade,
                    crr: p.crr,
                    cw_kg_per_m: p.cw_kg_per_m,
                },
                out,
            ),
            DeviceCommand::SetTargetPower(w) => self.send(ControlRequest::SetTargetPower(w), out),
            _ => {}
        }
    }

    /// An app command (SIM / ERG target).
    pub fn on_command(&mut self, cmd: DeviceCommand, now: Duration) -> Output {
        let mut out = Output::default();
        if let Some(w) = self.wahoo.as_mut() {
            wahoo_command(w, cmd, &mut out);
            self.flush(&mut out, now);
            return out;
        }
        let supported = match (cmd, self.features) {
            (DeviceCommand::SetSimulation(_), Some(f)) => f.supports_simulation(),
            (DeviceCommand::SetTargetPower(_), Some(f)) => f.supports_power_target(),
            (DeviceCommand::SetSimulation(_) | DeviceCommand::SetTargetPower(_), None) => true,
            _ => return out, // not a trainer command
        };
        if !supported {
            out.warn(format!("trainer does not support {cmd:?}"));
            return out;
        }
        match self.control {
            ControlState::Unavailable => out.warn("device has no FTMS control point"),
            ControlState::Refused => out.warn("trainer refused control; command dropped"),
            ControlState::Granted => self.send_command(cmd, &mut out),
            ControlState::Requested { .. } => self.pending = Some(cmd),
            ControlState::Idle => {
                self.pending = Some(cmd);
                self.control = ControlState::Requested {
                    since: now,
                    attempt: 1,
                };
                self.send(ControlRequest::RequestControl, &mut out);
            }
        }
        self.flush(&mut out, now);
        out
    }

    /// Periodic tick: retries unanswered control requests.
    pub fn on_tick(&mut self, now: Duration) -> Output {
        let mut out = Output::default();
        if let ControlState::Requested { since, attempt } = self.control
            && now.saturating_sub(since) >= CONTROL_TIMEOUT
        {
            if attempt >= CONTROL_ATTEMPTS {
                self.control = ControlState::Refused;
                self.pending = None;
                out.warn("trainer did not answer the control request");
            } else {
                self.control = ControlState::Requested {
                    since: now,
                    attempt: attempt + 1,
                };
                self.send(ControlRequest::RequestControl, &mut out);
            }
        }
        self.flush(&mut out, now);
        out
    }
}

/// Wahoo control path: unlock once, SIM mode when needed, then the target.
fn wahoo_command(w: &mut WahooState, cmd: DeviceCommand, out: &mut Output) {
    let mut send = |r: wahoo::Request| out.control_writes.push(r.encode());
    if !matches!(
        cmd,
        DeviceCommand::SetSimulation(_) | DeviceCommand::SetTargetPower(_)
    ) {
        return;
    }
    if !w.unlocked {
        send(wahoo::Request::Unlock);
        w.unlocked = true;
    }
    match cmd {
        DeviceCommand::SetSimulation(p) => {
            let params = (p.mass_kg, p.crr, p.cw_kg_per_m);
            if w.sim != Some(params) {
                send(wahoo::Request::SimMode {
                    mass_kg: p.mass_kg,
                    crr: p.crr,
                    cw_kg_per_m: p.cw_kg_per_m,
                });
                w.sim = Some(params);
            }
            if p.wind_speed_mps != w.wind_mps {
                send(wahoo::Request::WindSpeed(p.wind_speed_mps));
                w.wind_mps = p.wind_speed_mps;
            }
            send(wahoo::Request::Grade(p.grade));
        }
        DeviceCommand::SetTargetPower(watts) => {
            send(wahoo::Request::Erg(watts));
            w.sim = None; // leaving SIM: send the SIM parameters again later
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimulationParameters;

    const TRAINER: [u16; 4] = [
        uuids::FITNESS_MACHINE_FEATURE,
        uuids::INDOOR_BIKE_DATA,
        uuids::FITNESS_MACHINE_CONTROL_POINT,
        uuids::CYCLING_POWER_MEASUREMENT,
    ];

    fn s(secs: f64) -> Duration {
        Duration::from_secs_f64(secs)
    }

    fn sim(grade: f64) -> DeviceCommand {
        DeviceCommand::SetSimulation(SimulationParameters {
            grade,
            wind_speed_mps: 0.0,
            crr: 0.004,
            cw_kg_per_m: 0.51,
            mass_kg: 83.0,
        })
    }

    fn ok(op: u8) -> [u8; 3] {
        [0x80, op, 0x01]
    }

    #[test]
    fn subscribes_only_to_offered_characteristics() {
        let p = TrainerProtocol::new(&TRAINER, None);
        assert_eq!(
            p.subscriptions(),
            vec![
                uuids::INDOOR_BIKE_DATA,
                uuids::CYCLING_POWER_MEASUREMENT,
                uuids::FITNESS_MACHINE_CONTROL_POINT
            ]
        );
        assert_eq!(p.describe(), "smart trainer (FTMS)");
        let hr = TrainerProtocol::new(&[uuids::HEART_RATE_MEASUREMENT], None);
        assert_eq!(hr.subscriptions(), vec![uuids::HEART_RATE_MEASUREMENT]);
        assert_eq!(hr.describe(), "heart-rate sensor");
        assert_eq!(hr.control_state(), ControlState::Unavailable);
    }

    #[test]
    fn ftms_power_and_cadence_become_events() {
        let mut p = TrainerProtocol::new(&TRAINER, None);
        let out = p.on_notification(
            uuids::INDOOR_BIKE_DATA,
            &[0x44, 0x02, 0xAD, 0x0C, 0xB4, 0x00, 0xFA, 0x00, 0x8E],
            s(1.0),
        );
        assert_eq!(
            out.events,
            vec![
                DeviceEvent::Power {
                    watts: 250.0,
                    cadence_rpm: Some(90.0),
                    timestamp: s(1.0)
                },
                DeviceEvent::HeartRate {
                    bpm: 142.0,
                    timestamp: s(1.0)
                }
            ]
        );
        // Once FTMS reports power, the Cycling Power Service is not used for power.
        let out = p.on_notification(
            uuids::CYCLING_POWER_MEASUREMENT,
            &[0, 0, 0x2C, 0x01],
            s(1.1),
        );
        assert!(out.events.is_empty());
    }

    #[test]
    fn power_meter_with_crank_cadence() {
        let mut p = TrainerProtocol::new(&[uuids::CYCLING_POWER_MEASUREMENT], None);
        // First crank sample: power without cadence.
        let out = p.on_notification(
            uuids::CYCLING_POWER_MEASUREMENT,
            &[0x20, 0x00, 0xC8, 0x00, 0x0A, 0x00, 0x00, 0x04],
            s(0.0),
        );
        assert_eq!(
            out.events,
            vec![DeviceEvent::Power {
                watts: 200.0,
                cadence_rpm: None,
                timestamp: s(0.0)
            }]
        );
        // +2 revolutions in 1 s → 120 rpm.
        let out = p.on_notification(
            uuids::CYCLING_POWER_MEASUREMENT,
            &[0x20, 0x00, 0xC8, 0x00, 0x0C, 0x00, 0x00, 0x08],
            s(1.0),
        );
        assert!(matches!(
            out.events[0],
            DeviceEvent::Power {
                cadence_rpm: Some(c),
                ..
            } if (c - 120.0).abs() < 1e-9
        ));
    }

    #[test]
    fn negative_power_is_reported_as_zero_and_garbage_warns() {
        let mut p = TrainerProtocol::new(&[uuids::CYCLING_POWER_MEASUREMENT], None);
        let out = p.on_notification(
            uuids::CYCLING_POWER_MEASUREMENT,
            &[0, 0, 0x18, 0xFC],
            s(0.0),
        );
        assert!(matches!(out.events[0], DeviceEvent::Power { watts, .. } if watts == 0.0));
        let out = p.on_notification(uuids::CYCLING_POWER_MEASUREMENT, &[0x20], s(0.1));
        assert!(out.events.is_empty());
        assert_eq!(out.warnings.len(), 1);
    }

    #[test]
    fn first_command_requests_control_then_starts_and_sends() {
        let mut p = TrainerProtocol::new(&TRAINER, None);
        let out = p.on_command(sim(0.02), s(0.0));
        assert_eq!(out.control_writes, vec![vec![ftms::op::REQUEST_CONTROL]]);
        // A newer command while waiting replaces the pending one.
        assert!(p.on_command(sim(0.03), s(0.5)).control_writes.is_empty());
        let out = p.on_notification(uuids::FITNESS_MACHINE_CONTROL_POINT, &ok(0x00), s(0.6));
        assert_eq!(p.control_state(), ControlState::Granted);
        // One procedure at a time: start first, the SIM target after its answer.
        assert_eq!(out.control_writes, vec![vec![ftms::op::START_OR_RESUME]]);
        let out = p.on_notification(uuids::FITNESS_MACHINE_CONTROL_POINT, &ok(0x07), s(0.7));
        assert_eq!(
            out.control_writes,
            vec![vec![0x11, 0x00, 0x00, 0x2C, 0x01, 0x28, 0x33]] // 3 %
        );
        // Still waiting for the SIM answer: ERG is queued, sent after it.
        assert!(
            p.on_command(DeviceCommand::SetTargetPower(200.0), s(1.0))
                .control_writes
                .is_empty()
        );
        let out = p.on_notification(uuids::FITNESS_MACHINE_CONTROL_POINT, &ok(0x11), s(1.1));
        assert_eq!(out.control_writes, vec![vec![0x05, 0xC8, 0x00]]);
    }

    #[test]
    fn unanswered_control_requests_are_retried_then_given_up() {
        let mut p = TrainerProtocol::new(&TRAINER, None);
        p.on_command(sim(0.02), s(0.0));
        assert!(p.on_tick(s(2.9)).control_writes.is_empty());
        assert_eq!(p.on_tick(s(3.0)).control_writes, vec![vec![0x00]]);
        assert_eq!(p.on_tick(s(6.0)).control_writes, vec![vec![0x00]]);
        let out = p.on_tick(s(9.0));
        assert!(out.control_writes.is_empty());
        assert_eq!(p.control_state(), ControlState::Refused);
        assert!(!out.warnings.is_empty());
        // Further commands are dropped with a warning.
        let out = p.on_command(sim(0.01), s(10.0));
        assert!(out.control_writes.is_empty() && !out.warnings.is_empty());
    }

    #[test]
    fn refused_control_and_lost_control() {
        let mut p = TrainerProtocol::new(&TRAINER, None);
        p.on_command(sim(0.02), s(0.0));
        let out = p.on_notification(
            uuids::FITNESS_MACHINE_CONTROL_POINT,
            &[0x80, 0x00, 0x05],
            s(0.1),
        );
        assert_eq!(p.control_state(), ControlState::Refused);
        assert!(out.control_writes.is_empty());

        let mut p = TrainerProtocol::new(&TRAINER, None);
        p.on_command(sim(0.02), s(0.0));
        p.on_notification(uuids::FITNESS_MACHINE_CONTROL_POINT, &ok(0x00), s(0.1));
        // Another app took over: the next command requests control again.
        p.on_notification(
            uuids::FITNESS_MACHINE_CONTROL_POINT,
            &[0x80, 0x11, 0x05],
            s(1.0),
        );
        assert_eq!(p.control_state(), ControlState::Idle);
        assert_eq!(
            p.on_command(sim(0.01), s(2.0)).control_writes,
            vec![vec![0x00]]
        );
    }

    #[test]
    fn unsupported_targets_and_missing_control_point() {
        // Features say: simulation supported, power target not.
        let features = Features {
            machine: 0,
            target_setting: 1 << 13,
        };
        let mut p = TrainerProtocol::new(&TRAINER, Some(features));
        let out = p.on_command(DeviceCommand::SetTargetPower(200.0), s(0.0));
        assert!(out.control_writes.is_empty() && !out.warnings.is_empty());
        assert_eq!(
            p.on_command(sim(0.0), s(0.0)).control_writes,
            vec![vec![0x00]]
        );

        let mut meter = TrainerProtocol::new(&[uuids::CYCLING_POWER_MEASUREMENT], None);
        let out = meter.on_command(sim(0.05), s(0.0));
        assert!(out.control_writes.is_empty() && !out.warnings.is_empty());
        // Non-trainer commands are ignored silently.
        assert_eq!(
            meter.on_command(DeviceCommand::SetVirtualPower(1.0), s(0.0)),
            Output::default()
        );
    }

    #[test]
    fn dedicated_heart_rate_sensor_wins_over_ftms_field() {
        let mut p = TrainerProtocol::new(
            &[uuids::INDOOR_BIKE_DATA, uuids::HEART_RATE_MEASUREMENT],
            None,
        );
        let out = p.on_notification(
            uuids::INDOOR_BIKE_DATA,
            &[0x40, 0x02, 0x00, 0x00, 0xFA, 0x00, 0x8E],
            s(0.0),
        );
        assert_eq!(out.events.len(), 1); // power only
        let out = p.on_notification(uuids::HEART_RATE_MEASUREMENT, &[0x00, 0x48], s(0.1));
        assert_eq!(
            out.events,
            vec![DeviceEvent::HeartRate {
                bpm: 72.0,
                timestamp: s(0.1)
            }]
        );
    }

    const WAHOO_KICKR: [u16; 2] = [
        uuids::CYCLING_POWER_MEASUREMENT,
        uuids::WAHOO_TRAINER_CONTROL,
    ];

    #[test]
    fn wahoo_trainer_without_ftms() {
        let mut p = TrainerProtocol::new(&WAHOO_KICKR, None);
        assert_eq!(p.describe(), "smart trainer (Wahoo control)");
        assert_eq!(p.control_characteristic(), uuids::WAHOO_TRAINER_CONTROL);
        assert_eq!(
            p.subscriptions(),
            vec![
                uuids::CYCLING_POWER_MEASUREMENT,
                uuids::WAHOO_TRAINER_CONTROL
            ]
        );
        let reply = |p: &mut TrainerProtocol, op: u8, result: u8, t: f64| {
            p.on_notification(uuids::WAHOO_TRAINER_CONTROL, &[0x01, op, result], s(t))
        };
        // First SIM command: unlock, SIM mode, gradient, one after the other.
        assert_eq!(
            p.on_command(sim(0.0), s(0.0)).control_writes,
            vec![vec![0x20, 0xEE, 0xFC]]
        );
        // Unlock answered with result 2 (not needed) is fine.
        let out = reply(&mut p, 0x20, 2, 0.1);
        assert!(out.warnings.is_empty());
        assert_eq!(
            out.control_writes,
            vec![vec![0x43, 0x6C, 0x20, 0x28, 0x00, 0xFE, 0x01]] // 83 kg, 0.004, 0.51
        );
        assert_eq!(
            reply(&mut p, 0x43, 1, 0.2).control_writes,
            vec![vec![0x46, 0x00, 0x80]]
        );
        assert!(reply(&mut p, 0x46, 1, 0.3).control_writes.is_empty());
        // Next gradient only.
        assert_eq!(
            p.on_command(sim(0.05), s(1.0)).control_writes,
            vec![vec![0x46, 0x66, 0x86]]
        );
        reply(&mut p, 0x46, 1, 1.1);
        // ERG, then back to SIM: the SIM parameters are sent again.
        assert_eq!(
            p.on_command(DeviceCommand::SetTargetPower(120.0), s(2.0))
                .control_writes,
            vec![vec![0x42, 0x78, 0x00]]
        );
        reply(&mut p, 0x42, 1, 2.1);
        assert_eq!(p.on_command(sim(0.0), s(3.0)).control_writes[0][0], 0x43);
        assert_eq!(reply(&mut p, 0x43, 1, 3.1).control_writes[0][0], 0x46);
        // A rejected request is reported.
        assert_eq!(reply(&mut p, 0x46, 3, 3.2).warnings.len(), 1);
    }

    /// Gradients arriving faster than the trainer answers: only the newest
    /// queued one is sent (recorded failure: a second write before the
    /// answer is rejected with ATT error 0x80).
    #[test]
    fn queued_targets_are_replaced_and_unanswered_writes_time_out() {
        let mut p = TrainerProtocol::new(&WAHOO_KICKR, None);
        p.on_command(sim(0.0), s(0.0)); // unlock in flight
        for (i, g) in [0.01, 0.02, 0.03].iter().enumerate() {
            assert!(
                p.on_command(sim(*g), s(0.1 + i as f64 * 0.1))
                    .control_writes
                    .is_empty()
            );
        }
        // Queue: SIM mode, then one gradient (3 %), not three.
        let out = p.on_notification(uuids::WAHOO_TRAINER_CONTROL, &[1, 0x20, 1], s(0.5));
        assert_eq!(out.control_writes[0][0], 0x43);
        let out = p.on_notification(uuids::WAHOO_TRAINER_CONTROL, &[1, 0x43, 1, 0], s(0.6));
        assert_eq!(
            out.control_writes,
            vec![wahoo::Request::Grade(0.03).encode()]
        );
        // No answer: after the timeout the next write goes out anyway.
        p.on_command(DeviceCommand::SetTargetPower(150.0), s(0.7));
        assert!(p.on_tick(s(1.0)).control_writes.is_empty());
        let out = p.on_tick(s(0.6 + WRITE_TIMEOUT.as_secs_f64()));
        assert_eq!(out.control_writes, vec![vec![0x42, 0x96, 0x00]]);
        assert!(!out.warnings.is_empty());
    }

    #[test]
    fn ftms_is_preferred_over_wahoo_control() {
        let mut both = TRAINER.to_vec();
        both.push(uuids::WAHOO_TRAINER_CONTROL);
        let p = TrainerProtocol::new(&both, None);
        assert_eq!(
            p.control_characteristic(),
            uuids::FITNESS_MACHINE_CONTROL_POINT
        );
        assert_eq!(p.describe(), "smart trainer (FTMS)");
    }
}

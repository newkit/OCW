//! ANT+ FE-C trainers (ANT+ Device Profile *Fitness Equipment*, device
//! type 17, channel period 8192; spec 14 Phase 17, 17E *FE-C trainers*,
//! Q5 (b)): the data pages a trainer broadcasts and the control pages OCW
//! sends it.
//!
//! - **Data:** page 16 (*General FE Data*: equipment type, elapsed time,
//!   distance, speed, FE state) and page 25 (*Specific Trainer /
//!   Stationary Bike Data*: event count, cadence, accumulated and
//!   instantaneous power, the trainer status bits – calibration and user
//!   configuration required –, the target power limits, FE state).
//!   [`FecDecoder`] keeps the rollovers (elapsed time 64 s, distance
//!   256 m, events 256, accumulated power 65536 W) and gives the average
//!   power over the events since the last page, as [`super::power`] does.
//! - **Control:** page 49 (*Target Power*, ERG), page 50 (*Wind
//!   Resistance*), page 51 (*Track Resistance*: grade, rolling
//!   resistance – SIM) and page 55 (*User Configuration*: rider and bike
//!   weight, wheel diameter), sent as acknowledged messages (`0x4F`).
//!   [`FecControl`] sends **one at a time**: each waits for the stick's
//!   `EVENT_TRANSFER_TX_COMPLETED` / `_FAILED`; a failed one is retried
//!   (up to [`ACK_ATTEMPTS`]) unless a newer page of the same kind waits,
//!   and a newer target replaces a queued one – as
//!   [`crate::ble::protocol`] queues FTMS control writes.
//!
//! Field values follow the FE-C profile: target power in 0.25 W; the wind
//! resistance coefficient in 0.01 kg/m (the profile's air drag force is
//! ½ · coefficient · (speed + wind)² · drafting factor, so the coefficient
//! is ρ · CdA = 2 × OCW's `cw_kg_per_m`, which is ½ · ρ · CdA and has the
//! drafting folded in already: the drafting factor is sent as 1.00); the
//! wind in km/h with an offset of 127 (positive: headwind); the grade in
//! 0.01 % with an offset of −200 %; Crr in 5 × 10⁻⁵; the user's weight in
//! 0.01 kg, the bike's in 0.05 kg (12 bits), the wheel diameter in 0.01 m
//! plus a 0 – 9 mm offset.

use std::collections::VecDeque;
use std::time::Duration;

use crate::{DeviceCommand, SimulationParameters};

/// Page 16: general FE data.
pub const PAGE_GENERAL: u8 = 0x10;
/// Page 25: specific trainer / stationary bike data.
pub const PAGE_TRAINER: u8 = 0x19;
/// Page 49: target power (ERG).
pub const PAGE_TARGET_POWER: u8 = 0x31;
/// Page 50: wind resistance.
pub const PAGE_WIND_RESISTANCE: u8 = 0x32;
/// Page 51: track resistance (SIM).
pub const PAGE_TRACK_RESISTANCE: u8 = 0x33;
/// Page 55: user configuration.
pub const PAGE_USER_CONFIGURATION: u8 = 0x37;

/// Equipment type of page 16 for trainers / stationary bikes.
pub const EQUIPMENT_TRAINER: u8 = 25;

/// The profile's defaults (what a trainer assumes for an invalid field).
pub const DEFAULT_CRR: f64 = 0.004;
pub const DEFAULT_WIND_COEFFICIENT: f64 = 0.51;
pub const DEFAULT_USER_KG: f64 = 75.0;
pub const DEFAULT_BIKE_KG: f64 = 10.0;
pub const DEFAULT_WHEEL_DIAMETER_M: f64 = 0.70;

/// The bike's share of OCW's rider + bike mass for page 55 (OCW's
/// [`SimulationParameters`] carries only the sum; a trainer uses the sum
/// for its inertia, so the split hardly matters).
pub const BIKE_KG: f64 = 9.0;
/// A 700c road wheel (700 × 25C, 2105 mm around): 0.670 m.
pub const WHEEL_DIAMETER_M: f64 = 0.670;

/// A page not answered by a transfer event in this time failed.
pub const ACK_TIMEOUT: Duration = Duration::from_secs(2);
/// Times one page is sent before it is given up.
pub const ACK_ATTEMPTS: u32 = 3;
/// The trainer asks for the user configuration: sent again at most this
/// often.
pub const USER_CONFIG_EVERY: Duration = Duration::from_secs(5);
/// No new power event for this long: 0 W (the rider stopped).
pub const IDLE_AFTER: Duration = Duration::from_secs(3);

/// The FE state (pages 16 and 25, byte 7 bits 4 – 6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FeState {
    #[default]
    Reserved,
    /// 1: asleep (off).
    Asleep,
    /// 2: ready.
    Ready,
    /// 3: in use.
    InUse,
    /// 4: finished (paused).
    Finished,
}

impl FeState {
    pub fn from_bits(b: u8) -> Self {
        match b & 0x07 {
            1 => Self::Asleep,
            2 => Self::Ready,
            3 => Self::InUse,
            4 => Self::Finished,
            _ => Self::Reserved,
        }
    }

    pub fn bits(self) -> u8 {
        match self {
            Self::Reserved => 0,
            Self::Asleep => 1,
            Self::Ready => 2,
            Self::InUse => 3,
            Self::Finished => 4,
        }
    }
}

/// The target power limits of page 25 (byte 7 bits 0 – 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetLimit {
    /// At the target power, or no target set.
    #[default]
    AtTarget,
    /// Too slow to reach the target (pedal faster or shift down).
    SpeedTooLow,
    /// Too fast to reach the target.
    SpeedTooHigh,
    /// Undetermined, maybe at the limit.
    Undetermined,
}

impl TargetLimit {
    pub fn from_bits(b: u8) -> Self {
        match b & 0x03 {
            0 => Self::AtTarget,
            1 => Self::SpeedTooLow,
            2 => Self::SpeedTooHigh,
            _ => Self::Undetermined,
        }
    }
}

/// The trainer status bits of page 25 (byte 6 bits 4 – 7) and its flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TrainerStatus {
    /// Bit 4: bicycle power calibration required.
    pub power_calibration_required: bool,
    /// Bit 5: resistance calibration required.
    pub resistance_calibration_required: bool,
    /// Bit 6: user configuration required (page 55).
    pub user_config_required: bool,
    pub target_limit: TargetLimit,
    pub state: FeState,
}

/// Page 16, read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct General {
    /// Bits 0 – 4 of byte 1 (25: trainer).
    pub equipment_type: u8,
    /// 0.25 s, rolls over at 64 s.
    pub elapsed_raw: u8,
    /// Metres, rolls over at 256 m.
    pub distance_raw: u8,
    /// m/s (`None`: invalid).
    pub speed_mps: Option<f64>,
    /// `None`: invalid.
    pub heart_rate: Option<u8>,
    /// Bit 2 of the capabilities: the distance field is used.
    pub distance_enabled: bool,
    /// Bit 3: virtual speed (the speed is computed, not measured).
    pub virtual_speed: bool,
    pub state: FeState,
    /// Bit 7: the lap toggle.
    pub lap_toggle: bool,
}

impl General {
    pub fn parse(p: &[u8; 8]) -> Option<Self> {
        if p[0] != PAGE_GENERAL {
            return None;
        }
        let speed = u16::from_le_bytes([p[4], p[5]]);
        Some(Self {
            equipment_type: p[1] & 0x1F,
            elapsed_raw: p[2],
            distance_raw: p[3],
            speed_mps: (speed != 0xFFFF).then(|| f64::from(speed) / 1000.0),
            heart_rate: (p[6] != 0xFF && p[6] != 0).then_some(p[6]),
            distance_enabled: p[7] & 0x04 != 0,
            virtual_speed: p[7] & 0x08 != 0,
            state: FeState::from_bits(p[7] >> 4),
            lap_toggle: p[7] & 0x80 != 0,
        })
    }
}

/// Page 25, read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trainer {
    pub event: u8,
    /// rpm (`None`: invalid).
    pub cadence_rpm: Option<u8>,
    /// W, rolls over at 65536.
    pub accumulated: u16,
    /// W, 12 bits (`None`: invalid, 0xFFF).
    pub instant: Option<u16>,
    pub status: TrainerStatus,
}

impl Trainer {
    pub fn parse(p: &[u8; 8]) -> Option<Self> {
        if p[0] != PAGE_TRAINER {
            return None;
        }
        let instant = u16::from(p[5]) | (u16::from(p[6] & 0x0F) << 8);
        let bits = p[6] >> 4;
        Some(Self {
            event: p[1],
            cadence_rpm: (p[2] != 0xFF).then_some(p[2]),
            accumulated: u16::from_le_bytes([p[3], p[4]]),
            instant: (instant != 0x0FFF).then_some(instant),
            status: TrainerStatus {
                power_calibration_required: bits & 0x01 != 0,
                resistance_calibration_required: bits & 0x02 != 0,
                user_config_required: bits & 0x04 != 0,
                target_limit: TargetLimit::from_bits(p[7]),
                state: FeState::from_bits(p[7] >> 4),
            },
        })
    }
}

/// Page 16 as a trainer sends it (the simulated trainer, tests).
pub fn general_page(
    elapsed_raw: u8,
    distance_raw: u8,
    speed_mps: Option<f64>,
    state: FeState,
) -> [u8; 8] {
    let speed = speed_mps.map_or(0xFFFF, |v| (v * 1000.0).round().clamp(0.0, 65534.0) as u16);
    let [s0, s1] = speed.to_le_bytes();
    [
        PAGE_GENERAL,
        EQUIPMENT_TRAINER,
        elapsed_raw,
        distance_raw,
        s0,
        s1,
        0xFF,
        // Capabilities: no HR source, distance used, real speed.
        0x04 | (state.bits() << 4),
    ]
}

/// Page 25 as a trainer sends it (the simulated trainer, tests).
pub fn trainer_page(
    event: u8,
    cadence_rpm: Option<u8>,
    accumulated: u16,
    instant: Option<u16>,
    status: TrainerStatus,
) -> [u8; 8] {
    let [a0, a1] = accumulated.to_le_bytes();
    let i = instant.map_or(0x0FFF, |w| w.min(0x0FFE));
    let mut bits = 0u8;
    if status.power_calibration_required {
        bits |= 0x01;
    }
    if status.resistance_calibration_required {
        bits |= 0x02;
    }
    if status.user_config_required {
        bits |= 0x04;
    }
    let limit = match status.target_limit {
        TargetLimit::AtTarget => 0,
        TargetLimit::SpeedTooLow => 1,
        TargetLimit::SpeedTooHigh => 2,
        TargetLimit::Undetermined => 3,
    };
    [
        PAGE_TRAINER,
        event,
        cadence_rpm.unwrap_or(0xFF),
        a0,
        a1,
        (i & 0xFF) as u8,
        ((i >> 8) as u8 & 0x0F) | (bits << 4),
        limit | (status.state.bits() << 4),
    ]
}

/// Page 49: target power (W, 0.25 W steps, 0 – 4000 W).
pub fn target_power(watts: f64) -> [u8; 8] {
    let raw = (watts * 4.0).round().clamp(0.0, 16000.0) as u16;
    let [lo, hi] = raw.to_le_bytes();
    [PAGE_TARGET_POWER, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, lo, hi]
}

/// Page 50: wind resistance – the coefficient ρ · CdA (kg/m, 0 – 1.86),
/// the wind (km/h, −127 … +127, positive: headwind), the drafting factor
/// (0 – 1).
pub fn wind_resistance(coefficient_kg_per_m: f64, wind_kmh: f64, drafting: f64) -> [u8; 8] {
    let c = (coefficient_kg_per_m * 100.0).round().clamp(0.0, 186.0) as u8;
    let w = (wind_kmh.round().clamp(-127.0, 127.0) + 127.0) as u8;
    let d = (drafting * 100.0).round().clamp(0.0, 100.0) as u8;
    [PAGE_WIND_RESISTANCE, 0xFF, 0xFF, 0xFF, 0xFF, c, w, d]
}

/// Page 51: track resistance – the grade (%, −200 … +200, 0.01 %) and the
/// rolling resistance coefficient (0 – 0.0127, 5 × 10⁻⁵).
pub fn track_resistance(grade_pct: f64, crr: f64) -> [u8; 8] {
    let g = ((grade_pct + 200.0) * 100.0).round().clamp(0.0, 40000.0) as u16;
    let [lo, hi] = g.to_le_bytes();
    let c = (crr / 5e-5).round().clamp(0.0, 254.0) as u8;
    [PAGE_TRACK_RESISTANCE, 0xFF, 0xFF, 0xFF, 0xFF, lo, hi, c]
}

/// Page 55: user configuration – the user's weight (0.01 kg), the bike's
/// (0.05 kg, 12 bits, 0 – 50 kg), the wheel diameter (0.01 m plus an
/// offset of 0 – 9 mm); the gear ratio is not given (0).
pub fn user_configuration(user_kg: f64, bike_kg: f64, wheel_diameter_m: f64) -> [u8; 8] {
    let u = (user_kg * 100.0).round().clamp(0.0, 65534.0) as u16;
    let [u0, u1] = u.to_le_bytes();
    let b = (bike_kg * 20.0).round().clamp(0.0, 1000.0) as u16;
    let mm = (wheel_diameter_m * 1000.0).round().clamp(0.0, 2549.0) as u16;
    let (cm, offset) = ((mm / 10) as u8, (mm % 10) as u8);
    [
        PAGE_USER_CONFIGURATION,
        u0,
        u1,
        0xFF,
        (offset & 0x0F) | (((b & 0x0F) as u8) << 4),
        (b >> 4) as u8,
        cm,
        0x00,
    ]
}

/// A control page, read back (the simulated trainer, tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Control {
    TargetPower {
        watts: f64,
    },
    Wind {
        coefficient_kg_per_m: f64,
        wind_kmh: f64,
        drafting: f64,
    },
    Track {
        grade_pct: f64,
        crr: f64,
    },
    User {
        user_kg: Option<f64>,
        bike_kg: Option<f64>,
        wheel_diameter_m: Option<f64>,
    },
}

impl Control {
    /// The page as the trainer reads it (invalid fields: the defaults).
    pub fn parse(p: &[u8; 8]) -> Option<Self> {
        Some(match p[0] {
            PAGE_TARGET_POWER => Self::TargetPower {
                watts: f64::from(u16::from_le_bytes([p[6], p[7]])) / 4.0,
            },
            PAGE_WIND_RESISTANCE => Self::Wind {
                coefficient_kg_per_m: if p[5] == 0xFF {
                    DEFAULT_WIND_COEFFICIENT
                } else {
                    f64::from(p[5]) / 100.0
                },
                wind_kmh: if p[6] == 0xFF {
                    0.0
                } else {
                    f64::from(p[6]) - 127.0
                },
                drafting: if p[7] == 0xFF {
                    1.0
                } else {
                    f64::from(p[7]) / 100.0
                },
            },
            PAGE_TRACK_RESISTANCE => {
                let g = u16::from_le_bytes([p[5], p[6]]);
                Self::Track {
                    grade_pct: if g == 0xFFFF {
                        0.0
                    } else {
                        f64::from(g) / 100.0 - 200.0
                    },
                    crr: if p[7] == 0xFF {
                        DEFAULT_CRR
                    } else {
                        f64::from(p[7]) / 20000.0
                    },
                }
            }
            PAGE_USER_CONFIGURATION => {
                let u = u16::from_le_bytes([p[1], p[2]]);
                let b = u16::from(p[4] >> 4) | (u16::from(p[5]) << 4);
                let offset = p[4] & 0x0F;
                Self::User {
                    user_kg: (u != 0xFFFF).then(|| f64::from(u) / 100.0),
                    bike_kg: (b != 0x0FFF).then(|| f64::from(b) / 20.0),
                    wheel_diameter_m: (p[6] != 0xFF).then(|| {
                        let off = if offset == 0x0F { 0 } else { offset };
                        (f64::from(p[6]) * 10.0 + f64::from(off)) / 1000.0
                    }),
                }
            }
            _ => return None,
        })
    }
}

/// What a page 25 gave.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FecReading {
    pub watts: f64,
    pub cadence_rpm: Option<f64>,
    /// From the last page 16.
    pub speed_mps: Option<f64>,
}

/// One trainer's data pages.
#[derive(Debug, Clone, Default)]
pub struct FecDecoder {
    last_power: Option<(u8, u16)>,
    last_event_at: Option<Duration>,
    idle_sent: bool,
    last_general: Option<(u8, u8)>,
    /// Seconds since the first page 16 (rollovers counted).
    pub elapsed_s: f64,
    /// Metres since the first page 16 (rollovers counted).
    pub distance_m: f64,
    pub speed_mps: Option<f64>,
    pub equipment_type: Option<u8>,
    /// The last page 25's status bits.
    pub status: TrainerStatus,
    /// The FE state of the last page 16 or 25.
    pub state: FeState,
}

impl FecDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// A page; page 25 gives a reading.
    pub fn on_page(&mut self, p: &[u8; 8], now: Duration) -> Option<FecReading> {
        if let Some(g) = General::parse(p) {
            if let Some((t, d)) = self.last_general {
                self.elapsed_s += f64::from(g.elapsed_raw.wrapping_sub(t)) * 0.25;
                if g.distance_enabled {
                    self.distance_m += f64::from(g.distance_raw.wrapping_sub(d));
                }
            }
            self.last_general = Some((g.elapsed_raw, g.distance_raw));
            self.speed_mps = g.speed_mps;
            self.equipment_type = Some(g.equipment_type);
            self.state = g.state;
            return None;
        }
        let t = Trainer::parse(p)?;
        self.status = t.status;
        self.state = t.status.state;
        let cadence = t.cadence_rpm.map(f64::from);
        let reading = |watts| FecReading {
            watts,
            cadence_rpm: cadence,
            speed_mps: self.speed_mps,
        };
        let out = match self.last_power {
            None => reading(f64::from(t.instant.unwrap_or(0))),
            Some((e, a)) if e != t.event => {
                let events = t.event.wrapping_sub(e);
                reading(f64::from(t.accumulated.wrapping_sub(a)) / f64::from(events))
            }
            Some(_) => {
                let idle = self
                    .last_event_at
                    .is_some_and(|at| now.saturating_sub(at) > IDLE_AFTER);
                if idle && !self.idle_sent {
                    self.idle_sent = true;
                    return Some(FecReading {
                        watts: 0.0,
                        cadence_rpm: Some(0.0),
                        speed_mps: self.speed_mps,
                    });
                }
                return None;
            }
        };
        self.last_power = Some((t.event, t.accumulated));
        self.last_event_at = Some(now);
        self.idle_sent = false;
        Some(out)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct InFlight {
    page: [u8; 8],
    attempts: u32,
    since: Duration,
}

/// The trainer's control pages, one acknowledged message at a time.
#[derive(Debug, Clone, Default)]
pub struct FecControl {
    queue: VecDeque<([u8; 8], u32)>,
    in_flight: Option<InFlight>,
    /// The last page 50 / 55 queued (sent again only when it changes).
    last_wind: Option<[u8; 8]>,
    last_user: Option<[u8; 8]>,
    /// Rider + bike mass of the last SIM command.
    mass_kg: Option<f64>,
    /// The latest SIM or ERG command (sent again after a reconnect).
    latest: Option<DeviceCommand>,
    user_asked_at: Option<Duration>,
    /// Pages given up after [`ACK_ATTEMPTS`] (for tests and the log).
    pub given_up: u32,
    /// Pages sent, attempts counted (for tests and the log).
    pub sent: u32,
}

/// The page numbers SIM uses.
fn is_sim(page: u8) -> bool {
    matches!(page, PAGE_WIND_RESISTANCE | PAGE_TRACK_RESISTANCE)
}

impl FecControl {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pages waiting (not counting the one in flight).
    pub fn queued(&self) -> Vec<[u8; 8]> {
        self.queue.iter().map(|(p, _)| *p).collect()
    }

    /// The page awaiting its transfer event.
    pub fn in_flight(&self) -> Option<[u8; 8]> {
        self.in_flight.map(|f| f.page)
    }

    /// Queues a page: a newer one replaces a queued one with the same
    /// number; ERG (49) and SIM (50, 51) replace each other's queued pages
    /// (the newest mode wins).
    fn push(&mut self, page: [u8; 8]) {
        let n = page[0];
        self.queue.retain(|(q, _)| {
            let m = q[0];
            m != n
                && !(n == PAGE_TARGET_POWER && is_sim(m))
                && !(is_sim(n) && m == PAGE_TARGET_POWER)
        });
        self.queue.push_back((page, 0));
    }

    fn user_page(&self) -> [u8; 8] {
        let total = self.mass_kg.unwrap_or(DEFAULT_USER_KG + BIKE_KG);
        user_configuration((total - BIKE_KG).max(1.0), BIKE_KG, WHEEL_DIAMETER_M)
    }

    /// A SIM / ERG command (others are ignored).
    pub fn on_command(&mut self, cmd: DeviceCommand) {
        match cmd {
            DeviceCommand::SetSimulation(p) => {
                self.latest = Some(cmd);
                self.sim(&p);
            }
            DeviceCommand::SetTargetPower(w) => {
                self.latest = Some(cmd);
                self.push(target_power(w));
            }
            _ => {}
        }
    }

    fn sim(&mut self, p: &SimulationParameters) {
        if p.mass_kg > 0.0 {
            self.mass_kg = Some(p.mass_kg);
            let user = self.user_page();
            if self.last_user != Some(user) {
                self.last_user = Some(user);
                self.push(user);
            }
        }
        let (grade, wind_mps) = p.load(true);
        let wind = wind_resistance(2.0 * p.cw_kg_per_m, wind_mps * 3.6, 1.0);
        if self.last_wind != Some(wind) {
            self.last_wind = Some(wind);
            self.push(wind);
        }
        self.push(track_resistance(grade * 100.0, p.crr));
    }

    /// The trainer says it needs the user configuration (page 25's bit):
    /// page 55 again, at most every [`USER_CONFIG_EVERY`].
    pub fn user_config_required(&mut self, now: Duration) {
        let busy = self
            .in_flight
            .is_some_and(|f| f.page[0] == PAGE_USER_CONFIGURATION)
            || self
                .queue
                .iter()
                .any(|(p, _)| p[0] == PAGE_USER_CONFIGURATION);
        if busy
            || self
                .user_asked_at
                .is_some_and(|t| now.saturating_sub(t) < USER_CONFIG_EVERY)
        {
            return;
        }
        self.user_asked_at = Some(now);
        let user = self.user_page();
        self.last_user = Some(user);
        // Before the targets: the trainer needs it to simulate them.
        self.queue.push_front((user, 0));
    }

    /// The next page to send, when none is in flight (an unanswered one
    /// counts as failed after [`ACK_TIMEOUT`]).
    pub fn next(&mut self, now: Duration) -> Option<[u8; 8]> {
        if let Some(f) = self.in_flight
            && now.saturating_sub(f.since) >= ACK_TIMEOUT
        {
            tracing::debug!(page = f.page[0], "FE-C page not answered in time");
            self.on_result(false);
        }
        if self.in_flight.is_some() {
            return None;
        }
        let (page, attempts) = self.queue.pop_front()?;
        self.in_flight = Some(InFlight {
            page,
            attempts: attempts + 1,
            since: now,
        });
        self.sent += 1;
        Some(page)
    }

    /// The stick's answer for the page in flight: transferred (`true`) or
    /// failed (`EVENT_TRANSFER_TX_FAILED`, a refused message). A failed
    /// page is sent again unless a newer one of its kind waits (the
    /// newest wins), up to [`ACK_ATTEMPTS`] times.
    pub fn on_result(&mut self, ok: bool) {
        let Some(f) = self.in_flight.take() else {
            return;
        };
        if ok {
            return;
        }
        let n = f.page[0];
        let superseded = self.queue.iter().any(|(q, _)| {
            q[0] == n
                || (n == PAGE_TARGET_POWER && is_sim(q[0]))
                || (is_sim(n) && q[0] == PAGE_TARGET_POWER)
        });
        if superseded {
            return;
        }
        if f.attempts < ACK_ATTEMPTS {
            self.queue.push_front((f.page, f.attempts));
        } else {
            self.given_up += 1;
            tracing::warn!(
                page = n,
                attempts = f.attempts,
                "the FE-C trainer did not take a control page"
            );
            // A page 50 / 55 given up is sent again with the next command.
            if n == PAGE_WIND_RESISTANCE {
                self.last_wind = None;
            }
            if n == PAGE_USER_CONFIGURATION {
                self.last_user = None;
            }
        }
    }

    /// The trainer was lost: nothing is in flight; once it is back,
    /// [`FecControl::reconnected`] sends the latest target again.
    pub fn lost(&mut self) {
        self.in_flight = None;
        self.queue.clear();
        self.last_wind = None;
        self.last_user = None;
        self.user_asked_at = None;
    }

    /// Found again (or for the first time): the latest command again.
    pub fn reconnected(&mut self) {
        self.lost();
        if let Some(cmd) = self.latest {
            self.on_command(cmd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: Duration = Duration::from_millis(250);

    fn sim(grade: f64, wind: f64, mass: f64) -> DeviceCommand {
        DeviceCommand::SetSimulation(SimulationParameters {
            grade,
            wind_speed_mps: wind,
            crr: 0.004,
            cw_kg_per_m: 0.255,
            mass_kg: mass,
            wind_grade: grade,
        })
    }

    #[test]
    fn control_pages_are_byte_exact() {
        // Page 49: 250 W = 1000 × 0.25 W.
        assert_eq!(
            target_power(250.0),
            [0x31, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xE8, 0x03]
        );
        assert_eq!(target_power(0.0)[6..], [0, 0]);
        // 4000 W is the most: 16000 = 0x3E80.
        assert_eq!(target_power(9000.0)[6..], [0x80, 0x3E]);
        assert_eq!(target_power(-5.0)[6..], [0, 0]);
        // Page 50: the profile's defaults (0.51 kg/m, still air, no
        // drafting) and a 10 km/h tailwind at 0.40 kg/m, drafting 0.75.
        assert_eq!(
            wind_resistance(0.51, 0.0, 1.0),
            [0x32, 0xFF, 0xFF, 0xFF, 0xFF, 0x33, 0x7F, 0x64]
        );
        assert_eq!(wind_resistance(0.40, -10.0, 0.75)[5..], [40, 117, 75]);
        assert_eq!(wind_resistance(5.0, 300.0, 2.0)[5..], [186, 254, 100]);
        assert_eq!(wind_resistance(0.3, -300.0, -1.0)[5..], [30, 0, 0]);
        // Page 51: 5 % = (5 + 200) / 0.01 = 20500 = 0x5014; Crr 0.004 =
        // 80 × 5e-5; −3.5 % = 19650 = 0x4CC2; flat = 20000 = 0x4E20.
        assert_eq!(
            track_resistance(5.0, 0.004),
            [0x33, 0xFF, 0xFF, 0xFF, 0xFF, 0x14, 0x50, 0x50]
        );
        assert_eq!(track_resistance(-3.5, 0.0033)[5..], [0xC2, 0x4C, 66]);
        assert_eq!(track_resistance(0.0, 0.02)[5..], [0x20, 0x4E, 254]);
        assert_eq!(track_resistance(-250.0, 0.0)[5..], [0, 0, 0]);
        // Page 55: 75 kg = 7500 = 0x1D4C; bike 10 kg = 200 × 0.05 kg =
        // 0x0C8 (low nibble in byte 4's top, the rest in byte 5); wheel
        // 0.700 m = 70 cm, offset 0; no gear ratio.
        assert_eq!(
            user_configuration(75.0, 10.0, 0.70),
            [0x37, 0x4C, 0x1D, 0xFF, 0x80, 0x0C, 70, 0x00]
        );
        // 62.35 kg, 8.45 kg = 169 = 0x0A9, 0.672 m = 67 cm + 2 mm.
        assert_eq!(
            user_configuration(62.35, 8.45, 0.672),
            [0x37, 0x5B, 0x18, 0xFF, 0x92, 0x0A, 67, 0x00]
        );
        // And read back as a trainer reads them.
        assert_eq!(
            Control::parse(&user_configuration(62.35, 8.45, 0.672)),
            Some(Control::User {
                user_kg: Some(62.35),
                bike_kg: Some(8.45),
                wheel_diameter_m: Some(0.672)
            })
        );
        assert_eq!(
            Control::parse(&track_resistance(-3.5, 0.004)),
            Some(Control::Track {
                grade_pct: -3.5,
                crr: 0.004
            })
        );
        assert_eq!(
            Control::parse(&[0x32, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]),
            Some(Control::Wind {
                coefficient_kg_per_m: 0.51,
                wind_kmh: 0.0,
                drafting: 1.0
            })
        );
        assert_eq!(
            Control::parse(&target_power(212.25)),
            Some(Control::TargetPower { watts: 212.25 })
        );
        assert_eq!(Control::parse(&[0x10; 8]), None);
    }

    #[test]
    fn page_16_reads_speed_distance_time_and_state_with_rollovers() {
        // A trainer, 25.2 s, 200 m, 8.333 m/s, no HR, distance used, in use.
        let p = [0x10, 0x19, 101, 200, 0x8D, 0x20, 0xFF, 0x34];
        let g = General::parse(&p).unwrap();
        assert_eq!(g.equipment_type, 25);
        assert_eq!(g.speed_mps, Some(8.333));
        assert_eq!(g.heart_rate, None);
        assert!(g.distance_enabled && !g.virtual_speed && !g.lap_toggle);
        assert_eq!(g.state, FeState::InUse);
        assert_eq!(general_page(101, 200, Some(8.333), FeState::InUse), p);
        // Invalid speed, the lap toggle, ready.
        let g = General::parse(&[0x10, 0x19, 0, 0, 0xFF, 0xFF, 0, 0xA4]).unwrap();
        assert_eq!(g.speed_mps, None);
        assert!(g.lap_toggle);
        assert_eq!(g.state, FeState::Ready);
        // Elapsed time rolls over at 64 s (256 × 0.25 s), distance at 256 m.
        let mut d = FecDecoder::new();
        d.on_page(&general_page(250, 250, Some(8.0), FeState::InUse), S);
        d.on_page(&general_page(6, 4, Some(8.0), FeState::InUse), S * 2);
        assert_eq!(d.elapsed_s, 3.0);
        assert_eq!(d.distance_m, 10.0);
        assert_eq!(d.speed_mps, Some(8.0));
        assert_eq!(d.equipment_type, Some(25));
    }

    #[test]
    fn page_25_reads_power_cadence_and_the_status_bits() {
        // 12-bit power 0x2BC = 700 W, cadence 95, accumulated 0x1234,
        // user configuration required, speed too low, in use.
        let p = [0x19, 7, 95, 0x34, 0x12, 0xBC, 0x42, 0x31];
        let t = Trainer::parse(&p).unwrap();
        assert_eq!(t.event, 7);
        assert_eq!(t.cadence_rpm, Some(95));
        assert_eq!(t.accumulated, 0x1234);
        assert_eq!(t.instant, Some(700));
        assert!(t.status.user_config_required);
        assert!(!t.status.power_calibration_required);
        assert!(!t.status.resistance_calibration_required);
        assert_eq!(t.status.target_limit, TargetLimit::SpeedTooLow);
        assert_eq!(t.status.state, FeState::InUse);
        assert_eq!(trainer_page(7, Some(95), 0x1234, Some(700), t.status), p);
        // Both calibration bits, invalid power and cadence.
        let t = Trainer::parse(&[0x19, 0, 0xFF, 0, 0, 0xFF, 0x3F, 0x23]).unwrap();
        assert!(t.status.power_calibration_required);
        assert!(t.status.resistance_calibration_required);
        assert!(!t.status.user_config_required);
        assert_eq!(t.instant, None);
        assert_eq!(t.cadence_rpm, None);
        assert_eq!(t.status.target_limit, TargetLimit::Undetermined);
        assert_eq!(t.status.state, FeState::Ready);
        // Average power over the events, across the rollovers of the
        // event count and the accumulated power.
        let st = TrainerStatus {
            state: FeState::InUse,
            ..Default::default()
        };
        let mut d = FecDecoder::new();
        d.on_page(&general_page(0, 0, Some(9.0), FeState::InUse), S);
        let r = d
            .on_page(&trainer_page(254, Some(90), 65400, Some(210), st), S)
            .unwrap();
        assert_eq!(r.watts, 210.0, "the first page: its instantaneous power");
        assert_eq!(r.speed_mps, Some(9.0));
        let r = d
            .on_page(&trainer_page(1, Some(91), 300, Some(150), st), S * 2)
            .unwrap();
        // 3 events, 436 W·events: 145.33 W.
        assert!((r.watts - 436.0 / 3.0).abs() < 1e-9, "{r:?}");
        assert_eq!(r.cadence_rpm, Some(91.0));
        // The same event again: nothing, then 0 W once after a while.
        let same = trainer_page(1, Some(91), 300, Some(150), st);
        assert!(d.on_page(&same, S * 3).is_none());
        let r = d.on_page(&same, S * 2 + Duration::from_secs(4)).unwrap();
        assert_eq!(r.watts, 0.0);
        assert!(d.on_page(&same, S * 2 + Duration::from_secs(5)).is_none());
    }

    #[test]
    fn sim_sends_user_wind_and_track_once_each_then_only_what_changes() {
        let mut c = FecControl::new();
        c.on_command(sim(0.05, 2.0, 84.0));
        let q = c.queued();
        assert_eq!(q.len(), 3);
        assert_eq!(q[0], user_configuration(75.0, BIKE_KG, WHEEL_DIAMETER_M));
        // 2 × 0.255 kg/m, 7.2 km/h headwind, no extra drafting.
        assert_eq!(q[1], wind_resistance(0.51, 7.2, 1.0));
        assert_eq!(q[2], track_resistance(5.0, 0.004));
        // One at a time: the next only after the transfer event.
        assert_eq!(c.next(S), Some(q[0]));
        assert_eq!(c.next(S), None);
        c.on_result(true);
        assert_eq!(c.next(S), Some(q[1]));
        c.on_result(true);
        assert_eq!(c.next(S), Some(q[2]));
        c.on_result(true);
        assert_eq!(c.next(S), None);
        // The same rider and wind: only the grade.
        c.on_command(sim(0.06, 2.0, 84.0));
        assert_eq!(c.queued(), [track_resistance(6.0, 0.004)]);
    }

    #[test]
    fn the_newest_target_wins_and_failed_pages_are_retried() {
        let mut c = FecControl::new();
        c.on_command(DeviceCommand::SetTargetPower(200.0));
        assert_eq!(c.next(S), Some(target_power(200.0)));
        // Two more while 200 W is in flight: only the newest waits.
        c.on_command(DeviceCommand::SetTargetPower(220.0));
        c.on_command(DeviceCommand::SetTargetPower(240.0));
        assert_eq!(c.queued(), [target_power(240.0)]);
        // 200 W failed: superseded by 240 W, not sent again.
        c.on_result(false);
        assert_eq!(c.next(S), Some(target_power(240.0)));
        // 240 W fails: sent again, up to three times, then given up.
        c.on_result(false);
        assert_eq!(c.next(S), Some(target_power(240.0)));
        c.on_result(false);
        assert_eq!(c.next(S), Some(target_power(240.0)));
        c.on_result(false);
        assert_eq!(c.next(S), None);
        assert_eq!(c.given_up, 1);
        // No answer at all: failed after the timeout, sent again.
        c.on_command(DeviceCommand::SetTargetPower(150.0));
        assert_eq!(c.next(S), Some(target_power(150.0)));
        assert_eq!(c.next(S + ACK_TIMEOUT / 2), None);
        assert_eq!(c.next(S + ACK_TIMEOUT), Some(target_power(150.0)));
        // A mode change replaces the other mode's queued pages: SIM after
        // ERG drops the waiting ERG target, and the other way round.
        c.on_command(DeviceCommand::SetTargetPower(160.0));
        c.on_command(sim(0.02, 0.0, 70.0));
        assert!(c.queued().iter().all(|p| p[0] != PAGE_TARGET_POWER));
        c.on_command(DeviceCommand::SetTargetPower(170.0));
        let q = c.queued();
        assert!(q.iter().all(|p| !is_sim(p[0])), "{q:?}");
        assert!(q.contains(&target_power(170.0)));
        // Page 55 queued before stays (not a target).
        assert_eq!(q[0][0], PAGE_USER_CONFIGURATION);
    }

    #[test]
    fn user_configuration_on_request_and_the_latest_target_after_a_reconnect() {
        let mut c = FecControl::new();
        // ERG without a rider yet: the trainer asks, the defaults go.
        c.on_command(DeviceCommand::SetTargetPower(180.0));
        c.user_config_required(S);
        assert_eq!(
            c.queued()[0],
            user_configuration(DEFAULT_USER_KG, BIKE_KG, WHEEL_DIAMETER_M)
        );
        // Asked again at once: not queued twice, and not again within 5 s.
        c.user_config_required(S);
        assert_eq!(c.queued().len(), 2);
        while c.next(S).is_some() {
            c.on_result(true);
        }
        c.user_config_required(S * 2);
        assert!(c.queued().is_empty());
        c.user_config_required(S + USER_CONFIG_EVERY);
        assert_eq!(c.queued().len(), 1);
        // Lost and found: the latest target again.
        c.lost();
        assert!(c.queued().is_empty() && c.in_flight().is_none());
        c.reconnected();
        assert_eq!(c.queued(), [target_power(180.0)]);
        c.on_command(sim(-0.02, 0.0, 90.0));
        c.reconnected();
        let q = c.queued();
        assert_eq!(q.len(), 3, "user, wind and track again: {q:?}");
        assert_eq!(q[2], track_resistance(-2.0, 0.004));
    }
}

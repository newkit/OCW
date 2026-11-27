//! A simulated ANT stick (spec 14 Phase 17 Q2: built and tested without
//! hardware): it reads the host's messages, answers like a stick (channel
//! responses, start-up, capabilities, channel IDs) and broadcasts the pages
//! of simulated heart-rate straps, power meters (power-only, or torque
//! pages only), speed / cadence sensors and FE-C trainers on the open
//! channels that match them, at the channel period – with the toggle bit,
//! background and common pages, R-R timing, extended data (channel ID,
//! signal), missed messages, lost sensors and search timeouts. An
//! acknowledged message from the host goes out in the channel's next slot
//! and reaches the tracked sensor: `EVENT_TRANSFER_TX_COMPLETED` (or
//! `_FAILED` when told to fail some, or when the sensor is gone), a second
//! one while one is under way is refused (`TRANSFER_IN_PROGRESS`). A
//! simulated FE-C trainer ([`SimTrainer`]) takes the control pages (49
//! ERG, 50 wind, 51 track, 55 user) and its power follows them with a
//! simple model; a power meter answers a calibration request with its
//! calibration page.
//!
//! Pure (the time is given), so the stick core's tests step it by hand;
//! the native `SimTransport` runs it on the wall clock for the app
//! (`ocw-desktop --ant-sim …`) and the desktop's tests.
//!
//! The network key: a real stick hears ANT+ sensors only with the ANT+
//! key; the simulated one only with the key it is made with (or any key,
//! [`SimRadio::new`] with `None`), so a wrong key finds nothing, as on the
//! air.

use std::time::Duration;

use super::codec::{Decoder, Extended, Message, code, id, lib_config};
use super::common;
use super::fec::{self, Control, FeState, TrainerStatus};
use super::hr;
use super::power::{self, Torque};
use super::spdcad::{self, SpdCadKind};
use super::stick::response;
use super::{AntProfile, ChannelId, NetworkKey};

/// Channels of the simulated stick.
pub const CHANNELS: usize = 8;
/// Missed messages before a channel goes back to search.
pub const MISSES_TO_SEARCH: u32 = 4;
/// The simulated wheel's circumference (m): 700 × 25C.
pub const SIM_WHEEL_M: f64 = 2.105;
/// A simulated power meter's zero offset (its own units).
pub const SIM_ZERO_OFFSET: i16 = 512;
/// Every this many messages a power meter sends a common page instead.
const COMMON_EVERY: u32 = 8;

/// What a simulated sensor is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SimKind {
    HeartRate {
        bpm: f64,
    },
    /// A power meter with the power-only page.
    Power {
        watts: f64,
        cadence: f64,
    },
    /// A power meter with torque pages only: crank torque (0x12), or wheel
    /// torque (0x11, the wheel at `kmh`).
    Torque {
        watts: f64,
        cadence: f64,
        crank: bool,
        kmh: f64,
    },
    /// A speed / cadence sensor (types 121 – 123).
    SpeedCadence {
        kind: SpdCadKind,
        rpm: f64,
        kmh: f64,
    },
    /// An FE-C trainer; `watts`: what the simulated rider rides on the
    /// flat (the pace they hold), `cadence` in rpm.
    Trainer {
        watts: f64,
        cadence: f64,
    },
}

/// What a simulated FE-C trainer was last told.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SimLoad {
    /// Nothing yet: the rider's own pace.
    Free,
    /// ERG (page 49): this power.
    Erg(f64),
    /// SIM (page 51): this grade (%) and rolling resistance.
    Sim { grade_pct: f64, crr: f64 },
}

/// The power follows a new load with this time constant (s).
pub const TRAINER_TAU_S: f64 = 1.0;
/// The most a simulated trainer gives (W).
pub const TRAINER_MAX_W: f64 = 2000.0;

/// A simulated FE-C trainer's state. The model: the simulated rider
/// holds the speed they ride on a flat road with their own power
/// ([`SimKind::Trainer`]'s `watts`, at the profile's default Crr 0.004
/// and wind coefficient 0.51 kg/m); in SIM the power is what the grade,
/// the rolling resistance and the wind cost at that speed, in ERG the
/// target; the power moves to it with [`TRAINER_TAU_S`].
#[derive(Debug, Clone, PartialEq)]
pub struct SimTrainer {
    pub load: SimLoad,
    /// Page 50 (rho * CdA, kg/m; km/h, positive: headwind; 0 – 1).
    pub wind_coefficient: f64,
    pub wind_kmh: f64,
    pub drafting: f64,
    /// Page 55 (`None`: not told yet – page 25 asks for it).
    pub user_kg: Option<f64>,
    pub bike_kg: Option<f64>,
    pub wheel_diameter_m: Option<f64>,
    /// The power and speed it reports now.
    pub power_w: f64,
    pub speed_mps: f64,
    /// Every control page it took, in order.
    pub received: Vec<[u8; 8]>,
    /// Transfers to fail before the next one goes through (tests).
    pub fail_acks: u32,
    last_t: Option<f64>,
    elapsed_s: f64,
    distance_m: f64,
}

impl Default for SimTrainer {
    fn default() -> Self {
        Self {
            load: SimLoad::Free,
            wind_coefficient: fec::DEFAULT_WIND_COEFFICIENT,
            wind_kmh: 0.0,
            drafting: 1.0,
            user_kg: None,
            bike_kg: None,
            wheel_diameter_m: None,
            power_w: 0.0,
            speed_mps: 0.0,
            received: Vec::new(),
            fail_acks: 0,
            last_t: None,
            elapsed_s: 0.0,
            distance_m: 0.0,
        }
    }
}

impl SimTrainer {
    /// Rider + bike (the profile's defaults until page 55).
    pub fn mass_kg(&self) -> f64 {
        self.user_kg.unwrap_or(fec::DEFAULT_USER_KG) + self.bike_kg.unwrap_or(fec::DEFAULT_BIKE_KG)
    }

    /// A control page.
    pub fn apply(&mut self, page: [u8; 8]) {
        self.received.push(page);
        match Control::parse(&page) {
            Some(Control::TargetPower { watts }) => self.load = SimLoad::Erg(watts),
            Some(Control::Track { grade_pct, crr }) => self.load = SimLoad::Sim { grade_pct, crr },
            Some(Control::Wind {
                coefficient_kg_per_m,
                wind_kmh,
                drafting,
            }) => {
                self.wind_coefficient = coefficient_kg_per_m;
                self.wind_kmh = wind_kmh;
                self.drafting = drafting;
            }
            Some(Control::User {
                user_kg,
                bike_kg,
                wheel_diameter_m,
            }) => {
                self.user_kg = user_kg.or(Some(fec::DEFAULT_USER_KG));
                self.bike_kg = bike_kg.or(Some(fec::DEFAULT_BIKE_KG));
                self.wheel_diameter_m = wheel_diameter_m;
            }
            None => {}
        }
    }

    /// The speed a rider of this mass holds on the flat with `watts`.
    pub fn pace(&self, watts: f64) -> f64 {
        let m = self.mass_kg();
        let p = |v: f64| {
            (m * 9.81 * fec::DEFAULT_CRR + 0.5 * fec::DEFAULT_WIND_COEFFICIENT * v * v) * v
        };
        let (mut lo, mut hi) = (0.0, 40.0);
        for _ in 0..50 {
            let mid = 0.5 * (lo + hi);
            if p(mid) < watts.max(0.0) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// The power the load asks for at speed `v`.
    pub fn load_power(&self, base_watts: f64, v: f64) -> f64 {
        let p = match self.load {
            SimLoad::Free => base_watts,
            SimLoad::Erg(w) => w,
            SimLoad::Sim { grade_pct, crr } => {
                let m = self.mass_kg();
                let air = v + self.wind_kmh / 3.6;
                let f = m * 9.81 * (grade_pct / 100.0 + crr)
                    + 0.5 * self.wind_coefficient * self.drafting * air * air.abs();
                f * v
            }
        };
        p.clamp(0.0, TRAINER_MAX_W)
    }

    /// Time passes (s since start).
    fn step(&mut self, t: f64, base_watts: f64) {
        let v = self.pace(base_watts);
        let target = self.load_power(base_watts, v);
        match self.last_t {
            None => self.power_w = target,
            Some(last) => {
                let dt = (t - last).max(0.0);
                self.power_w += (target - self.power_w) * (1.0 - (-dt / TRAINER_TAU_S).exp());
                self.elapsed_s += dt;
                self.distance_m += v * dt;
            }
        }
        self.speed_mps = v;
        self.last_t = Some(t);
    }
}

/// One simulated sensor.
#[derive(Debug, Clone, PartialEq)]
pub struct SimSensor {
    pub id: ChannelId,
    pub kind: SimKind,
    /// Signal (dBm).
    pub rssi: i8,
    /// In range (on the rider).
    pub present: bool,
    /// The heart beats (off: the strap lies on the table and repeats its
    /// last beat).
    pub beating: bool,
    /// A legacy strap: no toggle bit, no pages.
    pub legacy: bool,
    /// Power meters: auto zero (`None`: not supported).
    pub auto_zero: Option<bool>,
    /// Power meters: a manual zero fails (the cranks move).
    pub calibration_fails: bool,
    /// Power meters: they never answer a calibration.
    pub calibration_silent: bool,
    /// Acknowledged messages that fail to arrive before one gets through.
    pub ack_failures: u32,
    /// Acknowledged messages it received (calibration requests …).
    pub received: Vec<[u8; 8]>,
    // The generator.
    messages: u32,
    last_beat_s: f64,
    prev_beat_s: f64,
    next_beat_s: f64,
    count: u8,
    events: u8,
    accumulated: u16,
    /// Torque meters and speed / cadence sensors: the next crank / wheel
    /// event (s), the last one, the counts and sums.
    next_crank_s: f64,
    crank_at_s: f64,
    crank_revs: u16,
    next_wheel_s: f64,
    wheel_at_s: f64,
    wheel_revs: u16,
    period_sum: f64,
    torque_sum: f64,
    /// A calibration answer to send (after this many messages).
    answer: Option<(u32, [u8; 8])>,
    /// An FE-C trainer's state ([`SimKind::Trainer`]).
    pub trainer: Option<SimTrainer>,
}

impl SimSensor {
    pub fn new(id: ChannelId, kind: SimKind) -> Self {
        Self {
            id,
            kind,
            rssi: -58,
            present: true,
            beating: true,
            legacy: false,
            auto_zero: Some(false),
            calibration_fails: false,
            calibration_silent: false,
            ack_failures: 0,
            received: Vec::new(),
            messages: 0,
            last_beat_s: 0.0,
            prev_beat_s: 0.0,
            next_beat_s: 0.0,
            count: 0,
            events: 0,
            accumulated: 0,
            next_crank_s: 0.0,
            crank_at_s: 0.0,
            crank_revs: 0,
            next_wheel_s: 0.0,
            wheel_at_s: 0.0,
            wheel_revs: 0,
            period_sum: 0.0,
            torque_sum: 0.0,
            answer: None,
            trainer: matches!(kind, SimKind::Trainer { .. }).then(SimTrainer::default),
        }
    }

    /// An FE-C trainer with device number `n` (transmission type 5) whose
    /// rider rides `watts` on the flat at `cadence` rpm.
    pub fn fe_c_trainer(n: u16, watts: f64, cadence: f64) -> Self {
        Self::new(
            ChannelId::new(n, AntProfile::FitnessEquipment.device_type(), 5),
            SimKind::Trainer { watts, cadence },
        )
    }

    /// A heart-rate strap with device number `n` (transmission type 1).
    pub fn strap(n: u16, bpm: f64) -> Self {
        Self::new(
            ChannelId::new(n, AntProfile::HeartRate.device_type(), 1),
            SimKind::HeartRate { bpm },
        )
    }

    /// A power meter with device number `n` (transmission type 5).
    pub fn power_meter(n: u16, watts: f64, cadence: f64) -> Self {
        Self::new(
            ChannelId::new(n, AntProfile::BikePower.device_type(), 5),
            SimKind::Power { watts, cadence },
        )
    }

    /// A crank torque meter that sends no power-only page.
    pub fn torque_meter(n: u16, watts: f64, cadence: f64) -> Self {
        Self::new(
            ChannelId::new(n, AntProfile::BikePower.device_type(), 5),
            SimKind::Torque {
                watts,
                cadence,
                crank: true,
                kmh: 0.0,
            },
        )
    }

    /// A wheel torque meter (a power hub) at `kmh`.
    pub fn wheel_torque_meter(n: u16, watts: f64, kmh: f64) -> Self {
        Self::new(
            ChannelId::new(n, AntProfile::BikePower.device_type(), 5),
            SimKind::Torque {
                watts,
                cadence: 0.0,
                crank: false,
                kmh,
            },
        )
    }

    /// A speed / cadence sensor of `profile` (121, 122 or 123).
    pub fn speed_cadence(n: u16, profile: AntProfile, rpm: f64, kmh: f64) -> Self {
        let kind = match profile {
            AntProfile::Cadence => SpdCadKind::Cadence,
            AntProfile::Speed => SpdCadKind::Speed,
            _ => SpdCadKind::Combined,
        };
        let device_type = match kind {
            SpdCadKind::Combined => AntProfile::SpeedCadence,
            SpdCadKind::Cadence => AntProfile::Cadence,
            SpdCadKind::Speed => AntProfile::Speed,
        }
        .device_type();
        Self::new(
            ChannelId::new(n, device_type, 1),
            SimKind::SpeedCadence { kind, rpm, kmh },
        )
    }

    /// The made-up identity of simulated power meters: manufacturer 255
    /// (development), hardware 2, model 0x0100, software 1.2, a serial
    /// from the device number, 3.1 V good.
    fn common_page(&self, i: u32) -> [u8; 8] {
        let n = self.id.device_number;
        match i % 4 {
            0 => common::manufacturer_page(255, 2, 0x0100),
            1 => common::product_page(12, 0xFF, 0x0042_0000 | u32::from(n)),
            2 => common::battery_page(3.1, 2),
            _ => match self.auto_zero {
                Some(on) => power::auto_zero_support(on),
                None => common::battery_page(3.1, 2),
            },
        }
    }

    /// An acknowledged message from the host reached it.
    fn on_host_page(&mut self, p: [u8; 8]) {
        self.received.push(p);
        if !matches!(self.kind, SimKind::Power { .. } | SimKind::Torque { .. })
            || p[0] != power::PAGE_CALIBRATION
            || self.calibration_silent
        {
            return;
        }
        let answer = match p[1] {
            power::cal::REQUEST => power::calibration_response(
                !self.calibration_fails,
                self.auto_zero,
                SIM_ZERO_OFFSET,
            ),
            power::cal::AUTO_ZERO_CONFIG => {
                if self.auto_zero.is_some() {
                    self.auto_zero = Some(p[2] == 1);
                }
                power::calibration_response(true, self.auto_zero, SIM_ZERO_OFFSET)
            }
            _ => return,
        };
        // A crank meter takes a moment.
        self.answer = Some((2, answer));
    }

    /// Crank (or wheel) events up to `t` for a rate (per second): their
    /// count added, the last event's time.
    fn advance(next: &mut f64, last: &mut f64, t: f64, per_s: f64) -> u32 {
        if per_s <= 0.0 {
            *next = t + 1.0;
            return 0;
        }
        let step = 1.0 / per_s;
        if *next == 0.0 {
            // The counters start with an event now.
            *next = t + step;
            *last = t;
        }
        let mut n = 0;
        while t >= *next {
            *last = *next;
            *next += step;
            n += 1;
        }
        n
    }

    /// The next page at time `t` (s).
    fn page(&mut self, t: f64) -> [u8; 8] {
        let n = self.messages;
        self.messages += 1;
        if let Some((wait, a)) = self.answer {
            if wait == 0 {
                self.answer = None;
                return a;
            }
            self.answer = Some((wait - 1, a));
        }
        match self.kind {
            SimKind::HeartRate { bpm } => {
                if self.next_beat_s == 0.0 {
                    self.next_beat_s = t;
                }
                let interval = 60.0 / bpm.max(20.0);
                while self.beating && t >= self.next_beat_s {
                    self.prev_beat_s = self.last_beat_s;
                    self.last_beat_s = self.next_beat_s;
                    self.next_beat_s += interval;
                    self.count = self.count.wrapping_add(1);
                }
                if !self.beating {
                    self.next_beat_s = t + interval;
                }
                let ticks = |s: f64| ((s * 1024.0) as u64 % 65536) as u16;
                let time = ticks(self.last_beat_s);
                let computed = bpm.round().clamp(0.0, 255.0) as u8;
                if self.legacy {
                    return hr::page(0, false, [0xFF; 3], time, self.count, computed);
                }
                let toggle = (n / 4) % 2 == 1;
                let (number, b) = if n % 65 == 64 {
                    match (n / 65) % 3 {
                        // Manufacturer 1, serial 0x1234.
                        0 => (2, [0x01, 0x34, 0x12]),
                        // Hardware 2, software 7, model 9.
                        1 => (3, [2, 7, 9]),
                        // 80 %, 2.9 V, good.
                        _ => (7, [80, 230, 0x22]),
                    }
                } else {
                    let [p0, p1] = ticks(self.prev_beat_s).to_le_bytes();
                    (4, [0xFF, p0, p1])
                };
                hr::page(number, toggle, b, time, self.count, computed)
            }
            SimKind::Power { watts, cadence } => {
                if n % COMMON_EVERY == COMMON_EVERY - 1 {
                    return self.common_page(n / COMMON_EVERY);
                }
                let w = watts.round().clamp(0.0, 65535.0) as u16;
                self.events = self.events.wrapping_add(1);
                self.accumulated = self.accumulated.wrapping_add(w);
                let cad = (cadence > 0.0).then(|| cadence.round().clamp(0.0, 254.0) as u8);
                power::power_only(self.events, cad, self.accumulated, w)
            }
            SimKind::Torque {
                watts,
                cadence,
                crank,
                kmh,
            } => {
                if n % COMMON_EVERY == COMMON_EVERY - 1 {
                    return self.common_page(n / COMMON_EVERY);
                }
                // Event-synchronous: one event per crank (or wheel)
                // revolution, with its period and its torque.
                let per_s = if crank {
                    cadence / 60.0
                } else {
                    kmh / 3.6 / SIM_WHEEL_M
                };
                let revs = Self::advance(&mut self.next_crank_s, &mut self.crank_at_s, t, per_s);
                if revs > 0 {
                    let omega = 2.0 * std::f64::consts::PI * per_s;
                    let torque_nm = watts / omega;
                    for _ in 0..revs {
                        self.events = self.events.wrapping_add(1);
                        self.period_sum += 2048.0 / per_s;
                        self.torque_sum += torque_nm * 32.0;
                    }
                }
                let wrap = |x: f64| (x.round() as u64 % 65536) as u16;
                let instant = if crank {
                    cadence.round().clamp(0.0, 254.0) as u8
                } else {
                    0xFF
                };
                Torque {
                    event: self.events,
                    ticks: self.events,
                    cadence: instant,
                    period: wrap(self.period_sum),
                    torque: wrap(self.torque_sum),
                }
                .page(if crank {
                    power::PAGE_CRANK_TORQUE
                } else {
                    power::PAGE_WHEEL_TORQUE
                })
            }
            SimKind::SpeedCadence { kind, rpm, kmh } => {
                let c = Self::advance(&mut self.next_crank_s, &mut self.crank_at_s, t, rpm / 60.0);
                self.crank_revs = self.crank_revs.wrapping_add(c as u16);
                let w = Self::advance(
                    &mut self.next_wheel_s,
                    &mut self.wheel_at_s,
                    t,
                    kmh / 3.6 / SIM_WHEEL_M,
                );
                self.wheel_revs = self.wheel_revs.wrapping_add(w as u16);
                let ticks = |s: f64| ((s * 1024.0) as u64 % 65536) as u16;
                if kind == SpdCadKind::Combined {
                    return spdcad::combined_page(
                        ticks(self.crank_at_s),
                        self.crank_revs,
                        ticks(self.wheel_at_s),
                        self.wheel_revs,
                    );
                }
                let (time, revs) = if kind == SpdCadKind::Cadence {
                    (ticks(self.crank_at_s), self.crank_revs)
                } else {
                    (ticks(self.wheel_at_s), self.wheel_revs)
                };
                let toggle = (n / 4) % 2 == 1;
                let (number, b) = if n % 16 == 15 {
                    match (n / 16) % 3 {
                        // Manufacturer 255 (development), serial upper 0x0042.
                        0 => (2, [0xFF, 0x42, 0x00]),
                        // Hardware 1, software 2.0, model 3.
                        1 => (3, [1, 20, 3]),
                        // 2.9 V, good.
                        _ => (4, [0xFF, 230, 0x22]),
                    }
                } else {
                    (0, [0xFF; 3])
                };
                spdcad::page(number, toggle, b, time, revs)
            }
            SimKind::Trainer { watts, cadence } => {
                let tr = self.trainer.get_or_insert_with(SimTrainer::default);
                tr.step(t, watts);
                let cad = (cadence > 0.0).then(|| cadence.round().clamp(0.0, 254.0) as u8);
                // Pages 16 and 25 in turn: 16, 25, 25, 16, ...
                if n.is_multiple_of(3) {
                    let elapsed = ((tr.elapsed_s * 4.0) as u64 % 256) as u8;
                    let distance = (tr.distance_m as u64 % 256) as u8;
                    return fec::general_page(
                        elapsed,
                        distance,
                        Some(tr.speed_mps),
                        FeState::InUse,
                    );
                }
                let w = tr.power_w.round().clamp(0.0, 4094.0) as u16;
                self.events = self.events.wrapping_add(1);
                self.accumulated = self.accumulated.wrapping_add(w);
                let status = TrainerStatus {
                    user_config_required: tr.user_kg.is_none(),
                    state: FeState::InUse,
                    ..Default::default()
                };
                fec::trainer_page(self.events, cad, self.accumulated, Some(w), status)
            }
        }
    }
}

#[derive(Debug, Clone, Default)]
struct SimChannel {
    assigned: bool,
    open: bool,
    id: ChannelId,
    period: u16,
    lp_timeout: u8,
    hp_timeout: u8,
    list: [Option<ChannelId>; 4],
    list_size: u8,
    exclude: bool,
    /// Paired with (the channel ID becomes the sensor's until closed).
    paired: Option<ChannelId>,
    tracking: bool,
    misses: u32,
    search_since: Duration,
    next_at: Duration,
    /// An acknowledged message waiting for the next slot.
    ack: Option<[u8; 8]>,
}

impl SimChannel {
    fn period(&self) -> Duration {
        Duration::from_secs_f64(f64::from(self.period.max(1)) / 32768.0)
    }

    fn search_timeout(&self) -> Duration {
        let t = |x: u8| {
            if x == 255 {
                Duration::from_secs(3600 * 24)
            } else {
                Duration::from_millis(2500 * u64::from(x))
            }
        };
        t(self.lp_timeout) + t(self.hp_timeout)
    }

    fn allows(&self, s: &ChannelId) -> bool {
        let want = self.paired.unwrap_or(self.id);
        if !want.matches(s) {
            return false;
        }
        let n = usize::from(self.list_size).min(4);
        let listed = self.list[..n]
            .iter()
            .flatten()
            .any(|l| l.device_number == s.device_number && l.matches(s));
        if n == 0 {
            true
        } else if self.exclude {
            !listed
        } else {
            listed
        }
    }
}

/// The simulated stick.
#[derive(Debug, Clone)]
pub struct SimRadio {
    /// The key it hears sensors with (`None`: any).
    accepts: Option<NetworkKey>,
    key: Option<NetworkKey>,
    /// It supports extended data (lib config).
    pub extended: bool,
    /// It sends a start-up message after a reset.
    pub startup_message: bool,
    lib_flags: u8,
    channels: Vec<SimChannel>,
    sensors: Vec<SimSensor>,
    decoder: Decoder,
    out: Vec<u8>,
    /// Acknowledged messages refused because one was under way (a host
    /// that sends one at a time has none).
    pub busy_refusals: u32,
}

impl SimRadio {
    pub fn new(accepts: Option<NetworkKey>) -> Self {
        Self {
            accepts,
            key: None,
            extended: true,
            startup_message: true,
            lib_flags: 0,
            channels: vec![SimChannel::default(); CHANNELS],
            sensors: Vec::new(),
            decoder: Decoder::new(),
            out: Vec::new(),
            busy_refusals: 0,
        }
    }

    /// The simulated FE-C trainer with device number `n`.
    pub fn trainer(&self, n: u16) -> Option<&SimTrainer> {
        self.sensors
            .iter()
            .find(|s| s.id.device_number == n)
            .and_then(|s| s.trainer.as_ref())
    }

    pub fn trainer_mut(&mut self, n: u16) -> Option<&mut SimTrainer> {
        self.sensor_mut(n).and_then(|s| s.trainer.as_mut())
    }

    /// The rider's own power on the flat (an FE-C trainer's pace) or a
    /// power meter's power.
    pub fn set_watts(&mut self, n: u16, w: f64) {
        if let Some(s) = self.sensor_mut(n) {
            match &mut s.kind {
                SimKind::Trainer { watts, .. }
                | SimKind::Power { watts, .. }
                | SimKind::Torque { watts, .. } => *watts = w,
                SimKind::HeartRate { .. } | SimKind::SpeedCadence { .. } => {}
            }
        }
    }

    pub fn add(&mut self, s: SimSensor) {
        self.sensors
            .retain(|o| o.id.device_number != s.id.device_number);
        self.sensors.push(s);
    }

    pub fn remove(&mut self, device_number: u16) {
        self.sensors.retain(|s| s.id.device_number != device_number);
    }

    fn sensor_mut(&mut self, n: u16) -> Option<&mut SimSensor> {
        self.sensors.iter_mut().find(|s| s.id.device_number == n)
    }

    pub fn set_present(&mut self, n: u16, present: bool) {
        if let Some(s) = self.sensor_mut(n) {
            s.present = present;
        }
    }

    pub fn set_beating(&mut self, n: u16, beating: bool) {
        if let Some(s) = self.sensor_mut(n) {
            s.beating = beating;
        }
    }

    pub fn set_bpm(&mut self, n: u16, bpm: f64) {
        if let Some(s) = self.sensor_mut(n)
            && let SimKind::HeartRate { bpm: b } = &mut s.kind
        {
            *b = bpm;
        }
    }

    pub fn sensors(&self) -> &[SimSensor] {
        &self.sensors
    }

    /// A sensor to change (tests: make its calibration fail, its
    /// acknowledged messages get lost, …).
    pub fn sensor(&mut self, n: u16) -> Option<&mut SimSensor> {
        self.sensor_mut(n)
    }

    /// A meter's or a cadence sensor's cadence (rpm), a speed sensor's
    /// speed (km/h).
    pub fn set_cadence(&mut self, n: u16, c: f64) {
        if let Some(s) = self.sensor_mut(n) {
            match &mut s.kind {
                SimKind::Power { cadence, .. }
                | SimKind::Torque { cadence, .. }
                | SimKind::Trainer { cadence, .. } => *cadence = c,
                SimKind::SpeedCadence { rpm, .. } => *rpm = c,
                _ => {}
            }
        }
    }

    pub fn set_speed(&mut self, n: u16, v: f64) {
        if let Some(s) = self.sensor_mut(n) {
            match &mut s.kind {
                SimKind::SpeedCadence { kmh, .. } | SimKind::Torque { kmh, .. } => *kmh = v,
                _ => {}
            }
        }
    }

    /// What it would send to the host.
    pub fn take_output(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.out)
    }

    fn emit(&mut self, m: Message) {
        self.out.extend_from_slice(&m.encode());
    }

    fn respond(&mut self, m: &Message, c: u8) {
        self.emit(response(m, c));
    }

    fn event(&mut self, channel: u8, c: u8) {
        self.emit(Message::new(id::CHANNEL_EVENT, [channel, id::RF_EVENT, c]));
    }

    /// Bytes from the host.
    pub fn host(&mut self, bytes: &[u8], now: Duration) {
        for m in self.decoder.push(bytes) {
            self.on_message(&m, now);
        }
    }

    fn on_message(&mut self, m: &Message, now: Duration) {
        let ch = usize::from(m.data.first().copied().unwrap_or(0));
        let valid = ch < CHANNELS;
        let wrong = code::CHANNEL_IN_WRONG_STATE;
        match m.id {
            id::RESET_SYSTEM => {
                self.channels = vec![SimChannel::default(); CHANNELS];
                self.key = None;
                self.lib_flags = 0;
                if self.startup_message {
                    self.emit(Message::new(id::STARTUP, [0x20]));
                }
            }
            id::REQUEST => match m.data.get(1).copied() {
                Some(id::CAPABILITIES) => {
                    let adv2 = if self.extended { 0x80 } else { 0 };
                    self.emit(Message::new(
                        id::CAPABILITIES,
                        [CHANNELS as u8, 8, 0, 0xBA, adv2, 0],
                    ));
                }
                Some(id::CHANNEL_ID) if valid => {
                    let c = &self.channels[ch];
                    let cid = c.paired.unwrap_or(c.id);
                    let mut d = vec![ch as u8];
                    d.extend_from_slice(&cid.to_bytes());
                    self.emit(Message::new(id::CHANNEL_ID, d));
                }
                _ => self.respond(m, code::INVALID_MESSAGE),
            },
            id::SET_NETWORK_KEY if m.data.len() == 9 => {
                let mut k = [0u8; 8];
                k.copy_from_slice(&m.data[1..]);
                self.key = Some(NetworkKey::new(k));
                self.respond(m, code::RESPONSE_NO_ERROR);
            }
            id::LIB_CONFIG => {
                let flags = m.data.get(1).copied().unwrap_or(0);
                if self.extended || flags == 0 {
                    self.lib_flags = flags & (lib_config::CHANNEL_ID | lib_config::RSSI);
                    self.respond(m, code::RESPONSE_NO_ERROR);
                } else {
                    self.respond(m, code::INVALID_MESSAGE);
                }
            }
            id::ASSIGN_CHANNEL if valid => {
                if self.channels[ch].assigned {
                    self.respond(m, wrong);
                } else {
                    self.channels[ch] = SimChannel {
                        assigned: true,
                        period: 8192,
                        ..Default::default()
                    };
                    self.respond(m, code::RESPONSE_NO_ERROR);
                }
            }
            id::UNASSIGN_CHANNEL if valid => {
                if self.channels[ch].open {
                    self.respond(m, wrong);
                } else {
                    self.channels[ch] = SimChannel::default();
                    self.respond(m, code::RESPONSE_NO_ERROR);
                }
            }
            id::CHANNEL_ID
            | id::CHANNEL_PERIOD
            | id::RF_FREQUENCY
            | id::SEARCH_TIMEOUT
            | id::LOW_PRIORITY_SEARCH_TIMEOUT
            | id::ADD_CHANNEL_ID_TO_LIST
            | id::CONFIG_ID_LIST
                if valid =>
            {
                let c = &mut self.channels[ch];
                if !c.assigned {
                    self.respond(m, wrong);
                    return;
                }
                let d = &m.data;
                match m.id {
                    id::CHANNEL_ID => {
                        c.id = ChannelId::from_bytes(&d[1..]).unwrap_or_default();
                        c.paired = None;
                    }
                    id::CHANNEL_PERIOD if d.len() >= 3 => {
                        c.period = u16::from_le_bytes([d[1], d[2]]);
                    }
                    id::SEARCH_TIMEOUT if d.len() >= 2 => c.hp_timeout = d[1],
                    id::LOW_PRIORITY_SEARCH_TIMEOUT if d.len() >= 2 => c.lp_timeout = d[1],
                    id::ADD_CHANNEL_ID_TO_LIST if d.len() >= 6 => {
                        if let Some(slot) = c.list.get_mut(usize::from(d[5])) {
                            *slot = ChannelId::from_bytes(&d[1..5]);
                        }
                    }
                    id::CONFIG_ID_LIST if d.len() >= 3 => {
                        c.list_size = d[1];
                        c.exclude = d[2] != 0;
                    }
                    _ => {}
                }
                self.respond(m, code::RESPONSE_NO_ERROR);
            }
            id::OPEN_CHANNEL if valid => {
                let c = &mut self.channels[ch];
                if !c.assigned || c.open {
                    self.respond(m, wrong);
                    return;
                }
                c.open = true;
                c.tracking = false;
                c.paired = None;
                c.misses = 0;
                c.search_since = now;
                c.next_at = now + c.period();
                self.respond(m, code::RESPONSE_NO_ERROR);
            }
            id::CLOSE_CHANNEL if valid => {
                if !self.channels[ch].open {
                    self.respond(m, wrong);
                    return;
                }
                self.respond(m, code::RESPONSE_NO_ERROR);
                let c = &mut self.channels[ch];
                c.open = false;
                c.tracking = false;
                self.event(ch as u8, code::EVENT_CHANNEL_CLOSED);
            }
            id::ACKNOWLEDGED_DATA if valid && m.data.len() >= 9 => {
                let c = &mut self.channels[ch];
                if !c.open {
                    self.respond(m, wrong);
                } else if c.ack.is_some() {
                    self.busy_refusals += 1;
                    self.respond(m, code::TRANSFER_IN_PROGRESS);
                } else {
                    let mut p = [0u8; 8];
                    p.copy_from_slice(&m.data[1..9]);
                    c.ack = Some(p);
                }
            }
            _ => self.respond(m, code::INVALID_MESSAGE),
        }
    }

    fn hears(&self) -> bool {
        match (self.accepts, self.key) {
            (_, None) => false,
            (None, Some(_)) => true,
            (Some(a), Some(k)) => a == k,
        }
    }

    /// Time passes: the open channels receive (or miss, or time out).
    pub fn step(&mut self, now: Duration) {
        let hears = self.hears();
        for ch in 0..CHANNELS {
            // At most a few messages per step (a long step catches up).
            for _ in 0..16 {
                let c = &self.channels[ch];
                if !c.open || now < c.next_at {
                    break;
                }
                let at = c.next_at;
                let period = c.period();
                self.channels[ch].next_at = at + period;
                self.receive(ch, at, hears);
            }
        }
    }

    fn receive(&mut self, ch: usize, at: Duration, hears: bool) {
        let c = self.channels[ch].clone();
        let pick = if !hears {
            None
        } else if c.tracking {
            let p = c.paired.expect("tracking a sensor");
            self.sensors
                .iter()
                .position(|s| s.present && s.id.device_number == p.device_number)
        } else {
            self.sensors
                .iter()
                .position(|s| s.present && c.allows(&s.id))
        };
        match pick {
            Some(i) => {
                let t = at.as_secs_f64();
                let s = &mut self.sensors[i];
                let page = s.page(t);
                let (sid, rssi) = (s.id, s.rssi);
                let c = &mut self.channels[ch];
                c.tracking = true;
                c.paired = Some(sid);
                c.misses = 0;
                let mut d = vec![ch as u8];
                d.extend_from_slice(&page);
                Extended {
                    id: (self.lib_flags & lib_config::CHANNEL_ID != 0).then_some(sid),
                    rssi: (self.lib_flags & lib_config::RSSI != 0).then_some(rssi),
                }
                .encode(&mut d);
                self.emit(Message::new(id::BROADCAST_DATA, d));
                // An acknowledged message goes back in this slot.
                if let Some(p) = self.channels[ch].ack.take() {
                    let s = &mut self.sensors[i];
                    let ok = match s.trainer.as_mut() {
                        Some(tr) if tr.fail_acks > 0 => {
                            tr.fail_acks -= 1;
                            false
                        }
                        Some(tr) => {
                            tr.apply(p);
                            true
                        }
                        // Another sensor (a power meter's calibration).
                        None if s.ack_failures > 0 => {
                            s.ack_failures -= 1;
                            false
                        }
                        None => {
                            s.on_host_page(p);
                            true
                        }
                    };
                    let c = if ok {
                        code::EVENT_TRANSFER_TX_COMPLETED
                    } else {
                        code::EVENT_TRANSFER_TX_FAILED
                    };
                    self.event(ch as u8, c);
                }
            }
            None if c.tracking => {
                if self.channels[ch].ack.take().is_some() {
                    self.event(ch as u8, code::EVENT_TRANSFER_TX_FAILED);
                }
                let c = &mut self.channels[ch];
                c.misses += 1;
                if c.misses < MISSES_TO_SEARCH {
                    self.event(ch as u8, code::EVENT_RX_FAIL);
                } else {
                    c.tracking = false;
                    c.search_since = at;
                    self.event(ch as u8, code::EVENT_RX_FAIL_GO_TO_SEARCH);
                }
            }
            None => {
                if at.saturating_sub(c.search_since) >= c.search_timeout() {
                    let c = &mut self.channels[ch];
                    c.open = false;
                    self.event(ch as u8, code::EVENT_RX_SEARCH_TIMEOUT);
                    self.event(ch as u8, code::EVENT_CHANNEL_CLOSED);
                }
            }
        }
    }
}

/// Reads a `--ant-sim` list: `hr:12345:142,hr:23456:128,power:4004:230,
/// fec:5005:200` (kind, device number, bpm or watts; `power:N:W:RPM` and
/// `fec:N:W:RPM` with a cadence; an FE-C trainer's watts are what its
/// rider rides on the flat); `torque:N:W[:RPM]` a crank torque meter
/// without the power-only page, `wheeltorque:N:W[:KMH]` a power hub,
/// `spdcad:N:RPM:KMH` a combined speed / cadence sensor, `cadence:N:RPM`,
/// `speed:N:KMH`.
pub fn parse_sensors(spec: &str) -> Result<Vec<SimSensor>, String> {
    let mut out = Vec::new();
    for item in spec.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        let parts: Vec<&str> = item.split(':').collect();
        let num = |i: usize| -> Result<f64, String> {
            parts
                .get(i)
                .ok_or_else(|| format!("{item}: missing a number"))?
                .parse::<f64>()
                .map_err(|_| format!("{item}: not a number"))
        };
        let n = num(1)?;
        if !(1.0..=65535.0).contains(&n) {
            return Err(format!("{item}: device numbers are 1 – 65535"));
        }
        let n = n as u16;
        match parts[0] {
            "hr" => out.push(SimSensor::strap(n, num(2)?)),
            "power" => {
                let cadence = if parts.len() > 3 { num(3)? } else { 90.0 };
                out.push(SimSensor::power_meter(n, num(2)?, cadence));
            }
            "torque" => {
                let cadence = if parts.len() > 3 { num(3)? } else { 90.0 };
                out.push(SimSensor::torque_meter(n, num(2)?, cadence));
            }
            "wheeltorque" => {
                let kmh = if parts.len() > 3 { num(3)? } else { 30.0 };
                out.push(SimSensor::wheel_torque_meter(n, num(2)?, kmh));
            }
            "spdcad" => out.push(SimSensor::speed_cadence(
                n,
                AntProfile::SpeedCadence,
                num(2)?,
                num(3)?,
            )),
            "cadence" => out.push(SimSensor::speed_cadence(
                n,
                AntProfile::Cadence,
                num(2)?,
                0.0,
            )),
            "speed" => out.push(SimSensor::speed_cadence(n, AntProfile::Speed, 0.0, num(2)?)),
            "fec" | "trainer" => {
                let cadence = if parts.len() > 3 { num(3)? } else { 90.0 };
                out.push(SimSensor::fe_c_trainer(n, num(2)?, cadence));
            }
            k => {
                return Err(format!(
                    "{item}: kind {k} is not hr, power, torque, wheeltorque, spdcad, cadence, speed or fec"
                ));
            }
        }
    }
    if out.is_empty() {
        return Err("no simulated sensors (e.g. hr:12345:142)".into());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ant::codec::Incoming;
    use crate::ant::hr::HrDecoder;

    #[test]
    fn the_simulated_strap_follows_the_profile() {
        let mut s = SimSensor::strap(1, 120.0);
        let mut d = HrDecoder::new();
        let mut rr = Vec::new();
        let mut toggles = 0;
        let mut last_toggle = None;
        for i in 0..200u32 {
            let t = f64::from(i) * 8070.0 / 32768.0;
            let p = s.page(t);
            let toggle = p[0] & 0x80 != 0;
            if last_toggle.is_some_and(|l| l != toggle) {
                toggles += 1;
            }
            last_toggle = Some(toggle);
            let r = d.on_page(&p, Duration::from_secs_f64(t));
            assert_eq!(r.bpm, Some(120));
            rr.extend(r.rr_s);
        }
        assert!(toggles >= 40, "flips every 4 messages: {toggles}");
        assert!(d.pages_valid());
        assert_eq!(d.manufacturer, Some(1));
        assert_eq!(d.model, Some(9));
        assert_eq!(d.battery.unwrap().level_pct, Some(80));
        // 120 bpm: 0.5 s between beats.
        assert!(rr.len() > 50);
        assert!(rr.iter().all(|r| (r - 0.5).abs() < 0.002), "{rr:?}");
    }

    #[test]
    fn the_simulated_meters_and_sensors_follow_their_profiles() {
        use crate::ant::power::PowerDecoder;
        use crate::ant::spdcad::SpdCadDecoder;
        let period = 8182.0 / 32768.0;
        // A crank torque meter: 250 W at 85 rpm from torque pages alone.
        let mut s = SimSensor::torque_meter(9, 250.0, 85.0);
        let mut d = PowerDecoder::new();
        let mut watts = Vec::new();
        let mut cad = Vec::new();
        for i in 1..400u32 {
            let t = f64::from(i) * period;
            let p = s.page(t);
            assert_ne!(p[0], power::PAGE_POWER_ONLY);
            if let Some(r) = d.on_page(&p, Duration::from_secs_f64(t)) {
                watts.push(r.watts);
                cad.extend(r.cadence_rpm);
            }
        }
        assert!(watts.len() > 80, "{}", watts.len());
        assert!(watts.iter().all(|w| (w - 250.0).abs() < 1.5), "{watts:?}");
        assert!(cad.iter().all(|c| (c - 85.0).abs() < 0.2), "{cad:?}");
        assert_eq!(d.common.manufacturer, Some(255));
        assert_eq!(d.common.serial, Some(0x0042_0009));
        assert_eq!(d.auto_zero, Some(false));
        // A combined speed / cadence sensor at 90 rpm and 32 km/h.
        let mut s = SimSensor::speed_cadence(21, AntProfile::SpeedCadence, 90.0, 32.0);
        let mut d = SpdCadDecoder::new(SpdCadKind::Combined);
        let mut kmh = Vec::new();
        let mut rpm = Vec::new();
        for i in 1..400u32 {
            let t = f64::from(i) * 8086.0 / 32768.0;
            let r = d.on_page(&s.page(t), Duration::from_secs_f64(t));
            rpm.extend(r.cadence_rpm);
            kmh.extend(r.wheel_rps.map(|w| w * SIM_WHEEL_M * 3.6));
        }
        assert!(rpm.iter().all(|c| (c - 90.0).abs() < 0.5), "{rpm:?}");
        assert!(kmh.iter().all(|v| (v - 32.0).abs() < 0.2), "{kmh:?}");
        // A cadence sensor with its pages.
        let mut s = SimSensor::speed_cadence(22, AntProfile::Cadence, 75.0, 0.0);
        let mut d = SpdCadDecoder::new(SpdCadKind::Cadence);
        for i in 1..100u32 {
            let t = f64::from(i) * 8102.0 / 32768.0;
            d.on_page(&s.page(t), Duration::from_secs_f64(t));
        }
        assert!(d.pages_valid());
        assert_eq!(d.manufacturer, Some(255));
        assert_eq!(d.model, Some(3));
    }

    #[test]
    fn the_stick_answers_and_rejects_like_one() {
        let mut r = SimRadio::new(None);
        r.host(&Message::reset().encode(), Duration::ZERO);
        let mut dec = Decoder::new();
        let got = dec.push(&r.take_output());
        assert_eq!(got[0].parse(), Incoming::StartUp(0x20));
        // Configuring a channel that is not assigned: wrong state.
        r.host(&Message::channel_period(0, 8070).encode(), Duration::ZERO);
        assert_eq!(
            dec.push(&r.take_output())[0].parse(),
            Incoming::Response {
                channel: 0,
                to: id::CHANNEL_PERIOD,
                code: code::CHANNEL_IN_WRONG_STATE
            }
        );
    }

    #[test]
    fn sim_lists_parse() {
        let s = parse_sensors("hr:12345:142, power:4004:230:85").unwrap();
        assert_eq!(s[0].id, ChannelId::new(12345, 120, 1));
        assert_eq!(s[0].kind, SimKind::HeartRate { bpm: 142.0 });
        assert_eq!(
            s[1].kind,
            SimKind::Power {
                watts: 230.0,
                cadence: 85.0
            }
        );
        let s = parse_sensors("torque:9:250:85,spdcad:21:90:32,cadence:22:80,speed:23:28").unwrap();
        assert_eq!(s[0].id, ChannelId::new(9, 11, 5));
        assert!(matches!(s[0].kind, SimKind::Torque { crank: true, .. }));
        assert_eq!(s[1].id, ChannelId::new(21, 121, 1));
        assert_eq!(s[2].id, ChannelId::new(22, 122, 1));
        assert_eq!(s[3].id, ChannelId::new(23, 123, 1));
        assert_eq!(
            s[3].kind,
            SimKind::SpeedCadence {
                kind: SpdCadKind::Speed,
                rpm: 0.0,
                kmh: 28.0
            }
        );
        assert!(parse_sensors("spdcad:21:90").is_err());
        assert!(parse_sensors("").is_err());
        let t = parse_sensors("fec:5005:180:85").unwrap();
        assert_eq!(t[0].id, ChannelId::new(5005, 17, 5));
        assert_eq!(
            t[0].kind,
            SimKind::Trainer {
                watts: 180.0,
                cadence: 85.0
            }
        );
        assert!(t[0].trainer.is_some());
        assert!(parse_sensors("rower:1:2").is_err());
        assert!(parse_sensors("hr:0:100").is_err());
        assert!(parse_sensors("hr:12").is_err());
    }
}

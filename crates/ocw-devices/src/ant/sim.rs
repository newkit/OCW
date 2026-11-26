//! A simulated ANT stick (spec 14 Phase 17 Q2: built and tested without
//! hardware): it reads the host's messages, answers like a stick (channel
//! responses, start-up, capabilities, channel IDs) and broadcasts the pages
//! of simulated heart-rate straps and power meters on the open channels
//! that match them, at the channel period – with the toggle bit,
//! background pages, R-R timing, extended data (channel ID, signal),
//! missed messages, lost sensors and search timeouts.
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
use super::hr;
use super::power;
use super::stick::response;
use super::{AntProfile, ChannelId, NetworkKey};

/// Channels of the simulated stick.
pub const CHANNELS: usize = 8;
/// Missed messages before a channel goes back to search.
pub const MISSES_TO_SEARCH: u32 = 4;

/// What a simulated sensor is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SimKind {
    HeartRate { bpm: f64 },
    Power { watts: f64, cadence: f64 },
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
    // The strap's generator.
    messages: u32,
    last_beat_s: f64,
    prev_beat_s: f64,
    next_beat_s: f64,
    count: u8,
    events: u8,
    accumulated: u16,
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
            messages: 0,
            last_beat_s: 0.0,
            prev_beat_s: 0.0,
            next_beat_s: 0.0,
            count: 0,
            events: 0,
            accumulated: 0,
        }
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

    /// The next page at time `t` (s).
    fn page(&mut self, t: f64) -> [u8; 8] {
        let n = self.messages;
        self.messages += 1;
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
                let w = watts.round().clamp(0.0, 65535.0) as u16;
                self.events = self.events.wrapping_add(1);
                self.accumulated = self.accumulated.wrapping_add(w);
                let cad = (cadence > 0.0).then(|| cadence.round().clamp(0.0, 254.0) as u8);
                power::power_only(self.events, cad, self.accumulated, w)
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
            }
            None if c.tracking => {
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

/// Reads a `--ant-sim` list: `hr:12345:142,hr:23456:128,power:4004:230`
/// (kind, device number, bpm or watts; `power:N:W:RPM` with a cadence).
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
            k => return Err(format!("{item}: kind {k} is not hr or power")),
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
        assert!(parse_sensors("").is_err());
        assert!(parse_sensors("fec:1:2").is_err());
        assert!(parse_sensors("hr:0:100").is_err());
        assert!(parse_sensors("hr:12").is_err());
    }
}

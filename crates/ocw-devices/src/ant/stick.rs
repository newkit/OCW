//! The stick core (spec 3.4.1 *Several sensors*, *Search and pairing*,
//! *Reconnection*): sets the stick up once – reset, capabilities, the
//! network key, extended data – then hands out its channels: one per
//! paired sensor ([`StickCore::add_sensor`]) and one per profile while a
//! scan runs ([`StickCore::start_scan`]). Every message from the stick is
//! routed to its channel by number; the sensors' pages become
//! [`DeviceEvent`]s, the scan's finds [`FoundDevice`]s.
//!
//! Pure and driven from outside: bytes in ([`StickCore::on_bytes`]), time
//! ([`StickCore::on_tick`], a [`Duration`] since the service started), the
//! bytes to send out ([`StickCore::take_writes`]); the native service
//! (`service.rs`) runs it on its thread, the tests step it by hand.
//!
//! - A sensor's channel searches for its device number; once its pages
//!   arrive it is `Connected`; a lost sensor (`EVENT_RX_FAIL_GO_TO_SEARCH`)
//!   is `Disconnected` and searched for again by the stick; a channel that
//!   closed after its search timeout is opened again after
//!   [`RECONNECT_DELAY`] (`Failed { NotFound }` first if it never found
//!   it).
//! - A scan searches with a wildcard channel per profile and an
//!   exclusion list: each find is reported, put on the list, and the
//!   channel opened again, so each round finds the next sensor (up to
//!   [`MAX_LIST`] per profile).
//! - An FE-C trainer ([`StickCore::command`]) gets the SIM / ERG commands
//!   as acknowledged messages on its channel, one at a time while it is
//!   tracked, each answered by `EVENT_TRANSFER_TX_COMPLETED` / `_FAILED`
//!   ([`super::fec::FecControl`]).
//! - An unplugged stick ends every channel (`Failed { NoStick }`,
//!   `Disconnected`); once it is back, every sensor gets a channel again.

use std::collections::VecDeque;
use std::time::Duration;

use super::channel::{Channel, ChannelConfig, ChannelEvent, ChannelState, MAX_LIST};
use super::codec::{self, Decoder, Incoming, Message, code, id, lib_config};
use super::fec::{FecControl, FecDecoder};
use super::hr::HrDecoder;
use super::power::PowerDecoder;
use super::{AntDeviceId, AntProfile, ChannelId, NetworkKey, RECONNECT_DELAY};
use crate::ble::FoundDevice;
use crate::{DeviceCommand, DeviceEvent, FailReason};

/// Who asked for a sensor (the service's device handles).
pub type UserId = u64;

/// Channels to assume when the stick does not say.
pub const DEFAULT_CHANNELS: u8 = 8;
/// Waiting for the start-up message after a reset (some sticks send
/// none: go on after this).
pub const RESET_WAIT: Duration = Duration::from_millis(1000);
/// Waiting for the capabilities.
pub const CAPABILITIES_WAIT: Duration = Duration::from_millis(1000);
/// Waiting for a setup response.
pub const SETUP_WAIT: Duration = Duration::from_secs(2);
/// A scan channel's low-priority search timeout (× 2.5 s): short, so the
/// scan's rounds go on.
pub const SCAN_SEARCH_TIMEOUT: u8 = 2;

/// What the core reports.
#[derive(Debug, Clone, PartialEq)]
pub enum Output {
    /// For one device handle.
    Event(UserId, DeviceEvent),
    /// The scan found a sensor.
    Found(FoundDevice),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No stick open.
    Down,
    Reset {
        since: Duration,
    },
    Capabilities {
        since: Duration,
    },
    Key {
        since: Duration,
    },
    LibConfig {
        flags: u8,
        since: Duration,
    },
    Ready,
    /// The stick refused the setup (the key): nothing works until a new
    /// key or a new stick.
    Refused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Owner {
    Sensor(UserId),
    Scan(AntProfile),
}

#[derive(Debug, Clone)]
struct Slot {
    channel: Channel,
    owner: Owner,
    /// Being closed and unassigned for good (the sensor was removed, the
    /// scan ended).
    retiring: bool,
}

#[derive(Debug, Clone)]
enum Pages {
    Hr(HrDecoder),
    Power(PowerDecoder),
    /// An FE-C trainer: its data pages and its control queue.
    Fec(FecDecoder, FecControl),
}

#[derive(Debug, Clone)]
struct Sensor {
    user: UserId,
    device: AntDeviceId,
    channel: Option<u8>,
    pages: Pages,
    connected: bool,
    /// When a closed channel searches again.
    reopen_at: Option<Duration>,
    /// The last failure told (told once per reason).
    told: Option<FailReason>,
    stale_told: bool,
}

#[derive(Debug, Clone)]
struct ScanRound {
    profile: AntProfile,
    channel: Option<u8>,
    found: Vec<ChannelId>,
    done: bool,
}

#[derive(Debug, Clone)]
struct Scan {
    until: Duration,
    rounds: Vec<ScanRound>,
}

/// The stick's state and its channels.
#[derive(Debug)]
pub struct StickCore {
    key: Option<NetworkKey>,
    phase: Phase,
    decoder: Decoder,
    slots: Vec<Option<Slot>>,
    max_channels: u8,
    /// The extended data the stick gives (lib config flags).
    ext_flags: u8,
    sensors: Vec<Sensor>,
    scan: Option<Scan>,
    writes: VecDeque<Vec<u8>>,
}

impl StickCore {
    pub fn new(key: Option<NetworkKey>) -> Self {
        Self {
            key,
            phase: Phase::Down,
            decoder: Decoder::new(),
            slots: Vec::new(),
            max_channels: DEFAULT_CHANNELS,
            ext_flags: 0,
            sensors: Vec::new(),
            scan: None,
            writes: VecDeque::new(),
        }
    }

    pub fn has_key(&self) -> bool {
        self.key.is_some()
    }

    /// Set up and ready for channels.
    pub fn is_ready(&self) -> bool {
        self.phase == Phase::Ready
    }

    /// The stick refused the network key.
    pub fn refused(&self) -> bool {
        self.phase == Phase::Refused
    }

    /// Something needs the stick open (a sensor or a scan).
    pub fn wants_link(&self) -> bool {
        !self.sensors.is_empty() || self.scan.is_some()
    }

    pub fn scanning(&self) -> bool {
        self.scan.is_some()
    }

    /// The extended data in use (lib config flags; 0: none).
    pub fn extended_flags(&self) -> u8 {
        self.ext_flags
    }

    /// Channels assigned now.
    pub fn channels_in_use(&self) -> usize {
        self.slots.iter().flatten().count()
    }

    pub fn max_channels(&self) -> u8 {
        self.max_channels
    }

    /// The bytes to send, one message each.
    pub fn take_writes(&mut self) -> Vec<Vec<u8>> {
        self.writes.drain(..).collect()
    }

    fn send(&mut self, m: Message) {
        self.writes.push_back(m.encode());
    }

    /// A new key (or none): the stick is set up again with it.
    pub fn set_key(&mut self, key: Option<NetworkKey>, now: Duration) -> Vec<Output> {
        self.key = key;
        if self.phase == Phase::Down {
            return Vec::new();
        }
        let mut out = self.drop_channels(FailReason::Connect, false);
        if self.key.is_some() {
            self.link_up(now);
        } else {
            self.phase = Phase::Down;
            out.extend(self.stick_failed(FailReason::NoKey));
        }
        out
    }

    /// The stick was opened: reset it and set it up.
    pub fn link_up(&mut self, now: Duration) {
        self.decoder = Decoder::new();
        self.slots.clear();
        self.ext_flags = 0;
        self.writes.clear();
        self.send(Message::reset());
        self.phase = Phase::Reset { since: now };
    }

    /// Every channel ends (the stick is gone, or reset): connected
    /// sensors are `Disconnected`; with `tell`, every sensor also gets
    /// `Failed { reason }` (once).
    fn drop_channels(&mut self, reason: FailReason, tell: bool) -> Vec<Output> {
        let mut out = Vec::new();
        self.slots.clear();
        for s in &mut self.sensors {
            s.channel = None;
            s.reopen_at = None;
            if let Pages::Fec(_, c) = &mut s.pages {
                c.lost();
            }
            let was = s.connected;
            s.connected = false;
            if tell && s.told != Some(reason) {
                s.told = Some(reason);
                out.push(Output::Event(s.user, DeviceEvent::Failed { reason }));
                out.push(Output::Event(s.user, DeviceEvent::Disconnected));
            } else if was {
                out.push(Output::Event(s.user, DeviceEvent::Disconnected));
            }
        }
        if let Some(scan) = &mut self.scan {
            for r in &mut scan.rounds {
                r.channel = None;
            }
        }
        out
    }

    /// The stick is gone (unplugged, or a read failed).
    pub fn link_down(&mut self, reason: FailReason) -> Vec<Output> {
        self.phase = Phase::Down;
        self.writes.clear();
        self.drop_channels(reason, true)
    }

    /// The stick could not be opened: every sensor is told why (once per
    /// reason).
    pub fn stick_failed(&mut self, reason: FailReason) -> Vec<Output> {
        self.drop_channels(reason, true)
    }

    /// A device handle wants this sensor.
    pub fn add_sensor(&mut self, user: UserId, device: AntDeviceId, now: Duration) -> Vec<Output> {
        let pages = match device.profile {
            AntProfile::HeartRate => Pages::Hr(HrDecoder::new()),
            AntProfile::BikePower => Pages::Power(PowerDecoder::new()),
            AntProfile::FitnessEquipment => Pages::Fec(FecDecoder::new(), FecControl::new()),
        };
        self.sensors.push(Sensor {
            user,
            device,
            channel: None,
            pages,
            connected: false,
            reopen_at: None,
            told: None,
            stale_told: false,
        });
        let mut out = Vec::new();
        if self.key.is_none() {
            out.extend(self.stick_failed(FailReason::NoKey));
        }
        out.extend(self.assign(now));
        out
    }

    /// A command for a sensor's handle: an FE-C trainer takes SIM / ERG
    /// (pages 49 – 51, 55); other sensors ignore it.
    pub fn command(&mut self, user: UserId, cmd: DeviceCommand, now: Duration) -> Vec<Output> {
        if let Some(s) = self.sensors.iter_mut().find(|s| s.user == user)
            && let Pages::Fec(_, c) = &mut s.pages
        {
            c.on_command(cmd);
            self.pump_control(now);
        }
        Vec::new()
    }

    /// The control queue of the FE-C trainer of `user` (tests, the log).
    pub fn fec_control(&self, user: UserId) -> Option<&FecControl> {
        self.sensors
            .iter()
            .find(|s| s.user == user)
            .and_then(|s| match &s.pages {
                Pages::Fec(_, c) => Some(c),
                _ => None,
            })
    }

    /// Sends each tracked FE-C trainer's next control page (one in flight
    /// per trainer).
    fn pump_control(&mut self, now: Duration) {
        if self.phase != Phase::Ready {
            return;
        }
        let mut msgs = Vec::new();
        for s in &mut self.sensors {
            let (Pages::Fec(_, c), Some(n), true) = (&mut s.pages, s.channel, s.connected) else {
                continue;
            };
            let tracking = self
                .slots
                .get(usize::from(n))
                .and_then(Option::as_ref)
                .is_some_and(|sl| !sl.retiring && sl.channel.state() == ChannelState::Tracking);
            if tracking && let Some(p) = c.next(now) {
                msgs.push(Message::acknowledged(n, p));
            }
        }
        for m in msgs {
            self.send(m);
        }
    }

    /// The stick's answer to an acknowledged message (an FE-C control
    /// page): transferred, failed, or refused (one already in progress).
    /// `true`: it was one (the channel's own state machine does not see
    /// it: a refused data message is not a failed channel command).
    fn transfer_result(&mut self, inc: &Incoming) -> bool {
        let (ch, ok) = match *inc {
            Incoming::Event {
                channel,
                code: code::EVENT_TRANSFER_TX_COMPLETED,
            } => (channel, true),
            Incoming::Event {
                channel,
                code: code::EVENT_TRANSFER_TX_FAILED,
            } => (channel, false),
            Incoming::Response {
                to: id::ACKNOWLEDGED_DATA,
                code: code::RESPONSE_NO_ERROR,
                ..
            } => return true,
            Incoming::Response {
                channel,
                to: id::ACKNOWLEDGED_DATA,
                code: c,
            } => {
                tracing::debug!(channel, code = c, "the ANT stick refused a control page");
                (channel, false)
            }
            _ => return false,
        };
        if let Some(s) = self.sensors.iter_mut().find(|s| s.channel == Some(ch))
            && let Pages::Fec(_, c) = &mut s.pages
        {
            c.on_result(ok);
        }
        true
    }

    /// The handle is gone: its channel is closed and freed.
    pub fn remove_sensor(&mut self, user: UserId) {
        let Some(i) = self.sensors.iter().position(|s| s.user == user) else {
            return;
        };
        let s = self.sensors.remove(i);
        if let Some(n) = s.channel {
            self.retire(n);
        }
    }

    /// Starts a scan of every profile until `until`.
    pub fn start_scan(&mut self, now: Duration, until: Duration) -> Vec<Output> {
        self.stop_scan();
        let rounds = AntProfile::ALL
            .into_iter()
            .map(|profile| ScanRound {
                profile,
                channel: None,
                // The paired sensors are known already.
                found: self
                    .sensors
                    .iter()
                    .filter(|s| s.device.profile == profile)
                    .map(|s| s.device.channel_id())
                    .take(MAX_LIST - 1)
                    .collect(),
                done: false,
            })
            .collect();
        self.scan = Some(Scan { until, rounds });
        self.assign(now)
    }

    pub fn stop_scan(&mut self) {
        if let Some(scan) = self.scan.take() {
            for n in scan.rounds.iter().filter_map(|r| r.channel) {
                self.retire(n);
            }
        }
    }

    fn retire(&mut self, n: u8) {
        if let Some(Some(slot)) = self.slots.get_mut(usize::from(n)) {
            slot.retiring = true;
            match slot.channel.state() {
                ChannelState::Closed | ChannelState::Failed => slot.channel.unassign(),
                ChannelState::Unassigned => self.slots[usize::from(n)] = None,
                _ => slot.channel.close(),
            }
        }
    }

    fn free_channel(&self) -> Option<u8> {
        (0..self.max_channels).find(|n| self.slots.get(usize::from(*n)).is_none_or(Option::is_none))
    }

    fn put(&mut self, n: u8, slot: Slot) {
        let i = usize::from(n);
        if self.slots.len() <= i {
            self.slots.resize(i + 1, None);
        }
        self.slots[i] = Some(slot);
    }

    /// Gives every sensor (first) and scan round without a channel one.
    fn assign(&mut self, now: Duration) -> Vec<Output> {
        let mut out = Vec::new();
        if self.phase != Phase::Ready {
            return out;
        }
        for i in 0..self.sensors.len() {
            let s = &self.sensors[i];
            if s.channel.is_some() || s.reopen_at.is_some_and(|t| t > now) {
                continue;
            }
            let Some(n) = self.free_channel() else {
                let s = &mut self.sensors[i];
                if s.told != Some(FailReason::Connect) {
                    s.told = Some(FailReason::Connect);
                    tracing::warn!("all {} ANT channels are in use", self.max_channels);
                    out.push(Output::Event(
                        s.user,
                        DeviceEvent::Failed {
                            reason: FailReason::Connect,
                        },
                    ));
                }
                continue;
            };
            let d = self.sensors[i].device;
            let mut channel = Channel::new(
                n,
                ChannelConfig::ant_plus(d.profile, d.device_number, d.trans_type),
            );
            channel.open();
            self.sensors[i].channel = Some(n);
            self.sensors[i].reopen_at = None;
            let user = self.sensors[i].user;
            self.put(
                n,
                Slot {
                    channel,
                    owner: Owner::Sensor(user),
                    retiring: false,
                },
            );
        }
        if let Some(mut scan) = self.scan.take() {
            for r in scan
                .rounds
                .iter_mut()
                .filter(|r| r.channel.is_none() && !r.done)
            {
                let Some(n) = self.free_channel() else {
                    break;
                };
                let mut cfg = ChannelConfig::ant_plus(r.profile, 0, 0);
                cfg.low_priority_timeout = SCAN_SEARCH_TIMEOUT;
                cfg.exclude = r.found.clone();
                let mut channel = Channel::new(n, cfg);
                channel.open();
                r.channel = Some(n);
                self.put(
                    n,
                    Slot {
                        channel,
                        owner: Owner::Scan(r.profile),
                        retiring: false,
                    },
                );
            }
            self.scan = Some(scan);
        }
        self.pump(now);
        out
    }

    /// Sends each channel's next command.
    fn pump(&mut self, now: Duration) {
        let mut msgs = Vec::new();
        for slot in self.slots.iter_mut().flatten() {
            if let Some(m) = slot.channel.next_message(now) {
                msgs.push(m);
            }
        }
        for m in msgs {
            self.send(m);
        }
    }

    /// Bytes from the stick.
    pub fn on_bytes(&mut self, bytes: &[u8], now: Duration) -> Vec<Output> {
        let mut out = Vec::new();
        for m in self.decoder.push(bytes) {
            out.extend(self.on_message(&m, now));
        }
        self.pump(now);
        self.pump_control(now);
        out
    }

    fn on_message(&mut self, m: &Message, now: Duration) -> Vec<Output> {
        let inc = m.parse();
        // The stick's own setup.
        match (&inc, self.phase) {
            (Incoming::StartUp(_), Phase::Reset { .. }) => {
                return self.after_reset(now);
            }
            (Incoming::StartUp(_), Phase::Ready) => {
                // It reset by itself: set it up again, channels and all.
                tracing::warn!("the ANT stick reset itself; setting it up again");
                let mut out = self.drop_channels(FailReason::Connect, false);
                self.phase = Phase::Reset { since: now };
                out.extend(self.after_reset(now));
                return out;
            }
            (
                Incoming::Capabilities {
                    channels, extended, ..
                },
                Phase::Capabilities { .. },
            ) => {
                self.max_channels = (*channels).clamp(1, 15);
                tracing::info!(
                    channels = self.max_channels,
                    extended = ?extended,
                    "ANT stick capabilities"
                );
                return self.send_key(now);
            }
            (
                Incoming::Response {
                    to: id::REQUEST, ..
                },
                Phase::Capabilities { .. },
            ) => return self.send_key(now),
            (
                Incoming::Response {
                    to: id::SET_NETWORK_KEY,
                    code: c,
                    ..
                },
                Phase::Key { .. },
            ) => {
                if *c == code::RESPONSE_NO_ERROR {
                    return self.send_lib_config(lib_config::CHANNEL_ID | lib_config::RSSI, now);
                }
                tracing::warn!(code = c, "the ANT stick refused the network key");
                self.phase = Phase::Refused;
                return self.stick_failed(FailReason::Connect);
            }
            (
                Incoming::Response {
                    to: id::LIB_CONFIG,
                    code: c,
                    ..
                },
                Phase::LibConfig { flags, .. },
            ) => {
                if *c == code::RESPONSE_NO_ERROR {
                    self.ext_flags = flags;
                    return self.ready(now);
                }
                if flags != lib_config::CHANNEL_ID {
                    // Without the signal, then.
                    return self.send_lib_config(lib_config::CHANNEL_ID, now);
                }
                // No extended data: channel IDs are requested.
                self.ext_flags = 0;
                return self.ready(now);
            }
            (Incoming::SerialError, _) => {
                tracing::warn!("the ANT stick could not read a message");
                return Vec::new();
            }
            (_, Phase::Ready) => {}
            _ => return Vec::new(),
        }
        if self.transfer_result(&inc) {
            return Vec::new();
        }
        let Some(n) = m.channel() else {
            return Vec::new();
        };
        let Some(Some(slot)) = self.slots.get_mut(usize::from(n)) else {
            return Vec::new();
        };
        let events = slot.channel.on_incoming(&inc, now);
        let mut out = Vec::new();
        for e in events {
            out.extend(self.on_channel_event(n, e, now));
        }
        out
    }

    fn after_reset(&mut self, now: Duration) -> Vec<Output> {
        self.send(Message::request(0, id::CAPABILITIES));
        self.phase = Phase::Capabilities { since: now };
        Vec::new()
    }

    fn send_key(&mut self, now: Duration) -> Vec<Output> {
        let Some(key) = self.key else {
            self.phase = Phase::Down;
            return self.stick_failed(FailReason::NoKey);
        };
        self.send(Message::set_network_key(0, &key));
        self.phase = Phase::Key { since: now };
        Vec::new()
    }

    fn send_lib_config(&mut self, flags: u8, now: Duration) -> Vec<Output> {
        self.send(Message::lib_config(flags));
        self.phase = Phase::LibConfig { flags, since: now };
        Vec::new()
    }

    fn ready(&mut self, now: Duration) -> Vec<Output> {
        tracing::info!(
            channels = self.max_channels,
            extended = self.ext_flags,
            "ANT stick ready"
        );
        self.phase = Phase::Ready;
        for s in &mut self.sensors {
            s.told = None;
        }
        self.assign(now)
    }

    fn on_channel_event(&mut self, n: u8, e: ChannelEvent, now: Duration) -> Vec<Output> {
        let i = usize::from(n);
        let Some(slot) = self.slots[i].as_mut() else {
            return Vec::new();
        };
        let (owner, retiring) = (slot.owner, slot.retiring);
        // A channel on its way out: only closing and freeing it matter.
        if retiring {
            match e {
                ChannelEvent::Closed | ChannelEvent::Failed { .. } => slot.channel.unassign(),
                ChannelEvent::Unassigned => self.slots[i] = None,
                _ => {}
            }
            return Vec::new();
        }
        if e == ChannelEvent::Unassigned {
            self.slots[i] = None;
            for s in &mut self.sensors {
                if s.channel == Some(n) {
                    s.channel = None;
                }
            }
            return self.assign(now);
        }
        match owner {
            Owner::Sensor(user) => self.sensor_event(user, n, e, now),
            Owner::Scan(profile) => self.scan_event(profile, n, e, now),
        }
    }

    fn sensor_event(&mut self, user: UserId, n: u8, e: ChannelEvent, now: Duration) -> Vec<Output> {
        let mut out = Vec::new();
        let Some(s) = self.sensors.iter_mut().find(|s| s.user == user) else {
            return out;
        };
        let ev = |e| Output::Event(user, e);
        match e {
            ChannelEvent::Data { payload, .. } => {
                if !s.connected {
                    s.connected = true;
                    s.told = None;
                    // A trainer found (again): the latest SIM / ERG target.
                    if let Pages::Fec(_, c) = &mut s.pages {
                        c.reconnected();
                    }
                    out.push(ev(DeviceEvent::Connected {
                        kind: s.device.profile.kind(),
                    }));
                }
                match &mut s.pages {
                    Pages::Hr(d) => {
                        let r = d.on_page(&payload, now);
                        match r.bpm {
                            Some(bpm) => {
                                s.stale_told = false;
                                out.push(ev(DeviceEvent::HeartRate {
                                    bpm: f64::from(bpm),
                                    timestamp: now,
                                }));
                            }
                            // Stale (or 0): no heart rate, once.
                            None if !s.stale_told => {
                                s.stale_told = true;
                                out.push(ev(DeviceEvent::HeartRate {
                                    bpm: 0.0,
                                    timestamp: now,
                                }));
                            }
                            None => {}
                        }
                    }
                    Pages::Power(d) => {
                        if let Some(r) = d.on_page(&payload, now) {
                            out.push(ev(DeviceEvent::Power {
                                watts: r.watts,
                                cadence_rpm: r.cadence_rpm,
                                timestamp: now,
                            }));
                        }
                    }
                    Pages::Fec(d, c) => {
                        let before = d.status;
                        if let Some(r) = d.on_page(&payload, now) {
                            out.push(ev(DeviceEvent::Power {
                                watts: r.watts,
                                cadence_rpm: r.cadence_rpm,
                                timestamp: now,
                            }));
                        }
                        let st = d.status;
                        if st.user_config_required {
                            c.user_config_required(now);
                        }
                        if (st.power_calibration_required || st.resistance_calibration_required)
                            && st != before
                        {
                            tracing::info!(
                                device = %s.device,
                                power = st.power_calibration_required,
                                resistance = st.resistance_calibration_required,
                                "the FE-C trainer asks for a calibration"
                            );
                        }
                    }
                }
            }
            ChannelEvent::Lost => {
                if let Pages::Fec(_, c) = &mut s.pages {
                    c.lost();
                }
                if s.connected {
                    s.connected = false;
                    out.push(ev(DeviceEvent::Disconnected));
                }
            }
            ChannelEvent::Closed => {
                if let Pages::Fec(_, c) = &mut s.pages {
                    c.lost();
                }
                // The search timed out: search again after a pause.
                if s.connected {
                    s.connected = false;
                    out.push(ev(DeviceEvent::Disconnected));
                } else {
                    out.push(ev(DeviceEvent::Failed {
                        reason: FailReason::NotFound,
                    }));
                    out.push(ev(DeviceEvent::Disconnected));
                }
                s.reopen_at = Some(now + RECONNECT_DELAY);
            }
            ChannelEvent::Failed { command, code } => {
                tracing::warn!(channel = n, command, code, "ANT channel command failed");
                s.connected = false;
                s.reopen_at = Some(now + RECONNECT_DELAY);
                out.push(ev(DeviceEvent::Failed {
                    reason: FailReason::Connect,
                }));
                out.push(ev(DeviceEvent::Disconnected));
                // Free the number; a new channel follows.
                s.channel = None;
                if let Some(slot) = self.slots[usize::from(n)].as_mut() {
                    slot.retiring = true;
                    slot.channel.unassign();
                }
            }
            ChannelEvent::Searching
            | ChannelEvent::Found(_)
            | ChannelEvent::Missed
            | ChannelEvent::Unassigned => {}
        }
        out
    }

    fn scan_event(
        &mut self,
        profile: AntProfile,
        n: u8,
        e: ChannelEvent,
        _now: Duration,
    ) -> Vec<Output> {
        let mut out = Vec::new();
        let i = usize::from(n);
        let found_id = self.slots[i].as_ref().and_then(|s| s.channel.found());
        let Some(scan) = self.scan.as_mut() else {
            return out;
        };
        let Some(round) = scan.rounds.iter_mut().find(|r| r.profile == profile) else {
            return out;
        };
        let slot = self.slots[i].as_mut().expect("checked");
        match e {
            ChannelEvent::Data { rssi, .. } => {
                if let Some(id) = found_id
                    && !round
                        .found
                        .iter()
                        .any(|f| f.device_number == id.device_number)
                {
                    round.found.push(id);
                    if let Some(d) = AntDeviceId::of(id) {
                        out.push(Output::Found(d.found(rssi)));
                    }
                    if round.found.len() >= MAX_LIST {
                        round.done = true;
                    }
                    // The next round excludes it.
                    slot.channel.close();
                }
            }
            ChannelEvent::Closed => {
                if round.done {
                    slot.retiring = true;
                    slot.channel.unassign();
                    round.channel = None;
                } else {
                    slot.channel.reopen(Some(round.found.clone()));
                }
            }
            ChannelEvent::Failed { .. } => {
                round.done = true;
                round.channel = None;
                slot.retiring = true;
                slot.channel.unassign();
            }
            _ => {}
        }
        out
    }

    /// Time passes: setup timeouts, reopening closed channels, the scan's
    /// end, the commands' timeouts.
    pub fn on_tick(&mut self, now: Duration) -> Vec<Output> {
        let mut out = Vec::new();
        match self.phase {
            Phase::Reset { since } if now.saturating_sub(since) > RESET_WAIT => {
                out.extend(self.after_reset(now));
            }
            Phase::Capabilities { since } if now.saturating_sub(since) > CAPABILITIES_WAIT => {
                self.max_channels = DEFAULT_CHANNELS;
                out.extend(self.send_key(now));
            }
            Phase::Key { since } if now.saturating_sub(since) > SETUP_WAIT => {
                tracing::warn!("the ANT stick did not answer the network key");
                self.phase = Phase::Refused;
                out.extend(self.stick_failed(FailReason::Connect));
            }
            Phase::LibConfig { since, .. } if now.saturating_sub(since) > SETUP_WAIT => {
                self.ext_flags = 0;
                out.extend(self.ready(now));
            }
            _ => {}
        }
        if self.phase != Phase::Ready {
            return out;
        }
        if self.scan.as_ref().is_some_and(|s| now >= s.until) {
            self.stop_scan();
        }
        // Commands without an answer.
        let mut events = Vec::new();
        for (n, slot) in self.slots.iter_mut().enumerate() {
            if let Some(slot) = slot {
                for e in slot.channel.on_tick(now) {
                    events.push((n as u8, e));
                }
            }
        }
        for (n, e) in events {
            out.extend(self.on_channel_event(n, e, now));
        }
        // Closed sensor channels search again; staleness without pages.
        let mut reopen = Vec::new();
        for s in &mut self.sensors {
            if s.reopen_at.is_some_and(|t| now >= t) {
                s.reopen_at = None;
                if let Some(n) = s.channel {
                    reopen.push(n);
                }
            }
            if let Pages::Hr(d) = &s.pages
                && s.connected
                && d.stale(now)
                && !s.stale_told
            {
                s.stale_told = true;
                out.push(Output::Event(
                    s.user,
                    DeviceEvent::HeartRate {
                        bpm: 0.0,
                        timestamp: now,
                    },
                ));
            }
        }
        for n in reopen {
            if let Some(Some(slot)) = self.slots.get_mut(usize::from(n))
                && slot.channel.state() == ChannelState::Closed
            {
                slot.channel.reopen(None);
            }
        }
        out.extend(self.assign(now));
        self.pump_control(now);
        out
    }
}

/// The channel response the stick gives for `m` (the simulated stick and
/// tests).
pub fn response(m: &Message, c: u8) -> Message {
    Message::new(
        codec::id::CHANNEL_EVENT,
        [m.data.first().copied().unwrap_or(0), m.id, c],
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ant::sim::{SimKind, SimLoad, SimRadio, SimSensor};

    const STEP: Duration = Duration::from_millis(20);

    /// The core and a simulated stick wired together, on a test clock.
    struct Bench {
        core: StickCore,
        radio: SimRadio,
        now: Duration,
        out: Vec<Output>,
    }

    impl Bench {
        fn new(key: Option<NetworkKey>, radio: SimRadio) -> Self {
            let mut b = Self {
                core: StickCore::new(key),
                radio,
                now: Duration::ZERO,
                out: Vec::new(),
            };
            b.core.link_up(b.now);
            b
        }

        /// Runs for `d` in steps.
        fn run(&mut self, d: Duration) {
            let end = self.now + d;
            while self.now < end {
                self.now += STEP;
                for w in self.core.take_writes() {
                    self.radio.host(&w, self.now);
                }
                self.radio.step(self.now);
                let bytes = self.radio.take_output();
                let o = self.core.on_bytes(&bytes, self.now);
                self.out.extend(o);
                let o = self.core.on_tick(self.now);
                self.out.extend(o);
            }
        }

        fn events(&mut self, user: UserId) -> Vec<DeviceEvent> {
            let mut got = Vec::new();
            self.out.retain(|o| match o {
                Output::Event(u, e) if *u == user => {
                    got.push(*e);
                    false
                }
                _ => true,
            });
            got
        }

        fn found(&mut self) -> Vec<FoundDevice> {
            let mut got = Vec::new();
            self.out.retain(|o| match o {
                Output::Found(d) => {
                    got.push(d.clone());
                    false
                }
                _ => true,
            });
            got
        }
    }

    fn strap(n: u16, bpm: f64) -> SimSensor {
        SimSensor::new(ChannelId::new(n, 120, 1), SimKind::HeartRate { bpm })
    }

    fn hr(e: &[DeviceEvent]) -> Vec<f64> {
        e.iter()
            .filter_map(|e| match e {
                DeviceEvent::HeartRate { bpm, .. } => Some(*bpm),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn set_up_then_a_strap_connects_and_sends_its_heart_rate() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(strap(12345, 142.0));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(200));
        assert!(b.core.is_ready());
        assert_eq!(
            b.core.extended_flags(),
            lib_config::CHANNEL_ID | lib_config::RSSI
        );
        let dev = AntDeviceId::new(AntProfile::HeartRate, 12345, 1);
        let o = b.core.add_sensor(1, dev, b.now);
        assert!(o.is_empty());
        b.run(Duration::from_secs(3));
        let e = b.events(1);
        assert_eq!(
            e[0],
            DeviceEvent::Connected {
                kind: "heart-rate sensor (ANT+)"
            }
        );
        let bpm = hr(&e);
        assert!(bpm.len() >= 8, "about 4 a second: {e:?}");
        assert!(bpm.iter().all(|b| *b == 142.0), "{bpm:?}");
    }

    #[test]
    fn a_wrong_key_hears_nothing_and_no_key_says_so() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(strap(1, 80.0));
        let mut b = Bench::new(Some(NetworkKey::new([9; 8])), radio);
        b.run(Duration::from_millis(200));
        b.core
            .add_sensor(1, AntDeviceId::new(AntProfile::HeartRate, 1, 1), b.now);
        // The search times out (25 s): not found, trying again.
        b.run(Duration::from_secs(27));
        let e = b.events(1);
        assert!(hr(&e).is_empty());
        assert_eq!(
            e[..2],
            [
                DeviceEvent::Failed {
                    reason: FailReason::NotFound
                },
                DeviceEvent::Disconnected
            ]
        );
        // No key at all.
        let mut core = StickCore::new(None);
        let o = core.add_sensor(
            7,
            AntDeviceId::new(AntProfile::HeartRate, 1, 1),
            Duration::ZERO,
        );
        assert_eq!(
            o[0],
            Output::Event(
                7,
                DeviceEvent::Failed {
                    reason: FailReason::NoKey
                }
            )
        );
    }

    #[test]
    fn two_straps_on_two_channels_and_a_lost_one_comes_back() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(strap(111, 120.0));
        radio.add(strap(222, 150.0));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(200));
        b.core
            .add_sensor(1, AntDeviceId::new(AntProfile::HeartRate, 111, 1), b.now);
        b.core
            .add_sensor(2, AntDeviceId::new(AntProfile::HeartRate, 222, 1), b.now);
        b.run(Duration::from_secs(2));
        assert_eq!(b.core.channels_in_use(), 2);
        assert!(hr(&b.events(1)).iter().all(|x| *x == 120.0));
        assert!(hr(&b.events(2)).iter().all(|x| *x == 150.0));
        // Strap 222 is taken off: missed messages, then lost.
        b.radio.set_present(222, false);
        b.run(Duration::from_secs(3));
        let e = b.events(2);
        assert!(e.contains(&DeviceEvent::Disconnected), "{e:?}");
        assert!(!hr(&b.events(1)).is_empty(), "the other strap goes on");
        // Back on: connected again.
        b.radio.set_present(222, true);
        b.run(Duration::from_secs(2));
        let e = b.events(2);
        assert!(matches!(e[0], DeviceEvent::Connected { .. }), "{e:?}");
        assert!(hr(&e).iter().all(|x| *x == 150.0));
        // Removing a sensor frees its channel.
        b.core.remove_sensor(2);
        b.run(Duration::from_millis(300));
        assert_eq!(b.core.channels_in_use(), 1);
    }

    #[test]
    fn an_unplugged_stick_ends_every_channel_and_all_come_back() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(strap(5, 100.0));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(200));
        b.core
            .add_sensor(1, AntDeviceId::new(AntProfile::HeartRate, 5, 1), b.now);
        b.run(Duration::from_secs(1));
        b.events(1);
        let o = b.core.link_down(FailReason::NoStick);
        assert_eq!(
            o,
            [
                Output::Event(
                    1,
                    DeviceEvent::Failed {
                        reason: FailReason::NoStick
                    }
                ),
                Output::Event(1, DeviceEvent::Disconnected)
            ]
        );
        // Opening fails again: told once only.
        assert!(b.core.stick_failed(FailReason::NoStick).is_empty());
        // Plugged in again (a fresh stick: reset).
        b.radio = {
            let mut r = SimRadio::new(Some(NetworkKey::TEST));
            r.add(strap(5, 100.0));
            r
        };
        b.core.link_up(b.now);
        b.run(Duration::from_secs(2));
        let e = b.events(1);
        assert!(matches!(e[0], DeviceEvent::Connected { .. }), "{e:?}");
        assert!(!hr(&e).is_empty());
    }

    #[test]
    fn a_scan_finds_three_straps_one_after_the_other_and_a_power_meter() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        for (n, bpm) in [(301, 90.0), (302, 95.0), (303, 99.0)] {
            radio.add(strap(n, bpm));
        }
        radio.add(SimSensor::new(
            ChannelId::new(4004, 11, 5),
            SimKind::Power {
                watts: 230.0,
                cadence: 88.0,
            },
        ));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(200));
        let until = b.now + Duration::from_secs(10);
        b.core.start_scan(b.now, until);
        b.run(Duration::from_secs(10));
        let mut ids: Vec<String> = b.found().into_iter().map(|d| d.id).collect();
        ids.sort();
        assert_eq!(
            ids,
            [
                "ant:11:4004:5",
                "ant:120:301:1",
                "ant:120:302:1",
                "ant:120:303:1"
            ]
        );
        // The scan ended: its channels are free.
        b.run(Duration::from_millis(500));
        assert!(!b.core.scanning());
        assert_eq!(b.core.channels_in_use(), 0);
    }

    #[test]
    fn a_power_meter_gives_power() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(SimSensor::new(
            ChannelId::new(4004, 11, 5),
            SimKind::Power {
                watts: 230.0,
                cadence: 88.0,
            },
        ));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(200));
        b.core
            .add_sensor(9, AntDeviceId::new(AntProfile::BikePower, 4004, 5), b.now);
        b.run(Duration::from_secs(2));
        let e = b.events(9);
        assert_eq!(
            e[0],
            DeviceEvent::Connected {
                kind: "power meter (ANT+)"
            }
        );
        let watts: Vec<f64> = e
            .iter()
            .filter_map(|e| match e {
                DeviceEvent::Power { watts, .. } => Some(*watts),
                _ => None,
            })
            .collect();
        assert!(watts.len() >= 4 && watts.iter().all(|w| (*w - 230.0).abs() < 1.0));
    }

    #[test]
    fn a_stick_without_extended_data_or_start_up_message_works_too() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.extended = false;
        radio.startup_message = false;
        radio.add(strap(808, 77.0));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(1500));
        assert!(b.core.is_ready());
        assert_eq!(b.core.extended_flags(), 0);
        let until = b.now + Duration::from_secs(4);
        b.core.start_scan(b.now, until);
        b.run(Duration::from_secs(4));
        let f = b.found();
        assert_eq!(f[0].id, "ant:120:808:1", "the ID was requested");
        assert_eq!(f[0].rssi, None);
    }

    #[test]
    fn a_key_change_sets_the_stick_up_again() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(strap(42, 64.0));
        let mut b = Bench::new(Some(NetworkKey::new([1; 8])), radio);
        b.run(Duration::from_millis(200));
        b.core
            .add_sensor(1, AntDeviceId::new(AntProfile::HeartRate, 42, 1), b.now);
        b.run(Duration::from_secs(2));
        assert!(hr(&b.events(1)).is_empty(), "the wrong key hears nothing");
        let now = b.now;
        b.core.set_key(Some(NetworkKey::TEST), now);
        b.run(Duration::from_secs(2));
        assert!(hr(&b.events(1)).contains(&64.0));
    }

    #[test]
    fn a_strap_lying_still_goes_stale() {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(strap(31, 70.0));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(200));
        b.core
            .add_sensor(1, AntDeviceId::new(AntProfile::HeartRate, 31, 1), b.now);
        b.run(Duration::from_secs(2));
        b.events(1);
        // Still sending, but no new beats.
        b.radio.set_beating(31, false);
        b.run(Duration::from_secs(7));
        let bpm = hr(&b.events(1));
        assert_eq!(bpm.last(), Some(&0.0), "{bpm:?}");
        assert_eq!(bpm.iter().filter(|b| **b == 0.0).count(), 1, "once");
    }

    fn watts(e: &[DeviceEvent]) -> Vec<f64> {
        e.iter()
            .filter_map(|e| match e {
                DeviceEvent::Power { watts, .. } => Some(*watts),
                _ => None,
            })
            .collect()
    }

    fn sim_cmd(grade: f64, mass_kg: f64) -> DeviceCommand {
        DeviceCommand::SetSimulation(crate::SimulationParameters {
            grade,
            wind_speed_mps: 0.0,
            crr: 0.004,
            cw_kg_per_m: 0.2,
            mass_kg,
            wind_grade: grade,
        })
    }

    const FEC: u16 = 5005;

    fn trainer_bench() -> Bench {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        radio.add(SimSensor::fe_c_trainer(FEC, 180.0, 90.0));
        let mut b = Bench::new(Some(NetworkKey::TEST), radio);
        b.run(Duration::from_millis(200));
        b.core.add_sensor(
            3,
            AntDeviceId::new(AntProfile::FitnessEquipment, FEC, 5),
            b.now,
        );
        b.run(Duration::from_secs(2));
        b
    }

    #[test]
    fn a_ride_on_an_fe_c_trainer_in_erg_and_sim() {
        use crate::ant::fec;
        let mut b = trainer_bench();
        let e = b.events(3);
        assert_eq!(
            e[0],
            DeviceEvent::Connected {
                kind: "smart trainer (ANT+ FE-C)"
            }
        );
        // Before any target: the rider's own 180 W, with the cadence.
        let w = watts(&e);
        assert!(w.len() >= 4, "page 25 about 2.7 a second: {e:?}");
        assert!(w.iter().all(|w| (*w - 180.0).abs() < 1.0), "{w:?}");
        assert!(e.iter().any(|e| matches!(
            e,
            DeviceEvent::Power {
                cadence_rpm: Some(c),
                ..
            } if *c == 90.0
        )));
        // The trainer asked for the user configuration (page 25's bit):
        // the defaults went, and it stopped asking.
        let tr = b.radio.trainer(FEC).unwrap();
        assert_eq!(tr.user_kg, Some(fec::DEFAULT_USER_KG));
        // ERG 250 W: the power follows the target.
        b.core
            .command(3, DeviceCommand::SetTargetPower(250.0), b.now);
        b.run(Duration::from_secs(6));
        let w = watts(&b.events(3));
        assert!((w.last().unwrap() - 250.0).abs() < 2.0, "{w:?}");
        assert_eq!(b.radio.trainer(FEC).unwrap().load, SimLoad::Erg(250.0));
        // SIM: 6 % for a rider of 80 kg – the user configuration, the wind
        // and the track pages, then the power the climb costs.
        b.core.command(3, sim_cmd(0.06, 80.0), b.now);
        b.run(Duration::from_secs(8));
        let tr = b.radio.trainer(FEC).unwrap();
        assert_eq!(
            tr.load,
            SimLoad::Sim {
                grade_pct: 6.0,
                crr: 0.004
            }
        );
        assert_eq!(tr.user_kg, Some(80.0 - fec::BIKE_KG));
        assert_eq!(tr.wind_coefficient, 0.4, "2 × cw");
        let v = tr.pace(180.0);
        let climb = tr.load_power(180.0, v);
        assert!(climb > 400.0, "{climb}");
        let w = watts(&b.events(3));
        assert!(
            (w.last().unwrap() - climb).abs() < climb * 0.02,
            "{w:?} {climb}"
        );
        // Downhill −2 %: no power needed.
        b.core.command(3, sim_cmd(-0.02, 80.0), b.now);
        b.run(Duration::from_secs(8));
        let w = watts(&b.events(3));
        assert!(*w.last().unwrap() < 30.0, "{w:?}");
        // The whole ride: one page at a time, never refused.
        assert_eq!(b.radio.busy_refusals, 0);
        assert_eq!(b.core.fec_control(3).unwrap().given_up, 0);
    }

    #[test]
    fn failed_transfers_are_retried_and_the_newest_target_wins() {
        use crate::ant::fec;
        let mut b = trainer_bench();
        b.events(3);
        // The next two transfers fail: the page is sent again, the third
        // attempt goes through.
        b.radio.trainer_mut(FEC).unwrap().fail_acks = 2;
        let before = b.core.fec_control(3).unwrap().sent;
        b.core
            .command(3, DeviceCommand::SetTargetPower(210.0), b.now);
        b.run(Duration::from_secs(3));
        assert_eq!(b.core.fec_control(3).unwrap().sent - before, 3);
        let tr = b.radio.trainer(FEC).unwrap();
        assert_eq!(tr.load, SimLoad::Erg(210.0));
        let erg: Vec<_> = tr
            .received
            .iter()
            .filter(|p| p[0] == fec::PAGE_TARGET_POWER)
            .collect();
        assert_eq!(erg, [&fec::target_power(210.0)]);
        // Three targets at once: the first goes, the newest replaces the
        // one in between.
        for w in [220.0, 230.0, 240.0] {
            b.core.command(3, DeviceCommand::SetTargetPower(w), b.now);
        }
        b.run(Duration::from_secs(3));
        let tr = b.radio.trainer(FEC).unwrap();
        let erg: Vec<_> = tr
            .received
            .iter()
            .filter(|p| p[0] == fec::PAGE_TARGET_POWER)
            .copied()
            .collect();
        assert_eq!(
            erg,
            [
                fec::target_power(210.0),
                fec::target_power(220.0),
                fec::target_power(240.0)
            ]
        );
        assert_eq!(b.radio.busy_refusals, 0);
        // Out of range: the page fails, the trainer is lost; back in range
        // it gets the latest target again.
        b.radio.set_present(FEC, false);
        b.core
            .command(3, DeviceCommand::SetTargetPower(300.0), b.now);
        b.run(Duration::from_secs(3));
        assert!(b.events(3).contains(&DeviceEvent::Disconnected));
        b.radio.set_present(FEC, true);
        b.run(Duration::from_secs(3));
        assert!(matches!(b.events(3)[0], DeviceEvent::Connected { .. }));
        assert_eq!(b.radio.trainer(FEC).unwrap().load, SimLoad::Erg(300.0));
    }
}

//! Both sides' relay logic, pure and tested natively (spec 14 Phase 10,
//! 10D.2 *Built (local bridge)*; 11.4):
//!
//! - [`Gate`] (desktop): **one browser at a time** – a hello with the right
//!   proof pairs; the same page reconnecting replaces its old connection;
//!   another page is refused while the paired one is alive.
//! - [`Relay`] (desktop): numbers the device readings and stamps them with
//!   the desktop's time.
//! - [`Session`] (browser): the hello, the clock (offset and round trip from
//!   pings), the readings in order (duplicates and late ones after a
//!   reconnect dropped, a new desktop run starts the count again) with
//!   timestamps on the browser's clock that never go back, the trainer
//!   control (sent when it changes, the round trip measured by `Applied`),
//!   and the reconnect delays ([`Backoff`]).

use ocw_devices::{DeviceCommand, DeviceEvent};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::time::Duration;

use crate::msg::{Control, DeviceState, ToBrowser, ToHost, VERSION};

/// The proof of the code in the browser's hello: a hash of the secret (sent
/// inside TLS to the certificate only the code's holder can derive).
pub fn proof(secret: u32) -> String {
    let h = Sha256::new()
        .chain_update(b"ocw-bridge-proof-v1")
        .chain_update(secret.to_le_bytes())
        .finalize();
    crate::cert::hex(&h[..16])
}

/// A paired browser is alive while it was heard within this (it pings once
/// a second).
pub const ALIVE_MS: f64 = 5_000.0;

/// The desktop's pairing rule.
#[derive(Debug, Default)]
pub struct Gate {
    secret: u32,
    /// The paired page: its session, connection and when it was heard.
    paired: Option<Paired>,
    next_conn: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Paired {
    pub session: String,
    pub client: String,
    pub conn: u64,
    pub heard_ms: f64,
}

/// What a hello leads to.
#[derive(Debug, Clone, PartialEq)]
pub enum Admit {
    /// Paired as connection `conn`; `replaces`: the same page's older
    /// connection, to close.
    Paired { conn: u64, replaces: Option<u64> },
    Refused(String),
}

impl Gate {
    pub fn new(secret: u32) -> Self {
        Self {
            secret,
            paired: None,
            next_conn: 1,
        }
    }

    pub fn paired(&self) -> Option<&Paired> {
        self.paired.as_ref()
    }

    /// A hello at `now_ms`.
    pub fn hello(&mut self, hello: &ToHost, now_ms: f64) -> Admit {
        let ToHost::Hello {
            v,
            proof: p,
            session,
            client,
        } = hello
        else {
            return Admit::Refused("the first message must be the hello".into());
        };
        if *v != VERSION {
            return Admit::Refused(format!(
                "this page speaks bridge version {v}, the desktop app {VERSION}: update the older one"
            ));
        }
        if *p != proof(self.secret) {
            return Admit::Refused(
                "wrong pairing code: type the code the desktop app shows now".into(),
            );
        }
        let replaces = match &self.paired {
            Some(old) if old.session == *session => Some(old.conn),
            Some(old) if now_ms - old.heard_ms < ALIVE_MS => {
                return Admit::Refused(format!(
                    "another browser ({}) uses this trainer – stop it there first",
                    old.client
                ));
            }
            // A page that went quiet: the new one takes over.
            Some(old) => Some(old.conn),
            None => None,
        };
        let conn = self.next_conn;
        self.next_conn += 1;
        self.paired = Some(Paired {
            session: session.clone(),
            client: client.clone(),
            conn,
            heard_ms: now_ms,
        });
        Admit::Paired { conn, replaces }
    }

    /// Connection `conn` was heard.
    pub fn heard(&mut self, conn: u64, now_ms: f64) {
        if let Some(p) = &mut self.paired
            && p.conn == conn
        {
            p.heard_ms = now_ms;
        }
    }

    /// Connection `conn` ended; true if it was the paired one.
    pub fn ended(&mut self, conn: u64) -> bool {
        if self.paired.as_ref().is_some_and(|p| p.conn == conn) {
            self.paired = None;
            true
        } else {
            false
        }
    }

    /// Whether `conn` is the paired connection.
    pub fn is_paired(&self, conn: u64) -> bool {
        self.paired.as_ref().is_some_and(|p| p.conn == conn)
    }
}

/// The desktop's numbering of readings.
#[derive(Debug, Default)]
pub struct Relay {
    seq: u64,
}

impl Relay {
    /// The message for a device event at the desktop's time `h_ms`;
    /// `virt`: from the virtual trainer.
    pub fn message(&mut self, name: &str, event: &DeviceEvent, h_ms: f64, virt: bool) -> Option<ToBrowser> {
        Some(match *event {
            DeviceEvent::Power {
                watts, cadence_rpm, ..
            } => {
                self.seq += 1;
                ToBrowser::Power {
                    seq: self.seq,
                    h: h_ms,
                    w: watts,
                    rpm: cadence_rpm,
                    virt,
                }
            }
            DeviceEvent::HeartRate { bpm, .. } => {
                self.seq += 1;
                ToBrowser::HeartRate {
                    seq: self.seq,
                    h: h_ms,
                    bpm,
                }
            }
            DeviceEvent::Connected { .. } => ToBrowser::Device {
                name: name.into(),
                state: DeviceState::Connected,
            },
            DeviceEvent::Disconnected => ToBrowser::Device {
                name: name.into(),
                state: DeviceState::Reconnecting,
            },
            DeviceEvent::Failed { .. } => ToBrowser::Device {
                name: name.into(),
                state: DeviceState::Failed,
            },
        })
    }
}

/// The reconnect delays: 0.5, 1, 2, 4 s, then every 5 s; reset once
/// paired.
#[derive(Debug, Clone, Default)]
pub struct Backoff {
    tries: u32,
}

impl Backoff {
    /// The delay before the next attempt (ms).
    pub fn next_ms(&mut self) -> f64 {
        let ms = match self.tries {
            0 => 500.0,
            1 => 1_000.0,
            2 => 2_000.0,
            3 => 4_000.0,
            _ => 5_000.0,
        };
        self.tries += 1;
        ms
    }

    pub fn reset(&mut self) {
        self.tries = 0;
    }
}

/// The kind the browser's device list shows for the bridge.
pub const KIND: &str = "trainer of the desktop app (bridge)";

/// Pings kept for the clock estimate.
const PINGS_KEPT: usize = 16;

/// Where the browser's session stands.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    /// Connecting (or waiting to reconnect).
    Connecting,
    Paired { host: String },
    /// The desktop refused; no more attempts (a wrong code, another
    /// browser).
    Refused(String),
}

/// The browser's side of the bridge.
#[derive(Debug)]
pub struct Session {
    secret: u32,
    /// This page's session id (kept across reconnects).
    id: String,
    client: String,
    pub phase: Phase,
    epoch: Option<u64>,
    last_seq: u64,
    /// The browser's time when the backend started (ms); event timestamps
    /// are relative to it.
    start_ms: f64,
    /// The last event timestamp given (ms since start): never goes back.
    last_ts_ms: f64,
    /// (round trip, offset = desktop − browser) of recent pings.
    pings: VecDeque<(f64, f64)>,
    next_ping: u64,
    /// The last control sent, and the one waiting for `Applied`.
    sent: Option<Control>,
    pending: Option<(u64, f64)>,
    next_control: u64,
    /// Round trip of the last applied control (ms).
    pub control_rtt_ms: Option<f64>,
    /// Age of the last reading on arrival (ms, the desktop's reading time
    /// on the browser's clock to its arrival).
    pub sample_age_ms: Option<f64>,
    /// Readings taken / dropped (duplicates, out of order).
    pub readings: u64,
    pub dropped: u64,
    pub controls_applied: u64,
    pub backoff: Backoff,
    /// The desktop's devices as it last said.
    pub devices: Vec<(String, DeviceState)>,
}

impl Session {
    pub fn new(secret: u32, id: String, client: String, start_ms: f64) -> Self {
        Self {
            secret,
            id,
            client,
            phase: Phase::Connecting,
            epoch: None,
            last_seq: 0,
            start_ms,
            last_ts_ms: 0.0,
            pings: VecDeque::new(),
            next_ping: 1,
            sent: None,
            pending: None,
            next_control: 1,
            control_rtt_ms: None,
            sample_age_ms: None,
            readings: 0,
            dropped: 0,
            controls_applied: 0,
            backoff: Backoff::default(),
            devices: Vec::new(),
        }
    }

    /// The first message of every connection.
    pub fn hello(&self) -> ToHost {
        ToHost::Hello {
            v: VERSION,
            proof: proof(self.secret),
            session: self.id.clone(),
            client: self.client.clone(),
        }
    }

    /// A connection opened: the hello, and the control to apply again (a
    /// trainer that lost the desktop keeps its last load; resending is
    /// harmless).
    pub fn on_open(&mut self, now_ms: f64) -> Vec<ToHost> {
        let mut out = vec![self.hello()];
        if let Some(c) = self.sent {
            out.push(self.control_msg(c, now_ms));
        }
        out
    }

    /// A connection ended: events for the device list (the ride gets no
    /// power until it is back, as with a Bluetooth trainer).
    pub fn on_close(&mut self) -> Vec<DeviceEvent> {
        self.pending = None;
        let was = matches!(self.phase, Phase::Paired { .. });
        if was {
            self.phase = Phase::Connecting;
        }
        // Also before a first pairing: the list says "reconnecting".
        vec![DeviceEvent::Disconnected]
    }

    /// The next ping (once a second).
    pub fn ping(&mut self, now_ms: f64) -> ToHost {
        let id = self.next_ping;
        self.next_ping += 1;
        ToHost::Ping { id, c: now_ms }
    }

    /// The offset desktop − browser (ms) from the ping with the shortest
    /// round trip (the least queueing), and that round trip.
    pub fn clock(&self) -> Option<(f64, f64)> {
        self.pings
            .iter()
            .copied()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(rtt, off)| (off, rtt))
    }

    /// The round trip of the latest ping (ms).
    pub fn rtt_ms(&self) -> Option<f64> {
        self.pings.back().map(|p| p.0)
    }

    fn control_msg(&mut self, c: Control, now_ms: f64) -> ToHost {
        let id = self.next_control;
        self.next_control += 1;
        self.pending = Some((id, now_ms));
        ToHost::Control { id, control: c }
    }

    /// A command from the ride: the control message if it changed.
    pub fn control(&mut self, cmd: DeviceCommand, now_ms: f64) -> Option<ToHost> {
        let c = Control::of(cmd)?;
        if self.sent == Some(c) {
            return None;
        }
        self.sent = Some(c);
        if !matches!(self.phase, Phase::Paired { .. }) {
            // Sent with the next hello.
            return None;
        }
        Some(self.control_msg(c, now_ms))
    }

    /// The desktop time `h` on the browser's clock as an event timestamp:
    /// relative to the start, never before the last one.
    fn timestamp(&mut self, h: f64, now_ms: f64) -> Duration {
        let local = match self.clock() {
            Some((off, _)) => h - off,
            // No ping yet: the arrival time.
            None => now_ms,
        }
        .min(now_ms);
        self.sample_age_ms = Some((now_ms - local).max(0.0));
        let ts = (local - self.start_ms).max(self.last_ts_ms);
        self.last_ts_ms = ts;
        Duration::from_micros((ts.max(0.0) * 1000.0) as u64)
    }

    /// Whether a reading's number is new (in order).
    fn fresh(&mut self, seq: u64) -> bool {
        if seq <= self.last_seq {
            self.dropped += 1;
            return false;
        }
        self.last_seq = seq;
        self.readings += 1;
        true
    }

    /// A message from the desktop at the browser's time `now_ms`: device
    /// events for the ride.
    pub fn on_message(&mut self, msg: ToBrowser, now_ms: f64) -> Vec<DeviceEvent> {
        let mut events = Vec::new();
        match msg {
            ToBrowser::Welcome {
                host,
                epoch,
                devices,
                ..
            } => {
                if self.epoch != Some(epoch) {
                    // A new desktop run counts from 1 again.
                    self.epoch = Some(epoch);
                    self.last_seq = 0;
                }
                self.devices = devices.into_iter().map(|d| (d.name, d.state)).collect();
                self.backoff.reset();
                self.phase = Phase::Paired { host };
                events.push(DeviceEvent::Connected { kind: KIND });
            }
            ToBrowser::Refused { why } => {
                self.phase = Phase::Refused(why);
            }
            ToBrowser::Closed { why } => {
                tracing::info!(%why, "bridge closed by the desktop");
            }
            ToBrowser::Power {
                seq, h, w, rpm, ..
            } => {
                if self.fresh(seq) {
                    let timestamp = self.timestamp(h, now_ms);
                    events.push(DeviceEvent::Power {
                        watts: w,
                        cadence_rpm: rpm,
                        timestamp,
                    });
                }
            }
            ToBrowser::HeartRate { seq, h, bpm } => {
                if self.fresh(seq) {
                    let timestamp = self.timestamp(h, now_ms);
                    events.push(DeviceEvent::HeartRate { bpm, timestamp });
                }
            }
            ToBrowser::Device { name, state } => {
                match self.devices.iter_mut().find(|d| d.0 == name) {
                    Some(d) => d.1 = state,
                    None => self.devices.push((name, state)),
                }
            }
            ToBrowser::Applied { id } => {
                if let Some((p, sent)) = self.pending
                    && p == id
                {
                    self.control_rtt_ms = Some(now_ms - sent);
                    self.pending = None;
                }
                self.controls_applied += 1;
            }
            ToBrowser::Pong { c, h, .. } => {
                let rtt = (now_ms - c).max(0.0);
                // The desktop's time at the middle of the round trip.
                let off = h - (c + now_ms) / 2.0;
                self.pings.push_back((rtt, off));
                if self.pings.len() > PINGS_KEPT {
                    self.pings.pop_front();
                }
            }
        }
        events
    }

    /// Whether the desktop refused this page (no more attempts).
    pub fn is_refused(&self) -> bool {
        matches!(self.phase, Phase::Refused(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg::DeviceInfo;
    use ocw_devices::SimulationParameters;

    fn hello(secret: u32, session: &str) -> ToHost {
        Session::new(secret, session.into(), "Test browser".into(), 0.0).hello()
    }

    #[test]
    fn one_browser_at_a_time_and_the_same_page_reconnects() {
        let mut g = Gate::new(77);
        // A wrong code is refused.
        assert!(matches!(g.hello(&hello(78, "a"), 0.0), Admit::Refused(w) if w.contains("wrong pairing code")));
        assert!(matches!(
            g.hello(&ToHost::Bye, 0.0),
            Admit::Refused(_)
        ));
        let Admit::Paired { conn: a, replaces: None } = g.hello(&hello(77, "page-a"), 0.0) else {
            panic!()
        };
        // Another page while the first is alive: refused.
        g.heard(a, 1_000.0);
        assert!(matches!(g.hello(&hello(77, "page-b"), 2_000.0), Admit::Refused(w) if w.contains("Test browser")));
        // The same page again (a reconnect): replaces its old connection.
        let Admit::Paired { conn: a2, replaces } = g.hello(&hello(77, "page-a"), 2_500.0) else {
            panic!()
        };
        assert_eq!(replaces, Some(a));
        assert!(g.is_paired(a2) && !g.is_paired(a));
        // The old connection's end does not unpair the new one.
        assert!(!g.ended(a));
        assert!(g.paired().is_some());
        // A page gone quiet for longer than ALIVE_MS is replaced.
        let Admit::Paired { replaces, .. } = g.hello(&hello(77, "page-b"), 2_500.0 + ALIVE_MS + 1.0) else {
            panic!()
        };
        assert_eq!(replaces, Some(a2));
        // Version mismatch.
        let ToHost::Hello { proof, session, client, .. } = hello(77, "x") else { panic!() };
        let old = ToHost::Hello { v: 0, proof, session, client };
        assert!(matches!(g.hello(&old, 0.0), Admit::Refused(w) if w.contains("version")));
    }

    fn power(seq: u64, h: f64, w: f64) -> ToBrowser {
        ToBrowser::Power { seq, h, w, rpm: Some(90.0), virt: false }
    }

    fn welcome(epoch: u64) -> ToBrowser {
        ToBrowser::Welcome {
            v: VERSION,
            host: "Desk".into(),
            epoch,
            devices: vec![DeviceInfo { name: "KICKR".into(), state: DeviceState::Connected }],
        }
    }

    fn watts(events: &[DeviceEvent]) -> Vec<(f64, u128)> {
        events
            .iter()
            .filter_map(|e| match e {
                DeviceEvent::Power { watts, timestamp, .. } => Some((*watts, timestamp.as_millis())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn readings_keep_their_order_and_map_to_the_browser_clock() {
        // The browser's page clock starts at 10 000 ms; the desktop's bridge
        // clock is 3 000 ms ahead of it.
        let mut s = Session::new(1, "p".into(), "b".into(), 10_000.0);
        let ev = s.on_message(welcome(5), 10_100.0);
        assert_eq!(ev, [DeviceEvent::Connected { kind: KIND }]);
        assert_eq!(s.phase, Phase::Paired { host: "Desk".into() });
        // Pings: 10 ms round trip, desktop 3 000 ahead; one slow ping (90 ms
        // queueing) does not move the estimate.
        let _ = s.ping(10_200.0);
        s.on_message(ToBrowser::Pong { id: 1, c: 10_200.0, h: 13_205.0 }, 10_210.0);
        s.on_message(ToBrowser::Pong { id: 2, c: 11_200.0, h: 14_290.0 }, 11_300.0);
        let (off, rtt) = s.clock().unwrap();
        assert_eq!((off, rtt), (3_000.0, 10.0));
        assert_eq!(s.rtt_ms(), Some(100.0));
        // A reading from desktop time 14 500 (browser 11 500) arriving at
        // 11 520: 20 ms old, timestamp 1 500 ms after the start.
        let mut all = Vec::new();
        let ev = s.on_message(power(1, 14_500.0, 200.0), 11_520.0);
        all.extend(watts(&ev));
        assert_eq!(s.sample_age_ms, Some(20.0));
        // A duplicate and a late one are dropped; the next one goes on.
        let ev = s.on_message(power(1, 14_500.0, 200.0), 11_530.0);
        assert!(ev.is_empty());
        let ev = s.on_message(power(3, 14_750.0, 220.0), 11_800.0);
        all.extend(watts(&ev));
        let ev = s.on_message(power(2, 14_600.0, 210.0), 11_810.0);
        assert!(ev.is_empty());
        assert_eq!((s.readings, s.dropped), (2, 2));
        // Heart rate too, numbered with the power.
        let ev = s.on_message(ToBrowser::HeartRate { seq: 4, h: 14_800.0, bpm: 140.0 }, 11_820.0);
        assert!(matches!(ev[..], [DeviceEvent::HeartRate { bpm, .. }] if bpm == 140.0));
        assert_eq!(all, [(200.0, 1_500), (220.0, 1_750)]);
        // A reading stamped in the browser's future (clock jitter) is taken
        // as now; timestamps never go back.
        let ev = s.on_message(power(5, 99_999.0, 230.0), 11_900.0);
        assert_eq!(watts(&ev), [(230.0, 1_900)]);
        let ev = s.on_message(power(6, 14_000.0, 240.0), 11_950.0);
        assert_eq!(watts(&ev), [(240.0, 1_900)]);
    }

    #[test]
    fn a_reconnect_continues_and_a_new_desktop_run_counts_again() {
        let mut s = Session::new(1, "p".into(), "b".into(), 0.0);
        s.on_message(welcome(5), 0.0);
        s.on_message(power(10, 0.0, 200.0), 100.0);
        // The connection drops: the list says reconnecting, the backoff
        // grows until paired again.
        assert_eq!(s.on_close(), [DeviceEvent::Disconnected]);
        assert_eq!(s.phase, Phase::Connecting);
        assert_eq!(s.backoff.next_ms(), 500.0);
        assert_eq!(s.backoff.next_ms(), 1_000.0);
        // Same desktop run: an old reading resent is dropped.
        s.on_message(welcome(5), 2_000.0);
        assert_eq!(s.backoff.next_ms(), 500.0, "reset when paired");
        let ev = s.on_message(power(10, 0.0, 200.0), 2_100.0);
        assert!(ev.is_empty());
        // The desktop restarted (a new epoch): numbers from 1 are taken.
        s.on_message(welcome(6), 3_000.0);
        let ev = s.on_message(power(1, 0.0, 180.0), 3_100.0);
        assert_eq!(watts(&ev).len(), 1);
        // Refused (another browser): no more attempts.
        s.on_message(ToBrowser::Refused { why: "another browser".into() }, 4_000.0);
        assert!(s.is_refused());
        let mut b = Backoff::default();
        let delays: Vec<f64> = (0..7).map(|_| b.next_ms()).collect();
        assert_eq!(delays, [500.0, 1_000.0, 2_000.0, 4_000.0, 5_000.0, 5_000.0, 5_000.0]);
    }

    #[test]
    fn control_goes_back_when_it_changes_and_its_round_trip_is_measured() {
        let mut s = Session::new(1, "p".into(), "b".into(), 0.0);
        let sim = |grade: f64| {
            DeviceCommand::SetSimulation(SimulationParameters {
                grade,
                wind_speed_mps: 0.0,
                crr: 0.004,
                cw_kg_per_m: 0.4,
                mass_kg: 80.0,
                wind_grade: grade,
            })
        };
        // Before pairing: kept for the hello.
        assert_eq!(s.control(sim(0.03), 0.0), None);
        s.on_message(welcome(1), 10.0);
        let open = s.on_open(20.0);
        assert!(matches!(open[0], ToHost::Hello { .. }));
        let ToHost::Control { id, control } = open[1] else { panic!() };
        assert_eq!(control.command(), sim(0.03));
        s.on_message(ToBrowser::Applied { id }, 45.0);
        assert_eq!(s.control_rtt_ms, Some(25.0));
        // The same command again: nothing; a new one: sent.
        assert_eq!(s.control(sim(0.03), 100.0), None);
        let Some(ToHost::Control { id, control }) = s.control(sim(0.05), 200.0) else { panic!() };
        assert_eq!(control.command(), sim(0.05));
        s.on_message(ToBrowser::Applied { id }, 212.0);
        assert_eq!(s.control_rtt_ms, Some(12.0));
        assert_eq!(s.controls_applied, 2);
        // ERG, and nothing for the virtual trainer's own commands.
        assert!(s.control(DeviceCommand::SetTargetPower(250.0), 300.0).is_some());
        assert_eq!(s.control(DeviceCommand::SetVirtualPower(100.0), 300.0), None);
        assert_eq!(s.control(DeviceCommand::Shutdown, 300.0), None);
    }

    #[test]
    fn the_desktop_numbers_readings_and_reports_devices() {
        let mut r = Relay::default();
        let p = DeviceEvent::Power { watts: 250.0, cadence_rpm: None, timestamp: Duration::ZERO };
        let hr = DeviceEvent::HeartRate { bpm: 120.0, timestamp: Duration::ZERO };
        assert_eq!(
            r.message("KICKR", &p, 5.0, false),
            Some(ToBrowser::Power { seq: 1, h: 5.0, w: 250.0, rpm: None, virt: false })
        );
        assert_eq!(
            r.message("TICKR", &hr, 6.0, false),
            Some(ToBrowser::HeartRate { seq: 2, h: 6.0, bpm: 120.0 })
        );
        assert_eq!(
            r.message("KICKR", &DeviceEvent::Disconnected, 7.0, false),
            Some(ToBrowser::Device { name: "KICKR".into(), state: DeviceState::Reconnecting })
        );
        let mut s = Session::new(1, "p".into(), "b".into(), 0.0);
        s.on_message(welcome(1), 0.0);
        s.on_message(ToBrowser::Device { name: "KICKR".into(), state: DeviceState::Reconnecting }, 1.0);
        assert_eq!(s.devices, [("KICKR".to_owned(), DeviceState::Reconnecting)]);
        assert_ne!(proof(1), proof(2));
        assert_eq!(proof(1).len(), 32);
    }
}

//! The ANT stick service (native, spec 3.4.1 *Backend*): one thread owns
//! the stick through an [`AntTransport`] and runs the [`StickCore`]; the
//! app shares an [`AntStick`] handle to connect sensors (each its own
//! [`DeviceHandle`], as each BLE device is) and to scan. The stick is
//! opened when something needs it (a sensor, a scan) and closed a little
//! after the last one went; while closed, the service looks every
//! [`FIND_EVERY`] whether a stick is plugged in, for the Devices page.
//! A stick that cannot be opened (no permission, busy) or is unplugged
//! fails its sensors with the reason, and the service tries again every
//! [`RECONNECT_DELAY`].
//!
//! The transports: `usb::UsbTransport` (the real sticks, feature
//! `ant-usb`) and [`SimTransport`] (the simulated stick).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::sim::{SimRadio, SimSensor};
use super::stick::{Output, StickCore, UserId};
use super::{AntDeviceId, NetworkKey, RECONNECT_DELAY, StickStatus};
use crate::ble::FoundDevice;
use crate::{DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle, FailReason};

/// How often a closed service looks for a stick.
pub const FIND_EVERY: Duration = Duration::from_secs(5);
/// The stick stays open this long after the last sensor or scan went (a
/// scan followed by a connect does not reopen it).
pub const IDLE_CLOSE: Duration = Duration::from_secs(3);
/// One read waits this long at most.
const READ_WAIT: Duration = Duration::from_millis(20);

/// Opens the stick.
pub trait AntTransport: Send + 'static {
    /// The name of a plugged-in stick, without opening it.
    fn find(&mut self) -> Option<String>;
    /// Opens the first stick (errors: [`DeviceError::NoStick`],
    /// [`DeviceError::StickPermission`], [`DeviceError::StickBusy`]).
    fn open(&mut self) -> Result<Box<dyn AntLink>, DeviceError>;
}

/// An open stick: bytes in and out.
pub trait AntLink {
    fn name(&self) -> String;
    fn write(&mut self, bytes: &[u8]) -> Result<(), DeviceError>;
    /// What arrived within `timeout` (empty: nothing).
    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError>;
}

enum Cmd {
    Connect {
        user: UserId,
        device: AntDeviceId,
        events: Sender<DeviceEvent>,
        commands: Receiver<DeviceCommand>,
    },
    Scan {
        found: Sender<FoundDevice>,
        duration: Duration,
    },
    StopScan,
    SetKey(Option<NetworkKey>),
    Shutdown,
}

/// The app's handle on the stick service (cheap to clone).
#[derive(Clone)]
pub struct AntStick {
    tx: Sender<Cmd>,
    status: Arc<Mutex<StickStatus>>,
    has_key: Arc<AtomicBool>,
    next_user: Arc<AtomicU64>,
}

impl AntStick {
    /// Starts the service's thread.
    pub fn start(transport: impl AntTransport, key: Option<NetworkKey>) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(if key.is_some() {
            StickStatus::NoStick
        } else {
            StickStatus::NoKey
        }));
        let thread_status = status.clone();
        let has_key = Arc::new(AtomicBool::new(key.is_some()));
        if let Err(e) = std::thread::Builder::new()
            .name("ocw-ant".into())
            .spawn(move || Service::new(transport, key, rx, thread_status).run())
        {
            tracing::error!("cannot start the ANT stick service: {e}");
        }
        Self {
            tx,
            status,
            has_key,
            next_user: Arc::new(AtomicU64::new(1)),
        }
    }

    /// What the stick is doing (the Devices page).
    pub fn status(&self) -> StickStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn has_key(&self) -> bool {
        self.has_key.load(Ordering::Relaxed)
    }

    /// A new network key (or none): the stick is set up again.
    pub fn set_key(&self, key: Option<NetworkKey>) {
        self.has_key.store(key.is_some(), Ordering::Relaxed);
        if let Ok(mut s) = self.status.lock()
            && key.is_none()
        {
            *s = StickStatus::NoKey;
        }
        let _ = self.tx.send(Cmd::SetKey(key));
    }

    /// Starts receiving the sensor `id` (`ant:120:12345:1`).
    pub fn connect(&self, id: &str, name: &str) -> Result<DeviceHandle, DeviceError> {
        let device = AntDeviceId::parse(id)
            .ok_or_else(|| DeviceError::Start(format!("{id} is not an ANT+ sensor id")))?;
        let (ev_tx, ev_rx) = mpsc::channel();
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let user = self.next_user.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(Cmd::Connect {
                user,
                device,
                events: ev_tx,
                commands: cmd_rx,
            })
            .map_err(|_| DeviceError::Start("the ANT stick service stopped".into()))?;
        Ok(DeviceHandle {
            name: name.to_owned(),
            events: ev_rx,
            commands: cmd_tx,
        })
    }

    /// Scans for ANT+ sensors for up to `duration` (blocking: the scan
    /// thread), reporting each as it is found; `stop` ends it early.
    pub fn scan(
        &self,
        duration: Duration,
        stop: &AtomicBool,
        found: &mut dyn FnMut(FoundDevice),
    ) -> Result<(), DeviceError> {
        if !self.has_key() {
            return Err(DeviceError::NoNetworkKey);
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(Cmd::Scan {
                found: tx,
                duration,
            })
            .map_err(|_| DeviceError::Start("the ANT stick service stopped".into()))?;
        let start = Instant::now();
        let mut any = false;
        while !stop.load(Ordering::Relaxed) && start.elapsed() < duration {
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(d) => {
                    any = true;
                    found(d);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        let _ = self.tx.send(Cmd::StopScan);
        if any {
            return Ok(());
        }
        match self.status() {
            StickStatus::NoStick => Err(DeviceError::NoStick),
            StickStatus::NoPermission(_) => Err(DeviceError::StickPermission),
            StickStatus::Busy(_) => Err(DeviceError::StickBusy),
            StickStatus::NoKey => Err(DeviceError::NoNetworkKey),
            _ => Ok(()),
        }
    }

    /// Stops the service (its sensors end).
    pub fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

/// An ANT+ sensor as a [`DeviceBackend`] on a shared stick.
pub struct AntDevice {
    stick: AntStick,
    id: String,
    name: String,
}

impl AntDevice {
    pub fn new(stick: &AntStick, id: &str, name: &str) -> Self {
        Self {
            stick: stick.clone(),
            id: id.to_owned(),
            name: name.to_owned(),
        }
    }
}

impl DeviceBackend for AntDevice {
    fn name(&self) -> &str {
        &self.name
    }

    fn start(self) -> Result<DeviceHandle, DeviceError> {
        self.stick.connect(&self.id, &self.name)
    }
}

struct User {
    id: UserId,
    events: Sender<DeviceEvent>,
    commands: Receiver<DeviceCommand>,
}

struct Service<T: AntTransport> {
    transport: T,
    core: StickCore,
    rx: Receiver<Cmd>,
    status: Arc<Mutex<StickStatus>>,
    link: Option<Box<dyn AntLink>>,
    name: String,
    users: Vec<User>,
    scan: Option<Sender<FoundDevice>>,
    started: Instant,
    next_open: Duration,
    next_find: Duration,
    idle_since: Option<Duration>,
}

impl<T: AntTransport> Service<T> {
    fn new(
        transport: T,
        key: Option<NetworkKey>,
        rx: Receiver<Cmd>,
        status: Arc<Mutex<StickStatus>>,
    ) -> Self {
        Self {
            transport,
            core: StickCore::new(key),
            rx,
            status,
            link: None,
            name: String::new(),
            users: Vec::new(),
            scan: None,
            started: Instant::now(),
            next_open: Duration::ZERO,
            next_find: Duration::ZERO,
            idle_since: None,
        }
    }

    fn set_status(&self, s: StickStatus) {
        if let Ok(mut st) = self.status.lock()
            && *st != s
        {
            tracing::info!(status = %s.text(), "ANT stick");
            *st = s;
        }
    }

    fn run(mut self) {
        loop {
            let now = self.started.elapsed();
            let mut out = Vec::new();
            // The app's commands.
            loop {
                match self.rx.try_recv() {
                    Ok(Cmd::Connect {
                        user,
                        device,
                        events,
                        commands,
                    }) => {
                        tracing::info!(%device, "ANT+ sensor wanted");
                        self.users.push(User {
                            id: user,
                            events,
                            commands,
                        });
                        out.extend(self.core.add_sensor(user, device, now));
                    }
                    Ok(Cmd::Scan { found, duration }) => {
                        self.scan = Some(found);
                        out.extend(self.core.start_scan(now, now + duration));
                    }
                    Ok(Cmd::StopScan) => {
                        self.scan = None;
                        self.core.stop_scan();
                    }
                    Ok(Cmd::SetKey(key)) => {
                        out.extend(self.core.set_key(key, now));
                        self.next_open = now;
                        self.next_find = now;
                    }
                    Ok(Cmd::Shutdown) | Err(TryRecvError::Disconnected) => return,
                    Err(TryRecvError::Empty) => break,
                }
            }
            // Each sensor's handle: gone or shut down → its channel goes.
            let core = &mut self.core;
            self.users.retain(|u| {
                loop {
                    match u.commands.try_recv() {
                        Ok(DeviceCommand::Shutdown) | Err(TryRecvError::Disconnected) => {
                            core.remove_sensor(u.id);
                            return false;
                        }
                        // SIM / ERG: FE-C trainers will take them (spec 14
                        // Phase 17, the next steps); sensors ignore them.
                        Ok(_) => {}
                        Err(TryRecvError::Empty) => return true,
                    }
                }
            });
            self.manage_link(now, &mut out);
            if let Some(mut link) = self.link.take() {
                let mut lost = None;
                match link.read(READ_WAIT) {
                    Ok(bytes) if !bytes.is_empty() => {
                        out.extend(self.core.on_bytes(&bytes, now));
                    }
                    Ok(_) => {}
                    Err(e) => lost = Some(e),
                }
                let now = self.started.elapsed();
                out.extend(self.core.on_tick(now));
                if lost.is_none() {
                    for w in self.core.take_writes() {
                        if let Err(e) = link.write(&w) {
                            lost = Some(e);
                            break;
                        }
                    }
                }
                if let Some(e) = lost {
                    tracing::warn!("the ANT stick is gone: {e}");
                    out.extend(self.core.link_down(FailReason::NoStick));
                    self.next_open = now + RECONNECT_DELAY;
                    self.next_find = now;
                    self.set_status(StickStatus::NoStick);
                } else {
                    if self.core.is_ready() {
                        self.set_status(StickStatus::Ready(self.name.clone()));
                    } else if self.core.refused() {
                        self.set_status(StickStatus::Failed(
                            "the stick refused the network key".into(),
                        ));
                    }
                    self.link = Some(link);
                }
            } else {
                out.extend(self.core.on_tick(now));
                std::thread::sleep(Duration::from_millis(50));
            }
            self.dispatch(out);
            if !self.core.scanning() {
                self.scan = None;
            }
        }
    }

    /// Opens the stick when it is needed, closes it when not; looks for
    /// one while closed.
    fn manage_link(&mut self, now: Duration, out: &mut Vec<Output>) {
        let wanted = self.core.wants_link() && self.core.has_key();
        match (self.link.is_some(), wanted) {
            (false, true) if now >= self.next_open => match self.transport.open() {
                Ok(link) => {
                    self.name = link.name();
                    tracing::info!(stick = %self.name, "ANT stick opened");
                    self.set_status(StickStatus::Opening(self.name.clone()));
                    self.core.link_up(now);
                    self.link = Some(link);
                    self.idle_since = None;
                }
                Err(e) => {
                    let reason = FailReason::of(&e);
                    let name = self
                        .transport
                        .find()
                        .unwrap_or_else(|| "ANT stick".to_owned());
                    self.set_status(match e {
                        DeviceError::StickPermission => StickStatus::NoPermission(name),
                        DeviceError::StickBusy => StickStatus::Busy(name),
                        DeviceError::NoStick => StickStatus::NoStick,
                        other => StickStatus::Failed(other.to_string()),
                    });
                    out.extend(self.core.stick_failed(reason));
                    self.next_open = now + RECONNECT_DELAY;
                }
            },
            (false, false) if now >= self.next_find => {
                self.next_find = now + FIND_EVERY;
                let s = if !self.core.has_key() {
                    StickStatus::NoKey
                } else {
                    match self.transport.find() {
                        Some(n) => StickStatus::Found(n),
                        None => StickStatus::NoStick,
                    }
                };
                self.set_status(s);
            }
            (true, false) => {
                let since = *self.idle_since.get_or_insert(now);
                if now.saturating_sub(since) >= IDLE_CLOSE || !self.core.has_key() {
                    tracing::info!(stick = %self.name, "ANT stick closed (not in use)");
                    self.link = None;
                    out.extend(self.core.link_down(FailReason::NoStick));
                    self.idle_since = None;
                    self.next_find = now;
                }
            }
            (true, true) => self.idle_since = None,
            _ => {}
        }
    }

    fn dispatch(&mut self, out: Vec<Output>) {
        for o in out {
            match o {
                Output::Event(user, e) => {
                    if let Some(u) = self.users.iter().find(|u| u.id == user) {
                        let _ = u.events.send(e);
                    }
                }
                Output::Found(d) => {
                    if let Some(tx) = &self.scan {
                        let _ = tx.send(d);
                    }
                }
            }
        }
    }
}

/// The simulated stick as a transport: a [`SimRadio`] on the wall clock,
/// which the app (`--ant-sim`) and tests control through this handle:
/// sensors on and off, the stick unplugged, busy or without permission.
#[derive(Clone)]
pub struct SimTransport(Arc<Mutex<SimWorld>>);

struct SimWorld {
    radio: SimRadio,
    plugged: bool,
    busy: bool,
    denied: bool,
    /// Bumped on every unplug: older links fail.
    generation: u32,
    /// Links open now (a stick is opened by one program at a time).
    open: usize,
    started: Instant,
}

impl SimTransport {
    /// A simulated stick that hears sensors with `key` (`None`: any key).
    pub fn new(key: Option<NetworkKey>, sensors: Vec<SimSensor>) -> Self {
        let mut radio = SimRadio::new(key);
        for s in sensors {
            radio.add(s);
        }
        Self(Arc::new(Mutex::new(SimWorld {
            radio,
            plugged: true,
            busy: false,
            denied: false,
            generation: 0,
            open: 0,
            started: Instant::now(),
        })))
    }

    fn with<R>(&self, f: impl FnOnce(&mut SimWorld) -> R) -> R {
        let mut w = self.0.lock().unwrap_or_else(|e| e.into_inner());
        f(&mut w)
    }

    /// Changes the simulated sensors (add, remove, heart rate, …).
    pub fn radio<R>(&self, f: impl FnOnce(&mut SimRadio) -> R) -> R {
        self.with(|w| f(&mut w.radio))
    }

    pub fn set_plugged(&self, plugged: bool) {
        self.with(|w| {
            if w.plugged && !plugged {
                w.generation += 1;
            }
            w.plugged = plugged;
        });
    }

    /// Another program has it.
    pub fn set_busy(&self, busy: bool) {
        self.with(|w| w.busy = busy);
    }

    /// The system does not let us open it.
    pub fn set_denied(&self, denied: bool) {
        self.with(|w| w.denied = denied);
    }

    /// How many links have it open (0 or 1).
    pub fn links_open(&self) -> usize {
        self.with(|w| w.open)
    }
}

const SIM_NAME: &str = "Simulated ANT stick";

impl AntTransport for SimTransport {
    fn find(&mut self) -> Option<String> {
        self.with(|w| w.plugged.then(|| SIM_NAME.to_owned()))
    }

    fn open(&mut self) -> Result<Box<dyn AntLink>, DeviceError> {
        self.with(|w| {
            if !w.plugged {
                Err(DeviceError::NoStick)
            } else if w.denied {
                Err(DeviceError::StickPermission)
            } else if w.busy || w.open > 0 {
                Err(DeviceError::StickBusy)
            } else {
                // A stick plugged in again is reset by the core first.
                w.open += 1;
                Ok(())
            }
        })?;
        let generation = self.with(|w| w.generation);
        Ok(Box::new(SimLink {
            world: self.0.clone(),
            generation,
        }))
    }
}

struct SimLink {
    world: Arc<Mutex<SimWorld>>,
    generation: u32,
}

impl Drop for SimLink {
    fn drop(&mut self) {
        let mut w = self.world.lock().unwrap_or_else(|e| e.into_inner());
        w.open = w.open.saturating_sub(1);
    }
}

impl SimLink {
    fn with<R>(&self, f: impl FnOnce(&mut SimWorld) -> R) -> Result<R, DeviceError> {
        let mut w = self.world.lock().unwrap_or_else(|e| e.into_inner());
        if !w.plugged || w.generation != self.generation {
            return Err(DeviceError::NoStick);
        }
        Ok(f(&mut w))
    }
}

impl AntLink for SimLink {
    fn name(&self) -> String {
        SIM_NAME.to_owned()
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        self.with(|w| {
            let now = w.started.elapsed();
            w.radio.host(bytes, now);
        })
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        std::thread::sleep(timeout.min(Duration::from_millis(10)));
        self.with(|w| {
            let now = w.started.elapsed();
            w.radio.step(now);
            w.radio.take_output()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ant::AntProfile;

    fn wait_for(
        rx: &Receiver<DeviceEvent>,
        what: &str,
        ok: impl Fn(&DeviceEvent) -> bool,
    ) -> Vec<DeviceEvent> {
        let start = Instant::now();
        let mut seen = Vec::new();
        while start.elapsed() < Duration::from_secs(10) {
            if let Ok(e) = rx.recv_timeout(Duration::from_millis(50)) {
                seen.push(e);
                if ok(&e) {
                    return seen;
                }
            }
        }
        panic!("{what}: {seen:?}");
    }

    #[test]
    fn the_service_scans_connects_and_reconnects_after_an_unplug() {
        let sim = SimTransport::new(
            Some(NetworkKey::TEST),
            vec![
                SimSensor::strap(12345, 142.0),
                SimSensor::strap(23456, 128.0),
            ],
        );
        let stick = AntStick::start(sim.clone(), Some(NetworkKey::TEST));
        // Scan: both straps.
        let stop = AtomicBool::new(false);
        let mut found = Vec::new();
        stick
            .scan(Duration::from_secs(3), &stop, &mut |d| found.push(d.id))
            .unwrap();
        found.sort();
        assert_eq!(found, ["ant:120:12345:1", "ant:120:23456:1"]);
        // Connect one: its heart rate, as from a BLE strap.
        let h = AntDevice::new(&stick, "ant:120:12345:1", "ANT+ heart rate 12345")
            .start()
            .unwrap();
        wait_for(&h.events, "connected", |e| {
            *e == DeviceEvent::Connected {
                kind: AntProfile::HeartRate.kind(),
            }
        });
        wait_for(
            &h.events,
            "142 bpm",
            |e| matches!(e, DeviceEvent::HeartRate { bpm, .. } if *bpm == 142.0),
        );
        assert!(matches!(stick.status(), StickStatus::Ready(_)));
        // Unplugged: failed (no stick), and back once plugged in again.
        sim.set_plugged(false);
        wait_for(&h.events, "no stick", |e| {
            *e == DeviceEvent::Failed {
                reason: FailReason::NoStick,
            }
        });
        assert_eq!(stick.status(), StickStatus::NoStick);
        sim.set_plugged(true);
        wait_for(&h.events, "connected again", |e| {
            matches!(e, DeviceEvent::Connected { .. })
        });
        // The handle goes: the stick closes a little later.
        h.commands.send(DeviceCommand::Shutdown).unwrap();
        let start = Instant::now();
        while !matches!(stick.status(), StickStatus::Found(_)) {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "{:?}",
                stick.status()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        stick.shutdown();
    }

    #[test]
    fn a_stick_without_permission_or_busy_says_why() {
        let sim = SimTransport::new(None, vec![SimSensor::strap(1, 60.0)]);
        sim.set_denied(true);
        let stick = AntStick::start(sim.clone(), Some(NetworkKey::TEST));
        let h = stick.connect("ant:120:1:1", "ANT+ heart rate 1").unwrap();
        wait_for(&h.events, "permission", |e| {
            *e == DeviceEvent::Failed {
                reason: FailReason::NoPermission,
            }
        });
        assert!(matches!(stick.status(), StickStatus::NoPermission(_)));
        sim.set_denied(false);
        sim.set_busy(true);
        wait_for(&h.events, "busy", |e| {
            *e == DeviceEvent::Failed {
                reason: FailReason::StickBusy,
            }
        });
        sim.set_busy(false);
        wait_for(&h.events, "connected", |e| {
            matches!(e, DeviceEvent::Connected { .. })
        });
        stick.shutdown();
    }

    #[test]
    fn without_a_key_nothing_is_opened() {
        let sim = SimTransport::new(None, vec![SimSensor::strap(1, 60.0)]);
        let stick = AntStick::start(sim, None);
        assert_eq!(stick.status(), StickStatus::NoKey);
        let stop = AtomicBool::new(false);
        assert!(matches!(
            stick.scan(Duration::from_millis(100), &stop, &mut |_| {}),
            Err(DeviceError::NoNetworkKey)
        ));
        let h = stick.connect("ant:120:1:1", "x").unwrap();
        wait_for(&h.events, "no key", |e| {
            *e == DeviceEvent::Failed {
                reason: FailReason::NoKey,
            }
        });
        // The key arrives: it connects.
        stick.set_key(Some(NetworkKey::TEST));
        wait_for(&h.events, "connected", |e| {
            matches!(e, DeviceEvent::Connected { .. })
        });
        assert!(stick.connect("hci0/dev_AA", "x").is_err());
        stick.shutdown();
    }
}

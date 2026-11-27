//! ANT USB sticks over the browser's WebUSB (spec 14 Phase 17 A5, 17E
//! *The browser*; Q3 (a) + (b): Chromium browsers on the desktop and on
//! Android; on Windows the stick needs the WinUSB driver, Q7).
//!
//! The browser hands the page `USBDevice` objects – after its chooser
//! (`navigator.usb.requestDevice` with the filters of [`request_options`],
//! from a user gesture), or without it for the sticks this site was
//! granted before (`navigator.usb.getDevices()`). This module drives the
//! **same** pure [`StickCore`] as the desktop's stick service over them:
//! nothing of the ANT protocol, the channels, the heart-rate or power pages
//! is implemented twice.
//!
//! - [`UsbStick`] / [`UsbHost`] are the boundary: the few WebUSB calls the
//!   link needs (open + `selectConfiguration(1)` + `claimInterface(0)` →
//!   the interface's endpoints, `transferIn`, `transferOut`, `clearHalt`,
//!   `reset`, `close`; the permitted sticks and the `connect` /
//!   `disconnect` events). In the browser it is a thin binding over
//!   `navigator.usb` (`browser`, wasm32 only); in the tests a fake over the
//!   simulated stick ([`super::sim::SimRadio`]).
//! - [`WebStickService::run`] (pure, async): the stick service of the page
//!   on its event loop – as the native `service.rs` on its thread: the
//!   stick opened while a sensor or a scan needs it and closed
//!   [`IDLE_CLOSE`] after the last; one `transferIn` always pending, raced
//!   against a short timer; every write the core makes sent with
//!   `transferOut`, one at a time; a stall cleared (`clearHalt`); a
//!   transfer that fails otherwise resets the stick (`reset`) and opens it
//!   again after [`RECONNECT_DELAY`]; an unplugged stick (`disconnect`, the
//!   pending transfer rejecting) fails every sensor with *no ANT stick* and
//!   all come back when it is plugged in again (`connect`, found again with
//!   `getDevices`).
//! - [`WebAntStick`]: the page's handle (as the native `AntStick`):
//!   connect a sensor (a [`DeviceHandle`]), scan, the network key, the
//!   stick's state.
//!
//! The ANT+ network key is never in this crate (Q1 (b)): in the browser the
//! rider types it on Devices → ANT+, the page keeps it in its own storage
//! only and hands it in as a [`NetworkKey`] (whose `Debug` hides it).

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::future::{Either, LocalBoxFuture, select};

use super::stick::{Output, StickCore, UserId};
use super::{
    AntDeviceId, NetworkKey, RECONNECT_DELAY, StickStatus, USB_PRODUCT_USB2, USB_PRODUCT_USBM,
    USB_VENDOR,
};
use crate::ble::FoundDevice;
use crate::ble::runner::Timer;
use crate::{DeviceCommand, DeviceError, DeviceEvent, DeviceHandle, FailReason};

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
pub use browser::{JsUsbHost, JsUsbStick, has_web_usb, request, start};

/// How often a closed service looks for a stick (as the desktop's).
pub const FIND_EVERY: Duration = Duration::from_secs(5);
/// The stick stays open this long after the last sensor or scan went.
pub const IDLE_CLOSE: Duration = Duration::from_secs(3);
/// One wait for the pending `transferIn` (then time passes for the core).
pub const READ_WAIT: Duration = Duration::from_millis(20);
/// The loop's pause while no stick is open.
pub const IDLE_WAIT: Duration = Duration::from_millis(50);
/// Bytes asked for per `transferIn` (the sticks' bulk packets are 64).
pub const IN_LENGTH: u32 = 64;

/// The USB ids the browser's chooser offers: the ANT USB2 and the ANT
/// USB-m (and the clones that copy them).
pub const FILTERS: [(u16, u16); 2] = [
    (USB_VENDOR, USB_PRODUCT_USB2),
    (USB_VENDOR, USB_PRODUCT_USBM),
];

/// Whether a USB device is an ANT stick OCW drives.
pub fn is_stick(vendor: u16, product: u16) -> bool {
    FILTERS.contains(&(vendor, product))
}

/// The options of `navigator.usb.requestDevice` as JSON.
pub fn request_options() -> String {
    let filters: Vec<String> = FILTERS
        .iter()
        .map(|(v, p)| format!(r#"{{"vendorId":{v},"productId":{p}}}"#))
        .collect();
    format!(r#"{{"filters":[{}]}}"#, filters.join(","))
}

/// One endpoint of the stick's interface 0, as WebUSB reports it
/// (`USBEndpoint`: `endpointNumber`, `direction`, `type`, `packetSize`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UsbEndpoint {
    /// The number without the direction bit (what `transferIn` takes).
    pub number: u8,
    /// `direction == "in"`.
    pub is_in: bool,
    /// `type == "bulk"`.
    pub bulk: bool,
    pub packet_size: u16,
}

/// The bulk endpoints the link uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Endpoints {
    pub ep_in: u8,
    pub ep_out: u8,
    /// Bytes per `transferIn` (at least [`IN_LENGTH`]).
    pub in_length: u32,
}

/// The first bulk IN and the first bulk OUT endpoint (the ANT USB-m and
/// the USB2 have one of each on interface 0; the numbers differ between
/// them, so they come from the descriptor, as on the desktop).
pub fn bulk_endpoints(eps: &[UsbEndpoint]) -> Option<Endpoints> {
    let ep_in = eps.iter().find(|e| e.bulk && e.is_in)?;
    let ep_out = eps.iter().find(|e| e.bulk && !e.is_in)?;
    Some(Endpoints {
        ep_in: ep_in.number,
        ep_out: ep_out.number,
        in_length: u32::from(ep_in.packet_size).max(IN_LENGTH),
    })
}

/// A transfer's `status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferStatus {
    Ok,
    /// The endpoint stalled: `clearHalt` and go on.
    Stall,
    /// More data than asked for (the bytes are dropped).
    Babble,
}

impl TransferStatus {
    pub fn parse(s: &str) -> Self {
        match s {
            "stall" => Self::Stall,
            "babble" => Self::Babble,
            _ => Self::Ok,
        }
    }
}

/// What a `transferIn` gave.
pub type InResult = Result<(TransferStatus, Vec<u8>), DeviceError>;

/// One `USBDevice` (an ANT stick) as the link uses it. The futures own
/// what they need (`'static`), so a pending `transferIn` lives across the
/// loop's turns.
pub trait UsbStick {
    /// "ANT USB-m stick".
    fn name(&self) -> String;
    /// `open()`, `selectConfiguration(1)` when none is selected,
    /// `claimInterface(0)`; the endpoints of interface 0's alternate.
    fn open(&self) -> LocalBoxFuture<'static, Result<Vec<UsbEndpoint>, DeviceError>>;
    fn transfer_in(&self, endpoint: u8, length: u32) -> LocalBoxFuture<'static, InResult>;
    fn transfer_out(
        &self,
        endpoint: u8,
        data: Vec<u8>,
    ) -> LocalBoxFuture<'static, Result<TransferStatus, DeviceError>>;
    /// `clearHalt(direction, endpoint)` after a stall.
    fn clear_halt(
        &self,
        is_in: bool,
        endpoint: u8,
    ) -> LocalBoxFuture<'static, Result<(), DeviceError>>;
    /// `reset()`: after a transfer failed (not an unplug).
    fn reset(&self) -> LocalBoxFuture<'static, Result<(), DeviceError>>;
    /// `releaseInterface(0)` and `close()` (errors ignored: it may be gone).
    fn close(&self) -> LocalBoxFuture<'static, ()>;
}

/// The page's WebUSB: the sticks it may use.
pub trait UsbHost {
    type Stick: UsbStick;
    /// The ANT sticks this site was granted that are plugged in
    /// (`navigator.usb.getDevices()`, by [`FILTERS`]).
    fn sticks(&self) -> LocalBoxFuture<'static, Vec<Self::Stick>>;
    /// A stick was plugged in or out, or granted, since the last call
    /// (`navigator.usb`'s `connect` / `disconnect` events, *Add an ANT+
    /// stick*): the service looks at once instead of waiting.
    fn take_changed(&self) -> bool;
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
    /// A stick was granted (the chooser): look now.
    Look,
    Shutdown,
}

/// The page's handle on its stick service (cheap to clone; `Send`, so the
/// app's resources can hold it).
#[derive(Clone)]
pub struct WebAntStick {
    tx: Sender<Cmd>,
    status: Arc<Mutex<StickStatus>>,
    has_key: Arc<AtomicBool>,
    scanning: Arc<AtomicBool>,
    next_user: Arc<AtomicU64>,
}

impl WebAntStick {
    /// The handle and its service; the page runs
    /// [`WebStickService::run`] as a task on its event loop.
    pub fn new(key: Option<NetworkKey>) -> (Self, WebStickService) {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(if key.is_some() {
            StickStatus::NoStick
        } else {
            StickStatus::NoKey
        }));
        let has_key = Arc::new(AtomicBool::new(key.is_some()));
        let scanning = Arc::new(AtomicBool::new(false));
        let service = WebStickService {
            core: StickCore::new(key),
            rx,
            status: status.clone(),
            scanning: scanning.clone(),
            users: Vec::new(),
            scan: None,
            next_open: Duration::ZERO,
            next_find: Duration::ZERO,
            idle_since: None,
            stopped: false,
        };
        (
            Self {
                tx,
                status,
                has_key,
                scanning,
                next_user: Arc::new(AtomicU64::new(1)),
            },
            service,
        )
    }

    /// What the stick is doing (the Devices page).
    pub fn status(&self) -> StickStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }

    pub fn has_key(&self) -> bool {
        self.has_key.load(Ordering::Relaxed)
    }

    /// A scan runs.
    pub fn scanning(&self) -> bool {
        self.scanning.load(Ordering::Relaxed)
    }

    /// A new network key (or none): the stick is set up again.
    pub fn set_key(&self, key: Option<NetworkKey>) {
        self.has_key.store(key.is_some(), Ordering::Relaxed);
        if key.is_none()
            && let Ok(mut s) = self.status.lock()
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

    /// Scans for ANT+ sensors for `duration`; each find arrives on the
    /// receiver (the page polls it), which closes when the scan ends.
    pub fn scan(&self, duration: Duration) -> Result<Receiver<FoundDevice>, DeviceError> {
        if !self.has_key() {
            return Err(DeviceError::NoNetworkKey);
        }
        let (tx, rx) = mpsc::channel();
        self.scanning.store(true, Ordering::Relaxed);
        self.tx
            .send(Cmd::Scan {
                found: tx,
                duration,
            })
            .map_err(|_| DeviceError::Start("the ANT stick service stopped".into()))?;
        Ok(rx)
    }

    pub fn stop_scan(&self) {
        let _ = self.tx.send(Cmd::StopScan);
    }

    /// A stick was just granted (the chooser): the service looks now.
    pub fn look(&self) {
        let _ = self.tx.send(Cmd::Look);
    }

    /// Stops the service (its sensors end, the stick is closed).
    pub fn shutdown(&self) {
        let _ = self.tx.send(Cmd::Shutdown);
    }
}

struct User {
    id: UserId,
    events: Sender<DeviceEvent>,
    commands: Receiver<DeviceCommand>,
}

/// An opened stick.
struct OpenLink<S> {
    stick: S,
    name: String,
    eps: Endpoints,
    /// The pending `transferIn`.
    read: Option<LocalBoxFuture<'static, InResult>>,
}

/// The stick service of the page (see the module): the pure core, the
/// sensors' channels and the scan.
pub struct WebStickService {
    core: StickCore,
    rx: Receiver<Cmd>,
    status: Arc<Mutex<StickStatus>>,
    scanning: Arc<AtomicBool>,
    users: Vec<User>,
    scan: Option<Sender<FoundDevice>>,
    next_open: Duration,
    next_find: Duration,
    idle_since: Option<Duration>,
    stopped: bool,
}

impl WebStickService {
    fn set_status(&self, s: StickStatus) {
        if let Ok(mut st) = self.status.lock()
            && *st != s
        {
            tracing::info!(status = %s.text(), "ANT stick (WebUSB)");
            *st = s;
        }
    }

    /// The page's commands; `false` once it shut the service down.
    fn commands(&mut self, now: Duration, out: &mut Vec<Output>) -> bool {
        loop {
            match self.rx.try_recv() {
                Ok(Cmd::Connect {
                    user,
                    device,
                    events,
                    commands,
                }) => {
                    tracing::info!(%device, "ANT+ sensor wanted (WebUSB)");
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
                Ok(Cmd::Look) => {
                    self.next_open = now;
                    self.next_find = now;
                }
                Ok(Cmd::Shutdown) | Err(TryRecvError::Disconnected) => return false,
                Err(TryRecvError::Empty) => return true,
            }
        }
    }

    /// Each sensor's handle: gone or shut down → its channel goes.
    fn users(&mut self) {
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
        if !self.core.scanning() {
            self.scan = None;
            self.scanning.store(false, Ordering::Relaxed);
        }
    }

    fn open_failed(&mut self, e: DeviceError, name: String, now: Duration, out: &mut Vec<Output>) {
        let reason = FailReason::of(&e);
        tracing::info!(stick = %name, "the ANT stick cannot be opened over WebUSB: {e}");
        self.set_status(match e {
            DeviceError::StickPermission => StickStatus::NoPermission(name),
            DeviceError::StickBusy => StickStatus::Busy(name),
            DeviceError::StickDriver(_) => StickStatus::NoDriver(name),
            DeviceError::NoStick => StickStatus::NoStick,
            other => StickStatus::Failed(other.to_string()),
        });
        out.extend(self.core.stick_failed(reason));
        self.next_open = now + RECONNECT_DELAY;
    }

    /// Opens the stick when it is needed, closes it when not; looks for
    /// one while closed.
    async fn manage<H: UsbHost>(
        &mut self,
        host: &H,
        link: &mut Option<OpenLink<H::Stick>>,
        now: Duration,
        changed: bool,
        out: &mut Vec<Output>,
    ) {
        let wanted = self.core.wants_link() && self.core.has_key();
        match (link.is_some(), wanted) {
            (false, true) if now >= self.next_open || changed => {
                let Some(stick) = host.sticks().await.into_iter().next() else {
                    self.open_failed(DeviceError::NoStick, String::new(), now, out);
                    return;
                };
                let name = stick.name();
                self.set_status(StickStatus::Opening(name.clone()));
                let opened = stick.open().await.and_then(|eps| {
                    bulk_endpoints(&eps)
                        .ok_or_else(|| DeviceError::Ant(format!("{name}: no bulk endpoints")))
                });
                match opened {
                    Ok(eps) => {
                        tracing::info!(stick = %name, "ANT stick opened over WebUSB");
                        self.core.link_up(now);
                        self.idle_since = None;
                        *link = Some(OpenLink {
                            stick,
                            name,
                            eps,
                            read: None,
                        });
                    }
                    Err(e) => {
                        stick.close().await;
                        self.open_failed(e, name, now, out);
                    }
                }
            }
            (false, false) if now >= self.next_find || changed => {
                self.next_find = now + FIND_EVERY;
                let s = if !self.core.has_key() {
                    StickStatus::NoKey
                } else {
                    match host.sticks().await.first() {
                        Some(s) => StickStatus::Found(s.name()),
                        None => StickStatus::NoStick,
                    }
                };
                self.set_status(s);
            }
            (true, false) => {
                let since = *self.idle_since.get_or_insert(now);
                if now.saturating_sub(since) >= IDLE_CLOSE || !self.core.has_key() {
                    if let Some(l) = link.take() {
                        tracing::info!(stick = %l.name, "ANT stick closed (not in use)");
                        l.stick.close().await;
                    }
                    out.extend(self.core.link_down(FailReason::NoStick));
                    self.idle_since = None;
                    self.next_find = now;
                }
            }
            (true, true) => self.idle_since = None,
            _ => {}
        }
    }

    /// One turn with the stick open: wait a little for the pending
    /// `transferIn`, let time pass, send what the core wrote. `Err`: the
    /// stick is lost (unplugged, or a transfer failed).
    async fn exchange<S: UsbStick, T: Timer>(
        &mut self,
        link: &mut OpenLink<S>,
        timer: &T,
        clock: &impl Fn() -> Duration,
        out: &mut Vec<Output>,
    ) -> Result<(), DeviceError> {
        let read = match link.read.take() {
            Some(r) => r,
            None => link.stick.transfer_in(link.eps.ep_in, link.eps.in_length),
        };
        let sleep = Box::pin(timer.sleep(READ_WAIT));
        match select(read, sleep).await {
            Either::Left((got, _)) => match got? {
                (TransferStatus::Ok, bytes) => {
                    if !bytes.is_empty() {
                        out.extend(self.core.on_bytes(&bytes, clock()));
                    }
                }
                (TransferStatus::Stall, _) => {
                    tracing::debug!("ANT stick: IN endpoint stalled, clearing it");
                    link.stick.clear_halt(true, link.eps.ep_in).await?;
                }
                (TransferStatus::Babble, _) => {
                    tracing::debug!("ANT stick: babble on the IN endpoint (dropped)");
                }
            },
            Either::Right(((), read)) => link.read = Some(read),
        }
        out.extend(self.core.on_tick(clock()));
        for w in self.core.take_writes() {
            if link.stick.transfer_out(link.eps.ep_out, w).await? == TransferStatus::Stall {
                tracing::debug!("ANT stick: OUT endpoint stalled, clearing it");
                link.stick.clear_halt(false, link.eps.ep_out).await?;
            }
        }
        Ok(())
    }

    /// The page's stick service until [`WebAntStick::shutdown`] (or the
    /// handle is dropped): `clock` is the time since the service started
    /// (`web_time::Instant` in the browser), `timer` its sleeps.
    pub async fn run<H: UsbHost, T: Timer>(
        mut self,
        host: &H,
        timer: &T,
        clock: impl Fn() -> Duration,
    ) {
        let mut link: Option<OpenLink<H::Stick>> = None;
        while !self.stopped {
            let now = clock();
            let mut out = Vec::new();
            if !self.commands(now, &mut out) {
                self.stopped = true;
                break;
            }
            self.users();
            let changed = host.take_changed();
            self.manage(host, &mut link, now, changed, &mut out).await;
            match link.as_mut() {
                Some(l) => {
                    if let Err(e) = self.exchange(l, timer, &clock, &mut out).await {
                        let l = link.take().expect("open");
                        let now = clock();
                        if matches!(e, DeviceError::NoStick) {
                            tracing::warn!(stick = %l.name, "the ANT stick is gone (unplugged)");
                        } else {
                            // A transfer that failed for another reason:
                            // reset the stick, then open it again.
                            tracing::warn!(stick = %l.name, "the ANT stick failed: {e}; resetting it");
                            let _ = l.stick.reset().await;
                        }
                        l.stick.close().await;
                        out.extend(self.core.link_down(FailReason::NoStick));
                        self.next_open = now + RECONNECT_DELAY;
                        self.next_find = now;
                        self.set_status(StickStatus::NoStick);
                    } else if self.core.is_ready() {
                        self.set_status(StickStatus::Ready(l.name.clone()));
                    } else if self.core.refused() {
                        self.set_status(StickStatus::Failed(
                            "the stick refused the network key".into(),
                        ));
                    }
                }
                None => {
                    out.extend(self.core.on_tick(now));
                    timer.sleep(IDLE_WAIT).await;
                }
            }
            self.dispatch(out);
        }
        if let Some(l) = link.take() {
            l.stick.close().await;
        }
        tracing::info!("ANT stick service (WebUSB) stopped");
    }
}

#[cfg(test)]
pub(crate) mod fake;

#[cfg(test)]
mod tests;

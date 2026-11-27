//! A fake `navigator.usb` over the simulated stick, and virtual time: the
//! service's sleeps wait for a clock the test moves on, so a minute of
//! riding takes milliseconds and every run is the same.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use futures::FutureExt;
use futures::executor::LocalPool;
use futures::future::LocalBoxFuture;

use super::{InResult, TransferStatus, UsbEndpoint, UsbHost, UsbStick};
use crate::DeviceError;
use crate::ant::sim::{SimRadio, SimSensor};
use crate::ant::{NetworkKey, USB_PRODUCT_USBM, stick_name};
use crate::ble::runner::Timer;

/// The clock and the sleepers waiting for it.
#[derive(Default)]
pub struct VirtualTime {
    now: Cell<Duration>,
    sleepers: RefCell<Vec<(Duration, Waker)>>,
}

impl VirtualTime {
    pub fn now(&self) -> Duration {
        self.now.get()
    }

    /// To the next sleeper's deadline (waking every sleeper due then);
    /// `false`: nobody sleeps.
    fn advance(&self) -> bool {
        let next = self.sleepers.borrow().iter().map(|(t, _)| *t).min();
        let Some(next) = next else { return false };
        self.now.set(self.now.get().max(next));
        let now = self.now.get();
        let due: Vec<Waker> = {
            let mut s = self.sleepers.borrow_mut();
            let (due, keep): (Vec<_>, Vec<_>) = s.drain(..).partition(|(t, _)| *t <= now);
            *s = keep;
            due.into_iter().map(|(_, w)| w).collect()
        };
        for w in due {
            w.wake();
        }
        true
    }
}

/// [`Timer`] on [`VirtualTime`].
#[derive(Clone)]
pub struct FakeTimer(pub Rc<VirtualTime>);

struct Sleep {
    time: Rc<VirtualTime>,
    until: Duration,
}

impl Future for Sleep {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.time.now() >= self.until {
            Poll::Ready(())
        } else {
            self.time
                .sleepers
                .borrow_mut()
                .push((self.until, cx.waker().clone()));
            Poll::Pending
        }
    }
}

impl Timer for FakeTimer {
    fn sleep(&self, duration: Duration) -> impl Future<Output = ()> {
        Sleep {
            time: self.0.clone(),
            until: self.0.now() + duration,
        }
    }
}

/// Runs the pool's tasks, moving the clock on, until `until`.
pub fn drive(pool: &mut LocalPool, time: &VirtualTime, until: Duration) {
    loop {
        pool.run_until_stalled();
        if time.now() >= until || !time.advance() {
            return;
        }
    }
}

/// What the fake stick and its "browser" are doing.
pub struct World {
    pub radio: SimRadio,
    pub plugged: bool,
    /// Bumped on every unplug: transfers of older handles fail.
    pub generation: u32,
    /// `open()` fails: `SecurityError` (Linux without the udev rule).
    pub denied: bool,
    /// `claimInterface` fails: another program has the stick.
    pub busy: bool,
    pub endpoints: Vec<UsbEndpoint>,
    pub changed: bool,
    /// The next `transferIn` stalls.
    pub stall_in: bool,
    /// The next `transferOut` fails (not an unplug).
    pub fail_out: bool,
    /// Bytes the stick sent that no `transferIn` took yet.
    pending: Vec<u8>,
    pub opened: bool,
    pub opens: u32,
    pub closes: u32,
    pub resets: u32,
    pub clear_halts: Vec<(bool, u8)>,
    pub writes: u32,
}

/// The ANT USB-m's interface 0: bulk IN 1 and bulk OUT 1 (64 bytes).
pub fn usbm_endpoints() -> Vec<UsbEndpoint> {
    vec![
        UsbEndpoint {
            number: 1,
            is_in: true,
            bulk: true,
            packet_size: 64,
        },
        UsbEndpoint {
            number: 1,
            is_in: false,
            bulk: true,
            packet_size: 64,
        },
    ]
}

/// The fake browser with one simulated stick (granted to the site).
#[derive(Clone)]
pub struct FakeUsb {
    pub world: Rc<RefCell<World>>,
    pub time: Rc<VirtualTime>,
}

impl FakeUsb {
    /// A stick that hears sensors with the made-up test key.
    pub fn new(sensors: Vec<SimSensor>) -> Self {
        let mut radio = SimRadio::new(Some(NetworkKey::TEST));
        for s in sensors {
            radio.add(s);
        }
        Self {
            world: Rc::new(RefCell::new(World {
                radio,
                plugged: true,
                generation: 0,
                denied: false,
                busy: false,
                endpoints: usbm_endpoints(),
                changed: false,
                stall_in: false,
                fail_out: false,
                pending: Vec::new(),
                opened: false,
                opens: 0,
                closes: 0,
                resets: 0,
                clear_halts: Vec::new(),
                writes: 0,
            })),
            time: Rc::new(VirtualTime::default()),
        }
    }

    pub fn timer(&self) -> FakeTimer {
        FakeTimer(self.time.clone())
    }

    /// Unplugged: `disconnect`, pending transfers reject.
    pub fn unplug(&self) {
        let mut w = self.world.borrow_mut();
        w.plugged = false;
        w.opened = false;
        w.generation += 1;
        w.changed = true;
        w.pending.clear();
    }

    /// Plugged in again (`connect`): a fresh stick, still granted.
    pub fn plug(&self) {
        let mut w = self.world.borrow_mut();
        let sensors = w.radio.sensors().to_vec();
        w.radio = SimRadio::new(Some(NetworkKey::TEST));
        for s in sensors {
            w.radio.add(s);
        }
        w.plugged = true;
        w.changed = true;
    }
}

pub struct FakeStick {
    world: Rc<RefCell<World>>,
    time: Rc<VirtualTime>,
    generation: u32,
}

impl FakeStick {
    fn alive(&self) -> Result<(), DeviceError> {
        let w = self.world.borrow();
        if !w.plugged || w.generation != self.generation {
            return Err(DeviceError::NoStick);
        }
        Ok(())
    }
}

impl UsbStick for FakeStick {
    fn name(&self) -> String {
        stick_name(USB_PRODUCT_USBM).to_owned()
    }

    fn open(&self) -> LocalBoxFuture<'static, Result<Vec<UsbEndpoint>, DeviceError>> {
        let r = self.alive().and_then(|()| {
            let mut w = self.world.borrow_mut();
            if w.denied {
                Err(DeviceError::StickPermission)
            } else if w.busy {
                Err(DeviceError::StickBusy)
            } else {
                w.opened = true;
                w.opens += 1;
                Ok(w.endpoints.clone())
            }
        });
        async move { r }.boxed_local()
    }

    fn transfer_in(&self, endpoint: u8, length: u32) -> LocalBoxFuture<'static, InResult> {
        let world = self.world.clone();
        let time = self.time.clone();
        let generation = self.generation;
        futures::future::poll_fn(move |_| {
            let mut w = world.borrow_mut();
            if !w.plugged || w.generation != generation {
                return Poll::Ready(Err(DeviceError::NoStick));
            }
            assert_eq!(endpoint, 1, "the IN endpoint");
            if std::mem::take(&mut w.stall_in) {
                return Poll::Ready(Ok((TransferStatus::Stall, Vec::new())));
            }
            w.radio.step(time.now());
            let more = w.radio.take_output();
            w.pending.extend(more);
            if w.pending.is_empty() {
                // The service polls again after its short sleep.
                return Poll::Pending;
            }
            let n = w.pending.len().min(length as usize);
            Poll::Ready(Ok((TransferStatus::Ok, w.pending.drain(..n).collect())))
        })
        .boxed_local()
    }

    fn transfer_out(
        &self,
        endpoint: u8,
        data: Vec<u8>,
    ) -> LocalBoxFuture<'static, Result<TransferStatus, DeviceError>> {
        let r = self.alive().and_then(|()| {
            let mut w = self.world.borrow_mut();
            assert_eq!(endpoint, 1, "the OUT endpoint");
            if std::mem::take(&mut w.fail_out) {
                return Err(DeviceError::Ant(
                    "NetworkError: A transfer error has occurred.".into(),
                ));
            }
            w.writes += 1;
            let now = self.time.now();
            w.radio.host(&data, now);
            Ok(TransferStatus::Ok)
        });
        async move { r }.boxed_local()
    }

    fn clear_halt(
        &self,
        is_in: bool,
        endpoint: u8,
    ) -> LocalBoxFuture<'static, Result<(), DeviceError>> {
        let r = self.alive().map(|()| {
            self.world.borrow_mut().clear_halts.push((is_in, endpoint));
        });
        async move { r }.boxed_local()
    }

    fn reset(&self) -> LocalBoxFuture<'static, Result<(), DeviceError>> {
        let r = self.alive().map(|()| {
            self.world.borrow_mut().resets += 1;
        });
        async move { r }.boxed_local()
    }

    fn close(&self) -> LocalBoxFuture<'static, ()> {
        let mut w = self.world.borrow_mut();
        w.opened = false;
        w.closes += 1;
        async {}.boxed_local()
    }
}

impl UsbHost for FakeUsb {
    type Stick = FakeStick;

    fn sticks(&self) -> LocalBoxFuture<'static, Vec<FakeStick>> {
        let w = self.world.borrow();
        let list = if w.plugged {
            vec![FakeStick {
                world: self.world.clone(),
                time: self.time.clone(),
                generation: w.generation,
            }]
        } else {
            Vec::new()
        };
        async move { list }.boxed_local()
    }

    fn take_changed(&self) -> bool {
        std::mem::take(&mut self.world.borrow_mut().changed)
    }
}

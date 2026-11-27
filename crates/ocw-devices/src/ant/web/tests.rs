//! The WebUSB link against a fake `navigator.usb` over the simulated stick
//! (spec 14 Phase 17 A5, 17B: no hardware), in virtual time.

use std::sync::mpsc::Receiver;
use std::time::Duration;

use futures::executor::LocalPool;
use futures::task::LocalSpawnExt;

use super::fake::{FakeUsb, drive};
use super::*;
use crate::ant::sim::SimSensor;

const S: fn(u64) -> Duration = Duration::from_secs;

/// The service on a pool over the fake, the page's handle.
struct Page {
    pool: LocalPool,
    usb: FakeUsb,
    stick: WebAntStick,
}

impl Page {
    fn new(key: Option<NetworkKey>, sensors: Vec<SimSensor>) -> Self {
        let usb = FakeUsb::new(sensors);
        let (stick, service) = WebAntStick::new(key);
        let pool = LocalPool::new();
        let (host, timer) = (usb.clone(), usb.timer());
        let time = usb.time.clone();
        pool.spawner()
            .spawn_local(async move { service.run(&host, &timer, move || time.now()).await })
            .expect("spawned");
        Self { pool, usb, stick }
    }

    /// Lets `d` of virtual time pass.
    fn run(&mut self, d: Duration) {
        let until = self.usb.time.now() + d;
        drive(&mut self.pool, &self.usb.time, until);
    }
}

fn events(rx: &Receiver<DeviceEvent>) -> Vec<DeviceEvent> {
    rx.try_iter().collect()
}

fn bpm(e: &[DeviceEvent]) -> Vec<f64> {
    e.iter()
        .filter_map(|e| match e {
            DeviceEvent::HeartRate { bpm, .. } => Some(*bpm),
            _ => None,
        })
        .collect()
}

fn watts(e: &[DeviceEvent]) -> Vec<f64> {
    e.iter()
        .filter_map(|e| match e {
            DeviceEvent::Power { watts, .. } => Some(*watts),
            _ => None,
        })
        .collect()
}

fn failed(reason: FailReason) -> DeviceEvent {
    DeviceEvent::Failed { reason }
}

#[test]
fn the_chooser_asks_for_the_ant_sticks_only() {
    assert_eq!(
        request_options(),
        r#"{"filters":[{"vendorId":4047,"productId":4104},{"vendorId":4047,"productId":4105}]}"#
    );
    assert!(is_stick(0x0fcf, 0x1009) && is_stick(0x0fcf, 0x1008));
    assert!(!is_stick(0x0fcf, 0x1004), "the old serial USB1");
    assert!(!is_stick(0x1234, 0x1009));
    assert_eq!(TransferStatus::parse("ok"), TransferStatus::Ok);
    assert_eq!(TransferStatus::parse("stall"), TransferStatus::Stall);
    assert_eq!(TransferStatus::parse("babble"), TransferStatus::Babble);
}

#[test]
fn the_bulk_endpoints_come_from_the_interface() {
    let ep = |number, is_in, bulk, packet_size| UsbEndpoint {
        number,
        is_in,
        bulk,
        packet_size,
    };
    // The USB-m: IN 1 / OUT 1.
    assert_eq!(
        bulk_endpoints(&fake::usbm_endpoints()),
        Some(Endpoints {
            ep_in: 1,
            ep_out: 1,
            in_length: 64
        })
    );
    // Another layout (an interrupt endpoint first, other numbers).
    let other = [
        ep(3, true, false, 8),
        ep(2, false, true, 64),
        ep(4, true, true, 32),
    ];
    assert_eq!(
        bulk_endpoints(&other),
        Some(Endpoints {
            ep_in: 4,
            ep_out: 2,
            in_length: 64
        })
    );
    assert_eq!(bulk_endpoints(&[ep(1, true, true, 64)]), None);
}

#[test]
fn a_scan_finds_a_strap_and_a_power_meter_and_both_ride() {
    let mut p = Page::new(
        Some(NetworkKey::TEST),
        vec![
            SimSensor::strap(1234, 141.0),
            SimSensor::power_meter(5678, 247.0, 88.0),
        ],
    );
    p.run(S(1));
    // Nothing needs the stick yet: found, not opened.
    assert_eq!(
        p.stick.status(),
        StickStatus::Found("ANT USB-m stick".into())
    );
    assert_eq!(p.usb.world.borrow().opens, 0);
    let found = p.stick.scan(S(10)).unwrap();
    assert!(p.stick.scanning());
    p.run(S(11));
    let mut ids: Vec<String> = found.try_iter().map(|d| d.id).collect();
    ids.sort();
    assert_eq!(ids, ["ant:11:5678:5", "ant:120:1234:1"]);
    assert!(!p.stick.scanning(), "the scan ended");
    assert_eq!(p.usb.world.borrow().opens, 1);
    // Both connected: heart rate and power arrive.
    let hr = p
        .stick
        .connect("ant:120:1234:1", "ANT+ heart rate 1234")
        .unwrap();
    let pm = p.stick.connect("ant:11:5678:5", "ANT+ power 5678").unwrap();
    p.run(S(5));
    let e = events(&hr.events);
    assert_eq!(
        e[0],
        DeviceEvent::Connected {
            kind: "heart-rate sensor (ANT+)"
        }
    );
    let b = bpm(&e);
    assert!(b.len() >= 12 && b.iter().all(|b| *b == 141.0), "{b:?}");
    let w = watts(&events(&pm.events));
    assert!(w.len() >= 8, "{w:?}");
    assert!(w.iter().any(|w| (*w - 247.0).abs() < 1.0), "{w:?}");
    assert_eq!(
        p.stick.status(),
        StickStatus::Ready("ANT USB-m stick".into())
    );
    // Still open after the scan (it was in use): one open only.
    assert_eq!(p.usb.world.borrow().opens, 1);
    // The handles go: the stick is closed a little later.
    hr.commands.send(DeviceCommand::Shutdown).unwrap();
    drop(pm);
    p.run(IDLE_CLOSE + S(1));
    let w = p.usb.world.borrow();
    assert!(!w.opened && w.closes == 1);
    drop(w);
    p.run(FIND_EVERY);
    assert_eq!(
        p.stick.status(),
        StickStatus::Found("ANT USB-m stick".into())
    );
    p.stick.shutdown();
    p.run(S(1));
}

#[test]
fn unplugged_and_plugged_in_again_the_sensors_come_back() {
    let mut p = Page::new(Some(NetworkKey::TEST), vec![SimSensor::strap(77, 120.0)]);
    let hr = p.stick.connect("ant:120:77:1", "strap").unwrap();
    p.run(S(3));
    assert!(!bpm(&events(&hr.events)).is_empty());
    // `disconnect`: the pending transferIn rejects.
    p.usb.unplug();
    p.run(S(1));
    let e = events(&hr.events);
    assert_eq!(e, [failed(FailReason::NoStick), DeviceEvent::Disconnected]);
    assert_eq!(p.stick.status(), StickStatus::NoStick);
    // Still away: told once only.
    p.run(S(10));
    assert!(events(&hr.events).is_empty());
    // `connect`: found again with getDevices, opened at once (no wait).
    p.usb.plug();
    p.run(S(3));
    let e = events(&hr.events);
    assert!(matches!(e[0], DeviceEvent::Connected { .. }), "{e:?}");
    assert!(bpm(&e).iter().all(|b| *b == 120.0) && !bpm(&e).is_empty());
    assert_eq!(p.usb.world.borrow().opens, 2);
    assert_eq!(p.usb.world.borrow().resets, 0, "an unplug is no failure");
}

#[test]
fn without_a_key_nothing_is_opened_until_one_is_typed() {
    let mut p = Page::new(None, vec![SimSensor::strap(5, 99.0)]);
    let hr = p.stick.connect("ant:120:5:1", "strap").unwrap();
    p.run(S(2));
    assert_eq!(events(&hr.events)[0], failed(FailReason::NoKey));
    assert_eq!(p.stick.status(), StickStatus::NoKey);
    assert!(!p.stick.has_key());
    assert!(matches!(p.stick.scan(S(5)), Err(DeviceError::NoNetworkKey)));
    assert_eq!(p.usb.world.borrow().opens, 0);
    p.stick.set_key(Some(NetworkKey::TEST));
    p.run(S(4));
    let e = events(&hr.events);
    assert!(bpm(&e).contains(&99.0), "{e:?}");
    // A wrong key: the stick is set up again and hears nothing.
    p.stick.set_key(Some(NetworkKey::new([7; 8])));
    p.run(S(1));
    events(&hr.events);
    p.run(S(3));
    assert!(bpm(&events(&hr.events)).iter().all(|b| *b == 0.0));
    // The key removed: the stick is closed.
    p.stick.set_key(None);
    p.run(S(1));
    assert!(!p.usb.world.borrow().opened);
    assert_eq!(p.stick.status(), StickStatus::NoKey);
}

#[test]
fn a_stick_without_permission_or_busy_says_why() {
    let mut p = Page::new(Some(NetworkKey::TEST), vec![SimSensor::strap(5, 99.0)]);
    p.usb.world.borrow_mut().denied = true;
    let hr = p.stick.connect("ant:120:5:1", "strap").unwrap();
    p.run(S(1));
    assert_eq!(
        p.stick.status(),
        StickStatus::NoPermission("ANT USB-m stick".into())
    );
    assert_eq!(events(&hr.events)[0], failed(FailReason::NoPermission));
    {
        let mut w = p.usb.world.borrow_mut();
        w.denied = false;
        w.busy = true;
    }
    p.run(RECONNECT_DELAY + S(1));
    assert_eq!(
        p.stick.status(),
        StickStatus::Busy("ANT USB-m stick".into())
    );
    assert_eq!(events(&hr.events)[0], failed(FailReason::StickBusy));
    // The other program lets go: tried again after the delay.
    p.usb.world.borrow_mut().busy = false;
    p.run(RECONNECT_DELAY + S(2));
    assert!(bpm(&events(&hr.events)).contains(&99.0));
}

#[test]
fn a_stall_is_cleared_and_a_failed_transfer_resets_the_stick() {
    let mut p = Page::new(Some(NetworkKey::TEST), vec![SimSensor::strap(9, 133.0)]);
    let hr = p.stick.connect("ant:120:9:1", "strap").unwrap();
    p.run(S(2));
    events(&hr.events);
    p.usb.world.borrow_mut().stall_in = true;
    p.run(S(2));
    assert_eq!(p.usb.world.borrow().clear_halts, [(true, 1)]);
    assert!(bpm(&events(&hr.events)).contains(&133.0), "goes on");
    // A transfer error: reset, closed, opened again after the delay.
    p.usb.world.borrow_mut().fail_out = true;
    // The next write: a reopened channel after a short search is enough,
    // so force one by asking for a second sensor.
    let other = p.stick.connect("ant:120:10:1", "other").unwrap();
    p.run(S(1));
    {
        let w = p.usb.world.borrow();
        assert_eq!(w.resets, 1);
        assert!(!w.opened);
    }
    assert_eq!(
        events(&hr.events),
        [failed(FailReason::NoStick), DeviceEvent::Disconnected]
    );
    p.run(RECONNECT_DELAY + S(3));
    assert!(bpm(&events(&hr.events)).contains(&133.0));
    assert_eq!(p.usb.world.borrow().opens, 2);
    drop(other);
}

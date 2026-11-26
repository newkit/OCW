//! Hardware-in-the-loop tool for ANT USB sticks and ANT+ sensors (spec 14
//! Phase 17, A4 – postponed until a stick is there).
//!
//! ```text
//! cargo run -p ocw-devices --features ant-usb --example ant -- sticks
//! cargo run -p ocw-devices --features ant-usb --example ant -- scan [SECONDS]
//! cargo run -p ocw-devices --features ant-usb --example ant -- monitor <ID> [SECONDS]
//! ```
//!
//! `ID` is `ant:120:<device number>:<transmission type>` as `scan` prints
//! it. The ANT+ network key is read from the file `OCW_ANT_KEY_FILE` names
//! (16 hex digits, e.g. the `network_key` line of `secrets.toml` copied
//! into its own file); it is never printed and never in this repository.
//! `--sim` uses the simulated stick (any key) instead of a real one. On
//! Windows the stick is opened as the desktop app does: WinUSB (`nusb`)
//! first, then Garmin's libusb-win32 driver (`libusb0.dll`; Q7 (d)), and
//! `sticks` names both routes.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use ocw_devices::ant::sim::SimSensor;
use ocw_devices::ant::usb::UsbTransport;
use ocw_devices::ant::{AntStick, AntTransport, NetworkKey, SimTransport};

/// The real sticks, as the desktop app opens them.
#[cfg(windows)]
fn real() -> impl AntTransport {
    ocw_devices::ant::Fallback::new(
        UsbTransport::new(),
        ocw_devices::ant::libusb0::Libusb0Transport::system(),
    )
}

/// The real sticks, as the desktop app opens them.
#[cfg(not(windows))]
fn real() -> impl AntTransport {
    UsbTransport::new()
}

fn key() -> Option<NetworkKey> {
    let path = std::env::var_os("OCW_ANT_KEY_FILE")?;
    let text = std::fs::read_to_string(&path).ok()?;
    let digits = text
        .split('=')
        .next_back()
        .unwrap_or_default()
        .trim()
        .trim_matches('"');
    match NetworkKey::parse(digits) {
        Ok(k) => Some(k),
        Err(e) => {
            eprintln!("OCW_ANT_KEY_FILE: {e}");
            None
        }
    }
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let sim = args.iter().any(|a| a == "--sim");
    args.retain(|a| a != "--sim");
    let secs = |i: usize, default: u64| {
        args.get(i)
            .and_then(|s| s.parse().ok())
            .map_or(Duration::from_secs(default), Duration::from_secs)
    };
    let stick = |key| {
        if sim {
            AntStick::start(
                SimTransport::new(
                    None,
                    vec![
                        SimSensor::strap(12345, 142.0),
                        SimSensor::power_meter(4004, 230.0, 88.0),
                    ],
                ),
                key,
            )
        } else {
            AntStick::start(real(), key)
        }
    };
    match args.first().map(String::as_str) {
        Some("sticks") => {
            match UsbTransport::new().find() {
                Some(name) => println!("{name} (nusb)"),
                None => println!("no ANT stick plugged in (nusb)"),
            }
            #[cfg(windows)]
            match ocw_devices::ant::libusb0::Libusb0Transport::system().find() {
                Some(name) => println!("{name} (Garmin's libusb-win32 driver)"),
                None => println!("no ANT stick on Garmin's libusb-win32 driver"),
            }
        }
        Some("scan") => {
            let Some(k) = key().or(sim.then_some(NetworkKey::TEST)) else {
                eprintln!("set OCW_ANT_KEY_FILE to a file with the ANT+ network key");
                std::process::exit(2);
            };
            let s = stick(Some(k));
            let stop = AtomicBool::new(false);
            let r = s.scan(secs(1, 10), &stop, &mut |d| {
                println!(
                    "{:<26} rssi {:?} id {}",
                    d.name.unwrap_or_default(),
                    d.rssi,
                    d.id
                );
            });
            if let Err(e) = r {
                eprintln!("scan: {e}");
            }
            s.shutdown();
        }
        Some("monitor") => {
            let Some(id) = args.get(1) else {
                eprintln!("monitor <ID> [SECONDS]");
                std::process::exit(2);
            };
            let Some(k) = key().or(sim.then_some(NetworkKey::TEST)) else {
                eprintln!("set OCW_ANT_KEY_FILE to a file with the ANT+ network key");
                std::process::exit(2);
            };
            let s = stick(Some(k));
            let h = match s.connect(id, id) {
                Ok(h) => h,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            let until = Instant::now() + secs(2, 30);
            while Instant::now() < until {
                if let Ok(e) = h.events.recv_timeout(Duration::from_millis(200)) {
                    println!("{e:?}");
                }
            }
            println!("stick: {}", s.status().text());
            s.shutdown();
        }
        _ => {
            eprintln!("usage: ant sticks | scan [SECONDS] | monitor <ID> [SECONDS] [--sim]");
            std::process::exit(2);
        }
    }
}

//! Hardware-in-the-loop tool for Bluetooth trainers and sensors (spec §14,
//! Phase 2 integration tests).
//!
//! ```text
//! cargo run -p ocw-devices --example ble -- scan [SECONDS]
//! cargo run -p ocw-devices --example ble -- monitor <ID> [SECONDS]
//! cargo run -p ocw-devices --example ble -- ride <ID> [SECONDS]
//! cargo run -p ocw-devices --example ble -- services <ID>
//! ```
//!
//! `scan` and `monitor` only read. `ride` also controls the trainer: SIM
//! gradients 0 %, 2 %, 4 %, 0 %, then ERG 120 W (gentle values; harmless
//! without a rider).

use std::time::{Duration, Instant};

use ocw_devices::ble::{self, BleDevice};
use ocw_devices::{DeviceBackend, DeviceCommand, DeviceEvent, SimulationParameters};

fn main() {
    tracing_subscriber_init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let secs = |i: usize, default: u64| {
        args.get(i)
            .and_then(|s| s.parse().ok())
            .map_or(Duration::from_secs(default), Duration::from_secs)
    };
    match args.first().map(String::as_str) {
        Some("scan") => match ble::scan(secs(1, 8)) {
            Ok(found) if found.is_empty() => println!("no fitness devices found"),
            Ok(found) => {
                for d in found {
                    println!(
                        "{:<28} {:?} rssi {:?} id {}",
                        d.name.unwrap_or_default(),
                        d.profiles,
                        d.rssi,
                        d.id
                    );
                }
            }
            Err(e) => {
                eprintln!("scan failed: {e}");
                std::process::exit(1);
            }
        },
        Some("services") => services(args.get(1).expect("device id")),
        Some(mode @ ("monitor" | "ride")) => {
            let id = args.get(1).expect("device id").clone();
            run(&id, mode == "ride", secs(2, 20));
        }
        _ => {
            eprintln!("usage: ble scan [SECONDS] | monitor <ID> [SECONDS] | ride <ID> [SECONDS]");
            std::process::exit(2);
        }
    }
}

fn run(id: &str, control: bool, duration: Duration) {
    let handle = BleDevice::new(id, id).start().expect("start backend");
    let start = Instant::now();
    // SIM plan: (seconds after start, gradient); then ERG at the end.
    let plan = [(2.0, 0.0), (6.0, 0.02), (10.0, 0.04), (14.0, 0.0)];
    let mut next = 0;
    let mut erg_sent = false;
    let (mut samples, mut hr) = (0u32, 0u32);
    while start.elapsed() < duration {
        let t = start.elapsed().as_secs_f64();
        if control {
            if let Some(&(at, grade)) = plan.get(next)
                && t >= at
            {
                println!("{t:6.1} s  -> SIM {:.1} %", grade * 100.0);
                let _ = handle
                    .commands
                    .send(DeviceCommand::SetSimulation(SimulationParameters {
                        grade,
                        wind_speed_mps: 0.0,
                        crr: 0.004,
                        cw_kg_per_m: 0.2,
                        mass_kg: 83.0,
                    }));
                next += 1;
            }
            if !erg_sent && t >= 18.0 {
                println!("{t:6.1} s  -> ERG 120 W");
                let _ = handle.commands.send(DeviceCommand::SetTargetPower(120.0));
                erg_sent = true;
            }
        }
        while let Ok(e) = handle.events.try_recv() {
            match e {
                DeviceEvent::Power {
                    watts, cadence_rpm, ..
                } => {
                    samples += 1;
                    if samples % 4 == 1 {
                        println!("{t:6.1} s  power {watts:4.0} W  cadence {cadence_rpm:?}");
                    }
                }
                DeviceEvent::HeartRate { bpm, .. } => {
                    hr += 1;
                    if hr % 4 == 1 {
                        println!("{t:6.1} s  heart rate {bpm:.0}");
                    }
                }
                DeviceEvent::Disconnected => println!("{t:6.1} s  disconnected (reconnecting)"),
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = handle.commands.send(DeviceCommand::Shutdown);
    println!("done: {samples} power samples, {hr} heart-rate samples");
}

/// Lists all GATT services and characteristics (including vendor-specific ones).
fn services(id: &str) {
    use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter};
    use btleplug::platform::Manager;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    rt.block_on(async {
        let manager = Manager::new().await.expect("manager");
        let adapter = manager
            .adapters()
            .await
            .expect("adapters")
            .into_iter()
            .next()
            .expect("adapter");
        adapter
            .start_scan(ScanFilter::default())
            .await
            .expect("scan");
        let deadline = Instant::now() + Duration::from_secs(20);
        let p = loop {
            if let Some(p) = adapter
                .peripherals()
                .await
                .expect("peripherals")
                .into_iter()
                .find(|p| p.id().to_string() == id)
            {
                break p;
            }
            assert!(Instant::now() < deadline, "device not found");
            tokio::time::sleep(Duration::from_millis(250)).await;
        };
        let _ = adapter.stop_scan().await;
        p.connect().await.expect("connect");
        p.discover_services().await.expect("discover");
        for s in p.services() {
            println!("service {}", s.uuid);
            for c in &s.characteristics {
                println!("    {} {:?}", c.uuid, c.properties);
            }
        }
        let _ = p.disconnect().await;
    });
}

/// Logs to stderr (warnings from the protocol, e.g. refused requests).
fn tracing_subscriber_init() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .try_init();
}

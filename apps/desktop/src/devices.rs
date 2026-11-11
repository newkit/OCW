//! Devices of the desktop client (spec §3.4, §5.3): virtual trainer or
//! Bluetooth trainers/sensors, remembered devices, scanning, and the SIM/ERG
//! trainer control loop. Console commands:
//!
//! `devices.scan [seconds]`, `devices.connect <n|id>`, `devices.list`,
//! `devices.forget <id|all>`, `devices.virtual [W]`, `trainer.mode
//! <sim|erg|off>`, `trainer.difficulty <0-100>`, `erg.target <W>`,
//! `sim.power <W>` (virtual trainer, Dev).

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Duration;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_core::RiderProfile;
use ocw_devices::ble::{self, BleDevice, FoundDevice};
use ocw_devices::{
    DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle, VirtualTrainer,
};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use crate::Args;
use crate::connection::{ClientState, Link};
use crate::known_devices::{self, KnownDevice, KnownDevices};
use crate::trainer::{TrainerControl, TrainerMode};

pub struct DevicesPlugin;

impl Plugin for DevicesPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        let known_path = args
            .devices_file
            .clone()
            .or_else(known_devices::default_path);
        let known = match &known_path {
            Some(p) => KnownDevices::load(p).unwrap_or_else(|e| {
                warn!("cannot read remembered devices: {e}");
                KnownDevices::default()
            }),
            None => KnownDevices::default(),
        };
        let mut devices = Devices {
            handles: Mutex::new(Vec::new()),
            known,
            known_path,
            scan: Mutex::new(None),
            found: Vec::new(),
        };
        if args.virtual_trainer || devices.known.devices.is_empty() {
            devices.start(VirtualTrainer::new(args.power));
        } else {
            for d in devices.known.devices.clone() {
                info!(name = %d.name, id = %d.id, "connecting remembered device");
                devices.start(BleDevice::new(d.id, d.name));
            }
        }
        app.insert_resource(devices)
            .insert_resource(Trainer(TrainerControl::new(RiderProfile::default())))
            .add_systems(PreUpdate, poll_devices)
            .add_systems(Update, (poll_scan, control_trainer, publish_status))
            .add_systems(Last, shutdown_on_exit);
        register_commands(app);
    }
}

#[derive(Resource)]
pub struct Trainer(pub TrainerControl);

type ScanResult = Result<Vec<FoundDevice>, DeviceError>;

/// Running device backends and the remembered devices.
#[derive(Resource)]
pub struct Devices {
    handles: Mutex<Vec<DeviceHandle>>,
    known: KnownDevices,
    known_path: Option<PathBuf>,
    scan: Mutex<Option<Receiver<ScanResult>>>,
    /// Result of the last scan (for `devices.connect <n>`).
    found: Vec<FoundDevice>,
}

impl Devices {
    fn start(&mut self, backend: impl DeviceBackend) -> String {
        let name = backend.name().to_string();
        match backend.start() {
            Ok(h) => {
                self.handles.lock().expect("not poisoned").push(h);
                name
            }
            Err(e) => {
                error!("{name}: {e}");
                format!("{name}: {e}")
            }
        }
    }

    /// Sends a command to every running backend (each ignores what it does
    /// not support).
    pub fn command_all(&self, cmd: DeviceCommand) {
        for h in self.handles.lock().expect("not poisoned").iter() {
            let _ = h.commands.send(cmd);
        }
    }

    fn names(&self) -> Vec<String> {
        self.handles
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|h| h.name.clone())
            .collect()
    }

    fn save_known(&self) -> Result<(), String> {
        match &self.known_path {
            Some(p) => self.known.save(p),
            None => Err("no configuration directory".into()),
        }
    }
}

/// Reads all device events; power samples go to the server.
fn poll_devices(
    devices: Res<Devices>,
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
) {
    let mut events = Vec::new();
    for h in devices.handles.lock().expect("not poisoned").iter() {
        for e in h.events.try_iter() {
            events.push((h.name.clone(), e));
        }
    }
    for (name, event) in events {
        if event == DeviceEvent::Disconnected {
            console.print(format!("{name}: disconnected, reconnecting"));
        }
        if let Some(sample) = client.0.on_device_event(event) {
            link.send_bytes(sample);
        }
    }
}

/// SIM/ERG control: road gradient or target power to the trainer.
fn control_trainer(
    time: Res<Time>,
    client: Res<ClientState>,
    devices: Res<Devices>,
    mut trainer: ResMut<Trainer>,
) {
    let c = &client.0;
    if c.rider_id().is_none() {
        return;
    }
    if let Some(cmd) = trainer.0.update(
        time.elapsed_secs_f64(),
        c.track(),
        c.state(),
        c.cadence_rpm(),
    ) {
        devices.command_all(cmd);
    }
}

/// Collects the result of a background scan.
fn poll_scan(mut devices: ResMut<Devices>, mut console: ResMut<ConsoleState>) {
    let result = {
        let mut slot = devices.scan.lock().expect("not poisoned");
        let Some(rx) = slot.as_ref() else { return };
        match rx.try_recv() {
            Ok(r) => {
                *slot = None;
                r
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                *slot = None;
                Err(DeviceError::Bluetooth("scan thread ended".into()))
            }
        }
    };
    match result {
        Ok(found) if found.is_empty() => console.print("scan: no fitness devices found"),
        Ok(found) => {
            console.print(format!(
                "scan: {} device(s) (connect with devices.connect <n>)",
                found.len()
            ));
            for (i, d) in found.iter().enumerate() {
                console.print(format!(
                    "  {}: {} {:?} rssi {} id {}",
                    i + 1,
                    d.name.as_deref().unwrap_or("(no name)"),
                    d.profiles,
                    d.rssi.map_or("-".into(), |r| r.to_string()),
                    d.id
                ));
            }
            devices.found = found;
        }
        Err(e) => console.print(format!("scan failed: {e}")),
    }
}

fn publish_status(
    devices: Res<Devices>,
    trainer: Res<Trainer>,
    client: Res<ClientState>,
    mut view: ResMut<RideView>,
) {
    let brake = if client.0.braking() {
        " | BRAKE (up: ride on)"
    } else {
        ""
    };
    view.status = format!(
        "{} | {} | {}{brake}",
        devices.names().join(", "),
        trainer.0.status(),
        client.0.steering_status()
    );
}

fn shutdown_on_exit(mut exit: MessageReader<AppExit>, devices: Res<Devices>) {
    if exit.read().next().is_some() {
        devices.command_all(DeviceCommand::Shutdown);
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.scan")
            .about("Scan for Bluetooth trainers and sensors (runs in the background)")
            .arg(
                Arg::new("seconds")
                    .default_value("5")
                    .value_parser(value_parser!(u64).range(1..=60)),
            ),
        |world, m| {
            let secs = *m.get_one::<u64>("seconds").expect("has default");
            let devices = world.resource::<Devices>();
            let mut slot = devices.scan.lock().expect("not poisoned");
            if slot.is_some() {
                return Err("a scan is already running".into());
            }
            let (tx, rx) = mpsc::channel();
            std::thread::Builder::new()
                .name("ocw-ble-scan".into())
                .spawn(move || {
                    let _ = tx.send(ble::scan(Duration::from_secs(secs)));
                })
                .map_err(|e| e.to_string())?;
            *slot = Some(rx);
            Ok(format!("scanning for {secs} s ..."))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.connect")
            .about("Connect a device from the last scan (number) or by id, and remember it")
            .arg(Arg::new("device").required(true)),
        |world, m| {
            let which = m.get_one::<String>("device").expect("required").clone();
            let mut devices = world.resource_mut::<Devices>();
            let found = match which.parse::<usize>() {
                Ok(n) if n >= 1 => devices.found.get(n - 1).cloned(),
                _ => devices.found.iter().find(|d| d.id == which).cloned(),
            };
            let (id, name) = match found {
                Some(d) => {
                    let name = d.name.unwrap_or_else(|| d.id.clone());
                    (d.id, name)
                }
                // Unknown to the last scan: try the id as given.
                None if which.parse::<usize>().is_err() => (which.clone(), which),
                None => return Err(format!("no device {which} in the last scan")),
            };
            let started = devices.start(BleDevice::new(id.clone(), name.clone()));
            devices.known.remember(KnownDevice { id, name });
            let saved = match devices.save_known() {
                Ok(()) => "remembered".to_string(),
                Err(e) => format!("not remembered: {e}"),
            };
            Ok(format!("connecting {started} ({saved})"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.list").about("Show running and remembered devices"),
        |world, _| {
            let devices = world.resource::<Devices>();
            let mut out = format!("running: {}", devices.names().join(", "));
            out.push_str("\nremembered:");
            if devices.known.devices.is_empty() {
                out.push_str(" none");
            }
            for d in &devices.known.devices {
                out.push_str(&format!("\n  {} (id {})", d.name, d.id));
            }
            if let Some(p) = &devices.known_path {
                out.push_str(&format!("\nfile: {}", p.display()));
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.forget")
            .about("Forget a remembered device (id, or 'all'); takes effect at the next start")
            .arg(Arg::new("device").required(true)),
        |world, m| {
            let which = m.get_one::<String>("device").expect("required").clone();
            let mut devices = world.resource_mut::<Devices>();
            if which == "all" {
                devices.known.devices.clear();
            } else if !devices.known.forget(&which) {
                return Err(format!("{which} is not remembered"));
            }
            devices.save_known()?;
            Ok(format!("forgot {which}"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.virtual")
            .about("Start an additional virtual trainer")
            .arg(
                Arg::new("watts")
                    .default_value("200")
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let watts = *m.get_one::<f64>("watts").expect("has default");
            let name = world
                .resource_mut::<Devices>()
                .start(VirtualTrainer::new(watts));
            Ok(format!("started {name}"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("trainer.mode")
            .about("Trainer control: sim (simulate the road), erg (hold a power), off")
            .arg(
                Arg::new("mode")
                    .required(true)
                    .value_parser(["sim", "erg", "off"]),
            ),
        |world, m| {
            let mut trainer = world.resource_mut::<Trainer>();
            let mode = match m.get_one::<String>("mode").expect("required").as_str() {
                "sim" => TrainerMode::Sim,
                "off" => TrainerMode::Off,
                _ => match trainer.0.mode() {
                    erg @ TrainerMode::Erg { .. } => erg,
                    _ => TrainerMode::Erg { target_w: 150.0 },
                },
            };
            trainer.0.set_mode(mode);
            Ok(format!("trainer mode {mode}"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("trainer.difficulty")
            .about("Trainer difficulty in percent (scales the road gradient in SIM mode)")
            .arg(
                Arg::new("percent")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let p = *m.get_one::<f64>("percent").expect("required");
            if !(0.0..=100.0).contains(&p) {
                return Err("difficulty must be between 0 and 100 %".into());
            }
            world.resource_mut::<Trainer>().0.set_difficulty(p / 100.0);
            Ok(format!("trainer difficulty {p:.0} %"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("erg.target")
            .about("ERG mode with this target power in watts")
            .arg(
                Arg::new("watts")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let watts = *m.get_one::<f64>("watts").expect("required");
            if !(0.0..=2000.0).contains(&watts) {
                return Err("target must be between 0 and 2000 W".into());
            }
            world
                .resource_mut::<Trainer>()
                .0
                .set_mode(TrainerMode::Erg { target_w: watts });
            Ok(format!("trainer mode ERG {watts:.0} W"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("sim.power")
            .about("Set the virtual trainer's power in watts")
            .arg(
                Arg::new("watts")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let watts = *m.get_one::<f64>("watts").expect("required");
            if !(0.0..=2500.0).contains(&watts) {
                return Err("power must be between 0 and 2500 W".into());
            }
            world
                .resource::<Devices>()
                .command_all(DeviceCommand::SetVirtualPower(watts));
            Ok(format!("virtual power {watts:.0} W"))
        },
    ));
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;

    use super::*;
    use crate::connection::ConnectionPlugin;

    fn app(extra: &[&str]) -> App {
        let mut args = vec!["ocw-desktop", "--bots", "0", "--virtual"];
        args.extend_from_slice(extra);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Args::parse_from(args))
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins((
                ConnectionPlugin {
                    route: Route::test_loop(),
                },
                DevicesPlugin,
            ));
        app
    }

    fn run(app: &mut App, line: &str) -> String {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world()
            .resource::<ConsoleState>()
            .log()
            .last()
            .cloned()
            .unwrap_or_default()
    }

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-devices-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("devices.toml")
    }

    #[test]
    fn trainer_commands_change_the_mode() {
        let mut app = app(&[]);
        assert_eq!(run(&mut app, "erg.target 230"), "trainer mode ERG 230 W");
        assert_eq!(
            app.world().resource::<Trainer>().0.mode(),
            TrainerMode::Erg { target_w: 230.0 }
        );
        // erg keeps the current target.
        assert_eq!(run(&mut app, "trainer.mode erg"), "trainer mode ERG 230 W");
        assert_eq!(run(&mut app, "trainer.mode off"), "trainer mode off");
        assert!(run(&mut app, "trainer.difficulty 150").starts_with("error"));
        assert_eq!(
            run(&mut app, "trainer.difficulty 80"),
            "trainer difficulty 80 %"
        );
        assert_eq!(app.world().resource::<Trainer>().0.difficulty(), 0.8);
        assert!(run(&mut app, "erg.target 5000").starts_with("error"));
    }

    #[test]
    fn status_line_shows_devices_and_trainer() {
        let mut app = app(&[]);
        for _ in 0..3 {
            app.update();
        }
        let status = app.world().resource::<RideView>().status.clone();
        assert!(status.starts_with("Virtual trainer"), "{status}");
        assert!(status.contains("trainer SIM"), "{status}");
    }

    #[test]
    fn remembered_devices_are_listed_and_forgotten() {
        let path = temp_file("forget");
        let mut known = KnownDevices::default();
        known.remember(KnownDevice {
            id: "hci0/dev_AA".into(),
            name: "KICKR".into(),
        });
        known.save(&path).unwrap();
        // --virtual: remembered devices are not connected, only listed.
        let mut app = app(&["--devices-file", path.to_str().unwrap()]);
        let list = {
            app.world_mut()
                .resource_mut::<ConsoleState>()
                .queue("devices.list");
            app.update();
            app.world().resource::<ConsoleState>().log().join("\n")
        };
        assert!(list.contains("KICKR (id hci0/dev_AA)"), "{list}");
        assert_eq!(
            run(&mut app, "devices.forget hci0/dev_AA"),
            "forgot hci0/dev_AA"
        );
        assert!(KnownDevices::load(&path).unwrap().devices.is_empty());
        assert!(run(&mut app, "devices.forget nope").starts_with("error"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn connect_needs_a_scan_result_for_numbers() {
        let mut app = app(&["--devices-file", temp_file("connect").to_str().unwrap()]);
        assert!(run(&mut app, "devices.connect 1").starts_with("error: no device 1"));
    }
}

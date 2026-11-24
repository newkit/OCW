//! Devices of the desktop client (spec §3.4, §5.3, §3.2.1 *Devices*):
//! virtual trainer or Bluetooth trainers/sensors, remembered devices,
//! scanning, and the SIM/ERG trainer control loop. Console commands:
//!
//! `devices.scan [seconds] [--stop]`, `devices.connect <n|id>`,
//! `devices.list`, `devices.forget <id|name|all>`, `devices.virtual [W]`,
//! `trainer.mode <sim|erg|off>`, `trainer.difficulty <0-100>`,
//! `erg.target <W>`, `sim.power <W>` (virtual trainer, Dev).
//!
//! The menu's Settings → Trainer → Devices page (`ocw_client::menu::
//! devices`, shared with the browser) runs the same commands; this module
//! fills its [`ocw_ui::menu::DevicesMenu`] every frame:
//! - a scan lists each device **as it appears** (`ble::scan_live`), with
//!   its role (trainer, power, cadence, heart rate) and signal;
//! - choosing one connects it; a trainer or power meter **replaces the
//!   virtual trainer** once it is connected (forgetting the last one
//!   brings the virtual trainer back);
//! - a device is **remembered in `devices.toml` once it connected**
//!   (`known_devices`: the default file, or `--devices-file`), so the next
//!   start reconnects it; a failed attempt says why.
//!
//! The Bluetooth stack is behind [`DeviceIo`] (connect, scan), so the
//! tests run the whole flow with a mock.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command, value_parser};
use ocw_client::menu::devices::{DevicesInfo, Entry, Link, Role, Scan};
use ocw_devices::ble::{self, BleDevice, FoundDevice};
use ocw_devices::{
    DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle, VirtualTrainer,
};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use crate::Args;
use crate::connection::{ClientState, Link as ServerLink};
use crate::known_devices::{self, KnownDevice, KnownDevices};
use crate::trainer::{TrainerControl, TrainerMode};

/// Starts a Bluetooth device backend (id, name).
pub type Connector = Box<dyn Fn(&str, &str) -> Result<DeviceHandle, DeviceError> + Send + Sync>;
/// Scans for up to the duration (blocking, on the scan thread), reporting
/// each device as it appears; the flag stops it early.
pub type Scanner = Arc<
    dyn Fn(Duration, &AtomicBool, &mut dyn FnMut(FoundDevice)) -> Result<(), DeviceError>
        + Send
        + Sync,
>;

/// The Bluetooth stack: btleplug in the app, a mock in tests.
#[derive(Resource)]
pub struct DeviceIo {
    pub connect: Connector,
    pub scan: Scanner,
}

impl DeviceIo {
    /// The real one (BlueZ, WinRT, CoreBluetooth through btleplug).
    pub fn bluetooth() -> Self {
        Self {
            connect: Box::new(|id, name| BleDevice::new(id, name).start()),
            scan: Arc::new(|d, stop, found| ble::scan_live(d, stop, found)),
        }
    }
}

pub struct DevicesPlugin;

impl Plugin for DevicesPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        if !app.world().contains_resource::<DeviceIo>() {
            app.insert_resource(DeviceIo::bluetooth());
        }
        let default = app
            .world()
            .get_resource::<crate::config::ConfigFiles>()
            .map(|f| f.section("devices"));
        let known_path = known_devices::choose_slot(args.devices_file.clone(), default);
        info!(path = ?known_path, "remembered devices");
        let known = match &known_path {
            Some(p) => KnownDevices::load_slot(p).unwrap_or_else(|e| {
                warn!("cannot read remembered devices: {e}");
                KnownDevices::default()
            }),
            None => KnownDevices::default(),
        };
        let mut devices = Devices::new(known, known_path, args.power);
        if args.virtual_trainer || devices.known.devices.is_empty() {
            if !args.virtual_trainer {
                warn!(
                    "no remembered devices: riding with the virtual trainer (Settings -> Trainer -> Devices, or devices.scan / devices.connect, to add one)"
                );
            }
            devices.start_virtual();
        } else {
            let io = app.world().resource::<DeviceIo>();
            for d in devices.known.devices.clone() {
                info!(name = %d.name, id = %d.id, "connecting remembered device");
                let _ = devices.connect(io, &d.id, &d.name);
            }
        }
        // The rider profile's weight + the bike: the SIM mass (§3.2.1); the
        // bike's profile also scales the rolling resistance off the asphalt.
        let profile = crate::settings::local_rider(app.world()).physics();
        app.insert_resource(devices)
            .insert_resource(Trainer(TrainerControl::new(profile)))
            .add_systems(PreUpdate, poll_devices)
            .add_systems(
                Update,
                (poll_scan, control_trainer, publish_status, publish_menu),
            )
            .add_systems(Last, shutdown_on_exit);
        register_commands(app);
    }
}

#[derive(Resource)]
pub struct Trainer(pub TrainerControl);

/// What the scan thread reports.
enum ScanMsg {
    Found(FoundDevice),
    Done(Result<(), DeviceError>),
}

/// A running scan.
struct ScanJob {
    rx: Mutex<Receiver<ScanMsg>>,
    stop: Arc<AtomicBool>,
    until: Instant,
}

/// A running backend.
struct Running {
    /// The Bluetooth id; `None`: the virtual trainer.
    id: Option<String>,
    role: Option<Role>,
    handle: DeviceHandle,
    link: Link,
}

impl Running {
    fn delivers_power(&self) -> bool {
        // A device of unknown role (connected by id) may be a trainer.
        self.id.is_some() && self.role.is_none_or(Role::delivers_power)
    }
}

/// A device the Devices page lists (in a stable order: remembered ones
/// first, then as scans find them).
#[derive(Debug, Clone)]
struct Listed {
    id: String,
    name: String,
    role: Role,
    rssi: Option<i16>,
}

/// Running device backends and the remembered devices.
#[derive(Resource)]
pub struct Devices {
    running: Mutex<Vec<Running>>,
    known: KnownDevices,
    known_path: Option<crate::config::Slot>,
    scan: Option<ScanJob>,
    /// The devices of the last scan, in the order found (for
    /// `devices.connect <n>`).
    found: Vec<FoundDevice>,
    listed: Vec<Listed>,
    /// The latest messages for the Devices page.
    info: DevicesInfo,
    /// The virtual trainer's power when it comes back.
    virtual_w: f64,
}

impl Devices {
    fn new(known: KnownDevices, known_path: Option<crate::config::Slot>, virtual_w: f64) -> Self {
        let listed = known
            .devices
            .iter()
            .map(|d| Listed {
                id: d.id.clone(),
                name: d.name.clone(),
                role: d
                    .kind
                    .as_deref()
                    .and_then(Role::parse)
                    .unwrap_or(Role::Trainer),
                rssi: None,
            })
            .collect();
        Self {
            running: Mutex::new(Vec::new()),
            known,
            known_path,
            scan: None,
            found: Vec::new(),
            listed,
            info: DevicesInfo::default(),
            virtual_w,
        }
    }

    fn running(&mut self) -> &mut Vec<Running> {
        self.running.get_mut().expect("not poisoned")
    }

    fn start_virtual(&mut self) -> String {
        let backend = VirtualTrainer::new(self.virtual_w);
        let name = backend.name().to_string();
        match backend.start() {
            Ok(handle) => {
                self.running().push(Running {
                    id: None,
                    role: Some(Role::Trainer),
                    handle,
                    link: Link::Connected("virtual".into()),
                });
                name
            }
            Err(e) => {
                error!("{name}: {e}");
                format!("{name}: {e}")
            }
        }
    }

    /// Starts the backend of a Bluetooth device (it connects in the
    /// background; [`poll_devices`] follows it).
    fn connect(&mut self, io: &DeviceIo, id: &str, name: &str) -> Result<String, String> {
        if let Some(r) = self.running().iter().find(|r| r.id.as_deref() == Some(id)) {
            return Ok(format!("{name}: already {}", r.link.text()));
        }
        let role = self.role_of(id);
        self.list(id, name, role, None);
        let handle = (io.connect)(id, name).map_err(|e| format!("{name}: {e}"))?;
        self.running().push(Running {
            id: Some(id.to_owned()),
            role,
            handle,
            link: Link::Connecting,
        });
        let line = format!("{name}: connecting ...");
        self.info.push_message(line.clone());
        Ok(line)
    }

    fn role_of(&self, id: &str) -> Option<Role> {
        self.found
            .iter()
            .find(|d| d.id == id)
            .and_then(|d| Role::of(d.name.as_deref(), &d.profiles))
            .or_else(|| {
                self.known
                    .devices
                    .iter()
                    .find(|d| d.id == id)
                    .and_then(|d| d.kind.as_deref().and_then(Role::parse))
            })
    }

    /// Adds a device to the Devices page's list, or updates it.
    fn list(&mut self, id: &str, name: &str, role: Option<Role>, rssi: Option<i16>) {
        match self.listed.iter_mut().find(|l| l.id == id) {
            Some(l) => {
                l.name = name.to_owned();
                if let Some(r) = role {
                    l.role = r;
                }
                if rssi.is_some() {
                    l.rssi = rssi;
                }
            }
            None => self.listed.push(Listed {
                id: id.to_owned(),
                name: name.to_owned(),
                role: role.unwrap_or(Role::Trainer),
                rssi,
            }),
        }
    }

    /// Sends a command to every running backend (each ignores what it does
    /// not support).
    pub fn command_all(&self, cmd: DeviceCommand) {
        for r in self.running.lock().expect("not poisoned").iter() {
            let _ = r.handle.commands.send(cmd);
        }
    }

    /// The HUD's names: "Virtual trainer", "KICKR (connecting)".
    fn names(&self) -> Vec<String> {
        self.running
            .lock()
            .expect("not poisoned")
            .iter()
            .map(|r| match &r.link {
                Link::Connected(_) | Link::Idle => r.handle.name.clone(),
                Link::Connecting => format!("{} (connecting)", r.handle.name),
                Link::Reconnecting => format!("{} (reconnecting)", r.handle.name),
                Link::Failed(_) => format!("{} (not connected)", r.handle.name),
            })
            .collect()
    }

    fn save_known(&self) -> Result<(), String> {
        match &self.known_path {
            Some(p) => self.known.save_slot(p),
            None => Err("no configuration directory".into()),
        }
    }

    /// The remembered file for messages ("~/.config/ocw/ocw.toml [devices]").
    fn file_text(&self) -> Option<String> {
        self.known_path.as_ref().map(crate::config::Slot::describe)
    }

    /// Remembers a device that connected; the message says where, or why
    /// not.
    fn remember(&mut self, id: &str, name: &str, role: Option<Role>) -> String {
        if self.known.devices.iter().any(|d| d.id == id) {
            return String::new();
        }
        self.known.remember(KnownDevice {
            id: id.to_owned(),
            name: name.to_owned(),
            kind: role.map(|r| r.key().to_owned()),
        });
        match self.save_known() {
            Ok(()) => format!(", remembered in {}", self.file_text().unwrap_or_default()),
            Err(e) => {
                warn!("{name}: not remembered: {e}");
                format!(", not remembered: {e}")
            }
        }
    }

    /// Stops the virtual trainer (a trainer or power meter took over).
    fn stop_virtual(&mut self) -> bool {
        let running = self.running();
        let before = running.len();
        running.retain(|r| {
            let virt = r.id.is_none();
            if virt {
                let _ = r.handle.commands.send(DeviceCommand::Shutdown);
            }
            !virt
        });
        running.len() != before
    }

    /// Disconnects and forgets devices by id or name (any case), or all.
    fn forget(&mut self, which: &str) -> Result<String, String> {
        let all = which == "all";
        let hit = |id: &str, name: &str| all || id == which || name.eq_ignore_ascii_case(which);
        let mut stopped = 0;
        self.running().retain(|r| {
            let h = r.id.as_deref().is_some_and(|id| hit(id, &r.handle.name));
            if h {
                let _ = r.handle.commands.send(DeviceCommand::Shutdown);
                stopped += 1;
            }
            !h
        });
        let ids: Vec<String> = self
            .known
            .devices
            .iter()
            .filter(|d| hit(&d.id, &d.name))
            .map(|d| d.id.clone())
            .collect();
        let forgotten = ids.iter().filter(|id| self.known.forget(id)).count();
        if stopped == 0 && forgotten == 0 {
            return Err(format!("{which} is not connected or remembered"));
        }
        // Gone from the list unless a scan found it.
        let found = self.found.clone();
        self.listed
            .retain(|l| !hit(&l.id, &l.name) || found.iter().any(|f| f.id == l.id));
        if forgotten > 0 {
            self.save_known()?;
        }
        let mut line = format!("forgot {which}");
        if !self
            .running()
            .iter()
            .any(|r| r.delivers_power() || r.id.is_none())
        {
            self.start_virtual();
            line.push_str("; riding with the virtual trainer");
        }
        self.info.push_message(line.clone());
        Ok(line)
    }

    /// The Devices page's model.
    fn menu_info(&self) -> DevicesInfo {
        let running = self.running.lock().expect("not poisoned");
        let entries = self
            .listed
            .iter()
            .map(|l| Entry {
                id: l.id.clone(),
                name: l.name.clone(),
                role: l.role,
                rssi: l.rssi,
                link: running
                    .iter()
                    .find(|r| r.id.as_deref() == Some(l.id.as_str()))
                    .map_or(Link::Idle, |r| r.link.clone()),
                remembered: self.known.devices.iter().any(|d| d.id == l.id),
            })
            .collect();
        DevicesInfo {
            scan: match &self.scan {
                Some(job) => Scan::Scanning {
                    left_s: job
                        .until
                        .saturating_duration_since(Instant::now())
                        .as_secs_f64()
                        .ceil() as u32,
                },
                None => Scan::Idle,
            },
            entries,
            messages: self.info.messages.clone(),
            file: self.file_text(),
            virtual_trainer: running.iter().any(|r| r.id.is_none()),
            bridge: None,
        }
    }
}

/// Reads all device events: power samples go to the server; connections,
/// failures and drops update the Devices page; a device that connected is
/// remembered; a trainer or power meter replaces the virtual trainer.
fn poll_devices(
    mut devices: ResMut<Devices>,
    link: Res<ServerLink>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
    bridge: Option<Res<crate::bridge::Bridge>>,
) {
    let mut events = Vec::new();
    for (i, r) in devices.running().iter().enumerate() {
        for e in r.handle.events.try_iter() {
            events.push((i, r.handle.name.clone(), e));
        }
    }
    // The sensor bridge passes every reading on as it comes (spec 14
    // Phase 10, 10D.2).
    if let Some(b) = &bridge {
        for (_, name, e) in &events {
            b.relay(name, e, is_virtual(name));
        }
    }
    let mut take_over: Option<String> = None;
    for (i, name, event) in events {
        let mut line = None;
        match event {
            DeviceEvent::Connected { kind } => {
                let (id, role) = {
                    let r = &mut devices.running()[i];
                    r.link = Link::Connected(kind.to_owned());
                    if r.delivers_power() {
                        take_over = Some(name.clone());
                    }
                    (r.id.clone(), r.role)
                };
                let saved = match &id {
                    Some(id) => devices.remember(id, &name, role),
                    None => String::new(),
                };
                line = Some(format!("{name}: connected ({kind}){saved}"));
            }
            DeviceEvent::Disconnected => {
                let r = &mut devices.running()[i];
                if matches!(r.link, Link::Connected(_)) {
                    r.link = Link::Reconnecting;
                    line = Some(format!("{name}: disconnected, reconnecting"));
                }
            }
            DeviceEvent::Failed { reason } => {
                let r = &mut devices.running()[i];
                let why = reason.text().to_owned();
                if r.link != Link::Failed(why.clone()) {
                    line = Some(format!("{name}: {why}; trying again"));
                }
                r.link = Link::Failed(why);
            }
            _ => {}
        }
        if let Some(line) = line {
            console.print(line.clone());
            devices.info.push_message(line);
        }
        // Power from the virtual trainer earns no hearts (spec §14 Phase
        // 4, 4E): the samples say so.
        if matches!(event, DeviceEvent::Power { .. }) {
            client.0.set_virtual_device(is_virtual(&name));
        }
        if let Some(sample) = client.0.on_device_event(event) {
            link.send_bytes(sample);
        }
    }
    if let Some(name) = take_over
        && devices.stop_virtual()
    {
        let line = format!("{name} replaces the virtual trainer");
        console.print(line.clone());
        devices.info.push_message(line);
    }
}

/// Whether a device of this name is the virtual trainer.
pub fn is_virtual(name: &str) -> bool {
    name == ocw_devices::VirtualTrainer::new(0.0).name()
}

/// SIM/ERG control: road gradient or target power to the trainer (the
/// shared [`TrainerControl::follow`]).
pub fn control_trainer(
    time: Res<Time>,
    client: Res<ClientState>,
    devices: Res<Devices>,
    mut trainer: ResMut<Trainer>,
    bridge: Option<Res<crate::bridge::Bridge>>,
) {
    // A paired browser's ride sets the load (the bridge sends its control).
    if bridge.is_some_and(|b| b.browser_controls()) {
        return;
    }
    if let Some(cmd) = trainer.0.follow(time.elapsed_secs_f64(), &client.0) {
        devices.command_all(cmd);
    }
}

/// Follows a background scan: each device as it appears (console and the
/// Devices page), then the result.
fn poll_scan(mut devices: ResMut<Devices>, mut console: ResMut<ConsoleState>) {
    let msgs: Vec<ScanMsg> = {
        let Some(job) = &devices.scan else { return };
        let rx = job.rx.lock().expect("not poisoned");
        let mut out = Vec::new();
        loop {
            match rx.try_recv() {
                Ok(m) => out.push(m),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    out.push(ScanMsg::Done(Err(DeviceError::Bluetooth(
                        "scan thread ended".into(),
                    ))));
                    break;
                }
            }
        }
        out
    };
    for m in msgs {
        match m {
            ScanMsg::Found(d) => {
                let new = !devices.found.iter().any(|f| f.id == d.id);
                ble::merge_found(&mut devices.found, d.clone());
                let Some(role) = Role::of(d.name.as_deref(), &d.profiles) else {
                    continue;
                };
                let name = d.name.clone().unwrap_or_else(|| d.id.clone());
                devices.list(&d.id, &name, Some(role), d.rssi);
                if new {
                    let n = devices.found.len();
                    console.print(format!(
                        "  {n}: {name} ({}) rssi {} id {}",
                        role.label(),
                        d.rssi.map_or("-".into(), |r| r.to_string()),
                        d.id
                    ));
                }
            }
            ScanMsg::Done(result) => {
                devices.scan = None;
                let line = match result {
                    Ok(()) if devices.found.is_empty() => {
                        "scan: no fitness devices found (awake, in range, and not connected to another app?)".to_string()
                    }
                    Ok(()) => format!(
                        "scan: {} device(s) found, choose one to connect it",
                        devices.found.len()
                    ),
                    Err(e) => format!("scan failed: {e}"),
                };
                console.print(line.clone());
                devices.info.push_message(line);
                break;
            }
        }
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

/// Settings → Trainer → Devices (the shared menu's model).
fn publish_menu(
    devices: Res<Devices>,
    menu: Option<ResMut<ocw_ui::menu::DevicesMenu>>,
    bridge: Option<Res<crate::bridge::Bridge>>,
) {
    let Some(mut menu) = menu else { return };
    let mut info = devices.menu_info();
    info.bridge = bridge.map(|b| b.info());
    if menu.0 != info {
        menu.0 = info;
    }
}

fn shutdown_on_exit(mut exit: MessageReader<AppExit>, devices: Res<Devices>) {
    if exit.read().next().is_some() {
        devices.command_all(DeviceCommand::Shutdown);
    }
}

/// `devices.scan`: starts the background scan (or stops it).
fn start_scan(world: &mut World, secs: u64, stop: bool) -> Result<String, String> {
    let scanner = world.resource::<DeviceIo>().scan.clone();
    let mut devices = world.resource_mut::<Devices>();
    if stop {
        return match &devices.scan {
            Some(job) => {
                job.stop.store(true, Ordering::Relaxed);
                Ok("stopping the scan".into())
            }
            None => Err("no scan is running".into()),
        };
    }
    if devices.scan.is_some() {
        return Err("a scan is already running".into());
    }
    let (tx, rx) = mpsc::channel();
    let flag = Arc::new(AtomicBool::new(false));
    let thread_flag = flag.clone();
    let duration = Duration::from_secs(secs);
    std::thread::Builder::new()
        .name("ocw-ble-scan".into())
        .spawn(move || {
            let found_tx = tx.clone();
            let result = scanner(duration, &thread_flag, &mut |d| {
                let _ = found_tx.send(ScanMsg::Found(d));
            });
            let _ = tx.send(ScanMsg::Done(result));
        })
        .map_err(|e| e.to_string())?;
    devices.found.clear();
    devices.scan = Some(ScanJob {
        rx: Mutex::new(rx),
        stop: flag,
        until: Instant::now() + duration,
    });
    let line = format!("scanning for {secs} s ...");
    devices.info.push_message(line.clone());
    Ok(line)
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.scan")
            .about("Scan for Bluetooth trainers and sensors (in the background; they are listed as they appear)")
            .arg(
                Arg::new("seconds")
                    // `menu::devices::SCAN_SECONDS` (tested).
                    .default_value("10")
                    .value_parser(value_parser!(u64).range(1..=60)),
            )
            .arg(
                Arg::new("stop")
                    .long("stop")
                    .action(ArgAction::SetTrue)
                    .help("Stop a running scan"),
            ),
        |world, m| {
            let secs = *m.get_one::<u64>("seconds").expect("has default");
            start_scan(world, secs, m.get_flag("stop"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.connect")
            .about("Connect a device from the last scan (number) or by id; it is remembered once it connected")
            .arg(Arg::new("device").required(true)),
        |world, m| {
            let which = m.get_one::<String>("device").expect("required").clone();
            world.resource_scope(|world, mut devices: Mut<Devices>| {
                let found = match which.parse::<usize>() {
                    Ok(n) if n >= 1 => devices.found.get(n - 1).cloned(),
                    _ => devices.found.iter().find(|d| d.id == which).cloned(),
                };
                let (id, name) = match found {
                    Some(d) => {
                        let name = d.name.unwrap_or_else(|| d.id.clone());
                        (d.id, name)
                    }
                    None => match devices.listed.iter().find(|l| l.id == which) {
                        Some(l) => (l.id.clone(), l.name.clone()),
                        // Unknown to the last scan: try the id as given.
                        None if which.parse::<usize>().is_err() => (which.clone(), which),
                        None => return Err(format!("no device {which} in the last scan")),
                    },
                };
                devices.connect(world.resource::<DeviceIo>(), &id, &name)
            })
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.list").about("Show running and remembered devices"),
        |world, _| {
            let devices = world.resource::<Devices>();
            let info = devices.menu_info();
            let mut out = format!("running: {}", devices.names().join(", "));
            out.push_str("\nremembered:");
            if devices.known.devices.is_empty() {
                out.push_str(" none");
            }
            for d in &devices.known.devices {
                out.push_str(&format!("\n  {} (id {})", d.name, d.id));
            }
            let found: Vec<&Entry> = info.entries.iter().filter(|e| !e.remembered).collect();
            if !found.is_empty() {
                out.push_str("\nfound:");
                for e in found {
                    out.push_str(&format!("\n  {} ({}, id {})", e.name, e.summary(), e.id));
                }
            }
            if let Some(p) = &devices.known_path {
                out.push_str(&format!("\nfile: {}", p.describe()));
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.forget")
            .about("Disconnect and forget a device (id or name), or all")
            .arg(Arg::new("device").required(true)),
        |world, m| {
            let which = m.get_one::<String>("device").expect("required").clone();
            world.resource_mut::<Devices>().forget(&which)
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
            let mut devices = world.resource_mut::<Devices>();
            devices.virtual_w = watts;
            let name = devices.start_virtual();
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
    use std::path::PathBuf;

    use clap::Parser;
    use ocw_client::menu::pages::Page;
    use ocw_devices::FailReason;
    use ocw_devices::ble::Profile;
    use ocw_render::console::ConsoleCorePlugin;
    use ocw_world::Route;
    use std::sync::mpsc::Sender;

    use super::*;
    use crate::connection::ConnectionPlugin;

    fn app(extra: &[&str]) -> App {
        app_with(extra, None)
    }

    fn app_with(extra: &[&str], io: Option<DeviceIo>) -> App {
        let mut args = vec!["ocw-desktop", "--bots", "0"];
        if io.is_none() {
            args.push("--virtual");
        }
        args.extend_from_slice(extra);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Args::parse_from(args))
            .insert_resource(RideView::new(Route::test_loop()))
            .init_resource::<ocw_ui::menu::DevicesMenu>();
        if let Some(io) = io {
            app.insert_resource(io);
        }
        app.add_plugins((
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

    /// A device the mock connected: its events go in, its commands out.
    struct MockDevice {
        id: String,
        events: Sender<DeviceEvent>,
        commands: Receiver<DeviceCommand>,
    }

    /// The mock Bluetooth stack: the test feeds the scan's devices and
    /// each connected device's events.
    #[derive(Clone, Default)]
    struct Mock {
        connected: Arc<Mutex<Vec<MockDevice>>>,
        feed: Arc<Mutex<Option<Receiver<FoundDevice>>>>,
    }

    impl Mock {
        fn io(&self) -> (DeviceIo, Sender<FoundDevice>) {
            let (tx, rx) = mpsc::channel();
            *self.feed.lock().unwrap() = Some(rx);
            let connected = self.connected.clone();
            let feed = self.feed.clone();
            let io = DeviceIo {
                connect: Box::new(move |id, name| {
                    let (ev_tx, ev_rx) = mpsc::channel();
                    let (cmd_tx, cmd_rx) = mpsc::channel();
                    connected.lock().unwrap().push(MockDevice {
                        id: id.into(),
                        events: ev_tx,
                        commands: cmd_rx,
                    });
                    Ok(DeviceHandle {
                        name: name.into(),
                        events: ev_rx,
                        commands: cmd_tx,
                    })
                }),
                scan: Arc::new(move |duration, stop, found| {
                    let rx = feed.lock().unwrap().take().expect("one scan per feed");
                    let start = Instant::now();
                    while !stop.load(Ordering::Relaxed) && start.elapsed() < duration {
                        match rx.recv_timeout(Duration::from_millis(5)) {
                            Ok(d) => found(d),
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    Ok(())
                }),
            };
            (io, tx)
        }

        fn send(&self, id: &str, e: DeviceEvent) {
            let c = self.connected.lock().unwrap();
            let d = c.iter().rev().find(|d| d.id == id).expect("connected");
            d.events.send(e).unwrap();
        }

        fn ids(&self) -> Vec<String> {
            self.connected
                .lock()
                .unwrap()
                .iter()
                .map(|d| d.id.clone())
                .collect()
        }
    }

    const KICKR: &str = "hci0/dev_00_00_00_00_00_01";
    const STRAP: &str = "hci0/dev_00_00_00_00_00_02";

    fn found(id: &str, name: &str, p: Profile, rssi: i16) -> FoundDevice {
        FoundDevice {
            id: id.into(),
            name: Some(name.into()),
            profiles: vec![p],
            rssi: Some(rssi),
        }
    }

    fn menu(app: &App) -> DevicesInfo {
        app.world()
            .resource::<ocw_ui::menu::DevicesMenu>()
            .0
            .clone()
    }

    /// Updates until `ok` holds (the scan runs on its own thread).
    fn until(app: &mut App, what: &str, ok: impl Fn(&DevicesInfo) -> bool) {
        let start = Instant::now();
        loop {
            app.update();
            if ok(&menu(app)) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "{what}: {:?}",
                menu(app)
            );
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// The bug report of 2026-11-15: the scan's results reached only the
    /// console, nothing let the rider choose one, and nothing was saved.
    /// Now: scan → the list grows live in the menu → choosing connects →
    /// once connected it is remembered in devices.toml and replaces the
    /// virtual trainer → forgetting removes it → the next start reconnects
    /// it from the file.
    #[test]
    fn scan_select_remember_forget_and_reconnect_from_the_file() {
        let path = temp_file("flow");
        KnownDevices::default().save(&path).unwrap();
        let mock = Mock::default();
        let (io, feed) = mock.io();
        let mut app = app_with(&["--devices-file", path.to_str().unwrap()], Some(io));
        app.update();
        // No remembered devices: the virtual trainer rides.
        assert!(menu(&app).virtual_trainer);
        let page = |app: &App| ocw_client::menu::devices::rows(Page::Devices, &menu(app));
        assert_eq!(page(&app)[0].label, "Scan for devices");

        // Scan: the devices show as they appear.
        assert_eq!(
            run(&mut app, "devices.scan"),
            format!(
                "scanning for {} s ...",
                ocw_client::menu::devices::SCAN_SECONDS
            )
        );
        until(&mut app, "scanning", |m| {
            matches!(m.scan, Scan::Scanning { left_s: 9 | 10 })
        });
        assert_eq!(page(&app)[0].label, "Stop the scan");
        feed.send(found(KICKR, "Wahoo KICKR 0001", Profile::CyclingPower, -61))
            .unwrap();
        until(&mut app, "the KICKR", |m| m.entries.len() == 1);
        let e = &menu(&app).entries[0];
        assert_eq!(
            (e.role, e.rssi, &e.link),
            (Role::Trainer, Some(-61), &Link::Idle)
        );
        assert!(!e.remembered);
        feed.send(found(STRAP, "TICKR", Profile::HeartRate, -70))
            .unwrap();
        until(&mut app, "the strap", |m| m.entries.len() == 2);
        assert_eq!(menu(&app).entries[1].role, Role::HeartRate);
        assert_eq!(page(&app)[1].label, "Wahoo KICKR 0001");
        assert_eq!(page(&app)[2].value.as_deref(), Some("heart rate, -70 dBm"));
        // Stop it early.
        assert_eq!(run(&mut app, "devices.scan --stop"), "stopping the scan");
        until(&mut app, "the scan's end", |m| m.scan == Scan::Idle);
        assert!(
            menu(&app)
                .messages
                .last()
                .unwrap()
                .starts_with("scan: 2 device(s) found"),
            "{:?}",
            menu(&app).messages
        );

        // Choose the KICKR (the device page's Connect row).
        let rows = ocw_client::menu::devices::rows(Page::Device(0), &menu(&app));
        let connect = rows.iter().find(|r| r.label == "Connect").unwrap();
        let Some(ocw_client::menu::pages::Activate::Command(line)) = &connect.confirm else {
            panic!("{connect:?}")
        };
        assert_eq!(run(&mut app, line), "Wahoo KICKR 0001: connecting ...");
        assert_eq!(mock.ids(), [KICKR]);
        app.update();
        assert_eq!(menu(&app).entries[0].link, Link::Connecting);
        // Not saved before it connected; the virtual trainer still rides.
        assert!(KnownDevices::load(&path).unwrap().devices.is_empty());
        assert!(menu(&app).virtual_trainer);
        let status = app.world().resource::<RideView>().status.clone();
        assert!(status.contains("Wahoo KICKR 0001 (connecting)"), "{status}");

        // Connected: remembered (with its role), the virtual trainer goes.
        mock.send(
            KICKR,
            DeviceEvent::Connected {
                kind: "Wahoo KICKR",
            },
        );
        app.update();
        app.update();
        let m = menu(&app);
        assert_eq!(m.entries[0].link, Link::Connected("Wahoo KICKR".into()));
        assert!(m.entries[0].remembered && !m.virtual_trainer);
        let saved = KnownDevices::load(&path).unwrap();
        assert_eq!(
            saved.devices,
            [KnownDevice {
                id: KICKR.into(),
                name: "Wahoo KICKR 0001".into(),
                kind: Some("trainer".into()),
            }]
        );
        assert!(
            m.messages
                .iter()
                .any(|l| l.contains("connected (Wahoo KICKR), remembered in")),
            "{:?}",
            m.messages
        );
        let status = app.world().resource::<RideView>().status.clone();
        assert!(status.starts_with("Wahoo KICKR 0001 |"), "{status}");

        // The strap fails: why, and not remembered.
        run(&mut app, "devices.connect 2");
        mock.send(
            STRAP,
            DeviceEvent::Failed {
                reason: FailReason::NotFound,
            },
        );
        mock.send(STRAP, DeviceEvent::Disconnected);
        app.update();
        let strap = &menu(&app).entries[1];
        assert_eq!(strap.link, Link::Failed(FailReason::NotFound.text().into()));
        assert!(!strap.remembered);
        assert_eq!(KnownDevices::load(&path).unwrap().devices.len(), 1);

        // The next start reconnects the KICKR from the file, without the
        // virtual trainer.
        {
            let mock2 = Mock::default();
            let (io2, _feed2) = mock2.io();
            let mut next = app_with(&["--devices-file", path.to_str().unwrap()], Some(io2));
            next.update();
            assert_eq!(mock2.ids(), [KICKR]);
            let m = menu(&next);
            assert!(!m.virtual_trainer);
            assert_eq!(m.entries[0].role, Role::Trainer);
            assert!(m.entries[0].remembered);
        }

        // Forget: disconnected, out of the file, the virtual trainer back.
        let line = run(&mut app, &format!("devices.forget {KICKR}"));
        assert!(line.starts_with(&format!("forgot {KICKR}")), "{line}");
        assert!(line.ends_with("riding with the virtual trainer"), "{line}");
        let shutdown = mock.connected.lock().unwrap()[0]
            .commands
            .try_iter()
            .any(|c| c == DeviceCommand::Shutdown);
        assert!(shutdown);
        assert!(KnownDevices::load(&path).unwrap().devices.is_empty());
        app.update();
        let m = menu(&app);
        assert!(m.virtual_trainer);
        // Still in the list (the scan found it), not remembered.
        assert!(!m.entries[0].remembered && m.entries[0].link == Link::Idle);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    /// A read-only devices file: connected, but the message says why it
    /// is not remembered.
    #[cfg(unix)]
    #[test]
    fn a_read_only_devices_file_says_why() {
        use std::os::unix::fs::PermissionsExt;
        let path = temp_file("readonly");
        KnownDevices::default().save(&path).unwrap();
        let dir = path.parent().unwrap().to_path_buf();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444)).unwrap();
        let mock = Mock::default();
        let (io, _feed) = mock.io();
        let mut app = app_with(&["--devices-file", path.to_str().unwrap()], Some(io));
        run(&mut app, &format!("devices.connect {KICKR}"));
        mock.send(
            KICKR,
            DeviceEvent::Connected {
                kind: "smart trainer (FTMS)",
            },
        );
        app.update();
        let m = menu(&app);
        let line = m
            .messages
            .iter()
            .find(|l| l.contains("connected ("))
            .unwrap();
        // Root can write anyway (CI containers): then it is remembered.
        assert!(
            line.contains("not remembered:") || line.contains("remembered in"),
            "{line}"
        );
        assert!(matches!(m.entries[0].link, Link::Connected(_)));
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644));
        let _ = std::fs::remove_dir_all(dir);
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
            kind: None,
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
        // The Devices page shows it although it is not in range.
        let m = menu(&app);
        assert_eq!(m.entries.len(), 1);
        assert!(m.entries[0].remembered && m.entries[0].link == Link::Idle);
        assert_eq!(
            run(&mut app, "devices.forget hci0/dev_AA"),
            "forgot hci0/dev_AA"
        );
        assert!(KnownDevices::load(&path).unwrap().devices.is_empty());
        app.update();
        assert!(menu(&app).entries.is_empty());
        assert!(run(&mut app, "devices.forget nope").starts_with("error"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn connect_needs_a_scan_result_for_numbers() {
        let path = temp_file("connect");
        KnownDevices::default().save(&path).unwrap();
        let mut app = app(&["--devices-file", path.to_str().unwrap()]);
        assert!(run(&mut app, "devices.connect 1").starts_with("error: no device 1"));
        assert!(run(&mut app, "devices.scan --stop").starts_with("error: no scan"));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }
}

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
//! ANT+ (spec 3.4.1, 14 Phase 17): the ANT stick service
//! ([`ocw_devices::ant::AntStick`]: a real USB stick, or the simulated one
//! of `--ant-sim`) sits next to Bluetooth in [`DeviceIo`]: one scan runs
//! both, an `ant:` id connects through the stick, and the rest of the flow
//! is the same. Devices → ANT+ shows the stick and takes the ANT+ network
//! key, kept in `secrets.toml [ant]` (never logged; `ant.key.*`). With two
//! heart-rate sources only one is used: the one chosen with *Use for heart
//! rate* (`devices.hr <id>`), else the first that sent – no automatic
//! switching (Q6 (c)). The same rule holds for two trainers (a BLE FTMS
//! one and an ANT+ FE-C one, 17E): only one trainer's power counts and
//! only it gets SIM / ERG – the one chosen with *Use as trainer*
//! (`devices.trainer <id>`), else the first that sent power.
//!
//! The Bluetooth stack is behind [`DeviceIo`] (connect, scan), so the
//! tests run the whole flow with a mock (and ANT+ with the simulated
//! stick).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command, value_parser};
use ocw_client::menu::devices::{AntFix, AntInfo, DevicesInfo, Entry, Link, Role, Scan};
use ocw_devices::ant::{self, AntDeviceId, AntStick, NetworkKey, StickStatus};
use ocw_devices::ble::{self, BleDevice, FoundDevice};
use ocw_devices::{
    DeviceBackend, DeviceCommand, DeviceError, DeviceEvent, DeviceHandle, VirtualTrainer,
};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_ui::join::FieldEnd;
use ocw_ui::key_entry::KeyEntry;

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

/// The Bluetooth stack (btleplug in the app, a mock in tests) and the ANT
/// stick service.
#[derive(Resource)]
pub struct DeviceIo {
    pub connect: Connector,
    pub scan: Scanner,
    /// ANT+ (`None`: `--no-ant`).
    pub ant: Option<AntStick>,
}

impl DeviceIo {
    /// The real one (BlueZ, WinRT, CoreBluetooth through btleplug), no ANT.
    pub fn bluetooth() -> Self {
        Self {
            connect: Box::new(|id, name| BleDevice::new(id, name).start()),
            scan: Arc::new(|d, stop, found| ble::scan_live(d, stop, found)),
            ant: None,
        }
    }
}

/// The ANT stick service for these arguments: the real USB sticks, the
/// simulated stick of `--ant-sim`, or none (`--no-ant`).
pub fn ant_stick(args: &Args, key: Option<NetworkKey>) -> Option<AntStick> {
    if args.no_ant {
        return None;
    }
    if let Some(spec) = &args.ant_sim {
        return match ant::sim::parse_sensors(spec) {
            Ok(sensors) => {
                info!(sensors = sensors.len(), "a simulated ANT stick (--ant-sim)");
                Some(AntStick::start(ant::SimTransport::new(None, sensors), key))
            }
            Err(e) => {
                warn!("--ant-sim: {e}");
                None
            }
        };
    }
    // Windows (spec 14 Phase 17 Q7 (d)): WinUSB through nusb first (a
    // stick bound with Zadig), then Garmin's own libusb-win32 driver.
    #[cfg(windows)]
    let transport = ant::Fallback::new(
        ant::usb::UsbTransport::new(),
        ant::libusb0::Libusb0Transport::system(),
    );
    #[cfg(not(windows))]
    let transport = ant::usb::UsbTransport::new();
    Some(AntStick::start(transport, key))
}

/// What the Devices page offers for a stick that cannot be opened, on
/// this system (`std::env::consts::OS`): Windows – Garmin's driver from
/// Windows Update, or Zadig (Q7 (d)); Linux – the udev rule (Q8 (b)).
pub fn ant_fix(status: &StickStatus, os: &str) -> Option<AntFix> {
    match (os, status) {
        ("windows", StickStatus::NoDriver(_) | StickStatus::NoPermission(_)) => {
            Some(AntFix::WindowsDriver)
        }
        ("linux", StickStatus::NoPermission(_)) => Some(AntFix::LinuxAccess),
        _ => None,
    }
}

/// How *Install access for the ANT stick* runs (`pkexec` on the system; a
/// fake in tests) and where the rule goes.
#[derive(Resource, Clone)]
pub struct AntAccess {
    pub runner: Arc<dyn crate::ant_access::Runner>,
    pub path: std::path::PathBuf,
}

impl Default for AntAccess {
    fn default() -> Self {
        Self {
            runner: Arc::new(crate::ant_access::System),
            path: crate::ant_access::RULE_PATH.into(),
        }
    }
}

/// A running *Install access* (pkexec waits for the password) and what
/// the last one said.
#[derive(Resource, Default)]
struct AccessJob {
    rx: Option<Mutex<Receiver<crate::ant_access::Outcome>>>,
    last: Option<String>,
}

/// What `secrets.toml [ant]` holds.
#[derive(serde::Deserialize)]
struct AntSecrets {
    network_key: Option<String>,
}

/// The ANT+ network key the rider entered (never logged).
pub fn load_ant_key(slot: &crate::config::Slot) -> Option<NetworkKey> {
    let text = match slot.read() {
        Ok(t) => t?,
        Err(e) => {
            warn!("cannot read the ANT+ network key: {e}");
            return None;
        }
    };
    let Ok(s) = toml::from_str::<AntSecrets>(&text) else {
        warn!("{}: not readable (the ANT+ network key)", slot.describe());
        return None;
    };
    match NetworkKey::parse(s.network_key.as_deref()?) {
        Ok(k) => Some(k),
        Err(e) => {
            warn!("{}: the ANT+ network key: {e}", slot.describe());
            None
        }
    }
}

pub struct DevicesPlugin;

impl Plugin for DevicesPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        let files = app
            .world()
            .get_resource::<crate::config::ConfigFiles>()
            .cloned();
        let ant_key_slot = files.as_ref().map(|f| f.secret("ant"));
        let ant_key = ant_key_slot.as_ref().and_then(load_ant_key);
        if !app.world().contains_resource::<DeviceIo>() {
            let mut io = DeviceIo::bluetooth();
            io.ant = ant_stick(&args, ant_key);
            app.insert_resource(io);
        } else if let Some(stick) = &app.world().resource::<DeviceIo>().ant
            && ant_key.is_some()
            && !stick.has_key()
        {
            // A stick given from outside (tests) gets the saved key.
            stick.set_key(ant_key);
        }
        let default = files.as_ref().map(|f| f.section("devices"));
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
        devices.ant_key = ant_key_slot;
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
            .init_resource::<KeyEntry>()
            .init_resource::<AntAccess>()
            .init_resource::<AccessJob>()
            .add_systems(PreUpdate, poll_devices)
            .add_systems(
                Update,
                (
                    poll_scan,
                    control_trainer,
                    publish_status,
                    poll_key_entry,
                    poll_access,
                    publish_menu,
                )
                    .chain(),
            )
            .add_systems(Last, shutdown_on_exit);
        register_commands(app);
    }
}

#[derive(Resource)]
pub struct Trainer(pub TrainerControl);

/// What a scan thread reports (Bluetooth's, ANT+'s).
enum ScanMsg {
    Found(FoundDevice),
    Done(&'static str, Result<(), DeviceError>),
}

/// A running scan.
struct ScanJob {
    rx: Mutex<Receiver<ScanMsg>>,
    stop: Arc<AtomicBool>,
    until: Instant,
    /// Scan threads still running.
    pending: usize,
    /// Their failures ("Bluetooth: …").
    errors: Vec<String>,
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
    /// A trainer of the rider's (not the virtual one).
    fn is_trainer(&self) -> bool {
        self.id.is_some() && self.role == Some(Role::Trainer)
    }

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
    /// Where the ANT+ network key is kept (`secrets.toml [ant]`).
    ant_key: Option<crate::config::Slot>,
    /// The heart-rate source when none is chosen: the first device that
    /// sent a heart rate (spec 14 Phase 17 Q6 (c): no switching).
    hr_active: Option<String>,
    /// The trainer when none is chosen: the first trainer that sent power
    /// (17E, the Q6 (c) rule for trainers).
    trainer_active: Option<String>,
    /// The last SIM / ERG command (a newly chosen trainer gets it at once).
    last_control: Mutex<Option<DeviceCommand>>,
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
            ant_key: None,
            hr_active: None,
            trainer_active: None,
            last_control: Mutex::new(None),
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
        let handle = if AntDeviceId::is_ant(id) {
            match &io.ant {
                Some(stick) => stick.connect(id, name),
                None => Err(DeviceError::Start("ANT+ is off (--no-ant)".into())),
            }
        } else {
            (io.connect)(id, name)
        }
        .map_err(|e| format!("{name}: {e}"))?;
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
    /// not support); SIM / ERG go only to the trainer in use when there
    /// are two (17E, the Q6 (c) rule for trainers).
    pub fn command_all(&self, cmd: DeviceCommand) {
        let control = matches!(
            cmd,
            DeviceCommand::SetSimulation(_) | DeviceCommand::SetTargetPower(_)
        );
        if control {
            *self.last_control.lock().expect("not poisoned") = Some(cmd);
        }
        let source = self.trainer_source();
        for r in self.running.lock().expect("not poisoned").iter() {
            if control
                && r.is_trainer()
                && source
                    .as_deref()
                    .is_some_and(|s| r.id.as_deref() != Some(s))
            {
                continue;
            }
            let _ = r.handle.commands.send(cmd);
        }
    }

    /// The trainer whose power counts and which gets SIM / ERG: the one
    /// chosen with *Use as trainer* while it runs, else the first that
    /// sent power (`None`: none yet – every trainer gets the commands).
    fn trainer_source(&self) -> Option<String> {
        let running = self.running.lock().expect("not poisoned");
        let runs = |id: &str| running.iter().any(|r| r.id.as_deref() == Some(id));
        match &self.known.trainer {
            Some(c) if runs(c) => Some(c.clone()),
            _ => self.trainer_active.clone().filter(|a| runs(a)),
        }
    }

    /// Whether power from the trainer `id` is used (the Q6 (c) rule for
    /// trainers: no mixing, no switching by itself).
    fn accept_trainer_power(&mut self, id: &str) -> bool {
        match self.trainer_source() {
            Some(src) => src == id,
            None => {
                self.trainer_active = Some(id.to_owned());
                true
            }
        }
    }

    /// `devices.trainer <id|name|auto>`: *Use as trainer*.
    fn use_as_trainer(&mut self, which: &str) -> Result<String, String> {
        let line = if which == "auto" {
            self.known.trainer = None;
            self.trainer_active = None;
            "trainer: the first one that sends power".to_owned()
        } else {
            let l = self
                .listed
                .iter()
                .find(|l| l.id == which || l.name.eq_ignore_ascii_case(which))
                .ok_or_else(|| format!("{which} is not in the list"))?;
            if l.role != Role::Trainer {
                return Err(format!("{} is not a trainer", l.name));
            }
            let (id, name) = (l.id.clone(), l.name.clone());
            self.known.trainer = Some(id.clone());
            self.trainer_active = Some(id.clone());
            // It takes over the current SIM / ERG target at once.
            let last = *self.last_control.lock().expect("not poisoned");
            if let Some(cmd) = last
                && let Some(r) = self.running().iter().find(|r| r.id.as_deref() == Some(&id))
            {
                let _ = r.handle.commands.send(cmd);
            }
            format!("{name} is the trainer")
        };
        if let Err(e) = self.save_known() {
            warn!("the trainer choice is not saved: {e}");
        }
        self.info.push_message(line.clone());
        Ok(line)
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
            transport: AntDeviceId::is_ant(id).then(|| "ant".to_owned()),
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
        if self
            .hr_active
            .as_deref()
            .is_some_and(|a| ids.iter().any(|i| i == a) || all)
        {
            self.hr_active = None;
        }
        if self
            .trainer_active
            .as_deref()
            .is_some_and(|a| ids.iter().any(|i| i == a) || all)
        {
            self.trainer_active = None;
        }
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

    /// Whether a backend runs for this id.
    fn is_running(&self, id: &str) -> bool {
        self.running
            .lock()
            .expect("not poisoned")
            .iter()
            .any(|r| r.id.as_deref() == Some(id))
    }

    /// The device whose heart rate is used: the one chosen with *Use for
    /// heart rate* while it runs, else the first that sent one.
    fn hr_source(&self) -> Option<String> {
        match &self.known.heart_rate {
            Some(c) if self.is_running(c) => Some(c.clone()),
            _ => self.hr_active.clone(),
        }
    }

    /// Whether a heart rate from `id` is used (spec 14 Phase 17 Q6 (c)).
    fn accept_heart_rate(&mut self, id: Option<&str>) -> bool {
        match self.hr_source() {
            Some(src) if self.is_running(&src) => id == Some(src.as_str()),
            _ => {
                self.hr_active = id.map(str::to_owned);
                true
            }
        }
    }

    /// `devices.hr <id|name|auto>`: *Use for heart rate*.
    fn use_for_heart_rate(&mut self, which: &str) -> Result<String, String> {
        let line = if which == "auto" {
            self.known.heart_rate = None;
            self.hr_active = None;
            "heart rate: from the first device that sends one".to_owned()
        } else {
            let l = self
                .listed
                .iter()
                .find(|l| l.id == which || l.name.eq_ignore_ascii_case(which))
                .ok_or_else(|| format!("{which} is not in the list"))?;
            let (id, name) = (l.id.clone(), l.name.clone());
            self.known.heart_rate = Some(id.clone());
            self.hr_active = Some(id);
            format!("{name} gives the heart rate")
        };
        if let Err(e) = self.save_known() {
            warn!("the heart-rate choice is not saved: {e}");
        }
        self.info.push_message(line.clone());
        Ok(line)
    }

    /// The Devices page's model.
    fn menu_info(&self) -> DevicesInfo {
        let hr_source = self.hr_source();
        let hr_listed = self
            .listed
            .iter()
            .filter(|l| l.role == Role::HeartRate)
            .count();
        let trainer_source = self.trainer_source();
        let trainers_listed = self
            .listed
            .iter()
            .filter(|l| l.role == Role::Trainer)
            .count();
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
                heart_rate: (l.role == Role::HeartRate && hr_listed >= 2)
                    .then(|| hr_source.as_deref() == Some(l.id.as_str())),
                trainer: (l.role == Role::Trainer && trainers_listed >= 2)
                    .then(|| trainer_source.as_deref() == Some(l.id.as_str())),
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
            ant: None,
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
    bridge: Option<ResMut<crate::bridge::Bridge>>,
) {
    let mut events = Vec::new();
    for (i, r) in devices.running().iter().enumerate() {
        for e in r.handle.events.try_iter() {
            events.push((i, r.handle.name.clone(), e));
        }
    }
    // One heart-rate source (spec 14 Phase 17 Q6 (c)): a second strap's
    // heart rate is not mixed in.
    let ids: Vec<Option<String>> = devices.running().iter().map(|r| r.id.clone()).collect();
    events.retain(|(i, _, e)| {
        !matches!(e, DeviceEvent::HeartRate { .. }) || devices.accept_heart_rate(ids[*i].as_deref())
    });
    // One trainer (17E, the Q6 (c) rule for trainers): a second trainer's
    // power is not mixed in.
    let trainers: Vec<bool> = devices.running().iter().map(Running::is_trainer).collect();
    events.retain(|(i, _, e)| match (&ids[*i], e) {
        (Some(id), DeviceEvent::Power { .. }) if trainers[*i] => devices.accept_trainer_power(id),
        _ => true,
    });
    // The sensor bridge passes every reading on as it comes (spec 14
    // Phase 10, 10D.2).
    if let Some(mut b) = bridge {
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
                let id = r.id.clone();
                if matches!(r.link, Link::Connected(_)) {
                    r.link = Link::Reconnecting;
                    line = Some(format!("{name}: disconnected, reconnecting"));
                }
                // The heart-rate source is gone: no heart rate (not a frozen
                // one, and not another strap's: spec 14 Phase 17 Q6 (c)).
                if id.is_some() && devices.hr_source() == id {
                    client.0.on_device_event(DeviceEvent::HeartRate {
                        bpm: 0.0,
                        timestamp: Duration::ZERO,
                    });
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
                    for _ in 0..job.pending {
                        out.push(ScanMsg::Done(
                            "scan",
                            Err(DeviceError::Bluetooth("scan thread ended".into())),
                        ));
                    }
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
            ScanMsg::Done(what, result) => {
                let Some(job) = devices.scan.as_mut() else {
                    break;
                };
                job.pending = job.pending.saturating_sub(1);
                if let Err(e) = result {
                    job.errors.push(format!("{what}: {e}"));
                }
                if job.pending > 0 {
                    continue;
                }
                let errors = std::mem::take(&mut job.errors);
                devices.scan = None;
                let n = devices.found.len();
                let line = if n == 0 && !errors.is_empty() {
                    format!("scan failed: {}", errors.join("; "))
                } else {
                    let mut line = if n == 0 {
                        "scan: no fitness devices found (awake, in range, and not connected to another app?)".to_string()
                    } else {
                        format!("scan: {n} device(s) found, choose one to connect it")
                    };
                    if !errors.is_empty() {
                        line.push_str(&format!(" ({})", errors.join("; ")));
                    }
                    line
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
    io: Res<DeviceIo>,
    key: Res<KeyEntry>,
    access: Res<AccessJob>,
    menu: Option<ResMut<ocw_ui::menu::DevicesMenu>>,
    bridge: Option<Res<crate::bridge::Bridge>>,
) {
    let Some(mut menu) = menu else { return };
    let mut info = devices.menu_info();
    info.bridge = bridge.map(|b| b.info());
    info.ant = io.ant.as_ref().map(|stick| {
        let status = stick.status();
        AntInfo {
            state: status.text(),
            key_set: stick.has_key(),
            typing: key.typing.then(|| key.shown(16)),
            key_file: devices.ant_key.as_ref().map(crate::config::Slot::describe),
            fix: ant_fix(&status, std::env::consts::OS),
            access: access.last.clone(),
        }
    });
    if menu.0 != info {
        menu.0 = info;
    }
}

fn shutdown_on_exit(mut exit: MessageReader<AppExit>, devices: Res<Devices>, io: Res<DeviceIo>) {
    if exit.read().next().is_some() {
        devices.command_all(DeviceCommand::Shutdown);
        if let Some(stick) = &io.ant {
            stick.shutdown();
        }
    }
}

/// The ANT+ network key's field (Devices → ANT+): Enter saves, Esc drops,
/// Ctrl+V pastes.
fn poll_key_entry(world: &mut World) {
    let (end, paste) = {
        let Some(mut k) = world.get_resource_mut::<KeyEntry>() else {
            return;
        };
        (k.take_end(), k.take_paste())
    };
    if paste {
        let _ = paste_key(world);
    }
    let line = match end {
        Some(FieldEnd::Enter) => save_key(world),
        Some(FieldEnd::Escape) => cancel_key(world),
        None => return,
    };
    let line = line.unwrap_or_else(|e| e);
    world.resource_mut::<ConsoleState>().print(line);
}

/// `ant.key.save`: the typed key into `secrets.toml [ant]` and to the
/// stick. A key that does not read keeps the field open.
fn save_key(world: &mut World) -> Result<String, String> {
    let text = world.resource::<KeyEntry>().text().to_owned();
    let key = match NetworkKey::parse(&text) {
        Ok(k) => k,
        Err(e) => {
            let line = format!("ANT+ network key: {e}");
            world.resource_mut::<KeyEntry>().typing = true;
            world
                .resource_mut::<Devices>()
                .info
                .push_message(line.clone());
            return Err(line);
        }
    };
    let slot = world
        .resource::<Devices>()
        .ant_key
        .clone()
        .ok_or("no configuration folder: the ANT+ network key cannot be kept")?;
    let saved = slot.write(&format!("network_key = \"{}\"\n", key.to_hex()));
    world.resource_mut::<KeyEntry>().cancel();
    let line = match saved {
        Ok(()) => {
            if let Some(stick) = &world.resource::<DeviceIo>().ant {
                stick.set_key(Some(key));
            }
            format!("ANT+ network key saved in {}", slot.describe())
        }
        Err(e) => format!("ANT+ network key not saved: {e}"),
    };
    world
        .resource_mut::<Devices>()
        .info
        .push_message(line.clone());
    Ok(line)
}

fn cancel_key(world: &mut World) -> Result<String, String> {
    world.resource_mut::<KeyEntry>().cancel();
    Ok("the ANT+ network key was not changed".into())
}

/// `ant.key.paste`: the clipboard into the field.
fn paste_key(world: &mut World) -> Result<String, String> {
    let paste = world
        .get_resource::<ocw_ui::join::JoinHooks>()
        .map(|h| h.paste)
        .ok_or("no clipboard")?;
    let text = paste(world).ok_or("the clipboard has no text")?;
    let mut k = world.resource_mut::<KeyEntry>();
    if !k.typing {
        k.start();
    }
    k.field.paste(&text);
    Ok("pasted".into())
}

/// `ant.access.install`: the udev rule through pkexec, on a thread (the
/// password dialog blocks); [`poll_access`] reports the outcome.
fn start_access(world: &mut World) -> Result<String, String> {
    if world.resource::<AccessJob>().rx.is_some() {
        return Err("Install access is already running".into());
    }
    let access = world.resource::<AntAccess>().clone();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("ocw-ant-access".into())
        .spawn(move || {
            let outcome = crate::ant_access::install(access.runner.as_ref(), &access.path, false);
            let _ = tx.send(outcome);
        })
        .map_err(|e| e.to_string())?;
    world.resource_mut::<AccessJob>().rx = Some(Mutex::new(rx));
    let line = "installing access for the ANT stick (pkexec asks for your password) ...";
    world.resource_mut::<Devices>().info.push_message(line);
    Ok(line.into())
}

/// The outcome of *Install access*: the page's line, the console's text
/// (with the rule and the manual command when it was refused).
fn poll_access(world: &mut World) {
    let got = {
        let job = world.resource::<AccessJob>();
        let Some(rx) = &job.rx else { return };
        let rx = rx.lock().unwrap_or_else(|e| e.into_inner());
        match rx.try_recv() {
            Ok(o) => Some(o),
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => None,
        }
    };
    let outcome = got
        .unwrap_or_else(|| crate::ant_access::Outcome::Failed("the install thread ended".into()));
    let path = world.resource::<AntAccess>().path.clone();
    let short = outcome.short();
    let text = outcome.message(&path);
    if outcome.ok() {
        info!("{text}");
    } else {
        warn!("{text}");
    }
    let mut job = world.resource_mut::<AccessJob>();
    job.rx = None;
    job.last = Some(short.clone());
    world
        .resource_mut::<Devices>()
        .info
        .push_message(format!("Install access: {short}"));
    world.resource_mut::<ConsoleState>().print(text);
}

/// `ant.key.clear`: no key any more (ANT+ stops).
fn clear_key(world: &mut World) -> Result<String, String> {
    let slot = world
        .resource::<Devices>()
        .ant_key
        .clone()
        .ok_or("no configuration folder")?;
    slot.remove()?;
    if let Some(stick) = &world.resource::<DeviceIo>().ant {
        stick.set_key(None);
    }
    let line = format!("ANT+ network key removed from {}", slot.describe());
    world
        .resource_mut::<Devices>()
        .info
        .push_message(line.clone());
    Ok(line)
}

/// `devices.scan`: starts the background scan (or stops it).
fn start_scan(world: &mut World, secs: u64, stop: bool) -> Result<String, String> {
    let scanner = world.resource::<DeviceIo>().scan.clone();
    // ANT+ once the key is there (spec 3.4.1: one scan runs both).
    let ant = world
        .resource::<DeviceIo>()
        .ant
        .clone()
        .filter(AntStick::has_key);
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
    let mut pending = 1;
    if let Some(stick) = ant {
        let (tx, flag) = (tx.clone(), flag.clone());
        std::thread::Builder::new()
            .name("ocw-ant-scan".into())
            .spawn(move || {
                let found_tx = tx.clone();
                let result = stick.scan(duration, &flag, &mut |d| {
                    let _ = found_tx.send(ScanMsg::Found(d));
                });
                let _ = tx.send(ScanMsg::Done("ANT+", result));
            })
            .map_err(|e| e.to_string())?;
        pending += 1;
    }
    std::thread::Builder::new()
        .name("ocw-ble-scan".into())
        .spawn(move || {
            let found_tx = tx.clone();
            let result = scanner(duration, &thread_flag, &mut |d| {
                let _ = found_tx.send(ScanMsg::Found(d));
            });
            let _ = tx.send(ScanMsg::Done("Bluetooth", result));
        })
        .map_err(|e| e.to_string())?;
    devices.found.clear();
    devices.scan = Some(ScanJob {
        rx: Mutex::new(rx),
        stop: flag,
        until: Instant::now() + duration,
        pending,
        errors: Vec::new(),
    });
    let line = format!("scanning for {secs} s ...");
    devices.info.push_message(line.clone());
    Ok(line)
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.scan")
            .about("Scan for Bluetooth and ANT+ trainers and sensors (in the background; they are listed as they appear)")
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
        Command::new("devices.hr")
            .about("Use this device's heart rate (Use for heart rate; with two straps only one counts); auto: the first that sends")
            .arg(Arg::new("device").required(true)),
        |world, m| {
            let which = m.get_one::<String>("device").expect("required").clone();
            world.resource_mut::<Devices>().use_for_heart_rate(&which)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.trainer")
            .about("Use this trainer (Use as trainer; with two trainers only one counts and gets SIM / ERG); auto: the first that sends")
            .arg(Arg::new("device").required(true)),
        |world, m| {
            let which = m.get_one::<String>("device").expect("required").clone();
            world.resource_mut::<Devices>().use_as_trainer(&which)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ant.key.edit")
            .about("Type the ANT+ network key (Devices -> ANT+; Enter saves it in secrets.toml, Esc cancels)"),
        |world, _| {
            if world.resource::<DeviceIo>().ant.is_none() {
                return Err("ANT+ is off (--no-ant)".into());
            }
            world.resource_mut::<KeyEntry>().start();
            Ok("typing the ANT+ network key: Enter saves, Esc cancels".into())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ant.key.save").about("Save the typed ANT+ network key"),
        |world, _| save_key(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ant.key.paste").about("Paste the ANT+ network key from the clipboard"),
        |world, _| paste_key(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ant.key.cancel").about("Stop typing the ANT+ network key"),
        |world, _| cancel_key(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ant.key.clear").about("Remove the ANT+ network key (ANT+ stops)"),
        |world, _| clear_key(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new(ocw_client::menu::devices::ANT_ACCESS_COMMAND)
            .about("Linux: install the udev rule that lets you open the ANT stick (asks for the password with pkexec; the same as ocw-desktop --install-udev-rule)"),
        |world, _| start_access(world),
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("devices.list").about("Show running and remembered devices"),
        |world, _| {
            let ant = world
                .resource::<DeviceIo>()
                .ant
                .as_ref()
                .map(|s| s.status().text());
            let devices = world.resource::<Devices>();
            let info = devices.menu_info();
            let mut out = format!("running: {}", devices.names().join(", "));
            if let Some(a) = ant {
                out.push_str(&format!("\nANT+: {a}"));
            }
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
        app_in(extra, io, None)
    }

    /// With the config files of `files` (`ocw.toml`, `secrets.toml`).
    fn app_in(
        extra: &[&str],
        io: Option<DeviceIo>,
        files: Option<crate::config::ConfigFiles>,
    ) -> App {
        let mut args = vec!["ocw-desktop", "--bots", "0"];
        if io.is_none() {
            // No real ANT stick service in unit tests.
            args.extend(["--virtual", "--no-ant"]);
        }
        args.extend_from_slice(extra);
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Args::parse_from(args))
            .insert_resource(RideView::new(Route::test_loop()))
            .init_resource::<ocw_ui::menu::DevicesMenu>();
        if let Some(f) = files {
            app.insert_resource(f);
        }
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
                ant: None,
            };
            (io, tx)
        }

        fn send(&self, id: &str, e: DeviceEvent) {
            let c = self.connected.lock().unwrap();
            let d = c.iter().rev().find(|d| d.id == id).expect("connected");
            d.events.send(e).unwrap();
        }

        /// The commands the app sent to the device `id` since the last call.
        fn commands(&self, id: &str) -> Vec<DeviceCommand> {
            let c = self.connected.lock().unwrap();
            c.iter()
                .filter(|d| d.id == id)
                .flat_map(|d| d.commands.try_iter().collect::<Vec<_>>())
                .collect()
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
                transport: None,
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
            transport: None,
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

    // ANT+ (spec 14 Phase 17): the simulated stick in place of a real one.

    use ocw_client::menu::join::TextField;
    use ocw_devices::ant::sim::SimSensor;
    use ocw_devices::ant::{SimTransport, StickStatus};

    const ANT_STRAP: &str = "ant:120:12345:1";
    const ANT_STRAP_2: &str = "ant:120:23456:1";

    fn temp_config(name: &str) -> crate::config::ConfigFiles {
        let dir = std::env::temp_dir().join(format!("ocw-ant-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        crate::config::ConfigFiles::in_dir(&dir)
    }

    fn ant_io(mock: &Mock, sim: &SimTransport, key: Option<NetworkKey>) -> DeviceIo {
        let (mut io, _feed) = mock.io();
        io.ant = Some(AntStick::start(sim.clone(), key));
        io
    }

    fn hr_now(app: &App) -> Option<f64> {
        app.world().resource::<ClientState>().0.heart_rate_bpm()
    }

    /// Updates until `ok` holds on the app (the stick runs on its thread).
    fn until_app(app: &mut App, what: &str, secs: u64, ok: impl Fn(&App) -> bool) {
        let start = Instant::now();
        loop {
            app.update();
            if ok(app) {
                return;
            }
            assert!(
                start.elapsed() < Duration::from_secs(secs),
                "{what}: {:?}",
                menu(app)
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn type_key(app: &mut App, text: &str) -> String {
        run(app, "ant.key.edit");
        app.world_mut().resource_mut::<KeyEntry>().field = TextField::new(text);
        run(app, "ant.key.save")
    }

    /// The whole ANT+ path without hardware: the Devices page asks for the
    /// key → it is typed and kept in secrets.toml (mode 0600, never in
    /// ocw.toml or the console) → a scan lists the simulated straps →
    /// choosing one connects it and remembers it with `transport = "ant"`
    /// → its heart rate reaches the ride (the samples, the HUD's source)
    /// and the FIT recording → the next start reconnects it with the saved
    /// key.
    #[test]
    fn ant_strap_key_scan_pair_ride_and_heart_rate_in_the_fit_file() {
        let files = temp_config("flow");
        let devices_file = files.dir().join("devices.toml");
        KnownDevices::default().save(&devices_file).unwrap();
        let sim = SimTransport::new(
            Some(NetworkKey::TEST),
            vec![
                SimSensor::strap(12345, 142.0),
                SimSensor::strap(23456, 128.0),
            ],
        );
        let mock = Mock::default();
        let io = ant_io(&mock, &sim, None);
        let path = devices_file.to_str().unwrap().to_owned();
        let mut app = app_in(&["--devices-file", &path], Some(io), Some(files.clone()));
        app.update();
        // No key yet: the ANT+ row says so.
        let page = |app: &App| ocw_client::menu::devices::rows(Page::Devices, &menu(app));
        let ant_row = page(&app)
            .into_iter()
            .find(|r| r.label == "ANT+ (USB stick)")
            .unwrap();
        assert_eq!(ant_row.value.as_deref(), Some("network key needed"));

        // A key that does not read keeps the field open and saves nothing.
        let line = type_key(&mut app, "0A 1B 2C");
        assert!(line.contains("16 hex digits"), "{line}");
        assert!(app.world().resource::<KeyEntry>().typing);
        assert!(!files.secrets.exists());
        // The key: saved privately, the stick gets it.
        run(&mut app, "ant.key.cancel");
        let line = type_key(&mut app, "0A 1B 2C 3D 4E 5F 60 71");
        assert!(line.starts_with("ANT+ network key saved in"), "{line}");
        assert!(line.ends_with("secrets.toml [ant]"), "{line}");
        let secrets = std::fs::read_to_string(&files.secrets).unwrap();
        assert!(
            secrets.contains("network_key = \"0A1B2C3D4E5F6071\""),
            "{secrets}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&files.secrets)
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let log = app.world().resource::<ConsoleState>().log().join("\n");
        assert!(
            !log.contains("0A1B2C3D") && !log.contains("0A 1B 2C 3D"),
            "{log}"
        );
        if let Ok(config) = std::fs::read_to_string(&files.config) {
            assert!(!config.contains("0A1B2C3D"), "{config}");
        }
        assert!(!app.world().resource::<KeyEntry>().typing);

        // Scan: the straps join the list as the stick finds them.
        run(&mut app, "devices.scan 4");
        until(&mut app, "two ANT+ straps", |m| m.entries.len() == 2);
        let m = menu(&app);
        assert!(
            m.entries
                .iter()
                .all(|e| e.role == Role::HeartRate && e.is_ant())
        );
        assert_eq!(
            page(&app)[2]
                .value
                .as_deref()
                .map(|v| v.starts_with("heart rate, ANT+")),
            Some(true)
        );
        until(&mut app, "the scan's end", |m| m.scan == Scan::Idle);
        // The stick is plugged in (and open while the scan ran).
        let state = menu(&app).ant.unwrap().state;
        assert!(state.starts_with("Simulated ANT stick"), "{state}");

        // Connect the strap of 142 bpm: connected, remembered as ANT.
        let line = run(&mut app, &format!("devices.connect {ANT_STRAP}"));
        assert_eq!(line, "ANT+ heart rate 12345: connecting ...");
        until(&mut app, "connected", |m| {
            m.entries
                .iter()
                .any(|e| e.id == ANT_STRAP && matches!(e.link, Link::Connected(_)))
        });
        let saved = KnownDevices::load(&devices_file).unwrap();
        assert_eq!(
            saved.devices,
            [KnownDevice {
                id: ANT_STRAP.into(),
                name: "ANT+ heart rate 12345".into(),
                kind: Some("hr".into()),
                transport: Some("ant".into()),
            }]
        );
        // A strap does not replace the virtual trainer.
        assert!(menu(&app).virtual_trainer);
        until_app(&mut app, "142 bpm in the ride", 5, |a| {
            hr_now(a) == Some(142.0)
        });

        // Ride: the recorder takes the heart rate into the FIT file.
        let mut rec = ocw_client::recording::RideRecorder::new("Loop");
        for i in 0..5 {
            app.update();
            let c = &app.world().resource::<ClientState>().0;
            let mut s = ocw_client::recording::sample(c, f64::from(i));
            s.distance = f64::from(i) * 7.0;
            rec.observe(c.track(), None, s, 1_800_000_000);
            std::thread::sleep(Duration::from_millis(20));
        }
        let ride = rec.finish().unwrap();
        let fit = ocw_fit::decode(&ocw_fit::encode(&ride.activity)).unwrap();
        assert_eq!(fit.records.len(), 5);
        assert!(
            fit.records.iter().all(|r| r.heart_rate_bpm == Some(142.0)),
            "{:?}",
            fit.records
        );

        // The next start: the key from secrets.toml, the strap reconnected
        // (once this app let go of the stick: one program at a time).
        app.world()
            .resource::<DeviceIo>()
            .ant
            .as_ref()
            .unwrap()
            .shutdown();
        let start = Instant::now();
        while sim.links_open() > 0 {
            assert!(start.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(10));
        }
        {
            let mock2 = Mock::default();
            let io2 = ant_io(&mock2, &sim, None);
            let mut next = app_in(&["--devices-file", &path], Some(io2), Some(files.clone()));
            until_app(&mut next, "reconnected", 10, |a| hr_now(a) == Some(142.0));
            assert!(menu(&next).ant.unwrap().key_set);
        }

        // Removing the key: ANT+ asks for it again.
        let line = run(&mut app, "ant.key.clear");
        assert!(line.starts_with("ANT+ network key removed"), "{line}");
        assert!(
            !std::fs::read_to_string(&files.secrets)
                .unwrap()
                .contains("network_key")
        );
        app.update();
        assert!(!menu(&app).ant.unwrap().key_set);
        let _ = std::fs::remove_dir_all(files.dir());
    }

    /// Two straps (a BLE and an ANT+ one, spec 14 Phase 17 Q6 (c)): only
    /// one heart rate is used – the first that sent, until the rider picks
    /// the other with *Use for heart rate*; the choice is saved; when the
    /// chosen strap goes, the heart rate goes too (no switching to the
    /// other).
    #[test]
    fn two_straps_give_one_heart_rate_chosen_by_hand() {
        let files = temp_config("two");
        let devices_file = files.dir().join("devices.toml");
        let mut known = KnownDevices::default();
        known.remember(KnownDevice {
            id: STRAP.into(),
            name: "TICKR".into(),
            kind: Some("hr".into()),
            transport: None,
        });
        known.remember(KnownDevice {
            id: ANT_STRAP_2.into(),
            name: "ANT+ heart rate 23456".into(),
            kind: Some("hr".into()),
            transport: Some("ant".into()),
        });
        known.save(&devices_file).unwrap();
        files
            .secret("ant")
            .write("network_key = \"0A1B2C3D4E5F6071\"\n")
            .unwrap();
        let sim = SimTransport::new(Some(NetworkKey::TEST), vec![SimSensor::strap(23456, 128.0)]);
        let mock = Mock::default();
        let io = ant_io(&mock, &sim, None);
        let path = devices_file.to_str().unwrap().to_owned();
        let mut app = app_in(&["--devices-file", &path], Some(io), Some(files.clone()));
        app.update();
        // The BLE strap connects and sends first.
        mock.send(
            STRAP,
            DeviceEvent::Connected {
                kind: "heart-rate sensor",
            },
        );
        let ble = |bpm| DeviceEvent::HeartRate {
            bpm,
            timestamp: Duration::ZERO,
        };
        mock.send(STRAP, ble(100.0));
        app.update();
        assert_eq!(hr_now(&app), Some(100.0));
        // The ANT+ strap connects (the key came from secrets.toml) …
        until(&mut app, "the ANT+ strap", |m| {
            m.entries
                .iter()
                .any(|e| e.id == ANT_STRAP_2 && matches!(e.link, Link::Connected(_)))
        });
        // … but its 128 bpm are not mixed in.
        for _ in 0..40 {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(hr_now(&app), Some(100.0));
        let m = menu(&app);
        let flag =
            |m: &DevicesInfo, id: &str| m.entries.iter().find(|e| e.id == id).unwrap().heart_rate;
        assert_eq!(flag(&m, STRAP), Some(true));
        assert_eq!(flag(&m, ANT_STRAP_2), Some(false));
        // The ANT+ strap's page: Use for heart rate.
        let i = m.entries.iter().position(|e| e.id == ANT_STRAP_2).unwrap();
        let rows = ocw_client::menu::devices::rows(Page::Device(i), &m);
        let use_it = rows
            .iter()
            .find(|r| r.label == "Use for heart rate")
            .unwrap();
        let Some(ocw_client::menu::pages::Activate::Command(line)) = &use_it.confirm else {
            panic!("{use_it:?}")
        };
        assert_eq!(
            run(&mut app, line),
            "ANT+ heart rate 23456 gives the heart rate"
        );
        until_app(&mut app, "128 bpm", 5, |a| hr_now(a) == Some(128.0));
        mock.send(STRAP, ble(100.0));
        app.update();
        assert_eq!(hr_now(&app), Some(128.0));
        assert_eq!(
            KnownDevices::load(&devices_file)
                .unwrap()
                .heart_rate
                .as_deref(),
            Some(ANT_STRAP_2)
        );
        assert_eq!(flag(&menu(&app), ANT_STRAP_2), Some(true));
        // The chosen strap is taken off: no heart rate, not the BLE one's.
        sim.radio(|r| r.set_present(23456, false));
        until_app(&mut app, "the strap gone", 5, |a| hr_now(a).is_none());
        mock.send(STRAP, ble(101.0));
        app.update();
        assert_eq!(hr_now(&app), None);
        // Back on: its heart rate again.
        sim.radio(|r| r.set_present(23456, true));
        until_app(&mut app, "back", 5, |a| hr_now(a) == Some(128.0));
        let _ = std::fs::remove_dir_all(files.dir());
    }

    const ANT_TRAINER: &str = "ant:17:5005:5";

    fn power_now(app: &App) -> f64 {
        app.world().resource::<ClientState>().0.power_w()
    }

    /// A BLE trainer and an ANT+ FE-C trainer (spec 14 Phase 17, 17E, the
    /// Q6 (c) rule for trainers): only one trainer's power counts and only
    /// it gets SIM / ERG – the first that sent, until the rider picks the
    /// other with *Use as trainer*; it takes the current target at once,
    /// and the choice is saved.
    #[test]
    fn two_trainers_one_is_used_chosen_by_hand() {
        use ocw_devices::ant::sim::SimLoad;
        let files = temp_config("two-trainers");
        let devices_file = files.dir().join("devices.toml");
        let mut known = KnownDevices::default();
        known.remember(KnownDevice {
            id: KICKR.into(),
            name: "Wahoo KICKR 0001".into(),
            kind: Some("trainer".into()),
            transport: None,
        });
        known.remember(KnownDevice {
            id: ANT_TRAINER.into(),
            name: "ANT+ trainer 5005".into(),
            kind: Some("trainer".into()),
            transport: Some("ant".into()),
        });
        known.save(&devices_file).unwrap();
        files
            .secret("ant")
            .write("network_key = \"0A1B2C3D4E5F6071\"\n")
            .unwrap();
        let sim = SimTransport::new(
            Some(NetworkKey::TEST),
            vec![SimSensor::fe_c_trainer(5005, 150.0, 90.0)],
        );
        let mock = Mock::default();
        let io = ant_io(&mock, &sim, None);
        let path = devices_file.to_str().unwrap().to_owned();
        let mut app = app_in(&["--devices-file", &path], Some(io), Some(files.clone()));
        app.update();
        // The test sends the targets itself.
        assert_eq!(run(&mut app, "trainer.mode off"), "trainer mode off");
        // The BLE trainer connects and sends first.
        mock.send(
            KICKR,
            DeviceEvent::Connected {
                kind: "smart trainer (FTMS)",
            },
        );
        let ble = |watts| DeviceEvent::Power {
            watts,
            cadence_rpm: Some(85.0),
            timestamp: Duration::ZERO,
        };
        mock.send(KICKR, ble(200.0));
        app.update();
        assert_eq!(power_now(&app), 200.0);
        // The FE-C trainer connects through the simulated stick …
        until(&mut app, "the FE-C trainer", |m| {
            m.entries
                .iter()
                .any(|e| e.id == ANT_TRAINER && matches!(e.link, Link::Connected(_)))
        });
        // … but its 150 W are not mixed in, and ERG goes to the KICKR only.
        for _ in 0..40 {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(power_now(&app), 200.0);
        let m = menu(&app);
        let flag =
            |m: &DevicesInfo, id: &str| m.entries.iter().find(|e| e.id == id).unwrap().trainer;
        assert_eq!(flag(&m, KICKR), Some(true));
        assert_eq!(flag(&m, ANT_TRAINER), Some(false));
        mock.commands(KICKR);
        app.world()
            .resource::<Devices>()
            .command_all(DeviceCommand::SetTargetPower(230.0));
        assert!(
            mock.commands(KICKR)
                .contains(&DeviceCommand::SetTargetPower(230.0))
        );
        std::thread::sleep(Duration::from_millis(600));
        // (It may have had the first SIM target, sent before any trainer
        // had sent power.)
        assert_ne!(
            sim.radio(|r| r.trainer(5005).unwrap().load),
            SimLoad::Erg(230.0),
            "the FE-C trainer is not told"
        );
        // The FE-C trainer's page: Use as trainer.
        let i = m.entries.iter().position(|e| e.id == ANT_TRAINER).unwrap();
        let rows = ocw_client::menu::devices::rows(Page::Device(i), &m);
        let use_it = rows.iter().find(|r| r.label == "Use as trainer").unwrap();
        let Some(ocw_client::menu::pages::Activate::Command(line)) = &use_it.confirm else {
            panic!("{use_it:?}")
        };
        assert_eq!(run(&mut app, line), "ANT+ trainer 5005 is the trainer");
        // It takes the current target at once, and its power counts.
        until_app(&mut app, "ERG on the FE-C trainer", 5, |_| {
            sim.radio(|r| r.trainer(5005).unwrap().load) == SimLoad::Erg(230.0)
        });
        until_app(&mut app, "230 W", 8, |a| (power_now(a) - 230.0).abs() < 3.0);
        mock.send(KICKR, ble(200.0));
        app.update();
        assert!(
            (power_now(&app) - 230.0).abs() < 3.0,
            "the KICKR's power is not used"
        );
        // The next target: the FE-C trainer only.
        mock.commands(KICKR);
        app.world()
            .resource::<Devices>()
            .command_all(DeviceCommand::SetTargetPower(260.0));
        assert!(mock.commands(KICKR).is_empty());
        until_app(&mut app, "260 W asked", 5, |_| {
            sim.radio(|r| r.trainer(5005).unwrap().load) == SimLoad::Erg(260.0)
        });
        assert_eq!(
            KnownDevices::load(&devices_file)
                .unwrap()
                .trainer
                .as_deref(),
            Some(ANT_TRAINER)
        );
        assert_eq!(flag(&menu(&app), ANT_TRAINER), Some(true));
        assert_eq!(sim.radio(|r| r.busy_refusals), 0);
        let _ = std::fs::remove_dir_all(files.dir());
    }

    /// The Devices page says what is wrong with the stick, in neutral
    /// words; the fix for the system is a row of its own (below).
    #[test]
    fn a_stick_that_cannot_be_opened_is_named_neutrally() {
        let sim = SimTransport::new(None, vec![SimSensor::strap(1, 70.0)]);
        sim.set_denied(true);
        let mock = Mock::default();
        let io = ant_io(&mock, &sim, Some(NetworkKey::TEST));
        let mut app = app_with(&[], Some(io));
        run(&mut app, "devices.connect ant:120:1:1");
        until(&mut app, "permission", |m| {
            m.ant
                .as_ref()
                .is_some_and(|a| a.state == "cannot open the stick (permission)")
        });
        until(&mut app, "the entry says why", |m| {
            m.entries.first().is_some_and(|e| {
                e.link == Link::Failed(ocw_devices::FailReason::NoPermission.text().into())
            })
        });
        let status = StickStatus::NoPermission("x".into()).text();
        assert!(!status.contains("udev") && !status.contains("Zadig"));
    }

    /// A fake runner without pkexec: the real one is never run in tests.
    struct NoPkexec(std::sync::atomic::AtomicUsize);

    impl crate::ant_access::Runner for NoPkexec {
        fn find(&self, program: &str) -> Option<std::path::PathBuf> {
            assert_eq!(program, "pkexec");
            self.0.fetch_add(1, Ordering::Relaxed);
            None
        }
        fn run(&self, _: &std::path::Path, _: &[String]) -> std::io::Result<Option<i32>> {
            panic!("pkexec must not run in tests");
        }
    }

    /// Linux, no permission: the ANT+ page offers *Install access for the
    /// ANT stick* (spec 14 Phase 17 Q8 (b)); without pkexec it refuses and
    /// the console shows the rule and the manual command. Windows: the
    /// driver page (Q7 (d)).
    #[test]
    fn a_stick_without_permission_offers_the_fix_of_the_system() {
        use ocw_client::menu::devices::{ANT_ACCESS_ROW, rows};
        // The rule by system.
        let denied = StickStatus::NoPermission("ANT USB-m stick".into());
        let no_driver = StickStatus::NoDriver("ANT USB-m stick".into());
        assert_eq!(ant_fix(&denied, "linux"), Some(AntFix::LinuxAccess));
        assert_eq!(ant_fix(&no_driver, "windows"), Some(AntFix::WindowsDriver));
        assert_eq!(ant_fix(&denied, "windows"), Some(AntFix::WindowsDriver));
        assert_eq!(ant_fix(&denied, "macos"), None);
        assert_eq!(ant_fix(&StickStatus::Busy("x".into()), "linux"), None);
        assert_eq!(ant_fix(&StickStatus::NoStick, "windows"), None);
        assert_eq!(no_driver.text(), "the stick needs a driver");
        if std::env::consts::OS != "linux" {
            return;
        }
        let sim = SimTransport::new(None, vec![SimSensor::strap(1, 70.0)]);
        sim.set_denied(true);
        let mock = Mock::default();
        let io = ant_io(&mock, &sim, Some(NetworkKey::TEST));
        let mut app = app_with(&[], Some(io));
        let dir = std::env::temp_dir().join(format!("ocw-ant-access-{}", std::process::id()));
        let fake = Arc::new(NoPkexec(Default::default()));
        app.insert_resource(AntAccess {
            runner: fake.clone(),
            path: dir.join("60-ocw-ant.rules"),
        });
        run(&mut app, "devices.connect ant:120:1:1");
        until(&mut app, "the fix is offered", |m| {
            m.ant
                .as_ref()
                .is_some_and(|a| a.fix == Some(AntFix::LinuxAccess))
        });
        let r = rows(Page::Ant, &menu(&app));
        assert!(r.iter().any(|r| r.label == ANT_ACCESS_ROW), "{r:?}");
        // Install access: refused cleanly without pkexec.
        let line = run(&mut app, "ant.access.install");
        assert!(line.contains("pkexec"), "{line}");
        until(&mut app, "the outcome", |m| {
            m.ant
                .as_ref()
                .and_then(|a| a.access.as_deref())
                .is_some_and(|a| a.starts_with("no pkexec"))
        });
        assert_eq!(fake.0.load(Ordering::Relaxed), 1);
        let log = app.world().resource::<ConsoleState>().log().join("\n");
        let [usb, tty] = crate::ant_access::rule_lines();
        assert!(log.contains(&usb) && log.contains(&tty), "{log}");
        assert!(log.contains("sudo udevadm control --reload-rules"), "{log}");
        assert!(!dir.join("60-ocw-ant.rules").exists());
        let r = rows(Page::AntAccess, &menu(&app));
        assert!(r.iter().any(|r| r.label == "Last try"), "{r:?}");
    }
}

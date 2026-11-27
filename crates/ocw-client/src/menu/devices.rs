//! Settings → Trainer → Devices (spec §3.2.1 *Devices*, §3.4), Bevy-free
//! and shared by both apps: scan (desktop) or the browser's chooser, the
//! devices found, connected and remembered, one page per device to
//! connect or forget it, and the status lines.
//!
//! The apps fill a [`DevicesInfo`] from their own device layer (the
//! desktop's `devices.rs` over btleplug, the browser's `WebDevices` over
//! Web Bluetooth); every row is a console line (`devices.scan`,
//! `devices.connect <id>`, `devices.forget <id>`), so the menu, the
//! console and scripts share one path.
//!
//! The list keeps its order while it is open: the apps only append new
//! devices (a device's page is its index), and forgetting goes back to
//! the list.

use ocw_devices::ble::Profile;
use ocw_devices::ble::web::DeviceKind;

use super::pages::{Activate, Page, Row, info_row, open_row, short_label};

/// How long the menu's scan runs (s).
pub const SCAN_SECONDS: u32 = 10;

/// The Devices page's note.
pub const NOTE: &str = "Choose a device to connect it; once connected it is remembered.";

/// A device page's note.
pub const DEVICE_NOTE: &str =
    "Connect it in its role, or forget it: it is then not connected at the next start.";

/// The ANT+ page's note (spec 3.4.1, 14 Phase 17 Q1 (b)).
pub const ANT_NOTE: &str = "ANT+ sensors need the ANT+ network key (16 hex digits), which Garmin publishes for ANT+ developers. It stays on this device (Kept in) and is never sent.";

/// Devices -> ANT+ -> Driver for the ANT stick (Windows; spec 3.4.1 *The
/// driver route on Windows*, 14 Phase 17 Q7 (d)).
pub const ANT_DRIVER_NOTE: &str = "Needs Garmin's ANT driver (as for Zwift): Windows Update > Optional updates, then plug the stick in again. Fallback: WinUSB with Zadig.";

/// Devices -> ANT+ -> Install access (Linux; 14 Phase 17 Q8 (b)).
pub const ANT_ACCESS_NOTE: &str = "Linux opens the stick only with a udev rule. Install access asks for your password (pkexec) and adds it; then plug the stick in again.";

/// The command that installs the Linux access rule (`ocw-desktop
/// --install-udev-rule` in a terminal does the same).
pub const ANT_ACCESS_COMMAND: &str = "ant.access.install";

/// The ANT+ page's status line while the key is typed.
pub const ANT_TYPING: &str = "Type the 16 hex digits; Enter saves, Esc cancels";

/// What a device is for the ride.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A smart trainer (FTMS, or a Wahoo KICKR): power and resistance.
    Trainer,
    /// A power meter (Cycling Power).
    Power,
    /// A speed / cadence sensor (CSC).
    Cadence,
    HeartRate,
}

impl Role {
    /// The name in `devices.toml` and on the console.
    pub fn key(self) -> &'static str {
        match self {
            Self::Trainer => "trainer",
            Self::Power => "power",
            Self::Cadence => "cadence",
            Self::HeartRate => "hr",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "trainer" => Some(Self::Trainer),
            "power" | "power-meter" => Some(Self::Power),
            "cadence" | "speed" => Some(Self::Cadence),
            "hr" | "heart-rate" | "heartrate" => Some(Self::HeartRate),
            _ => None,
        }
    }

    /// For rows and messages.
    pub fn label(self) -> &'static str {
        match self {
            Self::Trainer => "trainer",
            Self::Power => "power meter",
            Self::Cadence => "cadence / speed",
            Self::HeartRate => "heart rate",
        }
    }

    /// Whether it gives the rider's power (it replaces the virtual
    /// trainer).
    pub fn delivers_power(self) -> bool {
        matches!(self, Self::Trainer | Self::Power)
    }

    /// From what a device advertises: FTMS is a trainer; Wahoo KICKRs
    /// advertise only Cycling Power (their control point is found after
    /// connecting, spec §3.4), so the name tells them apart from power
    /// meters. `None`: no fitness profile.
    pub fn of(name: Option<&str>, profiles: &[Profile]) -> Option<Self> {
        let kickr = name.is_some_and(|n| n.to_ascii_uppercase().contains("KICKR"));
        if profiles.contains(&Profile::FitnessMachine)
            || (kickr && profiles.contains(&Profile::CyclingPower))
        {
            Some(Self::Trainer)
        } else if profiles.contains(&Profile::CyclingPower) {
            Some(Self::Power)
        } else if profiles.contains(&Profile::SpeedCadence) {
            Some(Self::Cadence)
        } else if profiles.contains(&Profile::HeartRate) {
            Some(Self::HeartRate)
        } else {
            None
        }
    }

    /// The browser's chooser kinds.
    pub fn of_kind(k: DeviceKind) -> Self {
        match k {
            DeviceKind::Trainer => Self::Trainer,
            DeviceKind::PowerMeter => Self::Power,
            DeviceKind::HeartRate => Self::HeartRate,
        }
    }

    /// The browser's chooser for this role.
    pub fn kind(self) -> DeviceKind {
        match self {
            Self::Trainer => DeviceKind::Trainer,
            Self::Power | Self::Cadence => DeviceKind::PowerMeter,
            Self::HeartRate => DeviceKind::HeartRate,
        }
    }
}

/// A device's connection.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Link {
    /// Not connected (found by a scan, or remembered).
    #[default]
    Idle,
    Connecting,
    /// Connected: what the device turned out to be ("Wahoo KICKR (Cycling
    /// Power + Wahoo control)").
    Connected(String),
    /// Lost; reconnecting.
    Reconnecting,
    /// The last attempt failed (why); the next one follows.
    Failed(String),
}

impl Link {
    pub fn text(&self) -> String {
        match self {
            Self::Idle => "not connected".into(),
            Self::Connecting => "connecting ...".into(),
            Self::Connected(k) => format!("connected, {k}"),
            Self::Reconnecting => "reconnecting ...".into(),
            Self::Failed(why) => format!("failed: {why}; trying again"),
        }
    }

    /// Short, for the list.
    fn short(&self) -> Option<&'static str> {
        match self {
            Self::Idle => None,
            Self::Connecting => Some("connecting"),
            Self::Connected(_) => Some("connected"),
            Self::Reconnecting => Some("reconnecting"),
            Self::Failed(_) => Some("failed"),
        }
    }

    /// Whether a backend runs for it.
    pub fn running(&self) -> bool {
        !matches!(self, Self::Idle)
    }
}

/// One device of the list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The platform's id (`devices.connect <id>`).
    pub id: String,
    pub name: String,
    pub role: Role,
    /// Signal strength (dBm) from the last scan.
    pub rssi: Option<i16>,
    pub link: Link,
    /// In the remembered devices (`devices.toml`, the browser's
    /// `localStorage`).
    pub remembered: bool,
    /// With two or more heart-rate sources (spec 14 Phase 17 Q6 (c)):
    /// whether this one gives the heart rate (*Use for heart rate*);
    /// `None` otherwise.
    pub heart_rate: Option<bool>,
    /// With two or more trainers (a BLE and an ANT+ FE-C one, spec 14
    /// Phase 17, 17E): whether this one is the trainer in use (*Use as
    /// trainer*); `None` otherwise.
    pub trainer: Option<bool>,
}

impl Entry {
    /// An ANT+ sensor (its id `ant:…`), not Bluetooth.
    pub fn is_ant(&self) -> bool {
        self.id.starts_with("ant:")
    }

    /// The list's value: "trainer, -61 dBm, connected, remembered";
    /// "heart rate, ANT+, connected, HR in use" (spec 3.4.1 *Device
    /// menu*).
    pub fn summary(&self) -> String {
        let mut parts = vec![self.role.label().to_owned()];
        if self.is_ant() {
            parts.push("ANT+".into());
        }
        if let Some(r) = self.rssi {
            parts.push(format!("{r} dBm"));
        }
        if let Some(s) = self.link.short() {
            parts.push(s.into());
        }
        if self.heart_rate == Some(true) {
            parts.push("HR in use".into());
        }
        if self.trainer == Some(true) {
            parts.push("in use".into());
        }
        if self.remembered {
            parts.push("remembered".into());
        }
        parts.join(", ")
    }
}

/// How devices are found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Scan {
    /// Desktop: no scan running.
    #[default]
    Idle,
    /// Desktop: a scan runs, this long still.
    Scanning { left_s: u32 },
    /// The browser: its chooser lists the devices (`None`), or there is
    /// no Web Bluetooth (why).
    Chooser(Option<String>),
}

/// The sensor bridge page's note (spec 14 Phase 10, 10D.2).
pub const BRIDGE_NOTE: &str = "The desktop app keeps the trainer on Bluetooth and passes it to one browser on the same network.";

/// The sensor bridge as the Devices pages show it (spec 14 Phase 10,
/// 10D.2 *Built (local bridge)*).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BridgeInfo {
    /// The desktop: sharing its trainer with a browser.
    Host {
        /// Sharing (the endpoint runs).
        sharing: bool,
        /// The pairing code (`7K2M-Q9XP-4DA`).
        code: Option<String>,
        /// `https://192.168.1.23:47810`.
        address: Option<String>,
        /// The paired browser (what it says it is).
        browser: Option<String>,
        /// Why sharing could not start.
        error: Option<String>,
    },
    /// The browser: riding with the trainer of another device.
    Client {
        /// The code in use.
        code: Option<String>,
        /// "connecting", "paired with …", "refused: …".
        state: String,
        /// The browser has WebTransport (else the bridge cannot work here).
        supported: bool,
    },
}

impl BridgeInfo {
    /// The Devices list row's value.
    fn value(&self) -> String {
        match self {
            Self::Host {
                sharing: false,
                error: Some(_),
                ..
            } => "failed".into(),
            Self::Host { sharing: false, .. } => "off".into(),
            Self::Host {
                browser: Some(b), ..
            } => short_label(&format!("paired, {b}"), 24),
            Self::Host { code: Some(c), .. } => format!("on, {c}"),
            Self::Host { .. } => "on".into(),
            Self::Client {
                supported: false, ..
            } => "not in this browser".into(),
            Self::Client { code: None, .. } => "off".into(),
            Self::Client { state, .. } => short_label(state, 24),
        }
    }
}

/// ANT+ on the Devices pages (spec 3.4.1, 14 Phase 17; the desktop, and
/// the browser over WebUSB: [`AntInfo::web`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct AntInfo {
    /// The stick: "ANT USB-m stick, in use", "no stick", "cannot open the
    /// stick (permission)", "network key needed".
    pub state: String,
    /// The ANT+ network key is entered.
    pub key_set: bool,
    /// While the key is typed: the field as shown (masked, with the
    /// cursor and the count of digits).
    pub typing: Option<String>,
    /// Where the key is kept ("~/.config/ocw/secrets.toml [ant]").
    pub key_file: Option<String>,
    /// The stick cannot be opened, and this is how to fix it here.
    pub fix: Option<AntFix>,
    /// The last *Install access* run ("installed", "pkexec is missing:
    /// ...").
    pub access: Option<String>,
    /// The browser's stick over WebUSB (spec 14 Phase 17 A5; `None`: the
    /// desktop's stick service).
    pub web: Option<WebUsbInfo>,
}

/// ANT+ in the browser (spec 14 Phase 17 A5, 17E *The browser*): the
/// stick over WebUSB.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WebUsbInfo {
    /// The browser has WebUSB (`navigator.usb`: Chrome, Edge and the other
    /// Chromium browsers on the desktop and on Android); without it the
    /// rows say why (Firefox, Safari, iOS).
    pub supported: bool,
    /// The browser runs on Windows: the stick needs the WinUSB driver
    /// there (Zadig; Q7) – Garmin's driver is not one WebUSB can use.
    pub windows: bool,
    /// A stick was granted to this site (the chooser, now or before).
    pub granted: bool,
    /// A search for ANT+ sensors runs: seconds left.
    pub scanning: Option<u32>,
}

/// The Devices list's row that opens the browser's chooser for a stick.
pub const ANT_ADD_ROW: &str = "Add an ANT+ stick";
/// The command behind it (`navigator.usb.requestDevice` in the key press's
/// user gesture).
pub const ANT_ADD_COMMAND: &str = "ant.stick.add";
/// The browser's search for ANT+ sensors through the stick.
pub const ANT_SCAN_ROW: &str = "Find ANT+ sensors";

/// How to make a stick that cannot be opened work (by the system).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AntFix {
    /// Windows: Garmin's driver from Windows Update, or Zadig / WinUSB
    /// (Q7 (d)).
    WindowsDriver,
    /// Linux: the udev rule through `pkexec` (Q8 (b)).
    LinuxAccess,
}

/// The ANT+ page's row for the fix.
pub const ANT_DRIVER_ROW: &str = "Driver for the ANT stick";
/// See [`ANT_DRIVER_ROW`].
pub const ANT_ACCESS_ROW: &str = "Install access for the ANT stick";

impl AntInfo {
    /// The Devices list row's value.
    fn value(&self) -> String {
        if self.key_set {
            short_label(&self.state, 28)
        } else {
            "network key needed".into()
        }
    }
}

/// What the Devices pages show (filled by the app).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DevicesInfo {
    pub scan: Scan,
    pub entries: Vec<Entry>,
    /// The latest lines (connects, failures, scan results), oldest first.
    pub messages: Vec<String>,
    /// Where the remembered devices are kept.
    pub file: Option<String>,
    /// The virtual trainer gives the power now.
    pub virtual_trainer: bool,
    /// The sensor bridge (`None`: the app has none).
    pub bridge: Option<BridgeInfo>,
    /// ANT+ (`None`: the app has none, the browser).
    pub ant: Option<AntInfo>,
}

/// Messages a Devices page shows (the newest).
pub const MESSAGES_SHOWN: usize = 3;

impl DevicesInfo {
    /// Keeps the newest [`MESSAGES_SHOWN`] lines.
    pub fn push_message(&mut self, line: impl Into<String>) {
        self.messages.push(line.into());
        let n = self.messages.len();
        if n > MESSAGES_SHOWN {
            self.messages.drain(..n - MESSAGES_SHOWN);
        }
    }

    /// The Trainer page's Devices value: the connected devices, else the
    /// virtual trainer.
    pub fn trainer_value(&self) -> String {
        let connected: Vec<&str> = self
            .entries
            .iter()
            .filter(|e| matches!(e.link, Link::Connected(_)))
            .map(|e| e.name.as_str())
            .collect();
        match connected.as_slice() {
            [] if matches!(self.scan, Scan::Scanning { .. }) => "scanning".into(),
            [] if self.virtual_trainer => "virtual trainer".into(),
            [] => "none connected".into(),
            [one] => short_label(one, 20),
            [one, rest @ ..] => format!("{} +{}", short_label(one, 16), rest.len()),
        }
    }
}

/// The rows of the Devices pages.
pub fn rows(page: Page, info: &DevicesInfo) -> Vec<Row> {
    match page {
        Page::Devices => list_rows(info),
        Page::Device(i) => match info.entries.get(i) {
            Some(e) => device_rows(e, info),
            None => Vec::new(),
        },
        Page::Bridge => info.bridge.as_ref().map(bridge_rows).unwrap_or_default(),
        Page::Ant => info.ant.as_ref().map(ant_rows).unwrap_or_default(),
        Page::AntDriver => info.ant.as_ref().map(driver_rows).unwrap_or_default(),
        Page::AntAccess => info.ant.as_ref().map(access_rows).unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Devices → ANT+: the stick, the network key (enter, change, remove).
fn ant_rows(a: &AntInfo) -> Vec<Row> {
    if let Some(w) = &a.web
        && !w.supported
    {
        return no_web_usb_rows(w);
    }
    let mut out = vec![info_row("Stick", Some(short_label(&a.state, 40)))];
    if let Some(w) = &a.web {
        out.push(add_stick_row(w));
        if w.windows {
            out.push(info_row("On Windows", Some("needs WinUSB (Zadig)".into())));
        }
        if a.fix == Some(AntFix::LinuxAccess) {
            out.push(info_row(
                "On Linux",
                Some("needs the udev rule (README)".into()),
            ));
        }
    }
    // The desktop's fixes (the browser has its rows above).
    let fix = if a.web.is_some() { None } else { a.fix };
    match fix {
        Some(AntFix::WindowsDriver) => {
            out.push(open_row(ANT_DRIVER_ROW, None, Page::AntDriver));
        }
        Some(AntFix::LinuxAccess) => {
            out.push(open_row(ANT_ACCESS_ROW, None, Page::AntAccess));
        }
        None => {}
    }
    match &a.typing {
        Some(shown) => {
            let mut field = Row::action("Network key", "ant.key.save");
            field.value = Some(shown.clone());
            out.push(field);
            out.push(Row::action("Save the key", "ant.key.save"));
            out.push(Row::action("Paste the key", "ant.key.paste"));
            out.push(Row::action("Cancel", "ant.key.cancel"));
        }
        None => {
            let label = if a.key_set {
                "Change the network key"
            } else {
                "Enter the network key"
            };
            let mut r = Row::action(label, "ant.key.edit");
            r.value = Some(if a.key_set { "set" } else { "not set" }.into());
            out.push(r);
            if a.key_set {
                out.push(Row::action("Remove the key", "ant.key.clear"));
            }
        }
    }
    if let Some(w) = &a.web
        && a.key_set
        && w.granted
    {
        out.push(scan_row(w));
    }
    if let Some(f) = &a.key_file {
        out.push(info_row("Kept in", Some(short_label(f, 40))));
    }
    out
}

/// *Add an ANT+ stick* (the browser): the chooser, or why there is none.
fn add_stick_row(w: &WebUsbInfo) -> Row {
    if !w.supported {
        return open_row(ANT_ADD_ROW, Some("no WebUSB here".into()), Page::Ant);
    }
    let mut r = Row::action(ANT_ADD_ROW, ANT_ADD_COMMAND);
    r.value = Some(if w.granted { "added" } else { "Chrome, Edge" }.into());
    r
}

/// *Find ANT+ sensors* (the browser): a 10 s search, or stop it.
fn scan_row(w: &WebUsbInfo) -> Row {
    match w.scanning {
        Some(left) => {
            let mut r = Row::action("Stop the search", "ant.scan --stop");
            r.value = Some(format!("{left} s left"));
            r
        }
        None => {
            let mut r = Row::action(ANT_SCAN_ROW, "ant.scan");
            r.value = Some(format!("{SCAN_SECONDS} s"));
            r
        }
    }
}

/// Devices -> ANT+ in a browser without WebUSB: why, and what works.
fn no_web_usb_rows(w: &WebUsbInfo) -> Vec<Row> {
    let mut out = vec![
        info_row("WebUSB", Some("not in this browser".into())),
        info_row("Firefox, Safari, iOS", Some("have no WebUSB".into())),
        info_row("Use Chrome or Edge", Some("desktop or Android".into())),
        info_row("Or the desktop app", Some("its sensor bridge".into())),
    ];
    if w.windows {
        out.push(info_row("On Windows", Some("needs WinUSB (Zadig)".into())));
    }
    out
}

/// Devices -> ANT+ -> Driver for the ANT stick (Windows): the two routes
/// (the note explains them); the status line has the stick's state.
fn driver_rows(_: &AntInfo) -> Vec<Row> {
    vec![
        info_row(
            "1. Windows Update",
            Some("Optional updates: Garmin's ANT driver".into()),
        ),
        info_row(
            "2. Or Zadig",
            Some("WinUSB (Zwift then loses the stick)".into()),
        ),
        info_row(
            "One program at a time",
            Some("close Zwift, Garmin Express".into()),
        ),
    ]
}

/// Devices -> ANT+ -> Install access (Linux): install the udev rule, and
/// what the last try said.
fn access_rows(a: &AntInfo) -> Vec<Row> {
    let mut out = vec![
        Row::action(
            "Install access (asks for your password)",
            ANT_ACCESS_COMMAND,
        ),
        info_row(
            "Rule file",
            Some("/etc/udev/rules.d/60-ocw-ant.rules".into()),
        ),
    ];
    if let Some(r) = &a.access {
        out.push(info_row("Last try", Some(short_label(r, 40))));
    }
    out
}

/// The bridge row of the Devices list: the desktop shares, the browser
/// uses another device's trainer.
fn bridge_row(b: &BridgeInfo) -> Row {
    let label = match b {
        BridgeInfo::Host { .. } => "Share with a browser",
        BridgeInfo::Client { .. } => "Use a trainer from another device",
    };
    open_row(label, Some(b.value()), Page::Bridge)
}

fn bridge_rows(b: &BridgeInfo) -> Vec<Row> {
    match b {
        BridgeInfo::Host {
            sharing,
            code,
            address,
            browser,
            error,
        } => {
            let mut toggle = Row::action(
                "Share with a browser",
                if *sharing {
                    "bridge.share off"
                } else {
                    "bridge.share on"
                },
            );
            toggle.value = Some(if *sharing { "on" } else { "off" }.into());
            let mut out = vec![toggle];
            if *sharing {
                out.push(info_row("Pairing code", code.clone()));
                out.push(info_row("Address", address.clone()));
                out.push(info_row(
                    "Browser",
                    Some(
                        browser
                            .as_deref()
                            .map_or("none yet".into(), |b| short_label(b, 28)),
                    ),
                ));
                out.push(Row::action("New code", "bridge.share new-code"));
            } else if let Some(e) = error {
                out.push(info_row("Not sharing", Some(short_label(e, 40))));
            }
            out
        }
        BridgeInfo::Client {
            code,
            state,
            supported,
        } => {
            if !supported {
                return vec![info_row(
                    "Not in this browser",
                    Some("no WebTransport".into()),
                )];
            }
            let mut out = vec![Row::action("Enter the pairing code", "bridge.pair")];
            if code.is_some() {
                out.push(info_row("Code", code.clone()));
                out.push(info_row("State", Some(short_label(state, 40))));
                out.push(Row::action("Stop using it", "bridge.stop"));
            }
            out
        }
    }
}

fn list_rows(info: &DevicesInfo) -> Vec<Row> {
    let mut out = Vec::new();
    match &info.scan {
        Scan::Idle => {
            let mut r = Row::action("Scan for devices", "devices.scan");
            r.value = Some(format!("{SCAN_SECONDS} s"));
            out.push(r);
        }
        Scan::Scanning { left_s } => {
            let mut r = Row::action("Stop the scan", "devices.scan --stop");
            r.value = Some(format!("{left_s} s left"));
            out.push(r);
        }
        Scan::Chooser(missing) => {
            for (label, role) in [
                ("Add a trainer", Role::Trainer),
                ("Add a power meter", Role::Power),
                ("Add a heart-rate strap", Role::HeartRate),
            ] {
                out.push(match missing {
                    None => Row::action(label, connect_kind(role)),
                    Some(_) => Row {
                        label: label.into(),
                        value: Some("no Web Bluetooth".into()),
                        left: None,
                        right: None,
                        confirm: Some(Activate::Disabled),
                    },
                });
            }
        }
    }
    if let Some(b) = &info.bridge {
        out.push(bridge_row(b));
    }
    if let Some(a) = &info.ant {
        match &a.web {
            // The browser: the chooser's row first; without WebUSB it
            // opens the page that says why.
            Some(w) => {
                out.push(add_stick_row(w));
                if w.supported {
                    out.push(open_row("ANT+ (USB stick)", Some(a.value()), Page::Ant));
                    if a.key_set && w.granted {
                        out.push(scan_row(w));
                    }
                }
            }
            None => out.push(open_row("ANT+ (USB stick)", Some(a.value()), Page::Ant)),
        }
    }
    out.extend(info.entries.iter().enumerate().map(|(i, e)| {
        open_row(
            &short_label(&e.name, 24),
            Some(e.summary()),
            Page::Device(i),
        )
    }));
    if info.entries.is_empty() {
        out.push(info_row(
            "No devices yet",
            Some(match info.scan {
                Scan::Idle => "scan to find them".into(),
                Scan::Scanning { .. } => "looking ...".into(),
                Scan::Chooser(_) => "add one above".into(),
            }),
        ));
    }
    out
}

/// `devices.connect trainer|power|hr`: the browser's chooser.
fn connect_kind(role: Role) -> String {
    format!("devices.connect {}", role.kind().key())
}

fn device_rows(e: &Entry, info: &DevicesInfo) -> Vec<Row> {
    let mut out = vec![
        info_row("Kind", Some(e.role.label().into())),
        info_row("State", Some(short_label(&e.link.text(), 40))),
    ];
    if !matches!(info.scan, Scan::Chooser(_)) {
        out.push(info_row(
            "Signal",
            Some(e.rssi.map_or("-".into(), |r| format!("{r} dBm"))),
        ));
    }
    out.push(info_row(
        "Remembered",
        Some(if e.remembered { "yes" } else { "no" }.into()),
    ));
    // Two heart-rate sources: the rider picks one (spec 14 Phase 17 Q6 (c)).
    if let Some(used) = e.heart_rate {
        let mut r = Row::action("Use for heart rate", format!("devices.hr {}", e.id));
        r.value = Some(if used { "yes" } else { "no" }.into());
        out.push(r);
    }
    // Two trainers: the rider picks one (17E, the Q6 (c) rule).
    if let Some(used) = e.trainer {
        let mut r = Row::action("Use as trainer", format!("devices.trainer {}", e.id));
        r.value = Some(if used { "yes" } else { "no" }.into());
        out.push(r);
    }
    if matches!(e.link, Link::Idle | Link::Failed(_)) {
        let line = match info.scan {
            // The browser connects Bluetooth devices only from its
            // chooser; ANT+ sensors through the stick, by id.
            Scan::Chooser(_) if !e.is_ant() => connect_kind(e.role),
            _ => format!("devices.connect {}", e.id),
        };
        out.push(Row::action("Connect", line));
    }
    if e.remembered || e.link.running() {
        out.push(Row {
            label: "Disconnect and forget".into(),
            value: None,
            left: None,
            right: None,
            confirm: Some(Activate::CommandBack(format!("devices.forget {}", e.id))),
        });
    }
    out
}

/// The status line over the Devices pages: the scan, else the newest
/// message, else where the devices are remembered (a page of seven rows
/// has no room for more text).
pub fn status(page: Page, info: &DevicesInfo) -> String {
    if let Scan::Scanning { left_s } = info.scan {
        return format!("Scanning ... {left_s} s left, {} found", info.entries.len());
    }
    if let Page::Device(i) = page
        && let Some(e) = info.entries.get(i)
    {
        return short_label(&format!("{}: {}", e.name, e.link.text()), 60);
    }
    if matches!(page, Page::Ant | Page::AntDriver | Page::AntAccess) {
        return match (&info.ant, info.messages.last()) {
            (Some(a), _) if a.typing.is_some() => ANT_TYPING.into(),
            (_, Some(m)) => short_label(m, 56),
            (Some(a), None) => short_label(&format!("ANT+: {}", a.state), 56),
            _ => String::new(),
        };
    }
    if page == Page::Bridge {
        return match &info.bridge {
            Some(BridgeInfo::Host {
                sharing: true,
                code: Some(c),
                browser: None,
                ..
            }) => format!("Type {c} in the browser's Devices page"),
            Some(BridgeInfo::Host {
                browser: Some(b), ..
            }) => short_label(&format!("Paired with {b}"), 60),
            Some(BridgeInfo::Host { error: Some(e), .. }) => short_label(e, 60),
            Some(BridgeInfo::Client { state, .. }) => short_label(state, 60),
            _ => String::new(),
        };
    }
    match (info.messages.last(), &info.file) {
        (Some(m), _) => short_label(m, 56),
        (None, Some(f)) => format!("Remembered in {f}"),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu::model::{Action, Effect, Menu, Page as MenuPage};
    use crate::settings::Settings;

    fn kickr(link: Link, remembered: bool) -> Entry {
        Entry {
            id: "hci0/dev_00_00_00_00_00_01".into(),
            name: "Wahoo KICKR 0001".into(),
            role: Role::Trainer,
            rssi: Some(-61),
            link,
            remembered,
            heart_rate: None,
            trainer: None,
        }
    }

    fn label(r: &[Row], i: usize) -> &str {
        &r[i].label
    }

    #[test]
    fn roles_come_from_the_advertised_profiles_and_the_name() {
        use Profile::*;
        assert_eq!(Role::of(None, &[FitnessMachine]), Some(Role::Trainer));
        assert_eq!(
            Role::of(Some("Wahoo KICKR 0001"), &[CyclingPower]),
            Some(Role::Trainer)
        );
        assert_eq!(
            Role::of(Some("Assioma"), &[CyclingPower]),
            Some(Role::Power)
        );
        assert_eq!(Role::of(None, &[SpeedCadence]), Some(Role::Cadence));
        assert_eq!(Role::of(Some("TICKR"), &[HeartRate]), Some(Role::HeartRate));
        assert_eq!(Role::of(None, &[]), None);
        for r in [Role::Trainer, Role::Power, Role::Cadence, Role::HeartRate] {
            assert_eq!(Role::parse(r.key()), Some(r));
        }
        assert!(Role::Trainer.delivers_power() && !Role::HeartRate.delivers_power());
    }

    #[test]
    fn the_list_has_the_scan_and_every_device() {
        let mut info = DevicesInfo::default();
        let r = rows(Page::Devices, &info);
        assert_eq!(label(&r, 0), "Scan for devices");
        assert_eq!(r[0].confirm, Some(Activate::Command("devices.scan".into())));
        assert_eq!(label(&r, 1), "No devices yet");
        // Scanning: the row stops it; found devices show as they come.
        info.scan = Scan::Scanning { left_s: 7 };
        info.entries.push(kickr(Link::Idle, false));
        let r = rows(Page::Devices, &info);
        assert_eq!(label(&r, 0), "Stop the scan");
        assert_eq!(r[0].value.as_deref(), Some("7 s left"));
        assert_eq!(
            r[0].confirm,
            Some(Activate::Command("devices.scan --stop".into()))
        );
        assert_eq!(label(&r, 1), "Wahoo KICKR 0001");
        assert_eq!(r[1].value.as_deref(), Some("trainer, -61 dBm"));
        assert_eq!(r[1].confirm, Some(Activate::Open(Page::Device(0))));
        assert_eq!(r.len(), 2);
        assert_eq!(
            status(Page::Devices, &info),
            "Scanning ... 7 s left, 1 found"
        );
        // Connected and remembered: marked.
        info.entries[0] = kickr(Link::Connected("Wahoo KICKR".into()), true);
        let r = rows(Page::Devices, &info);
        assert_eq!(
            r[1].value.as_deref(),
            Some("trainer, -61 dBm, connected, remembered")
        );
        assert_eq!(info.trainer_value(), "Wahoo KICKR 0001");
    }

    #[test]
    fn a_device_page_connects_and_forgets() {
        let mut info = DevicesInfo {
            entries: vec![kickr(Link::Idle, false)],
            ..Default::default()
        };
        let r = rows(Page::Device(0), &info);
        let labels: Vec<&str> = r.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Kind", "State", "Signal", "Remembered", "Connect"]);
        assert_eq!(
            r[4].confirm,
            Some(Activate::Command(
                "devices.connect hci0/dev_00_00_00_00_00_01".into()
            ))
        );
        // Connected: forget, no connect.
        info.entries[0] = kickr(Link::Connected("Wahoo KICKR".into()), true);
        let r = rows(Page::Device(0), &info);
        let last = r.last().unwrap();
        assert_eq!(last.label, "Disconnect and forget");
        assert_eq!(
            last.confirm,
            Some(Activate::CommandBack(
                "devices.forget hci0/dev_00_00_00_00_00_01".into()
            ))
        );
        assert!(!r.iter().any(|r| r.label == "Connect"));
        // A failure says why and offers to connect again.
        info.entries[0] = kickr(Link::Failed("not found".into()), true);
        let r = rows(Page::Device(0), &info);
        assert_eq!(
            r[1].value.as_deref(),
            Some("failed: not found; trying again")
        );
        assert!(r.iter().any(|r| r.label == "Connect"));
        // Gone: no rows (the menu goes back to the list).
        assert!(rows(Page::Device(3), &info).is_empty());
    }

    #[test]
    fn the_browser_offers_its_chooser() {
        let mut info = DevicesInfo {
            scan: Scan::Chooser(None),
            entries: vec![kickr(Link::Idle, true)],
            ..Default::default()
        };
        let r = rows(Page::Devices, &info);
        assert_eq!(label(&r, 0), "Add a trainer");
        assert_eq!(
            r[0].confirm,
            Some(Activate::Command("devices.connect trainer".into()))
        );
        assert_eq!(
            r[2].confirm,
            Some(Activate::Command("devices.connect hr".into()))
        );
        // A remembered device connects through the chooser of its kind;
        // no signal row.
        let d = rows(Page::Device(0), &info);
        assert!(!d.iter().any(|r| r.label == "Signal"));
        assert_eq!(
            d.iter().find(|r| r.label == "Connect").unwrap().confirm,
            Some(Activate::Command("devices.connect trainer".into()))
        );
        info.scan = Scan::Chooser(Some("no Web Bluetooth".into()));
        let r = rows(Page::Devices, &info);
        assert_eq!(r[0].confirm, Some(Activate::Disabled));
    }

    #[test]
    fn the_bridge_page_shares_on_the_desktop_and_pairs_in_the_browser() {
        // Desktop, not sharing: the list row opens the page, which turns
        // sharing on.
        let mut info = DevicesInfo {
            bridge: Some(BridgeInfo::Host {
                sharing: false,
                code: None,
                address: None,
                browser: None,
                error: None,
            }),
            ..Default::default()
        };
        let r = rows(Page::Devices, &info);
        assert_eq!(label(&r, 1), "Share with a browser");
        assert_eq!(r[1].value.as_deref(), Some("off"));
        assert_eq!(r[1].confirm, Some(Activate::Open(Page::Bridge)));
        let b = rows(Page::Bridge, &info);
        assert_eq!(b.len(), 1);
        assert_eq!(
            b[0].confirm,
            Some(Activate::Command("bridge.share on".into()))
        );
        // Sharing: the code, the address, no browser yet, a new code.
        info.bridge = Some(BridgeInfo::Host {
            sharing: true,
            code: Some("7K2M-Q9XP-4DA".into()),
            address: Some("https://192.168.1.23:47810".into()),
            browser: None,
            error: None,
        });
        let b = rows(Page::Bridge, &info);
        let labels: Vec<&str> = b.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Share with a browser",
                "Pairing code",
                "Address",
                "Browser",
                "New code"
            ]
        );
        assert_eq!(
            b[0].confirm,
            Some(Activate::Command("bridge.share off".into()))
        );
        assert_eq!(b[1].value.as_deref(), Some("7K2M-Q9XP-4DA"));
        assert_eq!(b[3].value.as_deref(), Some("none yet"));
        assert_eq!(
            status(Page::Bridge, &info),
            "Type 7K2M-Q9XP-4DA in the browser's Devices page"
        );
        assert_eq!(
            rows(Page::Devices, &info)[1].value.as_deref(),
            Some("on, 7K2M-Q9XP-4DA")
        );
        if let Some(BridgeInfo::Host { browser, .. }) = &mut info.bridge {
            *browser = Some("Quest Browser".into());
        }
        assert_eq!(status(Page::Bridge, &info), "Paired with Quest Browser");
        // The browser: enter the code; once in use, its state and stop.
        info.bridge = Some(BridgeInfo::Client {
            code: None,
            state: String::new(),
            supported: true,
        });
        info.scan = Scan::Chooser(None);
        let r = rows(Page::Devices, &info);
        assert_eq!(label(&r, 3), "Use a trainer from another device");
        let b = rows(Page::Bridge, &info);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].confirm, Some(Activate::Command("bridge.pair".into())));
        info.bridge = Some(BridgeInfo::Client {
            code: Some("7K2M-Q9XP-4DA".into()),
            state: "paired with Test desktop".into(),
            supported: true,
        });
        let b = rows(Page::Bridge, &info);
        assert_eq!(
            b.last().unwrap().confirm,
            Some(Activate::Command("bridge.stop".into()))
        );
        assert_eq!(status(Page::Bridge, &info), "paired with Test desktop");
        // Without WebTransport: says so.
        info.bridge = Some(BridgeInfo::Client {
            code: None,
            state: String::new(),
            supported: false,
        });
        assert_eq!(rows(Page::Bridge, &info)[0].label, "Not in this browser");
    }

    #[test]
    fn messages_keep_the_newest() {
        let mut info = DevicesInfo::default();
        for i in 0..5 {
            info.push_message(format!("line {i}"));
        }
        assert_eq!(info.messages, ["line 2", "line 3", "line 4"]);
        assert_eq!(status(Page::Devices, &info), "line 4");
        info.messages.clear();
        info.file = Some("~/.config/ocw/devices.toml".into());
        assert_eq!(
            status(Page::Devices, &info),
            "Remembered in ~/.config/ocw/devices.toml"
        );
    }

    /// Through the menu model: Settings → Trainer → Devices → a device →
    /// Disconnect and forget runs the line and goes back to the list.
    #[test]
    fn the_menu_reaches_the_devices_and_forgetting_goes_back() {
        let info = DevicesInfo {
            entries: vec![kickr(Link::Connected("Wahoo KICKR".into()), true)],
            ..Default::default()
        };
        let settings = Settings::default();
        let riders = crate::riders::Riders::default();
        let rows_of = |p: Page| match p {
            Page::Devices | Page::Device(_) => rows(p, &info),
            _ => super::super::pages::rows(p, &settings, &riders),
        };
        let mut m = Menu::default();
        m.apply_with(Action::Open, rows_of);
        let go = |m: &mut Menu, label: &str| {
            let i = m
                .labels()
                .iter()
                .position(|l| l.starts_with(label))
                .unwrap_or_else(|| panic!("{label} in {:?}", m.labels()));
            m.apply_with(Action::Click(i), rows_of)
        };
        go(&mut m, "Settings");
        go(&mut m, "Trainer");
        go(&mut m, "Devices");
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Devices)));
        go(&mut m, "Wahoo KICKR 0001");
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Device(0))));
        assert_eq!(
            go(&mut m, "Disconnect and forget"),
            Effect::Command("devices.forget hci0/dev_00_00_00_00_00_01".into())
        );
        assert_eq!(m.page(), Some(MenuPage::Sub(Page::Devices)));
    }

    fn strap(id: &str, name: &str, used: Option<bool>) -> Entry {
        Entry {
            id: id.into(),
            name: name.into(),
            role: Role::HeartRate,
            rssi: Some(-58),
            link: Link::Connected("heart-rate sensor (ANT+)".into()),
            remembered: true,
            heart_rate: used,
            trainer: None,
        }
    }

    #[test]
    fn ant_sensors_say_so_and_two_straps_offer_the_choice() {
        let info = DevicesInfo {
            entries: vec![
                strap("ant:120:12345:1", "ANT+ heart rate 12345", Some(true)),
                strap("hci0/dev_00_00_00_00_00_02", "TICKR", Some(false)),
            ],
            ..Default::default()
        };
        let r = rows(Page::Devices, &info);
        assert_eq!(
            r[1].value.as_deref(),
            Some("heart rate, ANT+, -58 dBm, connected, HR in use, remembered")
        );
        assert_eq!(
            r[2].value.as_deref(),
            Some("heart rate, -58 dBm, connected, remembered")
        );
        // The other strap's page: Use for heart rate → devices.hr <id>.
        let d = rows(Page::Device(1), &info);
        let use_it = d.iter().find(|r| r.label == "Use for heart rate").unwrap();
        assert_eq!(use_it.value.as_deref(), Some("no"));
        assert_eq!(
            use_it.confirm,
            Some(Activate::Command(
                "devices.hr hci0/dev_00_00_00_00_00_02".into()
            ))
        );
        // One strap only: nothing to choose.
        let one = DevicesInfo {
            entries: vec![strap("ant:120:1:1", "ANT+ heart rate 1", None)],
            ..Default::default()
        };
        assert!(
            !rows(Page::Device(0), &one)
                .iter()
                .any(|r| r.label == "Use for heart rate")
        );
    }

    /// A BLE and an ANT+ FE-C trainer (spec 14 Phase 17, 17E): the list
    /// says which is in use, the other's page offers *Use as trainer*.
    #[test]
    fn two_trainers_offer_the_choice_and_the_fe_c_one_says_ant() {
        let trainer = |id: &str, name: &str, kind: &str, used| Entry {
            id: id.into(),
            name: name.into(),
            role: Role::Trainer,
            rssi: None,
            link: Link::Connected(kind.into()),
            remembered: true,
            heart_rate: None,
            trainer: Some(used),
        };
        let info = DevicesInfo {
            entries: vec![
                trainer(
                    "hci0/dev_00_00_00_00_00_01",
                    "Wahoo KICKR 0001",
                    "smart trainer (FTMS)",
                    false,
                ),
                trainer(
                    "ant:17:5005:5",
                    "ANT+ trainer 5005",
                    "smart trainer (ANT+ FE-C)",
                    true,
                ),
            ],
            ..Default::default()
        };
        let r = rows(Page::Devices, &info);
        assert_eq!(
            r[2].value.as_deref(),
            Some("trainer, ANT+, connected, in use, remembered")
        );
        assert_eq!(
            r[1].value.as_deref(),
            Some("trainer, connected, remembered")
        );
        let d = rows(Page::Device(0), &info);
        let use_it = d.iter().find(|r| r.label == "Use as trainer").unwrap();
        assert_eq!(use_it.value.as_deref(), Some("no"));
        assert_eq!(
            use_it.confirm,
            Some(Activate::Command(
                "devices.trainer hci0/dev_00_00_00_00_00_01".into()
            ))
        );
        assert!(
            !rows(Page::Device(0), &info)
                .iter()
                .any(|r| r.label == "Use for heart rate")
        );
    }

    #[test]
    fn the_ant_page_enters_the_network_key() {
        let mut info = DevicesInfo {
            ant: Some(AntInfo {
                state: "no stick".into(),
                key_set: false,
                typing: None,
                key_file: Some("~/.config/ocw/secrets.toml [ant]".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        // The list: the ANT+ row says what is missing.
        let r = rows(Page::Devices, &info);
        assert_eq!(label(&r, 1), "ANT+ (USB stick)");
        assert_eq!(r[1].value.as_deref(), Some("network key needed"));
        assert_eq!(r[1].confirm, Some(Activate::Open(Page::Ant)));
        let a = rows(Page::Ant, &info);
        let labels: Vec<&str> = a.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(labels, ["Stick", "Enter the network key", "Kept in"]);
        assert_eq!(a[1].confirm, Some(Activate::Command("ant.key.edit".into())));
        // Typing: the masked field, save, paste, cancel; the hint above.
        info.ant.as_mut().unwrap().typing = Some("******|  6 of 16".into());
        let a = rows(Page::Ant, &info);
        assert_eq!(a[1].value.as_deref(), Some("******|  6 of 16"));
        assert_eq!(a[2].confirm, Some(Activate::Command("ant.key.save".into())));
        assert_eq!(status(Page::Ant, &info), ANT_TYPING);
        // Set: change or remove; the list shows the stick.
        let ant = info.ant.as_mut().unwrap();
        ant.typing = None;
        ant.key_set = true;
        ant.state = "ANT USB-m stick, in use".into();
        let a = rows(Page::Ant, &info);
        assert_eq!(a[1].label, "Change the network key");
        assert_eq!(
            a[2].confirm,
            Some(Activate::Command("ant.key.clear".into()))
        );
        assert_eq!(
            rows(Page::Devices, &info)[1].value.as_deref(),
            Some("ANT USB-m stick, in use")
        );
        assert_eq!(status(Page::Ant, &info), "ANT+: ANT USB-m stick, in use");
        // The browser has no ANT+ row.
        assert!(
            !rows(Page::Devices, &DevicesInfo::default())
                .iter()
                .any(|r| r.label.starts_with("ANT+"))
        );
    }

    /// A stick that cannot be opened: the ANT+ page offers the fix of the
    /// system (spec 14 Phase 17 Q7 (d) on Windows, Q8 (b) on Linux).
    #[test]
    fn the_ant_page_offers_the_driver_or_the_access_rule() {
        let mut info = DevicesInfo {
            ant: Some(AntInfo {
                state: "cannot open the stick (permission)".into(),
                key_set: true,
                ..Default::default()
            }),
            ..Default::default()
        };
        // Nothing to fix: no row.
        let labels = |info: &DevicesInfo, p| -> Vec<String> {
            rows(p, info).into_iter().map(|r| r.label).collect()
        };
        assert!(!labels(&info, Page::Ant).iter().any(|l| l == ANT_ACCESS_ROW));
        // Linux: Install access for the ANT stick -> its page -> pkexec.
        info.ant.as_mut().unwrap().fix = Some(AntFix::LinuxAccess);
        let a = rows(Page::Ant, &info);
        assert_eq!(a[1].label, ANT_ACCESS_ROW);
        assert_eq!(a[1].confirm, Some(Activate::Open(Page::AntAccess)));
        assert_eq!(Page::AntAccess.parent(), Some(Page::Ant));
        let page = rows(Page::AntAccess, &info);
        assert_eq!(
            page[0].confirm,
            Some(Activate::Command(ANT_ACCESS_COMMAND.into()))
        );
        assert!(
            page.iter()
                .any(|r| { r.value.as_deref() == Some("/etc/udev/rules.d/60-ocw-ant.rules") })
        );
        assert!(Page::AntAccess.notes()[0].contains("pkexec"));
        // What the last try said.
        info.ant.as_mut().unwrap().access = Some("pkexec is missing".into());
        let page = rows(Page::AntAccess, &info);
        assert_eq!(
            page.last().unwrap().value.as_deref(),
            Some("pkexec is missing")
        );
        // Windows: Driver for the ANT stick -> Windows Update, Zadig, one
        // program at a time.
        let ant = info.ant.as_mut().unwrap();
        ant.fix = Some(AntFix::WindowsDriver);
        ant.state = "the stick needs a driver".into();
        let a = rows(Page::Ant, &info);
        assert_eq!(a[1].label, ANT_DRIVER_ROW);
        assert_eq!(a[1].confirm, Some(Activate::Open(Page::AntDriver)));
        assert_eq!(
            labels(&info, Page::AntDriver),
            ["1. Windows Update", "2. Or Zadig", "One program at a time"]
        );
        let note = Page::AntDriver.notes()[0];
        assert!(note.contains("Optional updates") && note.contains("Zadig"));
        // Short enough for the 16:9 page (the desktop's test measures it).
        assert!(note.len() <= ANT_NOTE.len() && ANT_ACCESS_NOTE.len() <= ANT_NOTE.len());
        for p in [Page::AntDriver, Page::AntAccess] {
            assert!(p.notes().iter().all(|t| t.is_ascii()), "{p:?}");
            assert!(rows(p, &info).len() < 7);
            assert!(rows(p, &info).iter().all(|r| r.label.len() <= 48));
        }
    }

    /// The browser (spec 14 Phase 17 A5): *Add an ANT+ stick* where
    /// WebUSB is, the reason where it is not, the WinUSB note on Windows,
    /// the search and the found sensors connected by id.
    #[test]
    fn the_browser_adds_an_ant_stick_over_web_usb() {
        let web = |supported, windows, granted| WebUsbInfo {
            supported,
            windows,
            granted,
            scanning: None,
        };
        let mut info = DevicesInfo {
            scan: Scan::Chooser(None),
            ant: Some(AntInfo {
                state: "no stick".into(),
                key_file: Some("this browser only (never sent)".into()),
                web: Some(web(true, false, false)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let labels = |info: &DevicesInfo, p| -> Vec<String> {
            rows(p, info).into_iter().map(|r| r.label).collect()
        };
        // The list: the three Bluetooth rows, then the stick's; no search
        // before a key and a stick.
        let r = rows(Page::Devices, &info);
        assert_eq!(r[3].label, ANT_ADD_ROW);
        assert_eq!(
            r[3].confirm,
            Some(Activate::Command(ANT_ADD_COMMAND.into()))
        );
        assert_eq!(r[4].label, "ANT+ (USB stick)");
        assert!(
            !labels(&info, Page::Devices)
                .iter()
                .any(|l| l == ANT_SCAN_ROW)
        );
        // Key and stick: Find ANT+ sensors, on the list and the page.
        let a = info.ant.as_mut().unwrap();
        a.key_set = true;
        a.state = "ANT USB-m stick, ready".into();
        a.web = Some(web(true, false, true));
        let r = rows(Page::Devices, &info);
        assert_eq!(r[3].value.as_deref(), Some("added"));
        assert_eq!(r[5].label, ANT_SCAN_ROW);
        assert_eq!(r[5].confirm, Some(Activate::Command("ant.scan".into())));
        assert_eq!(
            labels(&info, Page::Ant),
            [
                "Stick",
                ANT_ADD_ROW,
                "Change the network key",
                "Remove the key",
                ANT_SCAN_ROW,
                "Kept in"
            ]
        );
        info.ant.as_mut().unwrap().web.as_mut().unwrap().scanning = Some(7);
        let r = rows(Page::Ant, &info);
        assert_eq!(r[4].label, "Stop the search");
        assert_eq!(r[4].value.as_deref(), Some("7 s left"));
        // Windows: the WinUSB note (Q7); a Linux permission: the rule.
        let a = info.ant.as_mut().unwrap();
        a.web = Some(web(true, true, true));
        a.fix = Some(AntFix::LinuxAccess);
        let r = rows(Page::Ant, &info);
        assert!(
            r.iter()
                .any(|r| r.label == "On Windows"
                    && r.value.as_deref() == Some("needs WinUSB (Zadig)"))
        );
        assert!(r.iter().any(|r| r.label == "On Linux"));
        assert!(
            !r.iter().any(|r| r.label == ANT_ACCESS_ROW),
            "pkexec: the desktop only"
        );
        // A found ANT+ sensor connects through the stick, by id.
        info.entries.push(Entry {
            id: "ant:120:1234:1".into(),
            name: "ANT+ heart rate 1234".into(),
            role: Role::HeartRate,
            rssi: Some(-60),
            link: Link::Idle,
            remembered: false,
            heart_rate: None,
            trainer: None,
        });
        let d = rows(Page::Device(0), &info);
        assert!(d.iter().any(|r| r.label == "Connect"
            && r.confirm == Some(Activate::Command("devices.connect ant:120:1234:1".into()))));
        // Without WebUSB: the row says so and opens the page that says why.
        info.ant.as_mut().unwrap().web = Some(web(false, false, false));
        let r = rows(Page::Devices, &info);
        assert_eq!(r[3].label, ANT_ADD_ROW);
        assert_eq!(r[3].value.as_deref(), Some("no WebUSB here"));
        assert_eq!(r[3].confirm, Some(Activate::Open(Page::Ant)));
        assert!(
            !labels(&info, Page::Devices)
                .iter()
                .any(|l| l == "ANT+ (USB stick)")
        );
        assert_eq!(
            labels(&info, Page::Ant),
            [
                "WebUSB",
                "Firefox, Safari, iOS",
                "Use Chrome or Edge",
                "Or the desktop app"
            ]
        );
        for p in [Page::Devices, Page::Ant] {
            assert!(rows(p, &info).iter().all(|r| r.label.len() <= 48), "{p:?}");
            assert!(rows(p, &info).len() <= 8, "{p:?}");
        }
        assert!(ANT_NOTE.is_ascii() && !ANT_NOTE.contains("secrets.toml"));
    }
}

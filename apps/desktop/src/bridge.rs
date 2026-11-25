//! The local sensor bridge on the desktop (spec 14 Phase 10, 10D.2 *Built
//! (local bridge)*): Settings → Trainer → Devices → **Share with a
//! browser** shares the trainer (and the heart-rate strap) with one browser
//! on the same network – the Quest browser, an iPad, any browser – paired
//! with a short code; no account.
//!
//! - The endpoint is `ocw_bridge::host` (WebTransport on a thread of its
//!   own); this is the Bevy glue: start / stop, the code kept in
//!   `secrets.toml [bridge]` (the same code next time, until *New code*),
//!   the browser's trainer control to the devices, the Devices page's rows
//!   and a QR code of the page link while the page is open.
//! - `devices.rs` passes every device event on ([`Bridge::relay`]) and
//!   leaves the trainer's load to the paired browser
//!   ([`Bridge::browser_controls`]).
//!
//! - While a browser is paired the window shows the **Sharing** screen
//!   instead of the ride ([`sharing_screen`], drawn by
//!   `ocw_render::sharing`): the browser, the live numbers, the grade it
//!   sets, the bridge, the headset's last report, the code and *Stop
//!   sharing*; the own ride pauses meanwhile (no distance, no recording,
//!   no hearts: `ClientCore::set_paused`) and goes on when the browser
//!   leaves or sharing stops.
//!
//! Console: `bridge.share [on|off|new-code]`, `bridge.status`; the command
//! line's `--share-bridge` shares from the start.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::text::FontSize;
use clap::{Arg, Command};
use ocw_bridge::host::{BridgeHost, HostEvent, HostOptions, new_secret};
use ocw_client::menu::devices::BridgeInfo;
use ocw_client::menu::sharing::{Report, SetLoad, SharingInfo};
use ocw_devices::DeviceEvent;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::InputContext;
use ocw_render::sharing::{SharingPointer, SharingView};
use std::time::Instant;

use crate::Args;
use crate::config::Slot;
use crate::devices::Devices;

/// The section of `secrets.toml` with the pairing secret.
pub const SECTION: &str = "bridge";

/// The desktop's name for the browser (no host name: it reaches the page's
/// screen and the art project's pictures).
const HOST_NAME: &str = "Open Cycling World (desktop)";

#[derive(Resource, Default)]
pub struct Bridge {
    host: Option<BridgeHost>,
    secret: Option<u32>,
    slot: Option<Slot>,
    browser: Option<String>,
    error: Option<String>,
    /// The last power reading relayed: watts, cadence, when.
    power: Option<(f64, Option<f64>, Instant)>,
    /// The last heart rate relayed.
    heart: Option<(f64, Instant)>,
    /// What the paired browser last set on the trainer.
    load: Option<(SetLoad, Instant)>,
    /// The paired browser's last report (the headset's beacon).
    report: Option<(Report, Instant)>,
}

/// The secret in its section's text (`secret = 123`).
pub fn parse_secret(text: &str) -> Option<u32> {
    text.lines().find_map(|l| {
        let (k, v) = l.split_once('=')?;
        (k.trim() == "secret").then(|| v.trim().parse::<u32>().ok())?
    })
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

impl Bridge {
    fn load(slot: Option<Slot>) -> Self {
        let secret = slot
            .as_ref()
            .and_then(|s| s.read().ok().flatten())
            .and_then(|t| parse_secret(&t));
        Self {
            secret,
            slot,
            ..Default::default()
        }
    }

    /// The secret: the saved one, else a new one (saved).
    fn secret(&mut self, fresh: bool) -> u32 {
        if let (Some(s), false) = (self.secret, fresh) {
            return s;
        }
        let s = new_secret();
        self.secret = Some(s);
        if let Some(slot) = &self.slot
            && let Err(e) = slot.write(&format!(
                "# The sensor bridge's pairing secret (spec 14 Phase 10, 10D.2).\nsecret = {s}\n"
            ))
        {
            warn!("cannot save the bridge's code: {e}");
        }
        s
    }

    /// Starts sharing (`fresh`: with a new code).
    fn start(&mut self, fresh: bool) -> Result<String, String> {
        if let Some(h) = self.host.take() {
            h.stop();
        }
        self.browser = None;
        let secret = self.secret(fresh);
        match BridgeHost::start(HostOptions::new(secret, HOST_NAME, unix_now())) {
            Ok(h) => {
                self.error = None;
                let line = format!(
                    "sharing the trainer with a browser: code {} (address https://{}:{}); on the other device open {} → Settings → Trainer → Devices → Use a trainer from another device",
                    h.code,
                    h.code.ip,
                    h.code.port(),
                    ocw_bridge::PREVIEW_URL
                );
                if h.code.ip.is_loopback() {
                    warn!("no local network found: the bridge is only reachable on this computer");
                }
                self.host = Some(h);
                Ok(line)
            }
            Err(e) => {
                self.error = Some(e.clone());
                Err(format!("cannot share the trainer: {e}"))
            }
        }
    }

    fn stop(&mut self) -> String {
        self.browser = None;
        match self.host.take() {
            Some(h) => {
                h.stop();
                "stopped sharing the trainer".into()
            }
            None => "the trainer is not shared".into(),
        }
    }

    /// A device event for the paired browser (and the Sharing screen's
    /// live numbers).
    pub fn relay(&mut self, device: &str, event: &DeviceEvent, virt: bool) {
        let Some(h) = &self.host else { return };
        h.relay(device, event, virt);
        match event {
            DeviceEvent::Power {
                watts, cadence_rpm, ..
            } => self.power = Some((*watts, *cadence_rpm, Instant::now())),
            DeviceEvent::HeartRate { bpm, .. } => self.heart = Some((*bpm, Instant::now())),
            _ => {}
        }
    }

    /// The paired browser, while the trainer is shared with it.
    pub fn sharing_with(&self) -> Option<&str> {
        self.host.as_ref().and(self.browser.as_deref())
    }

    /// What the Sharing screen shows (`None`: no browser paired).
    pub fn sharing_info(&self, pad_confirm_east: bool) -> Option<SharingInfo> {
        let browser = self.sharing_with()?.to_owned();
        let host = self.host.as_ref()?;
        let link = host.link();
        let age = |t: Instant| t.elapsed().as_secs_f64();
        Some(SharingInfo {
            browser,
            power_w: self.power.map(|p| p.0),
            cadence_rpm: self.power.and_then(|p| p.1),
            heart_rate_bpm: self.heart.map(|h| h.0),
            power_age_s: self.power.map(|p| age(p.2)),
            heart_age_s: self.heart.map(|h| age(h.1)),
            load: self.load.map(|(l, t)| (l, age(t))),
            rtt_ms: link.as_ref().map(|l| l.rtt_ms),
            heard_s_ago: link.as_ref().map_or(0.0, |l| l.heard_ms_ago / 1000.0),
            paired_s: link.as_ref().map_or(0.0, |l| l.paired_ms_ago / 1000.0),
            relayed: host.stats().relayed,
            report: self.report.as_ref().map(|(r, t)| (r.clone(), age(*t))),
            code: host.code.to_string(),
            pad_confirm_east,
        })
    }

    /// A paired browser sets the trainer's load.
    pub fn browser_controls(&self) -> bool {
        self.host.is_some() && self.browser.is_some()
    }

    /// The pairing link (the QR code).
    pub fn link(&self) -> Option<String> {
        self.host
            .as_ref()
            .map(|h| ocw_bridge::page_link(ocw_bridge::PREVIEW_URL, &h.code))
    }

    pub fn info(&self) -> BridgeInfo {
        BridgeInfo::Host {
            sharing: self.host.is_some(),
            code: self.host.as_ref().map(|h| h.code.to_string()),
            address: self
                .host
                .as_ref()
                .map(|h| format!("https://{}:{}", h.code.ip, h.code.port())),
            browser: self.browser.clone(),
            error: self.error.clone(),
        }
    }

    fn status(&self) -> String {
        match &self.host {
            None => match &self.error {
                Some(e) => format!("not sharing: {e}"),
                None => "not sharing (bridge.share on)".into(),
            },
            Some(h) => {
                let s = h.stats();
                format!(
                    "sharing: code {}, address https://{}:{}, browser: {}; {} readings relayed, {} controls, {} pings, {} refused; link {}",
                    h.code,
                    h.code.ip,
                    h.code.port(),
                    self.browser.as_deref().unwrap_or("none"),
                    s.relayed,
                    s.controls,
                    s.pings,
                    s.refused,
                    self.link().unwrap_or_default()
                )
            }
        }
    }
}

pub struct BridgePlugin;

impl Plugin for BridgePlugin {
    fn build(&self, app: &mut App) {
        let slot = app
            .world()
            .get_resource::<crate::config::ConfigFiles>()
            .map(|f| f.secret(SECTION));
        let mut bridge = Bridge::load(slot);
        let share = app
            .world()
            .get_resource::<Args>()
            .is_some_and(|a| a.share_bridge);
        if share {
            match bridge.start(false) {
                Ok(line) => info!("{line}"),
                Err(e) => warn!("{e}"),
            }
        }
        app.insert_resource(bridge)
            .init_resource::<QrOverlay>()
            .add_plugins(ocw_render::sharing::SharingPlugin)
            .init_resource::<SharingPause>()
            .add_systems(Update, ((poll_bridge, sharing_screen).chain(), show_qr))
            .add_systems(Last, stop_on_exit);
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("bridge.share")
                .about("Share the trainer with a browser on the same network (the sensor bridge): on, off, or new-code")
                .arg(
                    Arg::new("what")
                        .default_value("on")
                        .value_parser(["on", "off", "new-code"]),
                ),
            |world, m| {
                let what = m.get_one::<String>("what").expect("has default").clone();
                let mut b = world.resource_mut::<Bridge>();
                match what.as_str() {
                    "off" => Ok(b.stop()),
                    "new-code" => b.start(true),
                    _ => b.start(false),
                }
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("bridge.status").about("The sensor bridge: code, address, browser, counters"),
            |world, _| Ok(world.resource::<Bridge>().status()),
        ));
    }
}

/// The bridge's events: who paired or left, the browser's trainer control.
fn poll_bridge(
    mut bridge: ResMut<Bridge>,
    devices: Res<Devices>,
    mut console: ResMut<ConsoleState>,
) {
    let Some(host) = &bridge.host else { return };
    let mut lines = Vec::new();
    let mut browser = bridge.browser.clone();
    let (mut load, mut report) = (bridge.load, bridge.report.clone());
    for e in host.events() {
        match e {
            HostEvent::Paired { client } => {
                lines.push(format!(
                    "bridge: paired with {client}; it sets the trainer's load now"
                ));
                if browser.as_ref() != Some(&client) {
                    (load, report) = (None, None);
                }
                browser = Some(client);
            }
            HostEvent::Unpaired { reason } => {
                lines.push(format!("bridge: the browser left ({reason})"));
                browser = None;
            }
            HostEvent::Refused { why } => lines.push(format!("bridge: refused {why}")),
            HostEvent::Control { id, command } => {
                if let Some(l) = SetLoad::of(&command) {
                    load = Some((l, Instant::now()));
                }
                devices.command_all(command);
                host.applied(id);
            }
            HostEvent::Report { kind, data } => {
                report = Some((Report::of(&kind, &data), Instant::now()));
                lines.push(keep_report(&kind, &data));
            }
        }
    }
    bridge.load = load;
    bridge.report = report;
    if browser != bridge.browser {
        bridge.browser = browser;
    }
    for l in lines {
        info!("{l}");
        console.print(l);
    }
}

/// What the Sharing screen paused, to resume.
#[derive(Resource, Default)]
pub struct SharingPause {
    /// The own ride is paused: where (m) and since when.
    since: Option<(f64, Instant)>,
    /// A workout the screen paused.
    workout: bool,
}

/// The Sharing screen's input: Enter / the pad's Confirm while it has the
/// input (not under the menu or the console).
type SharingInput<'w, 's> = (
    Option<Res<'w, InputContext>>,
    Option<Res<'w, ButtonInput<KeyCode>>>,
    Query<'w, 's, &'static Gamepad>,
    Option<Res<'w, ocw_ui::settings::UserSettings>>,
);

/// The Sharing screen (spec 14 Phase 10, 10D.2): up while a browser is
/// paired; the own ride (and a running workout) paused meanwhile, resumed
/// when the browser leaves or *Stop sharing* is chosen (Enter, the pad's
/// Confirm, a click).
#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub fn sharing_screen(
    mut bridge: ResMut<Bridge>,
    mut client: Option<ResMut<crate::connection::ClientState>>,
    link: Option<Res<crate::connection::Link>>,
    mut view: ResMut<SharingView>,
    mut pointer: ResMut<SharingPointer>,
    mut pause: ResMut<SharingPause>,
    (context, keys, pads, settings): SharingInput,
    mut workouts: Option<ResMut<crate::workout::Workouts>>,
    mut console: ResMut<ConsoleState>,
) {
    let east = settings.is_some_and(|s| {
        s.settings
            .get("controls.pad_confirm")
            .is_ok_and(|v| v.to_string() == "east")
    });
    let clicked = std::mem::take(&mut pointer.0);
    let confirm = view.open
        && context.is_some_and(|c| *c == InputContext::Sharing)
        && ocw_ui::controls::pressed(keys.as_deref(), pads.iter(), east).confirm;
    if view.open && (clicked || confirm) {
        let line = bridge.stop();
        console.print(line);
    }
    let sharing = bridge.sharing_with().map(str::to_owned);
    let mut lines = Vec::new();
    if let Some(c) = client.as_mut() {
        let c = &mut c.0;
        if sharing.is_some() != c.paused() {
            if let Some(sample) = c.set_paused(sharing.is_some())
                && let Some(l) = &link
            {
                l.send_bytes(sample);
            }
            let at = c.state().distance;
            if let Some(who) = &sharing {
                pause.since = Some((at, Instant::now()));
                lines.push(format!(
                    "sharing: the own ride pauses at {at:.0} m while {who} rides with the trainer (no distance, no recording, no hearts here)"
                ));
            } else if let Some((from, t)) = pause.since.take() {
                lines.push(format!(
                    "sharing: the own ride goes on at {at:.0} m after {:.0} s paused ({:+.1} m while paused)",
                    t.elapsed().as_secs_f64(),
                    at - from
                ));
            }
        }
    }
    if let Some(w) = workouts.as_mut() {
        if sharing.is_some() && !pause.workout && w.0.now().is_some() && w.0.pause().is_ok() {
            pause.workout = true;
            lines.push("sharing: the workout pauses too".into());
        } else if sharing.is_none() && std::mem::take(&mut pause.workout) {
            let _ = w.0.resume();
            lines.push("sharing: the workout goes on".into());
        }
    }
    for l in lines {
        console.print(l);
    }
    match bridge.sharing_info(east) {
        Some(info) => {
            view.set_if_neq(ocw_client::menu::sharing::view(&info));
        }
        None if view.open => *view = SharingView::default(),
        None => {}
    }
}

fn stop_on_exit(mut exit: MessageReader<AppExit>, mut bridge: ResMut<Bridge>) {
    if exit.read().next().is_some() && bridge.host.is_some() {
        bridge.stop();
    }
}

/// The QR code of the page link, beside the menu while the bridge page is
/// open and the trainer is shared.
#[derive(Resource, Default)]
struct QrOverlay {
    entity: Option<Entity>,
    link: Option<String>,
}

/// The QR code's modules as an RGBA image (black on white, a 4-module quiet
/// zone); its width in modules.
pub fn qr_image(text: &str) -> Option<(Vec<u8>, u32)> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let n = code.width();
    let quiet = 4;
    let w = n + 2 * quiet;
    let colors = code.to_colors();
    let mut rgba = vec![255u8; w * w * 4];
    for y in 0..n {
        for x in 0..n {
            if colors[y * n + x] == qrcode::Color::Dark {
                let i = ((y + quiet) * w + x + quiet) * 4;
                rgba[i..i + 3].fill(0);
            }
        }
    }
    Some((rgba, w as u32))
}

fn show_qr(
    mut commands: Commands,
    mut overlay: ResMut<QrOverlay>,
    bridge: Res<Bridge>,
    menu: Option<Res<ocw_ui::menu::MenuState>>,
    mut images: ResMut<Assets<Image>>,
) {
    use ocw_client::menu::model::Page as MenuPage;
    use ocw_client::menu::pages::Page;
    let open = menu.is_some_and(|m| m.menu.page() == Some(MenuPage::Sub(Page::Bridge)));
    let want = if open { bridge.link() } else { None };
    if want == overlay.link {
        return;
    }
    if let Some(e) = overlay.entity.take() {
        commands.entity(e).despawn();
    }
    overlay.link = want.clone();
    let Some(link) = want else { return };
    let Some((rgba, w)) = qr_image(&link) else {
        return;
    };
    let mut image = Image::new(
        Extent3d {
            width: w,
            height: w,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        rgba,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    let handle = images.add(image);
    let e = commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(32.0),
                top: Val::Percent(18.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(6.0),
                ..default()
            },
            GlobalZIndex(1000),
            Name::new("Bridge QR code"),
        ))
        .with_children(|p| {
            p.spawn((
                ImageNode::new(handle),
                Node {
                    width: Val::Px(240.0),
                    height: Val::Px(240.0),
                    ..default()
                },
            ));
            p.spawn((
                Text::new("Scan with the phone or tablet"),
                TextFont {
                    font_size: FontSize::Px(16.0),
                    ..default()
                },
                TextColor(Color::WHITE),
            ));
        })
        .id();
    overlay.entity = Some(e);
}

/// The browser's reports (spec 14 Phase 14, 14I W4: the headset run's
/// frame times and device facts): `<data dir>/ocw/reports/<kind>.jsonl`.
pub fn reports_dir() -> Option<std::path::PathBuf> {
    crate::ride::data_dir().map(|d| d.join("reports"))
}

/// One kept report: a JSON line with when it came (Unix ms) and what.
pub fn report_line(kind: &str, data: &serde_json::Value, unix_ms: u64) -> String {
    serde_json::json!({"received_unix_ms": unix_ms, "kind": kind, "data": data}).to_string()
}

/// A report's file name: the kind's letters only.
pub fn report_file(kind: &str) -> String {
    let k: String = kind
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(32)
        .collect();
    format!("{}.jsonl", if k.is_empty() { "report" } else { &k })
}

/// The headset's left-eye pictures (spec 14 Phase 14, 14I W3): the report
/// kind, and the folder below the reports where the JPEGs go.
pub const SHOT_KIND: &str = "xr-shot";
pub const SHOTS_DIR: &str = "xr-shots";

/// A picture report's JPEG and the line kept for it (the data without the
/// picture, with its file name): `None` when it holds no JPEG.
pub fn split_shot(
    data: &serde_json::Value,
    unix_ms: u64,
) -> Option<(Vec<u8>, String, serde_json::Value)> {
    use base64::Engine as _;
    let b64 = data.get("jpeg_base64")?.as_str()?;
    let jpeg = base64::engine::general_purpose::STANDARD.decode(b64).ok()?;
    // A JPEG starts with its SOI marker.
    if jpeg.len() < 4 || jpeg[..2] != [0xFF, 0xD8] {
        return None;
    }
    let name = format!("xr-shot-{unix_ms}.jpg");
    let mut line = data.clone();
    if let Some(o) = line.as_object_mut() {
        o.remove("jpeg_base64");
        o.insert("file".into(), format!("{SHOTS_DIR}/{name}").into());
        o.insert("bytes".into(), jpeg.len().into());
    }
    Some((jpeg, name, line))
}

/// Appends a report to its file; the console line says where (and the
/// headset run's main numbers). A picture (`xr-shot`) goes into its own
/// JPEG file, its line without the picture.
fn keep_report(kind: &str, data: &serde_json::Value) -> String {
    use std::io::Write as _;
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let Some(dir) = reports_dir() else {
        return format!("bridge: a {kind} report (no data directory to keep it)");
    };
    let shot = (kind == SHOT_KIND).then(|| split_shot(data, ms)).flatten();
    let mut picture = None;
    let data = match shot {
        Some((jpeg, name, line)) => {
            let path = dir.join(SHOTS_DIR).join(&name);
            let wrote = std::fs::create_dir_all(dir.join(SHOTS_DIR))
                .and_then(|()| std::fs::write(&path, &jpeg));
            picture = Some(match wrote {
                Ok(()) => format!(", the picture {}", path.display()),
                Err(e) => format!(", the picture not kept: {e}"),
            });
            line
        }
        None => data.clone(),
    };
    let data = &data;
    let path = dir.join(report_file(kind));
    let wrote = std::fs::create_dir_all(&dir).and_then(|()| {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        writeln!(f, "{}", report_line(kind, data, ms))
    });
    let num = |k: &str| data.pointer(k).and_then(serde_json::Value::as_f64);
    let summary = match (num("/xr/fps"), num("/xr/work_ms_p50")) {
        (Some(fps), Some(w)) => format!(" – {fps:.1} fps, work p50 {w:.1} ms"),
        _ => String::new(),
    };
    let picture = picture.unwrap_or_default();
    match wrote {
        Ok(()) => format!(
            "bridge: {kind} report{summary} → {}{picture}",
            path.display()
        ),
        Err(e) => format!("bridge: {kind} report{summary} not kept: {e}{picture}"),
    }
}

#[cfg(test)]
mod tests {
    /// 14I W4: a report is one JSON line with its time, kind and data; the
    /// file name keeps only the kind's letters.
    #[test]
    fn reports_are_kept_as_json_lines() {
        let data = serde_json::json!({"xr": {"fps": 71.9}});
        let line = super::report_line("xr", &data, 1_790_000_000_000);
        let back: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(back["kind"], "xr");
        assert_eq!(back["data"], data);
        assert_eq!(back["received_unix_ms"], 1_790_000_000_000u64);
        assert!(!line.contains('\n'));
        assert_eq!(super::report_file("xr"), "xr.jsonl");
        assert_eq!(super::report_file("../../etc"), "etc.jsonl");
        assert_eq!(super::report_file("/"), "report.jsonl");
    }

    use super::*;

    /// 14I W3: a left-eye picture over the bridge becomes a JPEG file; its
    /// line keeps the rest and names the file; a report without a JPEG is
    /// none.
    #[test]
    fn a_headset_picture_is_kept_as_a_jpeg() {
        use base64::Engine as _;
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 16, b'J', b'F', b'I', b'F', 0xFF, 0xD9,
        ];
        let b64 = base64::engine::general_purpose::STANDARD.encode(jpeg);
        let data = serde_json::json!({"jpeg_base64": b64, "w": 320, "h": 354});
        let (bytes, name, line) = split_shot(&data, 1_790_000_000_000).unwrap();
        assert_eq!(bytes, jpeg);
        assert_eq!(name, "xr-shot-1790000000000.jpg");
        assert_eq!(line["file"], "xr-shots/xr-shot-1790000000000.jpg");
        assert_eq!(line["w"], 320);
        assert_eq!(line["bytes"], 12);
        assert!(line.get("jpeg_base64").is_none());
        assert!(split_shot(&serde_json::json!({"w": 1}), 0).is_none());
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG....");
        assert!(split_shot(&serde_json::json!({"jpeg_base64": png}), 0).is_none());
        assert!(split_shot(&serde_json::json!({"jpeg_base64": "not base64!"}), 0).is_none());
        assert_eq!(report_file(SHOT_KIND), "xr-shot.jsonl");
    }

    /// The Sharing screen (10D.2): up as soon as a browser is paired, with
    /// its numbers; the own ride paused meanwhile; *Stop sharing* (a click
    /// or Enter) ends it and the ride goes on; so does the browser leaving.
    #[test]
    fn the_sharing_screen_follows_the_paired_browser() {
        use crate::client::ClientCore;
        use crate::connection::ClientState;
        use ocw_core::RiderProfile;
        let mut b = Bridge::default();
        b.start(false).expect("starts");
        let code = b.host.as_ref().unwrap().code.to_string();
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(b)
            .init_resource::<SharingView>()
            .init_resource::<SharingPointer>()
            .init_resource::<SharingPause>()
            .init_resource::<ConsoleState>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(InputContext::Sharing)
            .insert_resource(ClientState(ClientCore::new(
                ocw_world::Route::test_loop(),
                "Ada Rider",
                RiderProfile::default(),
            )))
            .add_systems(Update, sharing_screen);
        let open = |app: &App| app.world().resource::<SharingView>().open;
        let paused = |app: &App| app.world().resource::<ClientState>().0.paused();
        app.update();
        assert!(!open(&app), "sharing, but no browser yet: the ride");
        assert!(!paused(&app));
        // A browser pairs; the trainer's reading reaches the screen.
        let pair = |app: &mut App| {
            let mut b = app.world_mut().resource_mut::<Bridge>();
            b.browser = Some("Quest Browser".into());
            b.relay(
                "Virtual trainer",
                &DeviceEvent::Power {
                    watts: 182.0,
                    cadence_rpm: Some(88.0),
                    timestamp: std::time::Duration::ZERO,
                },
                true,
            );
        };
        pair(&mut app);
        app.update();
        assert!(open(&app));
        assert!(paused(&app), "the own ride pauses");
        let v = app.world().resource::<SharingView>().clone();
        assert_eq!(v.subtitle, "Your trainer rides in Quest Browser");
        assert_eq!(v.tiles[0].value, "182");
        assert_eq!(v.tiles[1].value, "88");
        assert!(v.rows.contains(&("Pairing code".into(), code)));
        // Stop sharing, clicked.
        app.world_mut().resource_mut::<SharingPointer>().0 = true;
        app.update();
        assert!(!open(&app));
        assert!(!paused(&app), "the ride goes on");
        assert!(app.world().resource::<Bridge>().host.is_none(), "stopped");
        assert!(!app.world().resource::<SharingPointer>().0);
        // Again by Enter.
        app.world_mut()
            .resource_mut::<Bridge>()
            .start(false)
            .unwrap();
        pair(&mut app);
        app.update();
        assert!(open(&app) && paused(&app));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Enter);
        app.update();
        assert!(!open(&app) && !paused(&app));
        // The browser leaves: back to the ride, still sharing.
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .clear();
        app.world_mut()
            .resource_mut::<Bridge>()
            .start(false)
            .unwrap();
        pair(&mut app);
        app.update();
        assert!(open(&app));
        app.world_mut().resource_mut::<Bridge>().browser = None;
        app.update();
        assert!(!open(&app) && !paused(&app));
        assert!(app.world().resource::<Bridge>().host.is_some());
        let lines = app.world().resource::<ConsoleState>().log().join("\n");
        assert!(lines.contains("the own ride pauses"), "{lines}");
        assert!(lines.contains("the own ride goes on"), "{lines}");
        app.world_mut().resource_mut::<Bridge>().stop();
    }

    #[test]
    fn the_secret_is_read_from_its_section() {
        assert_eq!(parse_secret("# x\nsecret = 123456\n"), Some(123456));
        assert_eq!(parse_secret("secret=7"), Some(7));
        assert_eq!(parse_secret("other = 1"), None);
        assert_eq!(parse_secret("secret = x"), None);
    }

    #[test]
    fn the_qr_code_has_a_quiet_zone() {
        let link = "https://newkit.github.io/OCW/?bridge=7K2MQ9XP4DA";
        let (rgba, w) = qr_image(link).unwrap();
        assert_eq!(rgba.len(), (w * w * 4) as usize);
        // Version 3 (29 modules) or so, plus 2 × 4 quiet modules.
        assert!((29..=45).contains(&w), "{w}");
        // The border is white, the finder pattern's corner black.
        assert!(rgba[..(w * 4 * 4) as usize].iter().all(|&b| b == 255));
        let i = ((4 * w + 4) * 4) as usize;
        assert_eq!(rgba[i..i + 3], [0, 0, 0]);
    }

    /// The whole flow in the app: share, a browser (the native client of
    /// the bridge's own tests is not here; the endpoint's loopback test in
    /// `ocw-bridge` covers the wire) – here: start, info for the menu, the
    /// same code after a restart, a new code, stop.
    #[test]
    fn sharing_starts_keeps_its_code_and_stops() {
        let dir = std::env::temp_dir().join(format!("ocw-bridge-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let files = crate::config::ConfigFiles::in_dir(&dir);
        let mut b = Bridge::load(Some(files.secret(SECTION)));
        assert!(matches!(b.info(), BridgeInfo::Host { sharing: false, .. }));
        b.start(false).expect("starts");
        let BridgeInfo::Host {
            sharing: true,
            code: Some(code),
            address: Some(addr),
            browser: None,
            ..
        } = b.info()
        else {
            panic!("{:?}", b.info())
        };
        assert!(addr.starts_with("https://"));
        assert!(b.link().unwrap().ends_with(&code.replace('-', "")));
        assert!(!b.browser_controls());
        b.stop();
        // The same code after a restart of the app (the secret is saved).
        let mut again = Bridge::load(Some(files.secret(SECTION)));
        again.start(false).unwrap();
        let BridgeInfo::Host {
            code: Some(code2), ..
        } = again.info()
        else {
            panic!()
        };
        assert_eq!(code2, code);
        again.start(true).unwrap();
        let BridgeInfo::Host {
            code: Some(code3), ..
        } = again.info()
        else {
            panic!()
        };
        assert_ne!(code3, code);
        assert!(again.status().contains(&code3));
        assert_eq!(again.stop(), "stopped sharing the trainer");
        assert_eq!(again.stop(), "the trainer is not shared");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

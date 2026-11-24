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
//! Console: `bridge.share [on|off|new-code]`, `bridge.status`; the command
//! line's `--share-bridge` shares from the start.

use bevy::asset::RenderAssetUsages;
use bevy::image::ImageSampler;
use bevy::prelude::*;
use bevy::text::FontSize;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use clap::{Arg, Command};
use ocw_bridge::host::{BridgeHost, HostEvent, HostOptions, new_secret};
use ocw_client::menu::devices::BridgeInfo;
use ocw_devices::DeviceEvent;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

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

    /// A device event for the paired browser.
    pub fn relay(&self, device: &str, event: &DeviceEvent, virt: bool) {
        if let Some(h) = &self.host {
            h.relay(device, event, virt);
        }
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
            .add_systems(Update, (poll_bridge, show_qr))
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
    for e in host.events() {
        match e {
            HostEvent::Paired { client } => {
                lines.push(format!("bridge: paired with {client}; it sets the trainer's load now"));
                browser = Some(client);
            }
            HostEvent::Unpaired { reason } => {
                lines.push(format!("bridge: the browser left ({reason})"));
                browser = None;
            }
            HostEvent::Refused { why } => lines.push(format!("bridge: refused {why}")),
            HostEvent::Control { id, command } => {
                devices.command_all(command);
                host.applied(id);
            }
        }
    }
    if browser != bridge.browser {
        bridge.browser = browser;
    }
    for l in lines {
        info!("{l}");
        console.print(l);
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
    let Some((rgba, w)) = qr_image(&link) else { return };
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

#[cfg(test)]
mod tests {
    use super::*;

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
        let BridgeInfo::Host { sharing: true, code: Some(code), address: Some(addr), browser: None, .. } = b.info() else {
            panic!("{:?}", b.info())
        };
        assert!(addr.starts_with("https://"));
        assert!(b.link().unwrap().ends_with(&code.replace('-', "")));
        assert!(!b.browser_controls());
        b.stop();
        // The same code after a restart of the app (the secret is saved).
        let mut again = Bridge::load(Some(files.secret(SECTION)));
        again.start(false).unwrap();
        let BridgeInfo::Host { code: Some(code2), .. } = again.info() else { panic!() };
        assert_eq!(code2, code);
        again.start(true).unwrap();
        let BridgeInfo::Host { code: Some(code3), .. } = again.info() else { panic!() };
        assert_ne!(code3, code);
        assert!(again.status().contains(&code3));
        assert_eq!(again.stop(), "stopped sharing the trainer");
        assert_eq!(again.stop(), "the trainer is not shared");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

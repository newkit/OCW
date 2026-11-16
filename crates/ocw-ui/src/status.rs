//! Rider status signs in both apps (spec §2.6 *Rider status signs*,
//! §7.1.7 Q11): every frame the own signs – in the menu (§2.6 Q2: the
//! in-game menu, the console, the workout and world pickers, which are
//! menu pages; not the workshop, not Unicorn mode's inspection), the
//! microphone muted and whom the rider does not hear (from the voice
//! HUD's [`VoiceSigns`], computed by `ocw_client::voice`), riding in VR
//! ([`VrStatus`], set by the VR session) – go to the client, which sends
//! them with its samples (at once on a change). The signs cannot be hidden
//! (§2.6 Q3: there is no setting).
//!
//! Console: `status.sign` shows the own signs and forces one for scenes and
//! tests (local server only), `dev.bot_status` sets the bots' signs.

use bevy::prelude::*;
use clap::{Arg, ArgAction, Command};
use ocw_client::client::ClientCore;
use ocw_protocol::v1;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::menu::InputContext;
use ocw_render::voice::{VoiceHud, VoiceSigns};

/// Riding in VR with the headset on (spec §2.6: the OpenXR session
/// *visible* or *focused*, or `--vr-sim`). Set by the VR session (§14
/// Phase 11, not built yet); false without one.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VrStatus(pub bool);

/// Signs forced with `status.sign` (scenes, tests; `None`: as they are).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ForcedSigns {
    pub muted: Option<bool>,
    pub menu: Option<bool>,
    pub vr: Option<bool>,
    /// Hears nobody.
    pub deaf: Option<bool>,
    /// In the voice call (to see headphones on others without a voice
    /// server).
    pub voice: Option<bool>,
}

/// Whether the context counts as "in the menu" (spec §2.6 Q2).
pub fn in_menu(context: Option<InputContext>) -> bool {
    matches!(context, Some(InputContext::Menu | InputContext::Console))
}

/// The own status from what the frame knows, with the forced signs.
pub fn own_status(
    voice: &VoiceSigns,
    menu: bool,
    vr: bool,
    forced: &ForcedSigns,
) -> v1::RiderStatus {
    let mut voice = voice.clone();
    if let Some(on) = forced.voice {
        voice.in_voice = on;
    }
    if let Some(on) = forced.muted {
        voice.mic_muted = on;
    }
    if let Some(on) = forced.deaf {
        voice.hears_nobody = on;
    }
    ocw_client::voice::status_message(&voice, forced.menu.unwrap_or(menu), forced.vr.unwrap_or(vr))
}

pub struct StatusPlugin;

impl Plugin for StatusPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VrStatus>()
            .init_resource::<ForcedSigns>()
            .add_systems(Update, send_status.after(crate::RideViewSet));
        register_commands(app);
    }
}

/// Hands the own signs to the client; a change goes out at once.
fn send_status(world: &mut World) {
    let menu = in_menu(world.get_resource::<InputContext>().copied());
    let voice = world
        .get_resource::<VoiceHud>()
        .map(|h| h.signs.clone())
        .unwrap_or_default();
    let vr = world.get_resource::<VrStatus>().is_some_and(|v| v.0);
    let forced = world
        .get_resource::<ForcedSigns>()
        .copied()
        .unwrap_or_default();
    let status = own_status(&voice, menu, vr, &forced);
    let msg = crate::client_mut(world).and_then(|mut c| c.set_status(status));
    if let Some(m) = msg {
        crate::send(world, m);
    }
}

fn describe(s: &v1::RiderStatus) -> String {
    let mut out = Vec::new();
    if s.voice_muted {
        out.push("microphone muted (X mouth)".to_string());
    }
    if s.in_menu {
        out.push("in the menu (phone)".into());
    }
    if s.in_vr {
        out.push("in VR (headset)".into());
    }
    if s.hears_nobody {
        out.push("hears nobody (headphones for everybody in the call)".into());
    }
    if !s.not_hearing.is_empty() {
        out.push(format!("does not hear {} rider(s)", s.not_hearing.len()));
    }
    if out.is_empty() {
        out.push("no signs".into());
    }
    format!(
        "{}{}",
        out.join(", "),
        if s.in_voice {
            "; in the voice call"
        } else {
            ""
        }
    )
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("status.sign")
            .about("The own status signs (spec §2.6); force one for scenes and tests: muted, menu, vr, deaf (hears nobody), voice (in the call) – on, off or auto")
            .arg(Arg::new("sign").value_parser(["muted", "menu", "vr", "deaf", "voice"]))
            .arg(Arg::new("state").value_parser(["on", "off", "auto"])),
        |world, m| {
            if let Some(sign) = m.get_one::<String>("sign") {
                let state = m
                    .get_one::<String>("state")
                    .map_or("on", String::as_str);
                let v = match state {
                    "on" => Some(true),
                    "off" => Some(false),
                    _ => None,
                };
                let mut f = world.resource_mut::<ForcedSigns>();
                match sign.as_str() {
                    "muted" => f.muted = v,
                    "menu" => f.menu = v,
                    "vr" => f.vr = v,
                    "deaf" => f.deaf = v,
                    _ => f.voice = v,
                }
            }
            send_status(world);
            let c = crate::client(world).ok_or("not riding")?;
            Ok(describe(c.status()))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("dev.bot_status")
            .about("Set the status signs of a bot (or every bot): the flags given are on, the others off (spec §2.6)")
            .arg(
                Arg::new("bot")
                    .long("bot")
                    .value_parser(clap::value_parser!(u32))
                    .help("The bot's rider id (default: every bot)"),
            )
            .arg(Arg::new("muted").long("muted").action(ArgAction::SetTrue))
            .arg(Arg::new("menu").long("menu").action(ArgAction::SetTrue))
            .arg(Arg::new("vr").long("vr").action(ArgAction::SetTrue))
            .arg(
                Arg::new("deaf")
                    .long("deaf")
                    .action(ArgAction::SetTrue)
                    .help("Hears nobody: headphones for every rider in the call"),
            )
            .arg(
                Arg::new("not-hearing")
                    .long("not-hearing")
                    .value_parser(clap::value_parser!(u32))
                    .action(ArgAction::Append)
                    .help("Does not hear this rider id (only that rider sees the headphones)"),
            ),
        |world, m| {
            let on = |k: &str| m.get_flag(k);
            let status = v1::RiderStatus {
                voice_muted: on("muted"),
                in_menu: on("menu"),
                in_vr: on("vr"),
                in_voice: on("muted") || on("deaf") || m.contains_id("not-hearing"),
                hears_nobody: on("deaf"),
                not_hearing: m
                    .get_many::<u32>("not-hearing")
                    .map(|v| v.copied().collect())
                    .unwrap_or_default(),
            };
            let bot = m.get_one::<u32>("bot").copied().unwrap_or(0);
            if crate::send(world, ClientCore::bot_status_message(bot, status)) {
                Ok(String::new())
            } else {
                Err("not riding".into())
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_menu_and_the_console_count_as_in_the_menu() {
        assert!(in_menu(Some(InputContext::Menu)));
        assert!(in_menu(Some(InputContext::Console)));
        assert!(!in_menu(Some(InputContext::Ride)));
        // The workshop scene is not one of §2.6 Q2's screens.
        assert!(!in_menu(Some(InputContext::Workshop)));
        assert!(!in_menu(None));
    }

    #[test]
    fn forced_signs_override_the_frame() {
        let voice = VoiceSigns {
            in_voice: true,
            mic_muted: true,
            hears_nobody: false,
            not_hearing: vec![ocw_core::RiderId(4)],
        };
        let s = own_status(&voice, false, false, &ForcedSigns::default());
        assert!(s.voice_muted && s.in_voice && !s.in_menu && !s.in_vr);
        assert_eq!(s.not_hearing, vec![4]);
        let f = ForcedSigns {
            muted: Some(false),
            menu: Some(true),
            vr: Some(true),
            deaf: Some(true),
            voice: None,
        };
        let s = own_status(&voice, false, false, &f);
        assert!(!s.voice_muted && s.in_menu && s.in_vr && s.hears_nobody);
    }
}

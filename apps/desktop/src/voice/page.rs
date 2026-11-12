//! Settings → Voice (spec §3.2.1 M8, §2.4) as rows of command lines, like
//! the other Settings pages: the mute switches, volume, participants
//! (mute one for yourself), devices and options. Bevy-free.

use crate::settings::pages::{Activate, Page, Row, setting_row};
use crate::settings::registry::Settings;

use super::VoiceMenuInfo;

fn open(label: &str, value: String, page: Page) -> Row {
    Row {
        label: label.into(),
        value: Some(value),
        left: None,
        right: None,
        confirm: Some(Activate::Open(page)),
    }
}

/// A switch row whose ← / → / Confirm all toggle (`line`).
fn toggle(label: &str, value: &str, line: &str) -> Row {
    Row {
        label: label.into(),
        value: Some(value.into()),
        left: Some(line.into()),
        right: Some(line.into()),
        confirm: Some(Activate::Command(line.into())),
    }
}

/// A device row: ← / → step through the default and the present devices.
fn device_row(label: &str, key: &str, saved: &str, present: &[String], missing: bool) -> Row {
    let current = (saved != "default").then_some(saved);
    let line = |dir| {
        let next = ocw_voice::device::step(current, present, dir);
        format!("set {key} {}", next.as_deref().unwrap_or("default"))
    };
    let value = match current {
        None => "default".to_owned(),
        Some(n) if missing => format!("{n} (not found: default)"),
        Some(n) => n.to_owned(),
    };
    Row {
        label: label.into(),
        value: Some(value),
        left: Some(line(-1)),
        right: Some(line(1)),
        confirm: Some(Activate::Command(line(1))),
    }
}

/// The rows of a voice page.
pub fn rows(page: Page, settings: &Settings, info: &VoiceMenuInfo) -> Vec<Row> {
    let text = |key: &str| settings.get(key).map(|v| v.to_string()).unwrap_or_default();
    let on = |key: &str| {
        settings
            .get(key)
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    };
    match page {
        Page::Voice => vec![
            toggle(
                "Microphone",
                if on("voice.mic_muted") { "muted" } else { "on" },
                "voice.mute-mic",
            ),
            toggle(
                "Speaker (voices)",
                if on("voice.speaker_muted") {
                    "muted"
                } else {
                    "on"
                },
                "voice.mute-speaker",
            ),
            setting_row(settings, "voice.volume", "Voice volume (%)"),
            open(
                "Participants",
                info.participants.len().to_string(),
                Page::VoiceParticipants,
            ),
            open("Devices and options", String::new(), Page::VoiceDevices),
            Row {
                label: "Voice chat".into(),
                value: Some(format!(
                    "{} - {}",
                    if on("voice.enabled") { "on" } else { "off" },
                    info.status
                )),
                left: Some(format!(
                    "set voice.enabled {}",
                    if on("voice.enabled") { "off" } else { "on" }
                )),
                right: Some(format!(
                    "set voice.enabled {}",
                    if on("voice.enabled") { "off" } else { "on" }
                )),
                confirm: Some(Activate::Command(format!(
                    "set voice.enabled {}",
                    if on("voice.enabled") { "off" } else { "on" }
                ))),
            },
        ],
        Page::VoiceDevices => vec![
            device_row(
                "Microphone",
                "voice.input",
                &text("voice.input"),
                &info.inputs,
                info.input_missing,
            ),
            device_row(
                "Speaker",
                "voice.output",
                &text("voice.output"),
                &info.outputs,
                info.output_missing,
            ),
            setting_row(settings, "voice.spatial", "Spatial voice"),
            setting_row(settings, "voice.max_voices", "Voices heard at most"),
        ],
        Page::VoiceParticipants => {
            if info.participants.is_empty() {
                return vec![Row {
                    label: "Nobody else in the voice room".into(),
                    value: None,
                    left: None,
                    right: None,
                    confirm: None,
                }];
            }
            info.participants
                .iter()
                .map(|p| {
                    let line = format!("voice.mute {}", p.identity);
                    let state = if p.muted_by_me {
                        "muted by you".to_owned()
                    } else if p.mic_muted {
                        "microphone off".to_owned()
                    } else if let Some(g) = p.heard {
                        format!("heard {:.0} %", g * 100.0)
                    } else {
                        "too far away".to_owned()
                    };
                    toggle(&p.name, &state, &line)
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voice::MenuParticipant;

    fn row<'a>(rows: &'a [Row], label: &str) -> &'a Row {
        rows.iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("{label}"))
    }

    fn cmd(line: &str) -> Option<Activate> {
        Some(Activate::Command(line.into()))
    }

    fn info() -> VoiceMenuInfo {
        VoiceMenuInfo {
            status: "connected".into(),
            inputs: vec!["Mic A".into(), "Headset".into()],
            outputs: vec!["Speakers".into()],
            input_missing: false,
            output_missing: true,
            participants: vec![
                MenuParticipant {
                    identity: "rider-2".into(),
                    name: "Rider Two".into(),
                    muted_by_me: false,
                    mic_muted: false,
                    heard: Some(0.5),
                },
                MenuParticipant {
                    identity: "rider-3".into(),
                    name: "Rider Three".into(),
                    muted_by_me: true,
                    mic_muted: false,
                    heard: None,
                },
            ],
        }
    }

    #[test]
    fn voice_page_toggles_and_opens() {
        let (s, _) = Settings::parse("[voice]\nmic_muted = true\n").unwrap();
        let r = rows(Page::Voice, &s, &info());
        assert!(r.len() <= 6, "ten-foot limit");
        let mic = row(&r, "Microphone");
        assert_eq!(mic.value.as_deref(), Some("muted"));
        assert_eq!(mic.confirm, cmd("voice.mute-mic"));
        assert_eq!(mic.left.as_deref(), Some("voice.mute-mic"));
        let spk = row(&r, "Speaker (voices)");
        assert_eq!(spk.value.as_deref(), Some("on"));
        assert_eq!(spk.confirm, cmd("voice.mute-speaker"));
        assert_eq!(
            row(&r, "Voice volume (%)").right.as_deref(),
            Some("set voice.volume 110")
        );
        assert_eq!(row(&r, "Participants").value.as_deref(), Some("2"));
        assert_eq!(
            row(&r, "Participants").confirm,
            Some(Activate::Open(Page::VoiceParticipants))
        );
        let chat = row(&r, "Voice chat");
        assert_eq!(chat.value.as_deref(), Some("on - connected"));
        assert_eq!(chat.confirm, cmd("set voice.enabled off"));
    }

    #[test]
    fn device_page_steps_through_devices() {
        let (s, _) = Settings::parse("[voice]\noutput = \"HDMI\"\n").unwrap();
        let r = rows(Page::VoiceDevices, &s, &info());
        let mic = row(&r, "Microphone");
        assert_eq!(mic.value.as_deref(), Some("default"));
        assert_eq!(mic.right.as_deref(), Some("set voice.input Mic A"));
        assert_eq!(mic.left.as_deref(), Some("set voice.input Headset"));
        let spk = row(&r, "Speaker");
        assert_eq!(spk.value.as_deref(), Some("HDMI (not found: default)"));
        assert_eq!(spk.right.as_deref(), Some("set voice.output Speakers"));
        assert_eq!(
            row(&r, "Spatial voice").confirm,
            cmd("set voice.spatial off")
        );
        assert_eq!(row(&r, "Voices heard at most").value.as_deref(), Some("10"));
    }

    #[test]
    fn participants_page_mutes_one_rider() {
        let r = rows(Page::VoiceParticipants, &Settings::default(), &info());
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].value.as_deref(), Some("heard 50 %"));
        assert_eq!(r[0].confirm, cmd("voice.mute rider-2"));
        assert_eq!(r[1].value.as_deref(), Some("muted by you"));
        let empty = rows(
            Page::VoiceParticipants,
            &Settings::default(),
            &VoiceMenuInfo::default(),
        );
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].confirm, None);
    }
}

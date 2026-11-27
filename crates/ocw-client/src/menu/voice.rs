//! Settings → Voice (spec §3.2.1 M8, §2.4) as rows of command lines, like
//! the other Settings pages: *My voice* (on / off · Everybody / Selected)
//! and *I hear* (everybody / selected / nobody) as two separate rows (spec
//! §7.1.7 Q16), *Choose riders* (one list for both, Q19), the speaker,
//! participants (each with Mute for me, a volume and Block, spec §7.1.7
//! Q9), the blocked riders, devices and options. Six rows (ten-foot rule):
//! the old *Voice chat* row (voice on / off) is gone – its off is both
//! rows off (`voice.off` does it in one command) and its state is the
//! *Participants* row's value. Bevy-free.

use super::pages::{Activate, Page, Row, setting_row};
use crate::settings::Settings;

use crate::voice::{ChoiceRider, VoiceMenuInfo, my_voice_text};
use ocw_voice::audience::{Listen, MyVoice};

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

/// A row that only informs.
fn info_line(label: &str) -> Row {
    Row {
        label: label.into(),
        value: None,
        left: None,
        right: None,
        confirm: None,
    }
}

fn info_line_value(label: &str, value: String) -> Row {
    Row {
        value: Some(value),
        ..info_line(label)
    }
}

/// A participant's state in the list: blocked, muted, the microphone
/// off, heard at how much, not selected, or out of hearing (too far, or
/// near but not letting the rider hear them: "private"); the own volume
/// if changed.
fn participant_state(p: &crate::voice::MenuParticipant, info: &VoiceMenuInfo) -> String {
    let state = if p.blocked {
        "blocked".to_owned()
    } else if p.muted_by_me {
        "muted by you".to_owned()
    } else if info.listen == Listen::Nobody {
        "you hear nobody".to_owned()
    } else if info.listen == Listen::Selected && !p.selected {
        "not selected".to_owned()
    } else if p.mic_muted {
        "microphone off".to_owned()
    } else if let Some(g) = p.heard {
        format!("heard {:.0} %", g * 100.0)
    } else if !p.may_hear
        && p.distance_m
            .is_some_and(|d| d < ocw_voice::spatial::RANGE_M)
    {
        "private".to_owned()
    } else {
        "too far away".to_owned()
    };
    if p.volume_pct == ocw_voice::riders::DEFAULT_VOLUME_PCT || p.blocked {
        state
    } else {
        format!("{state}, volume {} %", p.volume_pct)
    }
}

/// A choice row: ← / → / Confirm all run `line(value)` with the next or
/// previous of `values` (cycling).
fn choice_row(label: &str, values: &[(&str, &str)], current: &str, key: &str) -> Row {
    let at = values.iter().position(|(k, _)| *k == current).unwrap_or(0);
    let n = values.len();
    let line = |i: usize| format!("set {key} {}", values[i % n].0);
    Row {
        label: label.into(),
        value: Some(values[at].1.into()),
        left: Some(line(at + n - 1)),
        right: Some(line(at + 1)),
        confirm: Some(Activate::Command(line(at + 1))),
    }
}

/// "3 selected (2 here)".
fn selection_summary(info: &VoiceMenuInfo) -> String {
    let all = info.choices.iter().filter(|c| c.selected).count();
    let here = info.choices.iter().filter(|c| c.selected && c.here).count();
    format!("{all} selected ({here} here)")
}

/// The *My voice* row (spec §7.1.7 Q16): Confirm turns it on / off (the
/// microphone, M; every ride starts off), ← / → choose whom it reaches
/// (Everybody / Selected, kept while it is off).
fn my_voice_row(info: &VoiceMenuInfo) -> Row {
    let other = match info.my_voice {
        MyVoice::Everybody => MyVoice::Selected,
        MyVoice::Selected => MyVoice::Everybody,
    };
    let audience = format!("set voice.my_voice {}", other.key());
    Row {
        label: "My voice".into(),
        value: Some(my_voice_text(!info.mic_muted, info.my_voice)),
        left: Some(audience.clone()),
        right: Some(audience),
        confirm: Some(Activate::Command("voice.mute-mic toggle".into())),
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
            my_voice_row(info),
            choice_row(
                "I hear",
                &Listen::ALL.map(|l| (l.key(), l.label())),
                &text("voice.i_hear"),
                "voice.i_hear",
            ),
            open("Choose riders", selection_summary(info), Page::VoiceSelect),
            toggle(
                "Speaker (voices)",
                if on("voice.speaker_muted") {
                    "muted"
                } else {
                    "on"
                },
                "voice.mute-speaker",
            ),
            // The call's state (before on the *Voice chat* row), short: a
            // reason after ":" is in `voice.status`.
            open(
                "Participants",
                {
                    let state = info.status.split(':').next().unwrap_or_default();
                    if info.blocked.is_empty() {
                        state.to_owned()
                    } else {
                        format!("{state}, {} blocked", info.blocked.len())
                    }
                },
                Page::VoiceParticipants,
            ),
            open("Devices and options", String::new(), Page::VoiceDevices),
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
            setting_row(settings, "voice.volume", "Voice volume (%)"),
            setting_row(settings, "voice.spatial", "Spatial voice"),
            setting_row(settings, "voice.max_voices", "Voices heard at most"),
            // The voice volume and the echo test (on the Audio page's
            // Voice row until Settings → Voice made room for Traffic, spec
            // 14 Phase 18).
            open(
                "Volume and echo test",
                format!(
                    "{} %{}",
                    text("audio.voice"),
                    if on("audio.echo_bot") {
                        " - echo test"
                    } else {
                        ""
                    }
                ),
                Page::AudioVoice,
            ),
        ],
        Page::VoiceSelect => {
            let here: Vec<&ChoiceRider> = info.choices.iter().filter(|c| c.here).collect();
            let gone: Vec<&ChoiceRider> = info.choices.iter().filter(|c| !c.here).collect();
            let row = |c: &ChoiceRider| {
                let line = format!("voice.select \"{}\" toggle", c.who);
                let mut value = if c.selected { "[x]" } else { "[ ]" }.to_owned();
                if let Some(d) = c.distance_m {
                    value.push_str(&format!(" {d:.0} m"));
                }
                if c.speaking {
                    value.push_str(" talking");
                }
                Row {
                    label: c.name.clone(),
                    value: Some(value),
                    left: Some(line.clone()),
                    right: Some(line.clone()),
                    confirm: Some(Activate::Command(line)),
                }
            };
            let mut out: Vec<Row> = here.into_iter().map(row).collect();
            if out.is_empty() {
                out.push(info_line("Nobody else in the voice room"));
            }
            if !gone.is_empty() {
                out.push(info_line("Not here now"));
                out.extend(gone.into_iter().map(|c| Row {
                    label: format!("{} (not here)", c.name),
                    ..row(c)
                }));
            }
            out
        }
        // Settings → Audio → Voice: the voice bus and the echo test (spec
        // §7 *Echo bot*).
        Page::AudioVoice => vec![
            Row {
                confirm: Some(Activate::Command("audio.play test-voice".into())),
                ..setting_row(settings, "audio.voice", "Voice volume")
            },
            Row {
                // The echo test as it runs (also on by `--echo-bot`).
                value: Some(match (on("audio.echo_bot"), info.echo.is_empty()) {
                    (_, false) => format!("on - {}", info.echo),
                    (true, true) => "on".into(),
                    (false, true) => "off".into(),
                }),
                ..setting_row(settings, "audio.echo_bot", "Echo test")
            },
            setting_row(settings, "audio.echo_delay_ms", "Echo delay (ms)"),
        ],
        Page::VoiceParticipants => {
            let mut out: Vec<Row> = info
                .participants
                .iter()
                .enumerate()
                .map(|(i, p)| open(&p.name, participant_state(p, info), Page::VoiceRider(i)))
                .collect();
            if out.is_empty() {
                out.push(info_line("Nobody else in the voice room"));
            }
            out.push(open(
                "Blocked riders",
                info.blocked.len().to_string(),
                Page::VoiceBlocked,
            ));
            out
        }
        Page::VoiceRider(i) => {
            let Some(p) = info.participants.get(i) else {
                return vec![info_line("Not in the voice room any more")];
            };
            let id = &p.identity;
            let step = i64::from(ocw_voice::riders::VOLUME_STEP_PCT);
            let vol = |d: i64| {
                format!(
                    "voice.volume {id} {}",
                    ocw_voice::riders::clamp_volume(i64::from(p.volume_pct) + d * step)
                )
            };
            let block = format!("voice.block {id} {}", if p.blocked { "off" } else { "on" });
            vec![
                info_line_value(&p.name, participant_state(p, info)),
                toggle(
                    "Mute for me",
                    if p.muted_by_me { "muted" } else { "off" },
                    &format!("voice.mute {id}"),
                ),
                Row {
                    label: "Volume (%)".into(),
                    value: Some(p.volume_pct.to_string()),
                    left: Some(vol(-1)),
                    right: Some(vol(1)),
                    // Confirm: back to 100 %.
                    confirm: Some(Activate::Command(format!(
                        "voice.volume {id} {}",
                        ocw_voice::riders::DEFAULT_VOLUME_PCT
                    ))),
                },
                Row {
                    label: "Block".into(),
                    value: Some(if p.blocked { "blocked" } else { "off" }.into()),
                    left: Some(block.clone()),
                    right: Some(block.clone()),
                    confirm: Some(Activate::Command(block)),
                },
            ]
        }
        Page::VoiceBlocked => {
            if info.blocked.is_empty() {
                return vec![info_line("Nobody blocked")];
            }
            info.blocked
                .iter()
                .map(|b| Row {
                    label: b.name.clone(),
                    value: Some("blocked".into()),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Command(format!("voice.block \"{}\" off", b.key))),
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
            echo: "speak now".into(),
            participants: vec![
                MenuParticipant {
                    identity: "rider-2".into(),
                    name: "Rider Two".into(),
                    muted_by_me: false,
                    mic_muted: false,
                    heard: Some(0.5),
                    blocked: false,
                    volume_pct: 100,
                    selected: true,
                    may_hear: true,
                    distance_m: Some(20.0),
                },
                MenuParticipant {
                    identity: "rider-3".into(),
                    name: "Rider Three".into(),
                    muted_by_me: true,
                    mic_muted: false,
                    heard: None,
                    blocked: false,
                    volume_pct: 150,
                    selected: false,
                    may_hear: true,
                    distance_m: Some(5.0),
                },
            ],
            blocked: Vec::new(),
            mic_muted: true,
            ..Default::default()
        }
    }

    /// The Voice page (spec §7.1.7 Q16): *My voice* and *I hear* as two
    /// separate rows, *Choose riders*, the speaker, the participants with
    /// the call's state, the devices – six rows, no *Voice chat* row.
    #[test]
    fn voice_page_toggles_and_opens() {
        let s = Settings::default();
        let r = rows(Page::Voice, &s, &info());
        assert!(r.len() <= 6, "ten-foot limit");
        let labels: Vec<&str> = r.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "My voice",
                "I hear",
                "Choose riders",
                "Speaker (voices)",
                "Participants",
                "Devices and options"
            ]
        );
        // My voice as it is now (joined with it off, spec §7.1.7 Q7), not
        // a saved setting: Confirm turns it on / off (M), ← / → choose
        // whom it reaches.
        let mine = row(&r, "My voice");
        assert_eq!(mine.value.as_deref(), Some("off (everybody)"));
        assert_eq!(mine.confirm, cmd("voice.mute-mic toggle"));
        assert_eq!(mine.left.as_deref(), Some("set voice.my_voice selected"));
        assert_eq!(mine.right.as_deref(), Some("set voice.my_voice selected"));
        let on = VoiceMenuInfo {
            mic_muted: false,
            my_voice: MyVoice::Selected,
            ..info()
        };
        let r2 = rows(Page::Voice, &s, &on);
        let mine = row(&r2, "My voice");
        assert_eq!(mine.value.as_deref(), Some("on, selected"));
        assert_eq!(mine.right.as_deref(), Some("set voice.my_voice everybody"));
        // I hear: everybody → selected → nobody → everybody.
        let hear = row(&r, "I hear");
        assert_eq!(hear.value.as_deref(), Some("Everybody"));
        assert_eq!(hear.confirm, cmd("set voice.i_hear selected"));
        assert_eq!(hear.left.as_deref(), Some("set voice.i_hear nobody"));
        let (nobody, _) = Settings::parse("[voice]\ni_hear = \"nobody\"\n").unwrap();
        let r3 = rows(Page::Voice, &nobody, &info());
        assert_eq!(row(&r3, "I hear").value.as_deref(), Some("Nobody"));
        assert_eq!(
            row(&r3, "I hear").confirm,
            cmd("set voice.i_hear everybody")
        );
        let spk = row(&r, "Speaker (voices)");
        assert_eq!(spk.value.as_deref(), Some("on"));
        assert_eq!(spk.confirm, cmd("voice.mute-speaker"));
        let choose = row(&r, "Choose riders");
        assert_eq!(choose.value.as_deref(), Some("0 selected (0 here)"));
        assert_eq!(choose.confirm, Some(Activate::Open(Page::VoiceSelect)));
        // The call's state (once the *Voice chat* row's).
        assert_eq!(row(&r, "Participants").value.as_deref(), Some("connected"));
        let unavailable = VoiceMenuInfo {
            status: "unavailable: voice chat is not built into this app".into(),
            ..info()
        };
        let r4 = rows(Page::Voice, &s, &unavailable);
        assert_eq!(
            row(&r4, "Participants").value.as_deref(),
            Some("unavailable"),
            "short; the reason is in voice.status"
        );
        assert_eq!(
            row(&r, "Participants").confirm,
            Some(Activate::Open(Page::VoiceParticipants))
        );
        assert!(r.iter().all(|r| r.label != "Voice chat"));
        assert_eq!(Page::VoiceSelect.parent(), Some(Page::Voice));
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
        assert_eq!(row(&r, "Voices heard at most").value.as_deref(), Some("24"));
        assert_eq!(
            row(&r, "Voice volume (%)").right.as_deref(),
            Some("set voice.volume 110")
        );
        assert!(r.len() <= 6, "ten-foot limit");
    }

    /// Voice → Choose riders (spec §7.1.3, §7.1.7 Q19: one list for both
    /// rows), its summary on the Voice page.
    #[test]
    fn choose_riders_lists_the_selection() {
        let mut i = info();
        i.choices = vec![
            ChoiceRider {
                who: "rider-2".into(),
                name: "Bea Climber".into(),
                selected: true,
                here: true,
                distance_m: Some(12.4),
                speaking: true,
            },
            ChoiceRider {
                who: "rider-3".into(),
                name: "Cleo Sprinter".into(),
                selected: false,
                here: true,
                distance_m: None,
                speaking: false,
            },
            ChoiceRider {
                who: "name:local/Dan Descender".into(),
                name: "Dan Descender".into(),
                selected: true,
                here: false,
                distance_m: None,
                speaking: false,
            },
        ];
        let r = rows(Page::Voice, &Settings::default(), &i);
        let choose = row(&r, "Choose riders");
        assert_eq!(choose.value.as_deref(), Some("2 selected (1 here)"));
        assert_eq!(choose.confirm, Some(Activate::Open(Page::VoiceSelect)));
        // Choose riders: the riders here with a check mark, then the
        // selected ones not here now; Confirm toggles.
        let r = rows(Page::VoiceSelect, &Settings::default(), &i);
        assert_eq!(r[0].label, "Bea Climber");
        assert_eq!(r[0].value.as_deref(), Some("[x] 12 m talking"));
        assert_eq!(r[0].confirm, cmd("voice.select \"rider-2\" toggle"));
        assert_eq!(r[1].value.as_deref(), Some("[ ]"));
        assert_eq!(r[2].label, "Not here now");
        assert_eq!(r[2].confirm, None);
        assert_eq!(r[3].label, "Dan Descender (not here)");
        assert_eq!(
            r[3].confirm,
            cmd("voice.select \"name:local/Dan Descender\" toggle")
        );
        let empty = rows(
            Page::VoiceSelect,
            &Settings::default(),
            &VoiceMenuInfo::default(),
        );
        assert_eq!(empty.len(), 1);
        assert_eq!(empty[0].confirm, None);
    }

    /// The participants' states with the voice room (spec §7.1.3, Q6):
    /// "not selected" when listening to the selection only, "private" for
    /// a near rider who does not let the own rider hear them.
    #[test]
    fn participant_states_with_the_voice_room() {
        let mut i = info();
        i.participants[1].muted_by_me = false;
        i.participants[1].may_hear = false;
        let r = rows(Page::VoiceParticipants, &Settings::default(), &i);
        assert_eq!(r[1].value.as_deref(), Some("private, volume 150 %"));
        i.participants[1].distance_m = Some(800.0);
        let r = rows(Page::VoiceParticipants, &Settings::default(), &i);
        assert_eq!(r[1].value.as_deref(), Some("too far away, volume 150 %"));
        i.listen = Listen::Selected;
        let r = rows(Page::VoiceParticipants, &Settings::default(), &i);
        assert_eq!(r[0].value.as_deref(), Some("heard 50 %"), "selected");
        assert_eq!(r[1].value.as_deref(), Some("not selected, volume 150 %"));
        // I hear nobody (spec §7.1.7 Q16).
        i.listen = Listen::Nobody;
        let r = rows(Page::VoiceParticipants, &Settings::default(), &i);
        assert_eq!(r[0].value.as_deref(), Some("you hear nobody"));
    }

    /// Settings → Audio → Voice (spec §7 *Echo bot*, §3.2.1).
    #[test]
    fn audio_voice_page_has_the_echo_test() {
        // The echo test off: no state from the controller.
        let off = VoiceMenuInfo {
            echo: String::new(),
            ..info()
        };
        let r = rows(Page::AudioVoice, &Settings::default(), &off);
        assert!(r.len() < crate::menu::model::MAX_ITEMS, "fits one page");
        assert_eq!(Page::AudioVoice.parent(), Some(Page::VoiceDevices));
        // Reached from Voice → Devices and options (spec 14 Phase 18).
        let d = rows(Page::VoiceDevices, &Settings::default(), &off);
        assert!(d.len() < crate::menu::model::MAX_ITEMS);
        assert_eq!(
            row(&d, "Volume and echo test").confirm,
            Some(Activate::Open(Page::AudioVoice))
        );
        assert_eq!(
            row(&d, "Volume and echo test").value.as_deref(),
            Some("100 %")
        );
        let vol = row(&r, "Voice volume");
        assert_eq!(vol.confirm, cmd("audio.play test-voice"));
        let echo = row(&r, "Echo test");
        assert_eq!(echo.value.as_deref(), Some("off"));
        assert_eq!(echo.confirm, cmd("set audio.echo_bot on"));
        let delay = row(&r, "Echo delay (ms)");
        assert_eq!(delay.value.as_deref(), Some("300"));
        assert_eq!(delay.right.as_deref(), Some("set audio.echo_delay_ms 400"));
        assert_eq!(delay.left.as_deref(), Some("set audio.echo_delay_ms 200"));
        let (s, _) = Settings::parse("[audio]\necho_bot = true\n").unwrap();
        let r = rows(Page::AudioVoice, &s, &info());
        let echo = row(&r, "Echo test");
        assert_eq!(echo.value.as_deref(), Some("on - speak now"));
        assert_eq!(echo.confirm, cmd("set audio.echo_bot off"));
        // On by `--echo-bot` (the setting off): shown as it runs.
        let r = rows(Page::AudioVoice, &Settings::default(), &info());
        assert_eq!(
            row(&r, "Echo test").value.as_deref(),
            Some("on - speak now")
        );
    }

    /// The participants' list (spec §7.1.7 Q9): each rider opens a page
    /// with Mute for me, the volume and Block; the blocked riders at the
    /// end, Confirm unblocks.
    #[test]
    fn participants_page_opens_each_rider() {
        let r = rows(Page::VoiceParticipants, &Settings::default(), &info());
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].value.as_deref(), Some("heard 50 %"));
        assert_eq!(r[0].confirm, Some(Activate::Open(Page::VoiceRider(0))));
        assert_eq!(r[1].value.as_deref(), Some("muted by you, volume 150 %"));
        assert_eq!(r[2].label, "Blocked riders");
        assert_eq!(r[2].value.as_deref(), Some("0"));
        assert_eq!(r[2].confirm, Some(Activate::Open(Page::VoiceBlocked)));
        let empty = rows(
            Page::VoiceParticipants,
            &Settings::default(),
            &VoiceMenuInfo::default(),
        );
        assert_eq!(empty.len(), 2);
        assert_eq!(empty[0].confirm, None);
    }

    #[test]
    fn a_riders_page_mutes_sets_the_volume_and_blocks() {
        let r = rows(Page::VoiceRider(1), &Settings::default(), &info());
        assert!(r.len() < crate::menu::model::MAX_ITEMS, "fits one page");
        assert_eq!(r[0].label, "Rider Three");
        assert_eq!(r[0].confirm, None);
        let mute = row(&r, "Mute for me");
        assert_eq!(mute.value.as_deref(), Some("muted"));
        assert_eq!(mute.confirm, cmd("voice.mute rider-3"));
        let vol = row(&r, "Volume (%)");
        assert_eq!(vol.value.as_deref(), Some("150"));
        assert_eq!(vol.left.as_deref(), Some("voice.volume rider-3 140"));
        assert_eq!(vol.right.as_deref(), Some("voice.volume rider-3 160"));
        assert_eq!(vol.confirm, cmd("voice.volume rider-3 100"));
        let block = row(&r, "Block");
        assert_eq!(block.value.as_deref(), Some("off"));
        assert_eq!(block.confirm, cmd("voice.block rider-3 on"));
        // At the ends of the range the steps stop.
        let mut i = info();
        i.participants[0].volume_pct = 200;
        i.participants[0].blocked = true;
        let r = rows(Page::VoiceRider(0), &Settings::default(), &i);
        assert_eq!(
            row(&r, "Volume (%)").right.as_deref(),
            Some("voice.volume rider-2 200")
        );
        assert_eq!(row(&r, "Block").value.as_deref(), Some("blocked"));
        assert_eq!(row(&r, "Block").confirm, cmd("voice.block rider-2 off"));
        assert_eq!(r[0].value.as_deref(), Some("blocked"));
        // The rider left: the page says so.
        let gone = rows(Page::VoiceRider(9), &Settings::default(), &info());
        assert_eq!(gone.len(), 1);
        assert_eq!(gone[0].confirm, None);
    }

    #[test]
    fn the_blocked_list_unblocks() {
        let mut i = info();
        i.blocked = vec![crate::voice::BlockedRider {
            key: "name:ride.example.org/Bea Climber".into(),
            name: "Bea Climber".into(),
        }];
        let r = rows(Page::VoiceBlocked, &Settings::default(), &i);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].label, "Bea Climber");
        assert_eq!(
            r[0].confirm,
            cmd("voice.block \"name:ride.example.org/Bea Climber\" off")
        );
        let voice = rows(Page::Voice, &Settings::default(), &i);
        assert_eq!(
            row(&voice, "Participants").value.as_deref(),
            Some("connected, 1 blocked")
        );
        let none = rows(Page::VoiceBlocked, &Settings::default(), &info());
        assert_eq!(none[0].label, "Nobody blocked");
        assert_eq!(none[0].confirm, None);
    }
}

//! Voice chat on the client (spec §2.4, §7), shared by the desktop and the
//! browser app (spec §14 Phase 10 B5): what the apps feed the
//! [`VoiceController`] every frame and what they show of it.
//!
//! - [`VoiceDriver::frame`]: a new session's welcome, the human riders, the
//!   Echo rider and the echo test's request (spec §7 *Echo bot*), the token
//!   grants, the controller's update with the other riders' places, the
//!   messages for the server and the console, the HUD's voice indicator and
//!   the menu's voice page;
//! - [`peers`]: where the other riders are relative to the own one;
//! - [`settings_of`] / [`echo_of`]: the `[voice]` and echo settings of the
//!   shared registry ([`crate::settings`]);
//! - [`volume_command`] / [`block_command`]: a volume and Block per rider
//!   (spec §7.1.7 Q9), remembered in the settings' `[voice_riders]`; the
//!   driver tells the game server the blocks (`VoiceBlocks`) and hands
//!   its answer (`VoiceApart`) to the controller.
//!
//! The backend is the app's: LiveKit's Rust SDK on the desktop (cargo
//! feature `voice`), livekit-client (JS) in the browser
//! (`ocw_voice::web`). The token comes over the game connection in both
//! (`VoiceTokenRequest` → `VoiceGrant`; WebTransport in the browser).

use ocw_core::RiderId;
use ocw_protocol::v1;
use ocw_render::Signs;
use ocw_render::voice::{EchoHud, VoiceHud, VoiceSigns};
use ocw_render::{RideView, RiderView};
use ocw_voice::audience::{HeardBy, Listen};
use ocw_voice::hotkeys::{Input, Toggle};
use ocw_voice::spatial::Peer;
use ocw_voice::{EchoStatus, Levels, MuteState, Status, Switch, VoiceController, VoiceSettings};
use ocw_world::Track;

use crate::client::ClientCore;
use crate::settings::{Settings, Value};

/// The voice part of the own status signs (spec §2.6, §7.1.7 Q11): in the
/// call, the microphone muted (the X mouth), and whom the rider does not
/// hear – by [`not_hearing_reasons`].
pub fn voice_signs(c: &VoiceController, voice_offered: bool) -> VoiceSigns {
    let status = c.status();
    let in_voice = matches!(status, Status::Connected | Status::Reconnecting);
    let (hears_nobody, not_hearing) = not_hearing_reasons(c, &status, voice_offered);
    VoiceSigns {
        in_voice,
        mic_muted: in_voice && c.mute().mic_muted,
        hears_nobody,
        not_hearing,
    }
}

/// **Why a rider shows headphones ("cannot hear you") – spec §7.1.7 Q12,
/// still OPEN.** Q11 decided the meaning (headphones = this rider does
/// not receive the viewer's voice, seen per viewer); Q12 asks which
/// reasons count. Built here from the proposal, with only what the voice
/// state carries today:
///
/// - **hears nobody** (every viewer in the call sees the headphones): the
///   speaker is muted (Shift+M, `voice.speaker_muted`), or voice is turned
///   off in the settings while the server offers it;
/// - **does not hear these riders** (only they see the headphones): the
///   participants the rider muted for themselves (`voice.mute WHO`).
///
/// - **listens to *Selected riders* without them** (spec §7.1.3, the
///   proposal's "they listen to Selected riders without you"): the riders
///   in the call who are not selected.
///
/// Not counted (the proposal: not merely distance): riders out of hearing
/// range or beyond the subscription cap (`heard: None` without a mute),
/// and *Who hears me* (Q8, the other direction: the viewer is not heard).
/// **Block (Q9, built) does not count here** and neither does
/// a rider's volume at 0 %: whether the headphones show for them is the
/// open question §7.1.7 Q18 (a blocked rider would learn of the block
/// from the sign; Q12 / Q17 are open too), so today's signs stay as they
/// were.
pub fn not_hearing_reasons(
    c: &VoiceController,
    status: &Status,
    voice_offered: bool,
) -> (bool, Vec<RiderId>) {
    let in_voice = matches!(status, Status::Connected | Status::Reconnecting);
    let speaker_off = in_voice && c.mute().speaker_muted;
    let voice_off = matches!(status, Status::Off) && voice_offered;
    let selective = c.settings().listen == Listen::Selected && c.group().is_none();
    let mut muted: Vec<RiderId> = if in_voice {
        c.participants()
            .filter(|p| p.muted_by_me || (selective && !p.selected && !p.blocked_by_me))
            .filter_map(|p| p.rider)
            .collect()
    } else {
        Vec::new()
    };
    muted.sort_unstable();
    muted.dedup();
    (speaker_off || voice_off, muted)
}

/// The status signs `viewer` sees on rider `of` (spec §2.6, §7.1.7 Q11):
/// the X mouth, the phone and the VR headset as `theirs` says; the
/// headphones only when `of` does not hear the viewer (hears nobody, or
/// has the viewer in `not_hearing`) while the viewer is in the call – and
/// never on the own rider.
pub fn signs_seen(
    viewer: Option<RiderId>,
    mine: &v1::RiderStatus,
    of: RiderId,
    theirs: &v1::RiderStatus,
) -> Signs {
    let me = viewer.filter(|v| *v != of);
    let cannot_hear_you = me.is_some_and(|v| {
        mine.in_voice && (theirs.hears_nobody || theirs.not_hearing.contains(&v.0))
    });
    Signs {
        voice_muted: theirs.voice_muted,
        in_menu: theirs.in_menu,
        in_vr: theirs.in_vr,
        cannot_hear_you,
    }
}

/// The wire status from the own signs (spec §2.6): the voice part from
/// [`voice_signs`], the menu and VR from the app.
pub fn status_message(voice: &VoiceSigns, in_menu: bool, in_vr: bool) -> v1::RiderStatus {
    v1::RiderStatus {
        voice_muted: voice.mic_muted,
        in_menu,
        in_vr,
        in_voice: voice.in_voice,
        hears_nobody: voice.hears_nobody,
        not_hearing: voice.not_hearing.iter().map(|r| r.0).collect(),
    }
}

/// What the menu's voice pages show, refreshed every frame.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoiceMenuInfo {
    pub status: String,
    pub inputs: Vec<String>,
    pub outputs: Vec<String>,
    /// The saved device is not present (the default is used).
    pub input_missing: bool,
    pub output_missing: bool,
    pub participants: Vec<MenuParticipant>,
    /// The echo test's short state ("speak now", "120 m away", …).
    pub echo: String,
    /// The riders blocked (spec §7.1.7 Q9), by name: present or not.
    pub blocked: Vec<BlockedRider>,
    /// The microphone is muted now (riders join muted, spec §7.1.7 Q7).
    pub mic_muted: bool,
    /// *Voice room* and *Who hears me* (spec §7.1.3, §7.1.7 Q8).
    pub listen: Listen,
    pub heard_by: HeardBy,
    /// The riders to choose from for *Selected riders* (spec §7.1.3): the
    /// riders in the session (bots and the Echo rider not), then the
    /// selected ones not here now.
    pub choices: Vec<ChoiceRider>,
}

/// A rider of the *Choose riders* list (spec §7.1.3).
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceRider {
    /// For `voice.select`: the rider id when here, else the remembered key.
    pub who: String,
    pub name: String,
    pub selected: bool,
    /// In this session (else greyed, under *Not here now*).
    pub here: bool,
    /// Distance on the road (m), when here and known.
    pub distance_m: Option<f64>,
    /// Talking now.
    pub speaking: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MenuParticipant {
    pub identity: String,
    pub name: String,
    pub muted_by_me: bool,
    pub mic_muted: bool,
    pub heard: Option<f32>,
    /// Blocked by the own rider (spec §7.1.7 Q9).
    pub blocked: bool,
    /// The own rider's volume for this rider (0 … 200 %).
    pub volume_pct: u16,
    /// One of the selected riders (spec §7.1.3).
    pub selected: bool,
    /// The game server lets the own rider hear this one (spec §7.1.7 Q6).
    pub may_hear: bool,
    /// How far away (m), if known.
    pub distance_m: Option<f64>,
}

/// A blocked rider of the Voice page's list (spec §7.1.7 Q9).
#[derive(Debug, Clone, PartialEq)]
pub struct BlockedRider {
    /// The remembered key (`acct:…` / `name:SERVER/NAME`).
    pub key: String,
    pub name: String,
}

/// Where the other riders are relative to the local one (spec §7): on a
/// route the gap along it (shortest way round on a loop); in free ride,
/// where everyone rides their own path, the straight-line distance.
pub fn peers(view: &RideView) -> Vec<Peer> {
    let Some(own) = view.local() else {
        return Vec::new();
    };
    let position = |r: &RiderView| -> Option<[f64; 3]> {
        if view.free_ride {
            view.paths
                .get(&r.id)
                .map(|p| p.sample(r.state.distance).position)
        } else {
            None
        }
    };
    let own_pos = position(own);
    view.riders
        .iter()
        .filter(|r| !r.is_local && r.id != ocw_render::riders::GHOST_ID)
        .filter_map(|r| {
            let gap_m = if view.free_ride {
                let (a, b) = (own_pos?, position(r)?);
                (a[0] - b[0]).hypot(a[2] - b[2])
            } else {
                let mut gap = r.state.distance - own.state.distance;
                let len = view.route.length();
                if view.route.is_loop() && len > 0.0 {
                    gap = (gap + len / 2.0).rem_euclid(len) - len / 2.0;
                }
                gap
            };
            Some(Peer {
                rider: r.id,
                gap_m,
                lateral_m: r.state.lateral_offset - own.state.lateral_offset,
            })
        })
        .collect()
}

/// The voice settings (`[voice]`, with the session layers).
pub fn settings_of(s: &Settings) -> (VoiceSettings, MuteState) {
    let on = |k: &str| s.get(k).ok().and_then(|v| v.as_bool()).unwrap_or(false);
    let int = |k: &str, d: i64| match s.get(k) {
        Ok(Value::Int(i)) => i,
        _ => d,
    };
    let device = |k: &str| {
        s.get(k)
            .ok()
            .map(|v| v.to_string())
            .filter(|v| !v.is_empty() && v != "default")
    };
    let text = |k: &str| s.get(k).ok().map(|v| v.to_string()).unwrap_or_default();
    (
        VoiceSettings {
            enabled: on("voice.enabled"),
            spatial: on("voice.spatial"),
            volume: int("voice.volume", 100) as f32 / 100.0,
            max_voices: int("voice.max_voices", ocw_voice::spatial::MAX_VOICES as i64).max(1)
                as usize,
            input: device("voice.input"),
            output: device("voice.output"),
            heard_by: HeardBy::parse(&text("voice.heard_by")).unwrap_or_default(),
            listen: Listen::parse(&text("voice.room")).unwrap_or_default(),
            notice: notice_pending(s),
        },
        MuteState {
            mic_muted: on("voice.mic_muted"),
            speaker_muted: on("voice.speaker_muted"),
        },
    )
}

/// The echo test's settings: on (the setting or `--echo-bot`) and the
/// delay (ms).
pub fn echo_of(s: &Settings, args_echo_bot: bool) -> (bool, u32) {
    let on = s
        .get("audio.echo_bot")
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let delay = match s.get("audio.echo_delay_ms") {
        Ok(Value::Int(i)) => i.clamp(0, i64::from(u32::MAX)) as u32,
        _ => ocw_voice::echo::DEFAULT_DELAY_MS,
    };
    (on || args_echo_bot, ocw_voice::echo::clamp_delay(delay))
}

/// The echo test's short state for the menu row.
pub fn echo_short(s: &EchoStatus) -> String {
    match s {
        EchoStatus::Off => String::new(),
        EchoStatus::Unavailable(_) => "unavailable".into(),
        EchoStatus::NoEchoRider => "no Echo rider".into(),
        EchoStatus::Far { distance_m } => format!("{distance_m:.0} m away"),
        EchoStatus::Near { .. } => "speak now".into(),
        EchoStatus::Remote { .. } => "via server".into(),
    }
}

/// The echo test's HUD line (spec §7 *Echo bot*): `None` while it is off.
pub fn echo_hud(s: &EchoStatus, levels: Levels, delay_ms: u32) -> Option<EchoHud> {
    let text = match s {
        EchoStatus::Off => return None,
        EchoStatus::Near { .. } => format!("ECHO TEST - speak now ({delay_ms} ms echo)"),
        EchoStatus::Remote { .. } if s.near() => {
            "ECHO TEST - speak now (via the voice server)".into()
        }
        other => format!("echo test: {}", other.text()),
    };
    // Levels in 1 % steps: the HUD changes only when the meter moves.
    let q = |l: f32| (l * 100.0).round() / 100.0;
    Some(EchoHud {
        near: s.near(),
        text,
        input: q(levels.input),
        echo: q(levels.echo),
    })
}

/// The HUD's room line (spec §7.1.4): "ROOM: everybody" or "ROOM: 3
/// selected (2 here)", and who hears the rider.
pub fn room_text(c: &VoiceController) -> String {
    let s = c.settings();
    let selected = ocw_voice::riders::selected_list(c.rider_prefs())
        .into_iter()
        .filter(|(k, _)| !c.rider_prefs().get(k).is_some_and(|p| p.blocked))
        .count();
    let here = c
        .participants()
        .filter(|p| p.selected && !p.blocked_by_me)
        .count();
    let room = match (c.group(), s.listen) {
        (Some(g), _) => format!("group {g}"),
        (None, Listen::Everybody) => "everybody".to_owned(),
        (None, Listen::Selected) => format!("{selected} selected ({here} here)"),
    };
    format!(
        "ROOM: {room} - heard by: {}",
        s.heard_by.label().to_lowercase()
    )
}

/// The HUD text of a status (with the number of others in the room).
pub fn status_text(status: &Status, others: usize) -> String {
    match status {
        Status::Connected => format!("connected ({others})"),
        // The reason is in the console; the HUD stays short.
        Status::Failed(_) => "failed, retrying".into(),
        s => s.text(),
    }
}

/// When the app asks the server for the Echo rider (or to take it away).
#[derive(Debug, Default, Clone, PartialEq)]
pub struct EchoAsk {
    /// The session the Echo rider was asked for.
    asked: Option<Option<(u64, bool)>>,
    /// The echo test was on in the last frame.
    was_on: bool,
}

impl EchoAsk {
    /// The request to send now: `Some(true)` put the Echo rider on the
    /// road, `Some(false)` take it away. `session`: the current session's
    /// key; `joined`: the server welcomed the rider; `present`: an Echo
    /// rider is on the road.
    pub fn next(
        &mut self,
        on: bool,
        session: Option<(u64, bool)>,
        joined: bool,
        present: bool,
    ) -> Option<bool> {
        let was_on = std::mem::replace(&mut self.was_on, on);
        if !joined {
            return None;
        }
        if on && !present && self.asked != Some(session) {
            self.asked = Some(session);
            return Some(true);
        }
        if !on && was_on {
            self.asked = None;
            return present.then_some(false);
        }
        None
    }
}

/// What a frame of voice gives the app.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoiceFrame {
    /// Encoded client messages for the game server (token and echo
    /// requests), in order.
    pub send: Vec<Vec<u8>>,
    /// Lines for the console (and the log).
    pub console: Vec<String>,
    pub menu: VoiceMenuInfo,
    pub hud: VoiceHud,
    /// Console commands for the app to run (the first call's notice turns
    /// its setting off).
    pub run: Vec<String>,
}

/// The per-frame voice logic of both apps (see the module docs).
#[derive(Debug, Default)]
pub struct VoiceDriver {
    /// The session seen last (its key, whether the rider was welcomed).
    seen: Option<(u64, bool)>,
    echo_ask: EchoAsk,
    /// The blocks told to the game server last, and in which session.
    blocks_sent: Option<((u64, bool), ServerBlocks)>,
    /// The audience told to the game server last, and in which session.
    audience_sent: Option<((u64, bool), v1::VoiceAudience)>,
    /// The first call's notice: shown until then (s); asked to be
    /// forgotten once.
    notice_until: Option<f64>,
    notice_done: bool,
    was_connected: bool,
}

/// The blocks as told to the game server: (account ids, names).
type ServerBlocks = (Vec<String>, Vec<String>);

impl VoiceDriver {
    /// The own blocks for the server (spec §7.1.7 Q9): at the join (if
    /// any) and on every change; `None` if nothing is to be sent.
    fn blocks_message(&mut self, c: &VoiceController) -> Option<Vec<u8>> {
        let session = self.seen.filter(|(_, joined)| *joined)?;
        let blocks = c.server_blocks();
        let empty = blocks.0.is_empty() && blocks.1.is_empty();
        match &self.blocks_sent {
            Some((s, b)) if *s == session && *b == blocks => return None,
            // A new session without blocks: nothing to say.
            Some((s, _)) if *s != session && empty => return None,
            None if empty => return None,
            _ => {}
        }
        self.blocks_sent = Some((session, blocks.clone()));
        Some(ClientCore::voice_blocks_message(blocks.0, blocks.1))
    }

    /// The own audience for the server (spec §7.1.7 Q8, §7.1.3): at the
    /// join (if not the default) and on every change; `None` if nothing
    /// is to be sent.
    fn audience_message(&mut self, c: &VoiceController) -> Option<Vec<u8>> {
        let session = self.seen.filter(|(_, joined)| *joined)?;
        let audience = c.server_audience();
        let default = audience == v1::VoiceAudience::default();
        match &self.audience_sent {
            Some((s, a)) if *s == session && *a == audience => return None,
            Some((s, _)) if *s != session && default => return None,
            None if default => return None,
            _ => {}
        }
        self.audience_sent = Some((session, audience.clone()));
        Some(ClientCore::voice_audience_message(audience))
    }
}

impl VoiceDriver {
    /// One frame at `now_s`. `session_key` changes with every new game
    /// session (start, ride switch); `grants` are the server's answers that
    /// arrived since the last frame.
    pub fn frame(
        &mut self,
        c: &mut VoiceController,
        client: Option<&ClientCore>,
        session_key: u64,
        grants: Vec<v1::VoiceGrant>,
        view: Option<&RideView>,
        now_s: f64,
    ) -> VoiceFrame {
        let mut out = VoiceFrame::default();
        if let Some(client) = client {
            // A new session or its welcome: the call of the old one ends;
            // whether the new server offers voice.
            let key = (session_key, client.rider_id().is_some());
            if self.seen != Some(key) {
                self.seen = Some(key);
                c.new_session(client.voice_offered());
            }
            c.set_human_riders(client.human_riders());
            c.set_echo_rider(client.echo_rider());
            // Volume and Block (spec §7.1.7 Q9): who is who, and whom the
            // server keeps apart from this rider.
            let (names, accounts) = client.rider_identities();
            c.set_rider_identities(client.server_scope(), names, accounts);
            c.set_apart(client.voice_apart().clone());
            if let Some(m) = self.blocks_message(c) {
                out.send.push(m);
            }
            // Who hears whom (spec §7.1.7 Q6, Q8): the own audience to the
            // server, its answer to the controller.
            if let Some(m) = self.audience_message(c) {
                out.send.push(m);
            }
            c.set_hearing(client.voice_hearing().cloned());
            // The echo test: ask the server for the Echo rider once per
            // session while it is on and none is on the road; take it away
            // when turned off.
            let joined = client.rider_id().is_some();
            let present = client.echo_rider().is_some();
            match self.echo_ask.next(c.echo_on(), self.seen, joined, present) {
                Some(true) if !client.echo_bot_allowed() => out.console.push(
                    "echo test: this server has no Echo rider (ocw-server --echo-bot)".into(),
                ),
                Some(on) => out.send.push(ClientCore::echo_request_message(on, false)),
                None => {}
            }
        }
        for g in grants {
            c.on_grant(&g, now_s);
        }
        let peers = view.map(peers).unwrap_or_default();
        if let Some(req) = c.update(now_s, &peers) {
            out.send.push(ClientCore::voice_request_message(req));
        }
        out.console.extend(c.take_messages());
        let status = c.status();
        let participants: Vec<MenuParticipant> = c
            .participants()
            .map(|p| MenuParticipant {
                identity: p.identity.clone(),
                name: if p.name.is_empty() {
                    p.identity.clone()
                } else {
                    p.name.clone()
                },
                muted_by_me: p.muted_by_me,
                mic_muted: p.mic_muted,
                heard: p.heard,
                blocked: p.blocked_by_me,
                volume_pct: p.volume_pct,
                selected: p.selected,
                may_hear: p.may_hear,
                distance_m: p.distance_m,
            })
            .collect();
        // The first call's notice (spec §7.1.2): once, 8 s, then the
        // setting is turned off.
        let connected = matches!(status, Status::Connected);
        if connected && !self.was_connected && c.settings().notice && !self.notice_done {
            self.notice_done = true;
            self.notice_until = Some(now_s + NOTICE_S);
            out.run.push("set voice.first_call_notice off".into());
        }
        self.was_connected = connected;
        let notice = self
            .notice_until
            .filter(|until| now_s < *until)
            .map(|_| first_call_notice(c.settings().heard_by));
        let text = status_text(&status, participants.len());
        let inputs = c.input_devices();
        let outputs = c.output_devices();
        let s = c.settings().clone();
        let missing = |saved: &Option<String>, present: &[String]| {
            matches!(
                ocw_voice::device::choose(saved.as_deref(), present),
                ocw_voice::device::Choice::Default { missing: Some(_) }
            )
        };
        out.menu = VoiceMenuInfo {
            status: text.clone(),
            input_missing: missing(&s.input, &inputs),
            output_missing: missing(&s.output, &outputs),
            inputs,
            outputs,
            participants,
            echo: echo_short(&c.echo_status()),
            blocked: ocw_voice::riders::blocked_list(c.rider_prefs())
                .into_iter()
                .map(|(key, name)| BlockedRider { key, name })
                .collect(),
            mic_muted: c.mute().mic_muted,
            listen: s.listen,
            heard_by: s.heard_by,
            choices: choices(c, client),
        };
        let own = client.and_then(|c| c.rider_id());
        // Selected riders who talk first (spec §7.1.4).
        let mut talking: Vec<&ocw_voice::Participant> = c
            .participants()
            .filter(|p| p.speaking && p.rider != own)
            .collect();
        talking.sort_by_key(|p| !p.selected);
        let names: Vec<String> = talking.iter().map(|p| p.name.clone()).collect();
        let mute = c.mute();
        out.hud = VoiceHud {
            visible: status.in_call(),
            mic_muted: mute.mic_muted,
            speaker_muted: mute.speaker_muted,
            status: text,
            self_speaking: c.self_speaking(),
            speaking: c
                .speaking_riders()
                .into_iter()
                .filter(|r| Some(*r) != own)
                .collect(),
            speaking_names: names,
            echo: echo_hud(&c.echo_status(), c.levels(), c.echo_delay_ms()),
            signs: voice_signs(c, client.is_some_and(ClientCore::voice_offered)),
            room: room_text(c),
            notice,
        };
        out
    }
}

/// The *Choose riders* list (spec §7.1.3): the riders in the voice room
/// by name (the humans of the session in the call – bots never join it –
/// without the Echo rider; with their distance and a speaking marker),
/// then the selected riders not here.
fn choices(c: &VoiceController, client: Option<&ClientCore>) -> Vec<ChoiceRider> {
    let mut out = Vec::new();
    let mut here_keys = std::collections::BTreeSet::new();
    let echo = client.and_then(ClientCore::echo_rider);
    for p in c.participants() {
        if p.rider.is_none() || p.rider == echo {
            continue;
        }
        let keys = c.keys_of(p);
        if let Some(a) = &keys.account {
            here_keys.insert(a.clone());
        }
        here_keys.insert(keys.name.clone());
        out.push(ChoiceRider {
            who: p.identity.clone(),
            name: c.name_of(p).to_owned(),
            selected: p.selected,
            here: true,
            distance_m: p.distance_m,
            speaking: p.speaking,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    for (key, name) in ocw_voice::riders::selected_list(c.rider_prefs()) {
        if !here_keys.contains(&key) {
            out.push(ChoiceRider {
                who: key,
                name,
                selected: true,
                here: false,
                distance_m: None,
                speaking: false,
            });
        }
    }
    out
}

// ------------------------------------------------------------- commands

/// Where an app keeps its settings: `settings.toml` on the desktop, the
/// page's `localStorage` in the browser. The commands change the registry
/// and save it.
pub trait SettingsStore {
    fn settings_mut(&mut self) -> &mut Settings;
    fn save(&mut self) -> Result<(), String>;
}

/// Sets `key` to `value` in the store and saves; the note for the reply
/// ("" or " (not saved: …)").
fn remember(store: Option<&mut dyn SettingsStore>, key: &str, value: &str) -> String {
    let Some(store) = store else {
        return String::new();
    };
    if let Err(e) = store.settings_mut().set(key, value) {
        return format!(" (not saved: {e})");
    }
    store
        .save()
        .err()
        .map(|e| format!(" (not saved: {e})"))
        .unwrap_or_default()
}

/// `voice.mute-mic` / `voice.mute-speaker [on|off|toggle]` (M / Shift+M,
/// gamepad Select / R3): applied to the call; the speaker is remembered in
/// the settings (`voice.speaker_muted`), the microphone is not – every
/// shared ride is joined muted (spec §7.1.7 Q7).
pub fn mute_command(
    c: &mut VoiceController,
    store: Option<&mut dyn SettingsStore>,
    which: Toggle,
    arg: Option<&str>,
) -> Result<String, String> {
    let s = Switch::parse(arg)?;
    let note = match which {
        // Not remembered (spec §7.1.7 Q7).
        Toggle::Mic => String::new(),
        Toggle::Speaker => {
            let key = "voice.speaker_muted";
            // The saved value is the truth (a `set --temp` may differ from
            // the controller's until applied).
            let mut store = store;
            let saved_now = store
                .as_deref_mut()
                .and_then(|st| st.settings_mut().get(key).ok())
                .and_then(|v| v.as_bool())
                .unwrap_or(c.mute().speaker_muted);
            let new = s.apply(saved_now);
            remember(store, key, if new { "on" } else { "off" })
        }
    };
    match which {
        Toggle::Mic => c.set_mic(s),
        Toggle::Speaker => c.set_speaker(s),
    };
    let m = c.mute();
    Ok(match which {
        Toggle::Mic if m.mic_muted => format!("microphone muted{note}"),
        Toggle::Mic => format!("microphone on{note}"),
        Toggle::Speaker if m.speaker_muted => format!("speaker muted: no voices{note}"),
        Toggle::Speaker => format!("speaker on{note}"),
    })
}

/// Applies the `[voice]` and echo settings to the controller (the
/// settings' apply command `voice.apply`, at the start and on every
/// change). `echo_bot`: the app was started with the echo test on.
pub fn apply_settings(c: &mut VoiceController, settings: &Settings, echo_bot: bool) {
    let (s, mute) = settings_of(settings);
    let (echo, delay) = echo_of(settings, echo_bot);
    c.set_settings(s);
    c.set_echo(echo);
    c.set_echo_delay(delay);
    let on = |b: bool| if b { Switch::On } else { Switch::Off };
    // Not the microphone: every shared ride is joined muted (spec §7.1.7
    // Q7) and a change of another voice setting must not unmute it.
    c.set_speaker(on(mute.speaker_muted));
    c.set_rider_prefs(settings.voice_riders());
}

/// Whether the first-call notice is still to be shown
/// (`voice.first_call_notice`).
pub fn notice_pending(settings: &Settings) -> bool {
    settings
        .get("voice.first_call_notice")
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// The first call's notice (spec §7.1.2, adapted to Q7 / Q8: joined
/// muted, *Who hears me*), 8 s on the HUD.
pub fn first_call_notice(heard_by: HeardBy) -> String {
    format!(
        "Voice: you join muted - M unmutes. Who hears you: {} (Settings > Voice > Voice room).",
        heard_by.label().to_lowercase()
    )
}

/// How long the first call's notice stays (s).
pub const NOTICE_S: f64 = 8.0;

/// What `voice.echo here` sends: the Echo rider 20 m ahead.
pub fn echo_here_message() -> Vec<u8> {
    ClientCore::echo_request_message(true, true)
}

/// `voice.echo [on|off|here|delay MS|status]` (spec §7 *Echo bot*). The
/// reply, and for `here` the message for the game server (`connected`:
/// the app has one to send it to).
pub fn echo_command(
    c: &mut VoiceController,
    store: Option<&mut dyn SettingsStore>,
    action: Option<&str>,
    ms: Option<u32>,
    connected: bool,
) -> Result<(String, Option<Vec<u8>>), String> {
    match action.unwrap_or("status") {
        a @ ("on" | "off") => {
            let on = a == "on";
            let note = remember(store, "audio.echo_bot", if on { "on" } else { "off" });
            c.set_echo(on);
            Ok((
                if on {
                    format!(
                        "echo test on ({} ms): ride to the Echo rider and speak{note}",
                        c.echo_delay_ms()
                    )
                } else {
                    format!("echo test off{note}")
                },
                None,
            ))
        }
        "here" => {
            let note = remember(store, "audio.echo_bot", "on");
            c.set_echo(true);
            if !connected {
                return Err("not connected to a game server".into());
            }
            Ok((
                format!("echo test on: the Echo rider comes 20 m ahead{note}"),
                Some(echo_here_message()),
            ))
        }
        "delay" => {
            let ms = ms.ok_or("voice.echo delay MS (20 to 3000)")?;
            let set = c.set_echo_delay(ms);
            let note = remember(store, "audio.echo_delay_ms", &set.to_string());
            Ok((format!("echo delay {set} ms{note}"), None))
        }
        _ => {
            let l = c.levels();
            Ok((
                format!(
                    "echo test: {} (delay {} ms; microphone {:.0} %, echo {:.0} %)",
                    c.echo_status().text(),
                    c.echo_delay_ms(),
                    100.0 * ocw_render::voice::meter_fraction(l.input),
                    100.0 * ocw_render::voice::meter_fraction(l.echo),
                ),
                None,
            ))
        }
    }
}

/// `voice.status`: the call, the switches, the room and its participants;
/// `note`: why voice cannot work in this build, if so.
pub fn status_command(c: &VoiceController, note: Option<&str>) -> String {
    let m = c.mute();
    let mut out = vec![
        format!("voice: {}", c.status().text()),
        format!(
            "microphone {}, speaker {}",
            if m.mic_muted { "muted" } else { "on" },
            if m.speaker_muted { "muted" } else { "on" }
        ),
    ];
    if let Some(r) = c.room() {
        out.push(format!("room: {r}"));
    }
    if let Some(g) = c.group() {
        out.push(format!("group: {g}"));
    }
    let s = c.settings();
    out.push(format!(
        "voice room: {}, who hears me: {}",
        s.listen.label().to_lowercase(),
        s.heard_by.label().to_lowercase()
    ));
    if let Some(h) = c.hearing() {
        let list = |set: &std::collections::BTreeSet<RiderId>| {
            set.iter()
                .map(|r| r.0.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        out.push(format!(
            "server: may hear [{}], heard by [{}]",
            list(&h.may_hear),
            list(&h.heard_by)
        ));
    }
    for p in c.participants() {
        out.push(format!(
            "  {} ({}){}{}{}{}{}{}{}",
            p.name,
            p.identity,
            if p.speaking { ", talking" } else { "" },
            if p.muted_by_me { ", muted by you" } else { "" },
            if p.blocked_by_me { ", blocked" } else { "" },
            if p.selected { ", selected" } else { "" },
            if p.may_hear || c.group().is_some() {
                ""
            } else {
                ", not heard here"
            },
            if p.volume_pct != ocw_voice::riders::DEFAULT_VOLUME_PCT {
                format!(", volume {} %", p.volume_pct)
            } else {
                String::new()
            },
            p.heard
                .map(|g| format!(", heard {:.0} %", g * 100.0))
                .unwrap_or_default()
        ));
    }
    let blocked = ocw_voice::riders::blocked_list(c.rider_prefs());
    if !blocked.is_empty() {
        let names: Vec<&str> = blocked.iter().map(|(_, n)| n.as_str()).collect();
        out.push(format!("blocked: {}", names.join(", ")));
    }
    let selected = ocw_voice::riders::selected_list(c.rider_prefs());
    if !selected.is_empty() {
        let names: Vec<&str> = selected.iter().map(|(_, n)| n.as_str()).collect();
        out.push(format!("selected: {}", names.join(", ")));
    }
    if let Some(n) = note {
        out.push(format!("({n})"));
    }
    out.join("\n")
}

/// `voice.mute WHO [on|off|toggle]`: one participant (rider id, identity
/// or name), for the local rider only.
pub fn mute_who_command(
    c: &mut VoiceController,
    who: &str,
    arg: Option<&str>,
) -> Result<String, String> {
    let s = Switch::parse(arg)?;
    let rider = who.parse::<u32>().ok().map(ocw_core::RiderId);
    let (identity, name) = c
        .participants()
        .find(|p| p.identity == who || p.name == who || (rider.is_some() && p.rider == rider))
        .map(|p| (p.identity.clone(), p.name.clone()))
        .ok_or_else(|| format!("nobody called {who:?} in the voice room"))?;
    let muted = c.mute_participant(&identity, s);
    Ok(format!(
        "{name} {}",
        if muted { "muted" } else { "unmuted" }
    ))
}

/// Whom `voice.volume` / `voice.block` mean: a participant (rider id,
/// identity or name) or a remembered rider (its key or name). The key the
/// choice is kept under (the one already used, else the account's, else
/// the name's on this server) and the rider's name.
fn whom(c: &VoiceController, who: &str) -> Result<(String, String), String> {
    let rider = who.parse::<u32>().ok().map(ocw_core::RiderId);
    if let Some(p) = c
        .participants()
        .find(|p| p.identity == who || p.name == who || (rider.is_some() && p.rider == rider))
    {
        let keys = c.keys_of(p);
        let name = c.name_of(p).to_owned();
        let key = keys
            .lookup(c.rider_prefs())
            .map_or_else(|| keys.primary().to_owned(), |(k, _)| k.to_owned());
        return Ok((key, name));
    }
    c.rider_prefs()
        .iter()
        .find(|(k, p)| k.as_str() == who || p.name == who)
        .map(|(k, p)| (k.clone(), p.name.clone()))
        .ok_or_else(|| format!("nobody called {who:?} in the voice room or remembered"))
}

/// Remembers a rider's choice (the store saved) and hands it to the
/// controller; the note for the reply.
fn remember_rider(
    c: &mut VoiceController,
    store: Option<&mut dyn SettingsStore>,
    key: &str,
    pref: ocw_voice::riders::RiderPref,
) -> String {
    let mut prefs = c.rider_prefs().clone();
    if pref.is_default() {
        prefs.remove(key);
    } else {
        prefs.insert(key.to_owned(), pref.clone());
    }
    c.set_rider_prefs(prefs);
    let Some(store) = store else {
        return String::new();
    };
    store.settings_mut().set_voice_rider(key, &pref);
    store
        .save()
        .err()
        .map(|e| format!(" (not saved: {e})"))
        .unwrap_or_default()
}

/// `voice.volume WHO PERCENT` (spec §7.1.7 Q9): one rider's volume for
/// the own rider, 0 … 200 % in 10 % steps, on top of the distance and the
/// voice volume; remembered (by account, else by name on this server).
pub fn volume_command(
    c: &mut VoiceController,
    store: Option<&mut dyn SettingsStore>,
    who: &str,
    percent: i64,
) -> Result<String, String> {
    use ocw_voice::riders::{RiderPref, clamp_volume};
    let (key, name) = whom(c, who)?;
    let mut pref = c
        .rider_prefs()
        .get(&key)
        .cloned()
        .unwrap_or_else(|| RiderPref::new(name.clone()));
    pref.name = name.clone();
    pref.volume_pct = clamp_volume(percent);
    let pct = pref.volume_pct;
    let note = remember_rider(c, store, &key, pref);
    Ok(format!("{name}: volume {pct} %{note}"))
}

/// `voice.block WHO [on|off|toggle]` (spec §7.1.7 Q9): neither rider
/// hears the other – the game server keeps both apart, the voice server
/// enforces it; remembered across sessions; the blocked rider is not told.
pub fn block_command(
    c: &mut VoiceController,
    store: Option<&mut dyn SettingsStore>,
    who: &str,
    arg: Option<&str>,
) -> Result<String, String> {
    use ocw_voice::riders::RiderPref;
    let s = Switch::parse(arg)?;
    let (key, name) = whom(c, who)?;
    let mut pref = c
        .rider_prefs()
        .get(&key)
        .cloned()
        .unwrap_or_else(|| RiderPref::new(name.clone()));
    pref.name = name.clone();
    pref.blocked = s.apply(pref.blocked);
    let blocked = pref.blocked;
    let note = remember_rider(c, store, &key, pref);
    Ok(if blocked {
        format!("{name} blocked: you do not hear each other{note}")
    } else {
        format!("{name} unblocked{note}")
    })
}

/// `voice.select WHO [on|off|toggle]` (spec §7.1.3, §7.1.7 Q8): one of
/// the *Selected riders* – heard with *Voice room: Selected riders*,
/// hearing the own rider with *Who hears me: Selected riders*;
/// remembered like a block (by account, else by name on this server). A
/// block wins: a blocked rider stays unheard and does not hear the rider.
pub fn select_command(
    c: &mut VoiceController,
    store: Option<&mut dyn SettingsStore>,
    who: &str,
    arg: Option<&str>,
) -> Result<String, String> {
    use ocw_voice::riders::RiderPref;
    let s = Switch::parse(arg)?;
    let (key, name) = whom(c, who)?;
    let mut pref = c
        .rider_prefs()
        .get(&key)
        .cloned()
        .unwrap_or_else(|| RiderPref::new(name.clone()));
    pref.name = name.clone();
    pref.selected = s.apply(pref.selected);
    let (selected, blocked) = (pref.selected, pref.blocked);
    let note = remember_rider(c, store, &key, pref);
    let set = c.settings();
    Ok(match (selected, blocked) {
        (true, true) => format!("{name} selected, but blocked: the block wins{note}"),
        (true, false) => format!(
            "{name} selected (voice room: {}, who hears me: {}){note}",
            set.listen.label().to_lowercase(),
            set.heard_by.label().to_lowercase()
        ),
        (false, _) => format!("{name} no longer selected{note}"),
    })
}

/// `voice.group [NAME | --leave]`.
pub fn group_command(c: &mut VoiceController, name: Option<String>) -> Result<String, String> {
    c.set_group(name.clone())?;
    Ok(match name {
        Some(n) => format!("voice group {n}: joining"),
        None => "voice: the whole session".into(),
    })
}

/// `voice.devices`: the microphones and speakers with the saved choice.
pub fn devices_command(c: &mut VoiceController) -> String {
    let s = c.settings().clone();
    let list = |title: &str, present: Vec<String>, saved: &Option<String>| {
        let mut out = vec![format!(
            "{title}: {}",
            saved.as_deref().unwrap_or("default")
        )];
        out.extend(present.into_iter().map(|d| format!("  {d}")));
        out
    };
    let mut out = list("microphone", c.input_devices(), &s.input);
    out.extend(list("speaker", c.output_devices(), &s.output));
    out.join("\n")
}

/// The voice hotkeys' console commands for this frame's key and button
/// presses (`M`, `Shift+M`, gamepad Select / R3; spec §2.4).
pub fn hotkey_commands(inputs: impl IntoIterator<Item = Input>) -> Vec<&'static str> {
    inputs
        .into_iter()
        .map(|i| ocw_voice::hotkeys::toggle(i).command())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::RiderId;
    use ocw_voice::backend::{BackendCommand, BackendEvent, MockBackend};
    use ocw_world::Route;

    fn rider(id: u32, d: f64, lat: f64, local: bool) -> RiderView {
        RiderView {
            id: RiderId(id),
            name: format!("R{id}"),
            state: ocw_sim::RiderState {
                distance: d,
                lateral_offset: lat,
                ..Default::default()
            },
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: local,
            bike: Default::default(),
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
            cue: Default::default(),
            signs: Default::default(),
        }
    }

    #[test]
    fn peers_by_gap_along_the_route() {
        let route = Route::test_loop();
        let len = route.length();
        let mut view = RideView::new(route);
        view.riders = vec![
            rider(1, 100.0, 0.5, true),
            rider(2, 130.0, -1.0, false),
            rider(3, len - 50.0, 0.5, false),
            rider(ocw_render::riders::GHOST_ID.0, 100.0, 0.0, false),
        ];
        let p = peers(&view);
        assert_eq!(p.len(), 2, "no ghost, not the own rider");
        assert_eq!(p[0].rider, RiderId(2));
        assert!((p[0].gap_m - 30.0).abs() < 1e-9);
        assert!((p[0].lateral_m + 1.5).abs() < 1e-9);
        // On a loop the shortest way round: 150 m behind, not len-150 ahead.
        assert!(view.route.is_loop());
        assert!((p[1].gap_m + 150.0).abs() < 1e-6, "{}", p[1].gap_m);
        view.riders.retain(|r| !r.is_local);
        assert!(peers(&view).is_empty(), "no local rider yet");
    }

    #[test]
    fn settings_map_onto_the_controller() {
        let (s, _) = Settings::parse(
            "[voice]\nvolume = 80\nmax_voices = 4\ninput = \"Headset\"\nspeaker_muted = true\nspatial = false\n",
        )
        .unwrap();
        let (v, m) = settings_of(&s);
        assert_eq!(v.volume, 0.8);
        assert_eq!(v.max_voices, 4);
        assert_eq!(v.input.as_deref(), Some("Headset"));
        assert_eq!(v.output, None, "default");
        assert!(!v.spatial && v.enabled);
        assert_eq!(
            m,
            MuteState {
                mic_muted: false,
                speaker_muted: true
            }
        );
        // First start: both on.
        let (_, m) = settings_of(&Settings::default());
        assert_eq!(m, MuteState::default());
    }

    /// The echo test asks for the Echo rider once per session, again
    /// after a ride switch, and takes it away when turned off.
    #[test]
    fn echo_rider_is_asked_for_once_per_session() {
        let mut a = EchoAsk::default();
        let s1 = Some((1, true));
        assert_eq!(a.next(true, s1, false, false), None, "not joined yet");
        assert_eq!(a.next(true, s1, true, false), Some(true));
        assert_eq!(a.next(true, s1, true, false), None, "once");
        assert_eq!(a.next(true, s1, true, true), None);
        // Refused or gone in the same session: not asked again.
        assert_eq!(a.next(true, s1, true, false), None);
        // A new session (ride switch): asked again.
        let s2 = Some((2, true));
        assert_eq!(a.next(true, s2, true, false), Some(true));
        assert_eq!(a.next(false, s2, true, true), Some(false), "taken away");
        assert_eq!(a.next(false, s2, true, true), None);
        assert_eq!(a.next(true, s2, true, false), Some(true), "on again");
        // Off while none is on the road: nothing to say.
        assert_eq!(a.next(false, s2, true, false), None);
    }

    #[test]
    fn echo_hud_and_settings() {
        let l = Levels {
            input: 0.123,
            echo: 0.05,
        };
        assert_eq!(echo_hud(&EchoStatus::Off, l, 300), None);
        let near = echo_hud(
            &EchoStatus::Near {
                distance_m: 5.0,
                recording: 1.0,
            },
            l,
            300,
        )
        .unwrap();
        assert!(near.near && near.text.contains("speak now"), "{near:?}");
        assert_eq!((near.input, near.echo), (0.12, 0.05));
        let far = echo_hud(&EchoStatus::Far { distance_m: 120.0 }, l, 300).unwrap();
        assert!(!far.near && far.text.contains("120 m"), "{far:?}");
        assert_eq!(
            echo_short(&EchoStatus::Far { distance_m: 120.0 }),
            "120 m away"
        );
        let (s, _) = Settings::parse("[audio]\necho_bot = true\necho_delay_ms = 500\n").unwrap();
        assert_eq!(echo_of(&s, false), (true, 500));
        assert_eq!(echo_of(&Settings::default(), false), (false, 300));
        assert_eq!(
            echo_of(&Settings::default(), true),
            (true, 300),
            "--echo-bot"
        );
    }

    /// The driver without a client (before the ride): the indicator is
    /// hidden, the switches reach the HUD, nothing is sent.
    #[test]
    fn driver_shows_the_switches_and_hides_the_indicator_out_of_a_call() {
        let mut c = VoiceController::new(
            Box::new(MockBackend::default()),
            VoiceSettings::default(),
            MuteState {
                mic_muted: true,
                speaker_muted: false,
            },
        );
        let mut d = VoiceDriver::default();
        let f = d.frame(&mut c, None, 0, Vec::new(), None, 0.0);
        assert!(f.send.is_empty());
        assert!(!f.hud.visible && f.hud.mic_muted && !f.hud.speaker_muted);
        assert_eq!(f.menu.status, "no voice on this server");
    }

    /// A grant while requesting connects the backend; its events reach
    /// the HUD (connected, the other participant), a drop retries.
    #[test]
    fn driver_hands_grants_to_the_controller() {
        let backend = std::sync::Arc::new(std::sync::Mutex::new(MockBackend::default()));
        struct Shared(std::sync::Arc<std::sync::Mutex<MockBackend>>);
        impl ocw_voice::VoiceBackend for Shared {
            fn available(&self) -> Result<(), String> {
                Ok(())
            }
            fn input_devices(&mut self) -> Vec<String> {
                Vec::new()
            }
            fn output_devices(&mut self) -> Vec<String> {
                Vec::new()
            }
            fn send(&mut self, cmd: BackendCommand) {
                self.0.lock().unwrap().sent.push(cmd);
            }
            fn poll(&mut self) -> Vec<BackendEvent> {
                std::mem::take(&mut self.0.lock().unwrap().events)
            }
        }
        let mut c = VoiceController::new(
            Box::new(Shared(backend.clone())),
            VoiceSettings::default(),
            MuteState::default(),
        );
        c.new_session(true);
        c.set_human_riders(2);
        let mut d = VoiceDriver::default();
        let f = d.frame(&mut c, None, 1, Vec::new(), None, 0.0);
        assert_eq!(f.send.len(), 1, "the token request");
        let grant = v1::VoiceGrant {
            url: "wss://voice.example".into(),
            token: "t".into(),
            room: "ocw-1".into(),
            identity: "rider-1".into(),
            ..Default::default()
        };
        d.frame(&mut c, None, 1, vec![grant], None, 0.1);
        assert!(
            backend
                .lock()
                .unwrap()
                .sent
                .contains(&BackendCommand::Connect {
                    url: "wss://voice.example".into(),
                    token: "t".into()
                })
        );
        backend.lock().unwrap().events.extend([
            BackendEvent::Connected {
                room: "ocw-1".into(),
            },
            BackendEvent::ParticipantJoined {
                identity: "rider-2".into(),
                name: "Ada".into(),
            },
        ]);
        let f = d.frame(&mut c, None, 1, Vec::new(), None, 0.2);
        assert!(f.hud.visible);
        assert_eq!(f.hud.status, "connected (1)");
        assert_eq!(f.menu.participants[0].name, "Ada");
        assert!(f.console.iter().any(|l| l.contains("connected to ocw-1")));
        backend
            .lock()
            .unwrap()
            .events
            .push(BackendEvent::Disconnected {
                reason: "server left".into(),
            });
        let f = d.frame(&mut c, None, 1, Vec::new(), None, 0.3);
        assert_eq!(f.hud.status, "failed, retrying");
        assert!(f.console.iter().any(|l| l.contains("server left")));
    }

    /// One rider of [`three_riders_block_both_ways`]: the client, its
    /// voice controller on a recording backend, the driver.
    struct Side {
        conn: ocw_server::ConnectionId,
        client: ClientCore,
        c: VoiceController,
        backend: std::sync::Arc<std::sync::Mutex<MockBackend>>,
        d: VoiceDriver,
        grants: Vec<v1::VoiceGrant>,
    }

    struct Recording(std::sync::Arc<std::sync::Mutex<MockBackend>>);
    impl ocw_voice::VoiceBackend for Recording {
        fn available(&self) -> Result<(), String> {
            Ok(())
        }
        fn input_devices(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn output_devices(&mut self) -> Vec<String> {
            Vec::new()
        }
        fn send(&mut self, cmd: BackendCommand) {
            self.0.lock().unwrap().sent.push(cmd);
        }
        fn poll(&mut self) -> Vec<BackendEvent> {
            std::mem::take(&mut self.0.lock().unwrap().events)
        }
    }

    impl Side {
        /// Whom the backend is subscribed to now, from its commands.
        fn subscribed(&self) -> std::collections::BTreeSet<String> {
            let mut on = std::collections::BTreeSet::new();
            for cmd in &self.backend.lock().unwrap().sent {
                if let BackendCommand::Subscribe { identity, on: o } = cmd {
                    if *o {
                        on.insert(identity.clone());
                    } else {
                        on.remove(identity);
                    }
                }
            }
            on
        }
        /// Who may subscribe to the own microphone, as last told.
        fn subscribers(&self) -> Option<Vec<String>> {
            self.backend
                .lock()
                .unwrap()
                .sent
                .iter()
                .rev()
                .find_map(|c| match c {
                    BackendCommand::SetSubscribers(a) => Some(a.clone()),
                    _ => None,
                })
                .flatten()
        }
    }

    const TRIO: [&str; 3] = ["Ada Rider", "Bea Climber", "Cleo Sprinter"];

    /// Three riders on one in-memory game server with voice (dev commands
    /// on: the tests place riders), each with the shared driver and
    /// controller, *Who hears me: All nearby*.
    struct Trio {
        server: ocw_server::GameServer,
        sides: Vec<Side>,
        ids: Vec<RiderId>,
        t: f64,
    }

    impl Trio {
        fn new() -> Self {
            let mut server = ocw_server::GameServer::new(
                ocw_world::Route::test_loop(),
                ocw_server::ServerConfig {
                    voice: Some(ocw_server::voice::VoiceConfig::new(
                        "ws://127.0.0.1:7880",
                        "k",
                        "secret-secret-secret-secret-0000",
                    )),
                    allow_dev_commands: true,
                    ..Default::default()
                },
            );
            let mut sides: Vec<Side> = TRIO
                .iter()
                .map(|n| {
                    let backend =
                        std::sync::Arc::new(std::sync::Mutex::new(MockBackend::default()));
                    Side {
                        conn: server.connect(),
                        client: ClientCore::new(
                            ocw_world::Route::test_loop(),
                            *n,
                            ocw_core::RiderProfile::default(),
                        ),
                        c: VoiceController::new(
                            Box::new(Recording(backend.clone())),
                            VoiceSettings {
                                heard_by: HeardBy::Nearby,
                                ..Default::default()
                            },
                            MuteState::default(),
                        ),
                        backend,
                        d: VoiceDriver::default(),
                        grants: Vec::new(),
                    }
                })
                .collect();
            for s in &mut sides {
                let hello = s.client.hello();
                for r in server.handle(s.conn, &hello) {
                    Self::deliver(s, &r, 0.0);
                }
            }
            let ids = sides.iter().map(|s| s.client.rider_id().unwrap()).collect();
            Self {
                server,
                sides,
                ids,
                t: 0.0,
            }
        }

        fn deliver(side: &mut Side, bytes: &[u8], t: f64) {
            if let Some(crate::client::Notice::Voice(g)) = side.client.on_server_message(bytes, t) {
                side.grants.push(g);
            }
        }

        fn identity(&self, i: usize) -> String {
            ocw_protocol::voice_identity(self.ids[i])
        }

        /// Puts rider `i` at `km` on the loop (a dev teleport).
        fn place(&mut self, i: usize, km: f64) {
            let msg = ocw_protocol::encode(&ocw_protocol::client_message(
                v1::client_message::Msg::Dev(v1::DevCommand {
                    cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(km * 1000.0)),
                }),
            ));
            let conn = self.sides[i].conn;
            self.server.handle(conn, &msg);
        }

        /// One tick of the server and a frame of every client.
        fn step(&mut self) {
            self.t += 0.02;
            let t = self.t;
            let out = self.server.tick();
            let directed = self.server.take_directed();
            let ids = self.ids.clone();
            for s in self.sides.iter_mut() {
                for m in &out {
                    Self::deliver(s, m, t);
                }
                let conn = s.conn;
                for (_, m) in directed.iter().filter(|(c, _)| *c == conn) {
                    Self::deliver(s, m, t);
                }
                let grants = std::mem::take(&mut s.grants);
                let f = s.d.frame(&mut s.c, Some(&s.client), 1, grants, None, t);
                for bytes in f.send {
                    let replies = self.server.handle(s.conn, &bytes);
                    for r in replies {
                        Self::deliver(s, &r, t);
                    }
                }
                // The voice server: once asked to connect, the call is up
                // with the other two in it.
                let connect = s
                    .backend
                    .lock()
                    .unwrap()
                    .sent
                    .iter()
                    .any(|c| matches!(c, BackendCommand::Connect { .. }));
                if connect && !matches!(s.c.status(), ocw_voice::Status::Connected) {
                    let me = s.client.rider_id().unwrap();
                    let mut b = s.backend.lock().unwrap();
                    b.events.push(BackendEvent::Connected { room: "r".into() });
                    for (i, id) in ids.iter().enumerate() {
                        if *id != me {
                            b.events.push(BackendEvent::ParticipantJoined {
                                identity: ocw_protocol::voice_identity(*id),
                                name: TRIO[i].into(),
                            });
                        }
                    }
                }
            }
        }

        fn run(&mut self, steps: usize) {
            for _ in 0..steps {
                self.step();
            }
        }

        fn set(&self, idx: &[usize]) -> std::collections::BTreeSet<String> {
            idx.iter().map(|i| self.identity(*i)).collect()
        }

        fn allowed(&self, i: usize) -> Option<std::collections::BTreeSet<String>> {
            self.sides[i].subscribers().map(|v| v.into_iter().collect())
        }
    }

    /// Block (spec §7.1.7 Q9) end to end without a network: three riders
    /// on one in-memory game server, each with the shared driver and
    /// controller. A blocks B from the console: A's client tells the
    /// server, the server tells A and B, and neither subscribes to the
    /// other nor lets the other subscribe; C hears both and may be heard
    /// by both. Unblocking restores everything.
    #[test]
    fn three_riders_block_both_ways() {
        let mut w = Trio::new();
        w.run(30);
        for (i, s) in w.sides.iter().enumerate() {
            assert_eq!(s.c.status(), ocw_voice::Status::Connected);
            assert_eq!(s.subscribed().len(), 2, "everybody hears everybody");
            let others: Vec<usize> = (0..3).filter(|j| *j != i).collect();
            assert_eq!(w.allowed(i), Some(w.set(&others)), "both others may hear");
        }
        // A blocks B (by name, from the console).
        let reply = block_command(&mut w.sides[0].c, None, "Bea Climber", Some("on")).unwrap();
        assert!(reply.contains("Bea Climber blocked"), "{reply}");
        w.run(10);
        let (a, b, c) = (&w.sides[0], &w.sides[1], &w.sides[2]);
        assert_eq!(a.subscribed(), w.set(&[2]), "A does not hear B");
        assert_eq!(b.subscribed(), w.set(&[2]), "B does not hear A");
        assert_eq!(c.subscribed(), w.set(&[0, 1]), "C hears both");
        assert_eq!(w.allowed(0), Some(w.set(&[2])), "B may not hear A");
        assert_eq!(w.allowed(1), Some(w.set(&[2])), "A may not hear B");
        assert_eq!(w.allowed(2), Some(w.set(&[0, 1])), "both may hear C");
        // B is not told who blocked whom: only that A is apart.
        assert_eq!(b.client.voice_apart(), &[w.ids[0]].into());
        assert!(b.c.rider_prefs().is_empty(), "B blocked nobody");
        let bea_sees_ada =
            b.c.participants()
                .find(|p| p.rider == Some(w.ids[0]))
                .unwrap();
        assert!(bea_sees_ada.apart && !bea_sees_ada.blocked_by_me);
        // The menu of A lists Bea as blocked.
        let t = w.t;
        let a = &mut w.sides[0];
        let f = a.d.frame(&mut a.c, Some(&a.client), 1, Vec::new(), None, t);
        assert_eq!(f.menu.blocked.len(), 1);
        assert_eq!(f.menu.blocked[0].name, "Bea Climber");
        // Unblocked: all as before.
        block_command(
            &mut w.sides[0].c,
            None,
            "name:local/Bea Climber",
            Some("off"),
        )
        .unwrap();
        w.run(10);
        for i in 0..3 {
            assert_eq!(w.sides[i].subscribed().len(), 2);
            let others: Vec<usize> = (0..3).filter(|j| *j != i).collect();
            assert_eq!(w.allowed(i), Some(w.set(&others)));
            assert!(w.sides[i].client.voice_apart().is_empty());
        }
    }

    /// Voice by range and audience end to end without a network (spec
    /// §7.1.7 Q6, Q8, Q10, §7.1.3): the game server decides who receives
    /// whose voice from the riders' places; each client subscribes only to
    /// whom it may hear and allows its microphone only to whom the server
    /// lists. A far rider is neither sent nor heard, coming back near it
    /// is; *Who hears me: No one* (the default) and *Selected riders*,
    /// *Voice room: Selected riders* and a block over a selection hold.
    #[test]
    fn three_riders_hear_by_range_and_audience() {
        let mut w = Trio::new();
        w.run(30);
        for s in &w.sides {
            assert_eq!(s.subscribed().len(), 2, "all at the start: all heard");
        }
        // Cleo rides 1.5 km away (half the 3.9 km loop): out of range.
        w.place(2, 1.5);
        w.run(10);
        assert_eq!(w.sides[0].subscribed(), w.set(&[1]));
        assert_eq!(w.sides[1].subscribed(), w.set(&[0]));
        assert!(w.sides[2].subscribed().is_empty(), "Cleo hears nobody");
        assert_eq!(
            w.allowed(0),
            Some(w.set(&[1])),
            "Ada's voice not sent to Cleo"
        );
        assert_eq!(
            w.allowed(2),
            Some(w.set(&[])),
            "Cleo's voice sent to nobody"
        );
        let cleo = w.sides[0]
            .c
            .participants()
            .find(|p| p.rider == Some(w.ids[2]))
            .unwrap();
        assert!(!cleo.may_hear && cleo.heard.is_none());
        // Back near (20 m): heard again.
        w.place(2, 0.02);
        w.run(10);
        for s in &w.sides {
            assert_eq!(s.subscribed().len(), 2, "back in range");
        }
        // Ada: *Who hears me: Selected riders* with Cleo selected.
        assert!(
            select_command(&mut w.sides[0].c, None, "Cleo Sprinter", None)
                .unwrap()
                .contains("Cleo Sprinter selected")
        );
        let mut s = w.sides[0].c.settings().clone();
        s.heard_by = HeardBy::Selected;
        w.sides[0].c.set_settings(s);
        w.run(10);
        assert_eq!(w.allowed(0), Some(w.set(&[2])), "only Cleo may hear Ada");
        assert_eq!(w.sides[1].subscribed(), w.set(&[2]), "Bea does not get Ada");
        assert_eq!(w.sides[2].subscribed(), w.set(&[0, 1]));
        assert_eq!(
            w.sides[0].subscribed(),
            w.set(&[1, 2]),
            "Ada still hears both"
        );
        // Bea: *Who hears me: No one* (the default).
        let mut s = w.sides[1].c.settings().clone();
        s.heard_by = HeardBy::NoOne;
        w.sides[1].c.set_settings(s);
        w.run(10);
        assert_eq!(w.allowed(1), Some(w.set(&[])));
        assert_eq!(w.sides[0].subscribed(), w.set(&[2]));
        assert_eq!(w.sides[2].subscribed(), w.set(&[0]));
        // Cleo: *Voice room: Selected riders* with nobody selected: hears
        // nobody, and the server sends her nothing.
        let mut s = w.sides[2].c.settings().clone();
        s.listen = Listen::Selected;
        w.sides[2].c.set_settings(s);
        w.run(10);
        assert!(w.sides[2].subscribed().is_empty());
        assert!(w.server.voice_hearing(w.sides[2].conn).may_hear.is_empty());
        assert_eq!(
            w.allowed(0),
            Some(w.set(&[])),
            "Ada's voice reaches nobody now"
        );
        // The headphones (Q12's proposal): Cleo shows them to the riders
        // she does not listen to.
        let signs = voice_signs(&w.sides[2].c, true);
        assert_eq!(signs.not_hearing, vec![w.ids[0], w.ids[1]]);
        // Cleo selects Ada and listens to her again.
        select_command(&mut w.sides[2].c, None, "Ada Rider", Some("on")).unwrap();
        w.run(10);
        assert_eq!(w.sides[2].subscribed(), w.set(&[0]));
        assert_eq!(w.allowed(0), Some(w.set(&[2])));
        // A block always wins: Ada blocks her selected Cleo.
        block_command(&mut w.sides[0].c, None, "Cleo Sprinter", Some("on")).unwrap();
        w.run(10);
        assert_eq!(
            w.allowed(0),
            Some(w.set(&[])),
            "the block wins over the selection"
        );
        assert!(w.sides[2].subscribed().is_empty());
        assert!(!w.sides[0].subscribed().contains(&w.identity(2)));
        // What the server was told: Ada's selection without the blocked
        // Cleo.
        let a = w.sides[0].c.server_audience();
        assert!(a.selected_names.is_empty(), "{a:?}");
    }
}

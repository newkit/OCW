//! The voice controller: always-on voice as a call (spec §2.4, §7).
//!
//! Bevy-free; the app feeds it what it knows about the ride (the server
//! offers voice, how many human riders, the rider positions) and the token
//! grants, calls [`VoiceController::update`] every frame and sends the
//! token requests it returns. The controller decides when to join and
//! leave, retries, keeps the mute switches, chooses the voices to hear and
//! their placement, and drives the [`VoiceBackend`] with commands only
//! when something changed.

use std::collections::{BTreeMap, BTreeSet};

use ocw_core::RiderId;
use ocw_protocol::v1;

use crate::backend::{BackendCommand, BackendEvent, VoiceBackend};
use crate::device;
use crate::echo::{self, ECHO_IDENTITY, EchoConfig, EchoParams};
use crate::join::{JoinPolicy, REQUEST_TIMEOUT_S, Retry, RideFacts};
use crate::mix::Cue;
use crate::mute::{MuteChange, MuteState, Switch};
use crate::spatial::{self, Peer, SpatialConfig};

/// The rider's voice settings (`[voice]` in `settings.toml`; the mute
/// switches are kept in [`MuteState`]).
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceSettings {
    /// Voice chat at all (on by default: always on, like a call).
    pub enabled: bool,
    /// Attenuate by distance and hear only the nearest riders.
    pub spatial: bool,
    /// Voice volume, 0 … 2 (1 = as received).
    pub volume: f32,
    /// Nearest-N: at most this many voices in the session room.
    pub max_voices: usize,
    /// Microphone and speaker by name (`None`: the system default).
    pub input: Option<String>,
    pub output: Option<String>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            spatial: true,
            volume: 1.0,
            max_voices: SpatialConfig::default().max_voices,
            input: None,
            output: None,
        }
    }
}

/// What the HUD and the menu show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Voice is not built in (or cannot work here); why.
    Unavailable(String),
    /// Turned off in the settings.
    Off,
    /// The server has no voice (solo ride on the local server, or a server
    /// without LiveKit).
    NoServerVoice,
    /// Alone on the server: joins when another rider comes.
    WaitingForRiders,
    Connecting,
    Connected,
    Reconnecting,
    /// The last attempt failed; retrying.
    Failed(String),
}

impl Status {
    /// Shown on the HUD (the voice indicator appears while joining, in the
    /// call and after a failure).
    pub fn in_call(&self) -> bool {
        matches!(
            self,
            Self::Connecting | Self::Connected | Self::Reconnecting | Self::Failed(_)
        )
    }

    pub fn text(&self) -> String {
        match self {
            Self::Unavailable(why) => format!("unavailable: {why}"),
            Self::Off => "off".into(),
            Self::NoServerVoice => "no voice on this server".into(),
            Self::WaitingForRiders => "waiting for other riders".into(),
            Self::Connecting => "connecting".into(),
            Self::Connected => "connected".into(),
            Self::Reconnecting => "reconnecting".into(),
            Self::Failed(e) => format!("failed ({e}), retrying"),
        }
    }
}

/// The echo test as the HUD and the console show it (spec §7 *Echo bot*).
#[derive(Debug, Clone, PartialEq)]
pub enum EchoStatus {
    Off,
    /// Voice is not built in: nothing to echo.
    Unavailable(String),
    /// On, but no Echo rider on the road (yet).
    NoEchoRider,
    /// The Echo rider is too far to hear the rider.
    Far {
        distance_m: f64,
    },
    /// Near: what the rider says is echoed (`recording` 0 … 1 of it).
    Near {
        distance_m: f64,
        recording: f32,
    },
    /// In a call with the server's echo participant: it echoes over the
    /// real voice server (Opus, the SFU) and is heard like any rider.
    Remote {
        distance_m: Option<f64>,
    },
}

impl EchoStatus {
    pub fn text(&self) -> String {
        match self {
            Self::Off => "off".into(),
            Self::Unavailable(why) => format!("unavailable: {why}"),
            Self::NoEchoRider => "on, no Echo rider on the road".into(),
            Self::Far { distance_m } => format!("Echo rider {distance_m:.0} m away: ride closer"),
            Self::Near {
                distance_m,
                recording,
            } => format!(
                "near the Echo rider ({distance_m:.0} m): speak now, {:.0} % echoed",
                recording * 100.0
            ),
            Self::Remote {
                distance_m: Some(d),
            } => format!("echo over the voice server, Echo rider {d:.0} m away"),
            Self::Remote { distance_m: None } => "echo over the voice server".into(),
        }
    }

    /// The HUD hint ("echo test – speak now") is shown.
    pub fn near(&self) -> bool {
        match self {
            Self::Near { .. } => true,
            Self::Remote {
                distance_m: Some(d),
            } => EchoConfig::default().record_gain(*d) > 0.0,
            _ => false,
        }
    }
}

/// The echo test's state in the controller.
#[derive(Debug, Clone, Default)]
struct Echo {
    on: bool,
    config: EchoConfig,
    rider: Option<RiderId>,
    /// What the backend was told last (`None`: nothing yet).
    sent: Option<EchoParams>,
    placed: Option<(f32, f32)>,
    status: Option<EchoStatus>,
}

/// A participant of the voice room.
#[derive(Debug, Clone, PartialEq)]
pub struct Participant {
    pub identity: String,
    pub name: String,
    /// The rider on the road (identity `rider-<id>`).
    pub rider: Option<RiderId>,
    pub speaking: bool,
    /// Muted their own microphone.
    pub mic_muted: bool,
    /// Muted by the local rider (not heard).
    pub muted_by_me: bool,
    /// Subscribed and the gain played at (spatial × volume).
    pub heard: Option<f32>,
}

#[derive(Debug, Clone, PartialEq)]
enum Phase {
    Idle,
    Requesting { since: f64 },
    Connecting,
    Connected,
    Reconnecting,
    Waiting { until: f64, error: String },
}

pub struct VoiceController {
    backend: Box<dyn VoiceBackend>,
    available: Result<(), String>,
    settings: VoiceSettings,
    mute: MuteState,
    facts: RideFacts,
    policy: JoinPolicy,
    retry: Retry,
    phase: Phase,
    room: Option<String>,
    own_identity: Option<String>,
    participants: BTreeMap<String, Participant>,
    /// Identities the rider muted (kept while they are away, for this
    /// app run).
    muted_by_me: BTreeSet<String>,
    /// Subscribed voices and their placement as sent.
    placed: BTreeMap<String, (f32, f32)>,
    speaking: BTreeSet<String>,
    spatial: SpatialConfig,
    echo: Echo,
    /// Messages for the console / log.
    messages: Vec<String>,
}

impl VoiceController {
    pub fn new(backend: Box<dyn VoiceBackend>, settings: VoiceSettings, mute: MuteState) -> Self {
        let available = backend.available();
        let mut c = Self {
            backend,
            available,
            settings,
            mute,
            facts: RideFacts::default(),
            policy: JoinPolicy::default(),
            retry: Retry::default(),
            phase: Phase::Idle,
            room: None,
            own_identity: None,
            participants: BTreeMap::new(),
            muted_by_me: BTreeSet::new(),
            placed: BTreeMap::new(),
            speaking: BTreeSet::new(),
            spatial: SpatialConfig::default(),
            echo: Echo::default(),
            messages: Vec::new(),
        };
        c.spatial.max_voices = c.settings.max_voices;
        c.backend.send(BackendCommand::SetMicMuted(mute.mic_muted));
        c.backend
            .send(BackendCommand::SetSpeakerMuted(mute.speaker_muted));
        c.apply_devices();
        c
    }

    // ------------------------------------------------------------ inputs

    /// A new game session (Welcome): whether its server offers voice. A
    /// call of the old session is left.
    pub fn new_session(&mut self, server_offers_voice: bool) {
        self.leave();
        self.facts.server_offers_voice = server_offers_voice;
        self.facts.human_riders = 0;
        self.policy = JoinPolicy::default();
        self.retry.succeeded();
    }

    /// Human riders in the session (from the snapshots).
    pub fn set_human_riders(&mut self, n: u32) {
        self.facts.human_riders = n;
    }

    /// Joins a named group room instead of the session room (`None`: back
    /// to the session room). A call in progress is left and joined again.
    pub fn set_group(&mut self, group: Option<String>) -> Result<(), String> {
        if let Some(g) = &group
            && !ocw_protocol::valid_voice_group(g)
        {
            return Err(format!(
                "invalid group name {g:?}: 1 to {} characters of a-z, 0-9 and -",
                ocw_protocol::MAX_VOICE_GROUP_LEN
            ));
        }
        if group != self.facts.group {
            self.facts.group = group;
            self.leave();
            self.retry.succeeded();
        }
        Ok(())
    }

    pub fn group(&self) -> Option<&str> {
        self.facts.group.as_deref()
    }

    /// The server's answer to a token request.
    pub fn on_grant(&mut self, g: &v1::VoiceGrant, now_s: f64) {
        if !matches!(self.phase, Phase::Requesting { .. }) {
            return;
        }
        if !g.error.is_empty() || g.token.is_empty() {
            let error = if g.error.is_empty() {
                "no token".to_owned()
            } else {
                g.error.clone()
            };
            self.fail(error, now_s);
            return;
        }
        self.room = Some(g.room.clone());
        self.own_identity = Some(g.identity.clone());
        self.backend.send(BackendCommand::Connect {
            url: g.url.clone(),
            token: g.token.clone(),
        });
        self.phase = Phase::Connecting;
    }

    pub fn set_settings(&mut self, s: VoiceSettings) {
        let devices = (s.input.clone(), s.output.clone())
            != (self.settings.input.clone(), self.settings.output.clone());
        self.settings = s;
        self.spatial.max_voices = self.settings.max_voices;
        if devices {
            self.apply_devices();
        }
    }

    pub fn settings(&self) -> &VoiceSettings {
        &self.settings
    }

    /// The microphone switch (`voice.mute-mic`, M, Select). The change, if
    /// any, with its confirmation sound.
    pub fn set_mic(&mut self, s: Switch) -> Option<MuteChange> {
        let change = self.mute.set_mic(s)?;
        self.backend
            .send(BackendCommand::SetMicMuted(self.mute.mic_muted));
        self.cue(if self.mute.mic_muted {
            Cue::Muted
        } else {
            Cue::On
        });
        Some(change)
    }

    /// The speaker switch (`voice.mute-speaker`, Shift+M, R3).
    pub fn set_speaker(&mut self, s: Switch) -> Option<MuteChange> {
        let change = self.mute.set_speaker(s)?;
        self.backend
            .send(BackendCommand::SetSpeakerMuted(self.mute.speaker_muted));
        self.cue(if self.mute.speaker_muted {
            Cue::Muted
        } else {
            Cue::On
        });
        Some(change)
    }

    pub fn mute(&self) -> MuteState {
        self.mute
    }

    /// Mutes one participant for the local rider (by identity or rider).
    /// Returns whether they are muted now.
    pub fn mute_participant(&mut self, identity: &str, s: Switch) -> bool {
        let muted = s.apply(self.muted_by_me.contains(identity));
        if muted {
            self.muted_by_me.insert(identity.to_owned());
        } else {
            self.muted_by_me.remove(identity);
        }
        if let Some(p) = self.participants.get_mut(identity) {
            p.muted_by_me = muted;
        }
        muted
    }

    // ------------------------------------------------------------ update

    /// Advances the controller at `now_s` with the positions of the other
    /// riders relative to the local one. Returns a token request to send
    /// to the game server, if one is due.
    pub fn update(&mut self, now_s: f64, peers: &[Peer]) -> Option<v1::VoiceTokenRequest> {
        for e in self.backend.poll() {
            self.on_event(e, now_s);
        }
        let wanted = self.available.is_ok()
            && self
                .policy
                .wanted(&self.facts, self.settings.enabled, now_s);
        let mut request = None;
        match (self.phase.clone(), wanted) {
            (Phase::Idle, false) => {}
            (_, false) => self.leave(),
            (Phase::Idle, true) => request = Some(self.request(now_s)),
            (Phase::Requesting { since }, true) if now_s - since > REQUEST_TIMEOUT_S => {
                self.fail("no answer from the game server".into(), now_s)
            }
            (Phase::Waiting { until, .. }, true) if now_s >= until => {
                request = Some(self.request(now_s))
            }
            _ => {}
        }
        if matches!(self.phase, Phase::Connected | Phase::Reconnecting) {
            self.place_voices(peers);
        }
        self.drive_echo(peers);
        request
    }

    // ------------------------------------------------------------ echo

    /// Turns the echo test on or off (spec §7 *Echo bot*).
    pub fn set_echo(&mut self, on: bool) {
        self.echo.on = on;
    }

    pub fn echo_on(&self) -> bool {
        self.echo.on
    }

    /// Sets the echo's delay (clamped to 20 ms … 3 s); the delay set.
    pub fn set_echo_delay(&mut self, ms: u32) -> u32 {
        self.echo.config.delay_ms = echo::clamp_delay(ms);
        self.echo.config.delay_ms
    }

    pub fn echo_delay_ms(&self) -> u32 {
        self.echo.config.delay_ms
    }

    /// The Echo rider of the session (from the snapshots).
    pub fn set_echo_rider(&mut self, rider: Option<RiderId>) {
        self.echo.rider = rider;
    }

    pub fn echo_rider(&self) -> Option<RiderId> {
        self.echo.rider
    }

    pub fn echo_status(&self) -> EchoStatus {
        self.echo.status.clone().unwrap_or(EchoStatus::Off)
    }

    /// The microphone and echo levels the backend measured.
    pub fn levels(&self) -> crate::backend::Levels {
        self.backend.levels()
    }

    /// The local loopback: on, near the Echo rider and not done by the
    /// voice server's echo participant. Sends only changes.
    fn drive_echo(&mut self, peers: &[Peer]) {
        let peer = self
            .echo
            .rider
            .and_then(|r| peers.iter().find(|p| p.rider == r));
        let distance = peer.map(|p| p.gap_m.abs());
        // The server's echo participant joined the call as the Echo rider:
        // it echoes, the voice is placed like any other rider's.
        let remote = self.echo.rider.is_some_and(|r| {
            self.participants
                .contains_key(&ocw_protocol::voice_identity(r))
        });
        let status = if !self.echo.on {
            EchoStatus::Off
        } else if let Err(e) = &self.available {
            EchoStatus::Unavailable(e.clone())
        } else if remote {
            EchoStatus::Remote {
                distance_m: distance,
            }
        } else {
            match distance {
                None => EchoStatus::NoEchoRider,
                Some(d) => {
                    let recording = self.echo.config.record_gain(d);
                    if recording > 0.0 {
                        EchoStatus::Near {
                            distance_m: d,
                            recording,
                        }
                    } else {
                        EchoStatus::Far { distance_m: d }
                    }
                }
            }
        };
        let local = matches!(status, EchoStatus::Near { .. } | EchoStatus::Far { .. })
            && distance.is_some_and(|d| d <= self.echo.config.open_m);
        let params = EchoParams {
            active: local,
            delay_ms: self.echo.config.delay_ms,
            record_gain: match status {
                EchoStatus::Near { recording, .. } if local => (recording * 100.0).round() / 100.0,
                _ => 0.0,
            },
        };
        // Nothing to say while it never ran.
        if self.echo.sent != Some(params) && (self.echo.sent.is_some() || params.active) {
            self.backend.send(BackendCommand::SetEcho(params));
            self.echo.sent = Some(params);
        }
        if local && let Some(p) = peer {
            let volume = self.settings.volume.clamp(0.0, 2.0);
            let (gain, pan) = if self.settings.spatial {
                let pl = self.spatial.place(p);
                (pl.gain * volume, pl.pan)
            } else {
                (volume, 0.0)
            };
            let changed = self
                .echo
                .placed
                .is_none_or(|(g, q)| (g - gain).abs() >= 0.01 || (q - pan).abs() >= 0.02);
            if changed {
                self.backend.send(BackendCommand::SetPlacement {
                    identity: ECHO_IDENTITY.into(),
                    gain,
                    pan,
                });
                self.echo.placed = Some((gain, pan));
            }
        } else {
            self.echo.placed = None;
        }
        self.echo.status = Some(status);
    }

    fn request(&mut self, now_s: f64) -> v1::VoiceTokenRequest {
        self.phase = Phase::Requesting { since: now_s };
        let (channel, group) = match &self.facts.group {
            Some(g) => (v1::VoiceChannel::Group, g.clone()),
            None => (v1::VoiceChannel::Session, String::new()),
        };
        v1::VoiceTokenRequest {
            channel: channel as i32,
            group,
        }
    }

    fn fail(&mut self, error: String, now_s: f64) {
        let wait = self.retry.failed();
        self.messages
            .push(format!("voice: {error}; retrying in {wait:.0} s"));
        self.clear_room();
        self.phase = Phase::Waiting {
            until: now_s + wait,
            error,
        };
    }

    /// Leaves the room (if in one or joining).
    fn leave(&mut self) {
        if matches!(
            self.phase,
            Phase::Connecting | Phase::Connected | Phase::Reconnecting
        ) {
            self.backend.send(BackendCommand::Disconnect);
            if self.phase == Phase::Connected {
                self.messages.push("voice: left the room".into());
            }
        }
        self.clear_room();
        self.phase = Phase::Idle;
    }

    fn clear_room(&mut self) {
        self.participants.clear();
        self.placed.clear();
        self.speaking.clear();
        self.room = None;
    }

    fn cue(&mut self, cue: Cue) {
        if self.available.is_ok() {
            self.backend.send(BackendCommand::PlayCue(cue));
        }
    }

    fn on_event(&mut self, e: BackendEvent, now_s: f64) {
        match e {
            BackendEvent::Connected { room } => {
                if self.phase == Phase::Connecting {
                    self.phase = Phase::Connected;
                    self.retry.succeeded();
                    self.messages.push(format!("voice: connected to {room}"));
                    self.cue(Cue::Joined);
                }
            }
            BackendEvent::Reconnecting => {
                if self.phase == Phase::Connected {
                    self.phase = Phase::Reconnecting;
                }
            }
            BackendEvent::Reconnected => {
                if self.phase == Phase::Reconnecting {
                    self.phase = Phase::Connected;
                }
            }
            BackendEvent::Disconnected { reason } => {
                if matches!(
                    self.phase,
                    Phase::Connecting | Phase::Connected | Phase::Reconnecting
                ) {
                    let reason = if reason.is_empty() {
                        "disconnected".to_owned()
                    } else {
                        reason
                    };
                    self.fail(reason, now_s);
                }
            }
            BackendEvent::ConnectFailed(e) => {
                if self.phase == Phase::Connecting {
                    self.fail(e, now_s);
                }
            }
            BackendEvent::ParticipantJoined { identity, name } => {
                if self.phase != Phase::Connected && self.phase != Phase::Reconnecting {
                    return;
                }
                let rider = ocw_protocol::rider_of_voice_identity(&identity);
                let muted_by_me = self.muted_by_me.contains(&identity);
                let fresh = !self.participants.contains_key(&identity);
                self.participants.insert(
                    identity.clone(),
                    Participant {
                        identity,
                        name,
                        rider,
                        speaking: false,
                        mic_muted: false,
                        muted_by_me,
                        heard: None,
                    },
                );
                if fresh {
                    self.cue(Cue::Joined);
                }
            }
            BackendEvent::ParticipantLeft { identity } => {
                if self.participants.remove(&identity).is_some() {
                    self.cue(Cue::Left);
                }
                self.placed.remove(&identity);
                self.speaking.remove(&identity);
            }
            BackendEvent::ParticipantMuted { identity, muted } => {
                if let Some(p) = self.participants.get_mut(&identity) {
                    p.mic_muted = muted;
                }
            }
            BackendEvent::Speaking(ids) => {
                self.speaking = ids.into_iter().collect();
                for p in self.participants.values_mut() {
                    p.speaking = self.speaking.contains(&p.identity);
                }
            }
            BackendEvent::DeviceError(e) => self.messages.push(format!("voice: audio device: {e}")),
        }
    }

    /// Chooses the voices to hear and their placement; sends only changes.
    fn place_voices(&mut self, peers: &[Peer]) {
        let volume = self.settings.volume.clamp(0.0, 2.0);
        let mut want: BTreeMap<String, (f32, f32)> = BTreeMap::new();
        let candidates: Vec<&Participant> = self
            .participants
            .values()
            .filter(|p| !p.muted_by_me)
            .collect();
        let spatial = self.settings.spatial && self.facts.group.is_none();
        if spatial {
            let by_rider: BTreeMap<RiderId, &Participant> = candidates
                .iter()
                .filter_map(|p| p.rider.map(|r| (r, *p)))
                .collect();
            let known: Vec<Peer> = peers
                .iter()
                .filter(|p| by_rider.contains_key(&p.rider))
                .copied()
                .collect();
            let current: Vec<RiderId> = self
                .placed
                .keys()
                .filter_map(|i| ocw_protocol::rider_of_voice_identity(i))
                .collect();
            for (r, pl) in spatial::select(&known, &current, &self.spatial) {
                want.insert(by_rider[&r].identity.clone(), (pl.gain * volume, pl.pan));
            }
            // Riders not (yet) on the road and participants who are not
            // riders: quiet, while there is room.
            for p in &candidates {
                if want.len() >= self.spatial.max_voices {
                    break;
                }
                let placed = p.rider.is_some_and(|r| known.iter().any(|k| k.rider == r));
                if !placed && !want.contains_key(&p.identity) {
                    let gain = if p.rider.is_some() {
                        self.spatial.min_gain
                    } else {
                        1.0
                    };
                    want.insert(p.identity.clone(), (gain * volume, 0.0));
                }
            }
        } else {
            for p in &candidates {
                want.insert(p.identity.clone(), (volume, 0.0));
            }
        }
        let gone: Vec<String> = self
            .placed
            .keys()
            .filter(|i| !want.contains_key(*i))
            .cloned()
            .collect();
        for identity in gone {
            self.placed.remove(&identity);
            self.backend.send(BackendCommand::Subscribe {
                identity,
                on: false,
            });
        }
        for (identity, (gain, pan)) in want {
            match self.placed.get(&identity) {
                None => {
                    self.backend.send(BackendCommand::Subscribe {
                        identity: identity.clone(),
                        on: true,
                    });
                }
                Some(&(g, p)) if (g - gain).abs() < 0.01 && (p - pan).abs() < 0.02 => continue,
                Some(_) => {}
            }
            self.backend.send(BackendCommand::SetPlacement {
                identity: identity.clone(),
                gain,
                pan,
            });
            self.placed.insert(identity, (gain, pan));
        }
        for p in self.participants.values_mut() {
            p.heard = self.placed.get(&p.identity).map(|(g, _)| *g);
        }
    }

    fn apply_devices(&mut self) {
        let inputs = self.backend.input_devices();
        let outputs = self.backend.output_devices();
        let input = device::choose(self.settings.input.as_deref(), &inputs);
        let output = device::choose(self.settings.output.as_deref(), &outputs);
        for (what, c) in [("microphone", &input), ("speaker", &output)] {
            if let device::Choice::Default { missing: Some(m) } = c {
                self.messages
                    .push(format!("voice: {what} {m:?} not found, using the default"));
            }
        }
        self.backend.send(BackendCommand::SetInputDevice(
            input.name().map(str::to_owned),
        ));
        self.backend.send(BackendCommand::SetOutputDevice(
            output.name().map(str::to_owned),
        ));
    }

    // ------------------------------------------------------------ outputs

    pub fn status(&self) -> Status {
        if let Err(e) = &self.available {
            return Status::Unavailable(e.clone());
        }
        if !self.settings.enabled {
            return Status::Off;
        }
        if !self.facts.server_offers_voice {
            return Status::NoServerVoice;
        }
        match &self.phase {
            Phase::Idle => Status::WaitingForRiders,
            Phase::Requesting { .. } | Phase::Connecting => Status::Connecting,
            Phase::Connected => Status::Connected,
            Phase::Reconnecting => Status::Reconnecting,
            Phase::Waiting { error, .. } => Status::Failed(error.clone()),
        }
    }

    pub fn room(&self) -> Option<&str> {
        self.room.as_deref()
    }

    pub fn participants(&self) -> impl Iterator<Item = &Participant> {
        self.participants.values()
    }

    /// Riders talking now (the speaking marker on the road).
    pub fn speaking_riders(&self) -> Vec<RiderId> {
        self.speaking
            .iter()
            .filter_map(|i| ocw_protocol::rider_of_voice_identity(i))
            .collect()
    }

    /// The local rider is talking (and heard).
    pub fn self_speaking(&self) -> bool {
        !self.mute.mic_muted
            && self
                .own_identity
                .as_ref()
                .is_some_and(|i| self.speaking.contains(i))
    }

    pub fn input_devices(&mut self) -> Vec<String> {
        self.backend.input_devices()
    }

    pub fn output_devices(&mut self) -> Vec<String> {
        self.backend.output_devices()
    }

    pub fn take_messages(&mut self) -> Vec<String> {
        std::mem::take(&mut self.messages)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::backend::MockBackend;

    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<MockBackend>>);

    impl VoiceBackend for Shared {
        fn available(&self) -> Result<(), String> {
            Ok(())
        }
        fn input_devices(&mut self) -> Vec<String> {
            self.0.lock().unwrap().inputs.clone()
        }
        fn output_devices(&mut self) -> Vec<String> {
            self.0.lock().unwrap().outputs.clone()
        }
        fn send(&mut self, cmd: BackendCommand) {
            self.0.lock().unwrap().sent.push(cmd);
        }
        fn poll(&mut self) -> Vec<BackendEvent> {
            std::mem::take(&mut self.0.lock().unwrap().events)
        }
    }

    impl Shared {
        fn take_sent(&self) -> Vec<BackendCommand> {
            std::mem::take(&mut self.0.lock().unwrap().sent)
        }
        fn push(&self, e: BackendEvent) {
            self.0.lock().unwrap().events.push(e);
        }
    }

    fn controller(settings: VoiceSettings, mute: MuteState) -> (VoiceController, Shared) {
        let b = Shared::default();
        {
            let mut m = b.0.lock().unwrap();
            m.inputs = vec!["Mic A".into(), "Headset".into()];
            m.outputs = vec!["Speakers".into()];
        }
        let c = VoiceController::new(Box::new(b.clone()), settings, mute);
        (c, b)
    }

    fn grant(room: &str) -> v1::VoiceGrant {
        v1::VoiceGrant {
            url: "ws://lk".into(),
            token: "tok".into(),
            room: room.into(),
            identity: "rider-1".into(),
            expires_unix_s: 0,
            channel: v1::VoiceChannel::Session as i32,
            error: String::new(),
        }
    }

    /// Brings a controller into a connected call with riders 2 and 3.
    fn connected() -> (VoiceController, Shared) {
        let (mut c, b) = controller(VoiceSettings::default(), MuteState::default());
        c.new_session(true);
        c.set_human_riders(3);
        assert!(c.update(0.0, &[]).is_some());
        c.on_grant(&grant("ocw-x"), 0.1);
        b.push(BackendEvent::Connected {
            room: "ocw-x".into(),
        });
        for id in ["rider-2", "rider-3"] {
            b.push(BackendEvent::ParticipantJoined {
                identity: id.into(),
                name: id.into(),
            });
        }
        c.update(0.2, &[]);
        assert_eq!(c.status(), Status::Connected);
        b.take_sent();
        (c, b)
    }

    fn peer(id: u32, gap: f64) -> Peer {
        Peer {
            rider: RiderId(id),
            gap_m: gap,
            lateral_m: 0.0,
        }
    }

    #[test]
    fn starts_with_the_remembered_mute_state_and_devices() {
        let settings = VoiceSettings {
            input: Some("Headset".into()),
            output: Some("Gone".into()),
            ..Default::default()
        };
        let (mut c, b) = controller(
            settings,
            MuteState {
                mic_muted: true,
                speaker_muted: false,
            },
        );
        let sent = b.take_sent();
        assert!(sent.contains(&BackendCommand::SetMicMuted(true)));
        assert!(sent.contains(&BackendCommand::SetSpeakerMuted(false)));
        assert!(sent.contains(&BackendCommand::SetInputDevice(Some("Headset".into()))));
        // A missing speaker: the default, with a message.
        assert!(sent.contains(&BackendCommand::SetOutputDevice(None)));
        assert!(
            c.take_messages()
                .iter()
                .any(|m| m.contains("\"Gone\" not found"))
        );
    }

    #[test]
    fn joins_automatically_on_shared_rides_only() {
        let (mut c, b) = controller(VoiceSettings::default(), MuteState::default());
        // Solo ride on the local server.
        c.new_session(false);
        c.set_human_riders(1);
        assert_eq!(c.update(0.0, &[]), None);
        assert_eq!(c.status(), Status::NoServerVoice);
        // A remote server with voice, alone.
        c.new_session(true);
        c.set_human_riders(1);
        assert_eq!(c.update(1.0, &[]), None);
        assert_eq!(c.status(), Status::WaitingForRiders);
        // Someone joins: a token request for the session room, once.
        c.set_human_riders(2);
        let req = c.update(2.0, &[]).expect("a token request");
        assert_eq!(req.channel(), v1::VoiceChannel::Session);
        assert_eq!(c.update(2.1, &[]), None);
        assert_eq!(c.status(), Status::Connecting);
        c.on_grant(&grant("ocw-x"), 2.2);
        assert!(b.take_sent().contains(&BackendCommand::Connect {
            url: "ws://lk".into(),
            token: "tok".into()
        }));
        b.push(BackendEvent::Connected {
            room: "ocw-x".into(),
        });
        c.update(2.5, &[]);
        assert_eq!(c.status(), Status::Connected);
        assert_eq!(c.room(), Some("ocw-x"));
        assert!(
            b.take_sent()
                .contains(&BackendCommand::PlayCue(Cue::Joined))
        );
        // Alone again: leaves after the grace period.
        c.set_human_riders(1);
        c.update(3.0, &[]);
        assert_eq!(c.status(), Status::Connected);
        c.update(3.0 + crate::join::LEAVE_GRACE_S, &[]);
        assert_eq!(c.status(), Status::WaitingForRiders);
        assert!(b.take_sent().contains(&BackendCommand::Disconnect));
    }

    #[test]
    fn a_ride_switch_leaves_the_call() {
        let (mut c, b) = connected();
        c.new_session(false);
        assert!(b.take_sent().contains(&BackendCommand::Disconnect));
        assert_eq!(c.status(), Status::NoServerVoice);
        assert_eq!(c.participants().count(), 0);
    }

    #[test]
    fn voice_off_in_the_settings_leaves_and_stays_out() {
        let (mut c, b) = connected();
        c.set_settings(VoiceSettings {
            enabled: false,
            ..Default::default()
        });
        assert_eq!(c.update(1.0, &[]), None);
        assert!(b.take_sent().contains(&BackendCommand::Disconnect));
        assert_eq!(c.status(), Status::Off);
        assert_eq!(c.update(2.0, &[]), None);
    }

    #[test]
    fn refused_and_failed_joins_are_retried_with_back_off() {
        let (mut c, b) = controller(VoiceSettings::default(), MuteState::default());
        c.new_session(true);
        c.set_human_riders(2);
        assert!(c.update(0.0, &[]).is_some());
        c.on_grant(
            &v1::VoiceGrant {
                error: "too many voice token requests".into(),
                ..Default::default()
            },
            0.1,
        );
        assert!(matches!(c.status(), Status::Failed(e) if e.contains("too many")));
        assert_eq!(c.update(1.0, &[]), None, "waits 2 s");
        assert!(c.update(2.2, &[]).is_some());
        // No answer at all: a timeout, then 4 s.
        assert_eq!(c.update(2.2 + REQUEST_TIMEOUT_S + 0.1, &[]), None);
        assert!(matches!(c.status(), Status::Failed(_)));
        let t = 2.2 + REQUEST_TIMEOUT_S + 0.1;
        assert_eq!(c.update(t + 3.9, &[]), None);
        assert!(c.update(t + 4.0, &[]).is_some());
        // The connection fails: retried.
        c.on_grant(&grant("ocw-x"), t + 4.1);
        b.push(BackendEvent::ConnectFailed("server unreachable".into()));
        c.update(t + 4.2, &[]);
        assert!(matches!(c.status(), Status::Failed(e) if e == "server unreachable"));
        // A grant arriving late (not asked for) is ignored.
        b.take_sent();
        c.on_grant(&grant("ocw-x"), t + 4.3);
        assert!(b.take_sent().is_empty());
    }

    #[test]
    fn a_dropped_call_reconnects() {
        let (mut c, b) = connected();
        b.push(BackendEvent::Reconnecting);
        c.update(1.0, &[]);
        assert_eq!(c.status(), Status::Reconnecting);
        b.push(BackendEvent::Reconnected);
        c.update(1.1, &[]);
        assert_eq!(c.status(), Status::Connected);
        b.push(BackendEvent::Disconnected {
            reason: "signal lost".into(),
        });
        c.update(2.0, &[]);
        assert!(matches!(c.status(), Status::Failed(_)));
        assert_eq!(c.participants().count(), 0);
        // A new token, then a new connection.
        assert!(c.update(4.0, &[]).is_some());
    }

    #[test]
    fn mute_switches_drive_the_track_and_play_a_cue() {
        let (mut c, b) = connected();
        assert_eq!(
            c.set_mic(Switch::Toggle),
            Some(MuteChange::Mic { muted: true })
        );
        assert_eq!(
            b.take_sent(),
            vec![
                BackendCommand::SetMicMuted(true),
                BackendCommand::PlayCue(Cue::Muted)
            ]
        );
        assert_eq!(c.set_mic(Switch::On), None, "already muted: nothing");
        assert!(b.take_sent().is_empty());
        assert_eq!(
            c.set_speaker(Switch::On),
            Some(MuteChange::Speaker { muted: true })
        );
        assert_eq!(
            b.take_sent(),
            vec![
                BackendCommand::SetSpeakerMuted(true),
                BackendCommand::PlayCue(Cue::Muted)
            ]
        );
        // The speaker mute keeps the microphone as it is; the call goes on.
        assert!(c.mute().mic_muted);
        assert_eq!(c.status(), Status::Connected);
        c.set_mic(Switch::Off);
        assert_eq!(
            b.take_sent(),
            vec![
                BackendCommand::SetMicMuted(false),
                BackendCommand::PlayCue(Cue::On)
            ]
        );
    }

    #[test]
    fn nearest_voices_attenuated_and_muted_participants_dropped() {
        let (mut c, b) = connected();
        // Joined before their positions were known: already subscribed
        // (quietly, see below); now placed by distance.
        c.update(1.0, &[peer(2, 5.0), peer(3, -40.0)]);
        let sent = b.take_sent();
        for id in ["rider-2", "rider-3"] {
            assert!(
                sent.iter().any(
                    |s| matches!(s, BackendCommand::SetPlacement { identity, .. } if identity == id)
                ),
                "{sent:?}"
            );
        }
        let gain_of = |c: &VoiceController, id: &str| {
            c.participants().find(|p| p.identity == id).unwrap().heard
        };
        assert_eq!(gain_of(&c, "rider-2"), Some(1.0));
        assert!(gain_of(&c, "rider-3").unwrap() < 0.5, "behind: quieter");
        // Nothing moved: no commands.
        c.update(1.1, &[peer(2, 5.0), peer(3, -40.0)]);
        assert!(b.take_sent().is_empty());
        // Muting rider 3 unsubscribes them.
        assert!(c.mute_participant("rider-3", Switch::Toggle));
        c.update(1.2, &[peer(2, 5.0), peer(3, -40.0)]);
        assert_eq!(
            b.take_sent(),
            vec![BackendCommand::Subscribe {
                identity: "rider-3".into(),
                on: false
            }]
        );
        assert_eq!(gain_of(&c, "rider-3"), None);
        // Out of range: not heard.
        c.mute_participant("rider-3", Switch::Off);
        c.update(1.3, &[peer(2, 1000.0), peer(3, -40.0)]);
        let sent = b.take_sent();
        assert!(sent.contains(&BackendCommand::Subscribe {
            identity: "rider-2".into(),
            on: false
        }));
        assert!(sent.contains(&BackendCommand::Subscribe {
            identity: "rider-3".into(),
            on: true
        }));
    }

    #[test]
    fn nearest_n_limits_the_voices() {
        let (mut c, b) = connected();
        c.set_settings(VoiceSettings {
            max_voices: 1,
            ..Default::default()
        });
        c.update(1.0, &[peer(2, 50.0), peer(3, 5.0)]);
        let heard: Vec<String> = c
            .participants()
            .filter(|p| p.heard.is_some())
            .map(|p| p.identity.clone())
            .collect();
        assert_eq!(heard, vec!["rider-3".to_owned()]);
        b.take_sent();
    }

    #[test]
    fn without_spatial_or_in_a_group_everyone_is_heard_at_the_volume() {
        let (mut c, _b) = connected();
        c.set_settings(VoiceSettings {
            spatial: false,
            volume: 0.8,
            ..Default::default()
        });
        c.update(1.0, &[peer(2, 5.0), peer(3, -250.0)]);
        assert!(c.participants().all(|p| p.heard == Some(0.8)));
        // Riders whose position is unknown are heard quietly with spatial on.
        let (mut c, _b) = connected();
        c.update(1.0, &[peer(2, 5.0)]);
        let p3 = c
            .participants()
            .find(|p| p.identity == "rider-3")
            .unwrap()
            .heard;
        assert_eq!(p3, Some(SpatialConfig::default().min_gain));
    }

    #[test]
    fn group_rooms_and_speaking_markers() {
        let (mut c, b) = connected();
        assert!(c.set_group(Some("Bad Name".into())).is_err());
        c.set_group(Some("team-1".into())).unwrap();
        assert!(b.take_sent().contains(&BackendCommand::Disconnect));
        let req = c.update(1.0, &[]).unwrap();
        assert_eq!(
            (req.channel(), req.group.as_str()),
            (v1::VoiceChannel::Group, "team-1")
        );
        c.on_grant(&grant("ocw-x-group-team-1"), 1.1);
        b.push(BackendEvent::Connected {
            room: "ocw-x-group-team-1".into(),
        });
        b.push(BackendEvent::ParticipantJoined {
            identity: "rider-2".into(),
            name: "B".into(),
        });
        b.push(BackendEvent::Speaking(vec![
            "rider-2".into(),
            "rider-1".into(),
        ]));
        c.update(1.2, &[]);
        assert_eq!(c.speaking_riders(), vec![RiderId(1), RiderId(2)]);
        assert!(c.self_speaking());
        c.set_mic(Switch::On);
        assert!(!c.self_speaking(), "muted: no marker");
        b.push(BackendEvent::ParticipantLeft {
            identity: "rider-2".into(),
        });
        c.update(1.3, &[]);
        assert!(b.take_sent().contains(&BackendCommand::PlayCue(Cue::Left)));
        assert_eq!(c.speaking_riders(), vec![RiderId(1)]);
    }

    #[test]
    fn null_backend_reports_not_built_in() {
        let mut c = VoiceController::new(
            Box::new(crate::NullBackend),
            VoiceSettings::default(),
            MuteState::default(),
        );
        c.new_session(true);
        c.set_human_riders(4);
        assert_eq!(c.update(0.0, &[]), None);
        assert!(matches!(c.status(), Status::Unavailable(e) if e.contains("not built")));
        // The switches still work (and are remembered).
        assert!(c.set_mic(Switch::Toggle).is_some());
        assert!(c.mute().mic_muted);
    }
}

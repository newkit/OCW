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

use crate::audience::{HeardBy, Hearing, Listen};
use crate::backend::{BackendCommand, BackendEvent, VoiceBackend};
use crate::device;
use crate::echo::{self, ECHO_IDENTITY, EchoConfig, EchoParams};
use crate::join::{JoinPolicy, REQUEST_TIMEOUT_S, Retry, RideFacts};
use crate::mix::Cue;
use crate::mute::{MuteChange, MuteState, Switch};
use crate::riders::{self, RiderKeys, RiderPrefs};
use crate::spatial::{self, Peer, SpatialConfig};

/// A rider who talked this long ago (s) still counts as talking for the
/// cap (spec §7.1.2), so a pause between two sentences does not swap the
/// voices.
pub const TALKED_RECENTLY_S: f64 = 5.0;

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
    /// At most this many voices in the session room (talking first, then
    /// the nearest; spec §7.1.2).
    pub max_voices: usize,
    /// Microphone and speaker by name (`None`: the system default).
    pub input: Option<String>,
    pub output: Option<String>,
    /// *Who hears me* (spec §7.1.7 Q8; default no one).
    pub heard_by: HeardBy,
    /// *Voice room*: whom the rider listens to (spec §7.1.3).
    pub listen: Listen,
    /// The first call's notice is still to be shown (spec §7.1.2;
    /// `voice.first_call_notice`, shown by the apps' driver).
    pub notice: bool,
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
            heard_by: HeardBy::default(),
            listen: Listen::default(),
            notice: true,
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
    /// Blocked by the local rider (spec §7.1.7 Q9): remembered, both
    /// directions.
    pub blocked_by_me: bool,
    /// The game server says the two exchange no voice (a block in either
    /// direction; which one is not said).
    pub apart: bool,
    /// The local rider's volume for this rider (0 … 200 %).
    pub volume_pct: u16,
    /// One of the local rider's *Selected riders* (spec §7.1.3).
    pub selected: bool,
    /// The game server lets the local rider hear this one (in range, their
    /// *Who hears me* allows it, the own *Voice room* includes them and no
    /// block; spec §7.1.7 Q6). Group rooms are not limited by it.
    pub may_hear: bool,
    /// How far away on the road, if the rider is on it.
    pub distance_m: Option<f64>,
    /// Subscribed and the gain played at (spatial × volume × the rider's
    /// volume).
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
    /// The rider's volume and blocks for other riders (spec §7.1.7 Q9),
    /// the server they are matched on (name keys) and what the snapshots
    /// say of the riders (names, accounts).
    prefs: RiderPrefs,
    scope: String,
    rider_names: BTreeMap<RiderId, String>,
    rider_accounts: BTreeMap<RiderId, String>,
    /// The riders the game server keeps apart from this one.
    apart: BTreeSet<RiderId>,
    /// Who may subscribe to the own microphone, as last told (`None`:
    /// everybody, the backend's default of every call).
    subscribers: Option<Vec<String>>,
    /// The game server's `VoiceHearing` (spec §7.1.7 Q6): `None` until the
    /// first one of the session – nobody is heard and nobody hears the
    /// rider in the session room until it comes.
    hearing: Option<Hearing>,
    /// When each rider last talked (s): riders who talked a moment ago
    /// keep their place in the cap (spec §7.1.2).
    last_talked: BTreeMap<RiderId, f64>,
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
            prefs: RiderPrefs::new(),
            scope: riders::LOCAL_SCOPE.into(),
            rider_names: BTreeMap::new(),
            rider_accounts: BTreeMap::new(),
            apart: BTreeSet::new(),
            subscribers: None,
            hearing: None,
            last_talked: BTreeMap::new(),
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
        self.hearing = None;
        self.last_talked.clear();
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
        // Who may hear the rider, before the microphone is published
        // (the backends apply it at the connect).
        self.apply_subscribers();
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

    // ------------------------------------------------- volume and block

    /// The remembered volumes and blocks (from the settings).
    pub fn set_rider_prefs(&mut self, prefs: RiderPrefs) {
        self.prefs = prefs;
        self.refresh_participants();
    }

    pub fn rider_prefs(&self) -> &RiderPrefs {
        &self.prefs
    }

    /// The server ridden on ([`riders::server_scope`]) and the riders'
    /// names and signed-in accounts from the snapshots: the keys the
    /// volumes and blocks are matched by.
    pub fn set_rider_identities(
        &mut self,
        scope: &str,
        names: BTreeMap<RiderId, String>,
        accounts: BTreeMap<RiderId, String>,
    ) {
        if self.scope != scope || self.rider_names != names || self.rider_accounts != accounts {
            self.scope = scope.to_owned();
            self.rider_names = names;
            self.rider_accounts = accounts;
            self.refresh_participants();
        }
    }

    pub fn scope(&self) -> &str {
        &self.scope
    }

    /// The keys of a participant: the rider's account and name (from the
    /// snapshots; the voice name if the rider is not on the road).
    pub fn keys_of(&self, p: &Participant) -> RiderKeys {
        let name = self.name_of(p);
        let account = p
            .rider
            .and_then(|r| self.rider_accounts.get(&r))
            .map(String::as_str);
        RiderKeys::new(&self.scope, name, account)
    }

    /// A participant's rider name: the snapshots' (the voice name if the
    /// rider is not on the road).
    pub fn name_of<'a>(&'a self, p: &'a Participant) -> &'a str {
        p.rider
            .and_then(|r| self.rider_names.get(&r))
            .filter(|n| !n.is_empty())
            .map_or(p.name.as_str(), String::as_str)
    }

    /// The riders the game server keeps apart from this one (its
    /// `VoiceApart`).
    pub fn set_apart(&mut self, riders: BTreeSet<RiderId>) {
        if self.apart != riders {
            self.apart = riders;
            self.refresh_participants();
        }
    }

    pub fn apart(&self) -> &BTreeSet<RiderId> {
        &self.apart
    }

    /// The blocks to tell the game server of this scope: (accounts,
    /// names).
    pub fn server_blocks(&self) -> (Vec<String>, Vec<String>) {
        riders::blocks_for(&self.prefs, &self.scope)
    }

    /// The audience to tell the game server (spec §7.1.7 Q8, §7.1.3):
    /// *Who hears me*, *Voice room* and the selected riders of this scope.
    pub fn server_audience(&self) -> v1::VoiceAudience {
        crate::audience::audience_message(
            self.settings.heard_by,
            self.settings.listen,
            riders::selected_for(&self.prefs, &self.scope),
        )
    }

    /// The game server's `VoiceHearing` (spec §7.1.7 Q6): whose voice the
    /// rider may receive and who may receive the rider's.
    pub fn set_hearing(&mut self, hearing: Option<Hearing>) {
        if self.hearing != hearing {
            self.hearing = hearing;
            self.refresh_participants();
        }
    }

    pub fn hearing(&self) -> Option<&Hearing> {
        self.hearing.as_ref()
    }

    fn refresh_participants(&mut self) {
        let flags: Vec<(String, bool, u16, bool)> = self
            .participants
            .values()
            .map(|p| {
                let k = self.keys_of(p);
                (
                    p.identity.clone(),
                    k.blocked(&self.prefs),
                    k.volume_pct(&self.prefs),
                    k.selected(&self.prefs),
                )
            })
            .collect();
        for (identity, blocked, volume, selected) in flags {
            if let Some(p) = self.participants.get_mut(&identity) {
                p.blocked_by_me = blocked;
                p.volume_pct = volume;
                p.selected = selected;
                p.apart = p.rider.is_some_and(|r| self.apart.contains(&r));
                p.may_hear = p.rider.is_some_and(|r| {
                    self.hearing
                        .as_ref()
                        .is_some_and(|h| h.may_hear.contains(&r))
                });
            }
        }
    }

    /// In a group room everybody in it is heard (the group is the
    /// selection, spec §7.1.3); elsewhere the game server's hearing rules.
    fn in_group(&self) -> bool {
        self.facts.group.is_some()
    }

    /// Who may subscribe to the own microphone (LiveKit's track
    /// subscription permissions, enforced by the voice server):
    ///
    /// - in a group room everybody but the riders blocked or kept apart
    ///   (spec §7.1.7 Q9; `None` when nobody is denied);
    /// - in the session and event rooms (spec §7.1.7 Q6, Q8): nobody with
    ///   *Who hears me: No one* or before the game server said; else the
    ///   riders the game server lets hear the rider (`heard_by`: in range,
    ///   their room, the own selection), never a blocked one – a block
    ///   always wins.
    ///
    /// Sends only changes.
    fn apply_subscribers(&mut self) {
        let want = if self.in_group() {
            let denied = self
                .participants
                .values()
                .any(|p| p.blocked_by_me || p.apart);
            denied.then(|| {
                self.participants
                    .values()
                    .filter(|p| !p.blocked_by_me && !p.apart)
                    .map(|p| p.identity.clone())
                    .collect::<Vec<_>>()
            })
        } else {
            let denied: BTreeSet<RiderId> = self
                .participants
                .values()
                .filter(|p| p.blocked_by_me || p.apart)
                .filter_map(|p| p.rider)
                .chain(self.apart.iter().copied())
                .collect();
            let allowed: Vec<String> = match (self.settings.heard_by, &self.hearing) {
                (HeardBy::NoOne, _) | (_, None) => Vec::new(),
                (heard_by, Some(h)) => h
                    .heard_by
                    .iter()
                    .filter(|r| !denied.contains(r))
                    .filter(|r| {
                        // The own selection, also before the server knows
                        // of a change.
                        heard_by != HeardBy::Selected || self.rider_selected(**r)
                    })
                    .map(|r| ocw_protocol::voice_identity(*r))
                    .collect(),
            };
            Some(allowed)
        };
        if want != self.subscribers {
            self.backend
                .send(BackendCommand::SetSubscribers(want.clone()));
            self.subscribers = want;
        }
    }

    /// Whether rider `r` is one of the selected riders (by the snapshots'
    /// name and account; spec §7.1.3).
    fn rider_selected(&self, r: RiderId) -> bool {
        let Some(name) = self.rider_names.get(&r) else {
            // Not on the road (yet): a participant's voice name.
            let identity = ocw_protocol::voice_identity(r);
            return self.participants.get(&identity).is_some_and(|p| p.selected);
        };
        RiderKeys::new(
            &self.scope,
            name,
            self.rider_accounts.get(&r).map(String::as_str),
        )
        .selected(&self.prefs)
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
            self.place_voices(peers, now_s);
            self.apply_subscribers();
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
            // The next call starts muted (spec §7.1.7 Q7: every shared
            // ride is joined muted; nothing is remembered across rides).
            if !self.mute.mic_muted {
                self.mute.mic_muted = true;
                self.backend.send(BackendCommand::SetMicMuted(true));
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
        // A new call starts with everybody allowed (the backend's default).
        if self.subscribers.take().is_some() {
            self.backend.send(BackendCommand::SetSubscribers(None));
        }
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
                        blocked_by_me: false,
                        apart: false,
                        volume_pct: riders::DEFAULT_VOLUME_PCT,
                        selected: false,
                        may_hear: false,
                        distance_m: None,
                        heard: None,
                    },
                );
                self.refresh_participants();
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
    ///
    /// - Never: muted for me, blocked (both ways) or kept apart by the
    ///   server (spec §7.1.7 Q9) – a block always wins.
    /// - In the session and event rooms only the riders the game server
    ///   lets the rider hear (`may_hear`: in range, their *Who hears me*,
    ///   the own *Voice room*; spec §7.1.7 Q6, Q8) and, with *Voice room:
    ///   Selected riders*, only the selected ones; spatial: the gain falls
    ///   to zero at 300 m, at most `max_voices` – talking riders first,
    ///   then the nearest (spec §7.1.2).
    /// - In a group room everybody in it.
    fn place_voices(&mut self, peers: &[Peer], now_s: f64) {
        let volume = self.settings.volume.clamp(0.0, 2.0);
        // Who talked a moment ago (the cap's priority).
        for r in self
            .speaking
            .iter()
            .filter_map(|i| ocw_protocol::rider_of_voice_identity(i))
        {
            self.last_talked.insert(r, now_s);
        }
        self.last_talked
            .retain(|_, t| now_s - *t <= TALKED_RECENTLY_S && *t <= now_s);
        let talking: BTreeSet<RiderId> = self.last_talked.keys().copied().collect();
        for p in self.participants.values_mut() {
            p.distance_m = p
                .rider
                .and_then(|r| peers.iter().find(|q| q.rider == r))
                .map(|q| q.gap_m.abs());
        }
        let mut want: BTreeMap<String, (f32, f32)> = BTreeMap::new();
        let group = self.in_group();
        let listen = self.settings.listen;
        let candidates: Vec<&Participant> = self
            .participants
            .values()
            .filter(|p| !p.muted_by_me && !p.blocked_by_me && !p.apart)
            .filter(|p| group || (p.may_hear && (listen == Listen::Everybody || p.selected)))
            .collect();
        let pct = |identity: &str| {
            self.participants
                .get(identity)
                .map_or(riders::DEFAULT_VOLUME_PCT, |p| p.volume_pct)
        };
        if group {
            for p in &candidates {
                want.insert(
                    p.identity.clone(),
                    (riders::gain(1.0, volume, pct(&p.identity)), 0.0),
                );
            }
        } else {
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
            for (r, pl) in spatial::select(&known, &current, &talking, &self.spatial) {
                let identity = &by_rider[&r].identity;
                let (gain, pan) = if self.settings.spatial {
                    (pl.gain, pl.pan)
                } else {
                    (1.0, 0.0)
                };
                want.insert(
                    identity.clone(),
                    (riders::gain(gain, volume, pct(identity)), pan),
                );
            }
            // Riders the server lets the rider hear whose place is not
            // known yet: quiet, while there is room.
            for p in &candidates {
                if want.len() >= self.spatial.max_voices {
                    break;
                }
                let placed = p.rider.is_some_and(|r| known.iter().any(|k| k.rider == r));
                if !placed && !want.contains_key(&p.identity) {
                    let gain = if self.settings.spatial {
                        spatial::UNPLACED_GAIN
                    } else {
                        1.0
                    };
                    want.insert(
                        p.identity.clone(),
                        (riders::gain(gain, volume, pct(&p.identity)), 0.0),
                    );
                }
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

    /// *Who hears me: All nearby* (the tests' usual case; the default is
    /// no one).
    fn nearby() -> VoiceSettings {
        VoiceSettings {
            heard_by: HeardBy::Nearby,
            ..Default::default()
        }
    }

    fn hearing(may_hear: &[u32], heard_by: &[u32]) -> Option<Hearing> {
        Some(Hearing {
            may_hear: may_hear.iter().copied().map(RiderId).collect(),
            heard_by: heard_by.iter().copied().map(RiderId).collect(),
        })
    }

    /// Brings a controller into a connected call with riders 2 and 3, the
    /// game server letting all of them hear each other.
    fn connected() -> (VoiceController, Shared) {
        let (mut c, b) = controller(nearby(), MuteState::default());
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
        c.set_hearing(hearing(&[2, 3], &[2, 3]));
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

    fn prefs(entries: &[(&str, u16, bool)]) -> RiderPrefs {
        entries
            .iter()
            .map(|(k, v, b)| {
                (
                    (*k).to_owned(),
                    riders::RiderPref {
                        name: k.rsplit('/').next().unwrap().to_owned(),
                        volume_pct: *v,
                        blocked: *b,
                        selected: false,
                    },
                )
            })
            .collect()
    }

    /// A rider's volume (spec §7.1.7 Q9) on top of the spatial gain and
    /// the voice volume, matched by name on this server, by account first.
    #[test]
    fn a_riders_volume_scales_its_gain() {
        let (mut c, b) = connected();
        c.set_settings(VoiceSettings {
            volume: 0.5,
            ..nearby()
        });
        c.set_rider_identities(
            "ride.example.org",
            [(RiderId(2), "Bea Climber".to_owned())].into(),
            [(RiderId(3), "acc-3".to_owned())].into(),
        );
        c.set_rider_prefs(prefs(&[
            ("name:ride.example.org/Bea Climber", 200, false),
            ("acct:acc-3", 0, false),
            // Another server's Bea: not this one.
            ("name:other.example.org/Bea Climber", 10, false),
        ]));
        c.update(1.0, &[peer(2, 5.0), peer(3, 5.0)]);
        fn of(c: &VoiceController, id: &str) -> Participant {
            c.participants().find(|p| p.identity == id).unwrap().clone()
        }
        assert_eq!(of(&c, "rider-2").volume_pct, 200);
        assert_eq!(of(&c, "rider-2").heard, Some(1.0), "1 × 0.5 × 200 %");
        assert_eq!(of(&c, "rider-3").volume_pct, 0);
        assert_eq!(of(&c, "rider-3").heard, Some(0.0), "subscribed, silent");
        // Further away: the spatial gain times the rider's volume.
        c.update(2.0, &[peer(2, 40.0), peer(3, 5.0)]);
        let spatial = SpatialConfig::default().gain(40.0);
        let heard = of(&c, "rider-2").heard.unwrap();
        assert!((heard - spatial * 0.5 * 2.0).abs() < 1e-6, "{heard}");
        // Without spatial voice too.
        c.set_settings(VoiceSettings {
            volume: 1.0,
            spatial: false,
            ..nearby()
        });
        c.update(3.0, &[peer(2, 40.0), peer(3, 5.0)]);
        assert_eq!(of(&c, "rider-2").heard, Some(2.0));
        // Nothing was blocked: the permissions stay as the server said.
        assert!(
            !b.take_sent()
                .iter()
                .any(|s| matches!(s, BackendCommand::SetSubscribers(_)))
        );
    }

    /// Block (spec §7.1.7 Q9): the blocked rider is not heard, and the
    /// own microphone is allowed to everybody else only; unblocking
    /// allows everybody again.
    #[test]
    fn a_blocked_rider_is_neither_heard_nor_allowed_to_hear() {
        let (mut c, b) = connected();
        c.set_rider_identities(
            "local",
            [
                (RiderId(2), "Bea Climber".to_owned()),
                (RiderId(3), "Cleo Sprinter".to_owned()),
            ]
            .into(),
            Default::default(),
        );
        c.update(1.0, &[peer(2, 5.0), peer(3, 5.0)]);
        b.take_sent();
        c.set_rider_prefs(prefs(&[("name:local/Bea Climber", 100, true)]));
        c.update(1.1, &[peer(2, 5.0), peer(3, 5.0)]);
        let sent = b.take_sent();
        assert!(sent.contains(&BackendCommand::Subscribe {
            identity: "rider-2".into(),
            on: false
        }));
        assert!(
            sent.contains(&BackendCommand::SetSubscribers(Some(vec![
                "rider-3".into()
            ]))),
            "{sent:?}"
        );
        let bea = c.participants().find(|p| p.identity == "rider-2").unwrap();
        assert!(bea.blocked_by_me && bea.heard.is_none());
        assert_eq!(c.server_blocks(), (vec![], vec!["Bea Climber".to_owned()]));
        // A newcomer is allowed to hear the rider as soon as the game
        // server says so (in range).
        b.push(BackendEvent::ParticipantJoined {
            identity: "rider-4".into(),
            name: "Dan Descender".into(),
        });
        c.update(1.2, &[peer(2, 5.0), peer(3, 5.0)]);
        assert!(
            !b.take_sent()
                .iter()
                .any(|s| matches!(s, BackendCommand::SetSubscribers(_)))
        );
        // The server lists Bea too (a stale answer); the block wins.
        c.set_hearing(hearing(&[2, 3, 4], &[2, 3, 4]));
        c.update(1.25, &[peer(2, 5.0), peer(3, 5.0)]);
        assert!(
            b.take_sent()
                .contains(&BackendCommand::SetSubscribers(Some(vec![
                    "rider-3".into(),
                    "rider-4".into()
                ])))
        );
        // Unblocked: everybody the server lists again, Bea heard again.
        c.set_rider_prefs(RiderPrefs::new());
        c.update(1.3, &[peer(2, 5.0), peer(3, 5.0)]);
        let sent = b.take_sent();
        assert!(sent.contains(&BackendCommand::SetSubscribers(Some(vec![
            "rider-2".into(),
            "rider-3".into(),
            "rider-4".into()
        ]))));
        assert!(sent.contains(&BackendCommand::Subscribe {
            identity: "rider-2".into(),
            on: true
        }));
        assert!(c.server_blocks().1.is_empty());
    }

    /// The other direction: the game server keeps this rider apart from
    /// one who blocked them (`VoiceApart`) – not heard, not allowed to
    /// hear, without knowing who blocked whom; a new call starts open.
    #[test]
    fn riders_kept_apart_by_the_server_exchange_no_voice() {
        let (mut c, b) = connected();
        c.update(1.0, &[peer(2, 5.0), peer(3, 5.0)]);
        b.take_sent();
        c.set_apart([RiderId(3)].into());
        c.update(1.1, &[peer(2, 5.0), peer(3, 5.0)]);
        let sent = b.take_sent();
        assert!(sent.contains(&BackendCommand::Subscribe {
            identity: "rider-3".into(),
            on: false
        }));
        assert!(sent.contains(&BackendCommand::SetSubscribers(Some(vec![
            "rider-2".into()
        ]))));
        let cleo = c.participants().find(|p| p.identity == "rider-3").unwrap();
        assert!(cleo.apart && !cleo.blocked_by_me);
        // Nothing to tell the server: the block is not this rider's.
        assert_eq!(c.server_blocks(), (vec![], vec![]));
        // The call drops: the backend is reset (the next call's list is
        // sent before it connects).
        b.push(BackendEvent::Disconnected {
            reason: "gone".into(),
        });
        c.update(1.2, &[]);
        assert!(
            b.take_sent()
                .contains(&BackendCommand::SetSubscribers(None))
        );
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
        // Riders the server lets the rider hear whose position is unknown
        // are heard quietly with spatial on.
        let (mut c, _b) = connected();
        c.update(1.0, &[peer(2, 5.0)]);
        let p3 = c
            .participants()
            .find(|p| p.identity == "rider-3")
            .unwrap()
            .heard;
        assert_eq!(p3, Some(spatial::UNPLACED_GAIN));
    }

    #[test]
    fn group_rooms_and_speaking_markers() {
        let (mut c, b) = connected();
        assert!(c.set_group(Some("Bad Name".into())).is_err());
        c.set_group(Some("team-1".into())).unwrap();
        let sent = b.take_sent();
        assert!(sent.contains(&BackendCommand::Disconnect));
        // Every call is joined muted (spec §7.1.7 Q7).
        assert!(sent.contains(&BackendCommand::SetMicMuted(true)));
        assert!(c.mute().mic_muted);
        c.set_mic(Switch::Off);
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

    fn heard(c: &VoiceController) -> Vec<String> {
        c.participants()
            .filter(|p| p.heard.is_some())
            .map(|p| p.identity.clone())
            .collect()
    }

    fn last_subscribers(sent: &[BackendCommand]) -> Option<Option<Vec<String>>> {
        sent.iter().rev().find_map(|c| match c {
            BackendCommand::SetSubscribers(a) => Some(a.clone()),
            _ => None,
        })
    }

    /// Spec §7.1.7 Q6: in the session room the rider hears only whom the
    /// game server allows (`may_hear`), nobody before it said; a rider
    /// beyond 300 m is not heard even if allowed.
    #[test]
    fn the_game_server_decides_whom_the_rider_hears() {
        let (mut c, b) = connected();
        c.set_hearing(None);
        c.update(1.0, &[peer(2, 5.0), peer(3, 5.0)]);
        assert!(heard(&c).is_empty(), "nothing from the server: nobody");
        assert_eq!(last_subscribers(&b.take_sent()), Some(Some(vec![])));
        c.set_hearing(hearing(&[2], &[2, 3]));
        c.update(1.1, &[peer(2, 5.0), peer(3, 5.0)]);
        assert_eq!(heard(&c), ["rider-2"], "rider 3 not allowed (far, private)");
        let three = c.participants().find(|p| p.identity == "rider-3").unwrap();
        assert!(!three.may_hear && three.distance_m == Some(5.0));
        c.set_hearing(hearing(&[2, 3], &[2, 3]));
        c.update(1.2, &[peer(2, 320.0), peer(3, 5.0)]);
        assert_eq!(heard(&c), ["rider-3"], "past 300 m: silent, unsubscribed");
        // A group room is not limited by the server's range.
        c.set_hearing(hearing(&[], &[]));
        c.set_group(Some("team-1".into())).unwrap();
        c.update(2.0, &[]);
        c.on_grant(&grant("ocw-x-group-team-1"), 2.1);
        b.push(BackendEvent::Connected {
            room: "ocw-x-group-team-1".into(),
        });
        for id in ["rider-2", "rider-3"] {
            b.push(BackendEvent::ParticipantJoined {
                identity: id.into(),
                name: id.into(),
            });
        }
        c.update(2.2, &[peer(2, 2000.0), peer(3, 5.0)]);
        assert_eq!(heard(&c), ["rider-2", "rider-3"]);
        assert_eq!(last_subscribers(&b.take_sent()), Some(None), "group: open");
    }

    /// *Who hears me* (spec §7.1.7 Q8): No one (the default) allows
    /// nobody, before the call connects; All nearby the riders the server
    /// lists; Selected riders only the selected among them; a block always
    /// wins.
    #[test]
    fn who_hears_me_sets_the_microphone_permissions() {
        let (mut c, b) = controller(VoiceSettings::default(), MuteState::default());
        assert_eq!(c.settings().heard_by, HeardBy::NoOne);
        c.new_session(true);
        c.set_human_riders(3);
        c.update(0.0, &[]);
        b.take_sent();
        c.on_grant(&grant("ocw-x"), 0.1);
        let sent = b.take_sent();
        let allow = sent
            .iter()
            .position(|s| *s == BackendCommand::SetSubscribers(Some(vec![])))
            .expect("nobody may hear the rider");
        let connect = sent
            .iter()
            .position(|s| matches!(s, BackendCommand::Connect { .. }))
            .unwrap();
        assert!(allow < connect, "set before the microphone is published");
        b.push(BackendEvent::Connected {
            room: "ocw-x".into(),
        });
        for id in ["rider-2", "rider-3", "rider-4"] {
            b.push(BackendEvent::ParticipantJoined {
                identity: id.into(),
                name: id.into(),
            });
        }
        c.set_rider_identities(
            "local",
            [
                (RiderId(2), "Bea Climber".to_owned()),
                (RiderId(3), "Cleo Sprinter".to_owned()),
                (RiderId(4), "Dan Descender".to_owned()),
            ]
            .into(),
            Default::default(),
        );
        c.set_hearing(hearing(&[2, 3, 4], &[2, 3, 4]));
        c.update(0.2, &[]);
        assert!(
            !b.take_sent()
                .iter()
                .any(|s| matches!(s, BackendCommand::SetSubscribers(_))),
            "still no one"
        );
        // All nearby: whom the server lists.
        c.set_settings(nearby());
        c.update(0.3, &[]);
        assert_eq!(
            last_subscribers(&b.take_sent()),
            Some(Some(vec![
                "rider-2".into(),
                "rider-3".into(),
                "rider-4".into()
            ]))
        );
        // Selected riders: Bea and Dan selected, Dan blocked.
        let pick = |selected: bool, blocked: bool, name: &str| riders::RiderPref {
            selected,
            blocked,
            ..riders::RiderPref::new(name)
        };
        c.set_rider_prefs(
            [
                (
                    "name:local/Bea Climber".to_owned(),
                    pick(true, false, "Bea Climber"),
                ),
                (
                    "name:local/Dan Descender".to_owned(),
                    pick(true, true, "Dan Descender"),
                ),
            ]
            .into(),
        );
        c.set_settings(VoiceSettings {
            heard_by: HeardBy::Selected,
            ..Default::default()
        });
        c.update(0.4, &[]);
        assert_eq!(
            last_subscribers(&b.take_sent()),
            Some(Some(vec!["rider-2".into()])),
            "Bea only: Cleo not selected, Dan blocked (a block wins)"
        );
        let a = c.server_audience();
        assert_eq!(a.heard_by(), v1::VoiceHeardBy::Selected);
        assert_eq!(
            a.selected_names,
            ["Bea Climber"],
            "a blocked rider is not sent"
        );
        // No one again.
        c.set_settings(VoiceSettings::default());
        c.update(0.5, &[]);
        assert_eq!(last_subscribers(&b.take_sent()), Some(Some(vec![])));
    }

    /// *Voice room: Selected riders* (spec §7.1.3): only the selected
    /// riders are heard (within range, as the server allows).
    #[test]
    fn voice_room_selected_hears_only_the_selection() {
        let (mut c, _b) = connected();
        c.set_rider_identities(
            "local",
            [
                (RiderId(2), "Bea Climber".to_owned()),
                (RiderId(3), "Cleo Sprinter".to_owned()),
            ]
            .into(),
            Default::default(),
        );
        c.set_rider_prefs(
            [(
                "name:local/Cleo Sprinter".to_owned(),
                riders::RiderPref {
                    selected: true,
                    ..riders::RiderPref::new("Cleo Sprinter")
                },
            )]
            .into(),
        );
        c.update(1.0, &[peer(2, 5.0), peer(3, 5.0)]);
        assert_eq!(heard(&c), ["rider-2", "rider-3"], "everybody");
        c.set_settings(VoiceSettings {
            listen: Listen::Selected,
            ..nearby()
        });
        c.update(1.1, &[peer(2, 5.0), peer(3, 5.0)]);
        assert_eq!(heard(&c), ["rider-3"]);
        assert_eq!(c.server_audience().listen(), v1::VoiceListen::Selected);
        // Out of range the selection is not heard either.
        c.update(1.2, &[peer(2, 5.0), peer(3, 400.0)]);
        assert!(heard(&c).is_empty());
    }

    /// The cap (spec §7.1.2): riders who talk come first, and keep their
    /// place for a moment after they stop.
    #[test]
    fn talking_riders_come_first_in_the_cap() {
        let (mut c, b) = connected();
        c.set_settings(VoiceSettings {
            max_voices: 1,
            ..nearby()
        });
        c.update(1.0, &[peer(2, 50.0), peer(3, 5.0)]);
        assert_eq!(heard(&c), ["rider-3"], "the nearest");
        b.push(BackendEvent::Speaking(vec!["rider-2".into()]));
        c.update(1.1, &[peer(2, 50.0), peer(3, 5.0)]);
        assert_eq!(heard(&c), ["rider-2"], "the talking one");
        b.push(BackendEvent::Speaking(vec![]));
        c.update(
            1.2 + TALKED_RECENTLY_S - 0.5,
            &[peer(2, 50.0), peer(3, 5.0)],
        );
        assert_eq!(heard(&c), ["rider-2"], "a pause keeps the place");
        c.update(
            1.2 + TALKED_RECENTLY_S + 0.5,
            &[peer(2, 50.0), peer(3, 5.0)],
        );
        assert_eq!(heard(&c), ["rider-3"]);
        assert_eq!(VoiceSettings::default().max_voices, 24);
    }

    /// Spec §7.1.7 Q7: every shared ride is joined muted – the apps start
    /// muted and a call that ends mutes the next; unmuting holds within the
    /// call.
    #[test]
    fn every_call_is_joined_muted() {
        let (mut c, b) = controller(nearby(), MuteState::joining(false));
        assert!(c.mute().mic_muted && !c.mute().speaker_muted);
        assert!(b.take_sent().contains(&BackendCommand::SetMicMuted(true)));
        c.set_mic(Switch::Off);
        let (mut c, b) = connected();
        assert!(!c.mute().mic_muted, "unmuted in this call");
        c.new_session(true);
        assert!(c.mute().mic_muted, "the next ride starts muted");
        assert!(b.take_sent().contains(&BackendCommand::SetMicMuted(true)));
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

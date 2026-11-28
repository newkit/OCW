//! The voice status panel of the HUD (spec 7.1.13): a few short rows, top
//! right, that say at a glance how the voice call stands –
//!
//! - **the call:** not in a call / connecting / connected (N) /
//!   reconnecting / failed (the reason, short), retrying;
//! - **my microphone:** on or off (the X mouth while off, with the key
//!   that turns it on), whom it reaches (everybody / selected N) and a
//!   calm speaking pulse while the call picks the own voice up;
//! - **I hear:** everybody / selected / a group / nobody (headphones) /
//!   voices muted (Shift+M);
//! - **who speaks now:** up to [`MAX_SPEAKERS`] names (selected riders
//!   first, then the nearest), "+N" for the rest, each with the pulse;
//!   headphones behind a rider who cannot hear me (the sign their avatar
//!   wears).
//!
//! All content is a [`VoicePanelView`], written by the app with the rest
//! of [`crate::voice::VoiceHud`] (`ocw_client::voice::panel_view`); this
//! module only folds and draws it.
//!
//! - **Where:** a child of the HUD's top-right column
//!   ([`crate::hud::HudTopRight`]), after the printed lines (the time
//!   stamp, REC, the app message, the Strava badge and notice, the FPS
//!   counter) and before the photo preview. The column is a flex column
//!   aligned to the right edge, so the panel is right-aligned, moves down
//!   when those lines appear, grow or wrap, and never overlaps them; it
//!   scales with the HUD size (`UiScale`) and lies inside the safe area
//!   like the column. The panels that have a fixed place further down the
//!   right edge (the event panel, the inspection panel) move below it
//!   ([`BelowVoicePanel`]).
//! - **Calm:** hidden without voice (no voice server, voice unavailable,
//!   a solo ride); one dim line while waiting for other riders. With
//!   `compact` (the default) the panel folds to icons (my microphone, I
//!   hear, the riders in the call) [`OPEN_S`] seconds after the last
//!   change and [`SPEECH_HOLD_S`] after the last word, and opens on a
//!   change or speech; `full` keeps the rows; `off` hides it
//!   (`hud.voice off|compact|full`, setting `display.voice_hud`). Nothing
//!   flashes: the pulse is three bars moving slowly, still with reduced
//!   motion.
//! - **Display only:** taps on it do nothing (a tap at the right side
//!   steers; a stray one must never open the microphone).

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

use crate::access::{Accessibility, status_rgb};
use crate::voice::VoiceHud;

/// Speakers named at most; the rest is "+N".
pub const MAX_SPEAKERS: usize = 3;
/// A name's longest (characters; longer ones are cut with an ellipsis).
pub const MAX_NAME: usize = 14;
/// A failure's reason at most (characters).
pub const MAX_REASON: usize = 22;
/// How long the compact panel stays open after a change (s).
pub const OPEN_S: f64 = 6.0;
/// … and after the last word spoken (s).
pub const SPEECH_HOLD_S: f64 = 3.0;
/// An icon's side (UI px) and its texture's (px).
pub const ICON_PX: f32 = 16.0;
const ICON_TEX: u32 = 32;
/// The gap between the panel and what is moved below it (UI px).
pub const GAP_BELOW_PX: f32 = 6.0;

/// The call, as the panel says it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum CallState {
    /// No voice at all (no voice server, unavailable, a solo local ride):
    /// the panel is hidden.
    #[default]
    NoVoice,
    /// Voice is there but no call yet (alone on the server).
    Idle,
    Connecting,
    /// In the call with this many others.
    Connected {
        others: usize,
    },
    Reconnecting,
    /// The last attempt failed (why); retrying.
    Failed {
        reason: String,
    },
}

/// Whom *My voice* reaches while it is on.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Audience {
    #[default]
    Everybody,
    /// The selected riders (how many).
    Selected(usize),
}

/// *I hear*.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Hearing {
    #[default]
    Everybody,
    /// The selected riders: how many, and how many of them are here.
    Selected {
        selected: usize,
        here: usize,
    },
    Group(String),
    Nobody,
    /// Voices muted (Shift+M) – whatever *I hear* says.
    SpeakerMuted,
}

/// A rider speaking now.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Speaker {
    pub name: String,
    /// They cannot hear me (their avatar wears headphones for me).
    pub cannot_hear_me: bool,
}

/// What the panel shows.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoicePanelView {
    pub call: CallState,
    /// *My voice* is on (the microphone is open).
    pub mic_on: bool,
    pub audience: Audience,
    /// The call picks the own voice up.
    pub self_speaking: bool,
    /// The microphone's level (0 … 1, linear) where the backend gives one
    /// (the echo test); `None`: only "speaking".
    pub level: Option<f32>,
    pub hearing: Hearing,
    /// Everyone speaking now, in the order shown (selected riders first,
    /// then the nearest).
    pub speakers: Vec<Speaker>,
}

/// Cuts `s` to `n` characters (an ellipsis when cut).
pub fn fit(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

impl VoicePanelView {
    /// Whether there is anything to show.
    pub fn visible(&self) -> bool {
        self.call != CallState::NoVoice
    }

    fn in_call(&self) -> bool {
        matches!(
            self.call,
            CallState::Connected { .. } | CallState::Reconnecting
        )
    }

    /// The call's row.
    pub fn call_text(&self) -> String {
        match &self.call {
            CallState::NoVoice => String::new(),
            CallState::Idle => "voice: not in a call".into(),
            CallState::Connecting => "connecting".into(),
            CallState::Connected { others } => format!("connected ({others})"),
            CallState::Reconnecting => "reconnecting".into(),
            CallState::Failed { reason } if reason.trim().is_empty() => "failed, retrying".into(),
            CallState::Failed { reason } => {
                format!("failed: {}, retrying", fit(reason.trim(), MAX_REASON))
            }
        }
    }

    /// The microphone's row; `key` turns it on ("M"; empty: no hint).
    pub fn mic_text(&self, key: &str) -> String {
        let who = match self.audience {
            Audience::Everybody => "everybody".to_owned(),
            Audience::Selected(n) => format!("selected {n}"),
        };
        match (self.mic_on, key.is_empty()) {
            (true, _) => format!("on - {who}"),
            (false, true) => format!("off - {who}"),
            (false, false) => format!("off ({key}) - {who}"),
        }
    }

    /// The *I hear* row.
    pub fn hear_text(&self) -> String {
        match &self.hearing {
            Hearing::Everybody => "everybody".into(),
            Hearing::Selected { selected, here } => format!("selected {selected} ({here} here)"),
            Hearing::Group(g) => format!("group {}", fit(g, MAX_NAME)),
            Hearing::Nobody => "nobody".into(),
            Hearing::SpeakerMuted => "muted (Shift+M)".into(),
        }
    }

    /// Whether the headphones stand for *I hear* (I hear no one).
    pub fn hears_no_one(&self) -> bool {
        matches!(self.hearing, Hearing::Nobody | Hearing::SpeakerMuted)
    }

    /// The speakers named and how many more speak.
    pub fn named(&self) -> (&[Speaker], usize) {
        let n = self.speakers.len().min(MAX_SPEAKERS);
        (&self.speakers[..n], self.speakers.len() - n)
    }

    /// One short line (at most [`SHORT_MAX`] characters) for the VR bike
    /// computer's status row: "MIC off (3)", "MIC on (3) Bea+2" (the first
    /// speaker's first name and how many more speak), "voice failed";
    /// empty without a call.
    pub fn short(&self) -> String {
        let mic = if self.mic_on { "MIC on" } else { "MIC off" };
        let s = match &self.call {
            CallState::NoVoice | CallState::Idle => String::new(),
            CallState::Connecting => "voice joining".into(),
            CallState::Reconnecting => "voice lost".into(),
            CallState::Failed { .. } => "voice failed".into(),
            CallState::Connected { others } => {
                let mut s = format!("{mic} ({others})");
                if let Some(first) = self.speakers.first() {
                    let name = first.name.split_whitespace().next().unwrap_or_default();
                    s.push_str(&format!(" {name}"));
                    if self.speakers.len() > 1 {
                        s.push_str(&format!("+{}", self.speakers.len() - 1));
                    }
                }
                s
            }
        };
        fit(&s, SHORT_MAX)
    }
}

/// The short line's longest ([`VoicePanelView::short`]).
pub const SHORT_MAX: usize = 16;

/// How the panel is shown (setting `display.voice_hud`, `hud.voice`).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VoicePanelMode {
    Off,
    /// Folds to icons when nothing changes and nobody speaks.
    #[default]
    Compact,
    /// Always the rows.
    Full,
}

impl VoicePanelMode {
    pub const NAMES: [&'static str; 3] = ["off", "compact", "full"];

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "off" => Some(Self::Off),
            "compact" | "on" => Some(Self::Compact),
            "full" => Some(Self::Full),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Compact => "compact",
            Self::Full => "full",
        }
    }

    /// Off → compact → full → off (the command without a word).
    pub fn next(self) -> Self {
        match self {
            Self::Off => Self::Compact,
            Self::Compact => Self::Full,
            Self::Full => Self::Off,
        }
    }
}

/// A panel view shown instead of the call's (`dev.voice_hud`: scenes and
/// tests without a voice server).
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct FakeVoicePanel(pub Option<VoicePanelView>);

/// The states of `dev.voice_hud`.
pub const DEMO_STATES: [&str; 9] = [
    "none",
    "idle",
    "connecting",
    "mic-off",
    "speaking",
    "others",
    "nobody",
    "failed",
    "reconnecting",
];

/// A made-up panel for scenes and tests (`dev.voice_hud STATE`); `None`
/// for "none" (the call's own again) and unknown words.
pub fn demo(state: &str) -> Option<VoicePanelView> {
    let speaker = |name: &str, deaf: bool| Speaker {
        name: name.into(),
        cannot_hear_me: deaf,
    };
    let connected = VoicePanelView {
        call: CallState::Connected { others: 4 },
        ..Default::default()
    };
    Some(match state {
        "idle" => VoicePanelView {
            call: CallState::Idle,
            ..Default::default()
        },
        "connecting" => VoicePanelView {
            call: CallState::Connecting,
            ..Default::default()
        },
        "mic-off" => connected,
        "speaking" => VoicePanelView {
            mic_on: true,
            self_speaking: true,
            audience: Audience::Selected(2),
            hearing: Hearing::Selected {
                selected: 2,
                here: 1,
            },
            ..connected
        },
        "others" => VoicePanelView {
            mic_on: true,
            speakers: vec![
                speaker("Bea Climber", false),
                speaker("Cleo Sprinter", true),
                speaker("Dana Rouleur", false),
                speaker("Eli Domestique", false),
                speaker("Fay Puncheur", false),
            ],
            call: CallState::Connected { others: 6 },
            ..Default::default()
        },
        "nobody" => VoicePanelView {
            mic_on: true,
            hearing: Hearing::Nobody,
            ..connected
        },
        "failed" => VoicePanelView {
            call: CallState::Failed {
                reason: "server unreachable".into(),
            },
            ..Default::default()
        },
        "reconnecting" => VoicePanelView {
            call: CallState::Reconnecting,
            mic_on: true,
            ..Default::default()
        },
        _ => return None,
    })
}

/// What changes open the compact panel: everything but who speaks.
type FoldKey = (CallState, bool, Audience, Hearing);

/// The compact panel's fold: open after a change and while someone
/// speaks, icons after that.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fold {
    key: Option<FoldKey>,
    open_until: f64,
}

impl Fold {
    /// Whether the rows show at `now` (s) for `v`.
    pub fn open(&mut self, v: &VoicePanelView, now: f64) -> bool {
        let key = (v.call.clone(), v.mic_on, v.audience, v.hearing.clone());
        if self.key.as_ref() != Some(&key) {
            self.key = Some(key);
            self.open_until = now + OPEN_S;
        }
        if v.self_speaking || !v.speakers.is_empty() {
            self.open_until = self.open_until.max(now + SPEECH_HOLD_S);
        }
        // Joining, a lost or failed call: always said in words.
        !matches!(v.call, CallState::Connected { .. }) || now < self.open_until
    }
}

/// Which parts show.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Shape {
    #[default]
    Hidden,
    /// One dim line: not in a call.
    Idle,
    /// Icons only: my microphone, I hear, the riders in the call.
    Icons,
    /// The rows.
    Rows,
}

/// The panel's shape for `v` in `mode` (`open`: [`Fold::open`]).
pub fn shape(v: &VoicePanelView, mode: VoicePanelMode, open: bool) -> Shape {
    match (mode, &v.call) {
        (VoicePanelMode::Off, _) | (_, CallState::NoVoice) => Shape::Hidden,
        (_, CallState::Idle) => Shape::Idle,
        (VoicePanelMode::Full, _) => Shape::Rows,
        (VoicePanelMode::Compact, _) if open => Shape::Rows,
        (VoicePanelMode::Compact, _) => Shape::Icons,
    }
}

/// The icons (white on clear, tinted by the row's tone).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Icon {
    /// The call: a dot with waves.
    Call,
    Mic,
    /// The X over the mouth: my voice is off (the avatar's sign).
    MouthX,
    /// A speaker with waves: I hear.
    Ear,
    /// Headphones: hears no one / cannot hear me (the avatar's sign).
    Headphones,
}

impl Icon {
    const ALL: [Icon; 5] = [
        Icon::Call,
        Icon::Mic,
        Icon::MouthX,
        Icon::Ear,
        Icon::Headphones,
    ];

    fn index(self) -> usize {
        Self::ALL.iter().position(|i| *i == self).unwrap_or(0)
    }
}

/// A row's colour.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Tone {
    #[default]
    Normal,
    Dim,
    /// On, speaking.
    Live,
    /// Off: the microphone, hearing no one.
    Muted,
    /// Reconnecting, failed.
    Warn,
}

const LIVE: [f32; 3] = [0.45, 1.0, 0.55];
const MUTED: [f32; 3] = [1.0, 0.35, 0.3];
const WARN: [f32; 3] = [0.98, 0.76, 0.18];

impl Tone {
    fn rgb(self) -> [f32; 3] {
        match self {
            Tone::Normal => [1.0, 1.0, 1.0],
            Tone::Dim => [0.72, 0.72, 0.72],
            Tone::Live => LIVE,
            Tone::Muted => MUTED,
            Tone::Warn => WARN,
        }
    }

    fn color(self) -> Color {
        let [r, g, b] = self.rgb();
        Color::srgb(r, g, b)
    }
}

/// One row as drawn: `[icon] [bars] [icon2] text [trail]`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowLook {
    pub show: bool,
    pub icon: Option<(Icon, Tone)>,
    /// The speaking pulse.
    pub bars: bool,
    pub icon2: Option<(Icon, Tone)>,
    pub text: String,
    pub tone: Tone,
    pub trail: Option<(Icon, Tone)>,
}

/// The rows: the call, my microphone, I hear, the speakers, "+N", and the
/// folded panel's icons.
pub const ROW_CALL: usize = 0;
pub const ROW_MIC: usize = 1;
pub const ROW_HEAR: usize = 2;
pub const ROW_SPEAKER: usize = 3;
pub const ROW_MORE: usize = ROW_SPEAKER + MAX_SPEAKERS;
pub const ROW_ICONS: usize = ROW_MORE + 1;
pub const ROWS: usize = ROW_ICONS + 1;

/// Everything the panel draws (pure, tested): the rows of `v` in `shape`;
/// `key` is the microphone's key.
pub fn looks(v: &VoicePanelView, shape: Shape, key: &str) -> Vec<RowLook> {
    let mut rows = vec![RowLook::default(); ROWS];
    let mic = if v.mic_on {
        (Icon::Mic, Tone::Live)
    } else {
        (Icon::MouthX, Tone::Muted)
    };
    let hear = if v.hears_no_one() {
        (Icon::Headphones, Tone::Muted)
    } else {
        (Icon::Ear, Tone::Normal)
    };
    let speaking = v.mic_on && v.self_speaking && v.in_call();
    match shape {
        Shape::Hidden => {}
        Shape::Idle => {
            rows[ROW_CALL] = RowLook {
                show: true,
                icon: Some((Icon::Call, Tone::Dim)),
                text: v.call_text(),
                tone: Tone::Dim,
                ..Default::default()
            };
        }
        Shape::Icons => {
            let others = match v.call {
                CallState::Connected { others } => others.to_string(),
                _ => String::new(),
            };
            rows[ROW_ICONS] = RowLook {
                show: true,
                icon: Some(mic),
                bars: speaking,
                icon2: Some(hear),
                text: others,
                tone: Tone::Normal,
                trail: None,
            };
        }
        Shape::Rows => {
            let tone = match v.call {
                CallState::Connected { .. } => Tone::Normal,
                CallState::Connecting => Tone::Dim,
                _ => Tone::Warn,
            };
            rows[ROW_CALL] = RowLook {
                show: true,
                icon: Some((Icon::Call, tone)),
                text: v.call_text(),
                tone,
                ..Default::default()
            };
            rows[ROW_MIC] = RowLook {
                show: true,
                icon: Some(mic),
                bars: speaking,
                text: v.mic_text(key),
                // White words (red ones are hard to read over a bright
                // sky); the X mouth is red.
                tone: Tone::Normal,
                ..Default::default()
            };
            rows[ROW_HEAR] = RowLook {
                show: true,
                icon: Some(hear),
                text: v.hear_text(),
                tone: Tone::Normal,
                ..Default::default()
            };
            let (named, more) = v.named();
            for (i, s) in named.iter().enumerate() {
                rows[ROW_SPEAKER + i] = RowLook {
                    show: true,
                    bars: true,
                    text: fit(&s.name, MAX_NAME),
                    tone: Tone::Normal,
                    trail: s.cannot_hear_me.then_some((Icon::Headphones, Tone::Muted)),
                    ..Default::default()
                };
            }
            if more > 0 {
                rows[ROW_MORE] = RowLook {
                    show: true,
                    text: format!("+{more}"),
                    tone: Tone::Dim,
                    ..Default::default()
                };
            }
        }
    }
    rows
}

/// Whether the panel `panel` lies clear under the column's printed lines
/// `lines` inside the column `column` (`[left, top, width, height]`, px):
/// below every line, its right edge on the column's, inside the column.
/// `Err` says what is wrong (the browser's and the console's layout check).
pub fn clear_below(panel: [f32; 4], lines: &[[f32; 4]], column: [f32; 4]) -> Result<(), String> {
    const EPS: f32 = 1.0;
    let (right, bottom) = (panel[0] + panel[2], panel[1] + panel[3]);
    for l in lines {
        if panel[1] + EPS < l[1] + l[3] {
            return Err(format!(
                "the panel (top {:.0}) overlaps a line ending at {:.0}",
                panel[1],
                l[1] + l[3]
            ));
        }
    }
    let (col_right, col_bottom) = (column[0] + column[2], column[1] + column[3]);
    if (right - col_right).abs() > EPS {
        return Err(format!(
            "the panel's right edge {right:.0} is not the column's {col_right:.0}"
        ));
    }
    if panel[0] + EPS < column[0] || panel[1] + EPS < column[1] || bottom > col_bottom + EPS {
        return Err("the panel leaves the column".into());
    }
    Ok(())
}

/// The microphone's key in the panel's hint (the input mapping's,
/// `ocw_ui::bindings`; "M" by default, empty: unbound).
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct VoiceKeys {
    pub mic: String,
}

impl Default for VoiceKeys {
    fn default() -> Self {
        Self { mic: "M".into() }
    }
}

/// What the panel shows now, for tests and the browser's checks.
#[derive(Resource, Debug, Clone, Default, PartialEq)]
pub struct VoicePanelInfo {
    pub shape: Shape,
    /// The texts of the rows shown, top to bottom.
    pub rows: Vec<String>,
}

/// A HUD box with a fixed place under the top-right column: kept below
/// the voice panel while that reaches further down (its `top` otherwise:
/// UI px, or a fraction of the UI's height).
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum BelowVoicePanel {
    Px(f32),
    Fraction(f32),
}

impl BelowVoicePanel {
    /// The box's top (UI px) in a UI `ui_h` high when the voice panel
    /// ends at `panel_bottom` (`None`: not shown).
    pub fn top(self, ui_h: f32, panel_bottom: Option<f32>) -> f32 {
        let base = match self {
            Self::Px(p) => p,
            Self::Fraction(f) => f * ui_h,
        };
        panel_bottom.map_or(base, |b| base.max(b + GAP_BELOW_PX))
    }
}

pub struct VoicePanelPlugin;

impl Plugin for VoicePanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VoiceHud>()
            .init_resource::<VoicePanelMode>()
            .init_resource::<FakeVoicePanel>()
            .init_resource::<VoiceKeys>()
            .init_resource::<VoicePanelInfo>()
            .add_systems(Update, update_panel)
            .add_systems(PostUpdate, keep_below);
    }
}

/// The panel's root.
#[derive(Component)]
pub struct VoicePanelRoot;

#[derive(Component)]
struct Row(usize);

/// A row's parts.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Part {
    Icon,
    Icon2,
    Trail,
}

#[derive(Component)]
struct RowIcon(usize, Part);

#[derive(Component)]
struct RowBars(usize);

/// One bar of a row's pulse (row, bar).
#[derive(Component)]
struct Bar(usize, usize);

#[derive(Component)]
struct RowText(usize);

/// The panel in the top-right column (called by the HUD after the printed
/// lines).
pub(crate) fn spawn(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                display: Display::None,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                row_gap: px(3),
                padding: UiRect::axes(px(8), px(5)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.55)),
            VoicePanelRoot,
            crate::hud::HudPanel,
            Pickable::IGNORE,
        ))
        .with_children(|p| {
            for row in 0..ROWS {
                p.spawn((
                    Node {
                        display: Display::None,
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        ..default()
                    },
                    Row(row),
                ))
                .with_children(|r| {
                    let icon = |r: &mut ChildSpawnerCommands, part: Part| {
                        r.spawn((
                            Node {
                                display: Display::None,
                                width: px(ICON_PX),
                                height: px(ICON_PX),
                                flex_shrink: 0.0,
                                ..default()
                            },
                            ImageNode::default(),
                            RowIcon(row, part),
                        ));
                    };
                    icon(r, Part::Icon);
                    r.spawn((
                        Node {
                            display: Display::None,
                            width: px(13),
                            height: px(12),
                            flex_shrink: 0.0,
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::SpaceBetween,
                            ..default()
                        },
                        RowBars(row),
                    ))
                    .with_children(|b| {
                        for i in 0..3 {
                            b.spawn((
                                Node {
                                    width: px(3),
                                    height: px(6),
                                    border_radius: BorderRadius::all(px(1)),
                                    ..default()
                                },
                                BackgroundColor(Tone::Live.color()),
                                Bar(row, i),
                            ));
                        }
                    });
                    icon(r, Part::Icon2);
                    r.spawn((
                        Text::new(""),
                        TextFont {
                            font_size: 14.0.into(),
                            ..default()
                        },
                        TextColor(Color::WHITE),
                        RowText(row),
                    ));
                    icon(r, Part::Trail);
                });
            }
        });
}

/// A bar's height (UI px) at `t` s: slow, each bar on its own phase;
/// with a level (0 … 1) scaled by it; `still`: no motion.
pub fn bar_height(bar: usize, t: f64, level: Option<f32>, still: bool) -> f32 {
    const MIN: f32 = 4.0;
    const MAX: f32 = 12.0;
    let wave = if still {
        [0.5, 0.9, 0.6][bar % 3]
    } else {
        // 1.1 – 1.6 Hz: a calm pulse, no flashing.
        let f = [1.1, 1.6, 1.3][bar % 3];
        (0.5 + 0.5 * (t * f * std::f64::consts::TAU + bar as f64 * 1.7).sin()) as f32
    };
    let k = level.map_or(1.0, |l| crate::voice::meter_fraction(l).max(0.25));
    MIN + (MAX - MIN) * wave * k
}

fn icon_image(icon: Icon) -> Image {
    Image::new(
        Extent3d {
            width: ICON_TEX,
            height: ICON_TEX,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        icon_pixels(icon, ICON_TEX),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// Whether the unit-square point is inside the icon's shape.
fn inside(icon: Icon, x: f32, y: f32) -> bool {
    let seg = |ax: f32, ay: f32, bx: f32, by: f32, r: f32| {
        let (dx, dy) = (bx - ax, by - ay);
        let t = (((x - ax) * dx + (y - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
        (x - ax - t * dx).hypot(y - ay - t * dy) <= r
    };
    let ring = |cx: f32, cy: f32, r: f32, w: f32| ((x - cx).hypot(y - cy) - r).abs() <= w / 2.0;
    let rect = |x0: f32, y0: f32, x1: f32, y1: f32| x >= x0 && x <= x1 && y >= y0 && y <= y1;
    match icon {
        Icon::Call => {
            let (dx, dy) = (x - 0.5, y - 0.78);
            let up = dy < 0.0 && dx.abs() <= -dy * 1.1;
            dx.hypot(dy) <= 0.1
                || (up && (ring(0.5, 0.78, 0.3, 0.085) || ring(0.5, 0.78, 0.52, 0.085)))
        }
        Icon::Mic => {
            seg(0.5, 0.2, 0.5, 0.46, 0.14)
                || (y > 0.46 && ring(0.5, 0.46, 0.27, 0.07))
                || rect(0.465, 0.73, 0.535, 0.88)
                || seg(0.34, 0.9, 0.66, 0.9, 0.035)
        }
        Icon::MouthX => {
            // The lips, and the X across them.
            let (ex, ey) = ((x - 0.5) / 0.42, (y - 0.5) / 0.24);
            let d = ex.hypot(ey);
            let lips = (0.72..=1.0).contains(&d);
            lips || seg(0.16, 0.16, 0.84, 0.84, 0.05) || seg(0.84, 0.16, 0.16, 0.84, 0.05)
        }
        Icon::Ear => {
            let (dx, dy) = (x - 0.42, y - 0.5);
            let cone = rect(0.1, 0.38, 0.26, 0.62)
                || ((0.26..=0.44).contains(&x) && dy.abs() <= 0.12 + (x - 0.26) * 1.2);
            let out = dx > 0.0 && dy.abs() <= dx;
            cone || (out && (ring(0.42, 0.5, 0.22, 0.07) || ring(0.42, 0.5, 0.4, 0.07)))
        }
        Icon::Headphones => {
            (y < 0.56 && ring(0.5, 0.56, 0.35, 0.08))
                || seg(0.17, 0.62, 0.17, 0.8, 0.09)
                || seg(0.83, 0.62, 0.83, 0.8, 0.09)
        }
    }
}

/// The icon as `n` × `n` RGBA (white, the shape in the alpha; 4 × 4
/// samples per pixel).
pub fn icon_pixels(icon: Icon, n: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((n * n * 4) as usize);
    for py in 0..n {
        for px in 0..n {
            let mut hit = 0u32;
            for j in 0..4 {
                for i in 0..4 {
                    let x = (px as f32 + (i as f32 + 0.5) / 4.0) / n as f32;
                    let y = (py as f32 + (j as f32 + 0.5) / 4.0) / n as f32;
                    hit += u32::from(inside(icon, x, y));
                }
            }
            out.extend_from_slice(&[255, 255, 255, (hit * 255 / 16) as u8]);
        }
    }
    out
}

/// The panel view in effect: the fake one (`dev.voice_hud`) or the call's.
pub fn current<'a>(hud: &'a VoiceHud, fake: &'a FakeVoicePanel) -> &'a VoicePanelView {
    fake.0.as_ref().unwrap_or(&hud.panel)
}

#[derive(Default)]
struct Drawn {
    rows: Vec<RowLook>,
    palette: Option<crate::access::Palette>,
    icons: Vec<Handle<Image>>,
    fold: Fold,
}

#[allow(clippy::type_complexity, clippy::too_many_arguments)] // a Bevy system: disjoint UI queries
fn update_panel(
    time: Res<Time>,
    hud: Res<VoiceHud>,
    fake: Res<FakeVoicePanel>,
    mode: Res<VoicePanelMode>,
    keys: Res<VoiceKeys>,
    access: Option<Res<Accessibility>>,
    mut images: Option<ResMut<Assets<Image>>>,
    mut info: ResMut<VoicePanelInfo>,
    mut drawn: Local<Drawn>,
    mut root: Query<&mut Node, (With<VoicePanelRoot>, Without<Row>)>,
    mut rows: Query<(&mut Node, &Row), Without<VoicePanelRoot>>,
    mut icons: Query<
        (&mut Node, &mut ImageNode, &RowIcon),
        (Without<VoicePanelRoot>, Without<Row>),
    >,
    mut bars: Query<
        (&mut Node, &RowBars),
        (
            Without<VoicePanelRoot>,
            Without<Row>,
            Without<RowIcon>,
            Without<Bar>,
        ),
    >,
    mut bar: Query<
        (&mut Node, &Bar),
        (
            Without<VoicePanelRoot>,
            Without<Row>,
            Without<RowIcon>,
            Without<RowBars>,
        ),
    >,
    mut texts: Query<(&mut Text, &mut TextColor, &RowText)>,
) {
    let v = current(&hud, &fake);
    let now = time.elapsed_secs_f64();
    let open = drawn.fold.open(v, now);
    let shape = shape(v, *mode, open);
    let want = looks(v, shape, &keys.mic);
    let access = access.map(|a| *a).unwrap_or_default();
    // The pulse moves every frame; the rest only on a change.
    let still = access.reduced_motion;
    for (mut n, b) in &mut bar {
        if want.get(b.0).is_some_and(|r| r.show && r.bars) {
            let level = if b.0 == ROW_MIC || b.0 == ROW_ICONS {
                v.level
            } else {
                None
            };
            let h = px(bar_height(b.0 * 3 + b.1, now, level, still).round());
            if n.height != h {
                n.height = h;
            }
        }
    }
    if drawn.rows == want && drawn.palette == Some(access.palette) {
        return;
    }
    if drawn.icons.is_empty()
        && let Some(images) = images.as_deref_mut()
    {
        drawn.icons = Icon::ALL
            .iter()
            .map(|i| images.add(icon_image(*i)))
            .collect();
    }
    let display = |on: bool| if on { Display::Flex } else { Display::None };
    for mut n in &mut root {
        n.display = display(shape != Shape::Hidden);
    }
    for (mut n, r) in &mut rows {
        n.display = display(want[r.0].show);
    }
    for (mut n, mut image, i) in &mut icons {
        let look = &want[i.0];
        let part = match i.1 {
            Part::Icon => look.icon,
            Part::Icon2 => look.icon2,
            Part::Trail => look.trail,
        };
        n.display = display(part.is_some() && !drawn.icons.is_empty());
        if let Some((icon, tone)) = part {
            if let Some(h) = drawn.icons.get(icon.index()) {
                image.image = h.clone();
            }
            let [r, g, b] = status_rgb(tone.rgb(), access.palette);
            image.color = Color::srgb(r, g, b);
        }
    }
    for (mut n, b) in &mut bars {
        n.display = display(want[b.0].bars);
    }
    for (mut t, mut c, r) in &mut texts {
        let look = &want[r.0];
        if t.0 != look.text {
            t.0.clone_from(&look.text);
        }
        c.0 = look.tone.color();
    }
    info.set_if_neq(VoicePanelInfo {
        shape,
        rows: want
            .iter()
            .filter(|r| r.show)
            .map(|r| r.text.clone())
            .collect(),
    });
    drawn.rows = want;
    drawn.palette = Some(access.palette);
}

/// Keeps the boxes with a fixed place under the column
/// ([`BelowVoicePanel`]) below the voice panel.
#[allow(clippy::type_complexity)] // a Bevy system
fn keep_below(
    windows: Query<&Window, With<bevy::window::PrimaryWindow>>,
    safe: Option<Res<crate::safe_area::SafeArea>>,
    scale: Option<Res<UiScale>>,
    panel: Query<(&ComputedNode, &UiGlobalTransform, &Node), With<VoicePanelRoot>>,
    mut boxes: Query<(&mut Node, &BelowVoicePanel), Without<VoicePanelRoot>>,
) {
    let Ok(w) = windows.single() else { return };
    let k = scale.map_or(1.0, |s| s.0).max(0.1);
    let ui_h = crate::safe_area::ui_area(w, safe.as_deref()).1.y / k;
    let bottom = panel.single().ok().and_then(|(c, t, n)| {
        (n.display != Display::None && c.size().y > 0.0).then(|| {
            let inv = c.inverse_scale_factor();
            (t.translation.y + c.size().y * 0.5) * inv
        })
    });
    for (mut n, b) in &mut boxes {
        let want = px(b.top(ui_h, bottom).round());
        if n.top != want {
            n.top = want;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected(others: usize) -> VoicePanelView {
        VoicePanelView {
            call: CallState::Connected { others },
            ..Default::default()
        }
    }

    fn speakers(names: &[&str]) -> Vec<Speaker> {
        names
            .iter()
            .map(|n| Speaker {
                name: (*n).into(),
                cannot_hear_me: false,
            })
            .collect()
    }

    #[test]
    fn the_call_row_says_each_state() {
        let mut v = VoicePanelView::default();
        assert!(!v.visible(), "no voice: nothing");
        assert_eq!(v.call_text(), "");
        v.call = CallState::Idle;
        assert!(v.visible());
        assert_eq!(v.call_text(), "voice: not in a call");
        v.call = CallState::Connecting;
        assert_eq!(v.call_text(), "connecting");
        v.call = CallState::Connected { others: 3 };
        assert_eq!(v.call_text(), "connected (3)");
        v.call = CallState::Reconnecting;
        assert_eq!(v.call_text(), "reconnecting");
        v.call = CallState::Failed {
            reason: "server unreachable".into(),
        };
        assert_eq!(v.call_text(), "failed: server unreachable, retrying");
        // A long reason is cut; none: just "failed".
        v.call = CallState::Failed {
            reason: "the voice server did not answer the token request in time".into(),
        };
        let t = v.call_text();
        assert!(t.starts_with("failed: the voice server did") && t.ends_with("…, retrying"));
        assert!(t.chars().count() <= "failed: , retrying".len() + MAX_REASON);
        v.call = CallState::Failed { reason: " ".into() };
        assert_eq!(v.call_text(), "failed, retrying");
    }

    #[test]
    fn the_microphone_row_says_on_off_and_the_audience() {
        let mut v = connected(2);
        assert_eq!(v.mic_text("M"), "off (M) - everybody");
        assert_eq!(v.mic_text(""), "off - everybody", "unbound: no hint");
        v.audience = Audience::Selected(2);
        assert_eq!(v.mic_text("M"), "off (M) - selected 2");
        v.mic_on = true;
        assert_eq!(v.mic_text("M"), "on - selected 2", "no hint while on");
        v.audience = Audience::Everybody;
        assert_eq!(v.mic_text("M"), "on - everybody");
        // The X mouth while off, the microphone while on; the pulse only
        // while the call hears the own voice.
        let rows = looks(&v, Shape::Rows, "M");
        assert_eq!(rows[ROW_MIC].icon, Some((Icon::Mic, Tone::Live)));
        assert!(!rows[ROW_MIC].bars);
        v.self_speaking = true;
        assert!(looks(&v, Shape::Rows, "M")[ROW_MIC].bars);
        v.mic_on = false;
        let rows = looks(&v, Shape::Rows, "M");
        assert_eq!(rows[ROW_MIC].icon, Some((Icon::MouthX, Tone::Muted)));
        assert!(!rows[ROW_MIC].bars, "muted: never shown speaking");
        assert_eq!(rows[ROW_MIC].tone, Tone::Normal, "white words");
    }

    #[test]
    fn the_i_hear_row_says_each_mode() {
        let mut v = connected(2);
        assert_eq!(v.hear_text(), "everybody");
        assert_eq!(
            looks(&v, Shape::Rows, "M")[ROW_HEAR].icon,
            Some((Icon::Ear, Tone::Normal))
        );
        v.hearing = Hearing::Selected {
            selected: 3,
            here: 2,
        };
        assert_eq!(v.hear_text(), "selected 3 (2 here)");
        v.hearing = Hearing::Group("Thursday Chain Gang".into());
        assert_eq!(v.hear_text(), "group Thursday Chai…");
        assert!(!v.hears_no_one());
        // Nobody and muted voices: the headphones.
        for (h, text) in [
            (Hearing::Nobody, "nobody"),
            (Hearing::SpeakerMuted, "muted (Shift+M)"),
        ] {
            v.hearing = h;
            assert_eq!(v.hear_text(), text);
            let row = &looks(&v, Shape::Rows, "M")[ROW_HEAR];
            assert_eq!(row.icon, Some((Icon::Headphones, Tone::Muted)));
            assert_eq!(row.tone, Tone::Normal);
        }
    }

    #[test]
    fn up_to_three_speakers_then_plus_n() {
        let mut v = connected(6);
        let rows = looks(&v, Shape::Rows, "M");
        assert!(
            rows[ROW_SPEAKER..=ROW_MORE].iter().all(|r| !r.show),
            "nobody speaks: no rows"
        );
        v.speakers = speakers(&["Bea", "Cleo"]);
        v.speakers[1].cannot_hear_me = true;
        let rows = looks(&v, Shape::Rows, "M");
        assert_eq!(rows[ROW_SPEAKER].text, "Bea");
        assert!(rows[ROW_SPEAKER].bars && rows[ROW_SPEAKER].trail.is_none());
        // Cannot hear me: the headphones, as on the avatar.
        assert_eq!(
            rows[ROW_SPEAKER + 1].trail,
            Some((Icon::Headphones, Tone::Muted))
        );
        assert!(!rows[ROW_SPEAKER + 2].show && !rows[ROW_MORE].show);
        v.speakers = speakers(&["Bea", "Cleo", "Dana", "Eli", "Fay"]);
        let rows = looks(&v, Shape::Rows, "M");
        let shown: Vec<&str> = rows[ROW_SPEAKER..ROW_MORE]
            .iter()
            .map(|r| r.text.as_str())
            .collect();
        assert_eq!(shown, ["Bea", "Cleo", "Dana"], "in the order given");
        assert!(rows[ROW_MORE].show);
        assert_eq!(rows[ROW_MORE].text, "+2");
        assert_eq!(v.named().1, 2);
        // A long name is cut.
        v.speakers = speakers(&["Maximiliana Longname-Rider"]);
        let name = &looks(&v, Shape::Rows, "M")[ROW_SPEAKER].text;
        assert_eq!(name.chars().count(), MAX_NAME);
        assert!(name.ends_with('…'));
    }

    #[test]
    fn the_shape_follows_the_mode_and_the_call() {
        use VoicePanelMode::*;
        let none = VoicePanelView::default();
        let idle = VoicePanelView {
            call: CallState::Idle,
            ..Default::default()
        };
        let call = connected(2);
        for mode in [Off, Compact, Full] {
            assert_eq!(shape(&none, mode, true), Shape::Hidden, "no voice");
            assert_eq!(shape(&call, Off, true), Shape::Hidden);
            if mode != Off {
                assert_eq!(shape(&idle, mode, false), Shape::Idle, "one dim line");
            }
        }
        assert_eq!(shape(&call, Full, false), Shape::Rows);
        assert_eq!(shape(&call, Compact, true), Shape::Rows);
        assert_eq!(shape(&call, Compact, false), Shape::Icons);
        // Hidden: no rows; idle: one dim line; icons: one row.
        assert!(looks(&call, Shape::Hidden, "M").iter().all(|r| !r.show));
        let rows = looks(&idle, Shape::Idle, "M");
        assert_eq!(rows.iter().filter(|r| r.show).count(), 1);
        assert_eq!(rows[ROW_CALL].tone, Tone::Dim);
        let rows = looks(&call, Shape::Icons, "M");
        assert_eq!(rows.iter().filter(|r| r.show).count(), 1);
        let icons = &rows[ROW_ICONS];
        assert_eq!(icons.icon, Some((Icon::MouthX, Tone::Muted)));
        assert_eq!(icons.icon2, Some((Icon::Ear, Tone::Normal)));
        assert_eq!(icons.text, "2", "the riders in the call");
        // The mode's words.
        assert_eq!(VoicePanelMode::default(), Compact, "on by default");
        for m in [Off, Compact, Full] {
            assert_eq!(VoicePanelMode::parse(m.name()), Some(m));
        }
        assert_eq!(VoicePanelMode::parse("on"), Some(Compact));
        assert_eq!(VoicePanelMode::parse("loud"), None);
        assert_eq!(Off.next().next().next(), Off);
    }

    #[test]
    fn the_compact_panel_folds_when_nothing_changes_and_opens_on_change_or_speech() {
        let mut fold = Fold::default();
        let mut v = connected(2);
        assert!(fold.open(&v, 0.0), "the call just came up");
        assert!(fold.open(&v, OPEN_S - 0.1));
        assert!(!fold.open(&v, OPEN_S + 0.1), "folded");
        // A change opens it again.
        v.mic_on = true;
        assert!(fold.open(&v, 20.0));
        assert!(!fold.open(&v, 20.0 + OPEN_S + 0.1));
        // Speech keeps it open, and a little longer.
        v.speakers = speakers(&["Bea"]);
        assert!(fold.open(&v, 40.0));
        assert!(fold.open(&v, 60.0), "still speaking");
        v.speakers.clear();
        assert!(fold.open(&v, 60.0 + SPEECH_HOLD_S - 0.1));
        assert!(!fold.open(&v, 60.0 + SPEECH_HOLD_S + 0.1));
        v.self_speaking = true;
        assert!(fold.open(&v, 80.0));
        v.self_speaking = false;
        // A rider joining is a change.
        v.call = CallState::Connected { others: 3 };
        assert!(fold.open(&v, 100.0));
        // Not connected: always in words.
        v.call = CallState::Failed { reason: "x".into() };
        assert!(fold.open(&v, 500.0) && fold.open(&v, 900.0));
    }

    #[test]
    fn the_panel_lies_clear_under_the_printed_lines() {
        // The column at the right edge of a 1280 px window; lines of
        // different widths, right-aligned, 4 px apart.
        let column = |h: f32| [1000.0, 12.0, 268.0, h];
        let line = |top: f32, w: f32, h: f32| [1268.0 - w, top, w, h];
        let panel = |top: f32| [1268.0 - 180.0, top, 180.0, 70.0];
        // No line: the panel is the column's first box.
        assert_eq!(clear_below(panel(12.0), &[], column(70.0)), Ok(()));
        // One line (the time stamp).
        let one = [line(12.0, 150.0, 18.0)];
        assert_eq!(clear_below(panel(34.0), &one, column(92.0)), Ok(()));
        // Several, one of them wrapped over three lines.
        let many = [
            line(12.0, 150.0, 18.0),
            line(34.0, 90.0, 26.0),
            line(64.0, 268.0, 62.0),
            line(130.0, 70.0, 20.0),
        ];
        assert_eq!(clear_below(panel(154.0), &many, column(212.0)), Ok(()));
        // Overlapping a line, off the right edge, outside the column.
        assert!(
            clear_below(panel(140.0), &many, column(212.0))
                .unwrap_err()
                .contains("overlaps")
        );
        let left = [900.0, 154.0, 180.0, 70.0];
        assert!(
            clear_below(left, &many, column(212.0))
                .unwrap_err()
                .contains("right edge")
        );
        assert!(clear_below(panel(154.0), &many, column(150.0)).is_err());
    }

    #[test]
    fn fixed_boxes_move_below_the_panel() {
        // The event panel: 21 % of the height, lower when the voice panel
        // reaches further down; back when it is gone.
        let event = BelowVoicePanel::Fraction(0.21);
        assert_eq!(event.top(720.0, None), 0.21 * 720.0);
        assert_eq!(event.top(720.0, Some(100.0)), 0.21 * 720.0);
        assert_eq!(event.top(720.0, Some(190.0)), 190.0 + GAP_BELOW_PX);
        let inspect = BelowVoicePanel::Px(56.0);
        assert_eq!(inspect.top(720.0, None), 56.0);
        assert_eq!(inspect.top(720.0, Some(120.0)), 126.0);
    }

    #[test]
    fn the_pulse_is_calm_and_still_with_reduced_motion() {
        for bar in 0..3 {
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for i in 0..200 {
                let h = bar_height(bar, i as f64 * 0.01, None, false);
                lo = lo.min(h);
                hi = hi.max(h);
            }
            assert!((4.0..=12.0).contains(&lo) && (4.0..=12.0).contains(&hi));
            assert!(hi - lo > 4.0, "it moves");
            assert_eq!(
                bar_height(bar, 0.0, None, true),
                bar_height(bar, 7.3, None, true),
                "still"
            );
        }
        // A quiet level: lower bars.
        assert!(bar_height(1, 0.0, Some(0.001), true) < bar_height(1, 0.0, Some(0.5), true));
    }

    #[test]
    fn the_icons_are_drawn_and_differ() {
        let all: Vec<Vec<u8>> = Icon::ALL.iter().map(|i| icon_pixels(*i, 32)).collect();
        for (i, px) in all.iter().enumerate() {
            assert_eq!(px.len(), 32 * 32 * 4);
            let ink = px.chunks(4).filter(|p| p[3] > 127).count();
            assert!(
                (60..700).contains(&ink),
                "{:?}: {ink} px of ink",
                Icon::ALL[i]
            );
            for other in &all[i + 1..] {
                assert_ne!(px, other);
            }
        }
    }

    #[test]
    fn the_short_line_and_the_demo_states() {
        assert_eq!(VoicePanelView::default().short(), "");
        let mut v = connected(3);
        assert_eq!(v.short(), "MIC off (3)");
        v.mic_on = true;
        v.speakers = speakers(&["Bea Climber", "Cleo", "Dana"]);
        assert_eq!(v.short(), "MIC on (3) Bea+2");
        v.speakers = speakers(&["Maximiliana Longname"]);
        assert_eq!(v.short().chars().count(), SHORT_MAX);
        v.call = CallState::Failed { reason: "x".into() };
        assert_eq!(v.short(), "voice failed");
        // Every demo state but "none" gives a panel; they differ.
        assert_eq!(demo("none"), None);
        assert_eq!(demo("whatever"), None);
        let demos: Vec<VoicePanelView> = DEMO_STATES[1..]
            .iter()
            .map(|s| demo(s).unwrap_or_else(|| panic!("{s}")))
            .collect();
        for (i, d) in demos.iter().enumerate() {
            assert!(d.visible());
            assert!(!demos[i + 1..].contains(d));
        }
        assert_eq!(demo("others").unwrap().speakers.len(), 5);
    }

    /// The panel's entities follow the view: hidden without voice, the
    /// rows in a call, nothing with `off`; the fake view wins.
    #[test]
    fn the_panel_follows_the_view_and_the_mode() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, VoicePanelPlugin));
        app.world_mut()
            .commands()
            .spawn(Node::default())
            .with_children(spawn);
        app.world_mut().flush();
        app.update();
        let shown = |app: &mut App| {
            let w = app.world_mut();
            let root = w
                .query_filtered::<&Node, With<VoicePanelRoot>>()
                .single(w)
                .unwrap()
                .display;
            let rows = w
                .query::<(&Node, &Row)>()
                .iter(w)
                .filter(|(n, _)| n.display != Display::None)
                .count();
            (root != Display::None, rows)
        };
        assert_eq!(shown(&mut app), (false, 0), "no voice: hidden");
        assert_eq!(
            app.world().resource::<VoicePanelInfo>().shape,
            Shape::Hidden
        );
        app.world_mut().resource_mut::<VoiceHud>().panel = VoicePanelView {
            speakers: speakers(&["Bea", "Cleo", "Dana", "Eli"]),
            ..connected(4)
        };
        app.update();
        // The call, my microphone, I hear, three speakers, "+1".
        assert_eq!(shown(&mut app), (true, 7));
        let info = app.world().resource::<VoicePanelInfo>().clone();
        assert_eq!(info.shape, Shape::Rows);
        assert_eq!(
            info.rows,
            [
                "connected (4)",
                "off (M) - everybody",
                "everybody",
                "Bea",
                "Cleo",
                "Dana",
                "+1"
            ]
        );
        // The key of the input mapping.
        app.world_mut().resource_mut::<VoiceKeys>().mic = "K".into();
        app.update();
        assert_eq!(
            app.world().resource::<VoicePanelInfo>().rows[1],
            "off (K) - everybody"
        );
        *app.world_mut().resource_mut::<VoicePanelMode>() = VoicePanelMode::Off;
        app.update();
        assert_eq!(shown(&mut app), (false, 0));
        *app.world_mut().resource_mut::<VoicePanelMode>() = VoicePanelMode::Full;
        app.world_mut().resource_mut::<FakeVoicePanel>().0 = demo("idle");
        app.update();
        assert_eq!(shown(&mut app), (true, 1), "the fake view, one dim line");
        assert_eq!(
            app.world().resource::<VoicePanelInfo>().rows,
            ["voice: not in a call"]
        );
    }
}

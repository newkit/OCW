//! The settings registry (spec §3.2.1): typed variables with a key, type,
//! default, range or choices, description and scope, stored in
//! `settings.toml`. Bevy-free; the console commands `set` / `get` and the
//! menu pages read and write it through [`Settings`].
//!
//! Layers, lowest first: the registry default < `settings.toml` <
//! `autoexec.cfg` (it runs `set` commands like the console) < the command
//! line and `set --temp`. The last two only hold for the session and are
//! never saved.
//!
//! The file has one section per key prefix (`display.fps` is `fps` in
//! `[display]`). Keys the registry does not know are kept as they are and
//! written back; a value of the wrong type or out of range is reported and
//! the default used (the file entry stays until the setting is set again).
//!
//! Shared by both apps (spec §14 Phase 10 B5): the desktop keeps the text
//! in `settings.toml` ([`Settings::load`] / [`Settings::save`], native
//! only), the browser app the same text in the page's `localStorage`
//! ([`Settings::parse`] / [`Settings::to_toml`]); keys, defaults, ranges and
//! the apply commands are the same.

use std::collections::BTreeMap;

use crate::bindings::{KEY_CHOICES, PAD_CHOICES};
#[cfg(not(target_arch = "wasm32"))]
use std::path::Path;

/// The type of a setting and its allowed values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    /// `on` / `off` (also true / false, yes / no, 1 / 0).
    Bool,
    /// A whole number in `min..=max`; menus change it by `step`.
    Int { min: i64, max: i64, step: i64 },
    /// One of these words.
    Choice(&'static [&'static str]),
    /// Free text (a device name), at most [`MAX_TEXT_LEN`] characters.
    Text,
}

/// Longest [`Kind::Text`] value.
pub const MAX_TEXT_LEN: usize = 200;

/// The file's section of the rider's choices for single other riders in
/// voice chat (spec §7.1.7 Q9): a volume and Block per rider key
/// (`ocw_voice::riders`), not registered settings:
///
/// ```toml
/// [voice_riders."name:ride.example.org/Bea Climber"]
/// name = "Bea Climber"
/// volume = 150
/// blocked = false
/// selected = true
/// ```
pub const VOICE_RIDERS: &str = "voice_riders";

/// Most riders remembered in [`VOICE_RIDERS`].
pub const MAX_VOICE_RIDERS: usize = 1000;

/// When a changed value takes effect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// At once (the apply command runs on every change, and at the start).
    Live,
    /// At the next start of the app (`reset` / R restarts it).
    NextStart,
}

/// What the app does with a setting at the start (and, for
/// [`Scope::Live`], on every change).
#[derive(Debug, Clone, Copy)]
pub enum Apply {
    /// Run this console command line; `{}` is replaced with the value.
    Command(&'static str),
    /// The app reads it at the start: the command-line option of this name
    /// (`bots` for `--bots`), which overrides it for the session.
    Arg(&'static str),
    /// The app reads it at the start (no command-line option).
    App,
}

/// One registered setting.
#[derive(Debug, Clone, Copy)]
pub struct SettingDef {
    pub key: &'static str,
    pub kind: Kind,
    /// The default, written as in the console (`on`, `50`, `chase`).
    pub default: &'static str,
    pub description: &'static str,
    pub scope: Scope,
    pub apply: Apply,
}

/// A setting's value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Text(String),
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Bool(b) => f.write_str(if *b { "on" } else { "off" }),
            Self::Int(i) => write!(f, "{i}"),
            Self::Text(s) => f.write_str(s),
        }
    }
}

impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            _ => None,
        }
    }

    fn to_toml(&self) -> toml::Value {
        match self {
            Self::Bool(b) => toml::Value::Boolean(*b),
            Self::Int(i) => toml::Value::Integer(*i),
            Self::Text(s) => toml::Value::String(s.clone()),
        }
    }
}

/// Where a setting's current value comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Default,
    File,
    /// `set --temp`: this session only.
    Temp,
    /// A command-line option: this session only.
    CommandLine,
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Default => "default",
            Self::File => "saved",
            Self::Temp => "temporary",
            Self::CommandLine => "command line",
        })
    }
}

/// A key or button binding (spec §3.2.1 M9): one of the names of
/// `bindings::KEY_CHOICES` / `PAD_CHOICES`.
macro_rules! binding {
    ($key:literal, $default:literal, $choices:expr, $desc:literal) => {
        SettingDef {
            key: $key,
            kind: Kind::Choice($choices),
            default: $default,
            description: $desc,
            scope: Scope::Live,
            apply: Apply::App,
        }
    };
}

/// All settings, in menu order (spec §3.2.1, Settings pages).
pub const SETTINGS: &[SettingDef] = &[
    SettingDef {
        key: "controls.autodrive",
        kind: Kind::Bool,
        default: "on",
        description: "Autodrive on at the start of a ride (A toggles it while riding)",
        scope: Scope::NextStart,
        apply: Apply::App,
    },
    SettingDef {
        key: "controls.invert_y",
        kind: Kind::Bool,
        default: "off",
        description: "Unicorn mode: moving the mouse up looks down (spec §14 Phase 13B)",
        scope: Scope::Live,
        apply: Apply::Command("god.invert {}"),
    },
    SettingDef {
        key: "controls.god_flight",
        kind: Kind::Choice(&["plane", "hover"]),
        default: "plane",
        description: "Unicorn mode: fly like a plane (always forward; arrows: throttle and bank) or hover (arrows: move, mouse: turn) (spec §14 Phase 13B)",
        scope: Scope::Live,
        apply: Apply::Command("god.flight {}"),
    },
    // Touch steering (spec §3.2.3): read by `ocw_ui::touch` every frame.
    SettingDef {
        key: "controls.touch_steer",
        kind: Kind::Choice(&crate::input::touch::TouchSetting::CHOICES),
        default: "auto",
        description: "Touch steering by the screen's left / right side: auto (hints on phones and tablets and after the first touch), on (always the hints) or off (no steering by touch) (spec §3.2.3)",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "controls.touch_zones",
        kind: Kind::Choice(&["full", "lower"]),
        default: "full",
        description: "Touch steering zones: the full height below the top band, or only the lower half (spec §3.2.3)",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "controls.touch_tip_seen",
        kind: Kind::Bool,
        default: "off",
        description: "The touch steering tip was shown on this device (set by the first touch steer)",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "trainer.difficulty",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "50",
        description: "Trainer difficulty in percent: how much of the road gradient SIM mode sends",
        scope: Scope::Live,
        apply: Apply::Command("trainer.difficulty {}"),
    },
    SettingDef {
        key: "trainer.virtual_power",
        kind: Kind::Int {
            min: 0,
            max: 2000,
            step: 10,
        },
        default: "200",
        description: "Power of the virtual trainer at the start, in watts",
        scope: Scope::NextStart,
        apply: Apply::Arg("power"),
    },
    SettingDef {
        key: "display.hud",
        kind: Kind::Bool,
        default: "on",
        description: "Show the HUD (power, speed, route, keys)",
        scope: Scope::Live,
        apply: Apply::Command("hud {}"),
    },
    SettingDef {
        key: "display.units",
        kind: Kind::Choice(&["metric", "imperial"]),
        default: "metric",
        description: "Units of speed and distance in the HUD (km/h and km, or mph and mi)",
        scope: Scope::Live,
        apply: Apply::Command("hud.units {}"),
    },
    SettingDef {
        key: "display.damage",
        kind: Kind::Bool,
        default: "on",
        description: "Show the damage indicator in the HUD (with ride.damage on; the effects stay when hidden)",
        scope: Scope::Live,
        apply: Apply::Command("hud.damage {}"),
    },
    SettingDef {
        key: "display.map",
        kind: Kind::Bool,
        default: "on",
        description: "Show the navigation map in the HUD (bottom right: the map around you heading-up, the route, the next turn)",
        scope: Scope::Live,
        apply: Apply::Command("hud.map {}"),
    },
    SettingDef {
        key: "display.map_zoom",
        kind: Kind::Choice(&["auto", "1", "2", "3", "4", "5", "6", "7"]),
        default: "auto",
        description: "The navigation map's zoom: auto (by speed, closer at junctions) or 1 (closest) - 7",
        scope: Scope::Live,
        apply: Apply::Command("hud.map.zoom {}"),
    },
    SettingDef {
        key: "display.gps",
        kind: Kind::Bool,
        default: "on",
        description: "Show the GPS line in the HUD (bottom left: latitude, longitude, altitude; world packages and route files)",
        scope: Scope::Live,
        apply: Apply::Command("hud.gps {}"),
    },
    SettingDef {
        key: "display.photo_preview",
        kind: Kind::Bool,
        default: "on",
        description: "Show a photo just saved (screenshot, ride photographer) for 3 seconds in the HUD (top right)",
        scope: Scope::Live,
        apply: Apply::Command("hud.photo_preview {}"),
    },
    SettingDef {
        key: "display.fps",
        kind: Kind::Bool,
        default: "off",
        description: "Show the FPS counter",
        scope: Scope::Live,
        apply: Apply::Command("fps {}"),
    },
    SettingDef {
        key: "display.camera",
        kind: Kind::Choice(&["chase", "side", "top"]),
        default: "chase",
        description: "Camera perspective",
        scope: Scope::Live,
        apply: Apply::Command("camera.mode {}"),
    },
    SettingDef {
        key: "display.quality",
        kind: Kind::Choice(crate::quality::QualitySetting::CHOICES),
        default: "auto",
        description: "Graphics quality: shadows, MSAA, view distance, trees, houses, particles; auto = by GPU and device (High on the desktop and WebGPU, Low on WebGL 2 and phones; spec §14 Phase 10 B7)",
        scope: Scope::Live,
        apply: Apply::Command("quality {}"),
    },
    // The render distance (spec §14 Phase 10, 10G; `view_distance`).
    SettingDef {
        key: "display.view_distance",
        kind: Kind::Choice(crate::view_distance::DISTANCE_CHOICES),
        default: "preset",
        description: "Render distance in km (1 to 60) or unlimited; preset = the quality preset's (High unlimited, Medium 30, Low 8). The upper limit of the dynamic mode",
        scope: Scope::Live,
        apply: Apply::Command("view {}"),
    },
    SettingDef {
        key: "display.view_distance_auto",
        kind: Kind::Bool,
        default: "on",
        description: "Adjust the render distance to the frame rate: shorter below the target, longer when there is room (up to display.view_distance)",
        scope: Scope::Live,
        apply: Apply::Command("view auto {}"),
    },
    SettingDef {
        key: "display.target_fps",
        kind: Kind::Choice(crate::view_distance::TARGET_CHOICES),
        default: "auto",
        description: "Target frame rate of the dynamic render distance: auto = the display's refresh on the desktop, 60 in the browser; display, or a rate",
        scope: Scope::Live,
        apply: Apply::Command("view target {}"),
    },
    SettingDef {
        key: "display.night_brightness",
        kind: Kind::Int {
            min: 0,
            max: 50,
            step: 5,
        },
        default: "10",
        description: "Least light at night in % of daylight (0 = realistic darkness; spec §14 Phase 9)",
        scope: Scope::Live,
        apply: Apply::Command("sky.night {}"),
    },
    SettingDef {
        key: "display.sky",
        kind: Kind::Choice(&["atmosphere", "simple"]),
        default: "atmosphere",
        description: "Sky: physically based atmosphere, or one plain colour (for weak GPUs)",
        scope: Scope::Live,
        apply: Apply::Command("sky.mode {}"),
    },
    SettingDef {
        key: "display.bike_lights",
        kind: Kind::Choice(&["auto", "on", "off"]),
        default: "auto",
        description: "Bike lights of all riders: from dusk to dawn (auto), always or never",
        scope: Scope::Live,
        apply: Apply::Command("sky.lights {}"),
    },
    SettingDef {
        key: "display.orthophoto",
        kind: Kind::Bool,
        default: "on",
        description: "Ground: the real place's orthophoto near the roads, where the world package has it (spec §14 Phase 8 T7)",
        scope: Scope::Live,
        apply: Apply::Command("ortho {}"),
    },
    SettingDef {
        key: "display.road_barriers",
        kind: Kind::Bool,
        default: "off",
        description: "World: gates, bollards and barriers on the roads (never solid on the road; spec §14 Phase 12 Q7)",
        scope: Scope::Live,
        apply: Apply::Command("road_barriers {}"),
    },
    SettingDef {
        key: "display.unicorn_trail",
        kind: Kind::Bool,
        default: "on",
        description: "Effects: the rainbow trail behind the unicorns of unicorn mode",
        scope: Scope::Live,
        apply: Apply::Command("god.trail {}"),
    },
    SettingDef {
        key: "display.fullscreen",
        kind: Kind::Bool,
        default: "off",
        description: "Borderless fullscreen on the current monitor (F11 toggles it while riding)",
        scope: Scope::NextStart,
        apply: Apply::Arg("fullscreen"),
    },
    // The ride photographer (spec §2.11).
    SettingDef {
        key: "photographer.enabled",
        kind: Kind::Bool,
        default: "on",
        description: "The ride photographer: a person or a drone takes up to 5 photos of nice views per ride",
        scope: Scope::Live,
        apply: Apply::Command("photographer {}"),
    },
    // The other riders' photographers (spec 2.11 Q5, decided 2026-11-19):
    // shown or hidden here only – the own photographer, the server and the
    // other riders are not affected.
    SettingDef {
        key: "photographer.others",
        kind: Kind::Bool,
        default: "on",
        description: "The other riders' photographers: their person, drone and flash and the shutter and drone sounds (off: hidden for you only)",
        scope: Scope::Live,
        apply: Apply::Command("photographer others {}"),
    },
    // The one folder for everything OCW saves for the user (spec §2.8.1
    // *Where an S recording is written*, v4.80 / v4.82): each Strava
    // (S … S) recording in its own subfolder (ISO date and place) with its
    // FIT file and photos, screenshots and other photos in a folder per
    // day; the browser chooses a folder instead (`activities.folder
    // choose`). It replaces `photos.folder` (v4.74 – v4.82).
    SettingDef {
        key: "activities.folder",
        kind: Kind::Text,
        default: "default",
        description: "The folder for everything OCW saves for you: default = ~/Activities/OCW, or a path (~/ for the home folder); each Strava recording (S) gets a subfolder with its FIT file and photos, screenshots and other photos a folder per day",
        scope: Scope::Live,
        apply: Apply::Command("activities.folder \"{}\""),
    },
    // Ride → Host a ride (spec §7.1.7 Q4): the router's ports opened by
    // UPnP while hosting; read at *Start hosting*, switched on the wizard's
    // Options page (`host.plan upnp`). The headless server has `--upnp`
    // (off by default).
    SettingDef {
        key: "host.upnp",
        kind: Kind::Bool,
        default: "on",
        description: "Host a ride: open the router's ports automatically (UPnP) while hosting, removed after",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "ride.bots",
        kind: Kind::Int {
            min: 0,
            max: 50,
            step: 1,
        },
        default: "10",
        description: "Number of test riders (bots) on the local server",
        scope: Scope::NextStart,
        apply: Apply::Arg("bots"),
    },
    SettingDef {
        key: "ride.random_start",
        kind: Kind::Bool,
        default: "on",
        description: "Start at a random point of the route or the road graph",
        scope: Scope::NextStart,
        apply: Apply::Arg("random_start"),
    },
    // Collisions (spec §14 Phase 12A, Q1 / Q3, 12D): the session's rules
    // (the host decides in shared rides) and an aid for manual steering.
    SettingDef {
        key: "ride.collisions",
        kind: Kind::Bool,
        default: "on",
        description: "Rider collisions: soft bumps and crashes (group rides: soft bumps only; in shared rides the host decides)",
        scope: Scope::Live,
        apply: Apply::Command("ride.collisions {}"),
    },
    SettingDef {
        key: "ride.chaotic_bots",
        kind: Kind::Bool,
        default: "off",
        description: "Bots sometimes make mistakes (late reactions, wheel touches) that can cause crashes",
        scope: Scope::Live,
        apply: Apply::Command("ride.chaotic_bots {}"),
    },
    // Damage (spec §14 Phase 12A, 12E, Q1: off by default; Q6: it carries
    // over between rides, repairs in the workshop).
    SettingDef {
        key: "ride.damage",
        kind: Kind::Bool,
        default: "off",
        description: "Damage: hits hurt the rider and damage the bike (only with collisions on; it carries over to the next ride, repairs in the workshop)",
        scope: Scope::Live,
        apply: Apply::Command("ride.damage {}"),
    },
    SettingDef {
        key: "ride.proximity_cue",
        kind: Kind::Bool,
        default: "off",
        description: "A soft sound and a HUD arrow when someone is close beside (1 m), in front or behind (2 m); it never steers",
        scope: Scope::Live,
        apply: Apply::Command("ride.proximity_cue {}"),
    },
    // Spec §14 Phase 12D: the "last moment" warning (an aid that never
    // steers; on by default, proposal v3.22).
    SettingDef {
        key: "ride.hit_warning",
        kind: Kind::Bool,
        default: "on",
        description: "An amber warning about a second before a predicted hit, and when the tyres slide on a wet road; it never steers",
        scope: Scope::Live,
        apply: Apply::Command("ride.hit_warning {}"),
    },
    // Spec §14 Phase 12C: the chase camera shakes after a crash or a hard
    // bump (off for motion comfort; the kiosk's show mode turns it off).
    SettingDef {
        key: "display.camera_shake",
        kind: Kind::Bool,
        default: "on",
        description: "The camera shakes briefly after a crash or a hard bump (turn off for motion comfort)",
        scope: Scope::Live,
        apply: Apply::Command("camera.shake {}"),
    },
    // The organiser's rules for the events this rider starts (spec §14
    // Phase 12 K6; the Organiser rules page, read by `event …` when its
    // option is not given): `auto` = the default by the event's kind.
    SettingDef {
        key: "event.collisions",
        kind: Kind::Choice(&["auto", "off", "soft", "full"]),
        default: "auto",
        description: "Collisions in the events you start: auto (group rides soft bumps, races the full model), off, soft (bumps only), full",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "event.damage",
        kind: Kind::Choice(&["auto", "on", "off"]),
        default: "auto",
        description: "Damage in the events you start: auto (your ride.damage), on, off",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "event.aids",
        kind: Kind::Choice(&["auto", "on", "off"]),
        default: "auto",
        description: "Riding aids (the last-moment warning, the proximity arrow and bell) allowed in the events you start: auto (allowed, races too), on, off",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "event.chaotic_bots",
        kind: Kind::Choice(&["auto", "on", "off"]),
        default: "auto",
        description: "Chaotic bots in the events you start: auto (your ride.chaotic_bots), on, off",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "event.neutral_service",
        kind: Kind::Bool,
        default: "off",
        description: "Neutral service in the events you start: a spare bike for a broken one (else DNF), a 30 s stop restores the bike",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "event.dnf_injured",
        kind: Kind::Bool,
        default: "off",
        description: "In the events you start a badly injured rider is DNF",
        scope: Scope::Live,
        apply: Apply::App,
    },
    // Real weather (spec §14 Phase 9, S7): the weather machine's Real
    // weather page saves them (`weather.real`, `weather.provider`).
    SettingDef {
        key: "weather.real",
        kind: Kind::Bool,
        default: "on",
        description: "Rides start in real weather from the providers (local rides; not in screenshot or capture runs unless --real-weather true)",
        scope: Scope::NextStart,
        apply: Apply::App,
    },
    // Weather physics (spec §14 Phase 9, S6, Q4): the local rides' switch;
    // in a shared ride the host's, in an event the organiser's applies.
    SettingDef {
        key: "weather.physics",
        kind: Kind::Bool,
        default: "on",
        description: "The weather affects the physics: wind, air density, wet roads; the trainer gets the wind (in shared rides the host decides, in events the organizer)",
        scope: Scope::Live,
        apply: Apply::Command("weather.physics {}"),
    },
    SettingDef {
        key: "weather.provider",
        kind: Kind::Choice(&["automatic", "meteoswiss", "dwd", "metno", "open-meteo"]),
        default: "automatic",
        description: "Data source of real weather: automatic (MeteoSwiss, DWD, then MET Norway), or one provider",
        scope: Scope::Live,
        apply: Apply::Command("weather.provider {}"),
    },
    SettingDef {
        key: "ride.record",
        kind: Kind::Bool,
        default: "on",
        description: "Record rides and save them as FIT files",
        scope: Scope::NextStart,
        apply: Apply::Arg("record"),
    },
    // Audio (spec §3.3): volumes in percent, master × bus.
    SettingDef {
        key: "audio.master",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "80",
        description: "Master volume in percent (scales all buses)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume master {}"),
    },
    SettingDef {
        key: "audio.effects",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "80",
        description: "Volume of the game sounds (bike, wind, tyres, menu, event cues) in percent",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume effects {}"),
    },
    SettingDef {
        key: "audio.others",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "80",
        description: "Volume of the other riders' bikes in percent (a part of the effects: master × effects × others)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume others {}"),
    },
    SettingDef {
        key: "audio.music",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "60",
        description: "Music volume in percent (no music yet: not on the Audio page until there is)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume music {}"),
    },
    SettingDef {
        key: "audio.voice",
        kind: Kind::Int {
            min: 0,
            max: 100,
            step: 5,
        },
        default: "100",
        description: "Voice chat volume in percent (voice chat comes in Phase 6)",
        scope: Scope::Live,
        apply: Apply::Command("audio.volume voice {}"),
    },
    SettingDef {
        key: "audio.mute",
        kind: Kind::Bool,
        default: "off",
        description: "Mute all sound",
        scope: Scope::Live,
        apply: Apply::Command("audio.mute {}"),
    },
    SettingDef {
        key: "audio.output",
        kind: Kind::Text,
        default: "default",
        description: "Output device by name (audio.devices lists them), or default; a missing device falls back to the default",
        scope: Scope::Live,
        apply: Apply::Command("audio.output \"{}\""),
    },
    // Voice chat (spec §2.4, §7, M8): always on like a call; every change
    // is handed to the voice controller by `voice.apply`.
    SettingDef {
        key: "voice.enabled",
        kind: Kind::Bool,
        default: "on",
        description: "Voice chat: join the voice room automatically on shared rides",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply enabled {}"),
    },
    SettingDef {
        key: "voice.mic_muted",
        kind: Kind::Bool,
        default: "off",
        description: "Own microphone muted now (M, gamepad Select); every shared ride is joined muted, so it is not remembered for the next",
        scope: Scope::Live,
        apply: Apply::Command("voice.mute-mic {}"),
    },
    SettingDef {
        key: "voice.speaker_muted",
        kind: Kind::Bool,
        default: "off",
        description: "Speaker muted: no voices (Shift+M, gamepad R3); game sounds stay",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply speaker_muted {}"),
    },
    SettingDef {
        key: "voice.volume",
        kind: Kind::Int {
            min: 0,
            max: 200,
            step: 10,
        },
        default: "100",
        description: "Voice volume in percent",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply volume {}"),
    },
    SettingDef {
        key: "voice.spatial",
        kind: Kind::Bool,
        default: "on",
        description: "Spatial voice: riders further away sound quieter, fading from 5 m to silence at 10 m; selected riders (Voice room) at full volume at any distance (off: the same volume up to 10 m)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply spatial {}"),
    },
    SettingDef {
        key: "voice.max_voices",
        kind: Kind::Int {
            min: 1,
            max: 32,
            step: 1,
        },
        default: "24",
        description: "Hear at most this many riders at once (those talking first, then the nearest)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply max_voices {}"),
    },
    SettingDef {
        key: "voice.room",
        kind: Kind::Choice(&["everybody", "selected"]),
        default: "everybody",
        description: "Voice room: listen to everybody nearby (up to 10 m), or only to the selected riders, at any distance (voice.select WHO)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply room {}"),
    },
    SettingDef {
        key: "voice.heard_by",
        kind: Kind::Choice(&["no_one", "nearby", "selected"]),
        default: "no_one",
        description: "Who hears me: no one, all nearby (up to 10 m; riders who selected you at any distance), or the selected riders (voice.select WHO)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply heard_by {}"),
    },
    SettingDef {
        key: "voice.first_call_notice",
        kind: Kind::Bool,
        default: "on",
        description: "Show the voice notice (joined muted, who hears you) at the next call; off once shown",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply first_call_notice {}"),
    },
    SettingDef {
        key: "voice.input",
        kind: Kind::Text,
        default: "default",
        description: "Microphone by name (default: the system's)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply input {}"),
    },
    SettingDef {
        key: "voice.output",
        kind: Kind::Text,
        default: "default",
        description: "Speaker for voices by name (default: the system's)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply output {}"),
    },
    SettingDef {
        key: "audio.echo_bot",
        kind: Kind::Bool,
        default: "off",
        description: "Echo test: an Echo rider on the road plays your microphone back to you when you are near it",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply echo {}"),
    },
    SettingDef {
        key: "audio.echo_delay_ms",
        kind: Kind::Int {
            min: 20,
            max: 3000,
            step: 100,
        },
        default: "300",
        description: "Echo test: how long the Echo rider waits before playing you back (ms)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply echo_delay_ms {}"),
    }, // Accessibility (spec §3.2.1 M9).
    SettingDef {
        key: "display.ui_scale",
        kind: Kind::Int {
            min: 100,
            max: 200,
            step: 25,
        },
        default: "100",
        description: "Size of the HUD's text and panels in percent (the menu is sized by the window height already)",
        scope: Scope::Live,
        apply: Apply::Command("ui.scale {}"),
    },
    SettingDef {
        key: "display.palette",
        kind: Kind::Choice(&["standard", "colour-safe"]),
        default: "standard",
        description: "Status colours of the HUD and the menu: standard, or colour-safe (blue to orange, for red-green colour blindness); icons and words stay",
        scope: Scope::Live,
        apply: Apply::Command("ui.palette {}"),
    },
    SettingDef {
        key: "display.high_contrast",
        kind: Kind::Bool,
        default: "off",
        description: "High-contrast HUD: opaque black panels with white text",
        scope: Scope::Live,
        apply: Apply::Command("ui.contrast {}"),
    },
    SettingDef {
        key: "display.reduced_motion",
        kind: Kind::Bool,
        default: "off",
        description: "Reduced motion: no camera shake and no flashing HUD warnings",
        scope: Scope::Live,
        apply: Apply::Command("ui.motion {}"),
    },
    SettingDef {
        key: "controls.pad_confirm",
        kind: Kind::Choice(&["south", "east"]),
        default: "south",
        description: "The gamepad's Confirm button in the menu: south (A / Cross) or east (Nintendo layout); Back is the other",
        scope: Scope::Live,
        apply: Apply::App,
    },
    // Key and button bindings (spec §3.2.1 M9, `bindings`): one key and
    // one gamepad button per ride action; read by the apps' input.
    binding!("keys.faster", "Up", KEY_CHOICES, "Key: Faster (power +)"),
    binding!(
        "pad.faster",
        "DPadUp",
        PAD_CHOICES,
        "Gamepad button: Faster (power +)"
    ),
    binding!(
        "keys.stop",
        "Down",
        KEY_CHOICES,
        "Key: Stop (power 0, brake)"
    ),
    binding!(
        "pad.stop",
        "DPadDown",
        PAD_CHOICES,
        "Gamepad button: Stop (power 0, brake)"
    ),
    binding!("keys.steer_left", "Left", KEY_CHOICES, "Key: Steer left"),
    binding!(
        "pad.steer_left",
        "none",
        PAD_CHOICES,
        "Gamepad button: Steer left"
    ),
    binding!("keys.steer_right", "Right", KEY_CHOICES, "Key: Steer right"),
    binding!(
        "pad.steer_right",
        "none",
        PAD_CHOICES,
        "Gamepad button: Steer right"
    ),
    binding!("keys.power_up", "PageUp", KEY_CHOICES, "Key: Power +10 W"),
    binding!(
        "pad.power_up",
        "none",
        PAD_CHOICES,
        "Gamepad button: Power +10 W"
    ),
    binding!(
        "keys.power_down",
        "PageDown",
        KEY_CHOICES,
        "Key: Power -10 W"
    ),
    binding!(
        "pad.power_down",
        "none",
        PAD_CHOICES,
        "Gamepad button: Power -10 W"
    ),
    binding!(
        "keys.autodrive",
        "A",
        KEY_CHOICES,
        "Key: Autodrive on / off"
    ),
    binding!(
        "pad.autodrive",
        "South",
        PAD_CHOICES,
        "Gamepad button: Autodrive on / off"
    ),
    binding!("keys.wild", "W", KEY_CHOICES, "Key: Wild mode on / off"),
    binding!(
        "pad.wild",
        "West",
        PAD_CHOICES,
        "Gamepad button: Wild mode on / off"
    ),
    binding!("keys.u_turn", "U", KEY_CHOICES, "Key: U-turn (free ride)"),
    binding!(
        "pad.u_turn",
        "East",
        PAD_CHOICES,
        "Gamepad button: U-turn (free ride)"
    ),
    binding!("keys.rejoin", "J", KEY_CHOICES, "Key: Back to the route"),
    binding!(
        "pad.rejoin",
        "North",
        PAD_CHOICES,
        "Gamepad button: Back to the route"
    ),
    binding!(
        "keys.exit_left",
        "Q",
        KEY_CHOICES,
        "Key: Junction: the exit to the left"
    ),
    binding!(
        "pad.exit_left",
        "DPadLeft",
        PAD_CHOICES,
        "Gamepad button: Junction: the exit to the left"
    ),
    binding!(
        "keys.exit_right",
        "E",
        KEY_CHOICES,
        "Key: Junction: the exit to the right"
    ),
    binding!(
        "pad.exit_right",
        "DPadRight",
        PAD_CHOICES,
        "Gamepad button: Junction: the exit to the right"
    ),
    binding!("keys.camera", "C", KEY_CHOICES, "Key: Camera mode"),
    binding!(
        "pad.camera",
        "none",
        PAD_CHOICES,
        "Gamepad button: Camera mode"
    ),
    binding!(
        "keys.video",
        "V",
        KEY_CHOICES,
        "Key: Video recording start / stop"
    ),
    binding!(
        "pad.video",
        "none",
        PAD_CHOICES,
        "Gamepad button: Video recording start / stop"
    ),
    binding!(
        "keys.fullscreen",
        "F11",
        KEY_CHOICES,
        "Key: Fullscreen on / off"
    ),
    binding!(
        "pad.fullscreen",
        "none",
        PAD_CHOICES,
        "Gamepad button: Fullscreen on / off"
    ),
    binding!("keys.restart", "R", KEY_CHOICES, "Key: Restart"),
    binding!(
        "pad.restart",
        "none",
        PAD_CHOICES,
        "Gamepad button: Restart"
    ),
    binding!(
        "keys.strava",
        "S",
        KEY_CHOICES,
        "Key: Strava recording start / stop"
    ),
    binding!(
        "pad.strava",
        "L3",
        PAD_CHOICES,
        "Gamepad button: Strava recording start / stop"
    ),
    binding!("keys.god", "G", KEY_CHOICES, "Key: Unicorn mode on / off"),
    binding!(
        "pad.god",
        "none",
        PAD_CHOICES,
        "Gamepad button: Unicorn mode on / off"
    ),
    binding!(
        "keys.mute_mic",
        "M",
        KEY_CHOICES,
        "Key: Mute the microphone"
    ),
    binding!(
        "pad.mute_mic",
        "Select",
        PAD_CHOICES,
        "Gamepad button: Mute the microphone"
    ),
    binding!(
        "keys.mute_voices",
        "Shift+M",
        KEY_CHOICES,
        "Key: Mute the voices"
    ),
    binding!(
        "pad.mute_voices",
        "R3",
        PAD_CHOICES,
        "Gamepad button: Mute the voices"
    ),
    binding!("keys.map", "N", KEY_CHOICES, "Key: Navigation map on / off"),
    binding!(
        "pad.map",
        "none",
        PAD_CHOICES,
        "Gamepad button: Navigation map on / off"
    ),
    binding!(
        "keys.map_zoom",
        "Z",
        KEY_CHOICES,
        "Key: Navigation map zoom (auto, 1 - 7)"
    ),
    binding!(
        "pad.map_zoom",
        "none",
        PAD_CHOICES,
        "Gamepad button: Navigation map zoom (auto, 1 - 7)"
    ),
];

/// Settings the browser app does not have (spec §14 Phase 10, B6): the
/// window's fullscreen (the browser has its own), the random start and
/// real weather at the start (the in-browser local server cannot fetch the
/// providers yet). God mode's settings apply in both apps since Phase 13
/// G5 (v3.37); the browser has no video, fullscreen or restart key.
const NOT_IN_BROWSER: &[&str] = &[
    "display.fullscreen",
    "ride.random_start",
    "weather.real",
    "keys.video",
    "pad.video",
    "keys.fullscreen",
    "pad.fullscreen",
    "keys.restart",
    "pad.restart",
    // The browser chooses its activities folder through the File System
    // Access API (`activities.folder choose`), not by path.
    "activities.folder",
    // Hosting needs the desktop app (browsers cannot open a port).
    "host.upnp",
];

/// Whether `key` takes effect on `platform` (the menu hides the others,
/// `get` lists only these, `set` keeps the rest without applying it).
pub fn applies(key: &str, platform: crate::menu::model::Platform) -> bool {
    match platform {
        crate::menu::model::Platform::Desktop => true,
        crate::menu::model::Platform::Browser => !NOT_IN_BROWSER.contains(&key),
    }
}

/// The setting registered under `key`.
pub fn def(key: &str) -> Option<&'static SettingDef> {
    SETTINGS.iter().find(|d| d.key == key)
}

impl SettingDef {
    /// Parses a value as typed in the console.
    pub fn parse(&self, text: &str) -> Result<Value, String> {
        let text = text.trim();
        match self.kind {
            Kind::Bool => match text.to_ascii_lowercase().as_str() {
                "on" | "true" | "yes" | "1" => Ok(Value::Bool(true)),
                "off" | "false" | "no" | "0" => Ok(Value::Bool(false)),
                _ => Err(format!("{}: expected on or off, not {text:?}", self.key)),
            },
            Kind::Int { .. } => {
                let v = text
                    .parse::<i64>()
                    .map_err(|_| format!("{}: expected a whole number, not {text:?}", self.key))?;
                self.check(Value::Int(v))
            }
            Kind::Choice(choices) => choices
                .iter()
                .find(|c| c.eq_ignore_ascii_case(text))
                .map(|c| Value::Text((*c).to_string()))
                .ok_or_else(|| {
                    format!(
                        "{}: expected one of {}, not {text:?}",
                        self.key,
                        choices.join(", ")
                    )
                }),
            Kind::Text => {
                if text.is_empty() {
                    Err(format!("{}: expected a name", self.key))
                } else if text.chars().count() > MAX_TEXT_LEN || text.contains(['\n', '"']) {
                    Err(format!(
                        "{}: at most {MAX_TEXT_LEN} characters on one line, without quotes",
                        self.key
                    ))
                } else {
                    Ok(Value::Text(text.to_string()))
                }
            }
        }
    }

    /// Converts a value from the settings file.
    fn value_from_toml(&self, v: &toml::Value) -> Result<Value, String> {
        match (self.kind, v) {
            (Kind::Bool, toml::Value::Boolean(b)) => Ok(Value::Bool(*b)),
            (Kind::Int { .. }, toml::Value::Integer(i)) => self.check(Value::Int(*i)),
            (Kind::Choice(_) | Kind::Text, toml::Value::String(s)) => self.parse(s),
            (_, other) => Err(format!(
                "{}: expected {}, not {other}",
                self.key,
                self.kind_text()
            )),
        }
    }

    fn check(&self, v: Value) -> Result<Value, String> {
        match (self.kind, &v) {
            (Kind::Int { min, max, .. }, Value::Int(i)) if !(min..=max).contains(i) => Err(
                format!("{}: {i} is out of range ({min} to {max})", self.key),
            ),
            _ => Ok(v),
        }
    }

    /// The default value.
    pub fn default_value(&self) -> Value {
        self.parse(self.default)
            .expect("registry defaults are valid (tested)")
    }

    /// "on / off", "0 to 100", "chase, side, top"; a long list shortened
    /// ("preset, 1, 2 … 60, unlimited").
    pub fn kind_text(&self) -> String {
        match self.kind {
            Kind::Bool => "on / off".into(),
            Kind::Int { min, max, .. } => format!("{min} to {max}"),
            Kind::Choice(c) if c.len() > 12 => format!(
                "{}, {}, {} … {}, {}",
                c[0],
                c[1],
                c[2],
                c[c.len() - 2],
                c[c.len() - 1]
            ),
            Kind::Choice(c) => c.join(", "),
            Kind::Text => "a name".into(),
        }
    }

    /// The value one step down (`dir` < 0) or up from `v`, for menus:
    /// toggles a switch, moves a number by its step (within the range),
    /// cycles through the choices; leaves text as it is.
    pub fn step(&self, v: &Value, dir: i32) -> Value {
        match (self.kind, v) {
            (Kind::Bool, Value::Bool(b)) => Value::Bool(!b),
            (Kind::Int { min, max, step }, Value::Int(i)) => {
                Value::Int((i + step * i64::from(dir.signum())).clamp(min, max))
            }
            (Kind::Choice(c), Value::Text(s)) => {
                let n = c.len() as i32;
                let at = c.iter().position(|x| x == s).unwrap_or(0) as i32;
                Value::Text(c[(at + dir.signum()).rem_euclid(n) as usize].to_string())
            }
            (Kind::Text, Value::Text(_)) => v.clone(),
            _ => self.default_value(),
        }
    }

    /// The console command line that applies `v`, if the setting has one.
    pub fn apply_command(&self, v: &Value) -> Option<String> {
        match self.apply {
            Apply::Command(template) => Some(template.replace("{}", &v.to_string())),
            Apply::Arg(_) | Apply::App => None,
        }
    }

    /// (section, name) in the file.
    fn file_key(&self) -> (&'static str, &'static str) {
        self.key.split_once('.').expect("keys are section.name")
    }
}

/// The settings of this app: the file's content and the session values.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    /// `settings.toml` as read, with saved changes (unknown keys kept).
    doc: toml::Table,
    /// Session-only values (`set --temp`, command-line options).
    session: BTreeMap<&'static str, (Value, Source)>,
}

impl Settings {
    /// Reads settings from the file text. Returns them and the warnings
    /// (unknown keys, invalid values); a syntax error is an error.
    pub fn parse(text: &str) -> Result<(Self, Vec<String>), String> {
        let doc: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let s = Self {
            doc,
            session: BTreeMap::new(),
        };
        let mut warnings = Vec::new();
        for key in s.unknown_keys() {
            warnings.push(format!("unknown setting {key} (kept in the file)"));
        }
        for d in SETTINGS {
            if let Some(Err(e)) = s.file_entry(d).map(|v| d.value_from_toml(v)) {
                warnings.push(format!("{e}; using the default {}", d.default));
            }
        }
        Ok((s, warnings))
    }

    /// Reads the file; a missing file means all defaults.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn load(path: &Path) -> Result<(Self, Vec<String>), String> {
        match std::fs::read_to_string(path) {
            Ok(text) => Self::parse(&text).map_err(|e| format!("{}: {e}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((Self::default(), Vec::new())),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// The file's content (known and unknown keys).
    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(&self.doc).expect("a TOML table serializes")
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        std::fs::write(path, self.to_toml()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The rider's volume and Block for other riders ([`VOICE_RIDERS`]);
    /// broken entries are skipped, volumes held in range.
    pub fn voice_riders(&self) -> ocw_voice::riders::RiderPrefs {
        use ocw_voice::riders::{RiderPref, clamp_volume};
        let mut out = ocw_voice::riders::RiderPrefs::new();
        let Some(t) = self.doc.get(VOICE_RIDERS).and_then(|v| v.as_table()) else {
            return out;
        };
        for (key, v) in t.iter().take(MAX_VOICE_RIDERS) {
            let Some(e) = v.as_table() else {
                continue;
            };
            let pref = RiderPref {
                name: e
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                volume_pct: e
                    .get("volume")
                    .and_then(|n| n.as_integer())
                    .map_or(ocw_voice::riders::DEFAULT_VOLUME_PCT, clamp_volume),
                blocked: e.get("blocked").and_then(|b| b.as_bool()).unwrap_or(false),
                selected: e.get("selected").and_then(|b| b.as_bool()).unwrap_or(false),
            };
            if !pref.is_default() {
                out.insert(key.clone(), pref);
            }
        }
        out
    }

    /// Remembers the choice for one rider (to be saved); a choice back at
    /// the defaults is forgotten.
    pub fn set_voice_rider(&mut self, key: &str, pref: &ocw_voice::riders::RiderPref) {
        let section = self
            .doc
            .entry(VOICE_RIDERS)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !section.is_table() {
            *section = toml::Value::Table(toml::Table::new());
        }
        let t = section.as_table_mut().expect("a table");
        if pref.is_default() {
            t.remove(key);
        } else {
            let mut e = toml::Table::new();
            e.insert("name".into(), toml::Value::String(pref.name.clone()));
            e.insert(
                "volume".into(),
                toml::Value::Integer(i64::from(pref.volume_pct)),
            );
            e.insert("blocked".into(), toml::Value::Boolean(pref.blocked));
            e.insert("selected".into(), toml::Value::Boolean(pref.selected));
            t.insert(key.to_owned(), toml::Value::Table(e));
        }
        if t.is_empty() {
            self.doc.remove(VOICE_RIDERS);
        }
    }

    /// Keys in the file the registry does not know, as `section.name`.
    pub fn unknown_keys(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (section, v) in self.doc.iter().filter(|(s, _)| s.as_str() != VOICE_RIDERS) {
            match v {
                toml::Value::Table(t) => {
                    for name in t.keys() {
                        let key = format!("{section}.{name}");
                        if def(&key).is_none() {
                            out.push(key);
                        }
                    }
                }
                _ => out.push(section.clone()),
            }
        }
        out
    }

    fn file_entry(&self, d: &SettingDef) -> Option<&toml::Value> {
        let (section, name) = d.file_key();
        self.doc.get(section)?.as_table()?.get(name)
    }

    /// The value from the file, if it is there and valid.
    pub fn file_value(&self, key: &str) -> Option<Value> {
        let d = def(key)?;
        self.file_entry(d).and_then(|v| d.value_from_toml(v).ok())
    }

    /// The current value and where it comes from.
    pub fn get_with_source(&self, key: &str) -> Result<(Value, Source), String> {
        let d = def(key).ok_or_else(|| unknown(key))?;
        if let Some((v, s)) = self.session.get(d.key) {
            return Ok((v.clone(), *s));
        }
        Ok(match self.file_value(key) {
            Some(v) => (v, Source::File),
            None => (d.default_value(), Source::Default),
        })
    }

    pub fn get(&self, key: &str) -> Result<Value, String> {
        self.get_with_source(key).map(|(v, _)| v)
    }

    /// Sets a value to be saved (`set KEY VALUE`); it replaces a session
    /// value of the key too.
    pub fn set(&mut self, key: &str, text: &str) -> Result<Value, String> {
        let d = def(key).ok_or_else(|| unknown(key))?;
        let v = d.parse(text)?;
        let (section, name) = d.file_key();
        let table = self
            .doc
            .entry(section)
            .or_insert_with(|| toml::Value::Table(toml::Table::new()));
        if !table.is_table() {
            // A top-level value with the section's name: replaced.
            *table = toml::Value::Table(toml::Table::new());
        }
        table
            .as_table_mut()
            .expect("a table")
            .insert(name.to_string(), v.to_toml());
        self.session.remove(d.key);
        Ok(v)
    }

    /// Sets a value for this session only (`set --temp`).
    pub fn set_temp(&mut self, key: &str, text: &str) -> Result<Value, String> {
        let d = def(key).ok_or_else(|| unknown(key))?;
        let v = d.parse(text)?;
        self.session.insert(d.key, (v.clone(), Source::Temp));
        Ok(v)
    }

    /// A command-line option overrides the setting for this session.
    pub fn set_command_line(&mut self, key: &str, v: Value) {
        if let Some(d) = def(key) {
            self.session.insert(d.key, (v, Source::CommandLine));
        }
    }

    /// The command lines that apply the settings at the start, in
    /// registry order (before `autoexec.cfg` and `--cmd`). Settings at
    /// their default are skipped: the app starts with the defaults anyway.
    pub fn startup_commands(&self) -> Vec<String> {
        SETTINGS
            .iter()
            .filter_map(|d| {
                let (v, source) = self.get_with_source(d.key).ok()?;
                if source == Source::Default {
                    return None;
                }
                d.apply_command(&v)
            })
            .collect()
    }

    /// `key = value (source)`.
    pub fn line(&self, key: &str) -> Result<String, String> {
        let (v, source) = self.get_with_source(key)?;
        Ok(format!("{key} = {v} ({source})"))
    }
}

fn unknown(key: &str) -> String {
    format!("unknown setting {key:?} (get lists them)")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The per-rider voice choices (spec §7.1.7 Q9) survive a save and a
    /// load, are no unknown keys, and broken entries do no harm.
    #[test]
    fn voice_riders_round_trip() {
        use ocw_voice::riders::RiderPref;
        let mut s = Settings::default();
        assert!(s.voice_riders().is_empty());
        let bea = RiderPref {
            volume_pct: 150,
            ..RiderPref::new("Bea Climber")
        };
        let ada = RiderPref {
            blocked: true,
            ..RiderPref::new("Ada Rider")
        };
        // A selected rider (spec §7.1.3, §7.1.7 Q8): remembered like a block.
        let cleo = RiderPref {
            selected: true,
            ..RiderPref::new("Cleo Sprinter")
        };
        s.set_voice_rider("name:ride.example.org/Bea Climber", &bea);
        s.set_voice_rider("acct:acc-1", &ada);
        s.set_voice_rider("acct:acc-3", &cleo);
        let text = s.to_toml();
        assert!(text.contains("selected = true"), "{text}");
        let (back, warnings) = Settings::parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        let prefs = back.voice_riders();
        assert_eq!(prefs.len(), 3);
        assert_eq!(prefs["name:ride.example.org/Bea Climber"], bea);
        assert_eq!(prefs["acct:acc-1"], ada);
        assert_eq!(prefs["acct:acc-3"], cleo);
        // Back to the defaults: forgotten, the section gone with the last.
        let mut back = back;
        back.set_voice_rider("acct:acc-3", &RiderPref::new("Cleo Sprinter"));
        back.set_voice_rider("acct:acc-1", &RiderPref::new("Ada Rider"));
        back.set_voice_rider(
            "name:ride.example.org/Bea Climber",
            &RiderPref::new("Bea Climber"),
        );
        assert!(back.voice_riders().is_empty());
        assert!(!back.to_toml().contains("voice_riders"));
        // Broken entries are skipped, volumes held in range.
        let (s, w) = Settings::parse("voice_riders = 3\n[voice]\nvolume = 80\n").unwrap();
        assert!(s.voice_riders().is_empty());
        assert!(w.is_empty(), "{w:?}");
        let (s, _) = Settings::parse(
            "[voice_riders]\nx = 1\n[voice_riders.\"name:local/Cleo\"]\nvolume = 999\n",
        )
        .unwrap();
        let p = s.voice_riders();
        assert_eq!(p.len(), 1);
        assert_eq!(p["name:local/Cleo"].volume_pct, 200);
        // A section that was not a table is replaced on the next change.
        let (mut s, _) = Settings::parse("voice_riders = 3\n").unwrap();
        s.set_voice_rider("acct:z", &ada);
        assert_eq!(s.voice_riders().len(), 1);
    }

    /// Host a ride opens the router's ports by default (spec §7.1.7 Q4);
    /// the browser cannot host, so it has no such setting.
    #[test]
    fn host_upnp_is_on_by_default_and_desktop_only() {
        let s = Settings::default();
        assert_eq!(s.get("host.upnp").unwrap(), Value::Bool(true));
        assert!(applies("host.upnp", crate::menu::model::Platform::Desktop));
        assert!(!applies("host.upnp", crate::menu::model::Platform::Browser));
        let (s, w) = Settings::parse("[host]\nupnp = false\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.get("host.upnp").unwrap(), Value::Bool(false));
    }

    /// The other riders' photographers are shown by default, in both apps
    /// (spec 2.11 Q5); the setting runs `photographer others on|off`.
    #[test]
    fn others_photographers_are_shown_by_default_in_both_apps() {
        let s = Settings::default();
        assert_eq!(s.get("photographer.others").unwrap(), Value::Bool(true));
        for p in [
            crate::menu::model::Platform::Desktop,
            crate::menu::model::Platform::Browser,
        ] {
            assert!(applies("photographer.others", p));
        }
        let d = def("photographer.others").unwrap();
        assert_eq!(d.scope, Scope::Live);
        assert_eq!(
            d.apply_command(&Value::Bool(false)).as_deref(),
            Some("photographer others off")
        );
        let (s, w) = Settings::parse("[photographer]\nothers = false\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.get("photographer.others").unwrap(), Value::Bool(false));
        assert_eq!(s.get("photographer.enabled").unwrap(), Value::Bool(true));
    }

    #[test]
    fn registry_is_valid() {
        let mut keys = std::collections::BTreeSet::new();
        for d in SETTINGS {
            assert!(keys.insert(d.key), "duplicate {}", d.key);
            let (section, name) = d.key.split_once('.').expect("section.name");
            assert!(!section.is_empty() && !name.is_empty() && !name.contains('.'));
            // Defaults parse and are in range; the step stays in range.
            let v = d.parse(d.default).unwrap_or_else(|e| panic!("{e}"));
            for dir in [-1, 1] {
                let stepped = d.step(&v, dir);
                assert!(d.parse(&stepped.to_string()).is_ok(), "{}", d.key);
            }
            assert!(!d.description.is_empty());
            if let Kind::Int { min, max, step } = d.kind {
                assert!(min < max && step > 0);
            }
            if let Apply::Command(c) = d.apply {
                assert!(c.contains("{}"), "{}", d.key);
            }
        }
    }

    #[test]
    fn values_parse_by_type_and_range() {
        let fps = def("display.fps").unwrap();
        assert_eq!(fps.parse("ON"), Ok(Value::Bool(true)));
        assert_eq!(fps.parse("0"), Ok(Value::Bool(false)));
        assert!(fps.parse("maybe").is_err());
        let d = def("trainer.difficulty").unwrap();
        assert_eq!(d.parse(" 80 "), Ok(Value::Int(80)));
        assert!(d.parse("101").unwrap_err().contains("out of range"));
        assert!(d.parse("-1").is_err());
        assert!(d.parse("12.5").is_err());
        let cam = def("display.camera").unwrap();
        assert_eq!(cam.parse("Top"), Ok(Value::Text("top".into())));
        assert!(cam.parse("bird").unwrap_err().contains("chase, side, top"));
    }

    #[test]
    fn text_settings_hold_device_names() {
        let d = def("voice.input").unwrap();
        assert_eq!(
            d.parse(" USB Headset "),
            Ok(Value::Text("USB Headset".into()))
        );
        assert!(d.parse("").is_err());
        assert!(d.parse("a\"b").is_err());
        assert!(d.parse(&"x".repeat(MAX_TEXT_LEN + 1)).is_err());
        let v = Value::Text("USB Headset".into());
        assert_eq!(d.step(&v, 1), v, "devices are stepped by the voice page");
        let (s, warnings) =
            Settings::parse("[voice]\ninput = \"USB Headset\"\nmic_muted = true\n").unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(s.get("voice.input"), Ok(v));
        assert_eq!(s.get("voice.mic_muted"), Ok(Value::Bool(true)));
        // First start: microphone and speaker on (not muted).
        let s = Settings::default();
        assert_eq!(s.get("voice.mic_muted"), Ok(Value::Bool(false)));
        assert_eq!(s.get("voice.speaker_muted"), Ok(Value::Bool(false)));
        assert_eq!(s.get("voice.enabled"), Ok(Value::Bool(true)));
    }

    #[test]
    fn steps_toggle_clamp_and_cycle() {
        let d = def("trainer.difficulty").unwrap();
        assert_eq!(d.step(&Value::Int(50), 1), Value::Int(55));
        assert_eq!(d.step(&Value::Int(100), 1), Value::Int(100));
        assert_eq!(d.step(&Value::Int(3), -1), Value::Int(0));
        let fps = def("display.fps").unwrap();
        assert_eq!(fps.step(&Value::Bool(false), -1), Value::Bool(true));
        let cam = def("display.camera").unwrap();
        let chase = Value::Text("chase".into());
        assert_eq!(cam.step(&chase, 1), Value::Text("side".into()));
        assert_eq!(cam.step(&chase, -1), Value::Text("top".into()));
    }

    #[test]
    fn toml_round_trip_keeps_unknown_keys() {
        let text = "\
top_level = 3

[display]
fps = true
hud = false
future_option = \"x\"

[sound]
master = 80
";
        let (mut s, warnings) = Settings::parse(text).unwrap();
        assert_eq!(s.get("display.fps"), Ok(Value::Bool(true)));
        assert_eq!(
            s.get_with_source("display.hud"),
            Ok((Value::Bool(false), Source::File))
        );
        assert_eq!(
            s.get_with_source("display.camera"),
            Ok((Value::Text("chase".into()), Source::Default))
        );
        let mut unknown = s.unknown_keys();
        unknown.sort();
        assert_eq!(
            unknown,
            ["display.future_option", "sound.master", "top_level"]
        );
        assert_eq!(warnings.len(), 3, "{warnings:?}");
        s.set("trainer.difficulty", "70").unwrap();
        s.set("display.fps", "off").unwrap();
        let (again, warnings) = Settings::parse(&s.to_toml()).unwrap();
        assert_eq!(warnings.len(), 3);
        assert_eq!(again.get("trainer.difficulty"), Ok(Value::Int(70)));
        assert_eq!(again.get("display.fps"), Ok(Value::Bool(false)));
        assert_eq!(again.get("display.hud"), Ok(Value::Bool(false)));
        let text = again.to_toml();
        assert!(text.contains("future_option = \"x\"") && text.contains("master = 80"));
        assert!(text.contains("top_level = 3"), "{text}");
        assert!(text.contains("[trainer]\ndifficulty = 70"), "{text}");
    }

    #[test]
    fn invalid_values_fall_back_to_the_default_with_a_warning() {
        let text = "[trainer]\ndifficulty = 300\n[display]\ncamera = \"bird\"\nfps = \"yes please\"\nhud = 1\n";
        let (s, warnings) = Settings::parse(text).unwrap();
        assert_eq!(s.get("trainer.difficulty"), Ok(Value::Int(50)));
        assert_eq!(s.get("display.camera"), Ok(Value::Text("chase".into())));
        assert_eq!(s.get("display.fps"), Ok(Value::Bool(false)));
        assert_eq!(s.get("display.hud"), Ok(Value::Bool(true)));
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings[0].contains("out of range") && warnings[0].contains("default 50"));
        // The invalid entries stay in the file until set again.
        assert!(s.to_toml().contains("difficulty = 300"));
        // A broken file is an error (the caller keeps it, uses defaults).
        assert!(Settings::parse("[display\nfps = true").is_err());
    }

    #[test]
    fn precedence_default_file_temp_command_line() {
        let (mut s, _) = Settings::parse("[ride]\nbots = 2\n").unwrap();
        assert_eq!(s.line("ride.bots").unwrap(), "ride.bots = 2 (saved)");
        s.set_temp("ride.bots", "7").unwrap();
        assert_eq!(
            s.get_with_source("ride.bots"),
            Ok((Value::Int(7), Source::Temp))
        );
        // Temporary values are not saved.
        assert!(s.to_toml().contains("bots = 2"));
        s.set_command_line("ride.bots", Value::Int(9));
        assert_eq!(
            s.get_with_source("ride.bots"),
            Ok((Value::Int(9), Source::CommandLine))
        );
        assert!(!s.to_toml().contains('9'));
        // `set` saves and replaces the session value.
        s.set("ride.bots", "3").unwrap();
        assert_eq!(
            s.get_with_source("ride.bots"),
            Ok((Value::Int(3), Source::File))
        );
        assert!(s.set("ride.bots", "51").is_err());
        assert!(
            s.set("no.such", "1")
                .unwrap_err()
                .contains("unknown setting")
        );
        assert!(s.get("no.such").is_err());
    }

    #[test]
    fn startup_commands_apply_the_non_default_settings() {
        let (mut s, _) = Settings::parse(
            "[display]\nfps = true\ncamera = \"top\"\nunits = \"imperial\"\n[ride]\nbots = 1\n[controls]\nautodrive = false\n",
        )
        .unwrap();
        s.set_temp("trainer.difficulty", "80").unwrap();
        assert_eq!(
            s.startup_commands(),
            [
                "trainer.difficulty 80",
                "hud.units imperial",
                "fps on",
                "camera.mode top",
            ]
        );
        assert!(Settings::default().startup_commands().is_empty());
    }

    /// The Audio page's settings (M5): saved in `[audio]`, read back, and
    /// applied by their commands.
    #[test]
    fn audio_settings_round_trip() {
        let mut s = Settings::default();
        s.set("audio.master", "55").unwrap();
        s.set("audio.mute", "on").unwrap();
        s.set("audio.output", " USB Audio Device, Analog ").unwrap();
        assert!(s.set("audio.output", "say \"hi\"").is_err());
        assert!(s.set("audio.output", "  ").is_err());
        assert!(s.set("audio.voice", "101").is_err());
        let text = s.to_toml();
        assert!(text.contains("[audio]"), "{text}");
        let (again, warnings) = Settings::parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(again.get("audio.master"), Ok(Value::Int(55)));
        assert_eq!(again.get("audio.mute"), Ok(Value::Bool(true)));
        assert_eq!(again.get("audio.effects"), Ok(Value::Int(80)));
        assert_eq!(again.get("audio.others"), Ok(Value::Int(80)));
        assert!(s.set("audio.others", "-5").is_err());
        let out = again.get("audio.output").unwrap();
        assert_eq!(out, Value::Text("USB Audio Device, Analog".into()));
        let d = def("audio.output").unwrap();
        assert_eq!(d.step(&out, 1), out, "text does not step");
        assert_eq!(
            again.startup_commands(),
            [
                "audio.volume master 55",
                "audio.mute on",
                "audio.output \"USB Audio Device, Analog\"",
            ]
        );
        // A number in the file where a name belongs: the default.
        let (bad, w) = Settings::parse("[audio]\noutput = 3\n").unwrap();
        assert_eq!(bad.get("audio.output"), Ok(Value::Text("default".into())));
        assert_eq!(w.len(), 1);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn load_and_save_files() {
        let dir = std::env::temp_dir().join(format!("ocw-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested/settings.toml");
        let (mut s, w) = Settings::load(&path).unwrap();
        assert!(w.is_empty());
        s.set("display.units", "imperial").unwrap();
        s.save(&path).unwrap();
        let (again, _) = Settings::load(&path).unwrap();
        assert_eq!(
            again.get("display.units"),
            Ok(Value::Text("imperial".into()))
        );
        std::fs::write(&path, "not toml [").unwrap();
        assert!(Settings::load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

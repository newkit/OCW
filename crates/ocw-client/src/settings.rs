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

use crate::bindings::PAD_CHOICES;
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
    /// A key binding (spec 3.2.5): one or two names of
    /// `bindings::KEY_CHOICES` separated by a comma ("Delete,Q"), or
    /// "none".
    Keys,
}

/// The choices of `trainer.wheel_mm`: the bike's, or a tyre's rolling
/// circumference in mm.
pub const WHEEL_CHOICES: [&str; 9] = [
    "bike", "2096", "2105", "2136", "2155", "2168", "2200", "2288", "2326",
];

/// The rider's wheel circumference of `trainer.wheel_mm` (`None`: the
/// bike's).
pub fn wheel_mm(value: &str) -> Option<u32> {
    value.trim().parse().ok().filter(|mm| *mm > 0)
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

/// A gamepad button binding (spec 3.2.1 M9, 3.2.5): one of the names of
/// `bindings::PAD_CHOICES`.
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

/// A key binding (spec 3.2.5): one or two key names of
/// `bindings::KEY_CHOICES`, comma separated ([`Kind::Keys`]).
macro_rules! keys_binding {
    ($key:literal, $default:literal, $desc:literal) => {
        SettingDef {
            key: $key,
            kind: Kind::Keys,
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
        key: "controls.touch_controls",
        kind: Kind::Choice(&["shown", "hidden"]),
        default: "shown",
        description: "The on-screen touch controls (the steering chevrons, the faster / slower marks, the tips): shown, or hidden – not drawn, but every touch area still works; the eye button beside the menu button switches it (spec §3.2.3)",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "controls.touch_hidden_hint_seen",
        kind: Kind::Bool,
        default: "off",
        description: "The hint \"Controls hidden – they still work\" was shown on this device (set when the touch controls are hidden the first time)",
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
        key: "trainer.wheel_mm",
        kind: Kind::Choice(&WHEEL_CHOICES),
        default: "bike",
        description: "The wheel's circumference (mm) for a speed sensor's speed: bike (from the bike: road 2105, gravel 2200, MTB 2288), or 2096 (700x23C), 2105 (700x25C), 2136 (700x28C), 2155 (700x32C), 2168 (700x35C), 2200 (700x40C), 2288 (29x2.25), 2326 (29x2.4) (spec 14 Phase 17)",
        scope: Scope::Live,
        apply: Apply::App,
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
    // Spec 4.6.2 Q1: the ground detail beside and on the roads.
    SettingDef {
        key: "display.ground_detail",
        kind: Kind::Choice(&["auto", "off", "low", "full"]),
        default: "auto",
        description: "World: stones, dirt, tufts, leaves and plant bits beside and on the roads; auto = by the quality preset (High all to 80 m, Medium 60 % to 55 m, Low 25 % to 35 m), low = 25 % to 35 m, full = all to 80 m (spec 4.6.2)",
        scope: Scope::Live,
        apply: Apply::Command("ground_detail {}"),
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
    // VR (spec §14 Phase 11, X1): the headset as a viewing device of the
    // desktop app (the `vr` build; `--vr` for one session).
    SettingDef {
        key: "vr.enabled",
        kind: Kind::Bool,
        default: "off",
        description: "Ride in a VR headset over OpenXR (Oculus Link, SteamVR, Monado); without a headset the app runs as before (desktop app built with VR)",
        scope: Scope::NextStart,
        apply: Apply::Arg("vr"),
    },
    SettingDef {
        key: "vr.mirror",
        kind: Kind::Choice(&["eye", "both", "chase"]),
        default: "eye",
        description: "What the monitor shows in VR: one eye, both eyes side by side, or the chase camera (for spectators; costs a third view)",
        scope: Scope::Live,
        apply: Apply::Command("vr mirror {}"),
    },
    // The bike computer on the bar (spec §14 Phase 11, v4.68): its size
    // (times the real one, %; readable on the Quest 1 from 275 %) and the
    // tilt added to its aim at the eyes.
    SettingDef {
        key: "vr.computer_size",
        kind: Kind::Int {
            min: 100,
            max: 400,
            step: 25,
        },
        default: "275",
        description: "VR: the bike computer's size on the bar, % of a real one (larger reads better in the headset)",
        scope: Scope::Live,
        apply: Apply::Command("vr computer size {}"),
    },
    SettingDef {
        key: "vr.computer_tilt",
        kind: Kind::Int {
            min: -30,
            max: 30,
            step: 5,
        },
        default: "0",
        description: "VR: the bike computer's screen tilted toward you (+) or flatter (−), degrees from facing your eyes",
        scope: Scope::Live,
        apply: Apply::Command("vr computer tilt {}"),
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
    // Strava (spec 2.8.1 Q6, v5.05): only the Strava recordings (S … S)
    // go to Strava, never whole rides; on: they are uploaded automatically
    // once the API app is set up and the account linked, off: they wait
    // for the upload by hand (the ride page's *Upload now*, or in the
    // browser). The desktop keeps it as the one `[strava] auto_upload` of
    // `ocw.toml` (spec 3.2.4; `config::SettingsStore`).
    SettingDef {
        key: "strava.auto_upload",
        kind: Kind::Bool,
        default: "on",
        description: "Upload the Strava recordings (S … S) to Strava automatically (your API app, account linked); off: they wait for you to upload them from the ride's page. Whole rides never go to Strava",
        scope: Scope::Live,
        apply: Apply::Command("strava.auto_upload {}"),
    },
    // Ride → Host a ride (spec §7.1.7 Q4): the router's ports opened by
    // UPnP while hosting; read at *Start hosting*, switched on the wizard's
    // Options page (`host.plan upnp`). The headless server has `--upnp`
    // (off by default).
    SettingDef {
        key: "net.public_server",
        kind: Kind::Text,
        default: "default",
        description: "The public server Ride -> Join and the start offer first (spec 11.5): default = ocw-server.fly.dev (for now), off = none, or another join info URL (https://.../join.json)",
        scope: Scope::Live,
        apply: Apply::App,
    },
    SettingDef {
        key: "host.upnp",
        kind: Kind::Bool,
        default: "on",
        description: "Host a ride: open the router's ports automatically (UPnP) while hosting, removed after",
        scope: Scope::Live,
        apply: Apply::App,
    },
    // Ride → Host a ride (spec §7.1.7 Q15): voice chat on the hosted
    // ride – the bundled voice server starts with it; read at *Start
    // hosting*, switched on the wizard's Options page (`host.plan voice`).
    // The headless server has `--voice off`.
    SettingDef {
        key: "host.voice",
        kind: Kind::Bool,
        default: "on",
        description: "Host a ride: voice chat on the hosted ride (off: no voice server, the riders have no voice on it)",
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
        description: "Damage: hits hurt the rider and damage the bike (only with collisions on; it carries over to the next ride, repairs in the workshop); while on, the HUD always shows the rider's health and the bike's condition",
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
    // Traffic (spec 14 Phase 18, 18A; Q1: aircraft on, road traffic and
    // UFOs off on a first start). Road traffic is the session's (the host
    // decides in shared rides, the organiser in events, Q2) and only in
    // the desktop app (Q9); aircraft and UFOs are the rider's own view.
    SettingDef {
        key: "traffic.enabled",
        kind: Kind::Bool,
        default: "on",
        description: "Traffic: the master switch; off = empty roads and sky (in shared rides the host's road traffic stays: it can touch you)",
        scope: Scope::Live,
        apply: Apply::Command("traffic enabled {}"),
    },
    SettingDef {
        key: "traffic.road",
        kind: Kind::Choice(&["off", "light", "normal", "busy", "lunatic"]),
        default: "off",
        description: "Road traffic: cars, vans, lorries, buses, motorbikes, tractors; normal follows the time of day, light a quarter, busy 1.5 x; lunatic: drivers speed and overtake dangerously (in shared rides the host decides, in events the organiser; desktop only)",
        scope: Scope::Live,
        apply: Apply::Command("traffic road {}"),
    },
    SettingDef {
        key: "traffic.manners",
        kind: Kind::Choice(&["polite", "mostly_polite", "some_realism"]),
        default: "polite",
        description: "Driver manners: polite (legal distance, no horns), mostly_polite (an occasional honk at a rider in the middle of the road), some_realism (now and then a close pass that never touches); the host decides in shared rides",
        scope: Scope::Live,
        apply: Apply::Command("traffic manners {}"),
    },
    SettingDef {
        key: "traffic.air",
        kind: Kind::Choice(&["off", "simulated", "live"]),
        default: "simulated",
        description: "Aircraft: airliners, small planes, helicopters, gliders, balloons; simulated, or live positions where a feed is reachable (until then simulated); for you only",
        scope: Scope::Live,
        apply: Apply::Command("traffic air {}"),
    },
    SettingDef {
        key: "traffic.ufo",
        kind: Kind::Choice(&["off", "rare", "often"]),
        default: "off",
        description: "UFOs: rare (now and then on clear nights), often (also by day high in the sky); they play with the unicorn, never touch a rider, never in events; for you only",
        scope: Scope::Live,
        apply: Apply::Command("traffic ufo {}"),
    },
    SettingDef {
        key: "traffic.sound",
        kind: Kind::Bool,
        default: "on",
        description: "Sounds of vehicles and aircraft",
        scope: Scope::Live,
        apply: Apply::Command("traffic sound {}"),
    },
    SettingDef {
        key: "traffic.live_road",
        kind: Kind::Bool,
        default: "off",
        description: "Live traffic data: live road events or density where the world has a suitable source (Tenerife: closures, roadworks and reported jams of the DGT; elsewhere the traffic follows the statistics)",
        scope: Scope::Live,
        apply: Apply::Command("traffic live_road {}"),
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
    // is handed to the voice controller by `voice.apply`. The old
    // `voice.enabled` is gone (spec §7.1.7 Q16: its off is *My voice* and
    // *I hear* both off; [`Settings::migrate_voice`]).
    SettingDef {
        key: "voice.mic_muted",
        kind: Kind::Bool,
        default: "off",
        description: "My voice off: the own microphone muted now (M, gamepad Select); every shared ride starts with My voice off, so it is not remembered for the next",
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
        key: "voice.my_voice",
        kind: Kind::Choice(&["everybody", "selected"]),
        default: "everybody",
        description: "My voice: whom it reaches while it is on (M) - everybody nearby (up to 10 m; riders who selected you at any distance) or the selected riders (voice.select WHO); off nobody hears you",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply my_voice {}"),
    },
    SettingDef {
        key: "voice.i_hear",
        kind: Kind::Choice(&["everybody", "selected", "nobody"]),
        default: "everybody",
        description: "I hear: everybody nearby (up to 10 m), only the selected riders at any distance (voice.select WHO), or nobody (others may still hear you)",
        scope: Scope::Live,
        apply: Apply::Command("voice.apply i_hear {}"),
    },
    SettingDef {
        key: "voice.first_call_notice",
        kind: Kind::Bool,
        default: "on",
        description: "Show the voice notice (My voice is off, M turns it on; whom you hear) at the next call; off once shown",
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
    keys_binding!("keys.faster", "Up", "Key: Faster (power +)"),
    binding!(
        "pad.faster",
        "DPadUp",
        PAD_CHOICES,
        "Gamepad button: Faster (power +)"
    ),
    keys_binding!("keys.stop", "Down", "Key: Stop (power 0, brake)"),
    binding!(
        "pad.stop",
        "none",
        PAD_CHOICES,
        "Gamepad button: Stop (power 0, brake)"
    ),
    keys_binding!("keys.steer_left", "Left", "Key: Steer left"),
    binding!(
        "pad.steer_left",
        "none",
        PAD_CHOICES,
        "Gamepad button: Steer left"
    ),
    keys_binding!("keys.steer_right", "Right", "Key: Steer right"),
    binding!(
        "pad.steer_right",
        "none",
        PAD_CHOICES,
        "Gamepad button: Steer right"
    ),
    keys_binding!("keys.autodrive", "A", "Key: Autodrive on / off"),
    binding!(
        "pad.autodrive",
        "South",
        PAD_CHOICES,
        "Gamepad button: Autodrive on / off"
    ),
    keys_binding!("keys.wild", "W", "Key: Wild mode on / off"),
    binding!(
        "pad.wild",
        "West",
        PAD_CHOICES,
        "Gamepad button: Wild mode on / off"
    ),
    keys_binding!("keys.u_turn", "U", "Key: U-turn (free ride)"),
    binding!(
        "pad.u_turn",
        "East",
        PAD_CHOICES,
        "Gamepad button: U-turn (free ride)"
    ),
    keys_binding!("keys.rejoin", "J", "Key: Back to the route"),
    binding!(
        "pad.rejoin",
        "North",
        PAD_CHOICES,
        "Gamepad button: Back to the route"
    ),
    keys_binding!(
        "keys.exit_left",
        "Delete,Q",
        "Key: Junction: the exit to the left"
    ),
    binding!(
        "pad.exit_left",
        "DPadLeft",
        PAD_CHOICES,
        "Gamepad button: Junction: the exit to the left"
    ),
    keys_binding!(
        "keys.exit_right",
        "PageDown,E",
        "Key: Junction: the exit to the right"
    ),
    binding!(
        "pad.exit_right",
        "DPadRight",
        PAD_CHOICES,
        "Gamepad button: Junction: the exit to the right"
    ),
    keys_binding!("keys.camera", "C", "Key: Camera mode"),
    binding!(
        "pad.camera",
        "none",
        PAD_CHOICES,
        "Gamepad button: Camera mode"
    ),
    keys_binding!("keys.video", "V", "Key: Video recording start / stop"),
    binding!(
        "pad.video",
        "none",
        PAD_CHOICES,
        "Gamepad button: Video recording start / stop"
    ),
    keys_binding!("keys.fullscreen", "F11", "Key: Fullscreen on / off"),
    binding!(
        "pad.fullscreen",
        "none",
        PAD_CHOICES,
        "Gamepad button: Fullscreen on / off"
    ),
    keys_binding!("keys.restart", "R", "Key: Restart"),
    binding!(
        "pad.restart",
        "none",
        PAD_CHOICES,
        "Gamepad button: Restart"
    ),
    keys_binding!("keys.strava", "S", "Key: Strava recording start / stop"),
    binding!(
        "pad.strava",
        "L3",
        PAD_CHOICES,
        "Gamepad button: Strava recording start / stop"
    ),
    keys_binding!("keys.god", "G", "Key: Unicorn mode on / off"),
    binding!(
        "pad.god",
        "none",
        PAD_CHOICES,
        "Gamepad button: Unicorn mode on / off"
    ),
    keys_binding!("keys.mute_mic", "M", "Key: Mute the microphone"),
    binding!(
        "pad.mute_mic",
        "Select",
        PAD_CHOICES,
        "Gamepad button: Mute the microphone"
    ),
    keys_binding!("keys.mute_voices", "Shift+M", "Key: Mute the voices"),
    binding!(
        "pad.mute_voices",
        "R3",
        PAD_CHOICES,
        "Gamepad button: Mute the voices"
    ),
    keys_binding!("keys.map", "N", "Key: Navigation map on / off"),
    binding!(
        "pad.map",
        "none",
        PAD_CHOICES,
        "Gamepad button: Navigation map on / off"
    ),
    keys_binding!(
        "keys.map_zoom",
        "Z",
        "Key: Navigation map zoom (auto, 1 - 7)"
    ),
    binding!(
        "pad.map_zoom",
        "DPadDown",
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
    // The page names its server (config.json, spec 11.5 *Join info*).
    "net.public_server",
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
    "host.voice",
    // The browser has no API upload (spec 2.8.1: the simple upload only).
    "strava.auto_upload",
    // The desktop's OpenXR headset and its monitor mirror (spec 14 Phase
    // 11); the browser's headset is WebXR (Enter VR on the page). The bike
    // computer's size and tilt apply in both.
    "vr.enabled",
    "vr.mirror",
    // Road traffic only in the desktop app (spec 14 Phase 18 Q9 (b));
    // aircraft and UFOs everywhere.
    "traffic.road",
    "traffic.manners",
    "traffic.live_road",
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
                } else if self.key == crate::public::SETTING {
                    // default, off or a join info URL.
                    crate::public::resolve(text).map(|_| Value::Text(text.to_string()))
                } else {
                    Ok(Value::Text(text.to_string()))
                }
            }
            Kind::Keys => crate::bindings::parse_keys(text)
                .map(Value::Text)
                .map_err(|e| format!("{}: {e}", self.key)),
        }
    }

    /// Converts a value from the settings file.
    fn value_from_toml(&self, v: &toml::Value) -> Result<Value, String> {
        match (self.kind, v) {
            (Kind::Bool, toml::Value::Boolean(b)) => Ok(Value::Bool(*b)),
            (Kind::Int { .. }, toml::Value::Integer(i)) => self.check(Value::Int(*i)),
            (Kind::Choice(_) | Kind::Text | Kind::Keys, toml::Value::String(s)) => self.parse(s),
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
            Kind::Keys => "a key, or two with a comma (A, Up, Delete,Q, F5, Shift+M, none)".into(),
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
            // From the first key on, one key (a list becomes one key).
            (Kind::Keys, Value::Text(s)) => {
                let first = crate::bindings::key_list(s).next().unwrap_or("none");
                Value::Text(
                    crate::bindings::step_choice(crate::bindings::KEY_CHOICES, first, dir).into(),
                )
            }
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

/// Settings that no longer exist; a file that still has them is read
/// without them (no warning) and saved without them: the keyboard's
/// power ±10 W on Page Up / Page Down (removed in v5.18, spec 3.2.2: the
/// keyboard's power is on ↑ / ↓ only, Page Down chooses the junction's
/// exit to the right).
pub const RETIRED: &[&str] = &[
    "keys.power_up",
    "keys.power_down",
    "pad.power_up",
    "pad.power_down",
];

/// Saved values that were the default before it changed: read as not set,
/// so the new default applies (`bind --reset` before v5.18 wrote every
/// default into the file). The junction exits' keys Q / E became
/// Delete / Page Down with Q / E kept as second keys (v5.18).
pub const OLD_DEFAULTS: &[(&str, &str)] = &[("keys.exit_left", "Q"), ("keys.exit_right", "E")];

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
        let mut s = Self {
            doc,
            session: BTreeMap::new(),
        };
        s.forget_retired();
        s.migrate_voice();
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

    /// Back to the default: the key leaves the file (an emptied section
    /// too) and the session; the next save writes the file without it.
    pub fn unset(&mut self, key: &str) -> Result<(), String> {
        let d = def(key).ok_or_else(|| unknown(key))?;
        let (section, name) = d.file_key();
        self.remove_entry(section, name);
        self.session.remove(d.key);
        Ok(())
    }

    fn remove_entry(&mut self, section: &str, name: &str) {
        if let Some(t) = self.doc.get_mut(section).and_then(|v| v.as_table_mut()) {
            t.remove(name);
            if t.is_empty() {
                self.doc.remove(section);
            }
        }
    }

    /// The voice settings before the Voice page's two rows (spec §7.1.7
    /// Q16), read into the new ones and dropped from the file (the next
    /// save writes only the new keys): `voice.heard_by` → `voice.my_voice`
    /// (`no_one` → everybody with My voice off, as every ride starts;
    /// `nearby` → everybody; `selected` → selected), `voice.room` →
    /// `voice.i_hear` (everybody / selected), `voice.enabled = off` →
    /// `voice.i_hear = nobody` (both rows off: the old voice off; My voice
    /// starts off anyway). A new key already in the file wins. The
    /// selected riders (`[voice_riders]`) are not touched.
    fn migrate_voice(&mut self) {
        let Some(voice) = self.doc.get_mut("voice").and_then(|v| v.as_table_mut()) else {
            return;
        };
        let text = |v: Option<toml::Value>| v.and_then(|v| v.as_str().map(str::to_owned));
        let heard_by = text(voice.remove("heard_by"));
        let room = text(voice.remove("room"));
        let enabled = voice.remove("enabled").and_then(|v| v.as_bool());
        let my_voice = heard_by
            .as_deref()
            .and_then(ocw_voice::audience::HeardBy::parse)
            .map(|h| ocw_voice::audience::MyVoice::from_heard_by(h).1);
        let i_hear = if enabled == Some(false) {
            Some(ocw_voice::audience::Listen::Nobody)
        } else {
            room.as_deref()
                .and_then(ocw_voice::audience::Listen::parse)
                .filter(|l| *l != ocw_voice::audience::Listen::Nobody)
        };
        // Only what differs from the defaults is written, as `set` would.
        if let Some(m) = my_voice.filter(|m| *m != Default::default()) {
            voice
                .entry("my_voice")
                .or_insert_with(|| toml::Value::String(m.key().into()));
        }
        if let Some(l) = i_hear.filter(|l| *l != Default::default()) {
            voice
                .entry("i_hear")
                .or_insert_with(|| toml::Value::String(l.key().into()));
        }
        if voice.is_empty() {
            self.doc.remove("voice");
        }
    }

    /// Drops the [`RETIRED`] settings and the saved [`OLD_DEFAULTS`].
    fn forget_retired(&mut self) {
        for key in RETIRED {
            let (section, name) = key.split_once('.').expect("section.name");
            self.remove_entry(section, name);
        }
        for (key, old) in OLD_DEFAULTS {
            let (section, name) = key.split_once('.').expect("section.name");
            let saved = self
                .doc
                .get(section)
                .and_then(|t| t.get(name))
                .and_then(|v| v.as_str());
            if saved.is_some_and(|v| v.eq_ignore_ascii_case(old)) {
                self.remove_entry(section, name);
            }
        }
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

    #[test]
    fn the_ground_detail_setting_defaults_to_the_preset() {
        // Spec 4.6.2 Q1: off / low / full override the quality preset,
        // auto (the default) follows it.
        let d = def("display.ground_detail").unwrap();
        assert_eq!(d.default, "auto");
        assert_eq!(d.kind, Kind::Choice(&["auto", "off", "low", "full"]));
        assert!(matches!(d.apply, Apply::Command("ground_detail {}")));
        let mut s = Settings::default();
        assert_eq!(
            s.get("display.ground_detail"),
            Ok(Value::Text("auto".into()))
        );
        s.set("display.ground_detail", "full").unwrap();
        assert_eq!(
            s.get("display.ground_detail"),
            Ok(Value::Text("full".into()))
        );
        assert!(s.set("display.ground_detail", "lots").is_err());
    }

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

    /// Host a ride has voice chat by default (spec §7.1.7 Q15), saved in
    /// `[host]` like the router switch; desktop only.
    #[test]
    fn host_voice_is_on_by_default_and_desktop_only() {
        let s = Settings::default();
        assert_eq!(s.get("host.voice").unwrap(), Value::Bool(true));
        assert!(applies("host.voice", crate::menu::model::Platform::Desktop));
        assert!(!applies(
            "host.voice",
            crate::menu::model::Platform::Browser
        ));
        let (mut s, w) = Settings::parse("[host]\nvoice = false\n").unwrap();
        assert!(w.is_empty(), "{w:?}");
        assert_eq!(s.get("host.voice").unwrap(), Value::Bool(false));
        s.set("host.voice", "on").unwrap();
        let text = s.to_toml();
        assert!(
            text.contains("[host]") && text.contains("voice = true"),
            "{text}"
        );
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

    /// Spec 14 Phase 18, T1: the `[traffic]` rows, Q1's first-start
    /// defaults, the file round trip, the apply commands, Q9's platforms.
    #[test]
    fn traffic_settings_round_trip() {
        use crate::menu::model::Platform;
        let s = Settings::default();
        for (k, v) in [
            ("traffic.enabled", "on"),
            ("traffic.road", "off"),
            ("traffic.manners", "polite"),
            ("traffic.air", "simulated"),
            ("traffic.ufo", "off"),
            ("traffic.sound", "on"),
            ("traffic.live_road", "off"),
        ] {
            assert_eq!(s.get(k).map(|v| v.to_string()).as_deref(), Ok(v), "{k}");
        }
        assert!(
            s.startup_commands().is_empty(),
            "defaults: nothing to apply"
        );
        let mut s = Settings::default();
        s.set("traffic.road", "lunatic").unwrap();
        s.set("traffic.manners", "some_realism").unwrap();
        s.set("traffic.ufo", "often").unwrap();
        s.set("traffic.air", "off").unwrap();
        assert!(s.set("traffic.road", "jammed").is_err());
        let text = s.to_toml();
        assert!(text.contains("[traffic]"), "{text}");
        let (back, warnings) = Settings::parse(&text).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert_eq!(back.get("traffic.road"), Ok(Value::Text("lunatic".into())));
        assert_eq!(back.get("traffic.ufo"), Ok(Value::Text("often".into())));
        assert_eq!(
            back.startup_commands(),
            [
                "traffic road lunatic",
                "traffic manners some_realism",
                "traffic air off",
                "traffic ufo often",
            ]
        );
        // Q9: road traffic only on the desktop, the sky everywhere.
        for k in ["traffic.road", "traffic.manners", "traffic.live_road"] {
            assert!(applies(k, Platform::Desktop) && !applies(k, Platform::Browser));
        }
        for k in [
            "traffic.enabled",
            "traffic.air",
            "traffic.ufo",
            "traffic.sound",
        ] {
            assert!(applies(k, Platform::Browser), "{k}");
        }
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
        assert!(
            def("voice.enabled").is_none(),
            "spec §7.1.7 Q16: no single voice off"
        );
        assert_eq!(s.get("voice.my_voice"), Ok(Value::Text("everybody".into())));
        assert_eq!(s.get("voice.i_hear"), Ok(Value::Text("everybody".into())));
    }

    /// Spec §7.1.7 Q16: the old voice settings (`ocw.toml` or the
    /// browser's `localStorage`, both read by `Settings::parse`) become
    /// the two rows without a warning, and the file keeps only the new
    /// keys; the selected riders stay.
    #[test]
    fn old_voice_settings_are_migrated() {
        let get = |text: &str| {
            let (s, w) = Settings::parse(text).unwrap();
            assert!(w.is_empty(), "{w:?}");
            let t = |k: &str| s.get(k).unwrap().to_string();
            (t("voice.my_voice"), t("voice.i_hear"), s.to_toml())
        };
        let (m, i, toml) = get("[voice]\nheard_by = \"nearby\"\nroom = \"selected\"\n");
        assert_eq!((m.as_str(), i.as_str()), ("everybody", "selected"));
        assert!(
            !toml.contains("heard_by") && !toml.contains("room"),
            "{toml}"
        );
        assert!(toml.contains("i_hear = \"selected\""), "{toml}");
        let (m, i, _) = get("[voice]\nheard_by = \"selected\"\n");
        assert_eq!((m.as_str(), i.as_str()), ("selected", "everybody"));
        // No one: My voice off, which every ride starts with anyway.
        let (m, _, toml) = get("[voice]\nheard_by = \"no_one\"\n");
        assert_eq!(m, "everybody");
        assert!(!toml.contains("voice"), "nothing left to keep: {toml}");
        // The old voice off: both rows off (I hear nobody; My voice starts
        // off), whatever the room said.
        let (_, i, toml) = get("[voice]\nenabled = false\nroom = \"selected\"\nvolume = 80\n");
        assert_eq!(i, "nobody");
        assert!(
            !toml.contains("enabled") && toml.contains("volume = 80"),
            "{toml}"
        );
        let (_, i, _) = get("[voice]\nenabled = true\n");
        assert_eq!(i, "everybody");
        // A new key already there wins.
        let (m, i, _) = get(
            "[voice]\nheard_by = \"selected\"\nmy_voice = \"everybody\"\nroom = \"selected\"\ni_hear = \"nobody\"\n",
        );
        assert_eq!((m.as_str(), i.as_str()), ("everybody", "nobody"));
        // The selection is kept.
        let (s, _) = Settings::parse(
            "[voice]\nheard_by = \"selected\"\n\n[voice_riders.\"name:local/Bea Climber\"]\nname = \"Bea Climber\"\nselected = true\n",
        )
        .unwrap();
        assert!(s.voice_riders()["name:local/Bea Climber"].selected);
        assert_eq!(s.get("voice.my_voice"), Ok(Value::Text("selected".into())));
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

    /// `display.damage` is gone (the user, 2026-11-19: the damage widget
    /// always shows while damage is on): an old value in the file is an
    /// unknown setting, noted and kept, and runs nothing.
    #[test]
    fn the_damage_indicator_switch_is_gone() {
        assert!(def("display.damage").is_none());
        assert!(
            SETTINGS
                .iter()
                .all(|d| !matches!(d.apply, Apply::Command(c) if c.starts_with("hud.damage")))
        );
        let (mut s, warnings) =
            Settings::parse("[display]\ndamage = false\n[ride]\ndamage = true\n").unwrap();
        assert_eq!(
            warnings,
            ["unknown setting display.damage (kept in the file)"]
        );
        assert!(s.get("display.damage").is_err());
        assert!(s.set("display.damage", "on").is_err());
        assert!(
            s.startup_commands()
                .iter()
                .all(|c| !c.contains("hud.damage"))
        );
        assert!(s.to_toml().contains("damage = false"), "kept in the file");
        assert_eq!(s.get("ride.damage"), Ok(Value::Bool(true)));
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

//! The Settings and Workshop pages of the in-game menu (spec §3.2.1, M2)
//! as a Bevy-free model: each page is a list of rows; a row shows a label
//! and a value and says which console command line ← / → (change the
//! value) and Confirm run, or which page Confirm opens. Menu, console and
//! scripts therefore share one code path, and each row's command is
//! testable without Bevy.
//!
//! Integration point for the menu shell (M1): a menu page for Settings →
//! Controls / Trainer / Strava / Display / Riders and for the Workshop
//! renders [`rows`] for its [`Page`], maps the abstract actions Left /
//! Right / Confirm onto [`Row::left`] / [`Row::right`] / [`Row::confirm`]
//! and queues the command lines with `ConsoleState::queue`; `menu::model`
//! does this ([`crate::menu::model::Page::Sub`]).

use crate::riders::Riders;
use crate::settings::{Kind, Settings, def};

/// A page of the Settings or Workshop menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Controls,
    Trainer,
    Strava,
    Display,
    /// Volumes and the output device (M5; the devices come from the audio
    /// engine, see [`audio_rows`]).
    Audio,
    Riders,
    /// Editing one rider (index into the profiles).
    Rider(usize),
    Workshop,
    /// A rider's bike and look (index into the profiles; Workshop → Bike).
    Bike(usize),
    /// Workshop → Character (spec §14 Phase 4, 4N N10; rows from
    /// `menu::character`, for the active rider): its categories.
    Character,
    /// … the body: base body, shape (any; weight and height propose one,
    /// *Match my weight*).
    CharacterBody,
    /// … face and age: the appearance age, skin, *Grey hair with age* (off),
    /// glasses.
    CharacterFace,
    /// … hair: style and colour.
    CharacterHair,
    /// … facial hair: moustache, beard, their colour.
    CharacterFacialHair,
    /// One attribute's choices as tiles (4N *Visual editor*): a grid, the
    /// focused tile shown on the rider at once, Confirm applies it.
    CharacterPick(super::character::Attr),
    /// World (top level, spec §3.2.1): change world, about this world,
    /// routes, time, weather (rows from `ride::world`, like the World
    /// pages below).
    WorldHome,
    /// World → Change world: the worlds found and the test routes.
    Worlds,
    /// One of them (index into the list): its default ride, free ride,
    /// routes, parts.
    World(usize),
    /// The routes of a world not ridden now (from its package summary).
    WorldRoutes(usize),
    /// The neighbouring packages of a world (spec §2.2.2).
    WorldParts(usize),
    /// World → About this world.
    WorldInfo,
    /// World → About → Sun and time at the rider.
    WorldSun,
    /// World → About → Data sources (the credits, spec §13).
    WorldSources,
    /// One source with its full notice (index into the credits).
    WorldSource(usize),
    /// World → Weather: the weather machine (spec §14 Phase 9, S5; rows
    /// from `weather`): mode, presets, custom, transition, random weather.
    Weather,
    /// Weather → Presets: the 17 presets as a grid with icons.
    WeatherPresets,
    /// Weather → Custom: the sky (clouds, precipitation, fog).
    WeatherCustom,
    /// Custom → Wind and temperature.
    WeatherAir,
    /// Weather → Real weather (spec §14 Phase 9, S7): on / off, the data
    /// source, ride a day; the attribution under the title.
    WeatherReal,
    /// Navi → Routes: the routes of the loaded package and the user's.
    Routes,
    /// Navi → Rides: the saved rides with their Strava state.
    Rides,
    /// Navi → GPX import: GPX files to match onto the loaded package.
    GpxImport,
    /// Ride → Workouts: start, skip and stop structured workouts.
    Workouts,
    /// Settings → Voice (M8; rows from `voice::page`): mute switches,
    /// volume, participants, devices.
    Voice,
    /// Voice → Devices and options.
    VoiceDevices,
    /// Voice → Participants: the riders in the voice room, each opens its
    /// page; the blocked riders at the end.
    VoiceParticipants,
    /// One participant (index into the voice room's list, spec §7.1.7
    /// Q9): Mute for me, the rider's volume, Block.
    VoiceRider(usize),
    /// Participants → Blocked riders: every rider blocked, here or not;
    /// Confirm unblocks.
    VoiceBlocked,
    /// Audio → Voice: the voice volume and the echo test (spec §7 *Echo
    /// bot*; rows from `voice::page`, the Audio page is full).
    AudioVoice,
    /// World → Time machine (spec §14 Phase 9; rows from `timemachine`).
    TimeMachine,
    /// World → Time → Calendar (spec §14 Phase 9 S9a; rows and the grid
    /// from `menu::calendar`): the month shown, the focus day to open on,
    /// the time of day of the stepper.
    Calendar(super::calendar::Cal),
    /// Calendar → Dates: today, the equinoxes and solstices, the season
    /// starts of the place (in the shown year).
    CalendarDates(super::calendar::Cal),
    /// World → Time → Season (S9b, decision Q11): the seasons of the
    /// place as date jumps.
    Season,
    /// Controls → Riding aids (spec §14 Phase 12C / 12D): the proximity
    /// cue and arrow, the "last moment" warning, the camera shake.
    RidingAids,
    /// Display → Graphics (spec §14 Phase 10 B7): the quality preset, the
    /// sky, fullscreen (the Display page is full).
    Graphics,
    /// Graphics → Render distance (spec §14 Phase 10, 10G; rows from
    /// `view_distance::rows`): the slider, the dynamic mode, the target.
    ViewDistance,
    /// Navi → Riders in this ride (spec §14 Phase 13E, 12F; rows from
    /// `rider_list`): who rides, where, who flies, who is hurt.
    RiderList,
    /// Settings → Account (spec §9.1; rows from `crate::account`): sign in
    /// with Google or Strava, with a code on a kiosk, link the other
    /// provider, sign out.
    Account,
    /// Controls → Touch and unicorn: touch steering (spec §3.2.3: auto /
    /// on / off, the zones' height), the unicorn's mouse and flight (the
    /// Controls page is full).
    UnicornControls,
    /// Controls → Keys and buttons (spec §3.2.1 M9): the groups, the pad's
    /// Confirm button, reset.
    Bindings,
    /// One group of bindings (index into `bindings::Group::ALL`).
    BindGroup(usize),
    /// Keys and buttons → Reset to defaults (asks first).
    BindReset,
    /// Display → Accessibility (M9): HUD size, status colours, contrast,
    /// reduced motion, camera shake.
    Accessibility,
    /// Navi → Training history (spec §14 Phase 7; rows from
    /// `menu::history`): rides, totals, personal bests.
    History,
    /// The rides, newest first.
    HistoryRides,
    /// One ride (index: 0 = the newest) with its power curve.
    HistoryRide(usize),
    /// The climbs of a ride.
    HistoryClimbs(usize),
    /// Weekly totals.
    HistoryWeeks,
    /// Monthly totals.
    HistoryMonths,
    /// Personal bests: the power curve and the climbs.
    HistoryBests,
    /// "Start the event?" → Organiser rules (spec §14 Phase 12 K6): the
    /// rules of the events this rider starts (the `event.*` settings).
    EventRules,
    /// Settings → Account → Trusted world keys (spec §4.7 *User keys*;
    /// rows from `menu::trust`): the user keys trusted here.
    TrustedKeys,
    /// One of them (index): name, fingerprint, date; remove it.
    TrustedKey(usize),
    /// Settings → Trainer → Devices (spec §3.2.1 *Devices*; rows from
    /// `menu::devices`): scan, the devices found, connected and
    /// remembered.
    Devices,
    /// One of them (index into the list): connect or forget it.
    Device(usize),
    /// Ride → Join (spec §3.2.1; rows from `menu::join`): the link and
    /// fingerprint fields, Paste, Join, the recent servers.
    Join,
    /// Ride → Host a ride (rows from `crate::host`): the world …
    HostWorld,
    /// … the route or free ride …
    HostRoute,
    /// … date and time …
    HostTime,
    /// … the weather …
    HostWeather,
    /// … bots, access, port, sign-in …
    HostOptions,
    /// … and the summary with Start hosting.
    HostSummary,
    /// While hosting: the join link (Copy link, Stop hosting).
    HostLink,
}

impl Page {
    /// The pages with a tab (Settings sub pages and the Workshop).
    pub const TABS: [Page; 7] = [
        Page::Workshop,
        Page::Riders,
        Page::Controls,
        Page::Trainer,
        Page::Audio,
        Page::Display,
        Page::Strava,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Self::Controls => "Controls",
            Self::Trainer => "Trainer",
            Self::Strava => "Strava",
            Self::Display => "Display",
            Self::Audio => "Audio",
            Self::Riders => "Riders",
            Self::Rider(_) => "Rider",
            Self::Workshop => "Workshop",
            Self::Bike(_) => "Bike",
            Self::Character => "Character",
            Self::CharacterBody => "Character: body",
            Self::CharacterFace => "Character: face and age",
            Self::CharacterHair => "Character: hair",
            Self::CharacterFacialHair => "Character: facial hair",
            Self::CharacterPick(a) => a.title(),
            Self::WorldHome => "World",
            Self::Worlds => "Change world",
            Self::World(_) => "World",
            Self::WorldRoutes(_) => "Routes of the world",
            Self::WorldParts(_) => "Parts of the world",
            Self::WorldInfo => "About this world",
            Self::WorldSun => "Sun and time",
            Self::WorldSources => "Data sources",
            Self::WorldSource(_) => "Data source",
            Self::Weather => "Weather machine",
            Self::WeatherPresets => "Weather presets",
            Self::WeatherCustom => "Custom weather: sky",
            Self::WeatherAir => "Custom: wind, temperature",
            Self::WeatherReal => "Real weather",
            Self::Routes => "Routes",
            Self::Rides => "Rides",
            Self::GpxImport => "GPX import",
            Self::Workouts => "Workouts",
            Self::Voice => "Voice",
            Self::VoiceDevices => "Voice devices",
            Self::VoiceParticipants => "Voice participants",
            Self::VoiceRider(_) => "Voice: rider",
            Self::VoiceBlocked => "Blocked riders",
            Self::AudioVoice => "Audio: voice",
            Self::TimeMachine => "Time machine",
            Self::Calendar(_) => "Calendar",
            Self::CalendarDates(_) => "Calendar: dates",
            Self::Season => "Season",
            Self::RidingAids => "Riding aids",
            Self::Graphics => "Graphics",
            Self::ViewDistance => "Render distance",
            Self::RiderList => "Riders in this ride",
            Self::Account => "Account",
            Self::UnicornControls => "Touch and unicorn mode",
            Self::Bindings => "Keys and buttons",
            Self::BindGroup(g) => crate::bindings::Group::ALL
                .get(g)
                .map_or("Keys", |g| g.label()),
            Self::BindReset => "Reset keys and buttons?",
            Self::Accessibility => "Accessibility",
            Self::History => "Training history",
            Self::HistoryRides => "Rides",
            Self::HistoryRide(_) => "Ride",
            Self::HistoryClimbs(_) => "Climbs of the ride",
            Self::HistoryWeeks => "Weekly totals",
            Self::HistoryMonths => "Monthly totals",
            Self::HistoryBests => "Personal bests",
            Self::EventRules => "Organiser rules",
            Self::TrustedKeys => "Trusted world keys",
            Self::TrustedKey(_) => "Trusted key",
            Self::Devices => "Devices",
            Self::Device(_) => "Device",
            Self::Join => "Join a ride",
            Self::HostWorld => "Host a ride: world",
            Self::HostRoute => "Host a ride: route",
            Self::HostTime => "Host a ride: date and time",
            Self::HostWeather => "Host a ride: weather",
            Self::HostOptions => "Host a ride: options",
            Self::HostSummary => "Host a ride",
            Self::HostLink => "Hosting: join link",
        }
    }

    /// The Ride and Navi lists (rows from `ride::pages`) and the World
    /// pages: lists without values to change.
    pub fn is_ride_page(self) -> bool {
        matches!(
            self,
            Self::Routes
                | Self::Rides
                | Self::GpxImport
                | Self::Workouts
                | Self::HostWorld
                | Self::HostRoute
        ) || self.is_world_page()
    }

    /// The World pages (rows from `ride::world`).
    pub fn is_world_page(self) -> bool {
        matches!(
            self,
            Self::WorldHome
                | Self::Worlds
                | Self::World(_)
                | Self::WorldRoutes(_)
                | Self::WorldParts(_)
                | Self::WorldInfo
                | Self::WorldSun
                | Self::WorldSources
                | Self::WorldSource(_)
        )
    }

    /// The weather machine's pages (rows from `weather`, spec §14 Phase 9,
    /// S5).
    pub fn is_weather_page(self) -> bool {
        matches!(
            self,
            Self::Weather
                | Self::WeatherPresets
                | Self::WeatherCustom
                | Self::WeatherAir
                | Self::WeatherReal
        )
    }

    /// Pages laid out as a grid of this many columns (the preset grid);
    /// the arrows move in two directions, Back is the last cell.
    pub fn grid_columns(self) -> Option<usize> {
        match self {
            Self::WeatherPresets => Some(super::weather::GRID_COLUMNS),
            // The month grid moves by its own rules (`calendar::navigate`).
            Self::Calendar(_) => Some(super::calendar::COLUMNS),
            // The character editor's tiles (spec §14 Phase 4, 4N).
            Self::CharacterPick(_) => Some(super::character::TILE_COLUMNS),
            _ => None,
        }
    }

    /// The training history's pages (rows from `menu::history`).
    pub fn is_history_page(self) -> bool {
        matches!(
            self,
            Self::History
                | Self::HistoryRides
                | Self::HistoryRide(_)
                | Self::HistoryClimbs(_)
                | Self::HistoryWeeks
                | Self::HistoryMonths
                | Self::HistoryBests
        )
    }

    /// The Keys and buttons pages (rows from [`binding_rows`]).
    pub fn is_binding_page(self) -> bool {
        matches!(self, Self::Bindings | Self::BindGroup(_) | Self::BindReset)
    }

    /// Whether `platform` has this page (both have every page since the
    /// browser flies too, Phase 13 G5; kept for pages one app lacks).
    pub fn on(self, _platform: super::model::Platform) -> bool {
        true
    }

    /// The voice pages (rows from `voice::page`).
    pub fn is_voice_page(self) -> bool {
        matches!(
            self,
            Self::Voice
                | Self::VoiceDevices
                | Self::VoiceParticipants
                | Self::VoiceRider(_)
                | Self::VoiceBlocked
                | Self::AudioVoice
        )
    }

    /// One page up (the rider editor goes back to the list).
    pub fn parent(self) -> Option<Self> {
        match self {
            Self::Rider(_) => Some(Self::Riders),
            Self::Bike(_) | Self::Character => Some(Self::Workshop),
            Self::CharacterBody
            | Self::CharacterFace
            | Self::CharacterHair
            | Self::CharacterFacialHair => Some(Self::Character),
            Self::CharacterPick(a) => Some(a.page()),
            Self::World(_) => Some(Self::Worlds),
            Self::WorldRoutes(i) | Self::WorldParts(i) => Some(Self::World(i)),
            Self::WorldSource(_) => Some(Self::WorldSources),
            Self::VoiceDevices | Self::VoiceParticipants => Some(Self::Voice),
            Self::VoiceRider(_) | Self::VoiceBlocked => Some(Self::VoiceParticipants),
            Self::AudioVoice => Some(Self::Audio),
            Self::RidingAids | Self::UnicornControls | Self::Bindings => Some(Self::Controls),
            Self::BindGroup(_) | Self::BindReset => Some(Self::Bindings),
            Self::Graphics | Self::Accessibility => Some(Self::Display),
            Self::ViewDistance => Some(Self::Graphics),
            Self::TrustedKey(_) => Some(Self::TrustedKeys),
            Self::Devices => Some(Self::Trainer),
            Self::Device(_) => Some(Self::Devices),
            Self::HostRoute => Some(Self::HostWorld),
            Self::HostTime => Some(Self::HostRoute),
            Self::HostWeather => Some(Self::HostTime),
            Self::HostOptions => Some(Self::HostWeather),
            Self::HostSummary => Some(Self::HostOptions),
            _ => None,
        }
    }

    /// Short explanation shown above the rows.
    pub fn notes(self) -> &'static [&'static str] {
        match self {
            Self::Workshop => &[
                "Choose who rides and their bike: weight and bike go into the physics and the trainer (from the next start).",
            ],
            Self::Bike(_) => &[
                "Bike type and setup choose the physics profile (differences capped); colours are only for the looks. From the next start.",
            ],
            Self::Riders => &["Rider profiles: weight, height, FTP. Choose one in the Workshop."],
            Self::Character => &[
                "Who you are on the road: free, for everyone, and never physics (the weight stays the profile's).",
            ],
            Self::CharacterBody => {
                &["Any shape; Match my weight picks the one your weight and height suggest."]
            }
            Self::Rider(_) => &["Changes apply from the next start (Restart)."],
            Self::CharacterFace => &["Age shows on face, body, skin and hair."],
            Self::CharacterHair => &[
                "Every style fits under every helmet. Any colour: character --hair-colour #RRGGBB.",
            ],
            Self::CharacterFacialHair => {
                &["The colour follows your hair (greying too) or is its own."]
            }
            Self::CharacterPick(_) => &["Confirm keeps the focused one."],
            Self::Trainer => &["Devices, SIM difficulty and the virtual trainer."],
            Self::Strava => &[
                "Uploads saved rides to Strava when an account is linked (API credentials: see README); the sync state is in Navi -> Rides.",
            ],
            Self::Audio => &[
                "Sounds are made by OCW itself; Enter on a volume plays a test sound. Output: the device (default = the system's).",
            ],
            Self::Controls | Self::Display => &[],
            Self::WorldHome => &[
                "The world you ride in: change it without restarting, its data and credits, the time of day and the weather.",
            ],
            Self::Worlds => &[
                "Installed world packages and the files next to the one ridden now; then the test routes. The ride so far is saved.",
            ],
            Self::World(_) => &[
                "Ride: its first route, else a free ride. In a shared ride only the host changes the world.",
            ],
            Self::WorldRoutes(_) => &["Routes of this world package. Choose one to ride it."],
            Self::WorldParts(_) => &[
                "Neighbouring packages: the roads continue into them. Start a free ride in one of them.",
            ],
            Self::WorldInfo => &["The world package ridden now and where its data comes from."],
            Self::WorldSun => &[
                "Today at your place in the world, in its local time (the date of the time machine).",
            ],
            Self::WorldSources => {
                &["The credits of the world's data. Enter shows the full notice."]
            }
            Self::WorldSource(_) => &[],
            Self::Weather => &[super::weather::MACHINE_NOTE],
            Self::WeatherPresets => &[],
            Self::WeatherCustom | Self::WeatherAir => &[super::weather::CUSTOM_NOTE],
            Self::WeatherReal => &[super::weather::REAL_NOTE],
            Self::Routes => &[
                "Routes of the loaded world package and your imported ones (GPX import). Choose one to ride it.",
            ],
            Self::Rides => &["Saved rides (FIT files), newest first, with their Strava state."],
            Self::GpxImport => &[
                "GPX files from the gpx folder of the OCW data folder and from Downloads, matched onto the roads of the loaded world package.",
            ],
            Self::Voice => &[
                "Voice chat is always on, like a call: shared rides join you automatically. M mutes your microphone, Shift+M the voices (pad: Select, R3).",
            ],
            Self::VoiceDevices => &[
                "Microphone and speaker for voice chat; a missing device falls back to the default.",
            ],
            Self::VoiceParticipants => &[
                "Riders in the voice room: Confirm opens one (mute for you, volume, block). Blocked riders at the end.",
            ],
            Self::VoiceRider(_) => &[
                "Mute: for this ride. Volume and Block are remembered. Blocked: neither of you hears the other; they are not told.",
            ],
            Self::VoiceBlocked => &["Riders you blocked, here or not. Confirm unblocks one."],
            Self::AudioVoice => &[
                "Voice volume (Enter plays a test sound). Echo test: an Echo rider on the road plays your microphone back to you, delayed, when you ride near it.",
            ],
            Self::TimeMachine => &[
                "Date, time and speed of the world's clock (sun, light, season). Enter on Day: the calendar. Solo you set it; in a shared ride the host does.",
            ],
            // The grid needs the room (spec §14 Phase 9 S9a).
            Self::Calendar(_) => &[],
            Self::CalendarDates(_) => {
                &["Dates of the shown year at your place and altitude; the time of day stays."]
            }
            Self::Season => &[
                "Each season's day at your place and altitude: the date jumps there and the time of day stays, so sun and landscape always match.",
            ],
            Self::ViewDistance => &[
                "How far the world is drawn. Adjusted: shorter below the target, longer with room, up to the slider (Confirm: the preset's).",
            ],
            Self::Graphics => &[
                "Quality: Low and Medium draw fewer shadows, trees and houses for weaker GPUs, WebGL 2 and phones; auto chooses by GPU and device.",
            ],
            Self::RidingAids => &[
                "Aids that never steer: an arrow to a rider close by, a warning before a hit or a slide. Shake: off for comfort.",
            ],
            Self::RiderList => {
                &["The unicorn: flying in unicorn mode. The dot: hurt, or the bike damaged."]
            }
            Self::EventRules => &[
                "The rules of the events you start; riders cannot change them until the event is over. Auto: by the kind of event (races: no riding aids).",
            ],
            Self::Workouts => &[
                "Structured workouts in ERG mode: yours from the workouts folder (.toml, .erg, .mrc), then the examples. % FTP targets use the rider's FTP.",
            ],
            Self::Account => &[crate::account::NOTE],
            Self::UnicornControls => &[
                "Touch: tap or hold a side to steer. Unicorn: cursor keys and + / -, or the gamepad.",
            ],
            Self::Bindings => &[
                "One key and one gamepad button per action. The menu keys, the sticks, the triggers and unicorn mode's flight keys stay fixed.",
            ],
            Self::BindGroup(_) => &[
                "Enter: press the new key or button (Esc / Start: cancel). Left / Right: step through the keys.",
            ],
            Self::BindReset => &["Every key and button back to its default. Back keeps them."],
            Self::Accessibility => &[
                "Status colours always come with an icon and a word. Reduced motion: no camera shake, no flashing warnings.",
            ],
            Self::History => &[
                "Your rides (this rider profile), the totals by week and month and your personal bests. Kept on this device.",
            ],
            Self::HistoryRides => &["Newest first. Choose one for its details and power curve."],
            Self::HistoryRide(_) | Self::HistoryClimbs(_) => &[],
            Self::HistoryWeeks | Self::HistoryMonths => &[],
            Self::HistoryBests => &[],
            Self::TrustedKeys => &[super::trust::KEYS_NOTE],
            Self::TrustedKey(_) => &[super::trust::KEY_NOTE],
            Self::Devices => &[super::devices::NOTE],
            Self::Device(_) => &[super::devices::DEVICE_NOTE],
            Self::Join => &[
                "Paste or type the link the host shares (ocw://HOST:PORT?cert=...). Your ride goes on until the shared one is ready.",
            ],
            Self::HostWorld => &[
                "Host a shared ride on this computer: others join with its link. First the world.",
            ],
            Self::HostRoute => &["The route, or a free ride on the world's roads."],
            Self::HostTime => {
                &["Real time, or a moment of the time machine (Left/Right step), a few days on."]
            }
            Self::HostWeather => {
                &["Simulated, real weather at the place, or a preset of the weather machine."]
            }
            Self::HostOptions => &[
                "Others may need UDP open in the firewall and a port forwarding on the router (README).",
            ],
            Self::HostSummary => &[
                "The server and its voice server run in this app on all addresses; you ride on it as the host.",
            ],
            Self::HostLink => &[],
        }
    }
}

/// A row label of at most this many characters fits in two lines.
pub const MAX_ROW_CHARS: usize = 48;

/// Shortens `s` to `max` characters (labels must fit the panel).
pub fn short_label(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max - 3).collect();
        t.push_str("...");
        t
    }
}

/// A row that only shows a value.
pub fn info_row(label: &str, value: Option<String>) -> Row {
    Row {
        label: label.into(),
        value,
        left: None,
        right: None,
        confirm: None,
    }
}

/// A row that opens `page`.
pub fn open_row(label: &str, value: Option<String>, page: Page) -> Row {
    Row {
        label: label.into(),
        value,
        left: None,
        right: None,
        confirm: Some(Activate::Open(page)),
    }
}

/// A workout of the Workouts list.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkoutEntry {
    pub name: String,
    /// The duration: "45:00".
    pub detail: String,
    pub source: WorkoutSource,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WorkoutSource {
    /// A file of `<data dir>/ocw/workouts` (the desktop).
    File(std::path::PathBuf),
    /// A bundled example, by name.
    Example(String),
}

impl WorkoutSource {
    /// The console line that loads and starts it.
    pub fn line(&self) -> String {
        match self {
            Self::File(p) => format!("workout start \"{}\"", p.display()),
            Self::Example(name) => format!("workout start --example {name}"),
        }
    }
}

/// The bundled example workouts as list entries (both apps; the desktop
/// lists the user's files first).
pub fn example_workouts() -> Vec<WorkoutEntry> {
    crate::workout::EXAMPLES
        .iter()
        .filter_map(|(file, text)| {
            let w = ocw_sim::workout::Workout::parse_file(file, text).ok()?;
            Some(WorkoutEntry {
                name: w.name.clone(),
                detail: ocw_sim::workout::format_duration(w.duration_s()),
                source: WorkoutSource::Example(crate::workout::example_name(file).into()),
            })
        })
        .collect()
}

/// Ride → Workouts: the running workout (its step, Skip, Stop), then the
/// list; `now` is (name, "step 2/6").
pub fn workout_rows(now: Option<&(String, String)>, list: &[WorkoutEntry]) -> Vec<Row> {
    let mut out = Vec::new();
    if let Some((name, step)) = now {
        out.push(info_row(&short_label(name, 24), Some(step.clone())));
        out.push(Row {
            label: "Skip step".into(),
            value: None,
            left: None,
            right: None,
            confirm: Some(Activate::Command("workout skip".into())),
        });
        out.push(Row {
            label: "Stop workout".into(),
            value: None,
            left: None,
            right: None,
            confirm: Some(Activate::Run("workout stop".into())),
        });
    }
    out.extend(list.iter().map(|w| Row {
        label: short_label(&w.name, 24),
        value: Some(match w.source {
            WorkoutSource::File(_) => w.detail.clone(),
            WorkoutSource::Example(_) => format!("{}, example", w.detail),
        }),
        left: None,
        right: None,
        confirm: Some(Activate::Run(w.source.line())),
    }));
    if out.is_empty() {
        out.push(info_row("No workouts found", Some("see README".into())));
    }
    out
}

/// What the top World page shows of the world ridden now.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct WorldHome {
    /// The world's name ("Liechtenstein", "Test route: Hills").
    pub name: String,
    /// Routes of the world (package and user routes).
    pub routes: usize,
    /// The app can switch worlds and list routes in the ride (the
    /// desktop; the browser rides the world of its page).
    pub switch: bool,
    /// God mode's row (spec §14 Phase 13A): its label, when the app has it.
    pub god: Option<&'static str>,
}

/// The rows of the top World page (spec §3.2.1): change world (or just
/// its name), about it, routes, Time, Weather, god mode.
pub fn world_home_rows(
    home: &WorldHome,
    tm: &super::time::TimeMachineInfo,
    weather: &super::weather::WeatherState,
) -> Vec<Row> {
    let time = match (&tm.time, tm.mode_label.as_str()) {
        (Some(t), "real") if tm.speed == 1.0 && !tm.paused => t.clone(),
        (Some(t), m) => format!(
            "{t}, {}",
            if tm.paused || tm.speed != 1.0 {
                ocw_sky::clock::speed_label(tm.speed, tm.paused)
            } else {
                m.to_owned()
            }
        ),
        (None, _) => "-".into(),
    };
    let mut out = Vec::new();
    if home.switch {
        out.push(open_row(
            "Change world",
            Some(short_label(&home.name, 28)),
            Page::Worlds,
        ));
        out.push(open_row("About this world", None, Page::WorldInfo));
        out.push(open_row(
            "Routes",
            Some(home.routes.to_string()),
            Page::Routes,
        ));
    } else {
        out.push(info_row("World", Some(short_label(&home.name, 28))));
    }
    out.push(open_row("Time", Some(time), Page::TimeMachine));
    out.push(open_row("Weather", Some(weather.summary()), Page::Weather));
    if let Some(label) = home.god {
        // God mode (spec §14 Phase 13A; the Ride page is full, the
        // ten-foot rule): fly over this world as a unicorn.
        out.push(Row {
            label: label.into(),
            value: Some("fly over it (G)".into()),
            left: None,
            right: None,
            confirm: Some(Activate::Run("god toggle".into())),
        });
    }
    out
}

/// What Confirm does on a row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Activate {
    /// Run a console command line.
    Command(String),
    /// Ask first ("Change the ride?"), then run the line and close the menu.
    Ask(String),
    /// Open a page.
    Open(Page),
    /// Nothing: shown dimmed (not available yet, or not yours to change,
    /// spec §14 Phase 9, S5).
    Disabled,
    /// Run a console command line and close the menu (starting a workout:
    /// its panel shows at once).
    Run(String),
    /// Ask "Remove this key?" first, then run the line and go back
    /// (Settings → Account → Trusted world keys).
    AskForget(String),
    /// Run a console line and go back one page (Devices → a device →
    /// Disconnect and forget: the device may leave the list).
    CommandBack(String),
    /// Run a console command line, then open a page (the host wizard's
    /// steps).
    RunOpen(String, Page),
}

/// One row of a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub label: String,
    /// The current value, if the row has one.
    pub value: Option<String>,
    /// Command line for ← (one step down) and → (one step up).
    pub left: Option<String>,
    pub right: Option<String>,
    pub confirm: Option<Activate>,
}

impl Row {
    /// A row that runs a console line.
    pub fn action(label: &str, line: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: None,
            left: None,
            right: None,
            confirm: Some(Activate::Command(line.into())),
        }
    }
}

/// A row for a registered setting: ← / → step it (`set KEY VALUE`);
/// Confirm toggles a switch or moves a choice on.
pub fn setting_row(settings: &Settings, key: &str, label: &str) -> Row {
    let d = def(key).expect("a registered setting");
    let v = settings.get(key).expect("registered");
    let set = |dir| format!("set {key} {}", d.step(&v, dir));
    let confirm = match d.kind {
        Kind::Bool | Kind::Choice(_) => Some(Activate::Command(set(1))),
        Kind::Int { .. } | Kind::Text => None,
    };
    Row {
        label: label.into(),
        value: Some(v.to_string()),
        left: Some(set(-1)),
        right: Some(set(1)),
        confirm,
    }
}

/// The Riding aids row's value: which aids are on ("warning, shake").
fn aids_summary(settings: &Settings) -> String {
    let on = |k: &str| settings.get(k).ok().and_then(|v| v.as_bool()) == Some(true);
    let names: Vec<&str> = [
        ("ride.proximity_cue", "arrow"),
        ("ride.hit_warning", "warning"),
        ("display.camera_shake", "shake"),
    ]
    .into_iter()
    .filter(|(k, _)| on(k))
    .map(|(_, n)| n)
    .collect();
    if names.is_empty() {
        "none".into()
    } else {
        names.join(", ")
    }
}

/// The Accessibility row's value: what is on ("125 %, colour-safe").
fn access_summary(settings: &Settings) -> String {
    let get = |k: &str| settings.get(k).map(|v| v.to_string()).unwrap_or_default();
    let mut out = Vec::new();
    if get("display.ui_scale") != "100" {
        out.push(format!("{} %", get("display.ui_scale")));
    }
    if get("display.palette") != "standard" {
        out.push(get("display.palette"));
    }
    if get("display.high_contrast") == "on" {
        out.push("contrast".into());
    }
    if get("display.reduced_motion") == "on" {
        out.push("less motion".into());
    }
    if out.is_empty() {
        "standard".into()
    } else {
        out.join(", ")
    }
}

/// The Keys and buttons row's value: "defaults", "changed", "2 conflicts".
pub fn bindings_summary(settings: &Settings, desktop: bool) -> String {
    let b = crate::bindings::Bindings::from_settings(settings);
    let n = b.conflicts(desktop).len();
    match n {
        0 if b.is_default() => "defaults".into(),
        0 => "changed".into(),
        1 => "1 conflict".into(),
        n => format!("{n} conflicts"),
    }
}

/// The rows of the Keys and buttons pages (spec §3.2.1 M9): the groups,
/// the pad's Confirm button and reset; a group's actions, each "key |
/// button" – ← / → step the key, Confirm waits for the new key or button
/// (`bind --capture ID`); `desktop`: the platform has the desktop's
/// actions (the browser leaves those out).
pub fn binding_rows(page: Page, settings: &Settings, desktop: bool) -> Vec<Row> {
    use crate::bindings::{
        ACTIONS, Bindings, Device, Group, KEY_CHOICES, key_label, pad_label, step_choice,
    };
    let b = Bindings::from_settings(settings);
    let conflicts = b.conflicts(desktop);
    match page {
        Page::Bindings => {
            let mut out: Vec<Row> = Group::ALL
                .iter()
                .enumerate()
                .map(|(i, g)| {
                    let n = conflicts
                        .iter()
                        .filter(|c| c.action.def().group == *g)
                        .count();
                    let actions = ACTIONS
                        .iter()
                        .filter(|d| d.group == *g && (desktop || !d.desktop_only))
                        .count();
                    open_row(
                        g.label(),
                        Some(match n {
                            0 => format!("{actions} actions"),
                            1 => "1 conflict".into(),
                            n => format!("{n} conflicts"),
                        }),
                        Page::BindGroup(i),
                    )
                })
                .collect();
            out.push(setting_row(
                settings,
                "controls.pad_confirm",
                "Pad Confirm button",
            ));
            out.push(open_row(
                "Reset to defaults",
                Some(if b.is_default() {
                    "nothing changed".into()
                } else {
                    "changed".into()
                }),
                Page::BindReset,
            ));
            out
        }
        Page::BindGroup(g) => {
            let Some(group) = Group::ALL.get(g) else {
                return Vec::new();
            };
            ACTIONS
                .iter()
                .filter(|d| d.group == *group && (desktop || !d.desktop_only))
                .map(|d| {
                    let key = b.key(d.action);
                    let pad = b.pad(d.action);
                    let marked = |dev| {
                        if conflicts
                            .iter()
                            .any(|c| c.action == d.action && c.device == dev)
                        {
                            " (!)"
                        } else {
                            ""
                        }
                    };
                    let set =
                        |dir| format!("set keys.{} {}", d.id, step_choice(KEY_CHOICES, key, dir));
                    Row {
                        label: d.label.into(),
                        value: Some(format!(
                            "{}{} | {}{}",
                            key_label(key),
                            marked(Device::Key),
                            pad_label(pad),
                            marked(Device::Pad)
                        )),
                        left: Some(set(-1)),
                        right: Some(set(1)),
                        confirm: Some(Activate::Command(format!("bind --capture {}", d.id))),
                    }
                })
                .collect()
        }
        Page::BindReset => vec![Row::action("Reset all keys and buttons", "bind --reset")],
        _ => Vec::new(),
    }
}

/// A rider edit command line: `riders.edit "NAME" ARGS`.
fn edit(name: &str, args: &str) -> String {
    format!("riders.edit \"{name}\" {args}")
}

/// A number field of the rider editor: ← / → by `step` within the range;
/// unset, they start from `start`.
fn number_row(
    label: &str,
    value: Option<f64>,
    unit: &str,
    (step, start, min, max): (f64, f64, f64, f64),
    line: impl Fn(f64) -> String,
) -> Row {
    let base = value.unwrap_or(start);
    let to = |dir: f64| {
        let v = if value.is_none() {
            base
        } else {
            (base + dir * step).clamp(min, max)
        };
        line((v * 10.0).round() / 10.0)
    };
    Row {
        label: label.into(),
        value: Some(value.map_or("-".into(), |v| {
            format!("{} {unit}", (v * 10.0).round() / 10.0)
        })),
        left: Some(to(-1.0)),
        right: Some(to(1.0)),
        confirm: None,
    }
}

/// One step through `items` from `current` (wrapping).
fn cycle<T: Copy + PartialEq>(items: &[T], current: T, dir: isize) -> T {
    let at = items.iter().position(|x| *x == current).unwrap_or(0) as isize;
    items[(at + dir).rem_euclid(items.len() as isize) as usize]
}

/// A row whose ← / → run `line(previous)` / `line(next)`.
fn choice_row(label: &str, value: String, left: String, right: String) -> Row {
    Row {
        label: label.into(),
        value: Some(value),
        left: Some(left.clone()),
        right: Some(right.clone()),
        confirm: Some(Activate::Command(right)),
    }
}

/// The Bike page of a rider (spec §2.5, §3.2.1 Workshop → Bike): the
/// bike (one of the curated physics profiles, grouped by bike type:
/// "Road - aero"), the profile's numbers, then the cosmetic colours and
/// kit; each change is a `riders.edit` line. Six rows (ten-foot limit).
fn bike_rows(r: &crate::riders::Rider) -> Vec<Row> {
    use ocw_core::{KitPattern, ProfileId, bike::palette_step};
    let n = &r.name;
    let bike = r.bike();
    let look = r.look();
    let bike_line = |p: ProfileId| edit(n, &format!("--bike {}", p.id()));
    let p = bike.profile();
    let colour = |label: &str, flag: &str, c: ocw_core::Rgb| {
        let line = |dir| edit(n, &format!("--{flag} {}", palette_step(c, dir)));
        choice_row(label, c.to_string(), line(-1), line(1))
    };
    let kit_line = |k: KitPattern| edit(n, &format!("--kit {}", k.id()));
    vec![
        choice_row(
            "Bike",
            bike.name().into(),
            bike_line(cycle(&ProfileId::ALL, bike, -1)),
            bike_line(cycle(&ProfileId::ALL, bike, 1)),
        ),
        // Shown separately: cosmetic choices never change it (§2.5).
        Row {
            label: "Physics".into(),
            value: Some(format!(
                "{} kg bike, CdA {:.3}, Crr {:.4}",
                p.bike_mass_kg, p.cda_m2, p.crr
            )),
            left: None,
            right: None,
            confirm: None,
        },
        colour("Frame colour", "frame-color", look.frame),
        colour("Jersey colour", "jersey-color", look.jersey),
        colour("Helmet colour", "helmet-color", look.helmet),
        choice_row(
            "Kit",
            look.pattern.id().into(),
            kit_line(cycle(&KitPattern::ALL, look.pattern, -1)),
            kit_line(cycle(&KitPattern::ALL, look.pattern, 1)),
        ),
    ]
}

/// The output devices as the audio engine sees them, for the Audio page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AudioDevices {
    /// Output device names.
    pub names: Vec<String>,
    /// The device playing now.
    pub current: Option<String>,
    /// The saved device is missing; the default plays.
    pub fell_back: bool,
}

/// Longest device name shown on the Audio page (ten-foot UI); longer
/// ones are cut with "...".
const MAX_DEVICE_LABEL: usize = 16;

fn short(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_string()
    } else {
        let cut: String = name.chars().take(max - 3).collect();
        format!("{}...", cut.trim_end())
    }
}

/// The Audio page (spec §3.2.1 M5): the volumes (Enter plays a test sound
/// on that bus) – master, effects, the other riders, voice; music joins
/// when there is music (seven items per page) –, mute, and the output
/// device (← / → step through "default" and the devices; a saved device
/// that is missing stays in the list, marked).
pub fn audio_rows(settings: &Settings, devices: &AudioDevices) -> Vec<Row> {
    let volume = |key: &str, label: &str, test: &str| Row {
        confirm: Some(Activate::Command(format!("audio.play {test}"))),
        ..setting_row(settings, key, label)
    };
    let saved = settings
        .get("audio.output")
        .map(|v| v.to_string())
        .unwrap_or_else(|_| "default".into());
    let is_default = saved.eq_ignore_ascii_case("default");
    let same = |d: &String| d.trim().eq_ignore_ascii_case(saved.trim());
    let listed = devices.names.iter().any(same);
    let mut options = vec!["default".to_string()];
    if !is_default && !listed {
        options.push(saved.clone());
    }
    options.extend(devices.names.iter().cloned());
    let at = options.iter().position(same).unwrap_or(0);
    let pick = |dir: isize| {
        let n = options.len() as isize;
        let o = &options[(at as isize + dir).rem_euclid(n) as usize];
        format!("set audio.output \"{o}\"")
    };
    // Short values: the row has room for about 16 characters (ten-foot
    // UI); `audio.status` and `audio.devices` show the full names.
    let value = if is_default {
        match &devices.current {
            Some(_) => "default".into(),
            None => "default (none)".into(),
        }
    } else if !listed || devices.fell_back {
        format!("{} (gone)", short(&saved, MAX_DEVICE_LABEL - 7))
    } else {
        short(&saved, MAX_DEVICE_LABEL)
    };
    vec![
        volume("audio.master", "Master volume", "test"),
        volume("audio.effects", "Effects (bike, wind)", "test-effects"),
        volume("audio.others", "Other riders", "test-others"),
        Row {
            label: "Voice".into(),
            value: Some(format!(
                "{} %{}",
                settings
                    .get("audio.voice")
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
                if settings
                    .get("audio.echo_bot")
                    .ok()
                    .and_then(|v| v.as_bool())
                    == Some(true)
                {
                    " - echo test"
                } else {
                    ""
                }
            )),
            left: None,
            right: None,
            confirm: Some(Activate::Open(Page::AudioVoice)),
        },
        setting_row(settings, "audio.mute", "Mute"),
        Row {
            label: "Output".into(),
            value: Some(value),
            left: Some(pick(-1)),
            right: Some(pick(1)),
            confirm: Some(Activate::Command(pick(1))),
        },
    ]
}

/// The rows of `page`.
pub fn rows(page: Page, settings: &Settings, riders: &Riders) -> Vec<Row> {
    match page {
        // Without the engine's device list (the menu passes it, see
        // `audio_rows`).
        Page::Audio => audio_rows(settings, &AudioDevices::default()),
        Page::Controls => vec![
            setting_row(settings, "controls.autodrive", "Autodrive at the start"),
            // Rebinding (M9) and the unicorn's controls have their own
            // pages (the ten-foot rule: six rows).
            open_row(
                "Keys and buttons",
                Some(bindings_summary(settings, true)),
                Page::Bindings,
            ),
            open_row(
                "Touch and unicorn",
                Some(format!(
                    "touch {}, {}",
                    settings
                        .get("controls.touch_steer")
                        .map_or_else(|_| "auto".into(), |v| v.to_string()),
                    settings
                        .get("controls.god_flight")
                        .map_or_else(|_| "plane".into(), |v| v.to_string()),
                )),
                Page::UnicornControls,
            ),
            // Collisions (spec §14 Phase 12A, 12D).
            setting_row(settings, "ride.collisions", "Collisions"),
            setting_row(settings, "ride.chaotic_bots", "Chaotic bots"),
            // The aids have their own page (the ten-foot rule: six rows).
            Row {
                label: "Riding aids".into(),
                value: Some(aids_summary(settings)),
                left: None,
                right: None,
                confirm: Some(Activate::Open(Page::RidingAids)),
            },
        ],
        Page::UnicornControls => vec![
            setting_row(settings, "controls.touch_steer", "Touch steering"),
            setting_row(settings, "controls.touch_zones", "Touch zones"),
            setting_row(settings, "controls.invert_y", "Unicorn: invert Y"),
            setting_row(settings, "controls.god_flight", "Unicorn: flight"),
        ],
        Page::Bindings | Page::BindGroup(_) | Page::BindReset => binding_rows(page, settings, true),
        Page::Accessibility => vec![
            setting_row(settings, "display.ui_scale", "HUD size (%)"),
            setting_row(settings, "display.palette", "Status colours"),
            setting_row(settings, "display.high_contrast", "High-contrast HUD"),
            setting_row(settings, "display.reduced_motion", "Reduced motion"),
            setting_row(settings, "display.camera_shake", "Camera shake"),
        ],
        // From the training history (`menu::history`).
        Page::History
        | Page::HistoryRides
        | Page::HistoryRide(_)
        | Page::HistoryClimbs(_)
        | Page::HistoryWeeks
        | Page::HistoryMonths
        | Page::HistoryBests => Vec::new(),
        // From the trusted world keys (`menu::trust`).
        Page::TrustedKeys | Page::TrustedKey(_) => Vec::new(),
        // From the app's devices (`menu::devices`).
        Page::Devices | Page::Device(_) => Vec::new(),
        // From the join state (`menu::join`) and the host wizard (`host`).
        Page::Join
        | Page::HostWorld
        | Page::HostRoute
        | Page::HostTime
        | Page::HostWeather
        | Page::HostOptions
        | Page::HostSummary
        | Page::HostLink => Vec::new(),
        // The organiser's rules (spec §14 Phase 12 K6): six rows.
        Page::EventRules => vec![
            setting_row(settings, "event.collisions", "Collisions"),
            setting_row(settings, "event.damage", "Damage"),
            setting_row(settings, "event.aids", "Riding aids allowed"),
            setting_row(settings, "event.chaotic_bots", "Chaotic bots"),
            setting_row(settings, "event.neutral_service", "Neutral service"),
            setting_row(settings, "event.dnf_injured", "DNF when badly injured"),
        ],
        Page::RidingAids => vec![
            setting_row(settings, "ride.proximity_cue", "Proximity arrow and bell"),
            setting_row(settings, "ride.hit_warning", "Last-moment warning"),
            setting_row(settings, "display.camera_shake", "Camera shake"),
        ],
        // From the session (`rider_list`).
        Page::RiderList => Vec::new(),
        // From the app's sign-in state (`crate::account::rows`).
        Page::Account => crate::account::rows(&crate::account::AccountInfo::default(), true),
        Page::Trainer => vec![
            setting_row(settings, "trainer.difficulty", "Difficulty (%)"),
            setting_row(
                settings,
                "trainer.virtual_power",
                "Virtual trainer power (W)",
            ),
            // Scan, connect, forget (spec §3.2.1 *Devices*; the value
            // comes from the app's devices, `ocw_ui::menu`).
            open_row("Devices", None, Page::Devices),
            // Damage (spec §14 Phase 12E; the Controls and Display pages
            // are full).
            setting_row(settings, "ride.damage", "Damage"),
            setting_row(settings, "display.damage", "Damage indicator"),
        ],
        Page::Strava => vec![
            Row::action("Status", "strava.status"),
            Row::action("Link account", "strava.connect"),
            Row::action("Sync now", "strava.sync"),
            Row::action("Retry failed uploads", "strava.sync --retry"),
            Row::action("Unlink", "strava.disconnect"),
        ],
        Page::Display => vec![
            setting_row(settings, "display.hud", "HUD"),
            setting_row(settings, "display.units", "Units"),
            setting_row(settings, "display.camera", "Camera"),
            setting_row(settings, "display.night_brightness", "Night brightness (%)"),
            // M9 (the FPS counter moved to Graphics: six rows).
            open_row(
                "Accessibility",
                Some(access_summary(settings)),
                Page::Accessibility,
            ),
            // The quality preset has its own page (spec §14 Phase 10 B7;
            // the ten-foot rule: six rows), with fullscreen and the sky.
            Row {
                label: "Graphics".into(),
                value: settings.get("display.quality").ok().map(|v| v.to_string()),
                left: None,
                right: None,
                confirm: Some(Activate::Open(Page::Graphics)),
            },
        ],
        Page::Graphics => vec![
            setting_row(settings, "display.quality", "Quality"),
            // Spec §14 Phase 10, 10G: its own page (four rows).
            open_row(
                "Render distance",
                Some(crate::view_distance::summary(settings)),
                Page::ViewDistance,
            ),
            setting_row(settings, "display.sky", "Sky"),
            setting_row(settings, "display.fps", "FPS counter"),
            setting_row(settings, "display.fullscreen", "Fullscreen (next start)"),
            // The ride photographer (spec §2.11): photos of the ride.
            setting_row(settings, "photographer.enabled", "Ride photographer"),
        ],
        // The app's shared UI fills in the preset and what runs now
        // (`ocw_ui::view_distance`).
        Page::ViewDistance => {
            crate::view_distance::rows(settings, &crate::view_distance::ViewInfo::default())
        }
        Page::Riders => {
            let mut out: Vec<Row> = riders
                .riders
                .iter()
                .enumerate()
                .map(|(i, r)| Row {
                    label: r.name.clone(),
                    value: Some(r.summary()),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Open(Page::Rider(i))),
                })
                .collect();
            out.push(Row::action(
                "Add a rider",
                format!("riders.add \"{}\"", riders.new_name()),
            ));
            out
        }
        Page::Rider(i) => {
            let Some(r) = riders.riders.get(i) else {
                return Vec::new();
            };
            let n = &r.name;
            let (hmin, hmax) = crate::riders::HEIGHT_RANGE_CM;
            let (fmin, fmax) = crate::riders::FTP_RANGE_W;
            vec![
                // The name (spec §14 Phase 4, 4N N1): typed with
                // `riders.edit NAME --rename NEW`; Confirm rolls the dice
                // for a made-up one. A name from before the 4N rules is
                // marked: servers show such a rider as "Rider NNNN".
                Row {
                    label: "Name".into(),
                    value: Some(if ocw_core::name::check(n).is_ok() {
                        n.clone()
                    } else {
                        format!("{n} (rename before a shared ride)")
                    }),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Command(edit(
                        n,
                        &format!("--rename \"{}\"", riders.dice_name(i)),
                    ))),
                },
                number_row(
                    "Weight",
                    Some(r.weight_kg),
                    "kg",
                    (
                        1.0,
                        75.0,
                        ocw_core::MIN_RIDER_WEIGHT_KG,
                        ocw_core::MAX_RIDER_WEIGHT_KG,
                    ),
                    |v| edit(n, &format!("--weight {v}")),
                ),
                number_row("Height", r.height_cm, "cm", (1.0, 175.0, hmin, hmax), |v| {
                    edit(n, &format!("--height {v}"))
                }),
                number_row(
                    "FTP",
                    r.ftp_w.map(f64::from),
                    "W",
                    (5.0, 200.0, f64::from(fmin), f64::from(fmax)),
                    |v| edit(n, &format!("--ftp {v}")),
                ),
                Row::action("Ride as this rider", format!("riders.select \"{n}\"")),
                Row::action("Delete this rider", format!("riders.remove \"{n}\"")),
            ]
        }
        Page::Bike(i) => {
            let Some(r) = riders.riders.get(i) else {
                return Vec::new();
            };
            bike_rows(r)
        }
        // From the ride catalog (`ride::pages::rows`, `ride::world::rows`).
        Page::WorldHome
        | Page::Worlds
        | Page::World(_)
        | Page::WorldRoutes(_)
        | Page::WorldParts(_)
        | Page::WorldInfo
        | Page::WorldSun
        | Page::WorldSources
        | Page::WorldSource(_)
        | Page::Routes
        | Page::Rides
        | Page::GpxImport
        | Page::Workouts => Vec::new(),
        // From the voice state (`voice::page::rows`).
        Page::Voice
        | Page::VoiceDevices
        | Page::VoiceParticipants
        | Page::VoiceRider(_)
        | Page::VoiceBlocked
        | Page::AudioVoice => Vec::new(),
        // From the world clock (`timemachine::rows`).
        Page::TimeMachine | Page::Calendar(_) | Page::CalendarDates(_) | Page::Season => Vec::new(),
        // From the session weather (`weather::rows`).
        Page::Weather
        | Page::WeatherPresets
        | Page::WeatherCustom
        | Page::WeatherAir
        | Page::WeatherReal => Vec::new(),
        Page::Workshop => {
            let mut out: Vec<Row> = riders
                .riders
                .iter()
                .map(|r| Row {
                    label: r.name.clone(),
                    value: Some(if riders.is_active(&r.name) {
                        format!("{} - riding", r.summary())
                    } else {
                        r.summary()
                    }),
                    left: None,
                    right: None,
                    confirm: Some(Activate::Command(format!("riders.select \"{}\"", r.name))),
                })
                .collect();
            if out.is_empty() {
                let d = crate::riders::Rider::default();
                out.push(Row {
                    label: d.name.clone(),
                    value: Some(format!("{} - riding (default)", d.summary())),
                    left: None,
                    right: None,
                    confirm: None,
                });
            }
            // The bike of the rider who rides next.
            let active = riders.active();
            out.push(Row {
                label: "Bike".into(),
                value: Some(if riders.riders.is_empty() {
                    format!("{} (add a rider to change it)", active.bike().name())
                } else {
                    active.bike_summary()
                }),
                left: None,
                right: None,
                confirm: riders
                    .find(&active.name)
                    .filter(|_| !riders.riders.is_empty())
                    .map(|i| Activate::Open(Page::Bike(i))),
            });
            // The character of the rider who rides next (spec §14 Phase 4,
            // 4N N10): body, face and age, hair, facial hair.
            out.push(Row {
                label: "Character".into(),
                value: Some(active.character().summary()),
                left: None,
                right: None,
                confirm: Some(Activate::Open(Page::Character)),
            });
            out.push(Row {
                label: "Edit riders".into(),
                value: None,
                left: None,
                right: None,
                confirm: Some(Activate::Open(Page::Riders)),
            });
            out
        }
        Page::Character
        | Page::CharacterBody
        | Page::CharacterFace
        | Page::CharacterHair
        | Page::CharacterFacialHair
        | Page::CharacterPick(_) => super::character::rows(page, &riders.active()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riders::Rider;

    fn riders() -> Riders {
        let mut r = Riders::default();
        r.add(Rider {
            name: "Alex".into(),
            weight_kg: 78.0,
            height_cm: None,
            ftp_w: Some(250),
            ..Rider::default()
        })
        .unwrap();
        r.add(Rider {
            name: "Anna".into(),
            weight_kg: 61.5,
            ..Rider::default()
        })
        .unwrap();
        r
    }

    fn row<'a>(rows: &'a [Row], label: &str) -> &'a Row {
        rows.iter().find(|r| r.label == label).expect(label)
    }

    fn cmd(line: &str) -> Option<Activate> {
        Some(Activate::Command(line.into()))
    }

    #[test]
    fn setting_rows_run_set_commands() {
        let (s, _) = Settings::parse("[display]\ncamera = \"side\"\n").unwrap();
        let display = rows(Page::Display, &s, &Riders::default());
        let hud = row(&display, "HUD");
        assert_eq!(hud.value.as_deref(), Some("on"));
        assert_eq!(hud.confirm, cmd("set display.hud off"));
        assert_eq!(hud.left.as_deref(), Some("set display.hud off"));
        let cam = row(&display, "Camera");
        assert_eq!(cam.value.as_deref(), Some("side"));
        assert_eq!(cam.right.as_deref(), Some("set display.camera top"));
        assert_eq!(cam.left.as_deref(), Some("set display.camera chase"));
        assert_eq!(
            row(&display, "Units").confirm,
            cmd("set display.units imperial")
        );
        let trainer = rows(Page::Trainer, &s, &Riders::default());
        let d = row(&trainer, "Difficulty (%)");
        assert_eq!(d.value.as_deref(), Some("50"));
        assert_eq!(d.left.as_deref(), Some("set trainer.difficulty 45"));
        assert_eq!(d.right.as_deref(), Some("set trainer.difficulty 55"));
        assert_eq!(d.confirm, None);
        assert_eq!(
            row(&trainer, "Devices").confirm,
            Some(Activate::Open(Page::Devices))
        );
        let controls = rows(Page::Controls, &s, &Riders::default());
        assert_eq!(controls[0].confirm, cmd("set controls.autodrive off"));
        let strava = rows(Page::Strava, &s, &Riders::default());
        assert_eq!(
            row(&strava, "Retry failed uploads").confirm,
            cmd("strava.sync --retry")
        );
    }

    /// K6 (spec §14 Phase 12): the Organiser rules page, six rows of
    /// `event.*` settings (`auto` by the event's kind).
    #[test]
    fn the_organiser_rules_page_sets_the_event_settings() {
        let (s, _) = Settings::parse("[event]\naids = \"on\"\n").unwrap();
        let r = rows(Page::EventRules, &s, &Riders::default());
        assert!(r.len() <= 6, "the ten-foot rule");
        let c = row(&r, "Collisions");
        assert_eq!(c.value.as_deref(), Some("auto"));
        assert_eq!(c.right.as_deref(), Some("set event.collisions off"));
        assert_eq!(row(&r, "Riding aids allowed").value.as_deref(), Some("on"));
        assert_eq!(
            row(&r, "Neutral service").confirm,
            cmd("set event.neutral_service on")
        );
        assert_eq!(
            row(&r, "DNF when badly injured").confirm,
            cmd("set event.dnf_injured on")
        );
        assert_eq!(Page::EventRules.title(), "Organiser rules");
    }

    #[test]
    fn workshop_selects_the_rider() {
        let s = Settings::default();
        let r = riders();
        let w = rows(Page::Workshop, &s, &r);
        assert_eq!(w.len(), 5);
        assert_eq!(w[0].value.as_deref(), Some("78 kg, FTP 250 W - riding"));
        assert_eq!(w[1].value.as_deref(), Some("61.5 kg"));
        assert_eq!(w[1].confirm, cmd("riders.select \"Anna\""));
        // The bike of the active rider (the first).
        assert_eq!(w[2].label, "Bike");
        assert_eq!(
            w[2].value.as_deref(),
            Some("Road - standard, frame #1E1E22, jersey #FF1472")
        );
        assert_eq!(w[2].confirm, Some(Activate::Open(Page::Bike(0))));
        // The character (spec §14 Phase 4, 4N N10).
        assert_eq!(w[3].label, "Character");
        assert_eq!(w[3].confirm, Some(Activate::Open(Page::Character)));
        assert!(w[3].value.as_deref().unwrap().contains("body a, normal"));
        assert_eq!(w[4].confirm, Some(Activate::Open(Page::Riders)));
        // Without profiles: the default rider, and the way to the editor.
        let w = rows(Page::Workshop, &s, &Riders::default());
        assert_eq!(w[0].value.as_deref(), Some("75 kg - riding (default)"));
        assert_eq!(w[0].confirm, None);
        assert_eq!(w[1].confirm, None, "no profile to give a bike");
        assert_eq!(w[2].confirm, Some(Activate::Open(Page::Character)));
        assert_eq!(w[3].confirm, Some(Activate::Open(Page::Riders)));
        // The character's pages come from `menu::character`, for the
        // active rider; they go back to the Workshop.
        for p in super::super::character::PAGES {
            assert!(!rows(p, &s, &r).is_empty(), "{p:?}");
            assert_eq!(p.parent(), Some(Page::Character));
        }
        assert_eq!(Page::Character.parent(), Some(Page::Workshop));
    }

    #[test]
    fn rider_pages_edit_the_profile() {
        let s = Settings::default();
        let r = riders();
        let list = rows(Page::Riders, &s, &r);
        assert_eq!(list[1].confirm, Some(Activate::Open(Page::Rider(1))));
        let add = format!("riders.add \"{}\"", r.new_name());
        assert_eq!(list.last().unwrap().confirm, cmd(&add));
        let alex = rows(Page::Rider(0), &s, &r);
        let weight = row(&alex, "Weight");
        assert_eq!(weight.value.as_deref(), Some("78 kg"));
        assert_eq!(
            weight.left.as_deref(),
            Some("riders.edit \"Alex\" --weight 77")
        );
        assert_eq!(
            weight.right.as_deref(),
            Some("riders.edit \"Alex\" --weight 79")
        );
        // No height yet: ← / → start at 175 cm.
        let height = row(&alex, "Height");
        assert_eq!(height.value.as_deref(), Some("-"));
        assert_eq!(
            height.right.as_deref(),
            Some("riders.edit \"Alex\" --height 175")
        );
        assert_eq!(
            row(&alex, "FTP").left.as_deref(),
            Some("riders.edit \"Alex\" --ftp 245")
        );
        assert_eq!(
            row(&alex, "Delete this rider").confirm,
            cmd("riders.remove \"Alex\"")
        );
        // The weight stays in range at the ends.
        let mut light = Riders::default();
        light
            .add(Rider {
                name: "Kid".into(),
                weight_kg: 30.0,
                ..Rider::default()
            })
            .unwrap();
        assert_eq!(
            row(&rows(Page::Rider(0), &s, &light), "Weight")
                .left
                .as_deref(),
            Some("riders.edit \"Kid\" --weight 30")
        );
        assert!(rows(Page::Rider(9), &s, &r).is_empty());
    }

    #[test]
    fn bike_page_edits_bike_and_look() {
        let s = Settings::default();
        let mut r = riders();
        let b = rows(Page::Bike(1), &s, &r);
        let bike = row(&b, "Bike");
        assert_eq!(bike.value.as_deref(), Some("Road - standard"));
        assert_eq!(
            bike.right.as_deref(),
            Some("riders.edit \"Anna\" --bike road-aero")
        );
        assert_eq!(
            bike.left.as_deref(),
            Some("riders.edit \"Anna\" --bike mtb")
        );
        assert_eq!(
            row(&b, "Physics").value.as_deref(),
            Some("8 kg bike, CdA 0.320, Crr 0.0040")
        );
        let jersey = row(&b, "Jersey colour");
        assert_eq!(jersey.value.as_deref(), Some("#FF1472"));
        assert_eq!(
            jersey.right.as_deref(),
            Some("riders.edit \"Anna\" --jersey-color #E53935")
        );
        assert_eq!(
            row(&b, "Kit").right.as_deref(),
            Some("riders.edit \"Anna\" --kit band")
        );
        // Another bike: its own numbers; the colours stay.
        r.edit(
            "Anna",
            crate::riders::RiderEdit {
                bike: Some(ocw_core::ProfileId::Gravel),
                ..Default::default()
            },
        )
        .unwrap();
        let b = rows(Page::Bike(1), &s, &r);
        assert_eq!(row(&b, "Bike").value.as_deref(), Some("Gravel"));
        assert_eq!(
            row(&b, "Physics").value.as_deref(),
            Some("9 kg bike, CdA 0.325, Crr 0.0045")
        );
        assert_eq!(
            row(&b, "Bike").right.as_deref(),
            Some("riders.edit \"Anna\" --bike mtb")
        );
        assert!(rows(Page::Bike(9), &s, &r).is_empty());
    }

    #[test]
    fn audio_page_sets_volumes_and_the_output() {
        let s = Settings::default();
        let devices = AudioDevices {
            names: vec!["Speakers".into(), "USB Headset".into()],
            current: Some("Speakers".into()),
            fell_back: false,
        };
        let a = audio_rows(&s, &devices);
        assert!(a.len() < crate::menu::model::MAX_ITEMS, "fits one page");
        let master = row(&a, "Master volume");
        assert_eq!(master.value.as_deref(), Some("80"));
        assert_eq!(master.left.as_deref(), Some("set audio.master 75"));
        assert_eq!(master.right.as_deref(), Some("set audio.master 85"));
        assert_eq!(master.confirm, cmd("audio.play test"));
        // Voice opens Audio → Voice (volume and the echo test).
        assert_eq!(
            row(&a, "Voice").confirm,
            Some(Activate::Open(Page::AudioVoice))
        );
        assert_eq!(row(&a, "Voice").value.as_deref(), Some("100 %"));
        let others = row(&a, "Other riders");
        assert_eq!(others.value.as_deref(), Some("80"));
        assert_eq!(others.left.as_deref(), Some("set audio.others 75"));
        assert_eq!(others.confirm, cmd("audio.play test-others"));
        assert_eq!(row(&a, "Mute").confirm, cmd("set audio.mute on"));
        let out = row(&a, "Output");
        assert_eq!(out.value.as_deref(), Some("default"));
        assert_eq!(out.right.as_deref(), Some("set audio.output \"Speakers\""));
        assert_eq!(
            out.left.as_deref(),
            Some("set audio.output \"USB Headset\"")
        );
        // A chosen device: steps on from it.
        let (s, _) = Settings::parse("[audio]\noutput = \"USB Headset\"\n").unwrap();
        let out = row(&audio_rows(&s, &devices), "Output").clone();
        assert_eq!(out.value.as_deref(), Some("USB Headset"));
        assert_eq!(out.right.as_deref(), Some("set audio.output \"default\""));
        // Unplugged: still in the list, marked; the default plays.
        let (s, _) = Settings::parse("[audio]\noutput = \"Old DAC\"\n").unwrap();
        let out = row(&audio_rows(&s, &devices), "Output").clone();
        assert_eq!(out.value.as_deref(), Some("Old DAC (gone)"));
        assert_eq!(out.left.as_deref(), Some("set audio.output \"default\""));
        assert_eq!(out.right.as_deref(), Some("set audio.output \"Speakers\""));
        // No devices (headless): the default, said plainly.
        let a = rows(Page::Audio, &Settings::default(), &Riders::default());
        let out = row(&a, "Output");
        assert_eq!(out.value.as_deref(), Some("default (none)"));
        assert_eq!(out.right.as_deref(), Some("set audio.output \"default\""));
        // Long names are cut for the ten-foot UI.
        let name = "Default ALSA Output (currently PipeWire Media Server)";
        let long = AudioDevices {
            names: vec![name.into()],
            current: Some(name.into()),
            fell_back: false,
        };
        let (s, _) = Settings::parse(&format!("[audio]\noutput = \"{name}\"\n")).unwrap();
        let v = row(&audio_rows(&s, &long), "Output").value.clone();
        assert_eq!(v.as_deref(), Some("Default ALSA..."));
        let v = row(&audio_rows(&s, &AudioDevices::default()), "Output")
            .value
            .clone();
        assert_eq!(v.as_deref(), Some("Defaul... (gone)"));
    }

    #[test]
    fn page_parents() {
        assert_eq!(Page::Bike(0).parent(), Some(Page::Workshop));
        assert_eq!(Page::Rider(2).parent(), Some(Page::Riders));
        assert_eq!(Page::Workshop.parent(), None);
    }
}

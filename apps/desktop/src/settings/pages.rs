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

use super::registry::{Kind, Settings, def};
use super::riders::Riders;

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
    /// Ride → World packages: the known packages (M3; rows from
    /// `ride::pages`, like the following).
    Worlds,
    /// One of them (index into the list): free ride, its routes.
    World(usize),
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
    /// Voice → Participants: mute single riders.
    VoiceParticipants,
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
            Self::Worlds => "World packages",
            Self::World(_) => "World package",
            Self::Routes => "Routes",
            Self::Rides => "Rides",
            Self::GpxImport => "GPX import",
            Self::Workouts => "Workouts",
            Self::Voice => "Voice",
            Self::VoiceDevices => "Voice devices",
            Self::VoiceParticipants => "Voice participants",
        }
    }

    /// The Ride and Navi lists (rows from `ride::pages`).
    pub fn is_ride_page(self) -> bool {
        matches!(
            self,
            Self::Worlds
                | Self::World(_)
                | Self::Routes
                | Self::Rides
                | Self::GpxImport
                | Self::Workouts
        )
    }

    /// The voice pages (rows from `voice::page`).
    pub fn is_voice_page(self) -> bool {
        matches!(
            self,
            Self::Voice | Self::VoiceDevices | Self::VoiceParticipants
        )
    }

    /// One page up (the rider editor goes back to the list).
    pub fn parent(self) -> Option<Self> {
        match self {
            Self::Rider(_) => Some(Self::Riders),
            Self::Bike(_) => Some(Self::Workshop),
            Self::World(_) => Some(Self::Worlds),
            Self::VoiceDevices | Self::VoiceParticipants => Some(Self::Voice),
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
            Self::Rider(_) => &["Changes apply from the next start (Restart)."],
            Self::Trainer => &["Devices, SIM difficulty and the virtual trainer."],
            Self::Strava => &[
                "Uploads rides to Strava after saving when an account is linked (needs API credentials, see README); the sync state is in Navi -> Rides. Later (Phase 5): upload through the OCW backend, Strava routes as GPX routes.",
            ],
            Self::Audio => &[
                "Sounds are made by OCW itself; Enter on a volume plays a test sound. Output: the device (default = the system's).",
            ],
            Self::Controls | Self::Display => &[],
            Self::Worlds => &[
                "Installed world packages (cache, and package files next to the app). Choose one for a free ride or its routes.",
            ],
            Self::World(_) => &["Free ride from a random road, or from where you are now."],
            Self::Routes => &[
                "Routes of the loaded world package and your imported ones (GPX import). Choose one to ride it.",
            ],
            Self::Rides => &["Saved rides (FIT files), newest first, with their Strava state."],
            Self::GpxImport => &[
                "GPX files from the gpx folder of the OCW data folder and from Downloads, matched onto the roads of the loaded world package.",
            ],
            Self::Voice => &[
                "Voice chat is always on, like a call: on shared rides you join automatically. M mutes your microphone, Shift+M the voices (gamepad: Select, R3).",
            ],
            Self::VoiceDevices => &[
                "Microphone and speaker for voice chat; a missing device falls back to the default.",
            ],
            Self::VoiceParticipants => {
                &["Riders in the voice room. Confirm mutes or unmutes one for you."]
            }
            Self::Workouts => &[
                "Structured workouts in ERG mode: yours from the workouts folder of the OCW data folder (.toml, .erg, .mrc), then the examples. % FTP targets use the rider's FTP (Settings -> Riders).",
            ],
        }
    }
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
    /// Run a console command line and close the menu (starting a workout:
    /// its panel shows at once).
    Run(String),
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
    fn action(label: &str, line: impl Into<String>) -> Self {
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
fn bike_rows(r: &super::riders::Rider) -> Vec<Row> {
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

/// The Audio page (spec §3.2.1 M5): the four volumes (Enter plays a test
/// sound on that bus), mute, and the output device (← / → step through
/// "default" and the devices; a saved device that is missing stays in the
/// list, marked).
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
        volume("audio.music", "Music", "test-music"),
        volume("audio.voice", "Voice", "test-voice"),
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
        Page::Controls => vec![setting_row(
            settings,
            "controls.autodrive",
            "Autodrive at the start",
        )],
        Page::Trainer => vec![
            setting_row(settings, "trainer.difficulty", "Difficulty (%)"),
            setting_row(
                settings,
                "trainer.virtual_power",
                "Virtual trainer power (W)",
            ),
            Row::action("Scan for devices", "devices.scan"),
            Row::action("Show devices", "devices.list"),
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
            setting_row(settings, "display.fps", "FPS counter"),
            setting_row(settings, "display.fullscreen", "Fullscreen (next start)"),
        ],
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
            let (hmin, hmax) = super::riders::HEIGHT_RANGE_CM;
            let (fmin, fmax) = super::riders::FTP_RANGE_W;
            vec![
                Row {
                    label: "Name".into(),
                    value: Some(n.clone()),
                    left: None,
                    right: None,
                    confirm: None,
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
        // From the ride catalog (`ride::pages::rows`).
        Page::Worlds
        | Page::World(_)
        | Page::Routes
        | Page::Rides
        | Page::GpxImport
        | Page::Workouts => Vec::new(),
        // From the voice state (`voice::page::rows`).
        Page::Voice | Page::VoiceDevices | Page::VoiceParticipants => Vec::new(),
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
                let d = super::riders::Rider::default();
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
            out.push(Row {
                label: "Edit riders".into(),
                value: None,
                left: None,
                right: None,
                confirm: Some(Activate::Open(Page::Riders)),
            });
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::riders::Rider;
    use super::*;

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
            row(&trainer, "Scan for devices").confirm,
            cmd("devices.scan")
        );
        let controls = rows(Page::Controls, &s, &Riders::default());
        assert_eq!(controls[0].confirm, cmd("set controls.autodrive off"));
        let strava = rows(Page::Strava, &s, &Riders::default());
        assert_eq!(
            row(&strava, "Retry failed uploads").confirm,
            cmd("strava.sync --retry")
        );
    }

    #[test]
    fn workshop_selects_the_rider() {
        let s = Settings::default();
        let r = riders();
        let w = rows(Page::Workshop, &s, &r);
        assert_eq!(w.len(), 4);
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
        assert_eq!(w[3].confirm, Some(Activate::Open(Page::Riders)));
        // Without profiles: the default rider, and the way to the editor.
        let w = rows(Page::Workshop, &s, &Riders::default());
        assert_eq!(w[0].value.as_deref(), Some("75 kg - riding (default)"));
        assert_eq!(w[0].confirm, None);
        assert_eq!(w[1].confirm, None, "no profile to give a bike");
        assert_eq!(w[2].confirm, Some(Activate::Open(Page::Riders)));
    }

    #[test]
    fn rider_pages_edit_the_profile() {
        let s = Settings::default();
        let r = riders();
        let list = rows(Page::Riders, &s, &r);
        assert_eq!(list[1].confirm, Some(Activate::Open(Page::Rider(1))));
        assert_eq!(list.last().unwrap().confirm, cmd("riders.add \"Rider\""));
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
            super::super::riders::RiderEdit {
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
        assert_eq!(row(&a, "Voice").confirm, cmd("audio.play test-voice"));
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

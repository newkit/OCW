//! Settings and rider profiles of the desktop app (spec §3.2.1, M2).
//!
//! - [`registry`]: typed settings in `settings.toml` (Bevy-free),
//! - [`riders`]: rider profiles in `riders.toml` (Bevy-free),
//! - [`pages`]: the Settings / Workshop menu pages as rows of command lines
//!   (Bevy-free; the menu shell renders them),
//! - this module: loading at the start (before the other plugins read
//!   [`Args`]), the console commands `set`, `get` and `riders.*`.
//!
//! Both files live in the user config directory (`~/.config/ocw/` on
//! Linux, `OCW_CONFIG_DIR` overrides it); `--settings-file` names another
//! settings file, and `riders.toml` is then read next to it.

pub mod pages;
pub mod registry;
pub mod riders;

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use clap::parser::ValueSource;
use clap::{Arg, ArgAction, ArgMatches, Command, value_parser};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

use crate::Args;
use registry::{Apply, SETTINGS, Scope, Settings, Value, def};
use riders::{Rider, RiderEdit, Riders};

/// The settings and where they are saved (`None`: not saved).
#[derive(Resource, Default)]
pub struct UserSettings {
    pub settings: Settings,
    pub path: Option<PathBuf>,
}

impl UserSettings {
    /// The value of `key`, which must be registered.
    pub fn value(&self, key: &str) -> Value {
        self.settings.get(key).expect("a registered setting")
    }

    pub(crate) fn save(&self) -> Result<(), String> {
        match &self.path {
            Some(p) => self.settings.save(p),
            None => Err("no configuration directory".into()),
        }
    }
}

/// The rider profiles, and the rider of the running ride.
#[derive(Resource, Default)]
pub struct RiderProfiles {
    pub riders: Riders,
    pub path: Option<PathBuf>,
    /// Who rides now: the active profile at the start, or `--rider`.
    /// Changes to the profiles apply from the next start.
    pub riding: Rider,
}

impl RiderProfiles {
    pub(crate) fn save(&self) -> Result<(), String> {
        match &self.path {
            Some(p) => self.riders.save(p),
            None => Err("no configuration directory".into()),
        }
    }

    /// A note when the next start would ride another profile than now.
    fn next_start_note(&self) -> &'static str {
        if self.riders.active() == self.riding {
            ""
        } else {
            " (applies from the next start; R restarts)"
        }
    }
}

/// The physics profile and display name of the local rider: from
/// [`RiderProfiles`] if the app loaded them, else the default rider.
pub fn local_rider(world: &World) -> Rider {
    world
        .get_resource::<RiderProfiles>()
        .map(|r| r.riding.clone())
        .unwrap_or_default()
}

/// Whether autodrive is on at the start (`controls.autodrive`).
pub fn autodrive_at_start(world: &World) -> bool {
    world
        .get_resource::<UserSettings>()
        .is_none_or(|s| s.value("controls.autodrive").as_bool().expect("a switch"))
}

/// `settings.toml`: `--settings-file`, else in the config directory.
pub fn settings_path(given: Option<&Path>) -> Option<PathBuf> {
    given
        .map(Path::to_path_buf)
        .or_else(|| crate::known_devices::config_dir().map(|d| d.join("settings.toml")))
}

/// `riders.toml`, next to the settings file.
pub fn riders_path(settings: &Path) -> PathBuf {
    settings.with_file_name("riders.toml")
}

/// What [`load`] found, for the app's resources.
pub struct Loaded {
    pub settings: UserSettings,
    pub riders: RiderProfiles,
    /// Problems to report (also printed to the console at the start).
    pub warnings: Vec<String>,
}

/// Loads settings and riders and applies them to the command-line
/// arguments: a setting read at the start (`ride.bots`, …) replaces the
/// option's default unless the option was given (`matches` tells), and a
/// given option becomes the setting's session value. The display name is
/// the riding profile's unless `--name` was given.
pub fn load(args: &mut Args, matches: &ArgMatches) -> Loaded {
    let mut warnings = Vec::new();
    let path = settings_path(args.settings_file.as_deref());
    let settings = match path.as_deref().map(Settings::load) {
        Some(Ok((s, w))) => {
            warnings.extend(w);
            UserSettings {
                settings: s,
                path: path.clone(),
            }
        }
        Some(Err(e)) => {
            // Keep the broken file: use the defaults and do not save.
            warnings.push(format!("{e}; using the default settings, not saving them"));
            UserSettings::default()
        }
        None => UserSettings::default(),
    };
    let rpath = path.as_deref().map(riders_path);
    let riders = match rpath.as_deref().map(Riders::load) {
        Some(Ok((r, w))) => {
            warnings.extend(w);
            r
        }
        Some(Err(e)) => {
            warnings.push(format!("{e}; riding as the default rider"));
            Riders::default()
        }
        None => Riders::default(),
    };
    let mut riding = riders.active();
    if let Some(name) = &args.rider {
        match riders.find(name) {
            Some(i) => riding = riders.riders[i].clone(),
            None => warnings.push(format!(
                "--rider {name}: no such profile, riding as {}",
                riding.name
            )),
        }
    }
    let mut out = Loaded {
        settings,
        riders: RiderProfiles {
            riders,
            path: rpath,
            riding,
        },
        warnings,
    };
    apply_to_args(args, matches, &mut out.settings.settings);
    if !from_command_line(matches, "name") {
        args.name.clone_from(&out.riders.riding.name);
    }
    out
}

fn from_command_line(matches: &ArgMatches, id: &str) -> bool {
    matches.value_source(id) == Some(ValueSource::CommandLine)
}

/// Settings ↔ command-line options (see [`load`]).
pub fn apply_to_args(args: &mut Args, matches: &ArgMatches, settings: &mut Settings) {
    for d in SETTINGS {
        let Apply::Arg(id) = d.apply else { continue };
        if from_command_line(matches, id) {
            let v = match id {
                "power" => Value::Int(args.power.round() as i64),
                "bots" => Value::Int(i64::from(args.bots)),
                "fullscreen" => Value::Bool(args.fullscreen),
                "random_start" => Value::Bool(args.random_start.unwrap_or(true)),
                "record" => Value::Bool(args.record.unwrap_or(true)),
                _ => unreachable!("unmapped option {id}"),
            };
            settings.set_command_line(d.key, v);
            continue;
        }
        // Only values from the file: the defaults are the options' own
        // (screenshot runs keep their reproducible defaults).
        let Some(v) = settings.file_value(d.key) else {
            continue;
        };
        match (id, v) {
            ("power", Value::Int(w)) => args.power = w as f64,
            ("bots", Value::Int(n)) => args.bots = n as u32,
            ("fullscreen", Value::Bool(b)) => args.fullscreen = b,
            ("random_start", Value::Bool(b)) => args.random_start = Some(b),
            ("record", Value::Bool(b)) => args.record = Some(b),
            (id, v) => unreachable!("option {id} with value {v:?}"),
        }
    }
}

/// Problems found while loading, printed to the console at the start.
#[derive(Resource, Default)]
pub struct LoadWarnings(pub Vec<String>);

/// Registers the commands; the app inserts [`UserSettings`] and
/// [`RiderProfiles`] from [`load`] (defaults that are not saved otherwise).
pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UserSettings>()
            .init_resource::<RiderProfiles>()
            .init_resource::<LoadWarnings>()
            .add_systems(Startup, print_warnings);
        register_commands(app);
    }
}

fn print_warnings(warnings: Res<LoadWarnings>, mut console: ResMut<ConsoleState>) {
    for w in &warnings.0 {
        warn!("{w}");
        console.print(format!("settings: {w}"));
    }
}

fn rider_args(c: Command) -> Command {
    c.arg(
        Arg::new("weight")
            .long("weight")
            .value_name("KG")
            .value_parser(value_parser!(f64))
            .help("Body weight in kg (30 to 200)"),
    )
    .arg(
        Arg::new("height")
            .long("height")
            .value_name("CM")
            .value_parser(value_parser!(f64))
            .help("Body height in cm"),
    )
    .arg(
        Arg::new("ftp")
            .long("ftp")
            .value_name("W")
            .value_parser(value_parser!(f64))
            .help("Functional threshold power in W"),
    )
    .arg(
        Arg::new("bike")
            .long("bike")
            .value_name("PROFILE")
            .value_parser(parse_bike)
            .help(bike_help()),
    )
    .arg(colour_arg(
        "frame_color",
        "frame-color",
        "Frame colour, #RRGGBB",
    ))
    .arg(colour_arg(
        "jersey_color",
        "jersey-color",
        "Jersey colour, #RRGGBB",
    ))
    .arg(colour_arg(
        "helmet_color",
        "helmet-color",
        "Helmet colour, #RRGGBB",
    ))
    .arg(
        Arg::new("kit")
            .long("kit")
            .value_name("PATTERN")
            .value_parser(|s: &str| {
                ocw_core::KitPattern::parse(s).ok_or_else(|| "plain, band or sash".to_string())
            })
            .help("Jersey pattern: plain, band or sash"),
    )
}

fn bike_help() -> String {
    let ids: Vec<&str> = ocw_core::ProfileId::ALL.iter().map(|p| p.id()).collect();
    format!("Bike (physics profile): {}", ids.join(", "))
}

fn parse_bike(s: &str) -> Result<ocw_core::ProfileId, String> {
    ocw_core::ProfileId::parse(s).ok_or_else(|| format!("unknown bike {s:?}; {}", bike_help()))
}

fn colour_arg(id: &'static str, long: &'static str, help: &'static str) -> Arg {
    Arg::new(id)
        .long(long)
        .value_name("#RRGGBB")
        .value_parser(|s: &str| {
            ocw_core::Rgb::parse(s).ok_or_else(|| format!("{s:?} is not a colour #RRGGBB"))
        })
        .help(help)
}

fn ftp(m: &ArgMatches) -> Option<u32> {
    m.get_one::<f64>("ftp").map(|f| f.round().max(0.0) as u32)
}

/// The bike and look options of `riders.add` / `riders.edit`.
fn bike_edit(m: &ArgMatches, mut edit: RiderEdit) -> RiderEdit {
    edit.bike = m.get_one::<ocw_core::ProfileId>("bike").copied();
    edit.frame = m.get_one::<ocw_core::Rgb>("frame_color").copied();
    edit.jersey = m.get_one::<ocw_core::Rgb>("jersey_color").copied();
    edit.helmet = m.get_one::<ocw_core::Rgb>("helmet_color").copied();
    edit.pattern = m.get_one::<ocw_core::KitPattern>("kit").copied();
    edit
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("set")
            .about("Change a setting and save it in settings.toml (--temp: this session only)")
            .arg(
                Arg::new("temp")
                    .long("temp")
                    .action(ArgAction::SetTrue)
                    .help("Only for this session, not saved"),
            )
            .arg(Arg::new("key").required(true))
            .arg(Arg::new("value").required(true).num_args(1..)),
        |world, m| {
            let key = m.get_one::<String>("key").expect("required").clone();
            let value = m
                .get_many::<String>("value")
                .expect("required")
                .cloned()
                .collect::<Vec<_>>()
                .join(" ");
            let temp = m.get_flag("temp");
            let mut us = world.resource_mut::<UserSettings>();
            let v = if temp {
                us.settings.set_temp(&key, &value)?
            } else {
                us.settings.set(&key, &value)?
            };
            let mut out = format!("{key} = {v}");
            if temp {
                out.push_str(" (this session)");
            } else if let Err(e) = us.save() {
                out.push_str(&format!(" (not saved: {e})"));
            }
            let d = def(&key).expect("set checked it");
            match d.scope {
                Scope::Live => {
                    if let Some(line) = d.apply_command(&v) {
                        world.resource_mut::<ConsoleState>().queue(line);
                    }
                }
                Scope::NextStart => out.push_str(", from the next start (R restarts)"),
            }
            Ok(out)
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("get")
            .about("Show a setting (with its range and description), or all settings")
            .arg(Arg::new("key")),
        |world, m| {
            let us = world.resource::<UserSettings>();
            let s = &us.settings;
            if let Some(key) = m.get_one::<String>("key") {
                let d = def(key).ok_or_else(|| format!("unknown setting {key:?}"))?;
                return Ok(format!(
                    "{}\n  {} ({}; default {}; {})",
                    s.line(key)?,
                    d.description,
                    d.kind_text(),
                    d.default,
                    match d.scope {
                        Scope::Live => "applies at once",
                        Scope::NextStart => "applies from the next start",
                    }
                ));
            }
            let mut out: Vec<String> = SETTINGS
                .iter()
                .map(|d| s.line(d.key).expect("registered"))
                .collect();
            let unknown = s.unknown_keys();
            if !unknown.is_empty() {
                out.push(format!("unknown keys (kept): {}", unknown.join(", ")));
            }
            out.push(match &us.path {
                Some(p) => format!("file: {}", p.display()),
                None => "file: none (not saved)".into(),
            });
            Ok(out.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("riders.list")
            .about("Show the rider profiles (* = chosen for the next start)"),
        |world, _| {
            let p = world.resource::<RiderProfiles>();
            let mut out = Vec::new();
            for r in &p.riders.riders {
                let mark = if p.riders.is_active(&r.name) {
                    '*'
                } else {
                    ' '
                };
                out.push(format!("{mark} {}: {}", r.name, r.summary()));
            }
            if out.is_empty() {
                out.push("no profiles (riders.add NAME --weight KG adds one)".into());
            }
            out.push(format!(
                "riding now: {} ({})",
                p.riding.name,
                p.riding.summary()
            ));
            if let Some(path) = &p.path {
                out.push(format!("file: {}", path.display()));
            }
            Ok(out.join("\n"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        rider_args(
            Command::new("riders.add")
                .about("Add a rider profile (the first one becomes the chosen rider)")
                .arg(Arg::new("name").required(true)),
        ),
        |world, m| {
            let rider = Rider {
                name: m.get_one::<String>("name").expect("required").clone(),
                weight_kg: m.get_one::<f64>("weight").copied().unwrap_or(75.0),
                height_cm: m.get_one::<f64>("height").copied(),
                ftp_w: ftp(m),
                ..Rider::default()
            };
            let mut p = world.resource_mut::<RiderProfiles>();
            p.riders.add(rider.clone())?;
            let bike = bike_edit(m, RiderEdit::default());
            let rider = if bike == RiderEdit::default() {
                rider
            } else {
                p.riders.edit(&rider.name, bike)?
            };
            p.save()?;
            Ok(format!(
                "added {}: {}{}",
                rider.name,
                rider.summary(),
                p.next_start_note()
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        rider_args(
            Command::new("riders.edit")
                .about("Change a rider profile")
                .arg(Arg::new("name").required(true))
                .arg(
                    Arg::new("rename")
                        .long("rename")
                        .value_name("NEW")
                        .help("New name"),
                )
                .arg(
                    Arg::new("no_height")
                        .long("no-height")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("height"),
                )
                .arg(
                    Arg::new("no_ftp")
                        .long("no-ftp")
                        .action(ArgAction::SetTrue)
                        .conflicts_with("ftp"),
                ),
        ),
        |world, m| {
            let name = m.get_one::<String>("name").expect("required").clone();
            let edit = RiderEdit {
                name: m.get_one::<String>("rename").cloned(),
                weight_kg: m.get_one::<f64>("weight").copied(),
                height_cm: if m.get_flag("no_height") {
                    Some(None)
                } else {
                    m.get_one::<f64>("height").map(|h| Some(*h))
                },
                ftp_w: if m.get_flag("no_ftp") {
                    Some(None)
                } else {
                    ftp(m).map(Some)
                },
                ..RiderEdit::default()
            };
            let edit = bike_edit(m, edit);
            let bike_changed = edit.bike.is_some()
                || edit.frame.is_some()
                || edit.jersey.is_some()
                || edit.helmet.is_some()
                || edit.pattern.is_some();
            let mut p = world.resource_mut::<RiderProfiles>();
            let r = p.riders.edit(&name, edit)?;
            p.save()?;
            let bike = if bike_changed {
                format!("; {}", r.bike_summary())
            } else {
                String::new()
            };
            Ok(format!(
                "{}: {}{bike}{}",
                r.name,
                r.summary(),
                p.next_start_note()
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("riders.remove")
            .about("Delete a rider profile")
            .arg(Arg::new("name").required(true)),
        |world, m| {
            let name = m.get_one::<String>("name").expect("required");
            let mut p = world.resource_mut::<RiderProfiles>();
            let r = p.riders.remove(name)?;
            p.save()?;
            Ok(format!("deleted {}{}", r.name, p.next_start_note()))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("riders.select")
            .about("Choose who rides (weight for the physics and the trainer, from the next start)")
            .arg(Arg::new("name").required(true)),
        |world, m| {
            let name = m.get_one::<String>("name").expect("required");
            let mut p = world.resource_mut::<RiderProfiles>();
            let r = p.riders.select(name)?;
            p.save()?;
            Ok(format!(
                "riding as {} ({}){}",
                r.name,
                r.summary(),
                p.next_start_note()
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, FromArgMatches};
    use ocw_render::console::ConsoleCorePlugin;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Parses the command line as `main` does and loads the settings.
    fn startup(argv: &[&str]) -> (Args, Loaded) {
        let matches = Args::command()
            .try_get_matches_from(argv)
            .expect("valid arguments");
        let mut args = Args::from_arg_matches(&matches).unwrap();
        let loaded = load(&mut args, &matches);
        (args, loaded)
    }

    #[test]
    fn settings_fill_in_options_the_command_line_did_not_give() {
        let dir = temp_dir("args");
        let file = dir.join("settings.toml");
        std::fs::write(
            &file,
            "[ride]\nbots = 2\nrecord = false\n[trainer]\nvirtual_power = 150\n[display]\nfullscreen = true\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("riders.toml"),
            "active = \"Anna\"\n[[rider]]\nname = \"Alex\"\nweight_kg = 80.0\n[[rider]]\nname = \"Anna\"\nweight_kg = 60.0\n",
        )
        .unwrap();
        let f = file.to_str().unwrap();
        let (args, loaded) = startup(&["ocw-desktop", "--settings-file", f]);
        assert_eq!(
            (args.bots, args.power, args.record),
            (2, 150.0, Some(false))
        );
        assert!(args.fullscreen);
        // Not in the file: the option's own default.
        assert_eq!(args.random_start, None);
        // The display name and weight are the active profile's.
        assert_eq!(args.name, "Anna");
        assert_eq!(loaded.riders.riding.physics().rider_mass_kg, 60.0);
        assert!(loaded.warnings.is_empty(), "{:?}", loaded.warnings);
        // The command line wins for the session and shows as such.
        let (args, loaded) = startup(&[
            "ocw-desktop",
            "--settings-file",
            f,
            "--bots",
            "7",
            "--name",
            "Guest",
            "--rider",
            "alex",
        ]);
        assert_eq!(args.bots, 7);
        assert_eq!(args.name, "Guest");
        assert_eq!(loaded.riders.riding.name, "Alex");
        let s = &loaded.settings.settings;
        assert_eq!(s.line("ride.bots").unwrap(), "ride.bots = 7 (command line)");
        // ... and is not saved.
        assert!(s.to_toml().contains("bots = 2"));
        let (_, loaded) = startup(&["ocw-desktop", "--settings-file", f, "--rider", "Ghost"]);
        assert_eq!(loaded.riders.riding.name, "Anna");
        assert_eq!(loaded.warnings.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_files_mean_defaults_and_a_broken_file_is_kept() {
        let dir = temp_dir("defaults");
        let file = dir.join("settings.toml");
        let f = file.to_str().unwrap();
        let (args, loaded) = startup(&["ocw-desktop", "--settings-file", f]);
        assert_eq!(
            (args.bots, args.power, args.name.as_str()),
            (10, 200.0, "Rider")
        );
        assert_eq!(loaded.riders.riding, Rider::default());
        assert_eq!(loaded.settings.path.as_deref(), Some(file.as_path()));
        // The registry defaults are the options' defaults.
        for (key, value) in [
            ("ride.bots", args.bots.to_string()),
            ("trainer.virtual_power", format!("{}", args.power)),
        ] {
            assert_eq!(loaded.settings.value(key).to_string(), value, "{key}");
        }
        std::fs::write(&file, "[display\n").unwrap();
        let (_, loaded) = startup(&["ocw-desktop", "--settings-file", f]);
        assert_eq!(loaded.warnings.len(), 1);
        assert_eq!(loaded.settings.path, None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn app(dir: &Path) -> App {
        let path = dir.join("settings.toml");
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            ConsoleCorePlugin,
            SettingsPlugin,
            crate::audio::GameAudioPlugin {
                backend: crate::audio::Backend::Null,
            },
            // `sky.night` / `sky.mode` / `sky.lights` (Display settings).
            ocw_render::sky::SkyPlugin,
            // `ortho` (display.orthophoto).
            ocw_render::ortho::OrthoPlugin,
        ))
        .insert_resource(UserSettings {
            settings: Settings::default(),
            path: Some(path.clone()),
        })
        .insert_resource(RiderProfiles {
            riders: Riders::default(),
            path: Some(riders_path(&path)),
            riding: Rider::default(),
        })
        // Registered by the devices plugin in the app.
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("trainer.difficulty").arg(
                Arg::new("percent")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
            |_, _| Ok(String::new()),
        ));
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        let log = app.world().resource::<ConsoleState>().log().to_vec();
        let at = log.iter().rposition(|l| l == &format!("> {line}")).unwrap();
        log[at + 1..].to_vec()
    }

    #[test]
    fn set_and_get_commands() {
        let dir = temp_dir("commands");
        let mut app = app(&dir);
        assert_eq!(run(&mut app, "set display.fps on")[0], "display.fps = on");
        // A live setting is applied at once with its command.
        assert!(app.world().resource::<ocw_render::hud::ShowFps>().0);
        // The orthophoto corridor (spec §14 Phase 8 T7): on by default.
        assert!(
            app.world()
                .resource::<ocw_render::ortho::OrthoSettings>()
                .enabled
        );
        assert_eq!(
            run(&mut app, "set display.orthophoto off")[0],
            "display.orthophoto = off"
        );
        assert!(
            !app.world()
                .resource::<ocw_render::ortho::OrthoSettings>()
                .enabled
        );
        let saved = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(saved.contains("[display]\nfps = true"), "{saved}");
        assert_eq!(
            run(&mut app, "set ride.bots 2")[0],
            "ride.bots = 2, from the next start (R restarts)"
        );
        assert!(
            run(&mut app, "set ride.bots 99")[0]
                .starts_with("error: ride.bots: 99 is out of range")
        );
        assert!(run(&mut app, "set nope 1")[0].starts_with("error: unknown setting"));
        assert_eq!(
            run(&mut app, "set --temp display.camera top")[0],
            "display.camera = top (this session)"
        );
        assert_eq!(
            *app.world().resource::<ocw_render::camera::CameraMode>(),
            ocw_render::camera::CameraMode::Top
        );
        let saved = std::fs::read_to_string(dir.join("settings.toml")).unwrap();
        assert!(!saved.contains("camera"), "{saved}");
        let get = run(&mut app, "get display.camera");
        assert_eq!(get[0], "display.camera = top (temporary)");
        assert!(get[1].contains("chase, side, top"));
        let all = run(&mut app, "get");
        assert_eq!(all.len(), SETTINGS.len() + 1);
        assert!(all.iter().any(|l| l == "ride.bots = 2 (settings.toml)"));
        assert!(all.last().unwrap().starts_with("file: "));
        // `set` is a user command: it works without dev permission.
        assert_eq!(
            app.world().resource::<ConsoleState>().permission,
            Permission::User
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rider_commands_manage_the_profiles() {
        let dir = temp_dir("riders");
        let mut app = app(&dir);
        assert!(run(&mut app, "riders.list")[0].starts_with("no profiles"));
        assert_eq!(
            run(&mut app, "riders.add Alex --weight 78 --ftp 250")[0],
            "added Alex: 78 kg, FTP 250 W (applies from the next start; R restarts)"
        );
        run(&mut app, "riders.add \"Anna B\" --weight 61.5 --height 168");
        assert!(run(&mut app, "riders.add Heavy --weight 260")[0].starts_with("error: weight"));
        assert_eq!(
            run(&mut app, "riders.select \"anna b\"")[0],
            "riding as Anna B (61.5 kg, 168 cm) (applies from the next start; R restarts)"
        );
        assert_eq!(
            run(&mut app, "riders.edit Alex --weight 77 --no-ftp")[0],
            "Alex: 77 kg (applies from the next start; R restarts)"
        );
        let list = run(&mut app, "riders.list");
        assert_eq!(list[0], "  Alex: 77 kg");
        assert_eq!(list[1], "* Anna B: 61.5 kg, 168 cm");
        assert_eq!(list[2], "riding now: Rider (75 kg)");
        // Saved: the next start rides Anna B with 61.5 kg.
        let (r, _) = Riders::load(&dir.join("riders.toml")).unwrap();
        assert_eq!(r.active().physics().rider_mass_kg, 61.5);
        assert!(run(&mut app, "riders.remove Nobody")[0].starts_with("error: no rider"));
        run(&mut app, "riders.remove \"Anna B\"");
        assert_eq!(
            app.world().resource::<RiderProfiles>().riders.active().name,
            "Alex"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Bike and look from the console (spec §2.5): validated, saved in
    /// riders.toml, the bike picks the physics profile.
    #[test]
    fn riders_bike_options() {
        let dir = temp_dir("bike");
        let mut app = app(&dir);
        assert_eq!(
            run(&mut app, "riders.add Alex --weight 78 --bike tt")[0],
            "added Alex: 78 kg (applies from the next start; R restarts)"
        );
        assert_eq!(
            run(
                &mut app,
                "riders.edit Alex --bike gravel --frame-color #1E88E5 --kit sash"
            )[0],
            "Alex: 78 kg; Gravel, frame #1E88E5, jersey #FF1472 (sash) (applies from the next start; R restarts)"
        );
        for bad in [
            "riders.edit Alex --bike rocket",
            "riders.edit Alex --frame-color blue",
            "riders.edit Alex --kit stars",
        ] {
            assert!(run(&mut app, bad)[0].starts_with("error"), "{bad}");
        }
        let (r, _) = Riders::load(&dir.join("riders.toml")).unwrap();
        let alex = r.active();
        assert_eq!(
            alex.physics(),
            ocw_core::RiderProfile::curated(ocw_core::ProfileId::Gravel, 78.0)
        );
        assert_eq!(alex.look().frame, ocw_core::Rgb(0x1E88E5));
        assert_eq!(alex.look().pattern, ocw_core::KitPattern::Sash);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Every row of every page runs a command line the console accepts
    /// (registered command, valid arguments) – except the ones that need
    /// the rest of the app (devices, Strava, the HUD), checked by name.
    #[test]
    fn page_rows_run_valid_commands() {
        let dir = temp_dir("pages");
        let mut app = app(&dir);
        run(&mut app, "riders.add Alex --weight 78");
        let mut lines = Vec::new();
        for page in pages::Page::TABS
            .into_iter()
            .chain([pages::Page::Rider(0), pages::Page::Bike(0)])
        {
            let rows = {
                let w = app.world();
                pages::rows(
                    page,
                    &w.resource::<UserSettings>().settings,
                    &w.resource::<RiderProfiles>().riders,
                )
            };
            for r in rows {
                lines.extend(r.left.into_iter().chain(r.right));
                if let Some(pages::Activate::Command(c)) = r.confirm {
                    lines.push(c);
                }
            }
        }
        let elsewhere = ["devices.", "strava.", "hud", "god."];
        for line in lines {
            // God mode's settings apply through its own plugin (`god.invert`).
            if elsewhere.iter().any(|p| line.starts_with(p))
                || line.contains("riders.remove")
                || line.contains("controls.invert_y")
            {
                continue;
            }
            let out = run(&mut app, &line);
            assert!(
                out.iter().all(|l| !l.starts_with("error")),
                "{line}: {out:?}"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

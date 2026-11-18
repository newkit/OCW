//! The activities folder (spec §2.8.1 *Where an S recording is written*,
//! v4.79 / v4.80 / v4.82): the **one folder for everything** OCW saves
//! for the user. Screenshots and the photos taken outside a Strava
//! recording go into the day's subfolder (`~/Activities/OCW/2026-11-18/`,
//! the [`PhotoFolder`] resource, kept up to date by [`day_folder`]); every
//! Strava (S … S) recording is written **always**
//! into its own subfolder of `~/Activities/OCW` (the home folder +
//! `Activities/OCW` on every platform; setting `activities.folder`,
//! console `activities.folder [PATH|default|choose]`, Settings → Account
//! → Folders; `OCW_ACTIVITIES_DIR` for the default in test runs, and the
//! unit tests never use the home folder) – its FIT file and the photos
//! the ride photographer takes while it runs, named by the shared rules
//! of [`ocw_client::activities`]:
//!
//! `~/Activities/OCW/2026-11-18 Puerto de la Cruz/OCW 2026-11-18 10-05 Puerto de la Cruz.fit`
//! and `… photo 1.jpg`.
//!
//! The internal copy in `<data dir>/ocw/rides/strava/` stays as the
//! history's and the API sync's record; its sidecar keeps the path of the
//! folder's FIT file (`RideMeta::folder`) for the simple upload, which
//! opens that folder. Photos are written once, into the recording's
//! folder; their sidecars in `rides/<ride>/photos/` point there.
//!
//! The photos folder of v4.74 – v4.82 (`~/Pictures/OCW`, setting
//! `photos.folder`) is gone; the photos saved there stay where they are
//! and are found through their sidecars.
//!
//! *Change* (`activities.folder choose`) opens the system's folder picker – `zenity` or `kdialog` on Linux,
//! PowerShell's folder dialog on Windows, `osascript` on macOS – on a
//! thread; without one (or in test runs) the console opens with
//! `set activities.folder ` to type the path.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::activities::{self, DEFAULT_SUBFOLDER, Target};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::stamp::PhotoFolder;

use crate::strava::manual::{OpenCmd, Os};

/// Test runs set this for the default folder (never the real home).
pub const ENV_DIR: &str = "OCW_ACTIVITIES_DIR";

/// The activities folder and its name in the HUD ("Activities/OCW").
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct ActivitiesFolder {
    pub dir: PathBuf,
    pub label: String,
}

/// The default: `home/Activities/OCW`.
pub fn default_folder(home: Option<&Path>) -> Option<PathBuf> {
    let mut p = home?.to_path_buf();
    for s in DEFAULT_SUBFOLDER {
        p.push(s);
    }
    Some(p)
}

/// The folder for the setting: `default` (or empty) the default, `~/…`
/// below `home`, else the path.
pub fn resolve(setting: &str, home: Option<&Path>) -> Option<PathBuf> {
    let s = setting.trim().trim_matches('"');
    if s.is_empty() || s == "default" {
        return default_folder(home);
    }
    if s == "~" {
        return home.map(Path::to_path_buf);
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        return home.map(|h| h.join(rest));
    }
    Some(PathBuf::from(s))
}

/// The folder's name in the HUD: below the home folder its path from
/// there ("Activities/OCW"), else as `folder_label` gives it.
pub fn label(dir: &Path, home: Option<&Path>) -> String {
    // The home folder as the "Pictures" root of `folder_label`: a folder
    // below it is named from its first part on.
    if let Some(h) = home
        && let Ok(rest) = dir.strip_prefix(h)
        && rest.components().next().is_some()
    {
        let parts: Vec<String> = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        let full = parts.join("/");
        if full.chars().count() <= ocw_render::photo_preview::MAX_LABEL {
            return full;
        }
    }
    ocw_render::photo_preview::folder_label(dir, None, home)
}

/// The activities folder for `setting` on this machine. The default is
/// `OCW_ACTIVITIES_DIR` when set; unit tests never get the home folder.
pub fn activities_folder(setting: &str) -> Option<ActivitiesFolder> {
    let default = matches!(setting.trim().trim_matches('"'), "" | "default");
    let env = std::env::var(ENV_DIR).ok().filter(|d| !d.is_empty());
    let home = dirs::home_dir();
    // A test folder is named like the real one.
    let (dir, label) = match env {
        Some(d) if default => (PathBuf::from(d), DEFAULT_SUBFOLDER.join("/")),
        _ if default && cfg!(test) => (
            std::env::temp_dir()
                .join(format!("ocw-test-home-{}", std::process::id()))
                .join("Activities")
                .join("OCW"),
            DEFAULT_SUBFOLDER.join("/"),
        ),
        _ => {
            let dir = resolve(setting, home.as_deref())?;
            let l = label(&dir, home.as_deref());
            (dir, l)
        }
    };
    Some(ActivitiesFolder { dir, label })
}

/// The day's folder for screenshots and loose photos: `root/YYYY-MM-DD`
/// (the local date at `unix_s` + `offset_s`), named `Activities/OCW/2026-11-18`.
pub fn day_folder_of(root: &ActivitiesFolder, unix_s: i64, offset_s: i32) -> PhotoFolder {
    let (date, _) = activities::local_date_time(unix_s, offset_s);
    PhotoFolder {
        dir: root.dir.join(&date),
        label: format!("{}/{date}", root.label),
    }
}

/// Keeps the [`PhotoFolder`] on the activities folder's day folder (a new
/// day, another activities folder).
pub fn day_folder(world: &mut World) {
    let Some(root) = world.get_resource::<ActivitiesFolder>().cloned() else {
        return;
    };
    let unix_s = crate::recording::unix_now();
    let f = day_folder_of(&root, unix_s, ocw_render::media::local_offset_s(unix_s));
    if world.get_resource::<PhotoFolder>() != Some(&f) {
        world.insert_resource(f);
    }
}

/// A running recording's folder: where it is and its files' names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordingDir {
    pub dir: PathBuf,
    pub target: Target,
    /// As the HUD names it: `Activities/OCW/2026-11-18 Puerto de la Cruz`.
    pub label: String,
}

impl RecordingDir {
    /// Makes the recording's subfolder in `root` (the first free name).
    pub fn create(
        root: &ActivitiesFolder,
        start_unix_s: i64,
        offset_s: i32,
        place: &str,
    ) -> Result<Self, String> {
        let target = Target::new(start_unix_s, offset_s, place, |n| root.dir.join(n).exists());
        let dir = root.dir.join(&target.folder);
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Self {
            label: target.label(&root.label),
            dir,
            target,
        })
    }

    /// The next photo's path (counted).
    pub fn next_photo(&mut self) -> PathBuf {
        self.dir.join(self.target.next_photo())
    }

    /// Writes the recording's FIT file; returns its path.
    pub fn write_fit(&self, bytes: &[u8]) -> Result<PathBuf, String> {
        let path = self.dir.join(self.target.fit_name());
        let tmp = path.with_extension("fit.part");
        std::fs::write(&tmp, bytes).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    /// A recording that was not saved (too short): its folder goes if
    /// nothing is in it.
    pub fn discard(&self) {
        let _ = std::fs::remove_dir(&self.dir);
    }
}

/// The place a recording starting now is named after (the nearest town
/// or village of the package's places to the rider, else the route, else
/// the world) and the world's UTC offset at `unix_s`.
pub fn place_and_offset(world: &World, unix_s: i64) -> (String, i32) {
    let client = world
        .get_resource::<crate::connection::ClientState>()
        .map(|c| &c.0);
    let at = client.filter(|c| c.rider_id().is_some()).map(|c| {
        let s = c.state();
        let p = c
            .track()
            .sample(s.distance)
            .offset_position(s.lateral_offset);
        [p[0], p[2]]
    });
    let route = client
        .map(|c| c.course_name().to_string())
        .unwrap_or_default();
    let world_name = world
        .get_resource::<crate::ride::RideCatalog>()
        .map(|c| crate::ride::world::current_name(&c.0))
        .filter(|n| n != "-")
        .unwrap_or_default();
    let places = world
        .get_resource::<ocw_render::package::PackageWorld>()
        .and_then(|p| p.home().places.clone());
    let place = activities::place_name(places.as_ref(), at, &route, &world_name);
    let offset = world
        .get_resource::<ocw_ui::timemachine::WorldPlace>()
        .map(|p| p.zone.offset_s(unix_s * 1000))
        .unwrap_or_else(|| ocw_render::media::local_offset_s(unix_s));
    (place, offset)
}

/// The system's folder pickers to try, in order (each prints the chosen
/// folder; nothing when cancelled).
pub fn picker_commands(os: Os, title: &str) -> Vec<OpenCmd> {
    match os {
        Os::Linux => vec![
            OpenCmd {
                program: "zenity",
                args: vec![
                    "--file-selection".into(),
                    "--directory".into(),
                    format!("--title={title}"),
                ],
                raw: false,
            },
            OpenCmd {
                program: "kdialog",
                args: vec![
                    "--getexistingdirectory".into(),
                    dirs::home_dir()
                        .map(|h| h.to_string_lossy().into_owned())
                        .unwrap_or_else(|| ".".into()),
                    "--title".into(),
                    title.into(),
                ],
                raw: false,
            },
        ],
        Os::Windows => vec![OpenCmd {
            program: "powershell",
            args: vec![
                "-NoProfile".into(),
                "-Command".into(),
                format!(
                    "Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = '{}'; if ($d.ShowDialog() -eq 'OK') {{ $d.SelectedPath }}",
                    title.replace('\'', "''")
                ),
            ],
            raw: false,
        }],
        Os::Mac => vec![OpenCmd {
            program: "osascript",
            args: vec![
                "-e".into(),
                format!(
                    "POSIX path of (choose folder with prompt \"{}\")",
                    title.replace('"', "'")
                ),
            ],
            raw: false,
        }],
    }
}

/// The folder a picker printed (`None`: cancelled).
pub fn picked(output: &str) -> Option<String> {
    let s = output.lines().map(str::trim).find(|l| !l.is_empty())?;
    let s = s.trim_end_matches(['/', '\\']);
    (!s.is_empty()).then(|| s.to_string())
}

/// What a picker found, for the frame to apply: (setting, the result).
#[derive(Resource, Default, Clone)]
pub struct Picked(pub Arc<Mutex<Vec<PickResult>>>);

/// A picker's answer for a setting: the folder (`None`: cancelled).
pub type PickResult = (String, Result<Option<String>, String>);

/// Runs the pickers for `setting` on a thread; `Err` when none starts
/// (the caller opens the console instead).
fn pick_folder(world: &mut World, setting: &str, title: &str) -> Result<(), String> {
    let cmds = picker_commands(Os::current(), title);
    let out = world.get_resource_or_insert_with(Picked::default).0.clone();
    let setting = setting.to_string();
    // Is any picker there? (a quick start of each, in order)
    let first = cmds
        .iter()
        .position(|c| {
            std::process::Command::new(c.program)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .stdin(std::process::Stdio::null())
                .status()
                .is_ok()
        })
        .ok_or("no folder picker on this system")?;
    let cmd = cmds[first].clone();
    std::thread::Builder::new()
        .name("ocw-folder-picker".into())
        .spawn(move || {
            let r = std::process::Command::new(cmd.program)
                .args(&cmd.args)
                .stdin(std::process::Stdio::null())
                .output()
                .map(|o| picked(&String::from_utf8_lossy(&o.stdout)))
                .map_err(|e| format!("{}: {e}", cmd.program));
            out.lock().expect("not poisoned").push((setting, r));
        })
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// `… choose`: the folder picker, or the console to type the path.
pub fn choose(world: &mut World, setting: &str, title: &str) -> Result<String, String> {
    let open = world
        .get_resource::<crate::strava::manual::ManualOpen>()
        .is_some_and(|o| o.0);
    if open && pick_folder(world, setting, title).is_ok() {
        return Ok(format!("{title}: choose a folder in the window that opens"));
    }
    if let Some(mut c) = world.get_resource_mut::<ConsoleState>() {
        c.set_open(true);
        c.set_input(format!("set {setting} "));
    }
    Ok(format!(
        "type the folder after \"set {setting} \" (~/ for the home folder) and press Enter"
    ))
}

/// Applies what a picker found (`set KEY "PATH"`, saved in the settings).
fn apply_picked(picked: Option<Res<Picked>>, console: Option<ResMut<ConsoleState>>) {
    let (Some(picked), Some(mut console)) = (picked, console) else {
        return;
    };
    let done: Vec<_> = std::mem::take(&mut *picked.0.lock().expect("not poisoned"));
    for (key, r) in done {
        match r {
            Ok(Some(path)) => console.queue(format!("set {key} \"{}\"", path.replace('"', ""))),
            Ok(None) => info!("{key}: no folder chosen"),
            Err(e) => warn!("{key}: {e}"),
        }
    }
}

pub struct ActivitiesPlugin;

impl Plugin for ActivitiesPlugin {
    fn build(&self, app: &mut App) {
        if let Some(f) = activities_folder("default") {
            app.insert_resource(f);
        }
        day_folder(app.world_mut());
        app.init_resource::<Picked>()
            .add_systems(Update, (apply_picked, day_folder));
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("activities.folder")
                .about("The one folder for everything OCW saves for you: the Strava recordings (S), each in its own subfolder with its FIT file and photos, and the screenshots and other photos in a folder per day. default (~/Activities/OCW), a path (~/ for the home folder), choose (a folder picker), or nothing to show it (setting activities.folder)")
                .arg(Arg::new("folder")),
            |world, m| {
                match m.get_one::<String>("folder").map(String::as_str) {
                    Some("choose") => {
                        return choose(world, "activities.folder", "Activities folder for the Strava recordings");
                    }
                    Some(s) => {
                        let f = activities_folder(s).ok_or("no home folder for the activities")?;
                        world.insert_resource(f);
                    }
                    None => {}
                }
                let f = world
                    .get_resource::<ActivitiesFolder>()
                    .ok_or("no activities folder")?;
                Ok(format!(
                    "everything goes to {} ({}): each Strava recording in its own folder, screenshots and photos in a folder per day",
                    f.dir.display(),
                    f.label
                ))
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> ActivitiesFolder {
        let dir =
            std::env::temp_dir().join(format!("ocw-activities-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        ActivitiesFolder {
            dir: dir.join("Activities/OCW"),
            label: "Activities/OCW".into(),
        }
    }

    /// `~/Activities/OCW` on every platform; the setting takes a path or
    /// `~/…`; the HUD names it from the home folder on.
    #[test]
    fn the_folder_per_setting() {
        let home = PathBuf::from("/home/ada");
        assert_eq!(
            resolve("default", Some(&home)),
            Some(home.join("Activities").join("OCW"))
        );
        assert_eq!(resolve("", Some(&home)), resolve("default", Some(&home)));
        assert_eq!(resolve("~/rides", Some(&home)), Some(home.join("rides")));
        assert_eq!(
            resolve("\"/srv/ocw\"", Some(&home)),
            Some(PathBuf::from("/srv/ocw"))
        );
        assert_eq!(resolve("default", None), None);
        assert_eq!(
            label(&home.join("Activities/OCW"), Some(&home)),
            "Activities/OCW"
        );
        assert_eq!(label(Path::new("/srv/ocw"), Some(&home)), "/srv/ocw");
        // The unit tests never get the real home folder.
        let f = activities_folder("default").unwrap();
        if std::env::var(ENV_DIR).is_err() {
            assert!(
                f.dir.starts_with(std::env::temp_dir()),
                "{}",
                f.dir.display()
            );
        }
        assert_eq!(f.label, "Activities/OCW");
        assert!(f.dir.ends_with("OCW"));
    }

    /// A recording's folder: made at its start, ` 2` for the next one at
    /// that place on that day, its FIT and photos inside; a too-short one
    /// leaves no empty folder.
    #[test]
    fn a_recordings_folder_and_its_files() {
        let root = root("dir");
        let t = 1_794_992_700; // 2026-11-18 09:05 UTC
        let mut a = RecordingDir::create(&root, t, 3600, "Puerto de la Cruz").unwrap();
        assert_eq!(a.dir, root.dir.join("2026-11-18 Puerto de la Cruz"));
        assert_eq!(a.label, "Activities/OCW/2026-11-18 Puerto de la Cruz");
        assert!(a.dir.is_dir());
        let b = RecordingDir::create(&root, t + 600, 3600, "Puerto de la Cruz").unwrap();
        assert_eq!(b.dir, root.dir.join("2026-11-18 Puerto de la Cruz 2"));
        let fit = a.write_fit(b"FIT").unwrap();
        assert_eq!(
            fit,
            a.dir.join("OCW 2026-11-18 10-05 Puerto de la Cruz.fit")
        );
        assert_eq!(
            a.next_photo(),
            a.dir
                .join("OCW 2026-11-18 10-05 Puerto de la Cruz photo 1.jpg")
        );
        b.discard();
        assert!(!b.dir.exists(), "an empty folder goes");
        a.discard();
        assert!(a.dir.exists(), "a folder with files stays");
        let _ = std::fs::remove_dir_all(root.dir.parent().unwrap().parent().unwrap());
    }

    /// Screenshots and loose photos: the day's folder of the activities
    /// folder (v4.82: one folder for everything), the local date.
    #[test]
    fn the_day_folder() {
        let root = root("day");
        // 2026-11-18 23:30 UTC: the 18th at UTC+0, the 19th at UTC+1.
        let f = day_folder_of(&root, 1_795_044_600, 0);
        assert_eq!(f.dir, root.dir.join("2026-11-18"));
        assert_eq!(f.label, "Activities/OCW/2026-11-18");
        assert_eq!(
            day_folder_of(&root, 1_795_044_600, 3600).label,
            "Activities/OCW/2026-11-19"
        );
        // In an app: set from the activities folder, changed with it.
        let mut w = World::new();
        w.insert_resource(root.clone());
        day_folder(&mut w);
        let p = w.resource::<PhotoFolder>().clone();
        assert!(p.dir.starts_with(&root.dir) && p.label.starts_with("Activities/OCW/20"));
        let other = ActivitiesFolder {
            dir: root.dir.join("x"),
            label: "~/x".into(),
        };
        w.insert_resource(other.clone());
        day_folder(&mut w);
        assert!(w.resource::<PhotoFolder>().dir.starts_with(&other.dir));
    }

    #[test]
    fn the_folder_pickers_per_system() {
        let l = picker_commands(Os::Linux, "Activities folder");
        assert_eq!(l[0].program, "zenity");
        assert!(l[0].args.contains(&"--directory".to_string()));
        assert_eq!(l[1].program, "kdialog");
        let w = picker_commands(Os::Windows, "Ada's folder");
        assert_eq!(w[0].program, "powershell");
        assert!(w[0].args[2].contains("FolderBrowserDialog"));
        assert!(w[0].args[2].contains("Ada''s folder"), "quoted");
        let m = picker_commands(Os::Mac, "x");
        assert!(m[0].args[1].starts_with("POSIX path of (choose folder"));
        assert_eq!(picked("/home/ada/Rides/\n"), Some("/home/ada/Rides".into()));
        assert_eq!(picked("\n  \n"), None);
        assert_eq!(
            picked("C:\\Users\\Ada\\Rides\r\n"),
            Some("C:\\Users\\Ada\\Rides".into())
        );
    }
}

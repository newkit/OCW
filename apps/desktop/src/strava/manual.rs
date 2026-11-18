//! The simple Strava upload on the desktop (spec §2.8.1 *Simple upload*):
//! no API application, Strava's own web upload page
//! ([`ocw_client::strava::UPLOAD_PAGE`]) takes our FIT files. For a ride
//! of the training history (its page's row *Upload to Strava (in your
//! browser)*, `strava.upload-manual N`) the app
//!
//! 1. copies the ride's FIT file – or its Strava recordings (S … S), if it
//!    has any – and its photos into `<rides>/strava-upload/` under names a
//!    rider recognises in a file picker
//!    ([`ocw_client::strava::upload_name`]); the folder holds only this
//!    upload (older copies are removed), the saved rides keep their names,
//! 2. opens that folder in the file manager with the file selected
//!    (`xdg-open` on the folder, `explorer /select,` on Windows, `open -R`
//!    on macOS: [`reveal_command`]),
//! 3. opens Strava's upload page in the default browser
//!    ([`browser_command`]),
//! 4. shows the steps (menu and HUD).
//!
//! The app cannot know whether the rider then uploaded it; the rider marks
//! the ride (`strava.mark-uploaded`), and [`marked`] keeps the marked files
//! in `strava-manual.toml`, which the API sync skips
//! ([`super::sync::ManualMarks`]).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use ocw_client::history::RideEntry;

use super::sync::ManualMarks;

/// The folder of the files to upload, under the rides folder.
pub const UPLOAD_DIR: &str = "strava-upload";

/// Whether the upload opens the file manager and the browser (off in test
/// runs, `--strava-dry-run`, and without the resource).
#[derive(Resource, Clone, Copy)]
pub struct ManualOpen(pub bool);

/// The system, for the commands that open a folder or a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Linux,
    Windows,
    Mac,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::Mac
        } else {
            Self::Linux
        }
    }
}

/// A program to start: its arguments, `raw` = passed as they are, without
/// Rust's quoting (Windows' `explorer /select,"…"` needs that).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenCmd {
    pub program: &'static str,
    pub args: Vec<String>,
    pub raw: bool,
}

/// Shows `file` in the file manager: its folder (Linux: `xdg-open` cannot
/// select a file), the file selected (Windows, macOS).
pub fn reveal_command(os: Os, file: &Path) -> OpenCmd {
    match os {
        Os::Linux => OpenCmd {
            program: "xdg-open",
            args: vec![
                file.parent()
                    .unwrap_or(Path::new("."))
                    .to_string_lossy()
                    .into_owned(),
            ],
            raw: false,
        },
        Os::Windows => OpenCmd {
            program: "explorer",
            args: vec![format!("/select,\"{}\"", file.to_string_lossy())],
            raw: true,
        },
        Os::Mac => OpenCmd {
            program: "open",
            args: vec!["-R".into(), file.to_string_lossy().into_owned()],
            raw: false,
        },
    }
}

/// Opens `url` in the default browser (`start` takes its first quoted
/// argument as the window's title, hence the empty one).
pub fn browser_command(os: Os, url: &str) -> OpenCmd {
    match os {
        Os::Linux => OpenCmd {
            program: "xdg-open",
            args: vec![url.into()],
            raw: false,
        },
        Os::Windows => OpenCmd {
            program: "cmd",
            args: vec!["/C".into(), "start".into(), String::new(), url.into()],
            raw: false,
        },
        Os::Mac => OpenCmd {
            program: "open",
            args: vec![url.into()],
            raw: false,
        },
    }
}

/// Starts `cmd` (not waited for; its output is dropped).
pub fn spawn(cmd: &OpenCmd) -> Result<(), String> {
    let mut c = std::process::Command::new(cmd.program);
    for a in &cmd.args {
        #[cfg(windows)]
        if cmd.raw {
            use std::os::windows::process::CommandExt;
            c.raw_arg(a);
            continue;
        }
        c.arg(a);
    }
    c.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("cannot start {}: {e}", cmd.program))
}

/// The Strava recordings (`strava/<name>.fit`) that lie within `ride`:
/// (the name as the sync knows it, its start in UTC Unix s), oldest first.
pub fn recordings_within(rides: &Path, ride: &RideEntry) -> Vec<(String, i64)> {
    let end = ride.start_unix_s + ride.elapsed_s.ceil() as i64;
    let mut out: Vec<(String, i64)> = std::fs::read_dir(super::recording::strava_dir(rides))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fit"))
        .filter_map(|p| {
            let a = ocw_fit::decode(&std::fs::read(&p).ok()?).ok()?;
            let name = p.file_name()?.to_string_lossy().into_owned();
            // A recording starts at or after the ride's first record
            // (a second of slack) and before its end.
            (a.start_unix_s + 1 >= ride.start_unix_s && a.start_unix_s <= end)
                .then(|| (format!("strava/{name}"), a.start_unix_s))
        })
        .collect();
    out.sort();
    out
}

/// What [`prepare`] put into the upload folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub dir: PathBuf,
    /// The FIT files' names, in the order to upload.
    pub files: Vec<String>,
    /// The photos' names.
    pub photos: Vec<String>,
}

impl Prepared {
    /// The first FIT file (the one the file manager selects).
    pub fn first(&self) -> PathBuf {
        self.dir.join(self.files.first().map_or("", String::as_str))
    }
}

/// Copies what to upload of `ride` into `<rides>/strava-upload/`: its
/// Strava recordings if it has any (unless `whole`), else the ride's FIT
/// file, and its photos; the names from the local time at `offset_s`.
pub fn prepare(
    rides: &Path,
    ride: &RideEntry,
    offset_s: i32,
    whole: bool,
) -> Result<Prepared, String> {
    use ocw_client::strava::upload_name;
    let mut sources: Vec<(PathBuf, String)> = Vec::new();
    if !whole {
        for (name, start) in recordings_within(rides, ride) {
            sources.push((
                rides.join(&name),
                upload_name(
                    start,
                    offset_s,
                    &ride.world,
                    &ride.route,
                    Some("Strava recording"),
                ),
            ));
        }
    }
    if sources.is_empty() {
        let fit = ride
            .fit
            .as_ref()
            .ok_or("this ride's FIT file is not kept")?;
        let path = rides.join(fit);
        if !path.is_file() {
            return Err(format!(
                "this ride's FIT file is missing: {}",
                path.display()
            ));
        }
        sources.push((
            path,
            upload_name(ride.start_unix_s, offset_s, &ride.world, &ride.route, None),
        ));
    }
    // Two recordings in the same minute: the second gets a number.
    let mut seen = std::collections::BTreeSet::new();
    for (_, name) in &mut sources {
        let stem = name.trim_end_matches(".fit").to_string();
        let mut n = 1;
        while !seen.insert(name.clone()) {
            n += 1;
            *name = format!("{stem} {n}.fit");
        }
    }
    let dir = rides.join(UPLOAD_DIR);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    // Only this upload in the folder: older copies go.
    for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_file()
            && p.extension()
                .is_some_and(|x| x == "fit" || x == "jpg" || x == "jpeg")
        {
            let _ = std::fs::remove_file(&p);
        }
    }
    let mut files = Vec::new();
    for (from, name) in &sources {
        std::fs::copy(from, dir.join(name)).map_err(|e| format!("{}: {e}", from.display()))?;
        files.push(name.clone());
    }
    // The ride's photos (spec §2.11), to add on the activity page.
    let stem = files[0].trim_end_matches(".fit").to_string();
    let mut photos = Vec::new();
    let photo_dir = crate::photos::photos_dir(rides, &ride.id);
    for (k, (file, _, _)) in crate::photos::ride_photos(rides, &ride.id)
        .iter()
        .enumerate()
    {
        let name = format!("{stem} photo {}.jpg", k + 1);
        if std::fs::copy(photo_dir.join(file), dir.join(&name)).is_ok() {
            photos.push(name);
        }
    }
    Ok(Prepared { dir, files, photos })
}

fn rides_dir(world: &World) -> Result<PathBuf, String> {
    world
        .get_resource::<crate::recording::Recording>()
        .and_then(|r| r.dir.clone())
        .ok_or_else(|| "no rides folder".to_string())
}

/// The history's upload hook (`strava.upload-manual`, the ride page's
/// row): prepares the files, opens the folder and Strava's upload page,
/// shows the steps in the HUD; returns them (the menu shows them on the
/// ride's page).
pub fn upload(world: &mut World, ride: &RideEntry, whole: bool) -> Result<String, String> {
    let rides = rides_dir(world)?;
    let p = prepare(
        &rides,
        ride,
        ocw_ui::history::offset(ride.start_unix_s),
        whole,
    )?;
    info!(folder = %p.dir.display(), files = ?p.files, "Strava upload in the browser");
    if world.get_resource::<ManualOpen>().is_some_and(|o| o.0) {
        let os = Os::current();
        spawn(&reveal_command(os, &p.first()))?;
        spawn(&browser_command(os, ocw_client::strava::UPLOAD_PAGE))?;
    }
    let steps = ocw_client::strava::upload_steps(&p.files, true, !p.photos.is_empty());
    let now = crate::recording::game_now(world);
    if let Some(mut s) = world.get_resource_mut::<super::recording::StravaRec>() {
        s.0.say_for(steps.clone(), now, ocw_client::strava::UPLOAD_NOTICE_S);
    }
    Ok(steps)
}

/// The ride's files as the sync knows them: its FIT file and its Strava
/// recordings.
pub fn sync_names(rides: &Path, ride: &RideEntry) -> Vec<String> {
    let mut out: Vec<String> = ride.fit.iter().cloned().collect();
    out.extend(recordings_within(rides, ride).into_iter().map(|(n, _)| n));
    out
}

/// The history's mark hook: the API sync skips a marked ride (and its
/// Strava recordings), so setting up the API later uploads no duplicate.
pub fn marked(world: &mut World, ride: &RideEntry) {
    let Ok(rides) = rides_dir(world) else {
        return;
    };
    let mut marks = ManualMarks::load(&rides);
    marks.set(&sync_names(&rides, ride), ride.strava_marked);
    if let Err(e) = marks.save(&rides) {
        warn!("Strava marks: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_client::history::RideContext;
    use ocw_client::recording::{RideRecorder, Sample};
    use ocw_world::Route;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ocw-manual-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A saved ride of `n` seconds from `start` (FIT file and sidecar).
    fn ride(dir: &Path, start: i64, n: i64) -> (PathBuf, ocw_client::recording::FinishedRide) {
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        for i in 0..n {
            r.observe(
                &route,
                None,
                Sample {
                    game_s: i as f64,
                    distance: i as f64 * 8.0,
                    speed: 8.0,
                    power_w: 200.0,
                    ..Sample::default()
                },
                start + i,
            );
        }
        let f = r.finish().unwrap();
        (crate::recording::save_ride(dir, &f).unwrap(), f)
    }

    fn entry(path: &Path, f: &ocw_client::recording::FinishedRide) -> RideEntry {
        RideEntry::from_activity(
            &f.activity,
            RideContext {
                id: path.file_stem().unwrap().to_string_lossy().into_owned(),
                world: "Tenerife".into(),
                route: "Puerto de la Cruz–Teide".into(),
                fit: Some(path.file_name().unwrap().to_string_lossy().into_owned()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn the_open_commands_per_system() {
        let file = Path::new(
            "/home/ada/.local/share/ocw/rides/strava-upload/OCW 2026-11-18 10-05 Loop.fit",
        );
        assert_eq!(
            reveal_command(Os::Linux, file),
            OpenCmd {
                program: "xdg-open",
                args: vec!["/home/ada/.local/share/ocw/rides/strava-upload".into()],
                raw: false
            }
        );
        assert_eq!(
            reveal_command(Os::Mac, file),
            OpenCmd {
                program: "open",
                args: vec!["-R".into(), file.to_string_lossy().into_owned()],
                raw: false
            }
        );
        let win = Path::new(
            r"C:\Users\Ada\AppData\Roaming\ocw\rides\strava-upload\OCW 2026-11-18 10-05 Loop.fit",
        );
        assert_eq!(
            reveal_command(Os::Windows, win),
            OpenCmd {
                program: "explorer",
                args: vec![format!("/select,\"{}\"", win.display())],
                raw: true
            }
        );
        let url = ocw_client::strava::UPLOAD_PAGE;
        assert_eq!(browser_command(Os::Linux, url).args, vec![url.to_string()]);
        assert_eq!(browser_command(Os::Mac, url).program, "open");
        let w = browser_command(Os::Windows, url);
        assert_eq!((w.program, w.raw), ("cmd", false));
        assert_eq!(w.args, vec!["/C", "start", "", url]);
    }

    /// The upload folder gets the ride's FIT file under its recognisable
    /// name (only this upload in it); the saved ride stays as it is.
    #[test]
    fn the_whole_ride_is_copied_under_its_name() {
        let dir = temp_dir("whole");
        let (path, f) = ride(&dir, 1_794_992_700, 30);
        let e = entry(&path, &f);
        std::fs::create_dir_all(dir.join(UPLOAD_DIR)).unwrap();
        std::fs::write(dir.join(UPLOAD_DIR).join("old.fit"), b"x").unwrap();
        std::fs::write(dir.join(UPLOAD_DIR).join("keep.txt"), b"x").unwrap();
        let p = prepare(&dir, &e, 3600, false).unwrap();
        assert_eq!(
            p.files,
            vec!["OCW 2026-11-18 10-05 Tenerife – Puerto de la Cruz–Teide.fit".to_string()]
        );
        assert!(p.photos.is_empty());
        assert_eq!(
            std::fs::read(p.first()).unwrap(),
            std::fs::read(&path).unwrap()
        );
        assert!(!p.dir.join("old.fit").exists(), "older copies go");
        assert!(p.dir.join("keep.txt").exists());
        assert!(path.exists(), "the saved ride keeps its name");
        // A ride whose file is gone, or not kept.
        let mut gone = e.clone();
        gone.fit = Some("nope.fit".into());
        assert!(
            prepare(&dir, &gone, 0, false)
                .unwrap_err()
                .contains("missing")
        );
        gone.fit = None;
        assert!(
            prepare(&dir, &gone, 0, false)
                .unwrap_err()
                .contains("not kept")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A ride with Strava recordings uploads those (each S … S its own
    /// activity), `--whole` the ride; the marks cover the ride and its
    /// recordings.
    #[test]
    fn a_rides_strava_recordings_are_uploaded_and_marked() {
        let dir = temp_dir("recordings");
        let (path, f) = ride(&dir, 1_794_992_700, 600);
        let e = entry(&path, &f);
        let strava = super::super::recording::strava_dir(&dir);
        let (a, _) = ride(&strava, 1_794_992_700 + 60, 30);
        let (b, _) = ride(&strava, 1_794_992_700 + 300, 30);
        // Another ride's recording (a day later) is not this ride's.
        ride(&strava, 1_794_992_700 + 86_400, 30);
        let within = recordings_within(&dir, &e);
        assert_eq!(within.len(), 2);
        assert_eq!(
            within[0].0,
            format!("strava/{}", a.file_name().unwrap().to_string_lossy())
        );
        let p = prepare(&dir, &e, 0, false).unwrap();
        assert_eq!(
            p.files,
            vec![
                "OCW 2026-11-18 09-06 Tenerife – Puerto de la Cruz–Teide (Strava recording).fit"
                    .to_string(),
                "OCW 2026-11-18 09-10 Tenerife – Puerto de la Cruz–Teide (Strava recording).fit"
                    .to_string(),
            ]
        );
        assert_eq!(
            std::fs::read(p.first()).unwrap(),
            std::fs::read(&a).unwrap()
        );
        let w = prepare(&dir, &e, 0, true).unwrap();
        assert_eq!(w.files.len(), 1);
        assert!(!w.files[0].contains("Strava recording"));
        assert_eq!(std::fs::read_dir(&w.dir).unwrap().count(), 1);
        let names = sync_names(&dir, &e);
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], path.file_name().unwrap().to_string_lossy());
        assert!(names.contains(&format!(
            "strava/{}",
            b.file_name().unwrap().to_string_lossy()
        )));
        let _ = std::fs::remove_dir_all(&dir);
    }
}

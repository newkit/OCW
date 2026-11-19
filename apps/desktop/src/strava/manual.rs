//! The simple Strava upload on the desktop (spec §2.8.1 *Simple upload*):
//! no API application, Strava's own web upload page
//! ([`ocw_client::strava::UPLOAD_PAGE`]) takes our FIT files. For a ride
//! of the training history (its page's row *Upload to Strava (in your
//! browser)*, `strava.upload-manual N`) the app
//!
//! 1. takes the ride's Strava recordings (S … S) – only those: whole
//!    rides never go to Strava (spec 2.8.1 Q6; a ride without one has no
//!    upload) – from their own folders in the activities folder
//!    (`~/Activities/OCW/<date> <place>/`, spec 2.8.1 *Where an S
//!    recording is written*: each recording's FIT file and photos are
//!    written there always, so nothing is copied) – or, for recordings
//!    from before v4.85 without such a folder (the fallback), copies the
//!    FIT file(s) and the ride's photos into `<rides>/strava-upload/` under
//!    names a rider recognises in a file picker
//!    ([`ocw_client::strava::upload_name`]); that folder holds only this
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

/// Every Strava recording (`strava/<name>.fit`, its start in UTC Unix
/// s), by name.
pub fn recordings(rides: &Path) -> Vec<(String, i64)> {
    let mut out: Vec<(String, i64)> = std::fs::read_dir(super::recording::strava_dir(rides))
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "fit"))
        .filter_map(|p| {
            let a = ocw_fit::decode(&std::fs::read(&p).ok()?).ok()?;
            let name = p.file_name()?.to_string_lossy().into_owned();
            Some((format!("strava/{name}"), a.start_unix_s))
        })
        .collect();
    out.sort();
    out
}

/// Those of `all` ([`recordings`]) that lie within `ride`: a recording
/// starts at or after the ride's first record (a second of slack) and
/// before its end.
pub fn within(all: &[(String, i64)], ride: &RideEntry) -> Vec<(String, i64)> {
    let end = ride.start_unix_s + ride.elapsed_s.ceil() as i64;
    all.iter()
        .filter(|(_, a)| a + 1 >= ride.start_unix_s && *a <= end)
        .cloned()
        .collect()
}

/// The Strava recordings (`strava/<name>.fit`) that lie within `ride`:
/// (the name as the sync knows it, its start in UTC Unix s), oldest first.
pub fn recordings_within(rides: &Path, ride: &RideEntry) -> Vec<(String, i64)> {
    within(&recordings(rides), ride)
}

/// What [`prepare`] found or put into the upload folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Prepared {
    pub dir: PathBuf,
    /// The FIT files' names, in the order to upload.
    pub files: Vec<String>,
    /// The photos' names.
    pub photos: Vec<String>,
    /// The recording's own folder as the HUD names it
    /// (`Activities/OCW/2026-11-18 Puerto de la Cruz`); `None`: the copy
    /// in `strava-upload/` (older recordings).
    pub folder: Option<String>,
}

/// A folder's name for the steps: below the activities folder from its
/// label on ("Activities/OCW/2026-11-18 Puerto de la Cruz"), else its path.
pub fn folder_name(dir: &Path, root: Option<&crate::activities::ActivitiesFolder>) -> String {
    if let Some(r) = root
        && let Ok(rest) = dir.strip_prefix(&r.dir)
    {
        let parts: Vec<String> = rest
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts.is_empty() {
            return r.label.clone();
        }
        return format!("{}/{}", r.label, parts.join("/"));
    }
    dir.display().to_string()
}

/// The ride's Strava recordings in their own folders (spec §2.8.1 *Where
/// an S recording is written*): each recording's FIT file there, if every
/// recording of the ride has one (`None`: none, or an older one without).
pub fn recording_folders(rides: &Path, ride: &RideEntry) -> Option<Vec<PathBuf>> {
    let recs = recordings_within(rides, ride);
    if recs.is_empty() {
        return None;
    }
    recs.iter()
        .map(|(name, _)| {
            crate::recording::load_meta(&rides.join(name))?
                .folder
                .map(PathBuf::from)
                .filter(|p| p.is_file())
        })
        .collect()
}

impl Prepared {
    /// The first FIT file (the one the file manager selects).
    pub fn first(&self) -> PathBuf {
        self.dir.join(self.files.first().map_or("", String::as_str))
    }
}

/// What to upload of `ride`: its Strava recordings (whole rides never go
/// to Strava, spec 2.8.1 Q6 – `Err` without one) in their own folders,
/// else (older recordings) copied with the ride's photos into
/// `<rides>/strava-upload/`; the names from the local time at `offset_s`.
pub fn prepare(
    rides: &Path,
    ride: &RideEntry,
    offset_s: i32,
    root: Option<&crate::activities::ActivitiesFolder>,
) -> Result<Prepared, String> {
    use ocw_client::strava::upload_name;
    // The recordings' own folders: nothing to copy, the first one opens
    // with its FIT file selected; its photos are beside it.
    if let Some(fits) = recording_folders(rides, ride) {
        let dir = fits[0].parent().unwrap_or(Path::new(".")).to_path_buf();
        let name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
        let files: Vec<String> = fits.iter().map(PathBuf::as_path).filter_map(name).collect();
        let mut photos: Vec<String> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "jpg" || x == "jpeg"))
            .filter_map(|p| name(&p))
            .collect();
        photos.sort_by_key(|n| photo_number(n));
        return Ok(Prepared {
            folder: Some(folder_name(&dir, root)),
            dir,
            files,
            photos,
        });
    }
    let mut sources: Vec<(PathBuf, String)> = Vec::new();
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
    if sources.is_empty() {
        return Err(
            "this ride has no Strava recording – whole rides never go to Strava; press S while riding to record one"
                .into(),
        );
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
    // They live in the activities folder (or an older ~/Pictures/OCW),
    // found through their
    // sidecars in `rides/<ride>/photos/` (older rides: beside them).
    let mut photos = Vec::new();
    for (k, (_, from)) in crate::photos::ride_photo_files(rides, &ride.id)
        .iter()
        .enumerate()
    {
        let name = format!("{stem} photo {}.jpg", k + 1);
        if std::fs::copy(from, dir.join(&name)).is_ok() {
            photos.push(name);
        }
    }
    Ok(Prepared {
        dir,
        files,
        photos,
        folder: None,
    })
}

/// The number in `… photo N.jpg` (for the order; others last).
fn photo_number(name: &str) -> (usize, String) {
    let n = name
        .rsplit_once(" photo ")
        .and_then(|(_, r)| r.split('.').next()?.parse().ok())
        .unwrap_or(usize::MAX);
    (n, name.to_string())
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
pub fn upload(world: &mut World, ride: &RideEntry) -> Result<String, String> {
    let rides = rides_dir(world)?;
    let root = world
        .get_resource::<crate::activities::ActivitiesFolder>()
        .cloned();
    let p = prepare(
        &rides,
        ride,
        ocw_ui::history::offset(ride.start_unix_s),
        root.as_ref(),
    )?;
    info!(folder = %p.dir.display(), files = ?p.files, "Strava upload in the browser");
    if world.get_resource::<ManualOpen>().is_some_and(|o| o.0) {
        let os = Os::current();
        spawn(&reveal_command(os, &p.first()))?;
        spawn(&browser_command(os, ocw_client::strava::UPLOAD_PAGE))?;
    }
    let steps =
        ocw_client::strava::upload_steps(&p.files, true, !p.photos.is_empty(), p.folder.as_deref());
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

    /// Whole rides never go to Strava (spec 2.8.1 Q6): a ride without a
    /// Strava recording has nothing to upload – nothing is copied, the
    /// upload folder stays as it is, the answer says to press S.
    #[test]
    fn a_ride_without_strava_recordings_uploads_nothing() {
        let dir = temp_dir("whole");
        let (path, f) = ride(&dir, 1_794_992_700, 30);
        let e = entry(&path, &f);
        std::fs::create_dir_all(dir.join(UPLOAD_DIR)).unwrap();
        std::fs::write(dir.join(UPLOAD_DIR).join("old.fit"), b"x").unwrap();
        let err = prepare(&dir, &e, 3600, None).unwrap_err();
        assert!(
            err.contains("no Strava recording") && err.contains("press S"),
            "{err}"
        );
        assert!(dir.join(UPLOAD_DIR).join("old.fit").exists(), "untouched");
        assert_eq!(std::fs::read_dir(dir.join(UPLOAD_DIR)).unwrap().count(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The ride's photos live where their sidecars say (here an older
    /// ~/Pictures/OCW, spec
    /// §2.11 Q3), referenced by their sidecars in `rides/<ride>/photos/`:
    /// the upload folder gets them under the FIT file's name.
    #[test]
    fn the_photos_from_the_photos_folder_go_with_the_upload() {
        let dir = temp_dir("photos");
        let (path, f) = ride(&dir, 1_794_992_700, 30);
        let e = entry(&path, &f);
        // Its Strava recording (from before v4.85: no folder of its own).
        ride(&super::super::recording::strava_dir(&dir), 1_794_992_700, 30);
        let pictures = dir.join("Pictures").join("OCW");
        let time = ocw_render::media::MediaTime {
            unix_s: 1_794_992_710,
            offset_s: 0,
        };
        let rgb = vec![90u8; 32 * 18 * 3];
        let photo = pictures.join("OCW 2026-11-18 photo.jpg");
        let m = crate::photos::tests::meta(
            1,
            "OCW 2026-11-18 photo.jpg".into(),
            Some(photo.to_string_lossy().into_owned()),
            &time,
        );
        crate::photos::write_photo(
            &photo,
            Some(&crate::photos::photos_dir(&dir, &e.id)),
            &rgb,
            32,
            18,
            &m,
            &time,
            "OCW",
        )
        .unwrap();
        let p = prepare(&dir, &e, 0, None).unwrap();
        assert_eq!(
            p.photos,
            vec![
                "OCW 2026-11-18 09-05 Tenerife – Puerto de la Cruz–Teide (Strava recording) photo 1.jpg"
                    .to_string()
            ]
        );
        assert_eq!(
            std::fs::read(p.dir.join(&p.photos[0])).unwrap(),
            std::fs::read(&photo).unwrap()
        );
        let steps = ocw_client::strava::upload_steps(&p.files, true, !p.photos.is_empty(), None);
        assert!(steps.contains("in the same folder"), "{steps}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A ride with Strava recordings uploads those (each S … S its own
    /// activity), never the whole ride (spec 2.8.1 Q6); the marks cover
    /// the ride and its recordings.
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
        let p = prepare(&dir, &e, 0, None).unwrap();
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
        assert!(p.files.iter().all(|f| f.contains("Strava recording")));
        assert_eq!(std::fs::read_dir(&p.dir).unwrap().count(), 2, "no whole ride");
        let names = sync_names(&dir, &e);
        assert_eq!(names.len(), 3);
        assert_eq!(names[0], path.file_name().unwrap().to_string_lossy());
        assert!(names.contains(&format!(
            "strava/{}",
            b.file_name().unwrap().to_string_lossy()
        )));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A ride whose Strava recording was written to its own folder in
    /// the activities folder (spec §2.8.1 *Where an S recording is
    /// written*): the upload opens that folder with the FIT file selected
    /// and copies nothing; its photos are there once. An older recording
    /// without such a folder falls back to the copy in `strava-upload/`
    /// (never the whole ride).
    #[test]
    fn the_upload_opens_the_recordings_own_folder() {
        let dir = temp_dir("folder");
        let (path, f) = ride(&dir, 1_794_992_700, 600);
        let e = entry(&path, &f);
        let root = crate::activities::ActivitiesFolder {
            dir: dir.join("home/Activities/OCW"),
            label: "Activities/OCW".into(),
        };
        // The recording as `handle` writes it: its folder's FIT, the
        // sidecar pointing there, a photo in the folder.
        let mut rec = crate::activities::RecordingDir::create(
            &root,
            1_794_992_760,
            3600,
            "Puerto de la Cruz",
        )
        .unwrap();
        let route = Route::test_loop();
        let mut r = RideRecorder::new("Loop");
        for i in 0..30 {
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
                1_794_992_760 + i,
            );
        }
        let mut seg = r.finish().unwrap();
        let fit = rec.write_fit(&ocw_fit::encode(&seg.activity)).unwrap();
        seg.meta.folder = Some(fit.to_string_lossy().into_owned());
        let internal =
            crate::recording::save_ride(&super::super::recording::strava_dir(&dir), &seg).unwrap();
        std::fs::write(rec.next_photo(), b"jpg").unwrap();
        std::fs::write(rec.next_photo(), b"jpg").unwrap();
        let p = prepare(&dir, &e, 3600, Some(&root)).unwrap();
        assert_eq!(p.dir, root.dir.join("2026-11-18 Puerto de la Cruz"));
        assert_eq!(
            p.files,
            vec!["OCW 2026-11-18 10-06 Puerto de la Cruz.fit".to_string()]
        );
        assert_eq!(p.first(), fit);
        assert_eq!(
            p.photos,
            vec![
                "OCW 2026-11-18 10-06 Puerto de la Cruz photo 1.jpg".to_string(),
                "OCW 2026-11-18 10-06 Puerto de la Cruz photo 2.jpg".to_string(),
            ]
        );
        assert_eq!(
            p.folder.as_deref(),
            Some("Activities/OCW/2026-11-18 Puerto de la Cruz")
        );
        assert!(!dir.join(UPLOAD_DIR).exists(), "nothing copied");
        // The file manager opens that folder (Linux) / selects the file.
        assert_eq!(
            reveal_command(Os::Linux, &p.first()).args,
            vec![p.dir.to_string_lossy().into_owned()]
        );
        assert_eq!(
            reveal_command(Os::Mac, &p.first()).args[1],
            fit.to_string_lossy()
        );
        let steps = ocw_client::strava::upload_steps(&p.files, true, true, p.folder.as_deref());
        assert!(
            steps.contains("its folder Activities/OCW/2026-11-18 Puerto de la Cruz is open"),
            "{steps}"
        );
        // The API sync still knows the internal record.
        assert!(sync_names(&dir, &e).contains(&format!(
            "strava/{}",
            internal.file_name().unwrap().to_string_lossy()
        )));
        // An older recording (no folder in its sidecar): the copy too.
        let side = internal.with_extension("toml");
        let mut meta = crate::recording::load_meta(&internal).unwrap();
        meta.folder = None;
        std::fs::write(&side, toml::to_string_pretty(&meta).unwrap()).unwrap();
        assert!(!std::fs::read_to_string(&side).unwrap().contains("folder"));
        let old = prepare(&dir, &e, 3600, Some(&root)).unwrap();
        assert_eq!(old.folder, None);
        assert_eq!(old.dir, dir.join(UPLOAD_DIR));
        assert!(
            old.files[0].ends_with("(Strava recording).fit"),
            "{:?}",
            old.files
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

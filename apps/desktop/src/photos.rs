//! The desktop's photos (spec §2.11, Q3): the ride photographer's photos
//! and the screenshots go to the **photos folder**, the platform's
//! Pictures folder's `OCW` (`dirs::picture_dir()`, else `~/Pictures`;
//! Linux: the XDG Pictures directory, Windows `Pictures\OCW`, macOS
//! `~/Pictures/OCW`; setting `photos.folder`, console `photos.folder`;
//! test runs set `OCW_PHOTOS_DIR` for the default).
//! A ride photo is `OCW-<ride>-photo-N.jpg` there (`<ride>`: the ride's
//! FIT file stem, its UTC start) with EXIF time and GPS position – no time
//! stamp drawn in the picture (§2.8.1 Q4); the ride keeps its reference:
//! the sidecar `<data dir>/ocw/rides/<ride>/photos/photo-N.toml` (time,
//! position, view, the score and its parts, the camera pose, the photo's
//! `path`), so the training history and the Strava recordings find the
//! photos. Rides before v4.73 have `photo-N.jpg` beside the sidecar and
//! keep working. The photographer itself is `ocw_ui::photographer`; this
//! module gives it the files and the sound, and keeps it off in scripted
//! screenshot and capture runs (like the ride recording).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_audio::mixer::MixerMsg;
use ocw_client::photographer::{CameraMeta, PhotoMeta, Taken, ride_photo_name};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::media::{GpsPosition, MediaTime, encode_jpeg, exif_tiff_gps};
use ocw_render::stamp::PhotoFolder;
use ocw_ui::photographer::{PhotographerHooks, PhotographerPlugin, RidePhotographer};

/// The photos folder's name in the Pictures folder.
pub const PHOTOS_SUBFOLDER: &str = "OCW";

/// The default photos folder: `pictures/OCW`, or `home/Pictures/OCW`
/// where the platform names no Pictures folder (a Linux box without XDG
/// user dirs).
pub fn default_folder(pictures: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    pictures
        .or_else(|| home.map(|h| h.join("Pictures")))
        .map(|p| p.join(PHOTOS_SUBFOLDER))
}

/// The photos folder for the setting `photos.folder`: `default` (or
/// empty) the [`default_folder`], `~/…` below `home`, else the path.
pub fn resolve_folder(
    setting: &str,
    pictures: Option<PathBuf>,
    home: Option<PathBuf>,
) -> Option<PathBuf> {
    let s = setting.trim();
    if s.is_empty() || s == "default" {
        return default_folder(pictures, home);
    }
    if let Some(rest) = s.strip_prefix("~/").or_else(|| s.strip_prefix("~\\")) {
        return home.map(|h| h.join(rest));
    }
    if s == "~" {
        return home;
    }
    Some(PathBuf::from(s))
}

/// The photos folder for `setting` on this machine, with its caption name
/// ("Pictures/OCW").
pub fn photo_folder(setting: &str) -> Option<PhotoFolder> {
    // Test runs keep their photos in their throwaway folder.
    let setting = match std::env::var("OCW_PHOTOS_DIR") {
        Ok(dir) if !dir.is_empty() && matches!(setting.trim(), "" | "default") => dir,
        _ => setting.to_string(),
    };
    let setting = setting.as_str();
    let (pictures, home) = (dirs::picture_dir(), dirs::home_dir());
    let pics_or_default = pictures
        .clone()
        .or_else(|| home.as_ref().map(|h| h.join("Pictures")));
    let dir = resolve_folder(setting, pictures, home.clone())?;
    let label =
        ocw_render::photo_preview::folder_label(&dir, pics_or_default.as_deref(), home.as_deref());
    Some(PhotoFolder { dir, label })
}

use crate::Args;
use crate::connection::ClientState;
use crate::recording::Recording;

/// JPEG quality of the photos.
pub const PHOTO_QUALITY: u8 = 92;

pub struct PhotosPlugin;

impl Plugin for PhotosPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        app.add_plugins(PhotographerPlugin {
            scale: args.photo_time_scale.max(1.0),
        })
        .insert_resource(PhotographerHooks {
            save: save_photo,
            sound: play,
        });
        // Scripted screenshot and capture runs take no photos (as they
        // record no ride), unless asked (`--record true`,
        // `photographer now`).
        let recording = app
            .world()
            .get_resource::<Recording>()
            .is_none_or(|r| r.enabled);
        app.world_mut().resource_mut::<RidePhotographer>().allowed = recording;
        // The photos folder (the setting applies `photos.folder` at the
        // start; this is the default until then).
        if let Some(f) = photo_folder("default") {
            app.insert_resource(f);
        }
        app.console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("photos.folder")
                .about("The folder for photos and screenshots: default (Pictures/OCW), a path (~/ for the home folder), or nothing to show it (setting photos.folder)")
                .arg(Arg::new("folder")),
            |world, m| {
                if let Some(s) = m.get_one::<String>("folder") {
                    let f = photo_folder(s).ok_or("no home folder to put the photos in")?;
                    world.insert_resource(f);
                }
                let f = world
                    .get_resource::<PhotoFolder>()
                    .ok_or("no photos folder")?;
                Ok(format!(
                    "photos go to {} ({})",
                    f.dir.display(),
                    f.label
                ))
            },
        ));
        if let Some(dir) = args.photo_chase_shots {
            app.insert_resource(ChaseShots {
                dir,
                n: 0,
                last: f64::NEG_INFINITY,
                preview: None,
            })
            .add_systems(PostUpdate, (chase_shots, preview_shots));
        }
    }
}

/// `--photo-chase-shots DIR` (test runs): the rider's view while the
/// photographer or the drone is in it, and 0.6 s after each photo with its
/// preview in the HUD.
#[derive(Resource)]
struct ChaseShots {
    dir: PathBuf,
    n: u32,
    last: f64,
    /// The preview last shot (its start, real clock) and whether it was.
    preview: Option<(f64, bool)>,
}

/// The window with the photo preview, once per preview (test runs).
fn preview_shots(
    mut commands: Commands,
    real: Res<Time<Real>>,
    preview: Option<Res<ocw_render::photo_preview::PhotoPreview>>,
    now: Res<ocw_render::stamp::MediaNow>,
    label: Option<Res<ocw_render::hud::VersionLabel>>,
    mut shots: ResMut<ChaseShots>,
) {
    let Some(start) = preview
        .as_ref()
        .and_then(|p| p.queue.shown())
        .map(|s| s.start_s)
    else {
        return;
    };
    let shot = match shots.preview {
        Some((s, done)) if s == start => done,
        _ => {
            shots.preview = Some((start, false));
            false
        }
    };
    if shot || real.elapsed_secs_f64() - start < 0.6 {
        return;
    }
    shots.preview = Some((start, true));
    shots.n += 1;
    let path = shots.dir.join(format!("chase-{:02}-preview.jpg", shots.n));
    let _ = std::fs::create_dir_all(&shots.dir);
    info!(path = %path.display(), "the photo's preview in the HUD");
    commands
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(ocw_render::stamp::save_shot(
            path,
            ocw_render::stamp::ShotMeta::new(&now, label.as_deref()),
        ));
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn chase_shots(
    mut commands: Commands,
    time: Res<Time>,
    view: Res<ocw_render::RideView>,
    frustum: Res<ocw_render::photographer::ViewFrustum>,
    now: Res<ocw_render::stamp::MediaNow>,
    label: Option<Res<ocw_render::hud::VersionLabel>>,
    mut shots: ResMut<ChaseShots>,
) {
    let v = |a: [f64; 3]| Vec3::new(a[0] as f32, a[1] as f32, a[2] as f32);
    let seen = |p: &ocw_render::photographer::PhotographerView| {
        let person = p.person.is_some_and(|f| frustum.sees(v(f.lens)));
        let drone = p
            .drone
            .is_some_and(|d| d.rotor > 0.5 && frustum.sees(v(d.pos)));
        let flash = p.flash.is_some_and(|(at, _)| frustum.sees(v(at)));
        (person, drone, flash)
    };
    // The own photographer, else another rider's (spec §2.11 Q2).
    let own = view.photographer.as_ref().map(seen);
    let other = view
        .photographers
        .iter()
        .map(seen)
        .find(|(p, d, _)| *p || *d);
    let (person, drone, flash, theirs) = match (own, other) {
        (Some((p, d, f)), _) if p || d => (p, d, f, false),
        (_, Some((p, d, f))) => (p, d, f, true),
        _ => return,
    };
    let t = time.elapsed_secs_f64();
    // Not in the first seconds (the window may not show the world yet).
    if t < 8.0 {
        return;
    }
    // A flash in view is shot at once (it lasts a tenth of a second).
    if !(person || drone) || (t - shots.last < 3.0 && !flash) || shots.n >= 60 {
        return;
    }
    shots.last = t;
    shots.n += 1;
    let kind = match (theirs, drone, flash) {
        (false, true, _) => "drone",
        (false, false, _) => "photographer",
        (true, _, true) => "other-flash",
        (true, true, false) => "other-drone",
        (true, false, false) => "other-photographer",
    };
    let path = shots.dir.join(format!("chase-{:02}-{kind}.jpg", shots.n));
    let _ = std::fs::create_dir_all(&shots.dir);
    info!(path = %path.display(), "chase view with the {kind}");
    commands
        .spawn(bevy::render::view::screenshot::Screenshot::primary_window())
        .observe(ocw_render::stamp::save_shot(
            path,
            ocw_render::stamp::ShotMeta::new(&now, label.as_deref()),
        ));
}

fn play(world: &mut World, msg: MixerMsg) {
    if let Some(a) = world.get_resource::<crate::audio::GameAudio>() {
        a.send(msg);
    }
}

/// `<rides>/<ride>/photos`.
pub fn photos_dir(rides: &Path, ride: &str) -> PathBuf {
    rides.join(ride).join("photos")
}

/// The sidecar of photo `n` in the ride's `photos` folder.
pub fn sidecar_name(n: usize) -> String {
    format!("photo-{n}.toml")
}

/// Writes the photo to `path` (JPEG with EXIF time and GPS) and, with a
/// `sidecar_dir`, its sidecar `photo-N.toml` there (the ride's reference
/// to it); returns the photo's path.
#[allow(clippy::too_many_arguments)]
pub fn write_photo(
    path: &Path,
    sidecar_dir: Option<&Path>,
    rgb: &[u8],
    w: u32,
    h: u32,
    meta: &PhotoMeta,
    time: &MediaTime,
    software: &str,
) -> Result<PathBuf, String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let gps = meta.lat.zip(meta.lon).map(|(lat, lon)| GpsPosition {
        lat,
        lon,
        height_m: meta.height_m,
    });
    let jpg = encode_jpeg(rgb, w, h, PHOTO_QUALITY, exif_tiff_gps(time, software, gps))?;
    let tmp = path.with_extension("jpg.part");
    std::fs::write(&tmp, jpg).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("{}: {e}", path.display()))?;
    if let Some(dir) = sidecar_dir {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let text = toml::to_string_pretty(meta).map_err(|e| e.to_string())?;
        let side = dir.join(sidecar_name(meta.n));
        std::fs::write(&side, text).map_err(|e| format!("{}: {e}", side.display()))?;
    }
    Ok(path.to_path_buf())
}

/// The ride's id (its FIT file's stem: the UTC start), or the time now
/// before its first record.
fn ride_id(world: &World) -> String {
    let start = world
        .get_resource::<Recording>()
        .and_then(|r| r.recorder.start_unix_s())
        .unwrap_or_else(crate::recording::unix_now);
    crate::recording::file_stem(start)
}

/// Where photo `n` of `ride` goes: in the photos folder
/// (`OCW-<ride>-photo-N.jpg`, a free name), else (no home folder) beside
/// its sidecar in the rides folder as before (`photo-N.jpg`); the caption
/// says where.
pub fn photo_target(
    folder: Option<&PhotoFolder>,
    rides: Option<&Path>,
    ride: &str,
    n: usize,
) -> Option<(PathBuf, String)> {
    if let Some(f) = folder
        && let Ok(p) = f.new_file(&ride_photo_name(ride, n))
    {
        return Some((p, format!("Saved to {}", f.label)));
    }
    let rides = rides?;
    Some((
        photos_dir(rides, ride).join(ocw_client::photographer::photo_name(n)),
        "Saved to the ride's folder".into(),
    ))
}

/// The photographer's hook: encodes and writes the photo off the main
/// thread; returns where it goes.
fn save_photo(world: &mut World, t: &Taken, image: Image) -> Option<String> {
    let rides = world
        .get_resource::<Recording>()
        .and_then(|r| r.dir.clone());
    let ride = ride_id(world);
    let folder = world.get_resource::<PhotoFolder>().cloned();
    let Some((path, mut went)) = photo_target(folder.as_ref(), rides.as_deref(), &ride, t.n) else {
        warn!("ride photo: no photos or rides folder");
        return None;
    };
    if rides.is_some() {
        went.push_str(", added to your ride");
    }
    let client = world.get_resource::<ClientState>().map(|c| &c.0);
    let geo = client.and_then(|c| c.geo_origin());
    let route = client
        .map(|c| c.course_name().to_string())
        .unwrap_or_default();
    let world_name = world
        .get_resource::<crate::ride::RideCatalog>()
        .map(|c| crate::ride::world::current_name(&c.0))
        .filter(|n| n != "-")
        .unwrap_or_default();
    let now = world
        .get_resource::<ocw_render::stamp::MediaNow>()
        .map(|m| m.0)
        .unwrap_or_else(|| {
            let unix_s = crate::recording::unix_now();
            MediaTime {
                unix_s,
                offset_s: ocw_render::media::local_offset_s(unix_s),
            }
        });
    let software = ocw_render::media::software(
        world
            .get_resource::<ocw_render::hud::VersionLabel>()
            .map_or("", |l| l.0.as_str()),
    );
    let (lat, lon) = match geo {
        Some((lat0, lon0)) => {
            let (a, b) = ocw_world::geo::LocalProjection::new(lat0, lon0)
                .unproject([t.pose.eye[0], t.pose.eye[2]]);
            (Some(a), Some(b))
        }
        None => (None, None),
    };
    let (yaw, pitch) = t.pose.yaw_pitch();
    let sidecar_dir = rides.as_deref().map(|r| photos_dir(r, &ride));
    let beside = sidecar_dir
        .as_deref()
        .is_some_and(|d| path.parent() == Some(d));
    let meta = PhotoMeta {
        n: t.n,
        file: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        // The ride's reference to the photo in the photos folder.
        path: (!beside).then(|| path.to_string_lossy().into_owned()),
        time_unix_s: now.unix_s,
        time: now.iso_local(),
        ride_s: t.ride_s,
        ride: ride.clone(),
        world: world_name,
        route,
        lat,
        lon,
        height_m: t.pose.eye[1],
        heading_deg: yaw.to_degrees().rem_euclid(360.0),
        pitch_deg: pitch.to_degrees(),
        score: t.score.into(),
        camera: CameraMeta::of(&t.pose),
    };
    let spawned = std::thread::Builder::new()
        .name("ocw-photo".into())
        .spawn(move || {
            let rgb = match image.try_into_dynamic() {
                Ok(i) => i.to_rgb8(),
                Err(e) => {
                    error!("ride photo: cannot convert the image: {e}");
                    return;
                }
            };
            match write_photo(
                &path,
                sidecar_dir.as_deref(),
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                &meta,
                &now,
                &software,
            ) {
                Ok(p) => info!(path = %p.display(), "ride photo saved"),
                Err(e) => error!("ride photo: {e}"),
            }
        });
    match spawned {
        Ok(_) => Some(went),
        Err(e) => {
            error!("ride photo: {e}");
            None
        }
    }
}

/// The photos of the ride `id` in `rides` that still exist: each sidecar
/// with the photo's path (the photos folder's, or beside the sidecar for
/// rides before v4.73), in ride order.
pub fn ride_photo_files(rides: &Path, id: &str) -> Vec<(PhotoMeta, PathBuf)> {
    let dir = photos_dir(rides, id);
    let mut out: Vec<(PhotoMeta, PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| {
            let m: PhotoMeta = toml::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            let at = m
                .path
                .as_ref()
                .map_or_else(|| dir.join(&m.file), PathBuf::from);
            at.exists().then_some((m, at))
        })
        .collect();
    out.sort_by(|a, b| a.0.ride_s.total_cmp(&b.0.ride_s));
    out
}

/// The photos of the ride `id` in `rides` (file, ride seconds, time), in
/// order: the training history and the Strava recordings (§2.8.1 ST3).
pub fn ride_photos(rides: &Path, id: &str) -> Vec<(String, f64, i64)> {
    ride_photo_files(rides, id)
        .into_iter()
        .map(|(m, _)| (m.file, m.ride_s, m.time_unix_s))
        .collect()
}

/// The folder to open for the ride's photos: where its first photo is
/// (the photos folder, or the ride's own folder for older rides).
pub fn ride_photo_folder(rides: &Path, id: &str) -> Option<PathBuf> {
    ride_photo_files(rides, id)
        .into_iter()
        .find_map(|(_, p)| p.parent().map(Path::to_path_buf))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ocw_world::photo::{CameraPose, Carrier, PoseKind, ViewScore};

    pub(crate) fn meta(
        n: usize,
        file: String,
        path: Option<String>,
        time: &MediaTime,
    ) -> PhotoMeta {
        let pose = CameraPose {
            kind: PoseKind::Side,
            carrier: Carrier::Person { crouch: false },
            feet: [10.0, 500.0, -4.0],
            eye: [10.0, 501.6, -4.0],
            look: [-1.0, 0.0, 0.0],
        };
        PhotoMeta {
            n,
            file,
            path,
            time_unix_s: time.unix_s,
            time: time.iso_local(),
            ride_s: 1500.0 + n as f64,
            ride: "2026-11-16_09-00-00".into(),
            world: "Test world".into(),
            route: "Loop".into(),
            lat: Some(47.5),
            lon: Some(8.25),
            height_m: 501.6,
            heading_deg: 270.0,
            pitch_deg: 0.0,
            score: ViewScore {
                total: 0.6,
                ..ViewScore::default()
            }
            .into(),
            camera: CameraMeta::of(&pose),
        }
    }

    const TIME: MediaTime = MediaTime {
        unix_s: 1_794_822_000,
        offset_s: 3600,
    };

    /// A photo in the photos folder (spec §2.11 Q3): the JPEG with EXIF
    /// time and GPS there, the ride's sidecar with the time, position,
    /// score, pose and the photo's path; the history and the Strava
    /// recordings find it through the sidecar (made-up place).
    #[test]
    fn a_photo_goes_to_the_photos_folder_and_the_ride_keeps_its_reference() {
        let base = std::env::temp_dir().join(format!("ocw-photo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let rides = base.join("rides");
        let folder = PhotoFolder {
            dir: base.join("Pictures/OCW"),
            label: "Pictures/OCW".into(),
        };
        let ride = "2026-11-16_09-00-00";
        let (path, went) = photo_target(Some(&folder), Some(&rides), ride, 2).unwrap();
        assert_eq!(
            path,
            base.join("Pictures/OCW/OCW-2026-11-16_09-00-00-photo-2.jpg")
        );
        assert_eq!(went, "Saved to Pictures/OCW");
        let m = meta(
            2,
            "OCW-2026-11-16_09-00-00-photo-2.jpg".into(),
            Some(path.to_string_lossy().into_owned()),
            &TIME,
        );
        let side_dir = photos_dir(&rides, ride);
        let rgb = vec![120u8; 64 * 36 * 3];
        let out = write_photo(
            &path,
            Some(&side_dir),
            &rgb,
            64,
            36,
            &m,
            &TIME,
            "Open Cycling World",
        )
        .unwrap();
        assert_eq!(out, path);
        let jpg = std::fs::read(&path).unwrap();
        assert!(jpg.starts_with(b"\xff\xd8"));
        // EXIF with the GPS block (the "N" / "E" refs and the date stamp).
        assert!(jpg.windows(4).any(|w| w == b"Exif"));
        assert!(jpg.windows(10).any(|w| w == b"2026:11:16"));
        // Only the photo in the Pictures folder, no sidecar there.
        let in_folder: Vec<_> = std::fs::read_dir(&folder.dir).unwrap().flatten().collect();
        assert_eq!(in_folder.len(), 1);
        let side: PhotoMeta =
            toml::from_str(&std::fs::read_to_string(side_dir.join("photo-2.toml")).unwrap())
                .unwrap();
        assert_eq!(side, m);
        assert_eq!(
            ride_photos(&rides, ride),
            vec![(
                "OCW-2026-11-16_09-00-00-photo-2.jpg".to_string(),
                1502.0,
                TIME.unix_s
            )]
        );
        assert_eq!(ride_photo_folder(&rides, ride), Some(folder.dir.clone()));
        // The next photo of the same name (another ride's start in the
        // same second) gets a free name.
        let (next, _) = photo_target(Some(&folder), Some(&rides), ride, 2).unwrap();
        assert_eq!(
            next,
            base.join("Pictures/OCW/OCW-2026-11-16_09-00-00-photo-2-2.jpg")
        );
        // A photo deleted from the Pictures folder is not listed.
        std::fs::remove_file(&path).unwrap();
        assert!(ride_photos(&rides, ride).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// Rides before v4.73: `photo-N.jpg` beside its sidecar (no path) –
    /// still listed for the history and the Strava recordings; a ride
    /// with both kinds lists both, in ride order.
    #[test]
    fn old_rides_keep_their_photos() {
        let base = std::env::temp_dir().join(format!("ocw-photo-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let rides = base.join("rides");
        let ride = "2026-11-16_09-00-00";
        let dir = photos_dir(&rides, ride);
        let rgb = vec![90u8; 32 * 18 * 3];
        let old = meta(1, "photo-1.jpg".into(), None, &TIME);
        write_photo(
            &dir.join("photo-1.jpg"),
            Some(&dir),
            &rgb,
            32,
            18,
            &old,
            &TIME,
            "OCW",
        )
        .unwrap();
        // The old sidecar has no path.
        let text = std::fs::read_to_string(dir.join("photo-1.toml")).unwrap();
        assert!(!text.contains("path"), "{text}");
        assert_eq!(
            ride_photos(&rides, ride),
            vec![("photo-1.jpg".to_string(), 1501.0, TIME.unix_s)]
        );
        assert_eq!(ride_photo_folder(&rides, ride), Some(dir.clone()));
        // A newer photo of that ride in the photos folder.
        let pics = base.join("Pictures/OCW/OCW-x-photo-2.jpg");
        let new = meta(
            2,
            "OCW-x-photo-2.jpg".into(),
            Some(pics.to_string_lossy().into()),
            &TIME,
        );
        write_photo(&pics, Some(&dir), &rgb, 32, 18, &new, &TIME, "OCW").unwrap();
        let files: Vec<String> = ride_photos(&rides, ride).into_iter().map(|f| f.0).collect();
        assert_eq!(files, ["photo-1.jpg", "OCW-x-photo-2.jpg"]);
        // No photos folder (no home folder): beside the sidecar, as before.
        let (p, went) = photo_target(None, Some(&rides), ride, 3).unwrap();
        assert_eq!(p, dir.join("photo-3.jpg"));
        assert_eq!(went, "Saved to the ride's folder");
        assert_eq!(photo_target(None, None, ride, 3), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// The photos folder per platform (spec §2.11 Q3): the Pictures
    /// folder's OCW – the XDG Pictures directory on Linux, `Pictures` in
    /// the profile on Windows and macOS – or `~/Pictures/OCW` without
    /// one; `photos.folder` takes a path or `~/…`.
    #[test]
    fn the_photos_folder_per_platform() {
        let home = PathBuf::from("/home/ada");
        assert_eq!(
            default_folder(Some(home.join("Bilder")), Some(home.clone())),
            Some(home.join("Bilder/OCW"))
        );
        assert_eq!(
            default_folder(None, Some(home.clone())),
            Some(home.join("Pictures/OCW"))
        );
        assert_eq!(default_folder(None, None), None);
        assert_eq!(
            resolve_folder("default", None, Some(home.clone())),
            Some(home.join("Pictures/OCW"))
        );
        assert_eq!(
            resolve_folder("", Some(home.join("Pics")), Some(home.clone())),
            Some(home.join("Pics/OCW"))
        );
        assert_eq!(
            resolve_folder("~/shots", None, Some(home.clone())),
            Some(home.join("shots"))
        );
        assert_eq!(
            resolve_folder("/srv/photos", None, Some(home.clone())),
            Some(PathBuf::from("/srv/photos"))
        );
        // This machine: always a folder named OCW under the Pictures
        // folder or the home folder.
        let f = photo_folder("default").expect("a home folder");
        assert!(f.dir.ends_with(PHOTOS_SUBFOLDER), "{}", f.dir.display());
        let home = dirs::home_dir().unwrap();
        // (Windows: the known folder Pictures, which may be moved, e.g.
        // into OneDrive; macOS: ~/Pictures; Linux: XDG_PICTURES_DIR.)
        #[cfg(target_os = "macos")]
        assert_eq!(f.dir, home.join("Pictures").join("OCW"));
        assert_eq!(
            f.dir,
            dirs::picture_dir()
                .unwrap_or_else(|| home.join("Pictures"))
                .join("OCW")
        );
        assert!(f.label.ends_with("/OCW"), "{}", f.label);
        assert!(!f.label.contains('\\'), "{}", f.label);
        let custom = photo_folder("~/ocw-test-shots").unwrap();
        assert_eq!(custom.dir, home.join("ocw-test-shots"));
        assert_eq!(custom.label, "~/ocw-test-shots");
    }
}

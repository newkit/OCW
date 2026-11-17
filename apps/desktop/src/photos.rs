//! The ride photographer's files on the desktop (spec §2.11): each photo
//! goes to `<data dir>/ocw/rides/<ride>/photos/photo-N.jpg` (`<ride>`: the
//! ride's FIT file stem, its UTC start) with its sidecar `photo-N.toml`
//! (time, position, view, the score and its parts, the camera pose) and
//! EXIF time and GPS position – no time stamp drawn in the picture
//! (§2.8.1 Q4). The photographer itself is `ocw_ui::photographer`; this
//! module gives it the files and the sound, and keeps it off in scripted
//! screenshot and capture runs (like the ride recording).

use std::path::{Path, PathBuf};

use bevy::prelude::*;
use ocw_audio::mixer::MixerMsg;
use ocw_client::photographer::{CameraMeta, PhotoMeta, Taken, photo_name};
use ocw_render::media::{GpsPosition, MediaTime, encode_jpeg, exif_tiff_gps};
use ocw_ui::photographer::{PhotographerHooks, PhotographerPlugin, RidePhotographer};

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
        if let Some(dir) = args.photo_chase_shots {
            app.insert_resource(ChaseShots {
                dir,
                n: 0,
                last: f64::NEG_INFINITY,
            })
            .add_systems(PostUpdate, chase_shots);
        }
    }
}

/// `--photo-chase-shots DIR` (test runs): the rider's view while the
/// photographer or the drone is in it.
#[derive(Resource)]
struct ChaseShots {
    dir: PathBuf,
    n: u32,
    last: f64,
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

/// Writes `photo-N.jpg` (with EXIF time and GPS) and `photo-N.toml` into
/// `dir`; returns the photo's path.
pub fn write_photo(
    dir: &Path,
    rgb: &[u8],
    w: u32,
    h: u32,
    meta: &PhotoMeta,
    time: &MediaTime,
    software: &str,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let gps = meta.lat.zip(meta.lon).map(|(lat, lon)| GpsPosition {
        lat,
        lon,
        height_m: meta.height_m,
    });
    let jpg = encode_jpeg(rgb, w, h, PHOTO_QUALITY, exif_tiff_gps(time, software, gps))?;
    let path = dir.join(&meta.file);
    let tmp = path.with_extension("jpg.part");
    std::fs::write(&tmp, jpg).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = toml::to_string_pretty(meta).map_err(|e| e.to_string())?;
    let side = path.with_extension("toml");
    std::fs::write(&side, text).map_err(|e| format!("{}: {e}", side.display()))?;
    Ok(path)
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

/// The photographer's hook: encodes and writes the photo off the main
/// thread.
fn save_photo(world: &mut World, t: &Taken, image: Image) {
    let Some(rides) = world
        .get_resource::<Recording>()
        .and_then(|r| r.dir.clone())
    else {
        warn!("ride photo: no rides folder");
        return;
    };
    let ride = ride_id(world);
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
    let meta = PhotoMeta {
        n: t.n,
        file: photo_name(t.n),
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
    let dir = photos_dir(&rides, &ride);
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
                &dir,
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
    if let Err(e) = spawned {
        error!("ride photo: {e}");
    }
}

/// The photos of the ride `id` in `rides` (file, ride seconds), from their
/// sidecars, in order.
pub fn ride_photos(rides: &Path, id: &str) -> Vec<(String, f64, i64)> {
    let dir = photos_dir(rides, id);
    let mut out: Vec<(String, f64, i64)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "toml"))
        .filter_map(|p| {
            let m: PhotoMeta = toml::from_str(&std::fs::read_to_string(&p).ok()?).ok()?;
            dir.join(&m.file)
                .exists()
                .then_some((m.file, m.ride_s, m.time_unix_s))
        })
        .collect();
    out.sort_by(|a, b| a.1.total_cmp(&b.1));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::photo::{CameraPose, Carrier, PoseKind, ViewScore};

    /// A photo file and its sidecar: the JPEG with EXIF time and GPS, the
    /// sidecar with the time, position, score and pose (made-up place).
    #[test]
    fn a_photo_is_saved_with_its_sidecar_and_exif() {
        let base = std::env::temp_dir().join(format!("ocw-photo-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let dir = photos_dir(&base, "2026-11-16_09-00-00");
        let pose = CameraPose {
            kind: PoseKind::Side,
            carrier: Carrier::Person { crouch: false },
            feet: [10.0, 500.0, -4.0],
            eye: [10.0, 501.6, -4.0],
            look: [-1.0, 0.0, 0.0],
        };
        let time = MediaTime {
            unix_s: 1_794_822_000,
            offset_s: 3600,
        };
        let meta = PhotoMeta {
            n: 2,
            file: photo_name(2),
            time_unix_s: time.unix_s,
            time: time.iso_local(),
            ride_s: 1500.0,
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
        };
        let rgb = vec![120u8; 64 * 36 * 3];
        let path = write_photo(&dir, &rgb, 64, 36, &meta, &time, "Open Cycling World").unwrap();
        assert_eq!(path, dir.join("photo-2.jpg"));
        let jpg = std::fs::read(&path).unwrap();
        assert!(jpg.starts_with(b"\xff\xd8"));
        // EXIF with the GPS block (the "N" / "E" refs and the date stamp).
        assert!(jpg.windows(4).any(|w| w == b"Exif"));
        assert!(jpg.windows(10).any(|w| w == b"2026:11:16"));
        let side: PhotoMeta =
            toml::from_str(&std::fs::read_to_string(dir.join("photo-2.toml")).unwrap()).unwrap();
        assert_eq!(side, meta);
        assert_eq!(
            ride_photos(&base, "2026-11-16_09-00-00"),
            vec![("photo-2.jpg".to_string(), 1500.0, time.unix_s)]
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}

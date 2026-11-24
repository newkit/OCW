//! WebXR in the browser app (spec §14 Phase 14, 14I W1; feature `webxr`):
//! the Bevy side of the immersive session that `web/ocw-xr.js` runs.
//!
//! The page steps the app from the session's frames (it runs the window's
//! `requestAnimationFrame` callbacks – winit's redraw, so Bevy's update –
//! inside each XR frame) and hands over the frame's poses first
//! (`window.ocw.xrFrame`). Here, while the session shows frames, two eye
//! cameras draw the scene side by side into the canvas – each with its
//! view's projection ([`EyeProjection`], from [`crate::xr`]) and pose,
//! where the layer's viewport says – and the chase camera rests; the page
//! then copies the canvas into the layer's framebuffer. When the session
//! ends the eyes go and the chase camera draws again.
//!
//! The rig (14I Q8, decided (a): `local-floor`): the reference space's
//! floor is the road under the bike, so the real floor is the game floor
//! and the headset's height is the rider's real eye height; the head where
//! the session started (the anchor; `recentre` takes a new one) is put on
//! the local rider's seated eye point – the saddle at the height the rider
//! profile gives (`AvatarRig::eye_point`) – and turned with the bike's
//! heading (never pitched or rolled, comfort). With `local` only (no
//! floor) the anchor's height goes to that eye point. Without a rider
//! (unicorn flight) the chase camera is the eye point. The own head is still
//! drawn (the eyes are inside it; its back faces are culled) and the HUD
//! lands in one eye (Bevy UI's default camera) – W2's bike computer and
//! hidden head replace both. W2 replaces this with the desktop VR
//! mode's eye cameras, anchor, recentring and bike computer
//! (`ocw-render::vr` and `ocw-xr` on branch `agent/vr-x1`).

use bevy::camera::visibility::VisibilitySystems;
use bevy::camera::{CameraProjection, Exposure, SubCameraView, Viewport};
use bevy::math::Vec3A;
use bevy::pbr::{AtmosphereSettings, DistanceFog};
use bevy::prelude::*;
use bevy::render::view::Msaa;
use bevy::transform::TransformSystems;
use bevy::window::PrimaryWindow;
use ocw_render::camera::FollowCamera;
use ocw_render::riders::{Avatar, AvatarRig};

use crate::xr::{self, Anchor, FrameStats, Space, XrEvent, XrFrame, XrState};

/// What the page sent since the last update (frames and session events;
/// wasm has one thread, the lock is never contended).
#[derive(Default)]
struct Inbox {
    frame: Option<Result<XrFrame, String>>,
    events: Vec<(String, String)>,
}

static INBOX: std::sync::Mutex<Inbox> = std::sync::Mutex::new(Inbox {
    frame: None,
    events: Vec::new(),
});

/// The state as JSON for the page (`window.ocw.xr()`).
static JSON: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// The page's frame data (`window.ocw.xrFrame(Float32Array)`).
pub fn page_frame(data: &[f32]) {
    INBOX.lock().expect("not poisoned").frame = Some(xr::parse_frame(data));
}

/// A session event from the page (`window.ocw.xrEvent(name, detail)`).
pub fn page_event(name: &str, detail: &str) {
    INBOX
        .lock()
        .expect("not poisoned")
        .events
        .push((name.to_owned(), detail.to_owned()));
}

/// `window.ocw.xr()`: the session's state, the frames, the eyes, timing.
pub fn page_json() -> String {
    JSON.lock().expect("not poisoned").clone()
}

/// The session as the app sees it.
#[derive(Resource, Default)]
pub struct WebXr {
    pub state: XrState,
    /// The latest frame (its views place the eyes).
    pub frame: Option<XrFrame>,
    /// Frames received / used to place the eyes this session.
    pub frames: u64,
    pub placed: u64,
    pub stats: FrameStats,
    /// The last refused frame or event (for the log and the test).
    pub last_error: Option<String>,
    /// Where the eyes were put last (world space; the test compares).
    pub eyes: Vec<(Vec3, Quat, [u32; 4])>,
    /// The page's last reason for not entering.
    pub failed: Option<String>,
    /// The reference space the browser gave (14I Q8: `local-floor`, else
    /// `local`).
    pub space: Space,
    /// The head where the session started (or was recentred): put on the
    /// rider's eye point. `None` until the first frame with a pose.
    pub anchor: Option<Anchor>,
    /// The local rider's saddle height and seated / standing eye heights
    /// above the road (m; for `window.ocw.xr()` and the headset run's
    /// report).
    pub saddle_m: Option<f32>,
    pub eye_seated_m: Option<f32>,
    pub eye_standing_m: Option<f32>,
    /// The headset's height above the floor in the last frame (m; with
    /// `local-floor` the rider's real eye height).
    pub head_m: Option<f32>,
    /// The eyes' middle above the road under the bike, and its distance
    /// across the ground from the rider's eye point (m; the test checks
    /// Q8 with them).
    pub eye_above_road_m: Option<f32>,
    pub eye_from_point_m: Option<f32>,
}

/// An eye camera (index of the view).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct XrEyeCamera(pub usize);

/// An eye's projection: WebXR's frustum, infinite reverse Z.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyeProjection {
    /// Left, right, down, up tangents ([`xr::frustum_tangents`]).
    pub tangents: [f32; 4],
    pub near: f32,
    /// For culling and the shadow cascades (the matrix has no far plane).
    pub far: f32,
}

impl Default for EyeProjection {
    fn default() -> Self {
        Self {
            tangents: [-1.0, 1.0, -1.0, 1.0],
            near: xr::EYE_NEAR_M,
            far: PerspectiveProjection::default().far,
        }
    }
}

impl CameraProjection for EyeProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        Mat4::from_cols_array(&xr::clip_from_view(self.tangents, self.near))
    }

    fn get_clip_from_view_for_sub(&self, _sub_view: &SubCameraView) -> Mat4 {
        self.get_clip_from_view()
    }

    fn update(&mut self, _width: f32, _height: f32) {}

    fn far(&self) -> f32 {
        self.far
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        xr::frustum_corners(self.tangents, z_near, z_far).map(Vec3A::from)
    }
}

pub struct WebXrPlugin;

impl Plugin for WebXrPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WebXr>()
            .add_systems(First, take_inbox)
            .add_systems(
                PostUpdate,
                // After the transforms (the bike's place this frame), before
                // the frusta and culling (the eyes' places this frame).
                (manage_eyes, place_eyes, copy_camera_look, publish)
                    .chain()
                    .after(TransformSystems::Propagate)
                    .before(VisibilitySystems::UpdateFrusta),
            );
    }
}

/// Applies the page's events and takes its latest frame.
fn take_inbox(mut xr: ResMut<WebXr>) {
    let (frame, events) = {
        let mut inbox = INBOX.lock().expect("not poisoned");
        (inbox.frame.take(), std::mem::take(&mut inbox.events))
    };
    for (name, detail) in events {
        // A new anchor at the next frame (the page's Recentre, later a
        // controller button); the session's state stays.
        if name == "recentre" {
            xr.anchor = None;
            info!("xr: recentred");
            continue;
        }
        let Some(event) = XrEvent::parse(&name, &detail) else {
            warn!("xr: unknown page event {name} {detail}");
            continue;
        };
        match xr.state.next(&event) {
            Ok(to) => {
                if to != xr.state {
                    info!("xr: {} → {} ({name} {detail})", xr.state.name(), to.name());
                }
                if to == XrState::Running && xr.state == XrState::Entering {
                    xr.stats.reset();
                    xr.frames = 0;
                    xr.placed = 0;
                }
                if event == XrEvent::Started {
                    xr.space = Space::parse(&detail);
                    xr.anchor = None;
                }
                if let XrEvent::Failed(why) = &event {
                    xr.failed = Some(why.clone());
                }
                if to == XrState::Ended {
                    xr.frame = None;
                    xr.anchor = None;
                }
                xr.state = to;
            }
            Err(why) => debug!("{why}"),
        }
    }
    match frame {
        Some(Ok(f)) if xr.state.in_session() => {
            xr.frames += 1;
            xr.stats.push(f.time_ms, f.previous_frame_ms);
            if xr.anchor.is_none() && !f.views.is_empty() {
                xr.anchor = Some(Anchor::from_viewer(f.viewer));
            }
            xr.head_m = (xr.space == Space::LocalFloor).then_some(f.viewer.position[1]);
            xr.frame = Some(f);
        }
        Some(Ok(_)) => {}
        Some(Err(why)) => {
            if xr.last_error.as_deref() != Some(&why) {
                warn!("{why}");
            }
            xr.last_error = Some(why);
        }
        None => {}
    }
}

/// Two eye cameras while the session shows frames; none (and the chase
/// camera drawing) otherwise.
fn manage_eyes(
    mut commands: Commands,
    xr: Res<WebXr>,
    eyes: Query<Entity, With<XrEyeCamera>>,
    mut follow: Query<&mut Camera, (With<FollowCamera>, Without<XrEyeCamera>)>,
) {
    let want = xr.state.in_headset() && xr.frame.is_some();
    if want && eyes.is_empty() {
        for i in 0..2 {
            commands.spawn((
                Camera3d::default(),
                Camera {
                    order: i as isize,
                    // The first eye clears the canvas, the second draws
                    // beside it (Bevy's split screen).
                    clear_color: if i == 0 {
                        ClearColorConfig::Default
                    } else {
                        ClearColorConfig::None
                    },
                    is_active: false,
                    ..default()
                },
                Projection::custom(EyeProjection::default()),
                Transform::default(),
                ocw_render::world::fog(),
                bevy::light::ShadowFilteringMethod::Gaussian,
                XrEyeCamera(i),
            ));
        }
        info!("xr: eye cameras on");
    } else if !want && !eyes.is_empty() {
        for e in &eyes {
            commands.entity(e).despawn();
        }
        info!("xr: eye cameras off, the flat view again");
    }
    // The chase camera rests while the eyes draw (the session hidden: it
    // stays off, nothing is shown).
    let follow_on = !xr.state.in_session();
    for mut cam in &mut follow {
        if cam.is_active != follow_on {
            cam.is_active = follow_on;
        }
    }
}

/// The rig's targets this frame: the rider's seated eye point in the
/// world, the road's height under the bike and the bike's heading, and the
/// rider's saddle and eye heights above the road (14I Q8).
struct RigTarget {
    eyes: [f32; 3],
    ground_y: f32,
    yaw: f32,
    heights: Option<[f32; 3]>,
}

#[allow(clippy::type_complexity)] // Bevy system queries
fn rig_target(
    avatars: &Query<(&Avatar, &AvatarRig, &GlobalTransform), Without<XrEyeCamera>>,
    follow: &Query<&GlobalTransform, (With<FollowCamera>, Without<XrEyeCamera>)>,
) -> Option<RigTarget> {
    if let Some((_, rig, g)) = avatars.iter().find(|(a, ..)| a.is_local) {
        let (_, r, t) = g.to_scale_rotation_translation();
        let seated = rig.eye_point(0.0);
        let standing = rig.eye_point(1.0);
        // Turned with the bike only (no pitch from the road: the eye point
        // in the bike's frame, the bike's heading).
        let yaw = xr::yaw_of(r.to_array());
        let eyes = t + Quat::from_array(xr::yaw_quat(yaw)) * seated;
        return Some(RigTarget {
            eyes: eyes.to_array(),
            ground_y: t.y,
            yaw,
            heights: Some([rig.saddle_height(), seated.y, standing.y]),
        });
    }
    let g = follow.single().ok()?;
    let (_, r, t) = g.to_scale_rotation_translation();
    Some(RigTarget {
        eyes: t.to_array(),
        ground_y: t.y - xr::SEATED_EYE_M,
        yaw: xr::yaw_of(r.to_array()),
        heights: None,
    })
}

/// Puts each eye where its view is, with its projection, in its part of
/// the canvas (after the transforms: the eyes are top-level entities, so
/// their global transform is written here too).
#[allow(clippy::type_complexity)] // a Bevy system query
fn place_eyes(
    mut xr: ResMut<WebXr>,
    avatars: Query<(&Avatar, &AvatarRig, &GlobalTransform), Without<XrEyeCamera>>,
    follow: Query<&GlobalTransform, (With<FollowCamera>, Without<XrEyeCamera>)>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut eyes: Query<(
        &XrEyeCamera,
        &mut Camera,
        &mut Projection,
        &mut Transform,
        &mut GlobalTransform,
    )>,
) {
    let Some(frame) = xr.frame.clone() else {
        return;
    };
    let Some(target) = rig_target(&avatars, &follow) else {
        return;
    };
    let anchor = xr.anchor.unwrap_or(Anchor::from_viewer(frame.viewer));
    let (origin, yaw) =
        xr::space_origin(xr.space, anchor, target.eyes, target.ground_y, target.yaw);
    if let Some([saddle, seated, standing]) = target.heights {
        xr.saddle_m = Some(saddle);
        xr.eye_seated_m = Some(seated);
        xr.eye_standing_m = Some(standing);
    }
    let size = window
        .single()
        .map_or([1, 1], |w| [w.physical_width(), w.physical_height()]);
    let mut placed = Vec::new();
    for (eye, mut cam, mut proj, mut t, mut g) in &mut eyes {
        let view = frame.views.get(eye.0);
        let rect = view.and_then(|v| v.canvas_rect(frame.framebuffer));
        let tangents = view.map(|v| xr::frustum_tangents(&v.projection));
        let (Some(view), Some(rect), Some(Ok(tangents))) = (view, rect, tangents) else {
            // No such view this frame (mono, or no pose): this eye rests.
            if cam.is_active {
                cam.is_active = false;
            }
            continue;
        };
        let pose = xr::rig_eye(origin, yaw, view.pose);
        *t = Transform {
            translation: Vec3::from_array(pose.position),
            rotation: Quat::from_array(pose.orientation),
            scale: Vec3::ONE,
        };
        *g = GlobalTransform::from(*t);
        let (pos, px) = xr::rect_pixels(rect, size);
        let viewport = Viewport {
            physical_position: UVec2::from_array(pos),
            physical_size: UVec2::from_array(px),
            ..default()
        };
        if cam.viewport.as_ref().is_none_or(|v| {
            v.physical_position != viewport.physical_position
                || v.physical_size != viewport.physical_size
        }) {
            cam.viewport = Some(viewport);
        }
        if !cam.is_active {
            cam.is_active = true;
        }
        if let Projection::Custom(c) = &mut *proj
            && let Some(ep) = c.get_mut::<EyeProjection>()
            && ep.tangents != tangents
        {
            ep.tangents = tangents;
        }
        placed.push((t.translation, t.rotation, [pos[0], pos[1], px[0], px[1]]));
    }
    if !placed.is_empty() {
        let mid = placed.iter().map(|(p, ..)| *p).sum::<Vec3>() / placed.len() as f32;
        xr.eye_above_road_m = Some(mid.y - target.ground_y);
        xr.eye_from_point_m =
            Some(Vec2::new(mid.x - target.eyes[0], mid.z - target.eyes[2]).length());
        xr.placed += 1;
        xr.eyes = placed;
    }
}

/// The eyes look like the chase camera: its fog, exposure, MSAA, far
/// plane and atmosphere (set by the sky, the weather and the quality
/// preset, which only know the chase camera) – as the desktop VR mode's
/// `copy_camera_look` (branch `agent/vr-x1`).
#[allow(clippy::type_complexity)] // a Bevy system query
fn copy_camera_look(
    mut commands: Commands,
    follow: Query<
        (
            Option<&DistanceFog>,
            Option<&Exposure>,
            Option<&Msaa>,
            Has<AtmosphereSettings>,
            &Projection,
        ),
        (With<FollowCamera>, Without<XrEyeCamera>),
    >,
    mut eyes: Query<
        (
            Entity,
            Option<&mut DistanceFog>,
            Option<&Exposure>,
            Option<&Msaa>,
            Has<AtmosphereSettings>,
            &mut Projection,
        ),
        With<XrEyeCamera>,
    >,
) {
    let Ok((fog, exposure, msaa, atmosphere, projection)) = follow.single() else {
        return;
    };
    let far = match projection {
        Projection::Perspective(p) => Some(p.far),
        _ => None,
    };
    for (e, eye_fog, eye_exposure, eye_msaa, eye_atmosphere, mut p) in &mut eyes {
        match (fog, eye_fog) {
            (Some(f), Some(mut ef)) => *ef = f.clone(),
            (Some(f), None) => {
                commands.entity(e).insert(f.clone());
            }
            (None, Some(_)) => {
                commands.entity(e).remove::<DistanceFog>();
            }
            (None, None) => {}
        }
        if let Some(x) = exposure
            && eye_exposure.is_none_or(|ex| ex.ev100 != x.ev100)
        {
            commands.entity(e).insert(*x);
        }
        if let Some(m) = msaa
            && eye_msaa != Some(m)
        {
            commands.entity(e).insert(*m);
        }
        if atmosphere && !eye_atmosphere {
            commands.entity(e).insert(AtmosphereSettings::default());
        } else if !atmosphere && eye_atmosphere {
            commands.entity(e).remove::<AtmosphereSettings>();
        }
        if let (Some(far), Projection::Custom(c)) = (far, &mut *p)
            && let Some(ep) = c.get_mut::<EyeProjection>()
            && ep.far != far
        {
            ep.far = far;
        }
    }
}

/// `window.ocw.xr()`'s JSON.
fn publish(xr: Res<WebXr>) {
    let r = |v: Option<f32>| v.map(|x| (x * 100.0).round() / 100.0);
    let eyes: Vec<_> = xr
        .eyes
        .iter()
        .map(|(p, q, vp)| {
            let (yaw, pitch, roll) = q.to_euler(EulerRot::YXZ);
            serde_json::json!({
                "pos": [p.x, p.y, p.z],
                "yaw_deg": yaw.to_degrees(),
                "pitch_deg": pitch.to_degrees(),
                "roll_deg": roll.to_degrees(),
                "viewport": vp,
            })
        })
        .collect();
    let s = &xr.stats;
    let json = serde_json::json!({
        "state": xr.state.name(),
        "in_headset": xr.state.in_headset(),
        "frames": xr.frames,
        "placed": xr.placed,
        "views": xr.frame.as_ref().map_or(0, |f| f.views.len()),
        "framebuffer": xr.frame.as_ref().map(|f| f.framebuffer),
        "viewer": xr.frame.as_ref().map(|f| f.viewer.position),
        "space": xr.space.name(),
        "anchored": xr.anchor.is_some(),
        "saddle_m": r(xr.saddle_m),
        "eye_seated_m": r(xr.eye_seated_m),
        "eye_standing_m": r(xr.eye_standing_m),
        "head_m": r(xr.head_m),
        "eye_above_road_m": xr.eye_above_road_m,
        "eye_from_point_m": xr.eye_from_point_m,
        "eyes": eyes,
        "fps": r(s.fps()),
        "gap_ms_p50": r(s.gap_ms(0.5)),
        "gap_ms_p95": r(s.gap_ms(0.95)),
        "work_ms_p50": r(s.work_ms(0.5)),
        "work_ms_p95": r(s.work_ms(0.95)),
        "longest_gap_ms": (s.longest_gap_ms * 100.0).round() / 100.0,
        "late": s.late(),
        "error": xr.last_error,
        "failed": xr.failed,
    });
    *JSON.lock().expect("not poisoned") = json.to_string();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(views: usize) -> Vec<f32> {
        let mut d = vec![xr::FRAME_LAYOUT, views as f32, 200.0, 100.0, 16.0, 5.0];
        d.extend_from_slice(&[0.0, 1.6, 0.0, 0.0, 0.0, 0.0, 1.0]);
        for i in 0..views {
            d.push(i as f32 + 1.0);
            d.extend_from_slice(&[100.0 * i as f32, 0.0, 100.0, 100.0]);
            // glFrustum(-0.1, 0.1, -0.1, 0.1, 0.1, 100)
            let (n, f) = (0.1f32, 100.0f32);
            d.extend_from_slice(&[
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                1.0,
                0.0,
                0.0,
                0.0,
                0.0,
                -(f + n) / (f - n),
                -1.0,
                0.0,
                0.0,
                -2.0 * f * n / (f - n),
                0.0,
            ]);
            d.extend_from_slice(&[-0.03 + 0.06 * i as f32, 1.6, 0.0, 0.0, 0.0, 0.0, 1.0]);
        }
        d
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .add_plugins(WebXrPlugin);
        // A 1280 × 720 canvas.
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app.world_mut().spawn((
            Camera3d::default(),
            Transform::from_xyz(5.0, 3.0, 9.0),
            FollowCamera,
        ));
        app
    }

    fn eyes(app: &mut App) -> Vec<(XrEyeCamera, Camera, GlobalTransform)> {
        let mut q = app
            .world_mut()
            .query::<(&XrEyeCamera, &Camera, &GlobalTransform)>();
        let mut v: Vec<_> = q
            .iter(app.world())
            .map(|(e, c, g)| (*e, c.clone(), *g))
            .collect();
        v.sort_by_key(|(e, ..)| e.0);
        v
    }

    fn follow_active(app: &mut App) -> bool {
        let mut q = app
            .world_mut()
            .query_filtered::<&Camera, With<FollowCamera>>();
        q.single(app.world()).unwrap().is_active
    }

    /// The page's session in the app: entering spawns the eyes at the
    /// views (the chase camera resting), ending brings the flat view back.
    #[test]
    fn a_session_puts_two_eyes_at_the_views_and_ending_removes_them() {
        let mut app = app();
        page_event("supported", "true");
        app.update();
        assert_eq!(app.world().resource::<WebXr>().state, XrState::Offered);
        page_event("enter", "");
        page_event("started", "");
        page_frame(&frame(2));
        // Spawned in one update, placed (with commands applied) in the next.
        app.update();
        page_frame(&frame(2));
        app.update();
        let xr = app.world().resource::<WebXr>();
        assert_eq!(xr.state, XrState::Running);
        assert_eq!(xr.frames, 2);
        assert!(xr.placed >= 1);
        let e = eyes(&mut app);
        assert_eq!(e.len(), 2);
        assert!(!follow_active(&mut app));
        // No rider: the chase camera is the eye point, the floor
        // SEATED_EYE_M below it (local-floor); the head 1.6 m above the
        // floor, the eyes 6 cm apart around the camera's place.
        let (l, r) = (e[0].2.translation(), e[1].2.translation());
        assert!((l.y - (3.0 - xr::SEATED_EYE_M + 1.6)).abs() < 1e-4, "{l}");
        assert!(
            ((l + r) * 0.5).abs_diff_eq(Vec3::new(5.0, l.y, 9.0), 1e-4),
            "{l} {r}"
        );
        assert!(((r - l).length() - 0.06).abs() < 1e-4, "{l} {r}");
        // Side by side in the 1280 × 720 window (the frame's framebuffer
        // is 200 × 100 with two 100 px views).
        let vp = |c: &Camera| c.viewport.clone().unwrap();
        assert_eq!(vp(&e[0].1).physical_position, UVec2::new(0, 0));
        assert_eq!(vp(&e[0].1).physical_size, UVec2::new(640, 720));
        assert_eq!(vp(&e[1].1).physical_position, UVec2::new(640, 0));
        assert!(e.iter().all(|(_, c, _)| c.is_active));
        assert!(matches!(e[1].1.clear_color, ClearColorConfig::None));
        page_event("ended", "");
        app.update();
        app.update();
        assert!(eyes(&mut app).is_empty());
        assert!(follow_active(&mut app));
        assert!(
            page_json().contains("\"state\":\"ended\""),
            "{}",
            page_json()
        );
    }
}

//! WebXR in the browser app (spec §14 Phase 14, 14I W1 – W3; feature
//! `webxr`): the Bevy side of the immersive session that `web/ocw-xr.js`
//! runs.
//!
//! The page steps the app from the session's frames (it runs the window's
//! `requestAnimationFrame` callbacks – winit's redraw, so Bevy's update –
//! inside each XR frame) and hands over the frame's poses and the
//! controllers first (`window.ocw.xrFrame`, `window.ocw.xrPads`). Here,
//! while the session shows frames, two eye cameras draw the scene side by
//! side into the canvas – each with its view's projection
//! (`ocw_render::vr::EyeProjection`, shared with the desktop's VR) and
//! pose, where the layer's viewport says – and the chase camera rests; the
//! page then copies the canvas into the layer's framebuffer. When the
//! session ends the eyes go and the chase camera draws again.
//!
//! The rig (14I Q8, decided (a): `local-floor`): the reference space's
//! floor is the road under the bike, so the real floor is the game floor
//! and the headset's height is the rider's real eye height; the head where
//! the session started (the anchor; `recentre` takes a new one) is put on
//! the local rider's seated eye point – the saddle at the height the rider
//! profile gives (`AvatarRig::eye_point`) – and turned with the bike's
//! heading (never pitched or rolled, comfort). With `local` only (no
//! floor) the anchor's height goes to that eye point. Without a rider
//! (unicorn flight) the chase camera is the eye point.
//!
//! W2 (the desktop VR work of branch `agent/vr-x1` reused,
//! `ocw-render::vr`, `ocw-xr`):
//! - **the bike computer on the bars** instead of the flat HUD: the eyes
//!   see the render layer `ocw_render::vr::VR_LAYER`, where the GPS unit
//!   sits on the local bike's out-front mount (power, speed, cadence, heart
//!   rate, grade, distance, time, the next turn, the profile, the bridge's
//!   state and the frame time);
//! - **no flat UI over the eyes**: during the session the default UI
//!   camera is ours ([`XrUiCamera`]), drawing into a picture, not the
//!   canvas – so the HUD is never drawn into an eye; while the in-game menu
//!   is open the picture is a panel in the world ahead of the rider
//!   ([`XrMenuPanel`]), otherwise the camera rests;
//! - the own head stays hidden in the session (v5.77);
//! - **the Touch controllers** (`oculus-touch-v2` WebXR gamepads, spec 14A)
//!   as the desktop VR mode's gamepad (`xr::PadReader` → `ocw_xr::
//!   controllers` → `ocw_render::vr::controllers`): the right stick rides,
//!   the left stick's sideways flicks are the junction exits (Delete /
//!   Page Down), its press the menu, B back, A confirm / autodrive, the
//!   grips recentre; the controllers are drawn at their grips.
//!
//! W3: on a Quest 1 (`ocw_xr::headset::from_user_agent`) `auto` quality is
//! the `quest1` level for the session ([`xr_quality`]; the page asks
//! `window.ocw.xrLayer` for the layer's render scale and foveation before
//! it creates the layer), and the frame time is on the bike computer.

use bevy::camera::visibility::{RenderLayers, VisibilitySystems};
use bevy::camera::{RenderTarget, Viewport};
use bevy::light::SimulationLightSystems;
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::transform::TransformSystems;
use bevy::window::PrimaryWindow;
use ocw_client::quality::{DeviceInfo, GpuApi, QualitySetting, XrHeadset, XrLayer, resolve_xr};
use ocw_render::camera::{FollowCamera, RiderHead};
use ocw_render::menu::MenuView;
use ocw_render::riders::{Avatar, AvatarRig};
use ocw_render::vr::{EyeProjection, VR_LAYER, VrSession};

use crate::xr::{self, Anchor, FrameStats, PadReader, Space, XrEvent, XrFrame, XrPad, XrState};

/// What the page sent since the last update (frames, controllers and
/// session events; wasm has one thread, the lock is never contended).
#[derive(Default)]
struct Inbox {
    frame: Option<Result<XrFrame, String>>,
    pads: Option<Result<Vec<XrPad>, String>>,
    events: Vec<(String, String)>,
}

static INBOX: std::sync::Mutex<Inbox> = std::sync::Mutex::new(Inbox {
    frame: None,
    pads: None,
    events: Vec::new(),
});

/// The state as JSON for the page (`window.ocw.xr()`).
static JSON: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// What `auto` quality decides by, for the page's layer options before a
/// session exists (`window.ocw.xrLayer`).
static QUALITY: std::sync::Mutex<Option<(QualitySetting, GpuApi, DeviceInfo)>> =
    std::sync::Mutex::new(None);

/// The page's frame data (`window.ocw.xrFrame(Float32Array)`).
pub fn page_frame(data: &[f32]) {
    INBOX.lock().expect("not poisoned").frame = Some(xr::parse_frame(data));
}

/// The page's controllers (`window.ocw.xrPads(Float32Array)`, before each
/// frame's `xrFrame`).
pub fn page_pads(data: &[f32]) {
    INBOX.lock().expect("not poisoned").pads = Some(xr::parse_pads(data));
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

/// The headset a user agent names, as the quality sees it.
pub fn headset_of(user_agent: &str) -> XrHeadset {
    if ocw_xr::headset::from_user_agent(user_agent).id == ocw_xr::headset::QUEST_1.id {
        XrHeadset::Quest1
    } else {
        XrHeadset::Other
    }
}

/// `window.ocw.xrLayer(userAgent)`: the level a session on this headset
/// would run and the layer's options for it (14I W3), as JSON:
/// `{"level": "quest1", "scale": 0.8, "foveation": 1.0, "headset": …}`.
pub fn page_layer(user_agent: &str) -> String {
    let (setting, api, device) = QUALITY.lock().expect("not poisoned").clone().unwrap_or((
        QualitySetting::Auto,
        GpuApi::WebGl2,
        DeviceInfo::default(),
    ));
    let headset = headset_of(user_agent);
    let (level, reason) = resolve_xr(setting, api, &device, Some(headset));
    let layer = XrLayer::of(level);
    serde_json::json!({
        "level": level.name(),
        "reason": reason,
        "scale": layer.scale,
        "foveation": layer.foveation,
        "headset": ocw_xr::headset::from_user_agent(user_agent).id,
    })
    .to_string()
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
    /// The browser's user agent (the page's `device` event): which
    /// headset (14I W3).
    pub user_agent: String,
    /// The controllers of the last frame.
    pub pads: Vec<XrPad>,
    pub pad_reader: PadReader,
    /// Recentres this session (the squeeze, the grips, the headset's own).
    pub recentres: u32,
}

/// An eye camera (index of the view).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct XrEyeCamera(pub usize);

/// The default UI camera during the session: the flat UI (the HUD, the
/// menu) draws into its picture, never into the canvas the eyes share; on
/// only while the menu is open, when the picture is the [`XrMenuPanel`].
#[derive(Component, Debug)]
pub struct XrUiCamera;

/// The menu's panel in the world (the UI camera's picture).
#[derive(Component, Debug)]
pub struct XrMenuPanel;

/// A part of a controller drawn at its grip: the hand and the part's place
/// in the grip's frame.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct XrController {
    pub hand: xr::Hand,
    pub offset: Transform,
}

pub struct WebXrPlugin;

impl Plugin for WebXrPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ocw_render::vr::VrPlugin>() {
            app.add_plugins(ocw_render::vr::VrPlugin);
        }
        app.init_resource::<WebXr>()
            .init_resource::<crate::XrCanvasScale>()
            .add_systems(First, take_inbox)
            .add_systems(Update, (vr_session, xr_canvas, xr_quality).chain())
            .add_systems(
                PostUpdate,
                // After the transforms (the bike's place this frame), before
                // the frusta and culling (the eyes' places this frame).
                (
                    manage_eyes,
                    place_eyes,
                    place_controllers,
                    // Shared with the desktop's VR (`ocw_render::vr`).
                    ocw_render::vr::copy_camera_look::<XrEyeCamera>,
                    publish,
                )
                    .chain()
                    .after(TransformSystems::Propagate)
                    .before(VisibilitySystems::UpdateFrusta)
                    // Cameras switched on or off here must be so before the
                    // shadow cascades are built: they are built only for
                    // active cameras, and the renderer unwraps a camera's
                    // cascades (bevy_pbr light.rs: the chase camera switched
                    // back on after the session panicked there).
                    .before(SimulationLightSystems::UpdateDirectionalLightCascades),
            );
    }
}

/// Applies the page's events and takes its latest frame and controllers.
fn take_inbox(mut xr: ResMut<WebXr>, menu: Option<Res<MenuView>>) {
    let (frame, pads, events) = {
        let mut inbox = INBOX.lock().expect("not poisoned");
        (
            inbox.frame.take(),
            inbox.pads.take(),
            std::mem::take(&mut inbox.events),
        )
    };
    let menu_open = menu.is_some_and(|m| m.open);
    for (name, detail) in events {
        // A new anchor at the next frame (the page's Recentre, the
        // controller's squeeze, the headset's own); the session's state
        // stays. With the menu open the grips switch its tabs: no recentre.
        if name == "recentre" {
            if detail == "squeeze" && menu_open {
                continue;
            }
            xr.anchor = None;
            xr.recentres += 1;
            info!("xr: recentred ({detail})");
            continue;
        }
        // The browser's user agent (the headset, 14I W3).
        if name == "device" {
            xr.user_agent = detail;
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
                    xr.recentres = 0;
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
                    xr.pads.clear();
                }
                xr.state = to;
            }
            Err(why) => debug!("{why}"),
        }
    }
    match pads {
        Some(Ok(p)) if xr.state.in_session() => xr.pads = p,
        Some(Ok(_)) => {}
        Some(Err(why)) => {
            if xr.last_error.as_deref() != Some(&why) {
                warn!("{why}");
            }
            xr.last_error = Some(why);
        }
        None => {}
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

/// The renderer's VR session (the bike computer, the controllers'
/// gamepad): shown, the controls hint, the status row, the controllers;
/// the controllers' recentre gesture taken back.
fn vr_session(
    time: Res<Time<Real>>,
    mut xr: ResMut<WebXr>,
    mut vr: ResMut<VrSession>,
    menu: Option<Res<MenuView>>,
    mut next_line_s: Local<f32>,
) {
    let shown = xr.state.in_headset() && xr.frame.is_some();
    if vr.shown != shown {
        vr.shown = shown;
    }
    if vr.controls_hint != xr::CONTROLS_HINT {
        vr.controls_hint = xr::CONTROLS_HINT.into();
    }
    vr.exits_on_left_stick = true;
    let menu_open = menu.is_some_and(|m| m.open);
    let xr = &mut *xr;
    vr.controllers = (xr.state.in_headset() && !xr.pads.is_empty())
        .then(|| xr.pad_reader.state(&xr.pads, menu_open));
    if vr.recentre_now {
        vr.recentre_now = false;
        if !menu_open {
            xr.anchor = None;
            xr.recentres += 1;
            info!("xr: recentred (both grips)");
        }
    }
    // The status row twice a second (a new text lays the screen out
    // again).
    let now = time.elapsed_secs();
    if shown && now >= *next_line_s {
        *next_line_s = now + 0.5;
        let link = xr::bridge_line(&crate::bridge::json());
        let frame = xr::frame_line(&xr.stats);
        if vr.link != link {
            vr.link = link;
        }
        if vr.frame != frame {
            vr.frame = frame;
        }
    }
}

/// 14I W3: during the session the canvas has the layer framebuffer's
/// pixels (`crate::XrCanvasScale`; the emulator's canvas is the layer, so
/// there nothing changes); the flat view's scale again after it.
fn xr_canvas(
    xr: Res<WebXr>,
    window: Query<&Window, With<PrimaryWindow>>,
    mut scale: ResMut<crate::XrCanvasScale>,
) {
    let want = match (&xr.frame, window.single()) {
        (Some(f), Ok(w)) if xr.state.in_session() => {
            crate::xr_canvas_scale(w.resolution.width(), f.framebuffer[0])
        }
        _ => None,
    };
    let near = |a: Option<f32>, b: Option<f32>| match (a, b) {
        (Some(a), Some(b)) => (a - b).abs() <= 0.01 * b,
        (a, b) => a == b,
    };
    if !near(scale.0, want) {
        if let Some(s) = want {
            info!("xr: the canvas at {s:.2} px per CSS px (the layer's framebuffer)");
        }
        scale.0 = want;
    }
}

/// 14I W3: in a session on a Quest 1 `auto` quality is the `quest1`
/// level; after the session the flat view's level again. Also keeps what
/// `window.ocw.xrLayer` decides by.
fn xr_quality(world: &mut World) {
    let (in_session, ua) = {
        let xr = world.resource::<WebXr>();
        (xr.state.in_session(), xr.user_agent.clone())
    };
    let Some(mut ctx) = world
        .get_resource::<ocw_ui::quality::QualityContext>()
        .cloned()
    else {
        return;
    };
    let setting = world
        .get_resource::<ocw_ui::quality::QualityChoice>()
        .map_or(QualitySetting::Auto, |c| c.setting);
    {
        let mut q = QUALITY.lock().expect("not poisoned");
        let want = (setting, ctx.api, ctx.device.clone());
        if q.as_ref() != Some(&want) {
            *q = Some(want);
        }
    }
    let want = in_session.then(|| headset_of(&ua));
    if ctx.xr == want {
        return;
    }
    ctx.xr = want;
    world.insert_resource(ctx);
    ocw_ui::quality::set_quality(world, setting);
    let level = world
        .get_resource::<ocw_ui::quality::QualityChoice>()
        .map(|c| (c.level, c.reason));
    info!(
        "xr: quality {} for the {}",
        level.map_or("?".into(), |(l, r)| format!("{l} ({r})")),
        if in_session { "session" } else { "flat view" }
    );
}

/// Two eye cameras while the session shows frames; none (and the chase
/// camera drawing) otherwise. During the session the flat UI's camera, the
/// menu's panel and the controllers are there too.
#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a Bevy system
fn manage_eyes(
    mut commands: Commands,
    xr: Res<WebXr>,
    menu: Option<Res<MenuView>>,
    eyes: Query<Entity, With<XrEyeCamera>>,
    ui_cams: Query<(Entity, &mut Camera), (With<XrUiCamera>, Without<XrEyeCamera>)>,
    panels: Query<(Entity, &mut Visibility), (With<XrMenuPanel>, Without<RiderHead>)>,
    controllers: Query<Entity, With<XrController>>,
    mut follow: Query<
        &mut Camera,
        (
            With<FollowCamera>,
            Without<XrEyeCamera>,
            Without<XrUiCamera>,
        ),
    >,
    mut heads: Query<(&RiderHead, &mut Visibility), Without<XrMenuPanel>>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let want = xr.state.in_headset() && xr.frame.is_some();
    // The own head is not drawn while the eyes are in it (the helmet's
    // straps and the face from inside): hidden during the session, shown
    // again after.
    let head = if xr.state.in_session() {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for (h, mut v) in &mut heads {
        if h.local && *v != head {
            *v = head;
        }
    }
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
                Projection::custom(EyeProjection::from_tangents(
                    [-1.0, 1.0, -1.0, 1.0],
                    xr::EYE_NEAR_M,
                )),
                Transform::default(),
                ocw_render::world::fog(),
                bevy::light::ShadowFilteringMethod::Gaussian,
                // The world and the VR-only things (the bike computer, the
                // menu's panel, the controllers).
                ocw_render::vr::eye_layers(),
                XrEyeCamera(i),
            ));
        }
        spawn_session_things(&mut commands, &mut images, &mut meshes, &mut materials);
        info!("xr: eye cameras on, the bike computer and the controllers; no flat UI");
    } else if !want && !eyes.is_empty() {
        for e in &eyes {
            commands.entity(e).despawn();
        }
        for (e, _) in &ui_cams {
            commands.entity(e).despawn();
        }
        for (e, _) in &panels {
            commands.entity(e).despawn();
        }
        for e in &controllers {
            commands.entity(e).despawn();
        }
        info!("xr: eye cameras off, the flat view again");
    }
    // The flat UI draws (into the panel's picture) only while the menu is
    // open; the panel shows it then.
    let menu_open = menu.is_some_and(|m| m.open);
    for (_, mut cam) in ui_cams {
        if cam.is_active != menu_open {
            cam.is_active = menu_open;
        }
    }
    let shown = if menu_open {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for (_, mut v) in panels {
        if *v != shown {
            *v = shown;
        }
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

/// The flat UI's camera (into a picture), the menu's panel showing it, and
/// the two controllers – all on the eyes' layer only.
fn spawn_session_things(
    commands: &mut Commands,
    images: &mut Assets<Image>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    let [w, h] = xr::MENU_PANEL_PX;
    let picture = images.add(Image::new_target_texture(
        w,
        h,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    commands.spawn((
        Camera2d,
        Camera {
            order: 20,
            is_active: false,
            clear_color: ClearColorConfig::Custom(Color::srgba(0.02, 0.03, 0.04, 0.92)),
            ..default()
        },
        RenderTarget::Image(picture.clone().into()),
        // Every UI node without its own camera is drawn by this one: the
        // HUD never lands in an eye (14I W2).
        bevy::ui::IsDefaultUiCamera,
        XrUiCamera,
    ));
    let pw = xr::MENU_PANEL_WIDTH_M;
    let ph = pw * h as f32 / w as f32;
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(pw, ph))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color_texture: Some(picture),
            unlit: true,
            alpha_mode: AlphaMode::Blend,
            ..default()
        })),
        Transform::default(),
        Visibility::Hidden,
        RenderLayers::layer(VR_LAYER),
        bevy::light::NotShadowCaster,
        XrMenuPanel,
    ));
    // A Touch controller, simply: the handle and the tracking ring.
    let body = materials.add(StandardMaterial {
        base_color: Color::srgb(0.09, 0.09, 0.1),
        perceptual_roughness: 0.45,
        ..default()
    });
    let ring = materials.add(StandardMaterial {
        base_color: Color::srgb(0.75, 0.77, 0.8),
        perceptual_roughness: 0.35,
        ..default()
    });
    let handle = meshes.add(Capsule3d::new(0.019, 0.075));
    let torus = meshes.add(Torus::new(0.032, 0.045));
    // Each part a top-level entity placed with its global transform in
    // the eyes' frame (a child would follow one frame late).
    for hand in [xr::Hand::Left, xr::Hand::Right] {
        let side = if hand == xr::Hand::Left { 1.0 } else { -1.0 };
        let parts = [
            // The grip's −Z runs along the handle (WebXR's grip space).
            (
                handle.clone(),
                body.clone(),
                Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
            ),
            (
                torus.clone(),
                ring.clone(),
                Transform::from_xyz(side * 0.012, 0.035, -0.055)
                    .with_rotation(Quat::from_rotation_x(0.6)),
            ),
        ];
        for (mesh, material, offset) in parts {
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
                Visibility::Hidden,
                RenderLayers::layer(VR_LAYER),
                bevy::light::NotShadowCaster,
                XrController { hand, offset },
            ));
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

/// The reference space's origin in the world and its heading this frame
/// (the anchor on the rider's eye point, 14I Q8), and the rig's target.
fn rig_origin(xr: &WebXr, target: &RigTarget, frame: &XrFrame) -> ([f32; 3], f32) {
    let anchor = xr.anchor.unwrap_or(Anchor::from_viewer(frame.viewer));
    xr::space_origin(xr.space, anchor, target.eyes, target.ground_y, target.yaw)
}

fn to_transform(p: xr::Pose) -> Transform {
    Transform {
        translation: Vec3::from_array(p.position),
        rotation: Quat::from_array(p.orientation),
        scale: Vec3::ONE,
    }
}

/// Puts each eye where its view is, with its projection, in its part of
/// the canvas (after the transforms: the eyes are top-level entities, so
/// their global transform is written here too); the menu's panel ahead of
/// the rider's eye point.
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
    mut panels: Query<
        (&mut Transform, &mut GlobalTransform),
        (
            With<XrMenuPanel>,
            Without<XrEyeCamera>,
            Without<Avatar>,
            Without<FollowCamera>,
        ),
    >,
) {
    let Some(frame) = xr.frame.clone() else {
        return;
    };
    let Some(target) = rig_target(&avatars, &follow) else {
        return;
    };
    let (origin, yaw) = rig_origin(&xr, &target, &frame);
    if let Some([saddle, seated, standing]) = target.heights {
        xr.saddle_m = Some(saddle);
        xr.eye_seated_m = Some(seated);
        xr.eye_standing_m = Some(standing);
    }
    for (mut t, mut g) in &mut panels {
        *t = to_transform(xr::menu_panel_pose(target.eyes, target.yaw));
        *g = GlobalTransform::from(*t);
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
        *t = to_transform(xr::rig_eye(origin, yaw, view.pose));
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
            && let fov = ocw_render::vr::fov_of_tangents(tangents)
            && ep.fov != fov
        {
            ep.fov = fov;
        }
        placed.push((
            eye.0,
            (t.translation, t.rotation, [pos[0], pos[1], px[0], px[1]]),
        ));
    }
    // In the views' order (the query's order is not).
    placed.sort_by_key(|(i, _)| *i);
    let placed: Vec<_> = placed.into_iter().map(|(_, e)| e).collect();
    if !placed.is_empty() {
        let mid = placed.iter().map(|(p, ..)| *p).sum::<Vec3>() / placed.len() as f32;
        xr.eye_above_road_m = Some(mid.y - target.ground_y);
        xr.eye_from_point_m =
            Some(Vec2::new(mid.x - target.eyes[0], mid.z - target.eyes[2]).length());
        xr.placed += 1;
        xr.eyes = placed;
    }
}

/// The controllers at their grips (in the rig like the eyes), hidden when
/// not tracked.
#[allow(clippy::type_complexity)] // a Bevy system query
fn place_controllers(
    xr: Res<WebXr>,
    avatars: Query<(&Avatar, &AvatarRig, &GlobalTransform), Without<XrEyeCamera>>,
    follow: Query<&GlobalTransform, (With<FollowCamera>, Without<XrEyeCamera>)>,
    mut controllers: Query<
        (
            &XrController,
            &mut Transform,
            &mut GlobalTransform,
            &mut Visibility,
        ),
        (Without<XrEyeCamera>, Without<FollowCamera>, Without<Avatar>),
    >,
) {
    if controllers.is_empty() {
        return;
    }
    let rig = xr
        .frame
        .as_ref()
        .zip(rig_target(&avatars, &follow))
        .map(|(f, target)| rig_origin(&xr, &target, f));
    for (c, mut t, mut gt, mut v) in &mut controllers {
        let grip = xr
            .pads
            .iter()
            .find(|p| p.hand == c.hand)
            .and_then(|p| p.grip);
        let want = match (rig, grip) {
            (Some((origin, yaw)), Some(g)) => {
                *t = to_transform(xr::rig_eye(origin, yaw, g)).mul_transform(c.offset);
                *gt = GlobalTransform::from(*t);
                Visibility::Inherited
            }
            _ => Visibility::Hidden,
        };
        if *v != want {
            *v = want;
        }
    }
}

/// `window.ocw.xr()`'s JSON.
#[allow(clippy::type_complexity)] // a Bevy system query
fn publish(
    xr: Res<WebXr>,
    vr: Res<VrSession>,
    quality: Option<Res<ocw_ui::quality::QualityChoice>>,
    ui_cams: Query<(&Camera, Option<&RenderTarget>), With<XrUiCamera>>,
    default_ui: Query<Entity, With<bevy::ui::IsDefaultUiCamera>>,
    computer: Query<
        (&GlobalTransform, &InheritedVisibility),
        With<ocw_render::vr::computer::ComputerRoot>,
    >,
    controllers: Query<(&XrController, &InheritedVisibility)>,
) {
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
    // No flat UI over the eyes (14I W2): the only default UI camera is
    // ours, drawing into a picture.
    let ui_into_picture = ui_cams
        .iter()
        .all(|(_, t)| matches!(t, Some(RenderTarget::Image(_))));
    let flat_ui =
        !(ui_cams.iter().count() == 1 && default_ui.iter().count() == 1 && ui_into_picture);
    let computer = computer.single().ok().map(|(g, v)| {
        let p = g.translation();
        serde_json::json!({"pos": [p.x, p.y, p.z], "visible": v.get()})
    });
    let controllers_shown = controllers.iter().filter(|(_, v)| v.get()).count();
    let pads: Vec<_> = xr
        .pads
        .iter()
        .map(|p| {
            serde_json::json!({
                "hand": format!("{:?}", p.hand).to_lowercase(),
                "tracked": p.grip.is_some(),
                "buttons": p.buttons,
                "stick": [p.axes[2], p.axes[3]],
            })
        })
        .collect();
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
        "recentres": xr.recentres,
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
        "quality": quality.map(|q| serde_json::json!({"level": q.level.name(), "reason": q.reason})),
        "headset": ocw_xr::headset::from_user_agent(&xr.user_agent).id,
        "flat_ui": flat_ui && xr.state.in_headset(),
        "menu_panel": ui_cams.iter().any(|(c, _)| c.is_active),
        "computer": computer,
        "computer_screen": {"shown": vr.shown, "link": vr.link, "frame": vr.frame},
        "controllers_shown": controllers_shown,
        "pads": pads,
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

    /// Both controllers, the right one's grip tracked at (0.2, 1.1, −0.3)
    /// and its stick pushed forward.
    fn pads() -> Vec<f32> {
        let mut d = vec![xr::PADS_LAYOUT, 2.0];
        d.extend_from_slice(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0]);
        d.extend_from_slice(&[0.0; 11]);
        d.extend_from_slice(&[2.0, 1.0, 0.2, 1.1, -0.3, 0.0, 0.0, 0.0, 1.0]);
        d.extend_from_slice(&[0.0; 7]);
        d.extend_from_slice(&[0.0, 0.0, 0.0, -1.0]);
        d
    }

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::input::InputPlugin,
            bevy::asset::AssetPlugin::default(),
        ))
        .init_asset::<Image>()
        .init_asset::<Mesh>()
        .init_asset::<StandardMaterial>()
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
    /// One test: the page's inbox is one static, shared by the tests.
    #[test]
    fn a_session_puts_two_eyes_at_the_views_and_ending_removes_them() {
        let mut app = app();
        page_event("supported", "true");
        app.update();
        assert_eq!(app.world().resource::<WebXr>().state, XrState::Offered);
        page_event(
            "device",
            "Mozilla/5.0 (X11; Linux x86_64; Quest) AppleWebKit/537.36 OculusBrowser/27.1 Chrome/114.0 VR Safari/537.36",
        );
        page_event("enter", "");
        page_event("started", "");
        page_pads(&pads());
        page_frame(&frame(2));
        // Spawned in one update, placed (with commands applied) in the next.
        app.update();
        page_pads(&pads());
        page_frame(&frame(2));
        app.update();
        let xr = app.world().resource::<WebXr>();
        assert_eq!(xr.state, XrState::Running);
        assert_eq!(xr.frames, 2);
        assert!(xr.placed >= 1);
        assert_eq!(xr.pads.len(), 2);
        // `window.ocw.xr()`'s eyes in the views' order (left first).
        let viewports: Vec<u32> = xr.eyes.iter().map(|(.., vp)| vp[0]).collect();
        assert_eq!(viewports, [0, 640]);
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
        // W2: the eyes see the bike computer's layer, the chase camera not.
        let mut layers = app
            .world_mut()
            .query_filtered::<&RenderLayers, With<XrEyeCamera>>();
        assert!(
            layers
                .iter(app.world())
                .all(|l| l.intersects(&RenderLayers::layer(VR_LAYER)))
        );
        // W2: no flat UI in the eyes – the default UI camera is ours, it
        // draws into a picture, and rests while the menu is closed.
        let mut ui = app.world_mut().query_filtered::<(&Camera, &RenderTarget), (
            With<XrUiCamera>,
            With<bevy::ui::IsDefaultUiCamera>,
        )>();
        let ui: Vec<_> = ui.iter(app.world()).collect();
        assert_eq!(ui.len(), 1);
        assert!(matches!(ui[0].1, RenderTarget::Image(_)));
        assert!(!ui[0].0.is_active);
        assert!(page_json().contains("\"flat_ui\":false"), "{}", page_json());
        // The menu open: the UI draws into the panel, which is shown.
        app.insert_resource(MenuView {
            open: true,
            ..default()
        });
        page_frame(&frame(2));
        app.update();
        assert!(
            page_json().contains("\"menu_panel\":true"),
            "{}",
            page_json()
        );
        app.insert_resource(MenuView::default());
        // The right controller is drawn at its grip (in the rig like the
        // eyes: its grip 0.2 m right of and 0.5 m below the head).
        let mut parts = app
            .world_mut()
            .query::<(&XrController, &GlobalTransform, &Visibility)>();
        let right: Vec<_> = parts
            .iter(app.world())
            .filter(|(c, ..)| c.hand == xr::Hand::Right)
            .map(|(_, g, v)| (g.translation(), *v))
            .collect();
        assert_eq!(right.len(), 2);
        assert!(right.iter().all(|(_, v)| *v == Visibility::Inherited));
        let head = (l + r) * 0.5;
        assert!(
            right
                .iter()
                .any(|(p, _)| (p.y - (head.y - 0.5)).abs() < 0.08),
            "{right:?} head {head}"
        );
        // The controllers reach the renderer's VR session, the stick
        // forward is "faster" on the riding stick.
        let vr = app.world().resource::<VrSession>();
        let c = vr.controllers.expect("controllers");
        assert_eq!(c.stick(ocw_xr::controllers::Input::RightStick), [0.0, 1.0]);
        assert!(vr.shown && vr.exits_on_left_stick);
        assert_eq!(vr.controls_hint, xr::CONTROLS_HINT);
        // W3: the Quest 1's user agent names the headset.
        assert!(
            page_json().contains("\"headset\":\"quest1\""),
            "{}",
            page_json()
        );
        page_event("ended", "");
        app.update();
        app.update();
        assert!(eyes(&mut app).is_empty());
        assert!(follow_active(&mut app));
        let mut ui = app.world_mut().query_filtered::<Entity, With<XrUiCamera>>();
        assert_eq!(
            ui.iter(app.world()).count(),
            0,
            "the flat UI back on screen"
        );
        assert!(
            page_json().contains("\"state\":\"ended\""),
            "{}",
            page_json()
        );
    }

    /// W3: the page's layer options before the session: a Quest 1 with
    /// `auto` gets the `quest1` level's render scale and foveation (the
    /// Quest 1's standalone scale), another headset the browser's own.
    #[test]
    fn the_quest_1_gets_the_quest1_layer() {
        let quest1 = "Mozilla/5.0 (X11; Linux x86_64; Quest 1) AppleWebKit/537.36 OculusBrowser/33.0 Chrome/126.0 VR Safari/537.36";
        let v: serde_json::Value = serde_json::from_str(&page_layer(quest1)).unwrap();
        assert_eq!(v["level"], "quest1");
        assert_eq!(v["headset"], "quest1");
        assert!((v["scale"].as_f64().unwrap() - 0.8).abs() < 1e-6);
        assert_eq!(v["foveation"], 1.0);
        assert_eq!(
            ocw_client::quality::QUEST1_RENDER_SCALE,
            ocw_xr::headset::QUEST_1.render_scale_standalone
        );
        let quest3 = "Mozilla/5.0 (X11; Linux x86_64; Quest 3) AppleWebKit/537.36 OculusBrowser/35.0 Chrome/128.0 VR Safari/537.36";
        let v: serde_json::Value = serde_json::from_str(&page_layer(quest3)).unwrap();
        assert_eq!(v["level"], "low");
        assert_eq!(v["scale"], 1.0);
        assert!(v["foveation"].is_null());
        assert_eq!(headset_of(quest1), XrHeadset::Quest1);
        assert_eq!(headset_of(quest3), XrHeadset::Other);
    }
}

//! VR in the renderer (spec §14 Phase 11, X1): the headset as a viewing
//! device of the ride.
//!
//! - **Two eye cameras** render into two images ([`EyeImages`]): Bevy 0.20
//!   has no multiview, so it is a camera per eye, each with the eye's
//!   asymmetric projection ([`EyeProjection`], from `ocw_xr::projection`).
//!   A headset backend (the `openxr` feature: [`openxr`]) copies the
//!   images into the runtime's swapchain; `--vr-sim` only shows them.
//! - **The camera rig** ([`place_vr`]): the rider's steady first-person
//!   eye on the bike (`AvatarRig::steady_eye`), horizon-locked
//!   (`ocw_xr::anchor`), plus the tracked head after the recentre.
//!   The rider's own head is hidden (the eyes are inside it).
//! - **The HUD panel** ([`VrHud`]): a fixed quad in front of and below the
//!   eyes, carried with the bike, showing power, speed, cadence, heart
//!   rate, gradient, distance and time (X3 brings the bike computer on
//!   the stem).
//! - **The mirror** on the monitor ([`VrMirror`]): one eye, both eyes side
//!   by side, or the usual chase camera (for spectators and captures).
//!
//! The world, the server and the inputs do not change: the head pose only
//! moves the cameras (spec §6.1). Everything here is Bevy glue; the maths
//! is `ocw_xr`'s, unit-tested there.

#[cfg(feature = "openxr")]
pub mod openxr;

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{CameraProjection, Exposure, RenderTarget, SubCameraView};
use bevy::math::Vec3A;
use bevy::pbr::{AtmosphereSettings, DistanceFog};
use bevy::prelude::*;
use bevy::render::render_resource::{TextureFormat, TextureUsages};
use bevy::render::view::Msaa;
use bevy::transform::TransformSystems;
use ocw_core::mps_to_kmh;
use ocw_xr::anchor::{world_eyes, world_from_anchor, world_head};
use ocw_xr::frame::FrameStats;
use ocw_xr::hud::HudPanel;
use ocw_xr::projection::{self, Fov};
use ocw_xr::recentre::{Recentre, RecentreAdvice};
use ocw_xr::session::SessionMachine;
use ocw_xr::view::{self, EyeView, StereoViews};

use crate::RideView;
use crate::camera::FollowCamera;
use crate::riders::{Avatar, AvatarRig};

/// How VR runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VrMode {
    /// A mocked head (`--vr-sim`): both eye views on the monitor, no
    /// headset, no OpenXR.
    Sim,
    /// A headset through a backend (OpenXR).
    Headset,
}

/// The VR setup (inserted by the app before [`VrPlugin`]).
#[derive(Resource, Debug, Clone)]
pub struct VrConfig {
    pub mode: VrMode,
    /// Each eye image (px).
    pub eye_px: UVec2,
    /// The eye images' format (the swapchain's).
    pub format: TextureFormat,
    /// Only the bike's heading turns the view (no roll, no pitch: the
    /// comfort default, spec 11B).
    pub horizon_lock: bool,
    /// The simulated eyes (sim mode; the runtime's own otherwise).
    pub sim_ipd_m: f32,
    pub sim_fov: Fov,
    pub hud: HudPanel,
}

impl VrConfig {
    /// `--vr-sim`: Quest 1-like eyes at half the panel's pixels (the
    /// screenshots need no more).
    pub fn sim() -> Self {
        Self {
            mode: VrMode::Sim,
            eye_px: UVec2::new(720, 800),
            format: TextureFormat::Rgba8UnormSrgb,
            horizon_lock: true,
            sim_ipd_m: view::DEFAULT_IPD_M,
            sim_fov: view::QUEST1_LEFT_FOV,
            hud: HudPanel::default(),
        }
    }
}

/// What the monitor shows while VR runs (`vr.mirror`).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VrMirror {
    /// The left eye, filling the window.
    #[default]
    Eye,
    /// Both eyes side by side.
    Both,
    /// The chase camera, as without VR (costs a third view).
    Chase,
}

impl VrMirror {
    pub const CHOICES: &'static [&'static str] = &["eye", "both", "chase"];

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "eye" => Some(Self::Eye),
            "both" => Some(Self::Both),
            "chase" => Some(Self::Chase),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Eye => "eye",
            Self::Both => "both",
            Self::Chase => "chase",
        }
    }
}

/// What the backend tells each frame (tracking space), and the rig's
/// state.
#[derive(Resource, Debug, Default)]
pub struct VrState {
    /// The located eyes (tracking space); `None` before the first frame or
    /// while tracking is lost (the last are kept by the backend).
    pub views: Option<StereoViews>,
    /// The session (headset mode).
    pub session: Option<SessionMachine>,
    /// The frame loop's numbers (headset mode).
    pub stats: FrameStats,
    /// The runtime and headset, one line (headset mode).
    pub device: String,
    pub recentre: Recentre,
    /// Recentre on the next frame (`vr recentre`, a new session).
    pub recentre_now: bool,
    pub advice: RecentreAdvice,
    /// The last frame's rig (world space).
    pub anchor: Option<ocw_xr::Pose>,
    pub head_in_anchor: Option<ocw_xr::Pose>,
    pub eyes: Option<StereoViews>,
    /// Render this frame (the runtime may say no: headset off).
    pub should_render: bool,
}

impl VrState {
    /// `vr status`.
    pub fn status(&self, cfg: &VrConfig, mirror: VrMirror) -> String {
        let mut lines = vec![match cfg.mode {
            VrMode::Sim => "VR: simulated head (--vr-sim), no headset".to_string(),
            VrMode::Headset => format!(
                "VR: {}",
                self.session
                    .as_ref()
                    .map_or("no session".into(), SessionMachine::summary)
            ),
        }];
        if !self.device.is_empty() {
            lines.push(format!("  device: {}", self.device));
        }
        lines.push(format!(
            "  eyes: {}×{} px, {}; mirror: {}; horizon lock: {}",
            cfg.eye_px.x,
            cfg.eye_px.y,
            format_name(cfg.format),
            mirror.name(),
            if cfg.horizon_lock { "on" } else { "off" }
        ));
        if let Some(v) = &self.views {
            let (h, vert) = v[0].fov.extent();
            lines.push(format!(
                "  IPD {:.1} mm, field of view {:.0}° × {:.0}° per eye",
                view::ipd(v) * 1000.0,
                h.to_degrees(),
                vert.to_degrees()
            ));
        }
        if let Some(h) = &self.head_in_anchor {
            lines.push(format!(
                "  head from the anchor: {:.2} m (recentre after {:.1} s away)",
                h.position.length(),
                self.advice.away_s()
            ));
        }
        if cfg.mode == VrMode::Headset {
            lines.push(format!("  {}", self.stats.summary().line()));
        }
        lines.join("\n")
    }
}

fn format_name(f: TextureFormat) -> &'static str {
    match f {
        TextureFormat::Rgba8UnormSrgb => "RGBA8 sRGB",
        TextureFormat::Bgra8UnormSrgb => "BGRA8 sRGB",
        TextureFormat::Rgba16Float => "RGBA16F",
        _ => "other",
    }
}

/// An eye camera (0 left, 1 right).
#[derive(Component, Debug, Clone, Copy)]
pub struct EyeCamera(pub usize);

/// The eye images and the HUD panel's image.
#[derive(Resource, Debug, Clone)]
pub struct EyeImages {
    pub eyes: [Handle<Image>; 2],
    pub hud: Handle<Image>,
}

/// The HUD panel's quad.
#[derive(Component)]
pub struct VrHud;

#[derive(Component)]
struct VrHudText;

/// The mirror camera (window) and its eye pictures.
#[derive(Component)]
struct MirrorCamera;

#[derive(Component)]
struct MirrorEye(usize);

/// The render layer only the eye cameras see (the HUD panel).
pub const VR_LAYER: usize = 7;

/// An eye's projection: OpenXR's asymmetric field of view, infinite
/// reverse Z (Bevy's convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EyeProjection {
    pub fov: Fov,
    pub near: f32,
    /// For the frustum's far side (culling, shadow cascades); the
    /// projection itself has no far plane.
    pub far: f32,
}

impl Default for EyeProjection {
    fn default() -> Self {
        Self {
            fov: view::QUEST1_LEFT_FOV,
            near: projection::NEAR_M,
            far: PerspectiveProjection::default().far,
        }
    }
}

impl CameraProjection for EyeProjection {
    fn get_clip_from_view(&self) -> Mat4 {
        projection::clip_from_view(&self.fov, self.near)
    }

    fn get_clip_from_view_for_sub(&self, _sub_view: &SubCameraView) -> Mat4 {
        self.get_clip_from_view()
    }

    fn update(&mut self, _width: f32, _height: f32) {}

    fn far(&self) -> f32 {
        self.far
    }

    fn get_frustum_corners(&self, z_near: f32, z_far: f32) -> [Vec3A; 8] {
        projection::frustum_corners(&self.fov, z_near, z_far).map(Vec3A::from)
    }
}

pub struct VrPlugin;

impl Plugin for VrPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<VrState>()
            .init_resource::<VrMirror>()
            .add_systems(Startup, setup)
            .add_systems(Update, (hud_text, apply_mirror, recentre_key))
            .add_systems(
                PostUpdate,
                place_vr
                    .after(crate::camera::follow)
                    .after(crate::riders::animate)
                    .before(TransformSystems::Propagate),
            )
            .add_systems(Last, copy_camera_look);
    }
}

fn eye_image(size: UVec2, format: TextureFormat) -> Image {
    let mut image = Image::new_target_texture(size.x.max(16), size.y.max(16), format, None);
    // The backend copies it into the swapchain.
    image.texture_descriptor.usage |= TextureUsages::COPY_SRC;
    image
}

fn setup(
    mut commands: Commands,
    cfg: Res<VrConfig>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let eyes = [0, 1].map(|_| images.add(eye_image(cfg.eye_px, cfg.format)));
    let (hw, hh) = cfg.hud.texture_px;
    let hud = images.add(Image::new_target_texture(
        hw,
        hh,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    for (i, image) in eyes.iter().enumerate() {
        commands.spawn((
            Camera3d::default(),
            Camera {
                // Before the window's cameras.
                order: -2 + i as isize,
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
            Projection::custom(EyeProjection {
                fov: if i == 0 {
                    cfg.sim_fov
                } else {
                    cfg.sim_fov.mirrored()
                },
                ..default()
            }),
            Transform::default(),
            crate::world::fog(),
            bevy::light::ShadowFilteringMethod::Gaussian,
            RenderLayers::from_layers(&[0, VR_LAYER]),
            EyeCamera(i),
        ));
    }
    // The HUD panel: its own UI camera into the panel's image …
    let hud_cam = commands
        .spawn((
            Camera2d,
            Camera {
                order: -10,
                clear_color: ClearColorConfig::Custom(Color::srgba(0.03, 0.04, 0.06, 0.78)),
                ..default()
            },
            RenderTarget::Image(hud.clone().into()),
        ))
        .id();
    commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            UiTargetCamera(hud_cam),
        ))
        .with_child((
            Text::new(""),
            TextFont {
                font_size: 52.0.into(),
                ..default()
            },
            TextColor(Color::srgb(0.95, 0.95, 0.92)),
            TextLayout::justify(Justify::Center),
            VrHudText,
        ));
    // … on a quad the eyes see (not the chase camera).
    let quad = materials.add(StandardMaterial {
        base_color_texture: Some(hud.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(cfg.hud.width_m, cfg.hud.height_m))),
        MeshMaterial3d(quad),
        Transform::default(),
        RenderLayers::layer(VR_LAYER),
        bevy::light::NotShadowCaster,
        VrHud,
    ));
    // The mirror: a camera on the window above the chase camera, with the
    // eye pictures behind the desktop HUD (it is the default UI camera).
    let mirror = commands
        .spawn((
            Camera2d,
            Camera {
                order: 10,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            MirrorCamera,
        ))
        .id();
    commands
        .spawn((
            Node {
                width: percent(100),
                height: percent(100),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                column_gap: px(4),
                ..default()
            },
            GlobalZIndex(-100),
            UiTargetCamera(mirror),
        ))
        .with_children(|row| {
            for (i, image) in eyes.iter().enumerate() {
                row.spawn((
                    ImageNode::new(image.clone()),
                    Node {
                        height: percent(100),
                        aspect_ratio: Some(cfg.eye_px.x as f32 / cfg.eye_px.y.max(1) as f32),
                        ..default()
                    },
                    MirrorEye(i),
                ));
            }
        });
    commands.insert_resource(EyeImages { eyes, hud });
}

/// `C` (the chase camera's reset) recentres in VR.
fn recentre_key(
    keys: Res<ButtonInput<KeyCode>>,
    context: Option<Res<crate::menu::InputContext>>,
    mut state: ResMut<VrState>,
) {
    if crate::menu::ride_input(context) && keys.just_pressed(KeyCode::KeyC) {
        state.recentre_now = true;
    }
}

/// The world pose of a Bevy transform.
fn pose_of(t: &Transform) -> ocw_xr::Pose {
    ocw_xr::Pose::new(t.rotation, t.translation)
}

fn transform_of(p: &ocw_xr::Pose) -> Transform {
    Transform::from_translation(p.position).with_rotation(p.orientation)
}

/// Places the eye cameras, the HUD panel and (unless the mirror shows the
/// chase view) the follow camera at the head, from the local rider's bike
/// and the tracked (or simulated) eyes.
#[allow(clippy::too_many_arguments, clippy::type_complexity)] // a Bevy system
pub fn place_vr(
    cfg: Res<VrConfig>,
    mirror: Res<VrMirror>,
    time: Res<Time>,
    mut state: ResMut<VrState>,
    avatars: Query<(&Avatar, &Transform, &AvatarRig), (Without<EyeCamera>, Without<VrHud>)>,
    mut eyes: Query<
        (&EyeCamera, &mut Transform, &mut Projection, &mut Camera),
        (Without<Avatar>, Without<VrHud>, Without<FollowCamera>),
    >,
    mut hud: Query<&mut Transform, (With<VrHud>, Without<Avatar>, Without<FollowCamera>)>,
    mut follow: Query<
        (&mut Transform, &mut Camera),
        (
            With<FollowCamera>,
            Without<EyeCamera>,
            Without<Avatar>,
            Without<VrHud>,
        ),
    >,
    mut visibility: Query<&mut Visibility>,
) {
    let Some((_, bike, rig)) = avatars.iter().find(|(a, _, _)| a.is_local) else {
        return;
    };
    // The rider's head is where the eyes are: hidden.
    if let Some(head) = rig.head_entity()
        && let Ok(mut v) = visibility.get_mut(head)
        && *v != Visibility::Hidden
    {
        *v = Visibility::Hidden;
    }
    let anchor = world_from_anchor(&pose_of(bike), rig.steady_eye(), cfg.horizon_lock);
    if cfg.mode == VrMode::Sim {
        let head = view::sim_head(time.elapsed_secs());
        state.views = Some(view::sim_views(&head, cfg.sim_ipd_m, &cfg.sim_fov));
        state.should_render = true;
    }
    let Some(views) = state.views else {
        return;
    };
    let head = view::head(&views);
    if state.recentre_now || state.anchor.is_none() {
        state.recentre = Recentre::at(&head);
        state.recentre_now = false;
    }
    let head_in_anchor = state.recentre.apply(&head);
    let dt = time.delta_secs();
    if state.advice.update(&head_in_anchor, dt) {
        state.recentre = Recentre::at(&head);
    }
    let world = world_eyes(&anchor, &state.recentre, &views);
    let render = state.should_render;
    for (eye, mut t, mut p, mut cam) in &mut eyes {
        let EyeView { pose, fov } = world[eye.0.min(1)];
        *t = transform_of(&pose);
        if let Projection::Custom(c) = &mut *p
            && let Some(ep) = c.get_mut::<EyeProjection>()
            && ep.fov != fov
        {
            ep.fov = fov;
        }
        if cam.is_active != render {
            cam.is_active = render;
        }
    }
    let panel = anchor.mul(&cfg.hud.anchor_from_panel());
    for mut t in &mut hud {
        *t = transform_of(&panel);
    }
    // The follow camera: at the head (distance-based detail and fading
    // around the rider's eyes), drawing only for the chase mirror.
    let chase = *mirror == VrMirror::Chase;
    for (mut t, mut cam) in &mut follow {
        if !chase {
            *t = transform_of(&world_head(&anchor, &state.recentre, &head));
        }
        if cam.is_active != chase {
            cam.is_active = chase;
        }
    }
    state.anchor = Some(anchor);
    state.head_in_anchor = Some(head_in_anchor);
    state.eyes = Some(world);
}

/// The mirror's layout.
fn apply_mirror(
    mirror: Res<VrMirror>,
    mut cams: Query<&mut Camera, With<MirrorCamera>>,
    mut nodes: Query<(&MirrorEye, &mut Node)>,
) {
    if !mirror.is_changed() {
        return;
    }
    for mut c in &mut cams {
        // The chase view below shows through.
        c.clear_color = match *mirror {
            VrMirror::Chase => ClearColorConfig::None,
            _ => ClearColorConfig::Custom(Color::BLACK),
        };
    }
    for (eye, mut n) in &mut nodes {
        let shown = match *mirror {
            VrMirror::Eye => eye.0 == 0,
            VrMirror::Both => true,
            VrMirror::Chase => false,
        };
        n.display = if shown { Display::Flex } else { Display::None };
        // One eye fills the window's height (cropped at the sides by
        // nothing: the aspect ratio stays), two share the width.
        n.height = if *mirror == VrMirror::Both {
            Val::Auto
        } else {
            percent(100)
        };
        n.width = if *mirror == VrMirror::Both {
            percent(49.5)
        } else {
            Val::Auto
        };
    }
}

/// The panel's text: the riding numbers.
pub fn hud_line(view: &RideView) -> String {
    let Some(l) = view.local() else {
        return "Open Cycling World".into();
    };
    let grade = view.track(l.id).sample(l.state.distance).grade * 100.0;
    let cadence = l
        .cadence_rpm
        .map_or("– rpm".to_string(), |c| format!("{c:.0} rpm"));
    let hr = l
        .heart_rate_bpm
        .map_or("– bpm".to_string(), |h| format!("{h:.0} bpm"));
    let t = view.elapsed_s.max(0.0) as u64;
    format!(
        "{:.0} W   {:.1} km/h\n{cadence}   {hr}\n{grade:+.1} %   {:.2} km   {}:{:02}:{:02}",
        l.power_w,
        mps_to_kmh(l.state.speed),
        l.state.distance / 1000.0,
        t / 3600,
        t / 60 % 60,
        t % 60
    )
}

fn hud_text(view: Option<Res<RideView>>, mut texts: Query<&mut Text, With<VrHudText>>) {
    let Some(view) = view else { return };
    let line = hud_line(&view);
    for mut t in &mut texts {
        if t.0 != line {
            t.0.clone_from(&line);
        }
    }
}

/// The eyes look like the chase camera: its fog, exposure, MSAA, far
/// plane and atmosphere (set by the sky, the weather and the quality
/// preset, which only know the follow camera).
#[allow(clippy::type_complexity)]
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
        (With<FollowCamera>, Without<EyeCamera>),
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
        With<EyeCamera>,
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
            // The sky changes the fog's colour every frame.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_words() {
        for w in VrMirror::CHOICES {
            assert_eq!(VrMirror::parse(w).unwrap().name(), *w);
        }
        assert_eq!(VrMirror::parse("left"), None);
        assert_eq!(VrMirror::default(), VrMirror::Eye);
    }

    /// The eye projection is ocw_xr's: Bevy gets the same matrix and the
    /// same frustum.
    #[test]
    fn the_eye_projection_is_bevys_custom_projection() {
        let p = EyeProjection::default();
        let m = p.get_clip_from_view();
        assert_eq!(
            m,
            projection::clip_from_view(&view::QUEST1_LEFT_FOV, projection::NEAR_M)
        );
        let c = p.get_frustum_corners(-1.0, -100.0);
        let expect = projection::frustum_corners(&view::QUEST1_LEFT_FOV, -1.0, -100.0);
        assert!(
            c.iter()
                .zip(expect)
                .all(|(a, b)| (Vec3::from(*a) - b).length() < 1e-5)
        );
        let proj = Projection::custom(p);
        let Projection::Custom(custom) = proj else {
            panic!("custom")
        };
        assert_eq!(custom.get::<EyeProjection>(), Some(&p));
    }

    #[test]
    fn the_panel_shows_the_riding_numbers() {
        let route = ocw_world::Route::from_points(
            "t",
            "T",
            vec![[0.0, 0.0, 0.0], [0.0, 10.0, 1000.0]],
            false,
            6.0,
        )
        .unwrap();
        let mut v = RideView::new(route);
        assert_eq!(hud_line(&v), "Open Cycling World");
        v.elapsed_s = 3725.0;
        v.riders.push(crate::RiderView {
            id: ocw_core::RiderId(1),
            name: "Me".into(),
            state: ocw_sim::RiderState {
                distance: 500.0,
                speed: 10.0,
                ..Default::default()
            },
            power_w: 250.0,
            cadence_rpm: Some(88.0),
            heart_rate_bpm: None,
            is_local: true,
            bike: ocw_core::ProfileId::Road,
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
        });
        let l = hud_line(&v);
        assert!(
            l.starts_with("250 W   36.0 km/h\n88 rpm   – bpm\n+1.0 %   0.50 km   1:02:05"),
            "{l}"
        );
    }

    #[test]
    fn status_names_the_mode_and_the_eyes() {
        let s = VrState::default().status(&VrConfig::sim(), VrMirror::Both);
        assert!(s.contains("simulated head"), "{s}");
        assert!(s.contains("720×800 px, RGBA8 sRGB; mirror: both"), "{s}");
        let mut st = VrState {
            views: Some(view::sim_views(
                &ocw_xr::Pose::IDENTITY,
                0.064,
                &view::QUEST1_LEFT_FOV,
            )),
            ..Default::default()
        };
        st.session = Some(SessionMachine::unavailable(
            ocw_xr::session::XrError::NoHeadset,
        ));
        let cfg = VrConfig {
            mode: VrMode::Headset,
            ..VrConfig::sim()
        };
        let s = st.status(&cfg, VrMirror::Eye);
        assert!(s.contains("VR: unavailable: no headset"), "{s}");
        assert!(s.contains("IPD 64.0 mm"), "{s}");
        assert!(s.contains("missed 0 of 0"), "{s}");
    }
}

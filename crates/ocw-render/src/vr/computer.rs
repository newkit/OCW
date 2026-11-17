//! The bike computer in VR (spec §14 Phase 11, 11B; decided 2026-11-17,
//! v4.38): all ride information on a GPS unit on an out-front mount at the
//! centre of the handlebar – no floating panel in front of the rider. The
//! model file is `parts/computer/computer-head-unit-wedge.glb` (Phase 15,
//! `ocw_assets::computer`); its place, size and tilt are `ocw_xr::computer`
//! (fitted to every bike's bar: drop, flat, the TT extensions). The screen
//! is a Bevy UI drawn into a texture ([`computer::TEXTURE_PX`]) on a quad
//! over the model's screen window: the clock and the weather, an LED bar in
//! the gradient's colour, power, speed, cadence, heart rate, gradient,
//! distance and time, the next turn with its distance (free ride) or the
//! route's progress, the elevation profile ahead, and the messages as a
//! banner over the bottom.
//!
//! The unit follows the local rider's sway entity (it rocks and turns
//! with the bike; placed in the same frame as the eyes, before the
//! transforms propagate) on the eyes' render layer only: the monitor's chase
//! mirror, the desktop HUD and the other riders do not see it.

use bevy::asset::RenderAssetUsages;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use ocw_core::mps_to_kmh;
use ocw_world::{ElevationProfile, GradeBand, ProfileSpec};
use ocw_xr::computer::{self, BarMount, ComputerSetup, Placement};

use super::VR_LAYER;
use crate::RideView;
use crate::hud::{LookAhead, Units, band_color, profile_bars};
use crate::riders::{Avatar, AvatarRig};

/// The model file (`ocw_assets::computer::FILE` without `.glb`).
pub const MODEL: &str = "parts/computer/computer-head-unit-wedge";
/// Bars of the elevation profile on the screen.
const PROFILE_BARS: usize = 40;

/// The rider's size and tilt (settings `vr.computer_size`,
/// `vr.computer_tilt`; console `vr computer …`); the screen aims at the
/// rider's steady eye (where a recentre puts the head).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct ComputerSettings {
    pub setup: ComputerSetup,
}

/// What the screen shows (pure: from the ride view).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScreenView {
    pub clock: String,
    pub weather: String,
    pub power: String,
    /// (value, unit) per field: speed, cadence, heart rate, gradient,
    /// distance, time.
    pub fields: [(String, &'static str); 6],
    /// The next turn's angle (degrees, + right) and the lines beside the
    /// arrow; `None`: straight on (no junction ahead).
    pub turn_deg: Option<f32>,
    pub nav: String,
    pub nav_detail: String,
    /// The message banner (notices, app messages).
    pub toast: Option<String>,
    pub band: Option<GradeBand>,
    /// LEDs lit (0 … 7): 1 per 2 % of gradient.
    pub leds: usize,
}

/// The sky's HUD clock ("☼ 18:42 x600", "◐ 21:40 paused") without the
/// time machine's word: the symbol and the time fit the screen's top line.
pub fn clock(hud: &str) -> String {
    hud.split_whitespace().take(2).collect::<Vec<_>>().join(" ")
}

/// The ride's time in at most five characters (a field is that wide):
/// "12:05" (minutes, seconds), from an hour on "1h02".
pub fn ride_time(s: f64) -> String {
    let t = s.max(0.0) as u64;
    if t < 3600 {
        format!("{:02}:{:02}", t / 60, t % 60)
    } else {
        format!("{}h{:02}", t / 3600, t / 60 % 60)
    }
}

/// A distance in at most five characters: "12.34", "123.4".
fn short_distance(d: f64) -> String {
    if d < 100.0 {
        format!("{d:.2}")
    } else {
        format!("{d:.1}")
    }
}

/// Cuts `s` to `n` characters (an ellipsis when cut).
fn fit(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// The screen's text from the ride (the same numbers as the desktop HUD).
pub fn screen_view(view: &RideView, track: &dyn ocw_world::Track, units: Units) -> ScreenView {
    let mut out = ScreenView {
        clock: view
            .sky
            .as_ref()
            .map(|s| clock(&s.hud))
            .unwrap_or_default(),
        weather: view
            .weather
            .as_ref()
            .map(|w| fit(&w.summary, 11))
            .unwrap_or_default(),
        ..Default::default()
    };
    let time = ride_time(view.elapsed_s);
    let Some(l) = view.local() else {
        out.power = "–".into();
        out.fields = [
            ("–".into(), "km/h"),
            ("–".into(), "rpm"),
            ("–".into(), "bpm"),
            ("–".into(), "%"),
            ("–".into(), "km"),
            (time, "time"),
        ];
        out.nav = "Connecting".into();
        return out;
    };
    let grade = track.sample(l.state.distance).grade;
    let tidy = |g: f64| if g.abs() < 0.0005 { 0.0 } else { g };
    let (speed, speed_unit) = units.speed(mps_to_kmh(l.state.speed));
    let (dist, dist_unit) = units.distance(l.state.distance);
    out.power = format!("{:.0}", l.power_w);
    out.fields = [
        (format!("{speed:.1}"), speed_unit),
        (
            l.cadence_rpm.map_or("–".into(), |c| format!("{c:.0}")),
            "rpm",
        ),
        (
            l.heart_rate_bpm.map_or("–".into(), |h| format!("{h:.0}")),
            "bpm",
        ),
        (format!("{:+.1}", tidy(grade) * 100.0), "%"),
        (short_distance(dist), dist_unit),
        (time, "time"),
    ];
    out.band = Some(GradeBand::of(grade));
    out.leds = ((grade.abs() * 100.0 / 2.0).ceil() as usize).min(7);
    if let Some(j) = &view.junction
        && let Some(exit) = j.exits.get(j.selected)
    {
        out.turn_deg = Some(exit.angle_deg as f32);
        out.nav = if j.distance_m >= 0.0 {
            format!("{:.0} m", j.distance_m)
        } else {
            "now".into()
        };
        out.nav_detail = fit(&exit.name, 11);
    } else if let Some(r) = &view.route_status {
        out.nav = fit(r, 11);
        out.nav_detail = fit(&track.name_at(l.state.distance), 11);
    } else {
        out.nav = if view.free_ride { "Free ride" } else { "Ride" }.into();
        out.nav_detail = fit(&track.name_at(l.state.distance), 11);
    }
    out.toast = view
        .notice
        .clone()
        .or_else(|| view.app_message.clone())
        .or_else(|| view.strava_message.clone())
        .map(|m| fit(&m, 40));
    out
}

/// The screen's parts.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
enum ScreenText {
    Clock,
    Weather,
    Power,
    Field(usize),
    Unit(usize),
    Arrow,
    Nav,
    NavDetail,
    Toast,
}

#[derive(Component)]
struct ToastBox;

#[derive(Component)]
struct Led(usize);

#[derive(Component)]
struct ProfileBar(usize);

#[derive(Component)]
struct ProfileRider;

/// The unit's root (where the sway entity is) and its parts.
#[derive(Component)]
pub struct ComputerRoot;

#[derive(Component)]
struct ComputerUnit;

#[derive(Component)]
struct ComputerArm;

/// The unit's lamps (the model's `computer_led` material).
#[derive(Resource)]
struct LedMaterial(Handle<StandardMaterial>);

/// Where the unit was placed (to redo it only on a change).
#[derive(Resource, Default)]
struct Mounted {
    key: Option<(BarMount, Vec3, ComputerSetup)>,
}

/// The screen's image.
#[derive(Resource, Debug, Clone)]
pub struct ComputerImage(pub Handle<Image>);

pub fn plugin(app: &mut App) {
    app.init_resource::<ComputerSettings>()
        .init_resource::<Mounted>()
        .add_systems(Startup, setup)
        .add_systems(Update, (screen_text, profile))
        .add_systems(
            PostUpdate,
            mount
                .after(crate::riders::animate)
                .before(bevy::transform::TransformSystems::Propagate),
        );
}

const FG: Color = Color::srgb(0.93, 0.95, 0.92);
const DIM: Color = Color::srgb(0.62, 0.68, 0.66);
const BG: Color = Color::srgb(0.035, 0.05, 0.045);

fn text(s: &str, px: f32, color: Color) -> (Text, TextFont, TextColor) {
    (
        Text::new(s),
        TextFont {
            font_size: px.into(),
            ..default()
        },
        TextColor(color),
    )
}

/// The model's meshes per slot (in the unit's frame).
fn meshes_of(model: &ocw_assets::model::Model) -> Vec<(Mesh, String, [f32; 4])> {
    let mut out = Vec::new();
    for (i, n) in model.nodes.iter().enumerate() {
        let w = model.world(i);
        for p in &n.primitives {
            let mut m = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            let pos: Vec<[f32; 3]> = p
                .positions
                .iter()
                .map(|v| w.transform_point3(Vec3::from(*v)).to_array())
                .collect();
            let nor: Vec<[f32; 3]> = p
                .normals
                .iter()
                .map(|v| {
                    w.transform_vector3(Vec3::from(*v))
                        .normalize_or_zero()
                        .to_array()
                })
                .collect();
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
            m.insert_indices(Indices::U32(p.indices.clone()));
            let mat = &model.materials[p.material];
            out.push((m, mat.name.clone(), mat.color));
        }
    }
    out
}

#[allow(clippy::too_many_lines)] // one UI tree
fn setup(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (tw, th) = computer::TEXTURE_PX;
    let image = images.add(Image::new_target_texture(
        tw,
        th,
        TextureFormat::Rgba8UnormSrgb,
        None,
    ));
    let cam = commands
        .spawn((
            Camera2d,
            Camera {
                order: -10,
                clear_color: ClearColorConfig::Custom(BG),
                ..default()
            },
            RenderTarget::Image(image.clone().into()),
        ))
        .id();
    let row = |h: f32| Node {
        width: percent(100),
        height: px(h),
        flex_shrink: 0.0,
        flex_direction: FlexDirection::Row,
        justify_content: JustifyContent::SpaceBetween,
        align_items: AlignItems::Center,
        padding: UiRect::horizontal(px(14)),
        ..default()
    };
    let field = |c: &mut ChildSpawnerCommands<'_>, i: usize| {
        c.spawn(Node {
            width: percent(50),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::SpaceEvenly,
            padding: UiRect::horizontal(px(14)),
            border: UiRect::all(px(1)),
            ..default()
        })
        .insert(BorderColor::all(Color::srgb(0.16, 0.2, 0.19)))
        .with_children(|f| {
            f.spawn((text("", computer::MIN_FONT_PX, DIM), ScreenText::Unit(i)));
            f.spawn((text("", 76.0, FG), ScreenText::Field(i)));
        });
    };
    commands
        .spawn((
            Node {
                width: px(tw as f32),
                height: px(th as f32),
                flex_direction: FlexDirection::Column,
                ..default()
            },
            UiTargetCamera(cam),
        ))
        .with_children(|s| {
            // The clock and the weather.
            s.spawn(row(58.0)).with_children(|r| {
                r.spawn((text("", computer::MIN_FONT_PX, FG), ScreenText::Clock));
                r.spawn((text("", computer::MIN_FONT_PX, DIM), ScreenText::Weather));
            });
            // The LED bar.
            s.spawn(Node {
                width: percent(100),
                height: px(16),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Center,
                column_gap: px(10),
                padding: UiRect::vertical(px(4)),
                ..default()
            })
            .with_children(|r| {
                for i in 0..7 {
                    r.spawn((
                        Node {
                            width: px(64),
                            height: percent(100),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.1, 0.12, 0.12)),
                        Led(i),
                    ));
                }
            });
            // Power, large.
            s.spawn(Node {
                width: percent(100),
                height: px(150),
                flex_shrink: 0.0,
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Baseline,
                column_gap: px(12),
                ..default()
            })
            .with_children(|r| {
                r.spawn((text("", 130.0, FG), ScreenText::Power));
                r.spawn(text("W", 64.0, DIM));
            });
            // Six fields.
            for pair in 0..3 {
                s.spawn(Node {
                    width: percent(100),
                    height: px(160),
                    flex_shrink: 0.0,
                    flex_direction: FlexDirection::Row,
                    ..default()
                })
                .with_children(|r| {
                    field(r, 2 * pair);
                    field(r, 2 * pair + 1);
                });
            }
            // The next turn.
            s.spawn(row(74.0)).with_children(|r| {
                r.spawn((
                    text("↑", 64.0, FG),
                    UiTransform::default(),
                    ScreenText::Arrow,
                ));
                r.spawn((text("", 56.0, FG), ScreenText::Nav));
                r.spawn((text("", computer::MIN_FONT_PX, DIM), ScreenText::NavDetail));
            });
            // The elevation profile ahead.
            s.spawn(Node {
                width: percent(100),
                flex_grow: 1.0,
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexEnd,
                padding: UiRect::new(px(14), px(14), px(8), px(10)),
                ..default()
            })
            .with_children(|r| {
                for i in 0..PROFILE_BARS {
                    r.spawn((
                        Node {
                            flex_grow: 1.0,
                            height: percent(0),
                            ..default()
                        },
                        BackgroundColor(Color::NONE),
                        ProfileBar(i),
                    ));
                }
                r.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        width: px(4),
                        top: px(8),
                        bottom: px(10),
                        left: percent(0),
                        ..default()
                    },
                    BackgroundColor(Color::WHITE),
                    ProfileRider,
                ));
            });
            // The message banner over the bottom.
            s.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(10),
                    right: px(10),
                    bottom: px(16),
                    padding: UiRect::all(px(12)),
                    border_radius: BorderRadius::all(px(10)),
                    display: Display::None,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.85, 0.55, 0.1, 0.95)),
                ToastBox,
            ))
            .with_child((
                text("", computer::MIN_FONT_PX, Color::BLACK),
                TextLayout::justify(Justify::Center),
                ScreenText::Toast,
            ));
        });
    // The unit: the model file's meshes, the live screen over its window.
    let model = ocw_assets::library::library().get(MODEL);
    let root = commands
        .spawn((
            Transform::default(),
            Visibility::Hidden,
            RenderLayers::layer(VR_LAYER),
            ComputerRoot,
        ))
        .id();
    let unit = commands
        .spawn((
            Transform::default(),
            Visibility::Inherited,
            RenderLayers::layer(VR_LAYER),
            ComputerUnit,
            ChildOf(root),
        ))
        .id();
    let mut led = None;
    let mut screen_at = Vec3::new(0.0, 0.5 * computer::BODY_M.y, -computer::SCREEN_AHEAD_M);
    match &model {
        Ok(m) => {
            if let Some(p) = m.position(ocw_assets::computer::SCREEN_ANCHOR) {
                screen_at = p;
            }
            for (mesh, slot, color) in meshes_of(m) {
                let mut mat = StandardMaterial {
                    base_color: Color::srgba(color[0], color[1], color[2], color[3]),
                    perceptual_roughness: 0.5,
                    ..default()
                };
                let s = ocw_assets::slots::get(&slot);
                if let Some(s) = s {
                    mat.metallic = s.metallic;
                    mat.perceptual_roughness = s.roughness;
                }
                let is_led = slot == "computer_led";
                if is_led {
                    mat.emissive = LinearRgba::rgb(0.2, 0.9, 1.6);
                }
                let h = materials.add(mat);
                if is_led {
                    led = Some(h.clone());
                }
                commands.spawn((
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(h),
                    Transform::default(),
                    RenderLayers::layer(VR_LAYER),
                    bevy::light::NotShadowCaster,
                    ChildOf(unit),
                ));
            }
        }
        Err(e) => warn!("VR bike computer: no model ({e}); the screen only"),
    }
    if let Some(h) = led {
        commands.insert_resource(LedMaterial(h));
    }
    // The live screen: unlit (it glows like a backlit display), over the
    // window, its picture's top toward the nose (−Z).
    let (sw, sl) = computer::SCREEN_M;
    let screen_mat = materials.add(StandardMaterial {
        base_color_texture: Some(image.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Opaque,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Rectangle::new(sw, sl))),
        MeshMaterial3d(screen_mat),
        Transform::from_translation(screen_at + Vec3::Y * 0.0003)
            .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
        RenderLayers::layer(VR_LAYER),
        bevy::light::NotShadowCaster,
        ChildOf(unit),
    ));
    // The mount's arm: a unit cylinder, stretched by `mount`.
    let arm_mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.08, 0.08, 0.09),
        perceptual_roughness: 0.45,
        metallic: 0.3,
        ..default()
    });
    commands.spawn((
        Mesh3d(meshes.add(Cylinder::new(1.0, 1.0))),
        MeshMaterial3d(arm_mat),
        Transform::default(),
        RenderLayers::layer(VR_LAYER),
        bevy::light::NotShadowCaster,
        ComputerArm,
        ChildOf(root),
    ));
    commands.insert_resource(ComputerImage(image));
}

/// The bar of a bike rig (bike frame).
pub fn bar_mount(bike: &ocw_assets::pose::BikeRig) -> BarMount {
    BarMount {
        clamp: bike.bar_clamp,
        aero_tips: bike
            .aero_bars
            .then(|| 0.5 * (bike.hoods[0] + bike.hoods[1])),
    }
}

/// The arm's transform: a unit cylinder (along Y, radius 1) from `a` to `b`.
fn arm_transform(a: Vec3, b: Vec3, radius: f32) -> Transform {
    let d = b - a;
    let len = d.length().max(1e-4);
    Transform::from_translation(0.5 * (a + b))
        .with_rotation(Quat::from_rotation_arc(Vec3::Y, d / len))
        .with_scale(Vec3::new(radius, len, radius))
}

fn transform_of(p: &ocw_xr::Pose, scale: f32) -> Transform {
    Transform::from_translation(p.position)
        .with_rotation(p.orientation)
        .with_scale(Vec3::splat(scale))
}

/// Puts the unit on the local rider's bar: where the bike and its sway
/// are this frame, and the unit on the mount (again when the bike, the
/// size, the tilt or the aim changed).
#[allow(clippy::type_complexity)]
fn mount(
    settings: Res<ComputerSettings>,
    mut mounted: ResMut<Mounted>,
    avatars: Query<(&Avatar, &AvatarRig, &Transform), Without<ComputerRoot>>,
    transforms: Query<
        &Transform,
        (
            Without<ComputerRoot>,
            Without<ComputerUnit>,
            Without<ComputerArm>,
        ),
    >,
    mut root: Query<(&mut Transform, &mut Visibility), With<ComputerRoot>>,
    mut unit: Query<
        &mut Transform,
        (
            With<ComputerUnit>,
            Without<ComputerArm>,
            Without<ComputerRoot>,
            Without<Avatar>,
        ),
    >,
    mut arm: Query<
        &mut Transform,
        (
            With<ComputerArm>,
            Without<ComputerUnit>,
            Without<ComputerRoot>,
            Without<Avatar>,
        ),
    >,
) {
    let Ok((mut root_t, mut vis)) = root.single_mut() else {
        return;
    };
    let Some((_, rig, bike_t)) = avatars.iter().find(|(a, _, _)| a.is_local) else {
        if *vis != Visibility::Hidden {
            *vis = Visibility::Hidden;
        }
        return;
    };
    // The sway is a child of the avatar: the same chain, this frame.
    let sway = transforms
        .get(rig.sway_entity())
        .copied()
        .unwrap_or_default();
    let at = bike_t.mul_transform(sway);
    if *root_t != at {
        *root_t = at;
    }
    if *vis != Visibility::Inherited {
        *vis = Visibility::Inherited;
    }
    let bar = bar_mount(rig.bike_rig());
    let eye = rig.steady_eye();
    let key = (bar, eye, settings.setup);
    if mounted.key == Some(key) {
        return;
    }
    let p: Placement = computer::place(&bar, eye, &settings.setup);
    if let Ok(mut t) = unit.single_mut() {
        *t = transform_of(&p.bike_from_unit, p.scale);
    }
    if let Ok(mut t) = arm.single_mut() {
        *t = arm_transform(p.arm_from, p.arm_to, 0.006 * p.scale.sqrt());
    }
    mounted.key = Some(key);
}

#[allow(clippy::too_many_arguments)] // a Bevy system
fn screen_text(
    view: Option<Res<RideView>>,
    ahead: Option<Res<LookAhead>>,
    units: Option<Res<Units>>,
    led_mat: Option<Res<LedMaterial>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut texts: Query<(&ScreenText, &mut Text, Option<&mut UiTransform>)>,
    mut toast: Query<&mut Node, With<ToastBox>>,
    mut leds: Query<(&Led, &mut BackgroundColor)>,
    mut last_band: Local<Option<GradeBand>>,
) {
    let Some(view) = view else { return };
    let units = units.map(|u| *u).unwrap_or_default();
    let s = match (view.local(), &ahead) {
        (Some(l), Some(a)) => screen_view(&view, a.track(&view, l.id), units),
        (Some(l), None) => screen_view(&view, view.track(l.id), units),
        (None, _) => screen_view(&view, &view.route, units),
    };
    for (part, mut t, ui) in &mut texts {
        let want = match *part {
            ScreenText::Clock => s.clock.clone(),
            ScreenText::Weather => s.weather.clone(),
            ScreenText::Power => s.power.clone(),
            ScreenText::Unit(i) => s.fields[i].1.to_string(),
            ScreenText::Field(i) => s.fields[i].0.clone(),
            ScreenText::Arrow => {
                if let Some(mut ui) = ui {
                    let deg = s.turn_deg.unwrap_or(0.0).clamp(-180.0, 180.0);
                    // Clockwise for a right turn (UiTransform's sense).
                    let r = Rot2::degrees(deg);
                    if ui.rotation != r {
                        ui.rotation = r;
                    }
                }
                "↑".into()
            }
            ScreenText::Nav => s.nav.clone(),
            ScreenText::NavDetail => s.nav_detail.clone(),
            ScreenText::Toast => s.toast.clone().unwrap_or_default(),
        };
        if t.0 != want {
            t.0 = want;
        }
    }
    if let Ok(mut n) = toast.single_mut() {
        let d = if s.toast.is_some() {
            Display::Flex
        } else {
            Display::None
        };
        if n.display != d {
            n.display = d;
        }
    }
    let lit = s.band.map_or(Color::srgb(0.3, 0.3, 0.3), |b| band_color(b, false));
    for (led, mut bg) in &mut leds {
        let c = if led.0 < s.leds.max(1) {
            lit
        } else {
            Color::srgb(0.1, 0.12, 0.12)
        };
        if bg.0 != c {
            bg.0 = c;
        }
    }
    if *last_band != s.band
        && let Some(h) = led_mat
        && let Some(mut m) = materials.get_mut(&h.0)
    {
        m.emissive = LinearRgba::from(lit) * 2.0;
        *last_band = s.band;
    }
}

#[allow(clippy::type_complexity)]
fn profile(
    view: Option<Res<RideView>>,
    ahead: Option<Res<LookAhead>>,
    mut bars: Query<(&ProfileBar, &mut Node, &mut BackgroundColor), Without<ProfileRider>>,
    mut rider: Query<&mut Node, (With<ProfileRider>, Without<ProfileBar>)>,
) {
    let Some(view) = view else { return };
    let Some(l) = view.local() else { return };
    let spec = ProfileSpec::default();
    let track = match &ahead {
        Some(a) => a.track(&view, l.id),
        None => view.track(l.id),
    };
    let p = ElevationProfile::sample(track, l.state.distance, &spec);
    let all = profile_bars(&p);
    // The desktop's bars, merged into fewer, wider ones.
    let per = (all.len() as f32 / PROFILE_BARS as f32).max(1.0);
    for (bar, mut node, mut color) in &mut bars {
        let from = (bar.0 as f32 * per) as usize;
        let to = (((bar.0 + 1) as f32 * per) as usize).min(all.len());
        let views: Vec<_> = all[from.min(all.len())..to].iter().flatten().collect();
        let (h, c) = match views.iter().max_by(|a, b| a.level.total_cmp(&b.level)) {
            Some(v) => (
                Val::Percent((v.level * 100.0) as f32),
                band_color(v.band, v.behind),
            ),
            None => (Val::Percent(0.0), Color::NONE),
        };
        if node.height != h {
            node.height = h;
        }
        if color.0 != c {
            color.0 = c;
        }
    }
    if let Ok(mut n) = rider.single_mut() {
        let left = Val::Percent((p.rider_frac * 100.0) as f32);
        if n.left != left {
            n.left = left;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_core::ProfileId;

    fn view() -> RideView {
        let route = ocw_world::Route::from_points(
            "t",
            "T",
            vec![[0.0, 0.0, 0.0], [0.0, 10.0, 1000.0]],
            false,
            6.0,
        )
        .unwrap();
        RideView::new(route)
    }

    fn rider() -> crate::RiderView {
        crate::RiderView {
            id: ocw_core::RiderId(1),
            name: "Ada Rider".into(),
            state: ocw_sim::RiderState {
                distance: 500.0,
                speed: 10.0,
                ..Default::default()
            },
            power_w: 250.0,
            cadence_rpm: Some(88.0),
            heart_rate_bpm: None,
            is_local: true,
            bike: ProfileId::Road,
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
            damage: None,
            cue: Default::default(),
            signs: Default::default(),
        }
    }

    #[test]
    fn the_screen_shows_the_ride() {
        let mut v = view();
        let s = screen_view(&v, &v.route, Units::Metric);
        assert_eq!(s.nav, "Connecting");
        v.elapsed_s = 3725.0;
        v.riders.push(rider());
        v.notice = Some("Dead end in 800 m".into());
        let s = screen_view(&v, &v.route, Units::Metric);
        assert_eq!(s.power, "250");
        assert_eq!(s.fields[0], ("36.0".into(), "km/h"));
        assert_eq!(s.fields[1], ("88".into(), "rpm"));
        assert_eq!(s.fields[2], ("–".into(), "bpm"));
        assert_eq!(s.fields[3], ("+1.0".into(), "%"));
        assert_eq!(s.fields[4], ("0.50".into(), "km"));
        assert_eq!(s.fields[5], ("1h02".into(), "time"));
        assert_eq!(s.leds, 1);
        assert_eq!(s.band, Some(GradeBand::Flat));
        assert_eq!(s.toast.as_deref(), Some("Dead end in 800 m"));
        assert_eq!(s.turn_deg, None);
        let s = screen_view(&v, &v.route, Units::Imperial);
        assert_eq!(s.fields[0].1, "mph");
        assert_eq!(s.fields[4].1, "mi");
    }

    /// Every field fits its width: five characters of the 76 px font.
    #[test]
    fn times_and_distances_stay_short() {
        assert_eq!(clock("☼ 18:42 x600"), "☼ 18:42");
        assert_eq!(clock("◐ 13:00 fixed"), "◐ 13:00");
        assert_eq!(clock("☼ 07:05"), "☼ 07:05");
        assert_eq!(ride_time(35.0), "00:35");
        assert_eq!(ride_time(3599.0), "59:59");
        assert_eq!(ride_time(3725.0), "1h02");
        assert_eq!(ride_time(-3.0), "00:00");
        assert_eq!(short_distance(0.22), "0.22");
        assert_eq!(short_distance(123.456), "123.5");
        for t in [0.0, 59.0, 3599.0, 36_000.0, 359_999.0] {
            assert!(ride_time(t).chars().count() <= 5, "{t}");
        }
    }

    #[test]
    fn the_next_turn_has_its_arrow_and_distance() {
        let mut v = view();
        v.riders.push(rider());
        v.junction = Some(crate::JunctionView {
            distance_m: 320.4,
            exits: vec![
                crate::ExitView {
                    angle_deg: -80.0,
                    name: "Bergstrasse".into(),
                    dead_end: false,
                    unpaved: false,
                    steep: false,
                },
                crate::ExitView {
                    angle_deg: 15.0,
                    name: "Talweg along the river".into(),
                    dead_end: false,
                    unpaved: false,
                    steep: false,
                },
            ],
            selected: 1,
            default: 1,
            locked: false,
            route: None,
        });
        let s = screen_view(&v, &v.route, Units::Metric);
        assert_eq!(s.turn_deg, Some(15.0));
        assert_eq!(s.nav, "320 m");
        assert_eq!(s.nav_detail, "Talweg alo…");
        assert!(s.nav_detail.chars().count() <= 11);
    }

    /// Every bike of the game gets the unit on its bar: ahead of the bar
    /// (or the TT extensions' tips), readable at the default size from the
    /// rider's steady eye.
    #[test]
    fn the_unit_fits_every_bike() {
        let rider = ocw_assets::shipped::rider_model();
        let rig = ocw_assets::pose::RiderRig::from_model(rider).unwrap();
        for p in ProfileId::ALL {
            let bike = ocw_assets::shipped::bike_rig(p);
            let bar = bar_mount(&bike);
            let body = ocw_assets::pose::BodyFit {
                scale: 1.0,
                rider: rig,
                bike,
                fit: ocw_assets::pose::Fit::new(&rig, &bike),
                saddle_offset: 0.0,
            };
            let eye = ocw_assets::pose::steady_eye(&body, &Default::default());
            let pl = computer::place(&bar, eye, &ComputerSetup::default());
            let front = match bar.aero_tips {
                Some(t) => t.z - 0.02,
                None => bar.clamp.z - computer::BAR_RADIUS_M,
            };
            for c in pl.corners() {
                assert!(c.z < front, "{p:?}: {c} behind {front}");
            }
            let deg = pl.glyph_deg(eye, computer::MIN_FONT_PX);
            assert!(deg >= computer::LEGIBLE_DEG, "{p:?}: {deg}°");
            // Below the eye, in front of it.
            let (c, _) = pl.screen();
            assert!(c.y < eye.y - 0.2 && c.z < eye.z, "{p:?}: {c} eye {eye}");
            assert_eq!(bar.aero_tips.is_some(), p == ProfileId::TimeTrial);
        }
    }

    /// The model file is there and is the generator's.
    #[test]
    fn the_model_file_is_the_generators() {
        assert_eq!(format!("{MODEL}.glb"), ocw_assets::computer::FILE);
        let m = ocw_assets::library::library().get(MODEL).expect("in the library");
        assert!(m.position(ocw_assets::computer::SCREEN_ANCHOR).is_some());
        let parts = meshes_of(&m);
        assert!(parts.iter().any(|(_, s, _)| s == "computer_led"));
        assert!(parts.iter().any(|(_, s, _)| s == "computer_body"));
    }
}

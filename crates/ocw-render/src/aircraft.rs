//! The simulated aircraft in the sky (spec 14 Phase 18, 18C, 18E T8):
//! what [`RideView::air`] holds – the airliners, small planes,
//! helicopters, gliders and balloons of `ocw_sim::air` – drawn with their
//! model files (T6, `artwork/3dmodels/vehicles/`).
//!
//! - **Level of detail:** the near model close by, its `lod1` beyond
//!   ([`lod_m`]), and far away the `lod1` kept at least
//!   [`MIN_PX`] pixels big – an airliner at cruise is a lit dot with its
//!   trail, not nothing; no new textures (the WebGL 2 budget, the iPhone's
//!   memory).
//! - **Lights:** the navigation lights (red left, green right, white
//!   tail), the red beacon and the white strobes flash at dusk and at
//!   night as glowing dots of a few pixels, the balloon's burner when it
//!   fires.
//! - **Moving parts:** propellers and rotors spin with the aircraft's
//!   rotor speed; an airliner's gear is down near the ground only.
//! - **Contrails:** one ribbon mesh for all trails, facing the camera,
//!   widening and fading with age, tinted by the low sun, without fog –
//!   seen above the haze as in life.
//!
//! - **Live aircraft** (18E T10) are drawn the same way; while they are,
//!   their source's credit ([`AirView::attribution`], "Aircraft data:
//!   adsb.lol (ODbL)") stands small in the lower left corner, above the
//!   place line (the lower right holds the map).
//!
//! Aircraft are never shadow casters and never touch anything.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::{NoFrustumCulling, VisibilityRange};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ocw_assets::model::Model;
use ocw_sim::air::upper::Contrail;
use ocw_sim::air::{AirKind, AircraftNow, Heard};

use crate::RideView;
use crate::camera::FollowCamera;

/// What the sky's aircraft are this frame (written by the client, spec
/// 18A: the rider's own view, `traffic.air`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AirView {
    /// The aircraft drawn (the nearest within the quality's budget).
    pub aircraft: Vec<AircraftNow>,
    /// The aircraft heard, each from where its sound now arriving left
    /// (empty with `traffic.sound off`).
    pub heard: Vec<Heard>,
    /// Where the listener is and which way it faces (x, z).
    pub listener: [f64; 3],
    pub listener_forward: [f64; 2],
    /// The live data's credit while live aircraft are drawn (ODbL:
    /// attribution); `None` for the simulated sky.
    pub attribution: Option<String>,
}

/// Never smaller than this on the screen (px).
pub const MIN_PX: f32 = 3.0;
/// The light dots' size on the screen (px).
pub const DOT_PX: f32 = 2.2;

/// How much an aircraft of `size_m` is enlarged at `dist` m with `focal`
/// px per radian: never smaller than [`MIN_PX`] on the screen, and true
/// size (1) whenever it is that big anyway – up close and well beyond the
/// switch to `lod1` – so only far dots grow.
pub fn draw_scale(dist: f32, focal: f32, size_m: f32) -> f32 {
    (MIN_PX * dist.max(1.0) / focal / size_m).clamp(1.0, 80.0)
}

/// Where a kind switches to `lod1` (m): aircraft are big and far.
pub fn lod_m(kind: AirKind) -> f32 {
    match kind {
        AirKind::Airliner => 900.0,
        AirKind::Balloon => 500.0,
        _ => 300.0,
    }
}

/// How many aircraft and whether contrails, by the quality level (18A's
/// budget table).
pub fn budget(level: crate::quality::QualityLevel) -> (usize, bool) {
    use crate::quality::QualityLevel as Q;
    match level {
        Q::High => (30, true),
        Q::Medium | Q::Vr => (20, true),
        Q::Low => (10, true),
        Q::Quest1 => (6, false),
    }
}

pub struct AircraftPlugin;

impl Plugin for AircraftPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AirModels>()
            .init_resource::<Planes>()
            .add_systems(
                PostUpdate,
                (sync_aircraft, contrails)
                    .chain()
                    .before(TransformSystems::Propagate),
            )
            .add_systems(Startup, spawn_credit)
            .add_systems(Update, (dock_credit, update_credit).chain());
    }
}

/// The live data's credit on the screen.
#[derive(Component)]
struct AirCredit;

fn spawn_credit(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            bottom: px(40),
            left: px(12),
            ..default()
        },
        Text::new(""),
        TextFont {
            font_size: 14.0.into(),
            ..default()
        },
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.7)),
        Visibility::Hidden,
        AirCredit,
    ));
}

/// The credit's widest line on a compact HUD (UI px): it wraps there, as
/// wide as the compact elevation profile below it.
fn credit_width(layout: Option<&crate::hud_fit::HudLayout>) -> Val {
    match layout {
        Some(l) if l.compact => px(crate::hud::profile_size(l.ui_px, true).x.max(120.0)),
        _ => Val::Auto,
    }
}

/// Puts the credit at the top of the HUD's bottom-left column, above the
/// elevation profile (spec 18J; before, it stood at a fixed 40 px from the
/// bottom and covered the profile): the column's children flow, so it
/// never overlaps the profile or the GPS line, on any window size. Without
/// a HUD it keeps its own place, bottom left.
fn dock_credit(
    mut commands: Commands,
    layout: Option<Res<crate::hud_fit::HudLayout>>,
    column: Query<Entity, With<crate::hud::HudBottomLeft>>,
    mut credit: Query<(Entity, &mut Node, Has<ChildOf>), With<AirCredit>>,
) {
    let Ok(column) = column.single() else { return };
    for (e, mut node, docked) in &mut credit {
        if !docked {
            node.position_type = PositionType::Relative;
            node.bottom = Val::Auto;
            node.left = Val::Auto;
            commands.entity(column).insert_children(0, &[e]);
        }
        let width = credit_width(layout.as_deref());
        if node.max_width != width {
            node.max_width = width;
        }
    }
}

/// The credit text of a view (`None`: hidden).
pub fn credit(view: Option<&AirView>) -> Option<&str> {
    view?.attribution.as_deref().filter(|a| !a.is_empty())
}

fn update_credit(
    view: Option<Res<RideView>>,
    mut q: Query<(&mut Text, &mut Visibility), With<AirCredit>>,
) {
    let want = view.as_deref().and_then(|v| credit(v.air.as_ref()));
    for (mut text, mut vis) in &mut q {
        // Visible, not inherited: the ODbL credit stays while the HUD is
        // hidden (`hud off`), at its place in the column.
        let shown = if want.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *vis != shown {
            *vis = shown;
        }
        if let Some(c) = want
            && text.0 != c
        {
            text.0 = c.to_owned();
        }
    }
}

/// A lamp of an aircraft.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Lamp {
    Red,
    Green,
    White,
    Beacon,
    Strobe,
    Burner,
}

impl Lamp {
    fn of(name: &str) -> Option<Self> {
        Some(match name {
            "light_nav_l" => Self::Red,
            "light_nav_r" => Self::Green,
            "light_tail" => Self::White,
            "light_beacon" => Self::Beacon,
            n if n.starts_with("light_strobe") => Self::Strobe,
            "light_burner" => Self::Burner,
            _ => return None,
        })
    }

    fn color(self) -> LinearRgba {
        match self {
            Self::Red | Self::Beacon => LinearRgba::rgb(30.0, 1.0, 0.6),
            Self::Green => LinearRgba::rgb(0.8, 24.0, 4.0),
            Self::White | Self::Strobe => LinearRgba::rgb(24.0, 24.0, 22.0),
            Self::Burner => LinearRgba::rgb(40.0, 16.0, 3.0),
        }
    }

    /// On at time `t` (s): the beacon 1 Hz, strobes a double flash every
    /// 1.2 s, the rest steady.
    fn lit(self, t: f64, phase: f64) -> bool {
        let t = t + phase;
        match self {
            Self::Beacon => (t * 1.0).fract() < 0.12,
            Self::Strobe => {
                let f = (t / 1.2).fract();
                f < 0.04 || (0.1..0.14).contains(&f)
            }
            _ => true,
        }
    }
}

/// A mesh part with its slot colour.
#[derive(Clone)]
struct Part {
    mesh: Handle<Mesh>,
    slot: String,
    color: [f32; 4],
}

/// A spinning part: its pivot in the model and its axis.
#[derive(Clone)]
struct Spinner {
    pivot: Transform,
    axis: Vec3,
    /// Revolutions per second at full speed (the real rate, v7.08).
    rate: f32,
    blades: u32,
    parts: Vec<Part>,
    /// The motion-blur disc in the pivot's frame (drawn as the blades
    /// blur, [`disc_blend`]).
    disc: Option<Handle<Mesh>>,
}

/// A landing-gear leg (`gear_*`): it swings about `pivot` (the leg's
/// top) by `up_angle` about `axis` into the bay as it retracts.
#[derive(Clone)]
struct Leg {
    pivot: Transform,
    axis: Vec3,
    up_angle: f32,
    parts: Vec<Part>,
}

/// Seconds the gear takes to swing in or out (v7.08).
pub const GEAR_TRAVEL_S: f32 = 5.0;

/// A spinner's real rate (revolutions per second in flight) and its
/// blades, by model and node (v7.08; the user, 2026-11-28: "the rotor of
/// a helicopter spins way faster"): a light helicopter's main rotor
/// ~450 rpm and its tail rotor ~2,000, the medium transport helicopter's
/// ~215 and ~1,150, a piston plane's propeller ~2,400 rpm.
pub fn spin_rate(model: &str, node: &str) -> (f32, u32) {
    let medium = model.contains("medium");
    match node {
        "prop" => (40.0, 2),
        "rotor_main" if medium => (3.6, 5),
        "rotor_main" => (7.5, 3),
        "rotor_tail" if medium => (19.0, 3),
        "rotor_tail" => (33.0, 2),
        _ => (5.0, 2),
    }
}

/// How much a rotor of `blades` turning `rev_s` is drawn as its blur disc
/// (0: solid blades … 1: the disc only): by the blades' passing rate
/// alone – solid under 6 Hz, the disc from 15 Hz – so it looks the same at
/// any video or frame rate (at 30 or 60 fps such rates alias: the blades
/// would seem slow, still or turning backwards).
pub fn disc_blend(rev_s: f32, blades: u32) -> f32 {
    let pass = rev_s.abs() * blades as f32;
    ((pass - 6.0) / 9.0).clamp(0.0, 1.0)
}

/// The rotor's angle after `dt` seconds at `rev_s` (time-based, so the
/// same after a second at any frame rate).
pub fn spin_step(angle: f32, dt: f32, rev_s: f32) -> f32 {
    (angle + dt * rev_s * std::f32::consts::TAU).rem_euclid(std::f32::consts::TAU)
}

/// The blur disc of a rotor of radius `r` about `axis` (in its pivot's
/// frame): an annulus of soft radial streaks in `color`, its rim a little
/// darker, coned up by `cone` (rad) as a main rotor in flight.
fn disc_mesh(axis: Vec3, r: f32, color: [f32; 4], blades: u32, cone: f32) -> Mesh {
    let n = 48u32;
    let u = if axis.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
    let v = axis.cross(u).normalize();
    let u = v.cross(axis).normalize();
    let mut pos = Vec::new();
    let mut col = Vec::new();
    let mut idx = Vec::new();
    let rings = [0.12f32, 0.55, 0.97, 1.0];
    let alpha = [0.18f32, 0.5, 0.62, 0.85];
    // A blur reads as a grey veil of the blades' dark colour.
    let color = [
        color[0] * 0.5 + 0.05,
        color[1] * 0.5 + 0.05,
        color[2] * 0.5 + 0.05,
        1.0,
    ];
    for k in 0..n {
        let th = k as f32 / n as f32 * std::f32::consts::TAU;
        let streak = 0.75 + 0.25 * (th * blades as f32 * 2.0).sin();
        for (ri, rf) in rings.iter().enumerate() {
            let rr = r * rf;
            pos.push(((u * th.cos() + v * th.sin()) * rr + axis * (rr * cone.tan())).to_array());
            col.push([color[0], color[1], color[2], alpha[ri] * streak]);
        }
    }
    let m = rings.len() as u32;
    for k in 0..n {
        let k2 = (k + 1) % n;
        for ri in 0..m - 1 {
            let (a, b, c, d) = (k * m + ri, k2 * m + ri, k2 * m + ri + 1, k * m + ri + 1);
            idx.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    let nor = vec![axis.to_array(); pos.len()];
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
    mesh.insert_indices(Indices::U32(idx));
    mesh
}

/// One model's meshes (a kind has one or more, 18G).
#[derive(Clone)]
struct KindModel {
    near: Vec<Part>,
    far: Vec<Part>,
    gear: Vec<Leg>,
    spinners: Vec<Spinner>,
    lamps: Vec<(Vec3, Lamp)>,
    /// The model's largest extent (m).
    size_m: f32,
}

#[derive(Resource, Default)]
struct AirModels {
    /// By model id ([`AircraftNow::model_id`]).
    kinds: HashMap<&'static str, KindModel>,
    materials: HashMap<(String, [u32; 4], bool), Handle<StandardMaterial>>,
    dot: Option<Handle<Mesh>>,
    dots: HashMap<Lamp, Handle<StandardMaterial>>,
    trail: Option<(Handle<Mesh>, Handle<StandardMaterial>)>,
}

#[derive(Resource, Default)]
struct Planes {
    roots: HashMap<u64, Entity>,
    trail: Option<Entity>,
}

#[derive(Component)]
struct Plane {
    /// The attitude of the last frame (kept while standing).
    last: Quat,
}

#[derive(Component)]
struct Spin {
    pivot: Transform,
    axis: Vec3,
    rate: f32,
    angle: f32,
    owner: u64,
}

/// The solid blades of a spinner (hidden once its disc takes over).
#[derive(Component)]
struct Blades {
    owner: u64,
    rate: f32,
    blades: u32,
}

/// A spinner's blur disc and its own material (its alpha the blend).
#[derive(Component)]
struct Disc {
    owner: u64,
    rate: f32,
    blades: u32,
    material: Handle<StandardMaterial>,
    /// The alpha set last.
    alpha: f32,
}

#[derive(Component)]
struct GearPart {
    owner: u64,
    pivot: Transform,
    axis: Vec3,
    up_angle: f32,
    /// 0 up … 1 down.
    ext: f32,
}

impl GearPart {
    /// The leg swung `up_angle` × (1 − ext) about its top.
    fn transform(&self) -> Transform {
        self.pivot
            * Transform::from_rotation(Quat::from_axis_angle(
                self.axis,
                self.up_angle * (1.0 - self.ext),
            ))
    }
}

/// The gear's extension (0 up … 1 down) after `dt` s moving towards
/// `down` over [`GEAR_TRAVEL_S`] (eased by the caller's transform).
pub fn gear_travel(ext: f32, down: bool, dt: f32) -> f32 {
    let step = dt / GEAR_TRAVEL_S;
    if down {
        (ext + step).min(1.0)
    } else {
        (ext - step).max(0.0)
    }
}

/// A blur disc's material: unlit, alpha-blended, both sides, its vertex
/// colours (the blades' colour and streaks) times `alpha`.
fn disc_material(alpha: f32) -> StandardMaterial {
    StandardMaterial {
        base_color: Color::linear_rgba(1.0, 1.0, 1.0, alpha),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        cull_mode: None,
        double_sided: true,
        fog_enabled: false,
        ..default()
    }
}

#[derive(Component)]
struct Dot {
    lamp: Lamp,
    owner: u64,
    at: Vec3,
}

#[derive(Component)]
struct TrailRibbon;

/// The group a node's meshes belong to.
#[derive(Debug, Clone, PartialEq)]
enum Group {
    Body,
    Far,
    Gear(usize),
    Spin(usize),
}

pub(crate) fn mesh_of(
    p: &ocw_assets::model::Primitive,
    xf: bevy::math::Affine3A,
    shade: f32,
) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let pos: Vec<[f32; 3]> = p
        .positions
        .iter()
        .map(|v| xf.transform_point3(Vec3::from(*v)).to_array())
        .collect();
    let nor: Vec<[f32; 3]> = p
        .normals
        .iter()
        .map(|v| {
            xf.transform_vector3(Vec3::from(*v))
                .normalize_or_zero()
                .to_array()
        })
        .collect();
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
    if p.colors.len() == p.positions.len() {
        let c: Vec<[f32; 4]> = p
            .colors
            .iter()
            .map(|c| ocw_assets::model::shaded(*c, shade))
            .collect();
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, c);
    }
    m.insert_indices(Indices::U32(p.indices.clone()));
    m
}

/// The meshes of one model file, grouped.
fn kind_model(id: &str, model: &Model, meshes: &mut Assets<Mesh>) -> KindModel {
    let lod = model.node("lod1");
    let spin_names = [
        ("prop", Vec3::Z),
        ("rotor_main", Vec3::Y),
        ("rotor_tail", Vec3::X),
    ];
    let mut spinners: Vec<Spinner> = spin_names
        .iter()
        .filter_map(|(n, axis)| {
            let i = model.node(n)?;
            let w = model.world(i);
            let (rate, blades) = spin_rate(id, n);
            Some(Spinner {
                pivot: Transform::from_matrix(Mat4::from(w)),
                axis: *axis,
                rate,
                blades,
                parts: Vec::new(),
                disc: None,
            })
        })
        .collect();
    let spin_nodes: Vec<usize> = spin_names
        .iter()
        .filter_map(|(n, _)| model.node(n))
        .collect();
    let mut radius = vec![0.0f32; spin_nodes.len()];
    let mut blade_color = vec![[0.1f32, 0.1, 0.1, 1.0]; spin_nodes.len()];
    // The gear legs: the outermost `gear_*` nodes of the near model.
    let leg_nodes: Vec<usize> = (0..model.nodes.len())
        .filter(|&i| {
            model.nodes[i].name.starts_with("gear_")
                && !lod.is_some_and(|l| model.is_below(i, l))
                && model.nodes[i]
                    .parent
                    .is_none_or(|p| !model.under_named(p, |s| s.starts_with("gear_")))
        })
        .collect();
    let mut legs: Vec<Leg> = leg_nodes
        .iter()
        .map(|&g| {
            let at = Vec3::from(model.world(g).translation);
            let top = model
                .bounds_where(|i| model.is_below(i, g))
                .map_or(at.y, |(_, hi)| hi.y);
            let name = model.nodes[g].name.as_str();
            // Nose legs fold forward, main legs inwards.
            let (axis, up_angle) = if name.contains("nose") {
                (Vec3::X, std::f32::consts::FRAC_PI_2)
            } else if at.x < -0.01 {
                (Vec3::Z, std::f32::consts::FRAC_PI_2)
            } else if at.x > 0.01 {
                (Vec3::Z, -std::f32::consts::FRAC_PI_2)
            } else {
                (Vec3::X, 0.0)
            };
            Leg {
                pivot: Transform::from_translation(Vec3::new(at.x, top, at.z)),
                axis,
                up_angle,
                parts: Vec::new(),
            }
        })
        .collect();
    let mut out = KindModel {
        near: Vec::new(),
        far: Vec::new(),
        gear: Vec::new(),
        spinners: Vec::new(),
        lamps: Vec::new(),
        size_m: 10.0,
    };
    let shade = model.shade_scale();
    for (i, n) in model.nodes.iter().enumerate() {
        let far = lod.is_some_and(|l| model.is_below(i, l));
        if !far && let Some(l) = Lamp::of(&n.name) {
            let p = model.world(i).translation;
            out.lamps.push((Vec3::from(p), l));
        }
        if n.primitives.is_empty() {
            continue;
        }
        let group = if far {
            Group::Far
        } else if let Some(k) = spin_nodes.iter().position(|&s| model.is_below(i, s)) {
            Group::Spin(k)
        } else if let Some(k) = leg_nodes.iter().position(|&g| model.is_below(i, g)) {
            Group::Gear(k)
        } else {
            Group::Body
        };
        let world = model.world(i);
        let xf = match &group {
            Group::Spin(k) => {
                let pivot = model.world(spin_nodes[*k]);
                pivot.inverse() * world
            }
            Group::Gear(k) => {
                bevy::math::Affine3A::from_translation(-legs[*k].pivot.translation) * world
            }
            _ => world,
        };
        for p in &n.primitives {
            let mat = &model.materials[p.material];
            let part = Part {
                mesh: meshes.add(mesh_of(p, xf, shade)),
                slot: mat.name.clone(),
                color: mat.color,
            };
            match &group {
                Group::Body => out.near.push(part),
                Group::Far => out.far.push(part),
                Group::Gear(k) => legs[*k].parts.push(part),
                Group::Spin(k) => {
                    let axis = spinners[*k].axis;
                    for v in &p.positions {
                        let q = xf.transform_point3(Vec3::from(*v));
                        let r = (q - axis * q.dot(axis)).length();
                        if r > radius[*k] {
                            radius[*k] = r;
                            if !ocw_assets::slots::LIGHTS.contains(&mat.name.as_str()) {
                                blade_color[*k] = mat.color;
                            }
                        }
                    }
                    spinners[*k].parts.push(part);
                }
            }
        }
    }
    if let Some((lo, hi)) = model.near_bounds() {
        out.size_m = (hi - lo).max_element().max(1.0);
    }
    for (k, s) in spinners.iter_mut().enumerate() {
        if radius[k] > 0.1 {
            let cone = if s.axis == Vec3::Y { 0.05 } else { 0.0 };
            s.disc = Some(meshes.add(disc_mesh(s.axis, radius[k], blade_color[k], s.blades, cone)));
        }
    }
    out.spinners = spinners
        .into_iter()
        .filter(|s| !s.parts.is_empty())
        .collect();
    out.gear = legs.into_iter().filter(|l| !l.parts.is_empty()).collect();
    out
}

/// A made-up airline's or owner's colour (no real company).
const LIVERIES: [u32; 10] = [
    0x284B8C, 0xB3262E, 0x1E7A4C, 0xE0A21B, 0x5B2C83, 0x0F8A9D, 0xD2582A, 0x3A3F47, 0x8C1D4F,
    0x2E6FD0,
];

/// The colour of `slot` on aircraft `kind` with look seed `look`.
fn paint(kind: AirKind, slot: &str, model: [f32; 4], look: u32) -> [f32; 4] {
    let pick = |salt: u32| {
        let i = (look.rotate_left(salt) as usize) % LIVERIES.len();
        ocw_assets::model::srgb(LIVERIES[i])
    };
    match (kind, slot) {
        (AirKind::Airliner, "livery") => pick(0),
        (AirKind::SmallPlane, "livery") | (AirKind::Glider, "livery") => pick(3),
        (AirKind::Balloon, "body") => pick(5),
        (AirKind::Balloon, "livery") => pick(11),
        _ => model,
    }
}

fn material(
    models: &mut AirModels,
    materials: &mut Assets<StandardMaterial>,
    slot: &str,
    color: [f32; 4],
) -> Handle<StandardMaterial> {
    let lamp = ocw_assets::slots::LIGHTS.contains(&slot);
    let key = (slot.to_string(), color.map(f32::to_bits), lamp);
    models
        .materials
        .entry(key)
        .or_insert_with(|| {
            let s = ocw_assets::slots::get(slot);
            let base = Color::linear_rgba(color[0], color[1], color[2], color[3]);
            materials.add(StandardMaterial {
                base_color: base,
                metallic: s.map_or(0.0, |s| s.metallic),
                perceptual_roughness: s.map_or(0.6, |s| s.roughness),
                emissive: if lamp {
                    base.to_linear() * 3.0
                } else {
                    LinearRgba::BLACK
                },
                fog_enabled: false,
                ..default()
            })
        })
        .clone()
}

fn ensure_models(
    models: &mut AirModels,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) {
    if !models.kinds.is_empty() {
        return;
    }
    for k in AirKind::ALL {
        for &id in k.model_ids() {
            let m = ocw_assets::shipped::model(id);
            models.kinds.insert(id, kind_model(id, &m, meshes));
        }
    }
    models.dot = Some(meshes.add(Sphere::new(1.0).mesh().ico(1).expect("ico sphere")));
    for l in [
        Lamp::Red,
        Lamp::Green,
        Lamp::White,
        Lamp::Beacon,
        Lamp::Strobe,
        Lamp::Burner,
    ] {
        let h = materials.add(dot_material(l));
        models.dots.insert(l, h);
    }
}

/// A light dot's material: unlit, so it draws its base colour – which
/// must be the lamp's bright colour (HDR, for the bloom), not black (the
/// dots were dark until v6.96: an unlit material ignores its emissive);
/// the emissive for the lit paths.
fn dot_material(l: Lamp) -> StandardMaterial {
    let c = l.color();
    StandardMaterial {
        base_color: Color::linear_rgb(c.red / 4.0, c.green / 4.0, c.blue / 4.0),
        emissive: c,
        unlit: true,
        fog_enabled: false,
        ..default()
    }
}

/// The camera's eye and its pixels per radian (vertical).
pub(crate) fn eye_and_focal(
    cams: &Query<(&Camera, &GlobalTransform, &Projection), With<FollowCamera>>,
) -> Option<(Vec3, f32)> {
    let (cam, t, proj) = cams.iter().next()?;
    let h = cam.logical_viewport_size().map_or(720.0, |s| s.y);
    let fov = match proj {
        Projection::Perspective(p) => p.fov,
        _ => 0.8,
    };
    Some((t.translation(), h / (2.0 * (fov / 2.0).tan())))
}

/// The aircraft's attitude: along its velocity with its bank (balloons
/// upright, helicopters nose down a little in forward flight).
fn attitude(a: &AircraftNow, last: Quat) -> Quat {
    let v = Vec3::new(a.vel[0] as f32, a.vel[1] as f32, a.vel[2] as f32);
    let flat = v.with_y(0.0);
    if a.kind == AirKind::Balloon {
        return Quat::from_rotation_y((a.id % 628) as f32 / 100.0);
    }
    if flat.length() < 0.5 {
        // Standing or hovering straight up: keep the heading.
        let (yaw, _, _) = last.to_euler(EulerRot::YXZ);
        return Quat::from_rotation_y(yaw);
    }
    let dir = match a.kind {
        AirKind::Helicopter => {
            let speed = flat.length();
            let pitch = -(speed / 60.0).min(1.2) * 0.1;
            (flat.normalize() + Vec3::Y * pitch).normalize()
        }
        _ if a.on_ground => flat.normalize(),
        _ => v.normalize(),
    };
    let nose_up = match a.kind {
        AirKind::Airliner if !a.on_ground && v.length() < 110.0 => 0.05,
        _ => 0.0,
    };
    let look = Transform::default().looking_to(dir, Vec3::Y).rotation;
    look * Quat::from_rotation_x(nose_up) * Quat::from_rotation_z(-(a.bank as f32))
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_aircraft(
    mut commands: Commands,
    view: Res<RideView>,
    time: Res<Time>,
    mut models: ResMut<AirModels>,
    mut planes: ResMut<Planes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cams: Query<(&Camera, &GlobalTransform, &Projection), With<FollowCamera>>,
    mut roots: Query<
        (&mut Plane, &mut Transform),
        (Without<Spin>, Without<Dot>, Without<GearPart>),
    >,
    mut spins: Query<
        (&mut Spin, &mut Transform),
        (Without<Plane>, Without<Dot>, Without<GearPart>),
    >,
    mut gears: Query<
        (&mut GearPart, &mut Transform, &mut Visibility),
        (Without<Dot>, Without<Plane>, Without<Spin>),
    >,
    mut rotors: Query<
        (Option<&Blades>, Option<&mut Disc>, &mut Visibility),
        (
            Or<(With<Blades>, With<Disc>)>,
            Without<GearPart>,
            Without<Dot>,
        ),
    >,
    mut dots: Query<
        (&Dot, &mut Transform, &mut Visibility),
        (Without<Plane>, Without<Spin>, Without<GearPart>),
    >,
) {
    let list = view.air.as_ref().map_or(&[][..], |a| &a.aircraft[..]);
    // Gone: despawn.
    let alive: std::collections::HashSet<u64> = list.iter().map(|a| a.id).collect();
    planes.roots.retain(|id, e| {
        let keep = alive.contains(id);
        if !keep && let Ok(mut c) = commands.get_entity(*e) {
            c.despawn();
        }
        keep
    });
    if list.is_empty() {
        return;
    }
    ensure_models(&mut models, &mut meshes, &mut materials);
    let Some((eye, focal)) = eye_and_focal(&cams) else {
        return;
    };
    let sun = view
        .sky
        .as_ref()
        .map_or(30.0, |s| s.sky.sun.elevation_deg as f32);
    let night = sun < 6.0;
    let t = time.elapsed_secs_f64();
    let mut info: HashMap<u64, (&AircraftNow, f32)> = HashMap::new();
    for a in list {
        let pos = Vec3::new(a.pos[0] as f32, a.pos[1] as f32, a.pos[2] as f32);
        let km = models
            .kinds
            .get(a.model_id())
            .cloned()
            .expect("every model");
        let scale = draw_scale(eye.distance(pos), focal, km.size_m);
        info.insert(a.id, (a, scale));
        if let Some(&e) = planes.roots.get(&a.id) {
            if let Ok((mut p, mut tf)) = roots.get_mut(e) {
                let rot = attitude(a, p.last);
                p.last = rot;
                *tf = Transform::from_translation(pos)
                    .with_rotation(rot)
                    .with_scale(Vec3::splat(scale));
            }
            continue;
        }
        // New: the root and its parts.
        let lod = lod_m(a.kind);
        let near_range = VisibilityRange {
            start_margin: 0.0..0.0,
            end_margin: lod * 0.95..lod * 1.05,
            use_aabb: false,
        };
        let far_range = VisibilityRange {
            start_margin: lod * 0.95..lod * 1.05,
            end_margin: 1.0e7..1.0e7,
            use_aabb: false,
        };
        let rot = attitude(a, Quat::IDENTITY);
        let root = commands
            .spawn((
                Plane { last: rot },
                Transform::from_translation(pos)
                    .with_rotation(rot)
                    .with_scale(Vec3::splat(scale)),
                Visibility::default(),
            ))
            .id();
        let mut kids = Vec::new();
        let disc_mats: Vec<Option<Handle<StandardMaterial>>> = km
            .spinners
            .iter()
            .map(|s| s.disc.as_ref().map(|_| materials.add(disc_material(0.0))))
            .collect();
        let mut part = |commands: &mut Commands, p: &Part, range: &VisibilityRange| {
            let c = paint(a.kind, &p.slot, p.color, a.look);
            let m = material(&mut models, &mut materials, &p.slot, c);
            commands
                .spawn((
                    Mesh3d(p.mesh.clone()),
                    MeshMaterial3d(m),
                    Transform::default(),
                    range.clone(),
                    NotShadowCaster,
                ))
                .id()
        };
        for p in &km.near {
            kids.push(part(&mut commands, p, &near_range));
        }
        for p in &km.far {
            kids.push(part(&mut commands, p, &far_range));
        }
        for leg in &km.gear {
            let ext = if a.gear_down { 1.0 } else { 0.0 };
            let g = GearPart {
                owner: a.id,
                pivot: leg.pivot,
                axis: leg.axis,
                up_angle: leg.up_angle,
                ext,
            };
            let pivot = commands
                .spawn((
                    g.transform(),
                    if a.gear_down {
                        Visibility::Inherited
                    } else {
                        Visibility::Hidden
                    },
                    g,
                ))
                .id();
            for p in &leg.parts {
                let e = part(&mut commands, p, &near_range);
                commands.entity(pivot).add_child(e);
            }
            kids.push(pivot);
        }
        for (s, disc_mat) in km.spinners.iter().zip(disc_mats) {
            let pivot = commands
                .spawn((
                    s.pivot,
                    Visibility::default(),
                    Spin {
                        pivot: s.pivot,
                        axis: s.axis,
                        rate: s.rate,
                        angle: 0.0,
                        owner: a.id,
                    },
                ))
                .id();
            let blades = commands
                .spawn((
                    Transform::default(),
                    Visibility::default(),
                    Blades {
                        owner: a.id,
                        rate: s.rate,
                        blades: s.blades,
                    },
                ))
                .id();
            commands.entity(pivot).add_child(blades);
            for p in &s.parts {
                let e = part(&mut commands, p, &near_range);
                commands.entity(blades).add_child(e);
            }
            kids.push(pivot);
            // The blur disc stands still at the pivot (a turning streak
            // pattern would alias too).
            if let (Some(mesh), Some(material)) = (&s.disc, disc_mat) {
                kids.push(
                    commands
                        .spawn((
                            Mesh3d(mesh.clone()),
                            MeshMaterial3d(material.clone()),
                            s.pivot,
                            Visibility::Hidden,
                            near_range.clone(),
                            NotShadowCaster,
                            Disc {
                                owner: a.id,
                                rate: s.rate,
                                blades: s.blades,
                                material,
                                alpha: 0.0,
                            },
                        ))
                        .id(),
                );
            }
        }
        let dot_mesh = models.dot.clone().expect("dot mesh");
        for (at, lamp) in &km.lamps {
            let mat = models.dots.get(lamp).cloned().expect("dot material");
            kids.push(
                commands
                    .spawn((
                        Mesh3d(dot_mesh.clone()),
                        MeshMaterial3d(mat),
                        Transform::from_translation(*at).with_scale(Vec3::splat(0.4)),
                        Visibility::Hidden,
                        NotShadowCaster,
                        Dot {
                            lamp: *lamp,
                            owner: a.id,
                            at: *at,
                        },
                    ))
                    .id(),
            );
        }
        commands.entity(root).add_children(&kids);
        planes.roots.insert(a.id, root);
    }
    // Spinning parts, gear, lights.
    let dt = time.delta_secs();
    for (mut s, mut tf) in &mut spins {
        let Some((a, _)) = info.get(&s.owner) else {
            continue;
        };
        s.angle = spin_step(s.angle, dt, s.rate * a.rotor as f32);
        *tf = s.pivot * Transform::from_rotation(Quat::from_axis_angle(s.axis, s.angle));
    }
    for (blades, disc, mut vis) in &mut rotors {
        let (owner, rate, n) = if let Some(b) = blades {
            (b.owner, b.rate, b.blades)
        } else if let Some(d) = disc.as_ref() {
            (d.owner, d.rate, d.blades)
        } else {
            continue;
        };
        let Some((a, _)) = info.get(&owner) else {
            continue;
        };
        let blend = disc_blend(rate * a.rotor as f32, n);
        let shown = if blades.is_some() {
            blend < 0.85
        } else {
            blend > 0.0
        };
        vis.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(mut d) = disc
            && (d.alpha - blend).abs() > 0.02
        {
            d.alpha = blend;
            if let Some(mut m) = materials.get_mut(&d.material) {
                m.base_color = Color::linear_rgba(1.0, 1.0, 1.0, blend);
            }
        }
    }
    for (mut g, mut tf, mut vis) in &mut gears {
        if let Some((a, _)) = info.get(&g.owner) {
            g.ext = gear_travel(g.ext, a.gear_down, dt);
            *tf = g.transform();
            vis.set_if_neq(if g.ext > 0.001 {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }
    for (d, mut tf, mut vis) in &mut dots {
        let Some((a, scale)) = info.get(&d.owner) else {
            continue;
        };
        let pos = Vec3::new(a.pos[0] as f32, a.pos[1] as f32, a.pos[2] as f32);
        let phase = (a.id % 1000) as f64 / 997.0;
        let on = match d.lamp {
            Lamp::Burner => a.burner,
            l => {
                night
                    && l.lit(t, phase)
                    && !(a.kind == AirKind::Glider || a.kind == AirKind::Balloon)
            }
        };
        vis.set_if_neq(if on {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if on {
            let dist = eye.distance(pos).max(1.0);
            let r = (DOT_PX * dist / focal).max(if d.lamp == Lamp::Burner { 0.8 } else { 0.25 });
            *tf = Transform::from_translation(d.at).with_scale(Vec3::splat(r / scale));
        }
    }
}

/// The trails' ribbon: rebuilt every frame from [`AirView`].
#[allow(clippy::too_many_arguments)]
fn contrails(
    mut commands: Commands,
    view: Res<RideView>,
    mut models: ResMut<AirModels>,
    mut planes: ResMut<Planes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cams: Query<(&Camera, &GlobalTransform, &Projection), With<FollowCamera>>,
    mut ribbons: Query<&mut Visibility, With<TrailRibbon>>,
) {
    let trails: Vec<_> = view
        .air
        .as_ref()
        .map(|a| a.aircraft.iter().filter_map(|a| a.trail.as_ref()).collect())
        .unwrap_or_default();
    if trails.is_empty() {
        if let Some(e) = planes.trail
            && let Ok(mut v) = ribbons.get_mut(e)
        {
            v.set_if_neq(Visibility::Hidden);
        }
        return;
    }
    let Some((eye, focal)) = eye_and_focal(&cams) else {
        return;
    };
    let (mesh, _) = models
        .trail
        .get_or_insert_with(|| {
            let mut m = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0f32; 3]; 3]);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 3]);
            m.insert_attribute(Mesh::ATTRIBUTE_COLOR, vec![[1.0f32; 4]; 3]);
            m.insert_indices(Indices::U32(vec![0, 1, 2]));
            let mat = materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                alpha_mode: AlphaMode::Blend,
                cull_mode: None,
                double_sided: true,
                fog_enabled: false,
                ..default()
            });
            (meshes.add(m), mat)
        })
        .clone();
    if planes.trail.is_none() {
        let mat = models.trail.as_ref().expect("trail").1.clone();
        planes.trail = Some(
            commands
                .spawn((
                    Mesh3d(mesh.clone()),
                    MeshMaterial3d(mat),
                    Transform::default(),
                    NoFrustumCulling,
                    NotShadowCaster,
                    TrailRibbon,
                    Visibility::default(),
                ))
                .id(),
        );
    } else if let Some(e) = planes.trail
        && let Ok(mut v) = ribbons.get_mut(e)
    {
        v.set_if_neq(Visibility::Inherited);
    }
    // The light on the trails: white by day, warm in a low sun, faint by
    // night (the moon).
    let sun = view
        .sky
        .as_ref()
        .map_or(30.0, |s| s.sky.sun.elevation_deg as f32);
    let day = ((sun + 6.0) / 10.0).clamp(0.08, 1.0);
    let warm = (1.0 - (sun / 12.0).clamp(0.0, 1.0)) * (sun > -4.0) as u8 as f32;
    let tint = Vec3::new(1.0, 1.0 - 0.25 * warm, 1.0 - 0.45 * warm) * day;
    let (pos, col, idx) = ribbon(&trails, eye, focal, tint);
    if let Some(mut m) = meshes.get_mut(&mesh) {
        let n = pos.len();
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
        m.insert_attribute(Mesh::ATTRIBUTE_COLOR, col);
        m.insert_indices(Indices::U32(idx));
    }
}

/// The trails' ribbon (positions, colours, triangles): two vertices per
/// point, facing the camera at `eye` (`focal` px per radian).
type Ribbon = (Vec<[f32; 3]>, Vec<[f32; 4]>, Vec<u32>);

fn ribbon(trails: &[&ocw_sim::air::Trail], eye: Vec3, focal: f32, tint: Vec3) -> Ribbon {
    let mut pos = Vec::new();
    let mut col = Vec::new();
    let mut idx = Vec::new();
    for tr in trails.iter().filter(|t| t.points.len() >= 2) {
        let life = tr.kind.lifetime_s() as f32;
        let pts: Vec<(Vec3, f32)> = tr
            .points
            .iter()
            .map(|p| {
                (
                    Vec3::new(p.pos[0] as f32, p.pos[1] as f32, p.pos[2] as f32),
                    p.age_s as f32,
                )
            })
            .collect();
        for (i, &(p, age)) in pts.iter().enumerate() {
            let next = pts
                .get(i + 1)
                .map_or_else(|| p - (pts[i - 1].0 - p), |n| n.0);
            let prev = if i > 0 { pts[i - 1].0 } else { p - (next - p) };
            let tangent = (next - prev).normalize_or(Vec3::X);
            let to_cam = (eye - p).normalize_or(Vec3::Y);
            let side = tangent.cross(to_cam).normalize_or(Vec3::Y);
            let grow = if tr.kind == Contrail::Persistent {
                0.12
            } else {
                0.5
            };
            let dist = eye.distance(p);
            let w = (3.0 + grow * age).max(1.6 * dist / focal);
            let fade = (1.0 - age / life).clamp(0.0, 1.0);
            let start = (age / 1.2).clamp(0.0, 1.0);
            let alpha = 0.7 * fade.powf(1.6) * start;
            let base = pos.len() as u32;
            pos.push((p + side * w * 0.5).to_array());
            pos.push((p - side * w * 0.5).to_array());
            let c = [tint.x, tint.y, tint.z, alpha];
            col.push(c);
            col.push(c);
            if i + 1 < pts.len() {
                idx.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 3, base + 2]);
            }
        }
    }
    (pos, col, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (the public server, 2026-11-27): the credit stood at a
    /// fixed 40 px from the bottom, over the elevation profile. It flows
    /// at the top of the bottom-left column now, above the profile and the
    /// GPS line (so it overlaps neither, at any size), wraps to the compact
    /// profile's width on a phone, and stays visible with the HUD hidden.
    #[test]
    fn the_credit_stands_above_the_elevation_profile() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .add_systems(Startup, (crate::hud::spawn_hud, spawn_credit))
            .add_systems(Update, (dock_credit, update_credit).chain());
        app.update();
        app.update();
        let w = app.world_mut();
        let credit = w
            .query_filtered::<Entity, With<AirCredit>>()
            .single(w)
            .unwrap();
        let column = w.get::<ChildOf>(credit).expect("docked").parent();
        assert!(w.get::<crate::hud::HudBottomLeft>(column).is_some());
        let kids: Vec<Entity> = w.get::<Children>(column).unwrap().iter().collect();
        assert_eq!(kids.first(), Some(&credit), "the first: above the others");
        assert!(
            kids.iter()
                .any(|k| w.get::<crate::hud::ProfilePanel>(*k).is_some()),
            "in the profile's column"
        );
        let node = w.get::<Node>(credit).unwrap();
        assert_eq!(node.position_type, PositionType::Relative, "it flows");
        assert_eq!(node.max_width, Val::Auto);
        assert_eq!(w.get::<Visibility>(credit), Some(&Visibility::Hidden));
        // Live aircraft drawn: shown, also with the HUD hidden.
        w.resource_mut::<RideView>().air = Some(AirView {
            attribution: Some("Aircraft data: adsb.lol (ODbL)".into()),
            ..Default::default()
        });
        // A phone: as wide as the compact profile.
        w.insert_resource(crate::hud_fit::HudLayout {
            compact: true,
            ui_px: Vec2::new(390.0, 700.0),
            ..Default::default()
        });
        app.update();
        let w = app.world_mut();
        assert_eq!(w.get::<Visibility>(credit), Some(&Visibility::Visible));
        assert_eq!(
            w.get::<Text>(credit).unwrap().0,
            "Aircraft data: adsb.lol (ODbL)"
        );
        let want = crate::hud::profile_size(Vec2::new(390.0, 700.0), true).x;
        assert_eq!(w.get::<Node>(credit).unwrap().max_width, px(want));
        assert_eq!(
            w.get::<ChildOf>(credit).unwrap().parent(),
            column,
            "docked once"
        );
    }

    /// The light dots are bright: an unlit material draws its base colour
    /// only, so that must be the lamp's colour, far above 1 (HDR) in its
    /// main channel – they were black dots until v6.96.
    #[test]
    fn the_light_dots_are_bright() {
        for l in [
            Lamp::Red,
            Lamp::Green,
            Lamp::White,
            Lamp::Beacon,
            Lamp::Strobe,
            Lamp::Burner,
        ] {
            let m = dot_material(l);
            assert!(m.unlit);
            let b = m.base_color.to_linear();
            let max = b.red.max(b.green).max(b.blue);
            assert!(max >= 5.0, "{l:?}: base {b:?}");
            assert!(m.emissive.red.max(m.emissive.green).max(m.emissive.blue) >= 20.0);
        }
        let red = dot_material(Lamp::Red).base_color.to_linear();
        assert!(red.red > 10.0 * red.green, "red stays red");
        let green = dot_material(Lamp::Green).base_color.to_linear();
        assert!(green.green > 5.0 * green.red, "green stays green");
    }

    use ocw_sim::air::Mission;

    fn plane(kind: AirKind, vel: [f64; 3], bank: f64) -> AircraftNow {
        AircraftNow {
            id: 7,
            kind,
            mission: Mission::Cruise,
            pos: [0.0, 1000.0, 0.0],
            vel,
            bank,
            on_ground: false,
            gear_down: false,
            rotor: 1.0,
            burner: false,
            look: 1,
            model: 0,
            trail: None,
        }
    }

    #[test]
    fn the_nose_points_along_the_flight_and_banks_into_turns() {
        // Flying east, banked right: the nose (−Z) east, the right wing (+X) down.
        let q = attitude(
            &plane(AirKind::Airliner, [200.0, 0.0, 0.0], 0.3),
            Quat::IDENTITY,
        );
        assert!((q * Vec3::NEG_Z).dot(Vec3::X) > 0.99);
        assert!((q * Vec3::X).y < -0.2, "{:?}", q * Vec3::X);
        // Climbing: the nose up.
        let q = attitude(
            &plane(AirKind::SmallPlane, [40.0, 6.0, 0.0], 0.0),
            Quat::IDENTITY,
        );
        assert!((q * Vec3::NEG_Z).y > 0.1);
        // A balloon stays upright.
        let q = attitude(
            &plane(AirKind::Balloon, [3.0, 0.5, 0.0], 0.4),
            Quat::IDENTITY,
        );
        assert!((q * Vec3::Y).y > 0.999);
    }

    #[test]
    fn the_ribbon_covers_every_trail_point() {
        use ocw_sim::air::{Trail, TrailPoint};
        let tr = Trail {
            kind: Contrail::Persistent,
            points: (0..5)
                .map(|i| TrailPoint {
                    pos: [f64::from(i) * -500.0, 10_000.0, 0.0],
                    age_s: f64::from(i) * 2.0 + 0.3,
                })
                .collect(),
        };
        let (pos, col, idx) = ribbon(&[&tr], Vec3::new(0.0, 0.0, 5000.0), 1000.0, Vec3::ONE);
        assert_eq!(pos.len(), 10);
        assert_eq!(col.len(), 10);
        assert_eq!(idx.len(), 4 * 6);
        assert!(idx.iter().all(|&i| (i as usize) < pos.len()));
        // Wider and fainter with age.
        let w = |i: usize| Vec3::from(pos[2 * i]).distance(Vec3::from(pos[2 * i + 1]));
        assert!(w(4) > w(0));
        assert!(col[8][3] < col[2][3]);
    }

    #[test]
    fn lamps_flash_and_the_budget_follows_the_quality() {
        assert!(Lamp::Red.lit(0.5, 0.0));
        let flashes = (0..1000)
            .filter(|i| Lamp::Beacon.lit(f64::from(*i) * 0.01, 0.0))
            .count();
        assert!((80..160).contains(&flashes), "{flashes}");
        use crate::quality::QualityLevel as Q;
        assert_eq!(budget(Q::High), (30, true));
        assert_eq!(budget(Q::Low), (10, true));
        assert_eq!(budget(Q::Quest1), (6, false));
    }

    /// The models are drawn at their true size (metres, not centimetres):
    /// an airliner of 30–40 m span and length, a small plane of ~11 m, a
    /// helicopter's rotor ~11 m, a sailplane of 15–18 m, a balloon ~23 m
    /// tall with its basket – and the far model (`lod1`) as big as the near one.
    #[test]
    fn aircraft_are_drawn_at_their_true_size() {
        // (kind, span range (x), length range (z), height range (y)), m.
        type Range = (f32, f32);
        let real: [(&str, Range, Range, Range, Range); 7] = [
            (
                "vehicles/airliner-twinjet",
                (33.0, 40.0),
                (35.0, 42.0),
                (9.0, 14.0),
                (0.0, 0.0),
            ),
            (
                "vehicles/airliner-twinjet-split-winglet",
                (33.0, 40.0),
                (35.0, 42.0),
                (9.0, 14.0),
                (0.0, 0.0),
            ),
            (
                "vehicles/plane-high-wing",
                (9.5, 12.0),
                (7.0, 9.5),
                (2.2, 3.5),
                (0.0, 0.0),
            ),
            // The rotor's blades are not along x at rest: its disc below.
            (
                "vehicles/helicopter-light",
                (8.5, 12.0),
                (9.5, 13.5),
                (2.8, 4.0),
                (8.5, 12.0),
            ),
            // The medium transport: a rotor of ~19 m, ~21 m with it.
            (
                "vehicles/helicopter-medium-transport",
                (15.0, 20.0),
                (18.0, 23.0),
                (5.0, 6.8),
                (16.0, 20.0),
            ),
            (
                "vehicles/glider-sailplane",
                (14.0, 19.0),
                (6.0, 8.5),
                (1.0, 2.0),
                (0.0, 0.0),
            ),
            // The envelope ~18 m and the basket under it: ~21–24 m overall.
            (
                "vehicles/balloon-hot-air",
                (13.0, 20.0),
                (13.0, 20.0),
                (15.0, 25.0),
                (0.0, 0.0),
            ),
        ];
        for k in AirKind::ALL {
            for id in k.model_ids() {
                assert!(real.iter().any(|r| r.0 == *id), "{id} has its sizes here");
            }
        }
        for (k, span, len, high, rotor) in real {
            let m = ocw_assets::shipped::model(k);
            let (lo, hi) = m.near_bounds().expect("near model");
            let d = hi - lo;
            let within = |v: f32, (a, b): (f32, f32)| (a..=b).contains(&v);
            assert!(
                within(d.x, span) && within(d.z, len) && within(d.y, high),
                "{k:?}: {:.1} m span, {:.1} m long, {:.1} m high",
                d.x,
                d.z,
                d.y
            );
            if rotor.1 > 0.0 {
                let r = m.node("rotor_main").expect("rotor");
                let (rlo, rhi) = m.bounds_where(|i| m.is_below(i, r)).expect("rotor");
                // The blades' widest extent: at most the disc's diameter.
                let blades = (rhi - rlo).max_element();
                assert!(within(blades, rotor), "{k}: rotor {blades:.1} m");
            }
            let lod = m.node("lod1").expect("lod1");
            let (flo, fhi) = m.bounds_where(|i| m.is_below(i, lod)).expect("far model");
            let f = fhi - flo;
            assert!(
                (f.max_element() / d.max_element() - 1.0).abs() < 0.1,
                "{k:?}: lod1 {f:?} vs near {d:?}"
            );
        }
    }

    #[test]
    fn only_far_aircraft_grow_to_stay_visible() {
        // 1080 rows at the default field of view (~45°): ~1300 px per rad.
        let focal = 1080.0 / (2.0 * (0.785f32 / 2.0).tan());
        let mut meshes = Assets::<Mesh>::default();
        for (k, id) in AirKind::ALL
            .into_iter()
            .flat_map(|k| k.model_ids().iter().map(move |id| (k, *id)))
        {
            let km = kind_model(id, &ocw_assets::shipped::model(id), &mut meshes);
            // True size alongside, at the switch to lod1 and well past it.
            for d in [0.0, 5.0, 30.0, 120.0, lod_m(k), 3.0 * lod_m(k)] {
                assert_eq!(draw_scale(d, focal, km.size_m), 1.0, "{k:?} at {d} m");
            }
            // A dot of MIN_PX far away.
            let far = 60_000.0;
            let s = draw_scale(far, focal, km.size_m);
            assert!(s > 1.0, "{k:?}");
            let px = km.size_m * s / far * focal;
            assert!((px - MIN_PX).abs() < 0.01 || s == 80.0, "{k:?}: {px} px");
        }
    }

    #[test]
    fn every_aircraft_model_has_its_parts() {
        let mut meshes = Assets::<Mesh>::default();
        for (k, id) in AirKind::ALL
            .into_iter()
            .flat_map(|k| k.model_ids().iter().map(move |id| (k, *id)))
        {
            let m = kind_model(id, &ocw_assets::shipped::model(id), &mut meshes);
            assert!(!m.near.is_empty() && !m.far.is_empty(), "{id}");
            match k {
                AirKind::Airliner => {
                    assert!(!m.gear.is_empty());
                    assert!(m.lamps.iter().any(|l| l.1 == Lamp::Red));
                    assert!(m.lamps.iter().any(|l| l.1 == Lamp::Strobe));
                    // Red on the left wing tip, green on the right.
                    let red = m.lamps.iter().find(|l| l.1 == Lamp::Red).unwrap().0;
                    let green = m.lamps.iter().find(|l| l.1 == Lamp::Green).unwrap().0;
                    assert!(red.x < -10.0 && green.x > 10.0);
                    assert!(m.size_m > 30.0);
                }
                AirKind::SmallPlane => assert_eq!(m.spinners.len(), 1),
                AirKind::Helicopter => assert_eq!(m.spinners.len(), 2),
                AirKind::Balloon => assert!(m.lamps.iter().any(|l| l.1 == Lamp::Burner)),
                AirKind::Glider => assert!(m.spinners.is_empty()),
            }
        }
    }

    /// Real rotor and propeller speeds (v7.08, the user 2026-11-28): the
    /// light helicopter's main rotor ~450 rpm, the medium one's ~215, tail
    /// rotors 1,100 – 2,200, a propeller ~2,400 rpm.
    #[test]
    fn rotors_turn_at_their_real_rates() {
        let rpm = |m: &str, n: &str| spin_rate(m, n).0 * 60.0;
        assert!((390.0..=530.0).contains(&rpm("vehicles/helicopter-light", "rotor_main")));
        assert!(
            (190.0..=260.0).contains(&rpm("vehicles/helicopter-medium-transport", "rotor_main"))
        );
        for m in [
            "vehicles/helicopter-light",
            "vehicles/helicopter-medium-transport",
        ] {
            assert!((1100.0..=2200.0).contains(&rpm(m, "rotor_tail")), "{m}");
        }
        assert!((2300.0..=2700.0).contains(&rpm("vehicles/plane-high-wing", "prop")));
        assert_eq!(
            spin_rate("vehicles/helicopter-medium-transport", "rotor_main").1,
            5
        );
        // Every spinner of the shipped models has its blur disc.
        let mut meshes = Assets::<Mesh>::default();
        for id in [
            "vehicles/helicopter-light",
            "vehicles/helicopter-medium-transport",
            "vehicles/plane-high-wing",
        ] {
            let km = kind_model(id, &ocw_assets::shipped::model(id), &mut meshes);
            assert!(!km.spinners.is_empty());
            assert!(km.spinners.iter().all(|s| s.disc.is_some()), "{id}");
        }
    }

    /// Solid blades when still or spooling slowly, the blur disc at speed:
    /// by the blades' passing rate only, so the same at 30 and 60 fps; the
    /// angle after a second is the same at either frame rate.
    #[test]
    fn the_blur_disc_takes_over_by_rpm_alone() {
        assert_eq!(disc_blend(0.0, 5), 0.0);
        assert_eq!(disc_blend(0.5, 3), 0.0, "spooling up slowly: solid");
        assert_eq!(disc_blend(3.6, 5), 1.0);
        assert_eq!(disc_blend(7.5, 3), 1.0);
        assert_eq!(disc_blend(40.0, 2), 1.0);
        assert_eq!(disc_blend(0.4 * 40.0, 2), 1.0, "an idling propeller blurs");
        let mid = disc_blend(2.0, 5);
        assert!(mid > 0.0 && mid < 1.0);
        assert!(disc_blend(2.4, 5) > mid);
        let run = |fps: u32| (0..fps).fold(0.0f32, |a, _| spin_step(a, 1.0 / fps as f32, 3.3));
        assert!((run(30) - run(60)).abs() < 1e-3, "{} {}", run(30), run(60));
        assert!((run(60) - 0.3 * std::f32::consts::TAU).abs() < 1e-3);
    }

    /// The airliners' gear legs swing into their bays over GEAR_TRAVEL_S
    /// (nose forward, mains inwards).
    #[test]
    fn the_gear_swings_in_and_out() {
        let mut meshes = Assets::<Mesh>::default();
        for id in [
            "vehicles/airliner-twinjet",
            "vehicles/airliner-twinjet-split-winglet",
        ] {
            let km = kind_model(id, &ocw_assets::shipped::model(id), &mut meshes);
            assert_eq!(km.gear.len(), 3, "{id}: nose and two mains");
            for leg in &km.gear {
                assert!(leg.up_angle.abs() > 1.5, "{id}: retracts");
                assert!(
                    leg.pivot.translation.y > 1.5,
                    "{id}: pivots at the leg's top"
                );
            }
        }
        let mut e = 1.0;
        let mut t = 0.0;
        while e > 0.0 {
            e = gear_travel(e, false, 1.0 / 60.0);
            t += 1.0 / 60.0;
        }
        assert!((t - GEAR_TRAVEL_S).abs() < 0.05, "{t}");
        assert_eq!(gear_travel(0.5, true, 100.0), 1.0);
        // A left main leg folds its wheels inwards (+X).
        let left = GearPart {
            owner: 0,
            pivot: Transform::from_xyz(-3.8, 2.5, 1.0),
            axis: Vec3::Z,
            up_angle: std::f32::consts::FRAC_PI_2,
            ext: 0.0,
        };
        let wheel = left.transform() * Vec3::new(0.0, -1.9, 0.0);
        assert!(wheel.x > -2.0 && (wheel.y - 2.5).abs() < 0.01, "{wheel}");
    }
}

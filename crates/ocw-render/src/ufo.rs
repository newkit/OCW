//! The UFOs (spec 14 Phase 18, 18C *UFOs*, 18E T9; Q7): what
//! [`RideView::ufo`] holds – the saucers of `ocw_sim::ufo`, their beams,
//! the cow of a beam scene and the crop circles – drawn with the T6 model
//! `vehicles/ufo-saucer` and the cow `creatures/cow-grazing`.
//!
//! - **Saucers:** the near model close by, its `lod1` beyond [`LOD_M`],
//!   and far away kept at least [`crate::aircraft::MIN_PX`] big; growing
//!   out of nothing when they materialise. The rim lights (`light_rim_*`)
//!   are glowing dots of a few pixels that run round the rim in many
//!   colours at dusk and at night, and flash in the unicorn's mane colours
//!   while the visitor circles and follows it; by day high up a saucer is a
//!   silver glint without lights.
//! - **The beam:** one open cone, additive and unlit, from under the hull
//!   to the ground, its strength from the scene.
//! - **The cow:** grazing, lifted, turning slowly in the beam, set down.
//! - **Crop circles:** flattened crop as a pale, translucent mesh of rings
//!   and discs laid on the terrain (no texture, no change to the world
//!   package), appearing while the saucer hovers and staying.
//!
//! No shadows, no new textures (the WebGL 2 and iPhone memory budget); a
//! handful of entities.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ocw_assets::model::Model;
use ocw_sim::ufo::{CowNow, CropCircle, Lights, SightingKind, UfoNow};

use crate::RideView;
use crate::aircraft::{DOT_PX, draw_scale, eye_and_focal, mesh_of};
use crate::camera::FollowCamera;

/// What the UFOs are this frame (written by the client: the rider's own
/// view, `traffic.ufo`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UfoView {
    pub ufos: Vec<UfoNow>,
    pub cows: Vec<CowNow>,
    pub circles: Vec<CropCircle>,
    /// The world clock (s): how far the circles are made.
    pub now_s: f64,
    /// `traffic.sound`: the hum and the whoosh are heard.
    pub heard: bool,
    pub listener: [f64; 3],
    pub listener_forward: [f64; 2],
}

/// The saucer's switch to `lod1` (m).
pub const LOD_M: f32 = 300.0;
/// The cow's switch to `lod1` (m).
pub const COW_LOD_M: f32 = 80.0;
/// The model file of the saucer and of the cow.
pub const SAUCER: &str = "vehicles/ufo-saucer";
pub const COW: &str = "creatures/cow-grazing";
/// The rim lights' running colours (red, amber, green, cyan, blue, magenta).
pub const CYCLE: [u32; 6] = [0xFF3030, 0xFFB020, 0x40FF60, 0x30E0FF, 0x4060FF, 0xFF40E0];
/// The beam's colour.
const BEAM: [f32; 3] = [0.55, 1.0, 0.75];
/// The flattened crop's colour (pale straw).
const CROP: [f32; 3] = [0.93, 0.86, 0.6];

pub struct UfoPlugin;

impl Plugin for UfoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UfoModels>()
            .init_resource::<UfoEntities>()
            .add_systems(
                PostUpdate,
                (sync_saucers, sync_cows, sync_circles).before(TransformSystems::Propagate),
            );
    }
}

/// A mesh part with its slot colour.
#[derive(Clone)]
struct Part {
    mesh: Handle<Mesh>,
    slot: String,
    color: [f32; 4],
}

#[derive(Clone, Default)]
struct Parts {
    near: Vec<Part>,
    far: Vec<Part>,
    /// The rim lights' places (model space), in their order.
    rim: Vec<Vec3>,
    size_m: f32,
}

#[derive(Resource, Default)]
struct UfoModels {
    saucer: Option<Parts>,
    cow: Option<Parts>,
    materials: HashMap<(String, [u32; 4]), Handle<StandardMaterial>>,
    dot: Option<Handle<Mesh>>,
    dots: HashMap<u32, Handle<StandardMaterial>>,
    cone: Option<Handle<Mesh>>,
}

#[derive(Resource, Default)]
struct UfoEntities {
    saucers: HashMap<u64, Entity>,
    cows: HashMap<u64, Entity>,
    circles: HashMap<u64, (Entity, Handle<StandardMaterial>)>,
}

#[derive(Component)]
struct Saucer;

#[derive(Component)]
struct RimDot {
    owner: u64,
    k: usize,
    at: Vec3,
}

#[derive(Component)]
struct Beam {
    owner: u64,
    material: Handle<StandardMaterial>,
}

#[derive(Component)]
struct Cow;

/// The meshes of a model file: near, `lod1`, the rim lights.
fn parts_of(model: &Model, meshes: &mut Assets<Mesh>) -> Parts {
    let lod = model.node("lod1");
    let mut out = Parts {
        size_m: 10.0,
        ..default()
    };
    let shade = model.shade_scale();
    let mut rim: Vec<(usize, Vec3)> = Vec::new();
    for (i, n) in model.nodes.iter().enumerate() {
        let far = lod.is_some_and(|l| model.is_below(i, l));
        if !far && let Some(k) = n.name.strip_prefix("light_rim_") {
            rim.push((
                k.parse().unwrap_or(rim.len()),
                Vec3::from(model.world(i).translation),
            ));
        }
        for p in &n.primitives {
            let mat = &model.materials[p.material];
            let part = Part {
                mesh: meshes.add(mesh_of(p, model.world(i), shade)),
                slot: mat.name.clone(),
                color: mat.color,
            };
            if far {
                out.far.push(part);
            } else {
                out.near.push(part);
            }
        }
    }
    rim.sort_by_key(|r| r.0);
    out.rim = rim.into_iter().map(|r| r.1).collect();
    if let Some((lo, hi)) = model.near_bounds() {
        out.size_m = (hi - lo).max_element().max(1.0);
    }
    out
}

fn material(
    models: &mut UfoModels,
    materials: &mut Assets<StandardMaterial>,
    slot: &str,
    color: [f32; 4],
) -> Handle<StandardMaterial> {
    let lamp = ocw_assets::slots::LIGHTS.contains(&slot);
    let key = (slot.to_string(), color.map(f32::to_bits));
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
                ..default()
            })
        })
        .clone()
}

/// An emissive dot's material of colour `rgb` (0xRRGGBB).
fn dot_material(
    models: &mut UfoModels,
    materials: &mut Assets<StandardMaterial>,
    rgb: u32,
) -> Handle<StandardMaterial> {
    models
        .dots
        .entry(rgb)
        .or_insert_with(|| {
            let c = ocw_assets::model::srgb(rgb);
            // Unlit draws the base colour (bright, for the bloom), the
            // emissive for the lit paths.
            materials.add(StandardMaterial {
                base_color: Color::linear_rgb(c[0] * 6.0, c[1] * 6.0, c[2] * 6.0),
                emissive: LinearRgba::rgb(c[0] * 24.0, c[1] * 24.0, c[2] * 24.0),
                unlit: true,
                fog_enabled: false,
                ..default()
            })
        })
        .clone()
}

/// The open cone of the beam: radius 0.25 at the top (y 0), 1 at the
/// bottom (y −1).
fn cone_mesh() -> Mesh {
    let n = 24u32;
    let mut pos = Vec::new();
    let mut idx = Vec::new();
    for i in 0..=n {
        let a = i as f32 / n as f32 * std::f32::consts::TAU;
        let (s, c) = a.sin_cos();
        pos.push([0.25 * c, 0.0, 0.25 * s]);
        pos.push([c, -1.0, s]);
    }
    for i in 0..n {
        let b = 2 * i;
        idx.extend_from_slice(&[b, b + 1, b + 2, b + 1, b + 3, b + 2]);
    }
    let len = pos.len();
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; len]);
    m.insert_indices(Indices::U32(idx));
    m
}

fn ensure(models: &mut UfoModels, meshes: &mut Assets<Mesh>) {
    if models.saucer.is_some() {
        return;
    }
    models.saucer = Some(parts_of(&ocw_assets::shipped::model(SAUCER), meshes));
    models.cow = Some(parts_of(&ocw_assets::shipped::model(COW), meshes));
    models.dot = Some(meshes.add(Sphere::new(1.0).mesh().ico(1).expect("ico sphere")));
    models.cone = Some(meshes.add(cone_mesh()));
}

/// Spawns a model's parts below `root` (near and far by `lod`).
fn spawn_parts(
    commands: &mut Commands,
    models: &mut UfoModels,
    materials: &mut Assets<StandardMaterial>,
    parts: &Parts,
    lod: f32,
) -> Vec<Entity> {
    let near = VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: lod * 0.95..lod * 1.05,
        use_aabb: false,
    };
    let far = VisibilityRange {
        start_margin: lod * 0.95..lod * 1.05,
        end_margin: 1.0e7..1.0e7,
        use_aabb: false,
    };
    let mut kids = Vec::new();
    for (list, range) in [(&parts.near, &near), (&parts.far, &far)] {
        for p in list {
            let m = material(models, materials, &p.slot, p.color);
            kids.push(
                commands
                    .spawn((
                        Mesh3d(p.mesh.clone()),
                        MeshMaterial3d(m),
                        Transform::default(),
                        range.clone(),
                        NotShadowCaster,
                    ))
                    .id(),
            );
        }
    }
    kids
}

/// The colour of rim light `k` of `n` at `t` (s), or `None` (off).
pub fn rim_colour(lights: Lights, k: usize, n: usize, t: f64, mane: &[u32]) -> Option<u32> {
    match lights {
        Lights::Off => None,
        Lights::Cycle => {
            // Running round the rim, four steps a second.
            let step = (t * 4.0).floor() as usize;
            let i = (k + step) % n.max(1);
            Some(CYCLE[i % CYCLE.len()])
        }
        Lights::Mane => {
            if mane.is_empty() {
                return None;
            }
            // Flashing: every other light, the colours stepping 3 times a second.
            let step = (t * 3.0).floor() as usize;
            ((k + step).is_multiple_of(2)).then(|| mane[(k / 2 + step) % mane.len()])
        }
    }
}

/// The local unicorn's mane colours: the rider's colour (when it is bright
/// enough to glow), then the rainbow.
fn mane(view: &RideView) -> Vec<u32> {
    let mut out: Vec<u32> = view
        .unicorns
        .iter()
        .find(|u| u.is_local)
        .map(|u| u.colour.0 & 0xFF_FFFF)
        // A dark rider colour would be a dark light: the rainbow only then.
        .filter(|c| [c >> 16, c >> 8, *c].iter().any(|v| v & 0xFF >= 0x70))
        .map_or_else(Vec::new, |c| vec![c]);
    out.extend(ocw_assets::unicorn::RAINBOW);
    out
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn sync_saucers(
    mut commands: Commands,
    view: Res<RideView>,
    time: Res<Time>,
    mut models: ResMut<UfoModels>,
    mut ents: ResMut<UfoEntities>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    cams: Query<(&Camera, &GlobalTransform, &Projection), With<FollowCamera>>,
    mut roots: Query<&mut Transform, (With<Saucer>, Without<RimDot>, Without<Beam>)>,
    mut dots: Query<
        (
            &RimDot,
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        (Without<Saucer>, Without<Beam>),
    >,
    mut beams: Query<(&Beam, &mut Transform, &mut Visibility), (Without<Saucer>, Without<RimDot>)>,
) {
    let list = view.ufo.as_ref().map_or(&[][..], |u| &u.ufos[..]);
    let alive: std::collections::HashSet<u64> = list.iter().map(|u| u.id).collect();
    ents.saucers.retain(|id, e| {
        let keep = alive.contains(id);
        if !keep && let Ok(mut c) = commands.get_entity(*e) {
            c.despawn();
        }
        keep
    });
    if list.is_empty() {
        return;
    }
    ensure(&mut models, &mut meshes);
    let Some((eye, focal)) = eye_and_focal(&cams) else {
        return;
    };
    let parts = models.saucer.clone().expect("loaded");
    let sun = view
        .sky
        .as_ref()
        .map_or(30.0, |s| s.sky.sun.elevation_deg as f32);
    let t = time.elapsed_secs_f64();
    let mane = mane(&view);
    let mut info: HashMap<u64, (&UfoNow, f32)> = HashMap::new();
    for u in list {
        let pos = Vec3::new(u.pos[0] as f32, u.pos[1] as f32, u.pos[2] as f32);
        let scale = draw_scale(eye.distance(pos), focal, parts.size_m) * u.presence.max(0.02);
        info.insert(u.id, (u, scale));
        let tf = Transform::from_translation(pos)
            .with_rotation(Quat::from_rotation_y(u.spin as f32))
            .with_scale(Vec3::splat(scale));
        if let Some(&e) = ents.saucers.get(&u.id) {
            if let Ok(mut t) = roots.get_mut(e) {
                *t = tf;
            }
            continue;
        }
        let root = commands.spawn((Saucer, tf, Visibility::default())).id();
        let mut kids = spawn_parts(&mut commands, &mut models, &mut materials, &parts, LOD_M);
        let dot = models.dot.clone().expect("dot");
        let first = dot_material(&mut models, &mut materials, CYCLE[0]);
        for (k, at) in parts.rim.iter().enumerate() {
            kids.push(
                commands
                    .spawn((
                        Mesh3d(dot.clone()),
                        MeshMaterial3d(first.clone()),
                        Transform::from_translation(*at).with_scale(Vec3::splat(0.25)),
                        Visibility::Hidden,
                        NotShadowCaster,
                        RimDot {
                            owner: u.id,
                            k,
                            at: *at,
                        },
                    ))
                    .id(),
            );
        }
        commands.entity(root).add_children(&kids);
        // The beam: in the world, not below the spinning root.
        let bm = materials.add(StandardMaterial {
            base_color: Color::linear_rgba(BEAM[0], BEAM[1], BEAM[2], 0.0),
            unlit: true,
            alpha_mode: AlphaMode::Add,
            cull_mode: None,
            double_sided: true,
            ..default()
        });
        let beam = commands
            .spawn((
                Mesh3d(models.cone.clone().expect("cone")),
                MeshMaterial3d(bm.clone()),
                Transform::default(),
                Visibility::Hidden,
                NotShadowCaster,
                Beam {
                    owner: u.id,
                    material: bm,
                },
            ))
            .id();
        commands.entity(root).add_child(beam);
        ents.saucers.insert(u.id, root);
    }
    let n = parts.rim.len();
    for (d, mut tf, mut vis, mut mat) in &mut dots {
        let Some((u, scale)) = info.get(&d.owner) else {
            continue;
        };
        // Lights at dusk and at night; the visitor's always (it shows off).
        let lit = sun < 4.0 || u.kind == SightingKind::Visitor;
        let colour = lit
            .then(|| rim_colour(u.lights, d.k, n, t + (u.id % 97) as f64 * 0.1, &mane))
            .flatten();
        vis.set_if_neq(if colour.is_some() {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Some(c) = colour {
            let h = dot_material(&mut models, &mut materials, c);
            if mat.0 != h {
                mat.0 = h;
            }
            let pos = Vec3::new(u.pos[0] as f32, u.pos[1] as f32, u.pos[2] as f32);
            let dist = eye.distance(pos).max(1.0);
            let r = (DOT_PX * dist / focal).max(0.35);
            *tf = Transform::from_translation(d.at).with_scale(Vec3::splat(r / scale.max(1e-3)));
        }
    }
    for (b, mut tf, mut vis) in &mut beams {
        let Some((u, scale)) = info.get(&b.owner) else {
            continue;
        };
        let top = u.pos[1] + 0.95;
        let len = (top - u.beam_ground_m).max(0.5) as f32;
        if u.beam <= 0.0 {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        }
        vis.set_if_neq(Visibility::Inherited);
        // In the root's frame: undo its spin and scale.
        let s = scale.max(1e-3);
        *tf = Transform::from_translation(Vec3::Y * 0.95 / s)
            .with_rotation(Quat::from_rotation_y(-(u.spin as f32)))
            .with_scale(Vec3::new(5.5 / s, len / s, 5.5 / s));
        if let Some(mut m) = materials.get_mut(&b.material) {
            // Brighter at night.
            let a = u.beam * if sun < 4.0 { 0.22 } else { 0.14 };
            m.base_color = Color::linear_rgba(BEAM[0], BEAM[1], BEAM[2], a);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn sync_cows(
    mut commands: Commands,
    view: Res<RideView>,
    time: Res<Time>,
    mut models: ResMut<UfoModels>,
    mut ents: ResMut<UfoEntities>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut roots: Query<&mut Transform, With<Cow>>,
) {
    let list = view.ufo.as_ref().map_or(&[][..], |u| &u.cows[..]);
    let alive: std::collections::HashSet<u64> = list.iter().map(|c| c.id).collect();
    ents.cows.retain(|id, e| {
        let keep = alive.contains(id);
        if !keep && let Ok(mut c) = commands.get_entity(*e) {
            c.despawn();
        }
        keep
    });
    if list.is_empty() {
        return;
    }
    ensure(&mut models, &mut meshes);
    let parts = models.cow.clone().expect("loaded");
    let t = time.elapsed_secs();
    for c in list {
        let pos = Vec3::new(c.pos[0] as f32, c.pos[1] as f32, c.pos[2] as f32);
        // In the beam: hanging a little tilted, legs dangling.
        let tilt = if c.lifted {
            Quat::from_rotation_x(0.18 * (t * 0.9).sin())
                * Quat::from_rotation_z(0.1 * (t * 0.7).cos())
        } else {
            Quat::IDENTITY
        };
        let tf = Transform::from_translation(pos)
            .with_rotation(Quat::from_rotation_y(c.yaw as f32) * tilt);
        if let Some(&e) = ents.cows.get(&c.id) {
            if let Ok(mut r) = roots.get_mut(e) {
                *r = tf;
            }
            continue;
        }
        let root = commands.spawn((Cow, tf, Visibility::default())).id();
        let kids = spawn_parts(
            &mut commands,
            &mut models,
            &mut materials,
            &parts,
            COW_LOD_M,
        );
        commands.entity(root).add_children(&kids);
        ents.cows.insert(c.id, root);
    }
}

/// The crop circle's mesh (positions on the terrain from `ground`, lifted
/// a little), in world metres.
pub fn circle_mesh(
    c: &CropCircle,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
) -> (Vec<[f32; 3]>, Vec<u32>) {
    let mut pos: Vec<[f32; 3]> = Vec::new();
    let mut idx = Vec::new();
    for (o, r0, r1) in &c.shapes {
        let centre = [c.centre[0] + o[0], c.centre[1] + o[1]];
        let segs = ((r1 * 2.0) as u32).clamp(20, 64);
        // Rings of vertices from the inner to the outer radius (a disc:
        // from near the middle), every couple of metres so they follow
        // the ground.
        let steps = (((r1 - r0) / 2.5).ceil() as u32).max(1);
        let radii: Vec<f64> = (0..=steps)
            .map(|k| r0 + (r1 - r0) * f64::from(k) / f64::from(steps))
            .collect();
        let base = pos.len() as u32;
        for &r in &radii {
            for i in 0..segs {
                let a = f64::from(i) / f64::from(segs) * std::f64::consts::TAU;
                let (x, z) = (centre[0] + r * a.cos(), centre[1] + r * a.sin());
                let y = ground(x, z).unwrap_or(c.ground_m) + 0.2;
                pos.push([x as f32, y as f32, z as f32]);
            }
        }
        for k in 0..steps {
            for i in 0..segs {
                let j = (i + 1) % segs;
                let a = base + k * segs + i;
                let b = base + k * segs + j;
                let a2 = base + (k + 1) * segs + i;
                let b2 = base + (k + 1) * segs + j;
                // Facing up (counter-clockwise seen from above).
                idx.extend_from_slice(&[a, b, a2, b, b2, a2]);
            }
        }
    }
    (pos, idx)
}

#[allow(clippy::too_many_arguments)]
fn sync_circles(
    mut commands: Commands,
    view: Res<RideView>,
    ground: Option<Res<crate::Ground>>,
    mut ents: ResMut<UfoEntities>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let (list, now) = view
        .ufo
        .as_ref()
        .map_or((&[][..], 0.0), |u| (&u.circles[..], u.now_s));
    let alive: std::collections::HashSet<u64> = list.iter().map(|c| c.id).collect();
    ents.circles.retain(|id, (e, _)| {
        let keep = alive.contains(id);
        if !keep && let Ok(mut c) = commands.get_entity(*e) {
            c.despawn();
        }
        keep
    });
    for c in list {
        let alpha = 0.55 * c.made(now) as f32;
        if let Some((_, m)) = ents.circles.get(&c.id) {
            if let Some(mut m) = materials.get_mut(m)
                && (m.base_color.alpha() - alpha).abs() > 1e-3
            {
                m.base_color.set_alpha(alpha);
            }
            continue;
        }
        let g = |x: f64, z: f64| ground.as_ref().and_then(|g| (g.0)(x, z));
        let (pos, idx) = circle_mesh(c, &g);
        let n = pos.len();
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
        mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; n]);
        mesh.insert_indices(Indices::U32(idx));
        let m = materials.add(StandardMaterial {
            base_color: Color::linear_rgba(CROP[0], CROP[1], CROP[2], alpha),
            perceptual_roughness: 1.0,
            alpha_mode: AlphaMode::Blend,
            ..default()
        });
        let e = commands
            .spawn((
                Mesh3d(meshes.add(mesh)),
                MeshMaterial3d(m.clone()),
                Transform::default(),
                NotShadowCaster,
                Visibility::default(),
            ))
            .id();
        ents.circles.insert(c.id, (e, m));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_saucer_has_its_rim_and_the_cow_its_far_model() {
        let mut meshes = Assets::<Mesh>::default();
        let s = parts_of(&ocw_assets::shipped::model(SAUCER), &mut meshes);
        assert!(s.rim.len() >= ocw_assets::kinds::UFO_RIM_LIGHTS);
        assert!(!s.near.is_empty() && !s.far.is_empty());
        assert!((10.0..14.0).contains(&s.size_m), "{}", s.size_m);
        // The rim round the hull, ~6 m out.
        assert!(s.rim.iter().all(|p| (p.x.hypot(p.z) - 6.0).abs() < 0.3));
        let c = parts_of(&ocw_assets::shipped::model(COW), &mut meshes);
        assert!(!c.near.is_empty() && !c.far.is_empty());
        assert!(c.rim.is_empty());
        assert!((2.0..3.0).contains(&c.size_m));
    }

    #[test]
    fn rim_lights_run_round_and_flash_in_the_mane_colours() {
        let a = rim_colour(Lights::Cycle, 0, 12, 0.0, &[]).unwrap();
        let b = rim_colour(Lights::Cycle, 0, 12, 0.3, &[]).unwrap();
        assert_ne!(a, b, "running");
        assert_eq!(rim_colour(Lights::Off, 0, 12, 0.0, &[]), None);
        let mane = [0x112233, 0xE53935, 0xFB8C00];
        let lit: Vec<Option<u32>> = (0..12)
            .map(|k| rim_colour(Lights::Mane, k, 12, 0.0, &mane))
            .collect();
        assert_eq!(lit.iter().filter(|c| c.is_some()).count(), 6, "every other");
        assert!(lit.iter().flatten().all(|c| mane.contains(c)));
        assert_eq!(rim_colour(Lights::Mane, 0, 12, 0.0, &[]), None);
    }

    #[test]
    fn a_crop_circle_lies_on_the_ground_facing_up() {
        let c = CropCircle {
            id: 1,
            centre: [100.0, -50.0],
            ground_m: 300.0,
            shapes: vec![([0.0, 0.0], 0.0, 9.0), ([0.0, 0.0], 12.0, 13.0)],
            from_s: 0.0,
            to_s: 10.0,
        };
        let slope = |x: f64, _z: f64| Some(300.0 + 0.1 * (x - 100.0));
        let (pos, idx) = circle_mesh(&c, &slope);
        assert!(!idx.is_empty() && idx.iter().all(|&i| (i as usize) < pos.len()));
        for p in &pos {
            let g = 300.0 + 0.1 * (f64::from(p[0]) - 100.0);
            assert!((f64::from(p[1]) - g - 0.2).abs() < 1e-3);
            let r = (f64::from(p[0]) - 100.0).hypot(f64::from(p[2]) + 50.0);
            assert!(r <= 13.01);
        }
        // Every triangle faces up (the disc's middle ones have no area).
        let mut up = 0;
        for t in idx.chunks(3) {
            let [a, b, cc] = [0, 1, 2].map(|k| Vec3::from(pos[t[k] as usize]));
            let n = (b - a).cross(cc - a).y;
            assert!(n > -1e-6);
            up += usize::from(n > 0.0);
        }
        assert!(up > idx.len() / 6);
    }
}

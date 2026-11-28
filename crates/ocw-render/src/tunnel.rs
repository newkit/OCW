//! Tunnels (spec §3.2 "Camera and avatar", §4.3): where the local rider's
//! road runs below the terrain surface.
//!
//! The world builder leaves the terrain natural above OSM `tunnel=*` roads
//! (it is not conformed to them, and they get no road strip), so in a tunnel
//! the road and the rider are inside the hill. Without help the camera
//! would stay above the terrain and the rider would be hidden for the whole
//! tunnel. Here the renderer finds those stretches itself, from the track
//! and the ground height – the terrain more than [`COVER_M`] above the road
//! centre – so it works for every kind of track (route files, package
//! routes, free ride) and needs no extra package data:
//!
//! - **Tube:** each stretch gets a simple tunnel tube (floor, walls, arch;
//!   unlit, dark, a strip of lamps along the crown) that reaches
//!   [`PORTAL_M`] out of the hill at both ends as a portal.
//! - **Camera:** in and around a stretch the camera leaves the chase orbit
//!   and follows close behind the rider on the road centre inside the tube
//!   ([`tunnel_eye`], under the hillside at the entry portal); terrain and
//!   buildings above do not lift it. It blends
//!   in from [`ENTER_AHEAD_M`] before the portal and out [`LEAVE_AFTER_M`]
//!   after the exit, smoothed over about half a second.

use std::collections::HashMap;

use bevy::prelude::*;
use ocw_world::Track;

use crate::world::SceneEntity;
use crate::{Ground, RideView, to_vec3};

/// The road counts as underground where the terrain is this much above
/// the road surface (m). Conformed terrain lies below the road.
pub const COVER_M: f64 = 0.5;
/// Sampling step along the track when looking for tunnels (m).
pub const SCAN_STEP_M: f64 = 2.0;
/// Covered stretches closer than this are one tunnel (m).
pub const MERGE_GAP_M: f64 = 20.0;
/// Shorter covered stretches are terrain artefacts, not tunnels (m).
pub const MIN_TUNNEL_M: f64 = 10.0;
/// The tube reaches this far beyond the covered stretch (portal, m).
pub const PORTAL_M: f64 = 4.0;
/// Tube: walls this high above the road, the arch's crown at
/// [`TUBE_CROWN_M`], and [`TUBE_SIDE_M`] beside the road edge (the
/// shared cross-section of [`ocw_world::tunnels`]).
pub const TUBE_WALL_M: f64 = ocw_world::tunnels::WALL_M;
pub const TUBE_CROWN_M: f64 = ocw_world::tunnels::CROWN_M;
pub const TUBE_SIDE_M: f64 = ocw_world::tunnels::SIDE_M;
/// Tunnels of the package's road graph are drawn within this distance of
/// the rider (m) and dropped beyond [`DROP_M`].
pub const NEAR_M: f64 = 1500.0;
pub const DROP_M: f64 = 2000.0;
/// The camera in a tunnel: this far behind the rider along the road, this
/// high above it, looking at a point [`TUNNEL_LOOK_AHEAD_M`] ahead.
pub const TUNNEL_CAM_BEHIND_M: f64 = 4.5;
pub const TUNNEL_CAM_HEIGHT_M: f32 = 2.2;
pub const TUNNEL_LOOK_AHEAD_M: f64 = 10.0;
/// The tunnel camera takes over this far before a tunnel …
pub const ENTER_AHEAD_M: f64 = 15.0;
/// … and hands back this far after it.
pub const LEAVE_AFTER_M: f64 = 12.0;
/// Time constant of the blend between chase and tunnel camera (s).
pub const BLEND_S: f32 = 0.4;
/// Window around the rider that is searched for tunnels (m); a tunnel that
/// reaches beyond it is followed to its end (up to [`MAX_TUNNEL_M`]).
const SCAN_BEHIND_M: f64 = 300.0;
const SCAN_AHEAD_M: f64 = 800.0;
const MAX_TUNNEL_M: f64 = 5000.0;
/// Seconds between two scans (the ground changes when tiles load).
const SCAN_INTERVAL_S: f32 = 0.5;

/// Whether the road centre at `distance` is underground.
pub fn covered(track: &dyn Track, ground: &dyn Fn(f64, f64) -> Option<f64>, distance: f64) -> bool {
    let p = track.sample(distance).position;
    ground(p[0], p[2]).is_some_and(|h| h > p[1] + COVER_M)
}

/// The tunnels (from, to distance, portals included) of `track` around
/// `[from, to]`: underground stretches, merged across short gaps, without
/// short artefacts. A tunnel cut by the window is followed to its end.
pub fn covered_stretches(
    track: &dyn Track,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
    from: f64,
    to: f64,
) -> Vec<(f64, f64)> {
    let (lo, hi) = track.extent().unwrap_or((f64::NEG_INFINITY, f64::INFINITY));
    let clamp = |d: f64| d.clamp(lo, hi);
    // Samples on a fixed grid of distances, so repeated scans around a
    // moving rider find exactly the same stretches.
    let grid = |d: f64| (d / SCAN_STEP_M).round() * SCAN_STEP_M;
    let (mut from, mut to) = (clamp(grid(from)), clamp(grid(to)));
    if from >= to {
        return Vec::new();
    }
    // Follow a tunnel cut by the window to its end.
    let start = from;
    while from > lo && start - from < MAX_TUNNEL_M && covered(track, ground, from) {
        from = clamp(from - SCAN_STEP_M);
    }
    let end = to;
    while to < hi && to - end < MAX_TUNNEL_M && covered(track, ground, to) {
        to = clamp(to + SCAN_STEP_M);
    }
    let n = ((to - from) / SCAN_STEP_M).ceil() as usize;
    let mut runs: Vec<(f64, f64)> = Vec::new();
    for i in 0..=n {
        let d = (from + i as f64 * SCAN_STEP_M).min(to);
        if !covered(track, ground, d) {
            continue;
        }
        match runs.last_mut() {
            Some(last) if d - last.1 <= MERGE_GAP_M => last.1 = d,
            _ => runs.push((d, d)),
        }
    }
    runs.into_iter()
        .filter(|(a, b)| b - a >= MIN_TUNNEL_M)
        .map(|(a, b)| (clamp(a - PORTAL_M), clamp(b + PORTAL_M)))
        .collect()
}

/// Whether the tunnel camera should be on for a rider at `distance`.
pub fn tunnel_camera_wanted(stretches: &[(f64, f64)], distance: f64) -> bool {
    stretches
        .iter()
        .any(|&(a, b)| distance >= a - ENTER_AHEAD_M && distance <= b + LEAVE_AFTER_M)
}

/// How far into a tunnel direct sunlight still reaches (m), past the
/// portal; deeper inside, the hill above shades everything.
pub const SUN_REACH_M: f64 = 25.0;

/// Share of direct sunlight at `distance` along the track (1 outside a
/// tunnel, 0 deeper than [`SUN_REACH_M`] inside, linear between). In the
/// renderer only the tube casts shadows (terrain does not, for speed), so
/// a low sun would otherwise shine through the hillside into the tube onto
/// the rider and the markings, in shadow-map squares at the tube's thin
/// shell (user, 2026-11-13, with the moving sun of spec 2.33).
pub fn daylight_share(stretches: &[(f64, f64)], distance: f64) -> f32 {
    stretches
        .iter()
        .map(|&(a, b)| {
            // The covered stretch starts PORTAL_M inside the tube's ends.
            let depth = (distance - (a + PORTAL_M)).min((b - PORTAL_M) - distance);
            (1.0 - depth / SUN_REACH_M).clamp(0.0, 1.0) as f32
        })
        .fold(1.0, f32::min)
}

/// Eye and look-at point of the camera in a tunnel: on the road (half the
/// rider's lateral offset) behind the rider, inside the tube even in a
/// curve. At the portals the terrain inside the tube is cut away
/// ([`crate::tunnel_cut`]), so the eye stays there too (it used to dive
/// under the hillside, close behind the rider, which still showed the
/// hillside for a few frames and then jumped).
pub fn tunnel_eye(track: &dyn Track, distance: f64, lateral: f64) -> (Vec3, Vec3) {
    let at = |d: f64| to_vec3(track.sample(d).offset_position(lateral * 0.5));
    let eye = at(distance - TUNNEL_CAM_BEHIND_M) + Vec3::Y * TUNNEL_CAM_HEIGHT_M;
    let target = at(distance + TUNNEL_LOOK_AHEAD_M) + Vec3::Y * 1.2;
    (eye, target)
}

/// Moves `blend` (0 chase … 1 tunnel camera) towards `wanted` over
/// [`BLEND_S`].
pub fn smooth_blend(blend: f32, wanted: bool, dt: f32) -> f32 {
    let goal = if wanted { 1.0 } else { 0.0 };
    let k = 1.0 - (-dt / BLEND_S).exp();
    let b = blend + (goal - blend) * k;
    if (b - goal).abs() < 1e-3 { goal } else { b }
}

/// Cross-section of the tube (lateral offset, height above the road) from
/// the left floor edge over the arch to the right floor edge, for a road
/// `width` wide.
pub fn tube_profile(width: f64) -> Vec<[f64; 2]> {
    let half = width / 2.0 + TUBE_SIDE_M;
    // The floor is the road surface riders stand on, 1 mm under a road
    // strip reaching into the portal (no z-fighting).
    let floor = f64::from(crate::ROAD_SURFACE_LIFT_M) - 0.001;
    let mut p = vec![[-half, floor], [-half, TUBE_WALL_M]];
    const ARCH: usize = 8;
    for k in 1..ARCH {
        let a = std::f64::consts::PI * (1.0 - k as f64 / ARCH as f64);
        p.push([
            half * a.cos(),
            TUBE_WALL_M + (TUBE_CROWN_M - TUBE_WALL_M) * a.sin(),
        ]);
    }
    p.push([half, TUBE_WALL_M]);
    p.push([half, floor]);
    p.push([-half, floor]); // the floor closes the ring
    p
}

/// The axis of `track` from `from` to `to` every [`SCAN_STEP_M`] (a
/// route without a package: the tube follows the track).
pub fn track_axis(track: &dyn Track, from: f64, to: f64) -> Vec<crate::tunnel_mesh::AxisPoint> {
    let n = ((to - from) / SCAN_STEP_M).ceil().max(1.0) as usize;
    (0..=n)
        .map(|i| {
            let d = (from + i as f64 * SCAN_STEP_M).min(to);
            let s = track.sample(d);
            let r = s.right();
            crate::tunnel_mesh::AxisPoint {
                s: d - from,
                pos: to_vec3(s.position) + Vec3::Y * crate::ROAD_SURFACE_LIFT_M,
                right: Vec2::new(r[0] as f32, r[1] as f32),
                width: track.road_width_at(d),
            }
        })
        .collect()
}

/// The tube along `track` from `from` to `to` (a bored tunnel over its
/// whole length, with its portals): the meshes of
/// [`crate::tunnel_mesh::tunnel_meshes`].
pub fn tube_meshes(track: &dyn Track, from: f64, to: f64) -> crate::tunnel_mesh::TunnelMeshes {
    let axis = track_axis(track, from, to);
    let len = axis.last().map_or(0.0, |a| a.s);
    let secs = [ocw_world::tunnels::Section {
        from: crate::tunnel_mesh::COLLAR_M,
        to: (len - crate::tunnel_mesh::COLLAR_M).max(crate::tunnel_mesh::COLLAR_M),
        kind: ocw_world::tunnels::SectionKind::Bored,
    }];
    let seed = (from * 10.0).round() as u64;
    crate::tunnel_mesh::tunnel_meshes(&axis, &secs, seed)
}

/// The tunnels as drawn before 2026-11-28 (a dark tube along the
/// track's covered stretches, the package's tunnels not used), for before
/// / after pictures: `OCW_TUNNELS=legacy` (read once).
pub fn legacy_mode() -> bool {
    static LEGACY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *LEGACY.get_or_init(|| std::env::var("OCW_TUNNELS").is_ok_and(|v| v == "legacy"))
}

/// The tube as drawn before 2026-11-28 (unlit vertex colours: concrete
/// walls, a darker vault with a strip of lamps along the crown, dark
/// asphalt), only for [`legacy_mode`].
pub fn legacy_tube_mesh(track: &dyn Track, from: f64, to: f64) -> Mesh {
    let n = ((to - from) / SCAN_STEP_M).ceil().max(1.0) as usize;
    // Unlit colours (sunlight and tree shadows would reach through the
    // hill): concrete walls, a darker vault with a strip of lamps along
    // the crown, a dark asphalt floor.
    let wall = [0.30, 0.29, 0.27, 1.0];
    let vault = [0.20, 0.19, 0.18, 1.0];
    let lamps = [0.95, 0.85, 0.55, 1.0];
    let floor = [0.09, 0.09, 0.10, 1.0];
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let ring = |d: f64| -> Vec<Vec3> {
        let s = track.sample(d);
        tube_profile(track.road_width_at(d))
            .iter()
            .map(|&[l, h]| to_vec3(s.offset_position(l)) + Vec3::Y * h as f32)
            .collect()
    };
    let mut prev = ring(from);
    for i in 1..=n {
        let d = (from + i as f64 * SCAN_STEP_M).min(to);
        let next = ring(d);
        let segments = prev.len() - 1;
        for k in 0..segments {
            let (a, b, c, e) = (prev[k], prev[k + 1], next[k], next[k + 1]);
            // Inward normal: the ring runs over the top from left to right
            // (b after a), the next ring is ahead (c), so (c - a) × (b - a)
            // points into the tube.
            let normal = (c - a).cross(b - a).normalize_or_zero();
            let base = positions.len() as u32;
            for v in [a, b, c, e] {
                positions.push(v.to_array());
                normals.push(normal.to_array());
                colors.push(match k {
                    _ if k == segments - 1 => floor,
                    0 => wall,
                    _ if k == segments - 2 => wall,
                    _ if k == segments / 2 - 1 || k == segments / 2 => lamps,
                    _ => vault,
                });
            }
            indices.extend([base, base + 1, base + 2, base + 1, base + 3, base + 2]);
        }
        prev = next;
    }
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    mesh
}

/// One tunnel of the package's road graph near the rider: its axis
/// samples (home metres), sections and entities.
struct Structure {
    samples: Vec<ocw_world::tunnels::AxisSample>,
    sections: Vec<ocw_world::tunnels::Section>,
    entities: Vec<Entity>,
}

/// The materials of tunnels (one set for all).
#[derive(Clone)]
struct TunnelMaterials {
    /// The old tube's vertex colours ([`legacy_mode`]).
    legacy: Handle<StandardMaterial>,
    tube: Handle<StandardMaterial>,
    fittings: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    lit: Handle<StandardMaterial>,
}

impl TunnelMaterials {
    fn new(materials: &mut Assets<StandardMaterial>, images: &mut Assets<Image>) -> Self {
        let tex = images.add(crate::tunnel_mesh::tunnel_image());
        Self {
            legacy: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
            // Unlit (sunlight and tree shadows would reach through the
            // hill): the lamps' light is baked into the vertex colours.
            tube: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(tex.clone()),
                unlit: true,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
            fittings: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
            // Lamps, signs and reflectors: brighter than white, so they
            // bloom where the camera has bloom.
            glow: materials.add(StandardMaterial {
                base_color: Color::LinearRgba(LinearRgba::rgb(2.6, 2.4, 2.1)),
                unlit: true,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
            lit: materials.add(StandardMaterial {
                base_color: Color::WHITE,
                base_color_texture: Some(tex),
                perceptual_roughness: 0.95,
                cull_mode: None,
                double_sided: true,
                ..default()
            }),
        }
    }
}

fn spawn_meshes(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &TunnelMaterials,
    m: crate::tunnel_mesh::TunnelMeshes,
) -> Vec<Entity> {
    let mut out = Vec::new();
    for (arrays, mat) in [
        (m.tube, &mats.tube),
        (m.fittings, &mats.fittings),
        (m.glow, &mats.glow),
        (m.lit, &mats.lit),
    ] {
        if arrays.indices.is_empty() {
            continue;
        }
        out.push(
            commands
                .spawn((
                    SceneEntity,
                    Mesh3d(meshes.add(arrays.mesh())),
                    MeshMaterial3d(mat.clone()),
                ))
                .id(),
        );
    }
    out
}

/// The tunnels around the local rider and their tube entities.
#[derive(Resource, Default)]
pub struct Tunnels {
    /// Tunnels (from, to distance) on the local rider's track near the rider.
    pub stretches: Vec<(f64, f64)>,
    /// Their tubes as cut into the terrain near the rider (the portals).
    pub cut: crate::tunnel_cut::TunnelCut,
    /// Gallery stretches (from, to distance) on the local rider's track:
    /// the tunnel camera there too (the sun still comes in).
    pub galleries: Vec<(f64, f64)>,
    /// Spawned tubes by their end points (rounded to 0.1 m).
    tubes: HashMap<[i64; 4], Vec<Entity>>,
    /// The package's tunnels near the rider by (member, edge).
    structures: HashMap<(usize, usize), Structure>,
    materials: Option<TunnelMaterials>,
    /// The tunnel lamps' light on the rider ([`TUNNEL_LIGHT_LM`]).
    light: Option<Entity>,
    since_scan: f32,
    scanned_at: Option<f64>,
}

impl Tunnels {
    /// Forgets everything (a ride switch despawns the tubes with the scene).
    pub fn reset(&mut self) {
        let materials = self.materials.take();
        *self = Self::default();
        self.materials = materials;
    }

    /// Where the tunnel camera is wanted: the tunnels and the galleries.
    pub fn camera_stretches(&self) -> Vec<(f64, f64)> {
        let mut v = self.stretches.clone();
        v.extend(self.galleries.iter().copied());
        v
    }

    /// The package tunnels drawn near the rider (tests, logs).
    pub fn structure_count(&self) -> usize {
        self.structures.len()
    }
}

pub struct TunnelPlugin;

impl Plugin for TunnelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tunnels>()
            .add_systems(Update, (update_tunnels, place_tunnel_light).chain());
    }
}

/// The tunnel lamps light the riders (they are lit like the rest of the
/// world, and at night the sun and sky are gone): one point light over
/// the local rider, fading in with the tunnel ([`daylight_share`]), no
/// shadows (lm).
pub const TUNNEL_LIGHT_LM: f32 = 9_000.0;

#[derive(Component)]
struct TunnelLight;

fn place_tunnel_light(
    mut commands: Commands,
    view: Res<RideView>,
    mut tunnels: ResMut<Tunnels>,
    mut lights: Query<(&mut PointLight, &mut Transform), With<TunnelLight>>,
) {
    let Some(local) = view.local() else {
        return;
    };
    let d = local.state.distance;
    let inside = 1.0 - daylight_share(&tunnels.stretches, d);
    let entity = match tunnels.light {
        Some(e) if lights.get(e).is_ok() => e,
        _ => {
            if inside <= 0.0 {
                return;
            }
            let e = commands
                .spawn((
                    SceneEntity,
                    TunnelLight,
                    PointLight {
                        color: Color::linear_rgb(1.0, 0.9, 0.75),
                        intensity: 0.0,
                        range: 30.0,
                        radius: 0.3,
                        shadow_maps_enabled: false,
                        ..default()
                    },
                    Transform::default(),
                ))
                .id();
            tunnels.light = Some(e);
            return;
        }
    };
    if let Ok((mut light, mut tf)) = lights.get_mut(entity) {
        let track = view.track(local.id);
        let p = track.sample(d + 2.0).position;
        tf.translation = to_vec3(p) + Vec3::Y * (TUBE_WALL_M as f32 + 0.5);
        light.intensity = TUNNEL_LIGHT_LM * inside;
    }
}

fn tube_key(track: &dyn Track, (a, b): (f64, f64)) -> [i64; 4] {
    let (p, q) = (track.sample(a).position, track.sample(b).position);
    [p[0], p[2], q[0], q[2]].map(|v| (v * 10.0).round() as i64)
}

/// A package tunnel: (member, edge index), its samples and sections.
type NearTunnel = (
    (usize, usize),
    Vec<ocw_world::tunnels::AxisSample>,
    Vec<ocw_world::tunnels::Section>,
);

/// The package's tunnels near `at` (home metres): (member, edge index)
/// → samples in home metres and sections: those the world builder stored
/// (section `TUNL`), else classified from the package's own terrain.
fn package_tunnels_near(
    set: &ocw_world::WorldSet,
    at: [f64; 2],
    radius: f64,
    have: &HashMap<(usize, usize), Structure>,
) -> Vec<NearTunnel> {
    let mut out = Vec::new();
    for (mi, m) in set.members().iter().enumerate() {
        let Some(pkg) = m.package() else {
            continue;
        };
        let Some(g) = pkg.graph.as_ref() else {
            continue;
        };
        let local = m.frame.invert(at);
        for (ei, e) in g.edges.iter().enumerate() {
            if !e.tunnel || have.contains_key(&(mi, ei)) {
                continue;
            }
            let near = e
                .points
                .iter()
                .any(|p| (p[0] - local[0]).hypot(p[1] - local[1]) < radius);
            if !near {
                continue;
            }
            let ground = |x: f64, z: f64| pkg.surface_height(x, z);
            let mut samples = ocw_world::tunnels::probe(e, &ground);
            // Only with all its terrain loaded (tried again on the next
            // scan): a tile not streamed in yet leaves gaps.
            let expected = (e.length_m() / ocw_world::tunnels::STEP_M).ceil().max(1.0) as usize + 1;
            if samples.len() < expected {
                continue;
            }
            // As the world builder decided them (section TUNL), else from
            // the terrain.
            let sections = ocw_world::tunnels::TunnelSections::sections_of(
                pkg.look.tunnels.as_ref(),
                e,
                &samples,
            );
            for x in &mut samples {
                x.p = m.frame.apply(x.p);
            }
            out.push(((mi, ei), samples, sections));
        }
    }
    out
}

/// Whether a structure is still within `radius` of `at`.
fn structure_near(st: &Structure, at: [f64; 2], radius: f64) -> bool {
    st.samples
        .iter()
        .any(|x| (x.p[0] - at[0]).hypot(x.p[1] - at[1]) < radius)
}

/// The kind of a structure at its sample `x`.
fn kind_at(st: &Structure, s: f64) -> ocw_world::tunnels::SectionKind {
    st.sections
        .iter()
        .find(|k| s >= k.from - 1e-6 && s <= k.to + 1e-6)
        .map_or(ocw_world::tunnels::SectionKind::Open, |k| k.kind)
}

/// The gallery stretches (from, to distance) of `track` between `from`
/// and `to`: where the road runs under a gallery's roof.
fn gallery_stretches(
    track: &dyn Track,
    structures: &HashMap<(usize, usize), Structure>,
    from: f64,
    to: f64,
) -> Vec<(f64, f64)> {
    use ocw_world::tunnels::SectionKind;
    let galleries: Vec<&ocw_world::tunnels::AxisSample> = structures
        .values()
        .flat_map(|st| {
            st.samples
                .iter()
                .filter(|x| matches!(kind_at(st, x.s), SectionKind::Gallery { .. }))
        })
        .collect();
    if galleries.is_empty() {
        return Vec::new();
    }
    let (lo, hi) = track.extent().unwrap_or((from, to));
    let (from, to) = (from.max(lo), to.min(hi));
    let mut runs: Vec<(f64, f64)> = Vec::new();
    let mut d = (from / SCAN_STEP_M).round() * SCAN_STEP_M;
    while d <= to {
        let p = track.sample(d).position;
        let under = galleries.iter().any(|x| {
            let (dx, dz) = (p[0] - x.p[0], p[2] - x.p[1]);
            dx.hypot(dz) <= x.half.max(SCAN_STEP_M) && (p[1] - x.road_y).abs() < 3.0
        });
        if under {
            match runs.last_mut() {
                Some(r) if d - r.1 <= 3.0 * SCAN_STEP_M => r.1 = d,
                _ => runs.push((d, d)),
            }
        }
        d += SCAN_STEP_M;
    }
    runs
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn update_tunnels(
    mut commands: Commands,
    view: Res<RideView>,
    ground: Option<Res<Ground>>,
    package: Option<Res<crate::package::PackageWorld>>,
    time: Res<Time>,
    mut tunnels: ResMut<Tunnels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut images: ResMut<Assets<Image>>,
) {
    let Some(local) = view.local() else {
        return;
    };
    let d = local.state.distance;
    tunnels.since_scan += time.delta_secs();
    let jumped = tunnels.scanned_at.is_none_or(|s| (s - d).abs() > 50.0);
    let ground_changed = ground.as_ref().is_some_and(|g| g.is_changed());
    if !jumped && !ground_changed && tunnels.since_scan < SCAN_INTERVAL_S {
        return;
    }
    tunnels.since_scan = 0.0;
    tunnels.scanned_at = Some(d);
    let track = view.track(local.id);
    let stretches = match &ground {
        Some(g) => covered_stretches(
            track,
            &|x, z| (g.0)(x, z),
            d - SCAN_BEHIND_M,
            d + SCAN_AHEAD_M,
        ),
        None => Vec::new(),
    };
    let mats = tunnels
        .materials
        .get_or_insert_with(|| TunnelMaterials::new(&mut materials, &mut images))
        .clone();
    let here = track.sample(d).position;
    let here2 = [here[0], here[2]];
    // With a package the tunnels of its road graph are drawn as what they
    // are in the terrain (tube, gallery, open road); without one, a tube
    // along the track's covered stretches.
    let pieces = if let Some(pkg) = package.as_ref().filter(|_| !legacy_mode()) {
        let tunnels = &mut *tunnels;
        tunnels.structures.retain(|_, st| {
            let keep = structure_near(st, here2, DROP_M);
            if !keep {
                for e in &st.entities {
                    commands.entity(*e).try_despawn();
                }
            }
            keep
        });
        for (key, samples, sections) in
            package_tunnels_near(&pkg.set, here2, NEAR_M, &tunnels.structures)
        {
            let width = samples
                .first()
                .map_or(6.0, |x| (x.half - ocw_world::tunnels::SIDE_M) * 2.0);
            let axis: Vec<crate::tunnel_mesh::AxisPoint> = samples
                .iter()
                .map(|x| {
                    crate::tunnel_mesh::AxisPoint::from_sample(x, width, crate::ROAD_SURFACE_LIFT_M)
                })
                .collect();
            let seed = (key.0 as u64) << 32 | key.1 as u64;
            // An end deep under the terrain goes on in the next edge.
            let open_end =
                |x: Option<&ocw_world::tunnels::AxisSample>| x.is_none_or(|x| x.roof_cover() < 1.0);
            let portals = [open_end(samples.first()), open_end(samples.last())];
            let m = crate::tunnel_mesh::tunnel_meshes_with(&axis, &sections, seed, portals);
            let entities = spawn_meshes(&mut commands, &mut meshes, &mats, m);
            tunnels.structures.insert(
                key,
                Structure {
                    samples,
                    sections,
                    entities,
                },
            );
        }
        // The old tubes along the track are not needed.
        for (_, es) in tunnels.tubes.drain() {
            for e in es {
                commands.entity(e).try_despawn();
            }
        }
        tunnels.galleries = gallery_stretches(
            track,
            &tunnels.structures,
            d - SCAN_BEHIND_M,
            d + SCAN_AHEAD_M,
        );
        match &ground {
            Some(g) => crate::tunnel_cut::structure_cut_pieces(
                tunnels
                    .structures
                    .values()
                    .map(|st| (st.samples.as_slice(), st.sections.as_slice())),
                &|x, z| (g.0)(x, z),
                here2,
            ),
            None => Vec::new(),
        }
    } else {
        let wanted: HashMap<[i64; 4], (f64, f64)> =
            stretches.iter().map(|&s| (tube_key(track, s), s)).collect();
        tunnels.tubes.retain(|key, es| {
            let keep = wanted.contains_key(key);
            if !keep {
                for e in es.iter() {
                    commands.entity(*e).try_despawn();
                }
            }
            keep
        });
        for (key, (a, b)) in wanted {
            if tunnels.tubes.contains_key(&key) {
                continue;
            }
            let es = if legacy_mode() {
                let mat = mats.legacy.clone();
                vec![
                    commands
                        .spawn((
                            SceneEntity,
                            Mesh3d(meshes.add(legacy_tube_mesh(track, a, b))),
                            MeshMaterial3d(mat),
                        ))
                        .id(),
                ]
            } else {
                spawn_meshes(&mut commands, &mut meshes, &mats, tube_meshes(track, a, b))
            };
            tunnels.tubes.insert(key, es);
        }
        match &ground {
            Some(g) => crate::tunnel_cut::cut_pieces(track, &|x, z| (g.0)(x, z), &stretches, d),
            None => Vec::new(),
        }
    };
    let cut = crate::tunnel_cut::TunnelCut::new(&pieces);
    if cut != tunnels.cut {
        tunnels.cut = cut;
    }
    if stretches != tunnels.stretches {
        info!(
            "tunnels near the rider (km): {:?}; package tunnels drawn: {}",
            stretches
                .iter()
                .map(|(a, b)| format!("{:.3}-{:.3}", a / 1000.0, b / 1000.0))
                .collect::<Vec<_>>(),
            tunnels.structures.len()
        );
    }
    tunnels.stretches = stretches;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    /// A straight 2 km road along +x at 100 m.
    fn road() -> Route {
        let pts = (0..=400)
            .map(|i| [f64::from(i) * 5.0, 100.0, 0.0])
            .collect();
        Route::from_points("flat", "Flat", pts, false, 6.0).unwrap()
    }

    /// A hill over x 800 … 1200 (the road in a tunnel there), conformed
    /// terrain (just below the road) elsewhere.
    fn hill(x: f64, _z: f64) -> Option<f64> {
        Some(if (800.0..=1200.0).contains(&x) {
            140.0
        } else {
            99.7
        })
    }

    #[test]
    fn finds_the_tunnel_with_its_portals() {
        let r = road();
        let s = covered_stretches(&r, &hill, 0.0, 2000.0);
        assert_eq!(s.len(), 1, "{s:?}");
        let (a, b) = s[0];
        assert!((a - (800.0 - PORTAL_M)).abs() <= SCAN_STEP_M, "{a}");
        assert!((b - (1200.0 + PORTAL_M)).abs() <= SCAN_STEP_M, "{b}");
        // No ground, no tunnel; conformed terrain everywhere, no tunnel.
        assert!(covered_stretches(&r, &|_, _| None, 0.0, 2000.0).is_empty());
        assert!(covered_stretches(&r, &|_, _| Some(99.7), 0.0, 2000.0).is_empty());
    }

    #[test]
    fn a_tunnel_cut_by_the_window_is_followed_to_its_ends() {
        let r = road();
        let s = covered_stretches(&r, &hill, 900.0, 1000.0);
        assert_eq!(s.len(), 1, "{s:?}");
        assert!(s[0].0 < 800.0 && s[0].1 > 1200.0, "{s:?}");
        // Scans from other windows (a moving rider) find the same tunnel
        // exactly, so its tube is not rebuilt.
        for from in [0.0, 333.3, 700.7, 1001.1] {
            assert_eq!(covered_stretches(&r, &hill, from, from + 900.0), s);
        }
    }

    #[test]
    fn short_bumps_are_ignored_and_short_gaps_bridged() {
        let r = road();
        // A 4 m bump of terrain over the road: an artefact, not a tunnel.
        let bump = |x: f64, _: f64| {
            Some(if (500.0..504.0).contains(&x) {
                101.0
            } else {
                99.7
            })
        };
        assert!(covered_stretches(&r, &bump, 0.0, 2000.0).is_empty());
        // Two hills with a 10 m dip between them: one tunnel.
        let two = |x: f64, _: f64| {
            Some(
                if (800.0..=900.0).contains(&x) || (910.0..=1000.0).contains(&x) {
                    120.0
                } else {
                    99.7
                },
            )
        };
        assert_eq!(covered_stretches(&r, &two, 0.0, 2000.0).len(), 1);
    }

    #[test]
    fn tunnel_camera_takes_over_before_the_portal_and_hands_back_after() {
        let s = [(796.0, 1204.0)];
        assert!(!tunnel_camera_wanted(&s, 796.0 - ENTER_AHEAD_M - 1.0));
        assert!(tunnel_camera_wanted(&s, 790.0));
        assert!(tunnel_camera_wanted(&s, 1000.0));
        assert!(tunnel_camera_wanted(&s, 1210.0));
        assert!(!tunnel_camera_wanted(&s, 1204.0 + LEAVE_AFTER_M + 1.0));
        // The blend is smooth: half-way after about BLEND_S · ln 2.
        let mut b = 0.0;
        for _ in 0..17 {
            b = smooth_blend(b, true, 1.0 / 60.0);
        }
        assert!((0.4..0.6).contains(&b), "{b}");
        for _ in 0..600 {
            b = smooth_blend(b, true, 1.0 / 60.0);
        }
        assert_eq!(b, 1.0);
    }

    /// No direct sun deep in a tunnel; full sun outside; a short fade
    /// behind each portal.
    #[test]
    fn no_direct_sun_inside_a_tunnel() {
        let r = road();
        let s = covered_stretches(&r, &hill, 0.0, 2000.0);
        assert_eq!(daylight_share(&s, 500.0), 1.0);
        assert_eq!(daylight_share(&s, 1500.0), 1.0);
        assert_eq!(daylight_share(&[], 1000.0), 1.0);
        for d in [850.0, 1000.0, 1150.0] {
            assert_eq!(daylight_share(&s, d), 0.0, "{d}");
        }
        let (a, b) = s[0];
        assert_eq!(daylight_share(&s, a), 1.0);
        assert_eq!(daylight_share(&s, b), 1.0);
        let half = daylight_share(&s, a + PORTAL_M + SUN_REACH_M / 2.0);
        assert!((half - 0.5).abs() < 1e-3, "{half}");
        // Darker the deeper in.
        let mut last = 1.0;
        for i in 0..60 {
            let v = daylight_share(&s, a + f64::from(i));
            assert!(v <= last);
            last = v;
        }
    }

    #[test]
    fn the_tunnel_eye_is_inside_the_tube_behind_the_rider() {
        let r = road();
        let (eye, target) = tunnel_eye(&r, 1000.0, 1.0);
        assert!((eye.x - (1000.0 - TUNNEL_CAM_BEHIND_M as f32)).abs() < 1e-3);
        assert!(eye.y > 100.0 + 1.0 && eye.y < 100.0 + TUBE_WALL_M as f32);
        assert!(eye.z.abs() < r.road_width_m() as f32 / 2.0);
        assert!(target.x > 1000.0 && target.y < 100.0 + TUBE_WALL_M as f32);
    }

    /// At the entry the hillside rises over the road at 30 %: the eye
    /// stays at its place behind the rider inside the tube (where the
    /// terrain is cut away), no jump close to the rider.
    #[test]
    fn at_the_entry_portal_the_eye_stays_behind_the_rider() {
        let r = road();
        for i in 0..40 {
            let d = 800.0 + f64::from(i) * 0.5;
            let (eye, _) = tunnel_eye(&r, d, 0.0);
            assert!((eye.x - (d - TUNNEL_CAM_BEHIND_M) as f32).abs() < 1e-3);
            assert!((eye.y - (100.0 + TUNNEL_CAM_HEIGHT_M)).abs() < 1e-3);
        }
    }

    #[test]
    fn the_tube_encloses_the_road() {
        let r = road();
        let m = tube_meshes(&r, 800.0, 820.0);
        let pos = &m.tube.positions;
        let half = (3.0 + TUBE_SIDE_M) as f32;
        for p in pos {
            assert!((800.0..=820.0).contains(&p[0]), "{p:?}");
            assert!(p[2].abs() <= half + 1e-3, "{p:?}");
            assert!(
                p[1] >= 99.9 && p[1] <= 100.0 + TUBE_CROWN_M as f32 + 0.05,
                "{p:?}"
            );
        }
        // The crown is reached (the arch closes over the road).
        assert!(pos.iter().any(|p| p[1] > 100.0 + TUBE_CROWN_M as f32 - 0.1));
        // Normals point into the tube: at the crown down, on the floor up.
        for (p, n) in pos.iter().zip(&m.tube.normals) {
            if p[1] > 100.0 + TUBE_CROWN_M as f32 - 0.2 {
                assert!(n[1] < 0.0, "{p:?} {n:?}");
            }
            if p[1] < 100.05 && n[1].abs() > 0.5 {
                assert!(n[1] > 0.0, "{p:?} {n:?}");
            }
        }
        // UVs in metres for the texture, lamps and portals.
        assert_eq!(m.tube.uvs.len(), pos.len());
        assert!(!m.glow.indices.is_empty() && !m.lit.indices.is_empty());
    }
}

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

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
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
/// [`TUBE_CROWN_M`], and [`TUBE_SIDE_M`] beside the road edge.
pub const TUBE_WALL_M: f64 = 4.0;
pub const TUBE_CROWN_M: f64 = 6.0;
pub const TUBE_SIDE_M: f64 = 1.0;
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
    let floor = -0.05;
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

/// Mesh of the tube along `track` from `from` to `to`: one quad per profile
/// segment and step, flat-shaded with the normal pointing inwards. The
/// floor is dark asphalt (package roads have no strip in a tunnel).
pub fn tube_mesh(track: &dyn Track, from: f64, to: f64) -> Mesh {
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
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    mesh.insert_indices(Indices::U32(indices));
    mesh
}

/// The tunnels around the local rider and their tube entities.
#[derive(Resource, Default)]
pub struct Tunnels {
    /// Tunnels (from, to distance) on the local rider's track near the rider.
    pub stretches: Vec<(f64, f64)>,
    /// Their tubes as cut into the terrain near the rider (the portals).
    pub cut: crate::tunnel_cut::TunnelCut,
    /// Spawned tubes by their end points (rounded to 0.1 m).
    tubes: HashMap<[i64; 4], Entity>,
    since_scan: f32,
    scanned_at: Option<f64>,
}

impl Tunnels {
    /// Forgets everything (a ride switch despawns the tubes with the scene).
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

pub struct TunnelPlugin;

impl Plugin for TunnelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Tunnels>()
            .add_systems(Update, update_tunnels);
    }
}

fn tube_key(track: &dyn Track, (a, b): (f64, f64)) -> [i64; 4] {
    let (p, q) = (track.sample(a).position, track.sample(b).position);
    [p[0], p[2], q[0], q[2]].map(|v| (v * 10.0).round() as i64)
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn update_tunnels(
    mut commands: Commands,
    view: Res<RideView>,
    ground: Option<Res<Ground>>,
    time: Res<Time>,
    mut tunnels: ResMut<Tunnels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut material: Local<Option<Handle<StandardMaterial>>>,
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
    let pieces = match &ground {
        Some(g) => crate::tunnel_cut::cut_pieces(track, &|x, z| (g.0)(x, z), &stretches, d),
        None => Vec::new(),
    };
    let cut = crate::tunnel_cut::TunnelCut::new(&pieces);
    if cut != tunnels.cut {
        tunnels.cut = cut;
    }
    let wanted: HashMap<[i64; 4], (f64, f64)> =
        stretches.iter().map(|&s| (tube_key(track, s), s)).collect();
    tunnels.tubes.retain(|key, entity| {
        let keep = wanted.contains_key(key);
        if !keep {
            commands.entity(*entity).try_despawn();
        }
        keep
    });
    for (key, (a, b)) in wanted {
        if tunnels.tubes.contains_key(&key) {
            continue;
        }
        let mat = material
            .get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: Color::WHITE, // vertex colours
                    unlit: true,
                    cull_mode: None,
                    double_sided: true,
                    ..default()
                })
            })
            .clone();
        let entity = commands
            .spawn((
                SceneEntity,
                Mesh3d(meshes.add(tube_mesh(track, a, b))),
                MeshMaterial3d(mat),
            ))
            .id();
        tunnels.tubes.insert(key, entity);
    }
    if stretches != tunnels.stretches {
        info!(
            "tunnels near the rider (km): {:?}",
            stretches
                .iter()
                .map(|(a, b)| format!("{:.3}-{:.3}", a / 1000.0, b / 1000.0))
                .collect::<Vec<_>>()
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
        let mesh = tube_mesh(&r, 800.0, 820.0);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(pos)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("positions");
        };
        let half = (3.0 + TUBE_SIDE_M) as f32;
        for p in pos {
            assert!((800.0..=820.0).contains(&p[0]), "{p:?}");
            assert!(p[2].abs() <= half + 1e-3, "{p:?}");
            assert!(
                p[1] >= 99.9 && p[1] <= 100.0 + TUBE_CROWN_M as f32 + 1e-3,
                "{p:?}"
            );
        }
        // The crown is reached (the arch closes over the road).
        assert!(pos.iter().any(|p| p[1] > 100.0 + TUBE_CROWN_M as f32 - 0.1));
        // Normals point into the tube: at the crown down, on the floor up.
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(nrm)) =
            mesh.attribute(Mesh::ATTRIBUTE_NORMAL)
        else {
            panic!("normals");
        };
        for (p, n) in pos.iter().zip(nrm) {
            if p[1] > 100.0 + TUBE_CROWN_M as f32 - 0.2 {
                assert!(n[1] < 0.0, "{p:?} {n:?}");
            }
            if p[1] < 100.0 && n[1].abs() > 0.5 {
                assert!(n[1] > 0.0, "{p:?} {n:?}");
            }
        }
    }
}

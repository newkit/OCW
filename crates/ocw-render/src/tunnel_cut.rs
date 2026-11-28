//! Tunnel portals cut into the terrain (spec §3.2 "Camera and avatar",
//! §4.3).
//!
//! The terrain of a world package is a 10 m height grid that is not cut
//! at tunnels: at a portal the hillside slopes through the tunnel tube
//! ([`crate::tunnel`]) instead of forming a vertical face, so riding into
//! a tunnel the camera saw a dark-green wall of (forest-coloured) terrain
//! inside the tube instead of the tube's interior. The terrain is therefore
//! drawn with [`TerrainMaterial`]: a [`StandardMaterial`] whose fragment
//! shader (`ground.wesl`, which also draws the ground materials,
//! [`crate::ground`]) discards every terrain fragment inside the tube near
//! the rider. The tube is given as short straight [`CutPiece`]s
//! along the track where the terrain can reach into it (the portals);
//! inside the hill, where the terrain is well above the crown, there is
//! nothing to cut. [`inside`] is the CPU twin of the shader, for the tests.

use bevy::asset::embedded_asset;
use bevy::pbr::MaterialPlugin;
use bevy::prelude::*;
use ocw_world::Track;

pub use crate::ground::{TerrainMaterial, terrain_material};
use crate::tunnel::{TUBE_CROWN_M, TUBE_SIDE_M, TUBE_WALL_M};

/// At most this many pieces are cut at a time (the nearest to the rider).
pub const MAX_CUT_PIECES: usize = 64;
/// In front of a bored section's portals the tube's profile is cut over
/// this length too (m): the camera follows the rider there.
pub const PORTAL_APPROACH_M: f64 = 6.0;
/// Length of a piece along the track (m); pieces overlap a little.
pub const CUT_STEP_M: f64 = 2.0;
/// Pieces are cut where the terrain beside or above the road is less than
/// this far above the tube's crown (m): there it may reach into the tube.
pub const CUT_COVER_M: f64 = 1.5;
/// The cut is this much larger than the tube (m), so no terrain is left
/// on the tube's inside.
const CUT_MARGIN_M: f64 = 0.03;

/// One straight piece of a tunnel tube.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CutPiece {
    /// The road centre in the middle of the piece.
    pub centre: Vec3,
    /// Horizontal unit direction of the road.
    pub direction: Vec2,
    /// Half the length along the road (m).
    pub half_length: f32,
    /// Half the tube's width (m).
    pub half_width: f32,
    /// Gradient of the road (rise over run).
    pub grade: f32,
}

/// The cut pieces (the uniform at binding 100 of the terrain material,
/// [`crate::ground::GroundExtension`]).
#[derive(Reflect, Debug, Clone, PartialEq)]
pub struct TunnelCut {
    /// x: number of pieces; y: tube wall height; z: crown height (m).
    pub shape: Vec4,
    /// Two per piece: (centre xyz, half length), (direction xz, half
    /// width, grade).
    pub pieces: [Vec4; 2 * MAX_CUT_PIECES],
}

impl Default for TunnelCut {
    fn default() -> Self {
        Self::new(&[])
    }
}

impl TunnelCut {
    /// The uniform for `pieces` (at most [`MAX_CUT_PIECES`]).
    pub fn new(pieces: &[CutPiece]) -> Self {
        let mut packed = [Vec4::ZERO; 2 * MAX_CUT_PIECES];
        let n = pieces.len().min(MAX_CUT_PIECES);
        for (i, p) in pieces.iter().take(n).enumerate() {
            packed[2 * i] = p.centre.extend(p.half_length);
            packed[2 * i + 1] = Vec4::new(p.direction.x, p.direction.y, p.half_width, p.grade);
        }
        Self {
            shape: Vec4::new(
                n as f32,
                (TUBE_WALL_M + CUT_MARGIN_M) as f32,
                (TUBE_CROWN_M + CUT_MARGIN_M) as f32,
                0.0,
            ),
            pieces: packed,
        }
    }

    pub fn len(&self) -> usize {
        self.shape.x as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Whether a terrain fragment at `p` is inside one of the tubes (and is
/// not drawn); mirrors `ground.wesl`. The tube's cross-section: walls
/// up to the wall height, then a half ellipse up to the crown, from the
/// road surface up.
pub fn inside(p: Vec3, cut: &TunnelCut) -> bool {
    let (wall, crown) = (cut.shape.y, cut.shape.z);
    (0..cut.len()).any(|i| {
        let a = cut.pieces[2 * i];
        let b = cut.pieces[2 * i + 1];
        let d = p - a.truncate();
        let dir = Vec3::new(b.x, 0.0, b.y);
        let along = d.dot(dir);
        if along.abs() > a.w {
            return false;
        }
        let lateral = d.dot(Vec3::new(-b.y, 0.0, b.x)).abs() / b.z;
        let h = d.y - along * b.w;
        if lateral > 1.0 || h <= 0.0 {
            return false;
        }
        if h <= wall {
            return true;
        }
        let e = (h - wall) / (crown - wall);
        lateral * lateral + e * e <= 1.0
    })
}

/// The pieces to cut for the tunnels `stretches` (from, to distance, as
/// [`crate::tunnel::covered_stretches`] finds them) of `track`: every
/// [`CUT_STEP_M`] where the terrain over the road or at the tube's sides is
/// less than [`CUT_COVER_M`] above the crown, the [`MAX_CUT_PIECES`]
/// nearest to the rider at `rider_m`.
pub fn cut_pieces(
    track: &dyn Track,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
    stretches: &[(f64, f64)],
    rider_m: f64,
) -> Vec<CutPiece> {
    let mut out: Vec<(f64, CutPiece)> = Vec::new();
    for &(from, to) in stretches {
        let n = ((to - from) / CUT_STEP_M).ceil().max(1.0) as usize;
        for i in 0..n {
            let d = (from + (i as f64 + 0.5) * CUT_STEP_M).min(to);
            let s = track.sample(d);
            let half = track.road_width_at(d) / 2.0 + TUBE_SIDE_M + CUT_MARGIN_M;
            let low = [-half, 0.0, half]
                .into_iter()
                .filter_map(|l| {
                    let q = s.offset_position(l);
                    ground(q[0], q[2]).map(|h| h - s.position[1])
                })
                .fold(f64::INFINITY, f64::min);
            if low >= TUBE_CROWN_M + CUT_COVER_M {
                continue;
            }
            let piece = CutPiece {
                centre: crate::to_vec3(s.position),
                direction: Vec2::new(s.direction[0] as f32, s.direction[1] as f32),
                // A little overlap, so no sliver stays between two pieces.
                half_length: (CUT_STEP_M / 2.0 + 0.1) as f32,
                half_width: half as f32,
                grade: s.grade as f32,
            };
            out.push(((d - rider_m).abs(), piece));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.truncate(MAX_CUT_PIECES);
    out.into_iter().map(|(_, p)| p).collect()
}

/// Pieces of a straight run are joined up to this length (m), so a long
/// gallery fits into [`MAX_CUT_PIECES`].
pub const MAX_PIECE_M: f64 = 24.0;

/// The pieces to cut for the package's tunnels near the rider at `at`
/// (each its axis samples and sections, [`ocw_world::tunnels`]): where a
/// bored stretch's terrain beside or over the road is less than
/// [`CUT_COVER_M`] above the crown (the portals) and along galleries;
/// straight runs joined up to [`MAX_PIECE_M`], the nearest
/// [`MAX_CUT_PIECES`] to the rider.
pub fn structure_cut_pieces<'a>(
    structures: impl Iterator<
        Item = (
            &'a [ocw_world::tunnels::AxisSample],
            &'a [ocw_world::tunnels::Section],
        ),
    >,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
    at: [f64; 2],
) -> Vec<CutPiece> {
    use ocw_world::tunnels::SectionKind;
    let mut out: Vec<(f64, CutPiece)> = Vec::new();
    for (samples, sections) in structures {
        let kind = |s: f64| {
            sections
                .iter()
                .find(|k| s >= k.from - 1e-6 && s <= k.to + 1e-6)
                .map_or(SectionKind::Open, |k| k.kind)
        };
        let bored: Vec<(f64, f64)> = sections
            .iter()
            .filter(|k| k.kind == SectionKind::Bored)
            .map(|k| (k.from, k.to))
            .collect();
        // In front of a portal: the camera behind a rider entering or
        // leaving the tube is there, and the mound's slope reaches into
        // the tube's profile (the Arcalís portal, 2026-11-28: it hid the
        // rider for half a second).
        let approach = |s: f64| {
            bored.iter().any(|&(a, b)| {
                (s >= a - PORTAL_APPROACH_M && s < a) || (s > b && s <= b + PORTAL_APPROACH_M)
            })
        };
        let low_cover = |x: &ocw_world::tunnels::AxisSample| {
            let low = [-x.half, 0.0, x.half]
                .into_iter()
                .filter_map(|l| {
                    let q = x.at(l);
                    ground(q[0], q[1]).map(|h| h - x.road_y)
                })
                .fold(f64::INFINITY, f64::min);
            low < TUBE_CROWN_M + CUT_COVER_M
        };
        let wanted: Vec<bool> = samples
            .iter()
            .map(|x| match kind(x.s) {
                SectionKind::Gallery { .. } => true,
                // Boxes lie under laid terrain (cities, road embankments):
                // cutting the tube's arch would open the ground over them.
                SectionKind::Box => false,
                SectionKind::Open => approach(x.s),
                SectionKind::Bored => low_cover(x),
            })
            .collect();
        let mut i = 0;
        while i + 1 < samples.len() {
            if !(wanted[i] || wanted[i + 1]) {
                i += 1;
                continue;
            }
            // Extend while straight and wanted.
            let a = samples[i];
            let mut j = i + 1;
            while j + 1 < samples.len()
                && (wanted[j] || wanted[j + 1])
                && samples[j + 1].s - a.s <= MAX_PIECE_M
                && a.dir[0] * samples[j + 1].dir[0] + a.dir[1] * samples[j + 1].dir[1] > 0.9998
            {
                j += 1;
            }
            let b = samples[j];
            let len = (b.p[0] - a.p[0]).hypot(b.p[1] - a.p[1]).max(1e-6);
            let dir = Vec2::new(
                ((b.p[0] - a.p[0]) / len) as f32,
                ((b.p[1] - a.p[1]) / len) as f32,
            );
            let centre = Vec3::new(
                ((a.p[0] + b.p[0]) / 2.0) as f32,
                ((a.road_y + b.road_y) / 2.0) as f32 + crate::ROAD_SURFACE_LIFT_M,
                ((a.p[1] + b.p[1]) / 2.0) as f32,
            );
            let piece = CutPiece {
                centre,
                direction: dir,
                half_length: (len / 2.0 + 0.1) as f32,
                half_width: (a.half + CUT_MARGIN_M) as f32,
                grade: ((b.road_y - a.road_y) / len) as f32,
            };
            let dist = (f64::from(centre.x) - at[0]).hypot(f64::from(centre.z) - at[1]);
            out.push((dist, piece));
            i = j;
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.truncate(MAX_CUT_PIECES);
    out.into_iter().map(|(_, p)| p).collect()
}

pub struct TunnelCutPlugin;

impl Plugin for TunnelCutPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "ground.wesl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default())
            .add_systems(
                PostUpdate,
                update_cuts.run_if(resource_exists::<crate::tunnel::Tunnels>),
            );
    }
}

/// Hands the current cut to every terrain material (also to ones created
/// after the last change).
fn update_cuts(
    tunnels: Res<crate::tunnel::Tunnels>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let cut = &tunnels.cut;
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.extension.cut() != *cut)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.set_cut(cut);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::Route;

    /// A straight road along +x at 100 m, 6 m wide.
    fn road() -> Route {
        let pts = (0..=400)
            .map(|i| [f64::from(i) * 5.0, 100.0, 0.0])
            .collect();
        Route::from_points("flat", "Flat", pts, false, 6.0).unwrap()
    }

    /// A hill rising at 30 % from x = 800 (the entry portal), conformed
    /// terrain before it.
    fn slope(x: f64, _z: f64) -> Option<f64> {
        Some(if x < 800.0 {
            99.7
        } else {
            100.0 + (x - 800.0) * 0.3
        })
    }

    /// Regression (scene `tunnel-video`): at the Steg tunnel's entry the
    /// hillside – a 10 m terrain grid – sloped through the tube and showed
    /// as a dark-green wall inside it. The terrain inside the tube is cut
    /// away at the portal; beside, above and below the tube it stays.
    #[test]
    fn the_terrain_inside_the_tube_is_cut_at_the_portal() {
        let r = road();
        let stretches = crate::tunnel::covered_stretches(&r, &slope, 0.0, 2000.0);
        assert_eq!(stretches.len(), 1, "{stretches:?}");
        let pieces = cut_pieces(&r, &slope, &stretches, 790.0);
        assert!(!pieces.is_empty());
        // Only near the portal: deep in the hill the terrain is far above.
        let cover_at = |x: f32| (x - 800.0) * 0.3;
        assert!(
            pieces.iter().all(|p| cover_at(p.centre.x - p.half_length)
                < (TUBE_CROWN_M + CUT_COVER_M) as f32 + 1.0),
            "{pieces:?}"
        );
        let cut = TunnelCut::new(&pieces);
        let half = (3.0 + TUBE_SIDE_M) as f32;
        // The hillside where it runs through the tube: cut.
        for x in [802.0f32, 805.0, 810.0, 815.0] {
            let h = cover_at(x);
            for z in [-half + 0.2, 0.0, half - 0.2] {
                let p = Vec3::new(x, 100.0 + h, z);
                if h < TUBE_WALL_M as f32 {
                    assert!(inside(p, &cut), "{p}");
                }
            }
        }
        // Beside the tube, under the road and above the crown: kept.
        assert!(!inside(Vec3::new(805.0, 101.0, half + 0.5), &cut));
        assert!(!inside(Vec3::new(805.0, 99.9, 0.0), &cut));
        assert!(!inside(
            Vec3::new(805.0, 100.0 + TUBE_CROWN_M as f32 + 0.2, 0.0),
            &cut
        ));
        // In the arch's corner (above the wall, near the side): kept.
        assert!(!inside(
            Vec3::new(805.0, 100.0 + TUBE_CROWN_M as f32 - 0.1, half - 0.1),
            &cut
        ));
        // No tunnel, nothing cut.
        assert!(cut_pieces(&r, &slope, &[], 790.0).is_empty());
        assert!(!inside(Vec3::new(805.0, 101.0, 0.0), &TunnelCut::default()));
    }

    /// Regression (the Arcalís portal, 2026-11-28): the mound's slope in
    /// front of a portal reached into the tube's profile where the camera
    /// follows the rider in, and hid the rider for half a second. The
    /// profile is cut over [`PORTAL_APPROACH_M`] in front of the portal too,
    /// not further out.
    #[test]
    fn the_mound_in_front_of_a_portal_is_cut_where_the_camera_follows() {
        use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, VertexId};
        use ocw_world::tunnels::{Section, SectionKind};
        let e = Edge {
            id: EdgeId(1),
            from: VertexId(0),
            to: VertexId(1),
            points: vec![[0.0, 0.0], [300.0, 0.0]],
            class: RoadClass::Secondary,
            name: None,
            width_m: 6.0,
            bridge: false,
            tunnel: true,
            dead_end_m: None,
            boundary: false,
            oneway: Oneway::No,
            osm_way: 7,
            profile: vec![100.0; 61],
        };
        // Laid under the road, the mound rising at 1:1.5 from x 92 to the
        // portal at 100, the hill beyond.
        let ground = |x: f64, _z: f64| {
            Some(if x < 92.0 {
                99.7
            } else {
                (100.0 + (x - 92.0) / 1.5).max(99.7)
            })
        };
        let samples = ocw_world::tunnels::probe(&e, &ground);
        let secs = [
            Section {
                from: 0.0,
                to: 100.0,
                kind: SectionKind::Open,
            },
            Section {
                from: 100.0,
                to: 300.0,
                kind: SectionKind::Bored,
            },
        ];
        let pieces = structure_cut_pieces(
            std::iter::once((&samples[..], &secs[..])),
            &ground,
            [97.0, 0.0],
        );
        let cut = TunnelCut::new(&pieces);
        // The slope 3 m in front of the portal, 2 m over the road: cut.
        assert!(inside(Vec3::new(97.0, 102.0, 0.0), &cut), "{pieces:?}");
        // 20 m out on the open road: nothing cut.
        assert!(!inside(Vec3::new(80.0, 101.0, 0.0), &cut));
    }

    #[test]
    fn the_cut_follows_the_road_gradient_and_keeps_the_nearest_pieces() {
        // A 10 % climb along +x: the cut is measured from the road surface
        // at each point, not from the piece's middle.
        let pts = (0..=200)
            .map(|i| [f64::from(i) * 5.0, f64::from(i) * 0.5, 0.0])
            .collect();
        let r = Route::from_points("climb", "Climb", pts, false, 6.0).unwrap();
        let piece = CutPiece {
            centre: Vec3::new(500.0, 50.0, 0.0),
            direction: Vec2::X,
            half_length: 1.1,
            half_width: 4.0,
            grade: 0.1,
        };
        let cut = TunnelCut::new(&[piece]);
        assert!(inside(Vec3::new(501.0, 50.1 + 0.2, 0.0), &cut));
        assert!(!inside(Vec3::new(501.0, 50.1 - 0.05, 0.0), &cut));
        // Many tunnels: at most MAX_CUT_PIECES, the nearest.
        let low = |_: f64, _: f64| Some(-10.0);
        let all = cut_pieces(&r, &low, &[(0.0, 1000.0)], 500.0);
        assert_eq!(all.len(), MAX_CUT_PIECES);
        assert!(all.iter().all(|p| (p.centre.x - 500.0).abs() < 70.0));
    }

    #[test]
    fn every_terrain_material_gets_the_cut() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<TerrainMaterial>()
            .init_resource::<crate::tunnel::Tunnels>()
            .add_systems(Update, update_cuts);
        let a = app
            .world_mut()
            .resource_mut::<Assets<TerrainMaterial>>()
            .add(terrain_material(
                StandardMaterial::default(),
                &TunnelCut::default(),
            ));
        let piece = CutPiece {
            centre: Vec3::ZERO,
            direction: Vec2::X,
            half_length: 1.0,
            half_width: 4.0,
            grade: 0.0,
        };
        app.world_mut().resource_mut::<crate::tunnel::Tunnels>().cut = TunnelCut::new(&[piece]);
        app.update();
        let mats = app.world().resource::<Assets<TerrainMaterial>>();
        assert_eq!(mats.get(&a).unwrap().extension.cut().len(), 1);
    }
}

//! Tunnel portals cut into the terrain (spec §3.2 "Camera and avatar",
//! §4.3).
//!
//! The terrain of a world package is a 10 m height grid that is not cut
//! at tunnels: at a portal the hillside slopes through the tunnel tube
//! ([`crate::tunnel`]) instead of forming a vertical face, so riding into
//! a tunnel the camera saw a dark-green wall of (forest-coloured) terrain
//! inside the tube instead of the tube's interior. The terrain is therefore
//! drawn with [`TerrainMaterial`]: a [`StandardMaterial`] whose fragment
//! shader (`tunnel_cut.wesl`) discards every terrain fragment inside the
//! tube near the rider. The tube is given as short straight [`CutPiece`]s
//! along the track where the terrain can reach into it (the portals);
//! inside the hill, where the terrain is well above the crown, there is
//! nothing to cut. [`inside`] is the CPU twin of the shader, for the tests.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use ocw_world::Track;

use crate::tunnel::{TUBE_CROWN_M, TUBE_SIDE_M, TUBE_WALL_M};

/// The terrain's material: standard, with the tunnel tubes cut out.
pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TunnelCut>;

/// At most this many pieces are cut at a time (the nearest to the rider).
pub const MAX_CUT_PIECES: usize = 64;
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

/// The cut pieces (one uniform, binding 100 after the standard material's
/// bindings).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, PartialEq)]
pub struct TunnelCut {
    /// x: number of pieces; y: tube wall height; z: crown height (m).
    #[uniform(100)]
    pub shape: Vec4,
    /// Two per piece: (centre xyz, half length), (direction xz, half
    /// width, grade).
    #[uniform(100)]
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

impl MaterialExtension for TunnelCut {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("tunnel_cut.wesl")).with_source("embedded"),
        )
    }
}

/// A terrain material on top of `base`, cut by `cut`.
pub fn terrain_material(base: StandardMaterial, cut: &TunnelCut) -> TerrainMaterial {
    ExtendedMaterial {
        base,
        extension: cut.clone(),
    }
}

/// Whether a terrain fragment at `p` is inside one of the tubes (and is
/// not drawn); mirrors `tunnel_cut.wesl`. The tube's cross-section: walls
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

pub struct TunnelCutPlugin;

impl Plugin for TunnelCutPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "tunnel_cut.wesl");
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
        .filter(|(_, m)| m.extension != *cut)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension = cut.clone();
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
        assert_eq!(mats.get(&a).unwrap().extension.len(), 1);
    }
}

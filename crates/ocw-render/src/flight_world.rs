//! The world around god mode's flight (spec §14 Phase 13B, 13D):
//!
//! - **Tree crowns** ([`TreeCrowns`]) as obstacles of the flight only
//!   ([`FlightObstacles`]): the unicorn keeps its clearance over them, the
//!   camera still sees through trees (they fade, `see_through`).
//! - **Look-ahead streaming** ([`stream_focus`]): the tiles stream around
//!   the unicorn in a radius that grows with the height above the ground,
//!   and around a point ahead along the flight; the neighbour packages
//!   load within 3 km plus 10 s of flight ([`neighbour_reach_m`], §4.7).
//! - **Details by altitude** ([`details_shown`]): trees and buildings are
//!   drawn only below [`DETAIL_CEILING_M`] above the ground; higher up the
//!   terrain and its colour map remain.
//!
//! All decisions are pure functions of the [`RideView`], tested here.

use bevy::prelude::*;

use crate::RideView;
use crate::camera::CameraObstacles;
use crate::package::{STREAM_RADIUS, TreeSite};

/// Obstacles that only the flight avoids (not the camera): the tree
/// crowns of the loaded tiles, a layer per tile like
/// [`CameraObstacles`]'s buildings.
#[derive(Resource, Default, Clone)]
pub struct FlightObstacles(pub CameraObstacles);

/// Trees and buildings are drawn only below this height above the ground
/// (m, spec §14 Phase 13D).
pub const DETAIL_CEILING_M: f64 = 1_500.0;

/// Whether trees and buildings are drawn: always while riding, in flight
/// below [`DETAIL_CEILING_M`] above the ground.
pub fn details_shown(view: &RideView) -> bool {
    view.flight
        .as_ref()
        .is_none_or(|f| f.height_m < DETAIL_CEILING_M)
}

/// A tree or building mesh of a streamed tile (hidden at altitude).
#[derive(Component)]
pub struct TileDetail;

/// Hides the trees and buildings high up, shows them again lower down.
pub(crate) fn show_details(
    view: Res<RideView>,
    mut details: Query<&mut Visibility, With<TileDetail>>,
) {
    let want = if details_shown(&view) {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut details {
        if *v != want {
            *v = want;
        }
    }
}

/// The tree crowns of a tile with a grid index: each a vertical cylinder
/// of the crown's radius up to its tip ([`crate::package::CROWN`] scaled
/// by the tree).
#[derive(Debug, Clone, Default)]
pub struct TreeCrowns {
    /// x, z, radius, top.
    trees: Vec<[f32; 4]>,
    origin: [f32; 2],
    cols: usize,
    rows: usize,
    grid: Vec<Vec<u32>>,
}

/// Grid cell of the [`TreeCrowns`] index (m).
const CROWN_CELL_M: f32 = 16.0;

impl TreeCrowns {
    /// From the tree sites of a tile (the foot of the trunk and the scale).
    pub fn from_sites(sites: &[TreeSite]) -> Self {
        let (_, tip, radius) = crate::package::CROWN;
        let trees: Vec<[f32; 4]> = sites
            .iter()
            .map(|(p, s)| [p.x, p.z, radius * s, p.y + tip * s])
            .collect();
        if trees.is_empty() {
            return Self::default();
        }
        let (lo, hi) = trees.iter().fold(
            ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]),
            |(lo, hi), t| {
                (
                    [lo[0].min(t[0] - t[2]), lo[1].min(t[1] - t[2])],
                    [hi[0].max(t[0] + t[2]), hi[1].max(t[1] + t[2])],
                )
            },
        );
        let cols = (((hi[0] - lo[0]) / CROWN_CELL_M) as usize + 1).min(4096);
        let rows = (((hi[1] - lo[1]) / CROWN_CELL_M) as usize + 1).min(4096);
        let mut grid = vec![Vec::new(); cols * rows];
        for (i, t) in trees.iter().enumerate() {
            let c0 = (((t[0] - t[2] - lo[0]) / CROWN_CELL_M) as usize).min(cols - 1);
            let c1 = (((t[0] + t[2] - lo[0]) / CROWN_CELL_M) as usize).min(cols - 1);
            let r0 = (((t[1] - t[2] - lo[1]) / CROWN_CELL_M) as usize).min(rows - 1);
            let r1 = (((t[1] + t[2] - lo[1]) / CROWN_CELL_M) as usize).min(rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    grid[r * cols + c].push(i as u32);
                }
            }
        }
        Self {
            trees,
            origin: lo,
            cols,
            rows,
            grid,
        }
    }

    pub fn len(&self) -> usize {
        self.trees.len()
    }

    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// The highest crown tip over (x, z) (in the tile's package metres).
    pub fn top_at(&self, x: f32, z: f32) -> Option<f32> {
        if self.trees.is_empty() {
            return None;
        }
        let c = (x - self.origin[0]) / CROWN_CELL_M;
        let r = (z - self.origin[1]) / CROWN_CELL_M;
        if c < 0.0 || r < 0.0 || c as usize >= self.cols || r as usize >= self.rows {
            return None;
        }
        self.grid[r as usize * self.cols + c as usize]
            .iter()
            .map(|&i| self.trees[i as usize])
            .filter(|t| (x - t[0]).hypot(z - t[1]) <= t[2])
            .map(|t| t[3])
            .reduce(f32::max)
    }
}

/// Where the tiles stream: around `centre` within `radius` tiles, and
/// around `ahead` (a point along the flight) within [`AHEAD_RADIUS`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamFocus {
    pub centre: [f64; 2],
    pub radius: i32,
    pub ahead: Option<[f64; 2]>,
}

/// The ring around the point ahead (1 = 3 × 3 tiles).
pub const AHEAD_RADIUS: i32 = 1;
/// How far ahead the tiles stream: this many seconds of flight …
pub const AHEAD_S: f64 = 3.0;
/// … but at most this far (m).
pub const AHEAD_MAX_M: f64 = 3_000.0;
/// The ring around the unicorn grows by one tile per this much height
/// above the ground (m) …
pub const RADIUS_PER_HEIGHT_M: f64 = 600.0;
/// … up to this radius (3 = 7 × 7 tiles; the far terrain covers the rest).
pub const MAX_FLIGHT_RADIUS: i32 = 3;
/// Neighbour packages load this many seconds of flight earlier (spec
/// §4.7: within 3 km + 10 s of flight).
pub const NEIGHBOUR_AHEAD_S: f64 = 10.0;

/// The ring of tiles around the unicorn at `height_m` above the ground.
pub fn flight_radius(height_m: f64) -> i32 {
    (STREAM_RADIUS + (height_m.max(0.0) / RADIUS_PER_HEIGHT_M) as i32).min(MAX_FLIGHT_RADIUS)
}

/// Where the tiles stream for the view: around the rider while riding,
/// around the unicorn and ahead of it while flying.
pub fn stream_focus(view: &RideView) -> Option<StreamFocus> {
    if let Some(f) = &view.flight {
        let v = f.velocity;
        let speed = v[0].hypot(v[1]);
        let ahead = (speed > 1.0).then(|| {
            let d = (speed * AHEAD_S).min(AHEAD_MAX_M);
            [f.pos[0] + v[0] / speed * d, f.pos[2] + v[1] / speed * d]
        });
        return Some(StreamFocus {
            centre: [f.pos[0], f.pos[2]],
            radius: flight_radius(f.height_m),
            ahead,
        });
    }
    let local = view.local()?;
    let p = view.track(local.id).sample(local.state.distance).position;
    Some(StreamFocus {
        centre: [p[0], p[2]],
        radius: STREAM_RADIUS,
        ahead: None,
    })
}

/// How much farther the neighbour packages load in flight (m): 10 s of
/// the horizontal speed; 0 while riding.
pub fn neighbour_reach_m(view: &RideView) -> f64 {
    view.flight.as_ref().map_or(0.0, |f| {
        f.velocity[0].hypot(f.velocity[1]) * NEIGHBOUR_AHEAD_S
    })
}

#[cfg(test)]
mod tests {
    use ocw_world::Route;

    use super::*;
    use crate::FlightView;

    fn flying(height_m: f64, velocity: [f64; 2]) -> RideView {
        let mut v = RideView::new(Route::test_loop());
        v.flight = Some(FlightView {
            pos: [1000.0, 500.0, -2000.0],
            height_m,
            velocity,
            ..Default::default()
        });
        v
    }

    #[test]
    fn tree_crowns_are_obstacles_up_to_their_tips() {
        let sites = vec![
            (Vec3::new(10.0, 100.0, 10.0), 1.0),
            (Vec3::new(12.0, 101.0, 10.0), 1.5),
            (Vec3::new(500.0, 50.0, 500.0), 0.5),
        ];
        let c = TreeCrowns::from_sites(&sites);
        assert_eq!(c.len(), 3);
        let (_, tip, r) = crate::package::CROWN;
        // Where both crowns overlap the bigger, higher one wins.
        assert_eq!(c.top_at(11.0, 10.0), Some(101.0 + tip * 1.5));
        assert_eq!(c.top_at(10.0 - r + 0.1, 10.0), Some(100.0 + tip));
        assert_eq!(c.top_at(500.0, 500.0), Some(50.0 + tip * 0.5));
        assert_eq!(c.top_at(500.0 + r * 0.5 + 0.1, 500.0), None);
        assert_eq!(c.top_at(250.0, 250.0), None);
        assert_eq!(c.top_at(-100.0, 10.0), None);
        assert!(TreeCrowns::from_sites(&[]).top_at(0.0, 0.0).is_none());
    }

    /// The unicorn flies low over a forest: its clearance holds over the
    /// crowns, not only over the ground (spec §14 Phase 13B).
    #[test]
    fn the_flight_keeps_its_clearance_over_the_tree_crowns() {
        use ocw_sim::god::{CLEARANCE_M, Flight, FlightInput};
        let sites: Vec<TreeSite> = (0..60)
            .flat_map(|i| {
                (0..5).map(move |k| (Vec3::new(i as f32 * 6.0, 0.0, -(k as f32) * 6.0), 1.3))
            })
            .collect();
        let crowns = TreeCrowns::from_sites(&sites);
        let surface = |x: f64, z: f64| {
            let tree = crowns.top_at(x as f32, z as f32).map(f64::from);
            Some(tree.map_or(0.0, |t| t.max(0.0)))
        };
        // Low over the meadow, then along +x through the forest, asking
        // to descend all the time.
        let mut f = Flight::new([-50.0, 3.0, -12.0], -std::f64::consts::FRAC_PI_2);
        f.pitch = 0.0;
        let input = FlightInput {
            forward: 1.0,
            climb: -1.0,
            ..Default::default()
        };
        let mut over_forest = false;
        for _ in 0..600 {
            f.step(&input, 1.0 / 60.0, &surface, None);
            let p = f.pos;
            let top = surface(p[0], p[2]).unwrap();
            assert!(p[1] >= top + CLEARANCE_M - 1e-6, "{p:?} over {top}");
            if top > 5.0 {
                over_forest = true;
            }
        }
        assert!(over_forest, "the flight crossed the forest");
    }

    #[test]
    fn the_tiles_stream_ahead_and_wider_with_altitude() {
        assert_eq!(flight_radius(0.0), STREAM_RADIUS);
        assert_eq!(flight_radius(599.0), STREAM_RADIUS);
        assert_eq!(flight_radius(700.0), STREAM_RADIUS + 1);
        assert_eq!(flight_radius(5_000.0), MAX_FLIGHT_RADIUS);
        // Hovering: around the unicorn only.
        let f = stream_focus(&flying(20.0, [0.0, 0.0])).unwrap();
        assert_eq!(f.centre, [1000.0, -2000.0]);
        assert_eq!(f.ahead, None);
        // 100 m/s east: 300 m ahead; 1,600 m/s north: capped at 3 km.
        let f = stream_focus(&flying(20.0, [100.0, 0.0])).unwrap();
        let a = f.ahead.unwrap();
        assert!((a[0] - 1300.0).abs() < 1e-9 && (a[1] + 2000.0).abs() < 1e-9);
        let f = stream_focus(&flying(900.0, [0.0, -1600.0])).unwrap();
        let a = f.ahead.unwrap();
        assert!((a[1] + 5000.0).abs() < 1e-9, "{a:?}");
        assert_eq!(f.radius, STREAM_RADIUS + 1);
        // Neighbours: 10 s of flight farther.
        assert_eq!(neighbour_reach_m(&flying(20.0, [300.0, 400.0])), 5_000.0);
        let riding = RideView::new(Route::test_loop());
        assert_eq!(neighbour_reach_m(&riding), 0.0);
        assert!(stream_focus(&riding).is_none(), "no rider yet");
    }

    #[test]
    fn trees_and_buildings_only_below_1500_m_above_ground() {
        assert!(details_shown(&RideView::new(Route::test_loop())));
        assert!(details_shown(&flying(1_499.0, [0.0, 0.0])));
        assert!(!details_shown(&flying(1_500.0, [0.0, 0.0])));
        assert!(!details_shown(&flying(6_000.0, [0.0, 0.0])));
    }
}

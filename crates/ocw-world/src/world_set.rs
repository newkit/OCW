//! Several world packages ridden as one world (spec §2.2.2 region border,
//! §4.5, §4.7): the package the ride starts in (the *home* package) and
//! its installed neighbours.
//!
//! - **Frames:** each package keeps its own local metres. Everything at
//!   runtime (physics, the joined road graph, rendering) is in the home
//!   package's metres; a neighbour's metres map into them with a
//!   [`Frame`]. For the local projection of [`crate::geo`] this map is
//!   exactly affine (a scale along x and a shift), so the neighbour's
//!   tiles, its roads in the joined graph and its ground heights line up
//!   with the home package to floating-point precision. Packages built
//!   with a common projection origin (`world-builder package --origin`)
//!   map with the identity.
//! - **Road graph:** [`WorldSet::add_neighbour`] links the neighbour's
//!   boundary vertices to the joined graph ([`crate::border`]) and adds it
//!   when at least one road crosses; the joined graph is fixed for the
//!   ride (junction ids are its vertex indices, spec §6.5).
//! - **Ground ownership:** tiles of neighbouring packages overlap (each
//!   package has a margin of terrain-only tiles beyond its border). Every
//!   point belongs to the first *resident* package that has a tile there
//!   and whose region contains it, else to the first resident package
//!   (home first) that has a tile there ([`WorldSet::owner`]). The renderer draws a terrain cell, water,
//!   trees and buildings of a package only where that package owns the
//!   ground, so the seam has no double or missing terrain.
//! - **Residency:** the graph of every member stays loaded; the tiles
//!   (terrain, land use, buildings – the bulk of a package) of a neighbour
//!   are kept only while the rider is near it ([`WorldSet::plan_residency`],
//!   [`LOAD_DISTANCE_M`], [`UNLOAD_DISTANCE_M`],
//!   [`MAX_RESIDENT_NEIGHBOURS`]). Loading is the caller's job (file
//!   access is not allowed here, spec §3.5).
//!
//! Pure: no files, threads or clocks.

use std::collections::BTreeSet;
use std::sync::Arc;

use crate::border::{Placed, join_graphs, match_border};
use crate::geo::LocalProjection;
use crate::graph::RoadGraph;
use crate::package::{TileKey, WorldPackage};
use crate::region::{PackageRegion, in_ring};

/// A neighbour's tiles are loaded when the rider comes this close to its
/// region (m).
pub const LOAD_DISTANCE_M: f64 = 3000.0;
/// … and dropped again beyond this distance (hysteresis).
pub const UNLOAD_DISTANCE_M: f64 = 5000.0;
/// Memory budget: at most this many neighbours have their tiles loaded
/// (the nearest ones). A package is about 30 MB in memory.
pub const MAX_RESIDENT_NEIGHBOURS: usize = 2;

/// Maps one package's local metres into another's: `x' = scale_x · x +
/// offset[0]`, `z' = z + offset[1]` (exact for [`LocalProjection`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub scale_x: f64,
    pub offset: [f64; 2],
}

impl Frame {
    pub const IDENTITY: Frame = Frame {
        scale_x: 1.0,
        offset: [0.0, 0.0],
    };

    /// From the metres around projection origin `from` into those around
    /// `to` (both (lat, lon)).
    pub fn between(from: (f64, f64), to: (f64, f64)) -> Self {
        if from == to {
            return Self::IDENTITY;
        }
        let target = LocalProjection::new(to.0, to.1);
        Self {
            scale_x: to.0.to_radians().cos() / from.0.to_radians().cos(),
            offset: target.project(from.0, from.1),
        }
    }

    pub fn apply(&self, p: [f64; 2]) -> [f64; 2] {
        [self.scale_x * p[0] + self.offset[0], p[1] + self.offset[1]]
    }

    pub fn invert(&self, p: [f64; 2]) -> [f64; 2] {
        [
            (p[0] - self.offset[0]) / self.scale_x,
            p[1] - self.offset[1],
        ]
    }

    pub fn is_identity(&self) -> bool {
        *self == Self::IDENTITY
    }
}

/// A region in the home package's metres.
#[derive(Debug, Clone, PartialEq)]
struct MetricRegion {
    min: [f64; 2],
    max: [f64; 2],
    /// Rings as (z, x) so [`in_ring`] works on them unchanged.
    outer: Vec<Vec<(f64, f64)>>,
    holes: Vec<Vec<(f64, f64)>>,
}

impl MetricRegion {
    fn new(r: &PackageRegion, home: &LocalProjection) -> Self {
        let a = home.project(r.north, r.west);
        let b = home.project(r.south, r.east);
        let ring = |ring: &Vec<(f64, f64)>| -> Vec<(f64, f64)> {
            ring.iter()
                .map(|&(lat, lon)| {
                    let p = home.project(lat, lon);
                    (p[1], p[0])
                })
                .collect()
        };
        Self {
            min: [a[0].min(b[0]), a[1].min(b[1])],
            max: [a[0].max(b[0]), a[1].max(b[1])],
            outer: r.outer.iter().map(ring).collect(),
            holes: r.holes.iter().map(ring).collect(),
        }
    }

    fn contains(&self, p: [f64; 2]) -> bool {
        (self.min[0]..=self.max[0]).contains(&p[0])
            && (self.min[1]..=self.max[1]).contains(&p[1])
            && (self.outer.is_empty()
                || (self.outer.iter().any(|r| in_ring(r, (p[1], p[0])))
                    && !self.holes.iter().any(|r| in_ring(r, (p[1], p[0])))))
    }

    /// Distance from `p` to the box (0 inside).
    fn box_distance(&self, p: [f64; 2]) -> f64 {
        let dx = (self.min[0] - p[0]).max(p[0] - self.max[0]).max(0.0);
        let dz = (self.min[1] - p[1]).max(p[1] - self.max[1]).max(0.0);
        dx.hypot(dz)
    }
}

/// One package of the set.
#[derive(Debug, Clone)]
pub struct Member {
    /// Package id (manifest id, or `local/<name>`), for logs.
    pub id: String,
    pub name: String,
    pub origin: (f64, f64),
    /// This package's metres → the home package's metres.
    pub frame: Frame,
    /// Roads linked to the packages before it (0 for the home package).
    pub links: usize,
    /// Where they cross the border (home metres).
    pub crossings: Vec<[f64; 2]>,
    region: Option<MetricRegion>,
    tile_size_m: f64,
    tile_keys: BTreeSet<TileKey>,
    /// Box of the tiles in home metres (the region when there is none).
    tiles_min: [f64; 2],
    tiles_max: [f64; 2],
    resident: Option<Arc<WorldPackage>>,
}

impl Member {
    /// The package with its tiles, while they are loaded.
    pub fn package(&self) -> Option<&Arc<WorldPackage>> {
        self.resident.as_ref()
    }

    pub fn is_resident(&self) -> bool {
        self.resident.is_some()
    }

    /// The package has a tile at `p` (home metres).
    pub fn has_tile_at(&self, p: [f64; 2]) -> bool {
        let q = self.frame.invert(p);
        self.tile_keys
            .contains(&TileKey::containing(q, self.tile_size_m))
    }

    /// `p` (home metres) lies in this package's region.
    pub fn region_contains(&self, p: [f64; 2]) -> bool {
        self.region.as_ref().is_some_and(|r| r.contains(p))
    }

    /// Distance from `p` (home metres) to the package's area.
    pub fn distance(&self, p: [f64; 2]) -> f64 {
        match &self.region {
            Some(r) => r.box_distance(p),
            None => {
                let dx = (self.tiles_min[0] - p[0])
                    .max(p[0] - self.tiles_max[0])
                    .max(0.0);
                let dz = (self.tiles_min[1] - p[1])
                    .max(p[1] - self.tiles_max[1])
                    .max(0.0);
                dx.hypot(dz)
            }
        }
    }
}

/// A change of which neighbours have their tiles loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Residency {
    /// Load the tiles of member `i`.
    Load(usize),
    /// Drop the tiles of member `i`.
    Unload(usize),
}

/// The home package and its neighbours (see the module docs).
#[derive(Debug, Clone)]
pub struct WorldSet {
    members: Vec<Member>,
    graph: Option<RoadGraph>,
    /// Counts changes of residency (renderers re-mesh seam tiles on it).
    generation: u64,
}

fn member(id: &str, pkg: &Arc<WorldPackage>, home_origin: (f64, f64), links: usize) -> Member {
    let home = LocalProjection::new(home_origin.0, home_origin.1);
    let frame = Frame::between(pkg.origin, home_origin);
    let size = f64::from(pkg.tile_size_m);
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for k in pkg.tiles.keys() {
        for corner in [
            k.origin(size),
            [k.origin(size)[0] + size, k.origin(size)[1] + size],
        ] {
            let c = frame.apply(corner);
            lo = [lo[0].min(c[0]), lo[1].min(c[1])];
            hi = [hi[0].max(c[0]), hi[1].max(c[1])];
        }
    }
    Member {
        id: id.into(),
        name: pkg.name.clone(),
        origin: pkg.origin,
        frame,
        links,
        crossings: Vec::new(),
        region: pkg.region.as_ref().map(|r| MetricRegion::new(r, &home)),
        tile_size_m: size,
        tile_keys: pkg.tiles.keys().copied().collect(),
        tiles_min: lo,
        tiles_max: hi,
        resident: Some(pkg.clone()),
    }
}

impl WorldSet {
    /// A set of one package (the ride starts in it).
    pub fn new(id: &str, home: Arc<WorldPackage>) -> Self {
        let graph = home.graph.clone();
        Self {
            members: vec![member(id, &home, home.origin, 0)],
            graph,
            generation: 0,
        }
    }

    /// Adds a neighbouring package: its roads are linked to the joined
    /// graph where they cross the border. Refused (nothing changes) when
    /// it has no region or no graph, covers the same area as a member
    /// (another build of the same region) or no road crosses to it.
    /// Returns the number of linked roads.
    pub fn add_neighbour(&mut self, id: &str, pkg: Arc<WorldPackage>) -> Result<usize, String> {
        if self.members.iter().any(|m| m.id == id) {
            return Err(format!("{id} is already part of the world"));
        }
        let Some(region) = &pkg.region else {
            return Err(format!(
                "{id}: no region section (REGN); rebuild it with a current world-builder"
            ));
        };
        let Some(nb_graph) = &pkg.graph else {
            return Err(format!("{id}: no road graph"));
        };
        let Some(joined) = &self.graph else {
            return Err("the home package has no road graph".into());
        };
        let home_origin = self.home().origin;
        let home_proj = LocalProjection::new(home_origin.0, home_origin.1);
        let centre = region.centre();
        let centre = home_proj.project(centre.0, centre.1);
        for m in &self.members {
            let own = m.region.as_ref().map(|r| {
                home_proj.unproject([(r.min[0] + r.max[0]) / 2.0, (r.min[1] + r.max[1]) / 2.0])
            });
            if m.region_contains(centre) || own.is_some_and(|c| region.contains(c)) {
                return Err(format!("{id} covers the same area as {}", m.id));
            }
        }
        let a = Placed {
            graph: joined,
            origin: home_origin,
        };
        let b = Placed {
            graph: nb_graph,
            origin: pkg.origin,
        };
        let links = match_border(a, b);
        if links.is_empty() {
            return Err(format!("{id}: no road crosses the border to it"));
        }
        let crossings = links
            .iter()
            .map(|l| joined.vertices[l.here.0 as usize])
            .collect();
        let graph = join_graphs(a, b, &links);
        self.graph = Some(graph);
        let mut m = member(id, &pkg, home_origin, links.len());
        m.crossings = crossings;
        self.members.push(m);
        self.generation += 1;
        Ok(links.len())
    }

    pub fn home(&self) -> &Member {
        &self.members[0]
    }

    pub fn members(&self) -> &[Member] {
        &self.members
    }

    pub fn len(&self) -> usize {
        self.members.len()
    }

    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }

    /// The joined road graph (home metres).
    pub fn graph(&self) -> Option<&RoadGraph> {
        self.graph.as_ref()
    }

    /// Name for the network and the session (`Home + Neighbour`).
    pub fn name(&self) -> String {
        self.members
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>()
            .join(" + ")
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// More than one package has its tiles loaded (the renderer clips
    /// tiles only then).
    pub fn has_seams(&self) -> bool {
        self.members.iter().filter(|m| m.is_resident()).count() > 1
    }

    /// Which member draws the ground at `p` (home metres): the first
    /// resident one with a tile there whose region contains it, else the
    /// first resident one with a tile there.
    pub fn owner(&self, p: [f64; 2]) -> Option<usize> {
        let resident = || {
            self.members
                .iter()
                .enumerate()
                .filter(|(_, m)| m.is_resident())
        };
        resident()
            .find(|(_, m)| m.region_contains(p) && m.has_tile_at(p))
            .or_else(|| resident().find(|(_, m)| m.has_tile_at(p)))
            .map(|(i, _)| i)
    }

    /// Height of the rendered terrain surface at `p` (home metres), from
    /// the package that owns the ground there.
    pub fn surface_height(&self, p: [f64; 2]) -> Option<f64> {
        let at = |i: usize| {
            let m = &self.members[i];
            let q = m.frame.invert(p);
            m.resident.as_ref()?.surface_height(q[0], q[1])
        };
        self.owner(p)
            .and_then(at)
            .or_else(|| (0..self.members.len()).find_map(at))
    }

    /// Whether member `i` draws the ground at `q` (in *its own* metres).
    pub fn draws(&self, i: usize, q: [f64; 2]) -> bool {
        !self.has_seams() || self.owner(self.members[i].frame.apply(q)) == Some(i)
    }

    /// Which neighbours to load or drop for a rider at `p` (home metres):
    /// those within [`LOAD_DISTANCE_M`] (resident ones until
    /// [`UNLOAD_DISTANCE_M`]), at most [`MAX_RESIDENT_NEIGHBOURS`], nearest
    /// first. `loading` are the loads already under way.
    pub fn plan_residency(&self, p: [f64; 2], loading: &BTreeSet<usize>) -> Vec<Residency> {
        let mut near: Vec<(f64, usize)> = self
            .members
            .iter()
            .enumerate()
            .skip(1)
            .filter_map(|(i, m)| {
                let d = m.distance(p);
                let keep = m.is_resident() || loading.contains(&i);
                (d < LOAD_DISTANCE_M || (keep && d < UNLOAD_DISTANCE_M)).then_some((d, i))
            })
            .collect();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        let wanted: BTreeSet<usize> = near
            .iter()
            .take(MAX_RESIDENT_NEIGHBOURS)
            .map(|&(_, i)| i)
            .collect();
        let mut out = Vec::new();
        for (i, m) in self.members.iter().enumerate().skip(1) {
            let have = m.is_resident() || loading.contains(&i);
            match (have, wanted.contains(&i)) {
                (false, true) => out.push(Residency::Load(i)),
                (true, false) if m.is_resident() => out.push(Residency::Unload(i)),
                _ => {}
            }
        }
        out
    }

    /// Sets the loaded tiles of neighbour `i` (`None` drops them). The
    /// package must be the same one (same tiles); the home package always
    /// stays loaded.
    pub fn set_resident(&mut self, i: usize, pkg: Option<Arc<WorldPackage>>) {
        if i == 0 || i >= self.members.len() {
            return;
        }
        let m = &mut self.members[i];
        if let Some(p) = &pkg
            && p.tiles.keys().copied().collect::<BTreeSet<_>>() != m.tile_keys
        {
            return;
        }
        if m.resident.is_some() != pkg.is_some() {
            self.generation += 1;
        }
        m.resident = pkg;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::graph::{EdgeId, VertexId};
    use crate::network::RoadNetwork;
    use crate::network::tests::graph;
    use crate::package::Tile;
    use crate::path::{RidePath, TurnReason};

    const HOME: (f64, f64) = (47.1, 9.5);
    /// The regions reach this far north and south (m), inside the tiles.
    const HALF_Z: f64 = 800.0;

    /// Terrain at height `h` on tiles x0..=x1, z0..=z1, 3 samples.
    fn tiles(x0: i32, x1: i32, z0: i32, z1: i32, h: f32) -> BTreeMap<TileKey, Tile> {
        let mut t = BTreeMap::new();
        for x in x0..=x1 {
            for z in z0..=z1 {
                let key = TileKey { x, z };
                t.insert(
                    key,
                    Tile {
                        key,
                        heights: vec![h; 9],
                        roads: vec![],
                    },
                );
            }
        }
        t
    }

    fn pkg(
        name: &str,
        origin: (f64, f64),
        region: PackageRegion,
        graph: RoadGraph,
        tiles: BTreeMap<TileKey, Tile>,
    ) -> Arc<WorldPackage> {
        Arc::new(WorldPackage {
            name: name.into(),
            origin,
            tile_size_m: 1000.0,
            samples: 3,
            tiles,
            graph: Some(graph),
            landuse: None,
            buildings: None,
            bridges: None,
            routes: vec![],
            region: Some(region),
            time_zone: None,
        })
    }

    /// West package (home): a road along z = 0 from x = −1500 to the
    /// border at lon 9.5 (x = 0) with a side road; terrain at 100 m on
    /// tiles x −2..=0 (one margin tile beyond the border). East package:
    /// the road from the border on to x = +1500, its own origin
    /// `east_origin`, terrain at 200 m on tiles reaching 1 km back west.
    fn west_east(east_origin: (f64, f64)) -> (Arc<WorldPackage>, Arc<WorldPackage>) {
        let home = LocalProjection::new(HOME.0, HOME.1);
        let (n, s) = (
            home.unproject([0.0, -HALF_Z]).0,
            home.unproject([0.0, HALF_Z]).0,
        );
        let (w, e) = (
            home.unproject([-3000.0, 0.0]).1,
            home.unproject([3000.0, 0.0]).1,
        );
        let mut a = graph(
            &[[-1500.0, 0.0], [-700.0, 0.0], [0.0, 0.0], [-700.0, 500.0]],
            &[(0, 1), (1, 2), (1, 3)],
        );
        for (i, e) in a.edges.iter_mut().enumerate() {
            e.osm_way = 77 + i as i64;
            e.id = EdgeId(100 + i as u64);
        }
        a.edges[1].osm_way = 77;
        a.edges[1].boundary = true;
        a.boundary_vertices.insert(VertexId(2));
        let east = LocalProjection::new(east_origin.0, east_origin.1);
        let to_east = |x: f64, z: f64| {
            let (lat, lon) = home.unproject([x, z]);
            east.project(lat, lon)
        };
        let mut b = graph(
            &[to_east(0.0, 0.0), to_east(800.0, 0.0), to_east(1500.0, 0.0)],
            &[(0, 1), (1, 2)],
        );
        for (i, e) in b.edges.iter_mut().enumerate() {
            e.osm_way = 77;
            e.id = EdgeId(200 + i as u64);
        }
        b.edges[0].boundary = true;
        b.boundary_vertices.insert(VertexId(0));
        // Its east end is its own border (to a third package).
        b.edges[1].boundary = true;
        b.boundary_vertices.insert(VertexId(2));
        // East tiles in its own metres: from 1 km west of the border.
        let size = 1000.0;
        let x0 = (to_east(-1000.0, 0.0)[0] / size).floor() as i32;
        let x1 = (to_east(2000.0, 0.0)[0] / size).floor() as i32;
        let z0 = (to_east(0.0, -1000.0)[1] / size).floor() as i32;
        let z1 = (to_east(0.0, 999.0)[1] / size).floor() as i32;
        let west = pkg(
            "West",
            HOME,
            PackageRegion::bbox(s, w, n, HOME.1),
            a,
            tiles(-2, 0, -1, 0, 100.0),
        );
        let east = pkg(
            "East",
            east_origin,
            PackageRegion::bbox(s, HOME.1, n, e),
            b,
            tiles(x0, x1, z0, z1, 200.0),
        );
        (west, east)
    }

    #[test]
    fn frames_map_exactly_between_projections() {
        let from = (47.3, 9.62);
        let to = (47.1, 9.5);
        let f = Frame::between(from, to);
        let (pf, pt) = (
            LocalProjection::new(from.0, from.1),
            LocalProjection::new(to.0, to.1),
        );
        for p in [[0.0, 0.0], [12_345.0, -6_789.0], [-20_000.0, 15_000.0]] {
            let (lat, lon) = pf.unproject(p);
            let exact = pt.project(lat, lon);
            let q = f.apply(p);
            assert!((q[0] - exact[0]).abs() < 1e-6 && (q[1] - exact[1]).abs() < 1e-6);
            let back = f.invert(q);
            assert!((back[0] - p[0]).abs() < 1e-6 && (back[1] - p[1]).abs() < 1e-6);
        }
        assert!(Frame::between(to, to).is_identity());
        // Same latitude: a pure shift.
        assert_eq!(Frame::between((47.1, 9.6), to).scale_x, 1.0);
    }

    #[test]
    fn a_neighbour_joins_the_graph_and_the_ride_crosses() {
        for east_origin in [HOME, (47.12, 9.53)] {
            let (west, east) = west_east(east_origin);
            let mut set = WorldSet::new("test/west", west.clone());
            assert_eq!(set.add_neighbour("test/east", east.clone()), Ok(1));
            assert_eq!(set.members()[1].crossings, vec![[0.0, 0.0]]);
            assert_eq!(set.name(), "West + East");
            assert_eq!(set.len(), 2);
            let g = set.graph().unwrap().clone();
            assert_eq!(g.vertices.len(), 4 + 2, "the border vertex is shared");
            // The east end of the road is in home metres at x = 1500.
            let far = g.boundary_vertices.iter().next().unwrap();
            assert!((g.vertices[far.0 as usize][0] - 1500.0).abs() < 1e-3);
            let net = Arc::new(RoadNetwork::new(g));
            let mut p = RidePath::start(net, 0, 100.0, true, 0.0, 0);
            p.extend(0.0, &mut |j| j.default);
            p.extend(800.0, &mut |j| j.default);
            p.extend(1500.0, &mut |j| j.default);
            let (at, reason) = p.turnaround_ahead(0.0).unwrap();
            assert_eq!(reason, TurnReason::Boundary);
            assert!(at > 2500.0, "turns at the east end, not the border: {at}");
            // Adding it twice or adding the home package again is refused.
            assert!(set.add_neighbour("test/east", east.clone()).is_err());
            assert!(set.add_neighbour("test/west-copy", west.clone()).is_err());
        }
    }

    #[test]
    fn neighbours_without_a_crossing_road_or_region_are_refused() {
        let (west, east) = west_east(HOME);
        let mut set = WorldSet::new("w", west);
        let mut no_road = (*east).clone();
        for e in &mut no_road.graph.as_mut().unwrap().edges {
            e.osm_way = 999;
            e.id = EdgeId(999_000 + e.id.0);
        }
        assert!(set.add_neighbour("e1", Arc::new(no_road)).is_err());
        let mut no_region = (*east).clone();
        no_region.region = None;
        assert!(set.add_neighbour("e2", Arc::new(no_region)).is_err());
        assert_eq!(set.len(), 1);
        assert_eq!(set.generation(), 0);
    }

    #[test]
    fn each_point_of_the_seam_is_drawn_by_one_package() {
        for east_origin in [HOME, (47.12, 9.53)] {
            let (west, east) = west_east(east_origin);
            let mut set = WorldSet::new("w", west);
            set.add_neighbour("e", east).unwrap();
            assert!(set.has_seams());
            // West of the border: home; east of it: the neighbour, also on
            // home's margin tile x 0..1000.
            assert_eq!(set.owner([-10.0, -100.0]), Some(0));
            assert_eq!(set.owner([10.0, -100.0]), Some(1));
            assert_eq!(set.owner([900.0, -100.0]), Some(1));
            // Heights come from the owner: 100 m west, 200 m east.
            assert_eq!(set.surface_height([-500.0, -500.0]), Some(100.0));
            assert_eq!(set.surface_height([500.0, -500.0]), Some(200.0));
            // Outside both regions (beyond the north edge) the first
            // package with a tile there draws.
            assert_eq!(set.owner([-500.0, -900.0]), Some(0));
            assert_eq!(set.owner([500.0, -900.0]), Some(0));
            assert_eq!(set.owner([-2500.0, -1100.0]), None);
            // In its own metres: the neighbour does not draw west of the
            // border, the home package not east of it.
            let e = &set.members()[1];
            assert!(!set.draws(1, e.frame.invert([-10.0, -100.0])));
            assert!(set.draws(1, e.frame.invert([10.0, -100.0])));
            assert!(set.draws(0, [-10.0, -100.0]));
            assert!(!set.draws(0, [10.0, -100.0]));
            // Without the neighbour's tiles home draws its margin again.
            let g = set.generation();
            set.set_resident(1, None);
            assert!(set.generation() > g);
            assert!(!set.has_seams());
            assert_eq!(set.owner([500.0, -100.0]), Some(0));
            assert!(set.draws(0, [10.0, -100.0]));
            assert_eq!(set.surface_height([500.0, -500.0]), Some(100.0));
        }
    }

    #[test]
    fn neighbour_tiles_load_near_the_border_and_unload_far_away() {
        let (west, east) = west_east(HOME);
        let mut set = WorldSet::new("w", west);
        set.add_neighbour("e", east.clone()).unwrap();
        let none = BTreeSet::new();
        // Starts resident (the graph was read from it): far away it goes.
        assert_eq!(
            set.plan_residency([-8000.0, 0.0], &none),
            vec![Residency::Unload(1)]
        );
        set.set_resident(1, None);
        // 4 km from the border: not yet (load at 3 km).
        assert!(set.plan_residency([-4000.0, 0.0], &none).is_empty());
        assert_eq!(
            set.plan_residency([-2500.0, 0.0], &none),
            vec![Residency::Load(1)]
        );
        // A load under way is not asked for again.
        let loading: BTreeSet<usize> = [1].into();
        assert!(set.plan_residency([-2500.0, 0.0], &loading).is_empty());
        set.set_resident(1, Some(east.clone()));
        // Hysteresis: kept at 4 km, dropped beyond 5 km.
        assert!(set.plan_residency([-4000.0, 0.0], &none).is_empty());
        assert_eq!(
            set.plan_residency([-5500.0, 0.0], &none),
            vec![Residency::Unload(1)]
        );
        // Another package's tiles are not accepted as the neighbour's.
        set.set_resident(1, None);
        let mut other = (*east).clone();
        other.tiles.pop_first();
        set.set_resident(1, Some(Arc::new(other)));
        assert!(!set.members()[1].is_resident());
        // The home package always stays.
        set.set_resident(0, None);
        assert!(set.home().is_resident());
    }

    #[test]
    fn the_memory_budget_keeps_the_nearest_neighbours() {
        // Three neighbours north of home, at increasing distance: only
        // MAX_RESIDENT_NEIGHBOURS stay resident.
        let (west, _) = west_east(HOME);
        let mut set = WorldSet::new("w", west);
        let home = LocalProjection::new(HOME.0, HOME.1);
        for (k, gap) in [(1, 0.0), (2, 500.0), (3, 1000.0)] {
            let (w, e) = (
                home.unproject([-3000.0 + 2000.0 * k as f64, 0.0]).1,
                home.unproject([-2000.0 + 2000.0 * k as f64, 0.0]).1,
            );
            let (n, s) = (
                home.unproject([0.0, -HALF_Z - gap - 1000.0]).0,
                home.unproject([0.0, -HALF_Z - gap]).0,
            );
            let m = member(
                &format!("n{k}"),
                &pkg(
                    "N",
                    HOME,
                    PackageRegion::bbox(s, w, n, e),
                    RoadGraph::default(),
                    tiles(0, 0, 0, 0, 1.0),
                ),
                HOME,
                1,
            );
            set.members.push(m);
        }
        let plan = set.plan_residency([0.0, 0.0], &BTreeSet::new());
        assert_eq!(plan, vec![Residency::Unload(3)], "{plan:?}");
    }
}

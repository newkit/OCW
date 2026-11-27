//! Static obstacles of the world (spec §14 Phase 12, 12B / K3): what a
//! rider in wild mode can hit – trees, buildings, walls, fences, hedges,
//! bollards, posts, gates and barriers – and what bounds every rider on
//! bridges and in tunnels (railings, tunnel walls).
//!
//! **Section `OBST`** (optional, additive; older readers skip it) holds
//! what the package has nowhere else: the OSM `barrier=*` nodes and ways
//! and the posts (`power=pole|tower`, `man_made=utility_pole`), and which
//! cells of the tree grid hold a tree ([`crate::trees`]). It is written by
//! the world builder with the **2 m corridor rule**: only obstacles a
//! rider can reach are stored – within [`WILD_REACH_M`] (wild mode's reach
//! beside the road centre line) plus 2 m of a road – and a point barrier
//! within 2 m of a road's carriageway is a *road barrier*
//! ([`ROAD_BARRIER`]): drawn only with `display.road_barriers` and never
//! solid (spec §14 Phase 12 Q7: barriers never block the routed road).
//! The parts of barrier lines (fences, walls) on a carriageway are cut out
//! by the builder (a fence ends at the road it meets).
//!
//! ```text
//! layout version: u8 (1)
//! tree cells per tile side: u16 (125: 8 m)
//! tile count: u32, then per tile:
//!   tile x, z: i32 i32
//!   point count: u32, per point:
//!     kind u8, flags u8, x, z: f32 f32 (package metres),
//!     angle u8 (the way's direction, × π / 256), width u8 (dm), height u8 (dm)
//!   line count: u32, per line:
//!     kind u8, flags u8, height u8 (dm), point count: u32, points: [f32; 2]
//!   tree bytes: u32 length + deflated cells (one byte per cell, row-major
//!     along +z: 0 = none, else land class + 1; length 0 = no trees)
//! ```
//!
//! Buildings (`BLDG` footprints), bridge railings (`BRDG`) and tunnel walls
//! (the road graph's tunnel edges) are not repeated in it: [`WorldObstacles`]
//! takes them from their own sections, so also packages without `OBST`
//! have them. [`WorldObstacles::tile_index`] builds the spatial index of a
//! tile (8 m cells) on demand; the session keeps the tiles near riders in
//! wild mode (`ocw-sim`).

use std::collections::BTreeMap;

use crate::bridges::{Bridges, KERB_M, PARAPET_T_M, Railing};
use crate::buildings::Buildings;
use crate::graph::RoadGraph;
use crate::package::{Bytes, TileKey, WorldPackage};

mod streamed;
pub use streamed::{Pole, StreamedLamps};

/// Section tag.
pub const OBSTACLES_TAG: &[u8; 4] = b"OBST";
const OBSTACLES_VERSION: u8 = 1;

/// How far beside a road centre line riders in wild mode go (m; spec
/// §5.2.1, `ocw_sim::physics::WILD_LIMIT_M`).
pub const WILD_REACH_M: f64 = 60.0;
/// The 2 m of the corridor rule: beyond the reach (a rider's half length
/// and a trunk), and the band beside a carriageway whose point barriers
/// are the road's.
pub const CORRIDOR_MARGIN_M: f64 = 2.0;
/// Tunnel walls stand this far beside the road edge (the drawn tube's
/// side, `ocw-render::tunnel`).
pub const TUNNEL_SIDE_M: f64 = 1.0;
/// The largest radius of a point obstacle (a pylon's foot, m): the
/// search margin around a body.
pub const MAX_POINT_RADIUS_M: f64 = 3.0;
/// Size of the cells of a tile's spatial index (m).
pub const INDEX_CELL_M: f64 = 8.0;
/// Railings and tunnel walls only bound riders whose road is within this
/// height of them (m): a road under a bridge passes below its railings.
pub const STRUCTURE_REACH_Y_M: f64 = 3.0;

/// Flag: a point barrier within [`CORRIDOR_MARGIN_M`] of a carriageway –
/// a barrier of the road (drawn with `display.road_barriers`, never solid).
pub const ROAD_BARRIER: u8 = 1;
/// Flag: on the carriageway itself (a road barrier on the road: a gate
/// across a through road is a data error the lint notes, spec 12H).
pub const ON_ROAD: u8 = 2;
/// Flag (stored only): the stored angle is turned by π – a point whose
/// direction matters beyond its way's axis, the face of a
/// [`ObstacleKind::NoBikeSign`] (16H Q6); the angle byte keeps the way's
/// axis (modulo π) as older readers expect it.
pub const TURNED: u8 = 4;

/// What an obstacle is. Codes are stored in packages: only add new ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(u8)]
pub enum ObstacleKind {
    // Points.
    Bollard = 1,
    /// A post: utility poles, power poles, guide posts.
    Post = 2,
    Gate = 3,
    LiftGate = 4,
    /// Concrete blocks, boulders, jersey barriers mapped as nodes.
    Block = 5,
    /// Cycle barriers, stiles, kissing gates, turnstiles.
    CycleBarrier = 6,
    /// Any other `barrier=*` node (chain, rope, …).
    Barrier = 7,
    /// A power line pylon (`power=tower`).
    Pylon = 8,
    /// A tree off the tree grid (spec §14 Phase 8 T8): a mapped tree
    /// (`natural=tree`, tree rows) or a fruit tree of an orchard's rows
    /// (section `VEGE`, `crate::plantings`); its width is the trunk's
    /// diameter. Solid as a [`ObstacleKind::Tree`]; drawn by the vegetation,
    /// not as an object.
    PlantedTree = 9,
    /// A street lamp mapped in OSM (`highway=street_lamp`; world look
    /// *Street lamps*): stored where it is mapped, placed beside the road
    /// by [`crate::lamps::osm_lamp`] (the same in both apps and on the
    /// server) and solid there, as a pole of its model file; never solid
    /// where it is mapped.
    StreetLamp = 10,
    /// A warning sign "no bicycles" at an end of a road a world patch
    /// closed for bikes (16H Q6, [`crate::bike_signs`]): beside the
    /// carriageway, facing the rider who comes onto the road (`angle`:
    /// the direction its face looks). Looks only (not solid); older
    /// readers skip the code.
    NoBikeSign = 11,
    // Lines.
    Fence = 32,
    Wall = 33,
    Hedge = 34,
    GuardRail = 35,
    /// `barrier=kerb` ways: ridden over (spec 12E: bike damage in K4),
    /// never solid.
    Kerb = 36,
    /// A gate mapped as a way.
    GateLine = 37,
    // Derived when loaded (never in `OBST`).
    Railing = 48,
    TunnelWall = 49,
    Building = 50,
    /// A parked car at the kerb or in a car park (spec 14 Phase 18, 18B;
    /// `ocw_sim::traffic::parked`): placed from the traffic data while
    /// the road traffic is on, never stored.
    ParkedCar = 51,
    Tree = 64,
}

impl ObstacleKind {
    pub const ALL: [ObstacleKind; 22] = [
        Self::Bollard,
        Self::Post,
        Self::Gate,
        Self::LiftGate,
        Self::Block,
        Self::CycleBarrier,
        Self::Barrier,
        Self::Pylon,
        Self::PlantedTree,
        Self::StreetLamp,
        Self::NoBikeSign,
        Self::Fence,
        Self::Wall,
        Self::Hedge,
        Self::GuardRail,
        Self::Kerb,
        Self::GateLine,
        Self::Railing,
        Self::TunnelWall,
        Self::Building,
        Self::ParkedCar,
        Self::Tree,
    ];

    pub fn code(self) -> u8 {
        self as u8
    }

    pub fn from_code(c: u8) -> Option<Self> {
        Self::ALL.iter().copied().find(|k| k.code() == c)
    }

    /// The kinds of OSM point barriers and posts (from their tags).
    pub fn of_point(tags: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        if let Some(b) = tags("barrier") {
            return Some(match b.as_str() {
                "bollard" | "rising_bollard" => Self::Bollard,
                "gate" | "swing_gate" | "hampshire_gate" | "sliding_gate" | "wicket_gate" => {
                    Self::Gate
                }
                "lift_gate" | "barrier_board" | "toll_booth" => Self::LiftGate,
                "block" | "jersey_barrier" | "planter" | "boulder" => Self::Block,
                "cycle_barrier"
                | "stile"
                | "kissing_gate"
                | "turnstile"
                | "full-height_turnstile"
                | "motorcycle_barrier"
                | "horse_stile" => Self::CycleBarrier,
                "post" => Self::Post,
                // Not obstacles a rider hits.
                "entrance" | "border_control" | "cattle_grid" | "kerb" | "height_restrictor"
                | "no" | "yes" | "debris" => return None,
                _ => Self::Barrier,
            });
        }
        match (tags("power").as_deref(), tags("man_made").as_deref()) {
            (Some("tower"), _) => Some(Self::Pylon),
            (Some("pole"), _) | (_, Some("utility_pole")) => Some(Self::Post),
            _ => match (tags("information").as_deref(), tags("highway").as_deref()) {
                (Some("guidepost"), _) => Some(Self::Post),
                (_, Some("street_lamp")) => Some(Self::StreetLamp),
                _ => None,
            },
        }
    }

    /// The kind of an OSM barrier way (`barrier=*`).
    pub fn of_line(barrier: &str) -> Option<Self> {
        Some(match barrier {
            "fence" | "railing" | "handrail" | "wire_fence" | "chain_link" => Self::Fence,
            "wall" | "city_wall" | "retaining_wall" | "jersey_barrier" | "block" => Self::Wall,
            "hedge" | "hedge_bank" => Self::Hedge,
            "guard_rail" | "crash_barrier" => Self::GuardRail,
            "kerb" => Self::Kerb,
            "gate" | "swing_gate" | "lift_gate" | "sliding_gate" => Self::GateLine,
            _ => return None,
        })
    }

    pub fn is_line(self) -> bool {
        self.code() >= 32 && self != Self::Tree
    }

    /// Whether riders in wild mode hit it (kerbs are ridden over).
    pub fn solid(self) -> bool {
        !matches!(self, Self::Kerb | Self::NoBikeSign)
    }

    /// Radius of a point obstacle (m).
    pub fn radius_m(self) -> f64 {
        match self {
            Self::Bollard => 0.1,
            Self::Post => 0.15,
            Self::Gate | Self::LiftGate => 0.15,
            Self::Block => 0.5,
            Self::CycleBarrier => 0.6,
            Self::Barrier => 0.3,
            Self::Pylon => 2.5,
            Self::PlantedTree => crate::trees::TRUNK_RADIUS_M,
            Self::StreetLamp => crate::lamps::POLE_R_M,
            _ => 0.0,
        }
    }

    /// The model file a point obstacle is drawn from (spec 15H names,
    /// `artwork/3dmodels/props/`; kinds without their own file use the
    /// generic barrier post). Street lamps have three files by height
    /// (`ocw-render::street_lamps`), whose poles are the same: this is the
    /// middle one. `None`: not drawn as an object (trees).
    pub fn model_id(self) -> Option<&'static str> {
        Some(match self {
            Self::Bollard => "props/bollard-round",
            Self::Post => "props/post-hexagonal",
            // An open lattice tower (v6.58; the solid pyramid stood
            // beside the roads as a grey obelisk).
            Self::Pylon => "props/pylon-lattice",
            Self::Block => "props/block-concrete-box",
            Self::Gate => "props/gate-swing",
            Self::LiftGate => "props/barrier-lift-arm",
            Self::CycleBarrier => "props/barrier-cycle-staggered",
            Self::StreetLamp => "props/lamp-street-arm-road",
            // Drawn from the world data (`ocw-render::obstacles`), not a file.
            Self::PlantedTree | Self::Tree | Self::NoBikeSign | Self::ParkedCar => return None,
            _ => "props/barrier-post-hexagonal",
        })
    }

    /// Half the thickness of a line obstacle (m).
    pub fn half_thickness_m(self) -> f64 {
        match self {
            Self::Fence | Self::GateLine => 0.05,
            Self::Wall => 0.2,
            Self::Hedge => 0.5,
            Self::GuardRail => 0.15,
            Self::Kerb => 0.08,
            Self::Railing => f64::from(PARAPET_T_M) / 2.0,
            Self::TunnelWall => 0.3,
            Self::Building => 0.05,
            _ => 0.0,
        }
    }

    /// Height when OSM says nothing (m).
    pub fn default_height_m(self) -> f64 {
        match self {
            Self::Bollard => 0.9,
            Self::Post => 7.0,
            Self::Gate | Self::GateLine => 1.2,
            Self::LiftGate => 1.0,
            Self::Block => 0.8,
            Self::CycleBarrier => 1.0,
            Self::Barrier => 1.0,
            Self::Pylon => 30.0,
            Self::Fence => 1.2,
            Self::Wall => 1.5,
            Self::Hedge => 1.6,
            Self::GuardRail => 0.75,
            Self::Kerb => 0.12,
            Self::StreetLamp => 6.0,
            Self::ParkedCar => 1.5,
            Self::NoBikeSign => crate::bike_signs::HEIGHT_M as f64,
            _ => 1.0,
        }
    }

    /// Name for the HUD and the logs ("Crash! (tree)").
    pub fn label(self) -> &'static str {
        match self {
            Self::Bollard => "bollard",
            Self::Post => "post",
            Self::Gate | Self::GateLine => "gate",
            Self::LiftGate => "lift gate",
            Self::Block => "block",
            Self::CycleBarrier => "cycle barrier",
            Self::Barrier => "barrier",
            Self::Pylon => "pylon",
            Self::Fence => "fence",
            Self::Wall => "wall",
            Self::Hedge => "hedge",
            Self::GuardRail => "guard rail",
            Self::Kerb => "kerb",
            Self::Railing => "bridge railing",
            Self::TunnelWall => "tunnel wall",
            Self::Building => "building",
            Self::ParkedCar => "parked car",
            Self::Tree | Self::PlantedTree => "tree",
            Self::StreetLamp => "street lamp",
            Self::NoBikeSign => "no-bicycles sign",
        }
    }
}

/// A point obstacle (OSM node).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PointObstacle {
    pub kind: ObstacleKind,
    pub flags: u8,
    /// Package metres (x, z).
    pub pos: [f32; 2],
    /// Direction of the way it is on (rad, modulo π; 0 = +x).
    pub angle: f32,
    /// Width it spans (a gate across a road, m; 0 = its own size).
    pub width_m: f32,
    pub height_m: f32,
}

impl PointObstacle {
    pub fn road_barrier(&self) -> bool {
        self.flags & ROAD_BARRIER != 0
    }

    /// Hit by riders in wild mode where it is stored (road barriers never
    /// are, Q7; a street lamp is hit where it is placed, beside the road:
    /// [`WorldObstacles`] adds the placed lamps).
    pub fn solid(&self) -> bool {
        self.kind.solid() && !self.road_barrier() && self.kind != ObstacleKind::StreetLamp
    }
}

/// A line obstacle (OSM way): fence, wall, hedge …
#[derive(Debug, Clone, PartialEq)]
pub struct LineObstacle {
    pub kind: ObstacleKind,
    pub flags: u8,
    pub height_m: f32,
    /// Package metres (x, z).
    pub points: Vec<[f32; 2]>,
}

/// The obstacles of one tile.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TileObstacles {
    pub points: Vec<PointObstacle>,
    pub lines: Vec<LineObstacle>,
    /// The tree cells, deflated ([`crate::trees::tree_cells`]; empty: none).
    pub trees: Vec<u8>,
}

impl TileObstacles {
    /// The tree cells (one byte per cell), if the tile has trees stored.
    pub fn tree_cells(&self, side: usize) -> Option<Vec<u8>> {
        if self.trees.is_empty() {
            return None;
        }
        miniz_oxide::inflate::decompress_to_vec_with_limit(&self.trees, side * side)
            .ok()
            .filter(|c| c.len() == side * side)
    }

    /// Stores the tree cells (deflated; all empty: none).
    pub fn set_tree_cells(&mut self, cells: &[u8]) {
        self.trees = if cells.iter().all(|c| *c == 0) {
            Vec::new()
        } else {
            miniz_oxide::deflate::compress_to_vec(cells, 9)
        };
    }

    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.lines.is_empty() && self.trees.is_empty()
    }
}

/// The section `OBST`: obstacles per tile.
#[derive(Debug, Clone, PartialEq)]
pub struct Obstacles {
    /// Tree cells per tile side (125 for 1 km tiles: 8 m).
    pub tree_side: u16,
    pub tiles: BTreeMap<TileKey, TileObstacles>,
}

impl Obstacles {
    pub fn new(tile_size_m: f32) -> Self {
        Self {
            tree_side: crate::trees::cells_per_side(tile_size_m) as u16,
            tiles: BTreeMap::new(),
        }
    }

    /// Adds a point obstacle to the tile it is in.
    pub fn add_point(&mut self, p: PointObstacle, tile_size_m: f32) {
        let key = TileKey::containing(
            [f64::from(p.pos[0]), f64::from(p.pos[1])],
            f64::from(tile_size_m),
        );
        self.tiles.entry(key).or_default().points.push(p);
    }

    /// Takes out every point obstacle of `kind` (a step that places them
    /// again); empty tiles go.
    pub fn take_points(&mut self, kind: ObstacleKind) -> Vec<PointObstacle> {
        let mut out = Vec::new();
        for t in self.tiles.values_mut() {
            let (k, rest): (Vec<_>, Vec<_>) = t.points.drain(..).partition(|p| p.kind == kind);
            t.points = rest;
            out.extend(k);
        }
        self.tiles.retain(|_, t| !t.is_empty());
        out
    }

    /// Counts per kind (points, lines) and stored trees, for reports.
    pub fn counts(&self) -> (BTreeMap<ObstacleKind, usize>, usize) {
        let mut by = BTreeMap::new();
        let mut trees = 0;
        let side = usize::from(self.tree_side);
        for t in self.tiles.values() {
            for p in &t.points {
                *by.entry(p.kind).or_default() += 1;
            }
            for l in &t.lines {
                *by.entry(l.kind).or_default() += 1;
            }
            trees += t
                .tree_cells(side)
                .map_or(0, |c| c.iter().filter(|c| **c != 0).count());
        }
        (by, trees)
    }
}

fn dm(v: f32) -> u8 {
    (v * 10.0).round().clamp(0.0, 255.0) as u8
}

pub fn write_obstacles(o: &Obstacles, b: &mut Vec<u8>) {
    b.push(OBSTACLES_VERSION);
    b.extend(o.tree_side.to_le_bytes());
    b.extend((o.tiles.len() as u32).to_le_bytes());
    for (key, t) in &o.tiles {
        b.extend(key.x.to_le_bytes());
        b.extend(key.z.to_le_bytes());
        b.extend((t.points.len() as u32).to_le_bytes());
        for p in &t.points {
            b.push(p.kind.code());
            // A sign's face keeps its side (the angle byte is modulo π).
            let full = p.angle.rem_euclid(std::f32::consts::TAU);
            let turned = p.kind == ObstacleKind::NoBikeSign && full >= std::f32::consts::PI;
            b.push((p.flags & !TURNED) | if turned { TURNED } else { 0 });
            b.extend(p.pos[0].to_le_bytes());
            b.extend(p.pos[1].to_le_bytes());
            let a = p.angle.rem_euclid(std::f32::consts::PI) / std::f32::consts::PI * 256.0;
            b.push((a.round() as u32 % 256) as u8);
            b.push(dm(p.width_m));
            b.push(dm(p.height_m));
        }
        b.extend((t.lines.len() as u32).to_le_bytes());
        for l in &t.lines {
            b.push(l.kind.code());
            b.push(l.flags);
            b.push(dm(l.height_m));
            b.extend((l.points.len() as u32).to_le_bytes());
            for p in &l.points {
                b.extend(p[0].to_le_bytes());
                b.extend(p[1].to_le_bytes());
            }
        }
        b.extend((t.trees.len() as u32).to_le_bytes());
        b.extend(&t.trees);
    }
}

/// Reads the section; an unknown layout is "not supported" (skipped by
/// the package reader), unknown kinds are left out.
pub fn read_obstacles(bytes: &[u8]) -> Result<Obstacles, String> {
    let mut r = Bytes::new(bytes);
    let version = r.u8()?;
    if version != OBSTACLES_VERSION {
        return Err(format!("obstacles layout {version} is not supported"));
    }
    let tree_side = u16::from_le_bytes(r.arr()?);
    let mut tiles = BTreeMap::new();
    for _ in 0..r.u32()? {
        let key = TileKey {
            x: r.i32()?,
            z: r.i32()?,
        };
        let mut t = TileObstacles::default();
        for _ in 0..r.u32()? {
            let kind = r.u8()?;
            let flags = r.u8()?;
            let pos = [r.f32()?, r.f32()?];
            let mut angle = f32::from(r.u8()?) / 256.0 * std::f32::consts::PI;
            if flags & TURNED != 0 {
                angle += std::f32::consts::PI;
            }
            let flags = flags & !TURNED;
            let width_m = f32::from(r.u8()?) / 10.0;
            let height_m = f32::from(r.u8()?) / 10.0;
            if let Some(kind) = ObstacleKind::from_code(kind) {
                t.points.push(PointObstacle {
                    kind,
                    flags,
                    pos,
                    angle,
                    width_m,
                    height_m,
                });
            }
        }
        for _ in 0..r.u32()? {
            let kind = r.u8()?;
            let flags = r.u8()?;
            let height_m = f32::from(r.u8()?) / 10.0;
            let n = r.u32()? as usize;
            let points = (0..n)
                .map(|_| Ok([r.f32()?, r.f32()?]))
                .collect::<Result<Vec<_>, String>>()?;
            if let Some(kind) = ObstacleKind::from_code(kind) {
                t.lines.push(LineObstacle {
                    kind,
                    flags,
                    height_m,
                    points,
                });
            }
        }
        let n = r.u32()? as usize;
        t.trees = r.take(n)?.to_vec();
        tiles.insert(key, t);
    }
    r.finish("obstacles section")?;
    Ok(Obstacles { tree_side, tiles })
}

/// A solid shape of the index (package metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    Circle {
        c: [f64; 2],
        r: f64,
    },
    /// A wall from `a` to `b`, `half` its half thickness.
    Segment {
        a: [f64; 2],
        b: [f64; 2],
        half: f64,
    },
}

/// A solid obstacle in a tile's index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Solid {
    pub shape: Shape,
    pub kind: ObstacleKind,
    /// Heights of the structure (railings, tunnel walls: the road height
    /// at their foot, lowest and highest); `None` on the ground.
    pub y: Option<[f32; 2]>,
    /// Stable id (the same shape from several tiles counts once).
    pub id: u64,
}

impl Solid {
    /// Whether a rider at height `y` (its road's) can touch it.
    pub fn reaches(&self, y: f64) -> bool {
        self.y.is_none_or(|[lo, hi]| {
            y >= f64::from(lo) - STRUCTURE_REACH_Y_M && y <= f64::from(hi) + STRUCTURE_REACH_Y_M
        })
    }

    /// The box around it (lo, hi).
    pub fn bounds(&self) -> ([f64; 2], [f64; 2]) {
        match self.shape {
            Shape::Circle { c, r } => ([c[0] - r, c[1] - r], [c[0] + r, c[1] + r]),
            Shape::Segment { a, b, half } => (
                [a[0].min(b[0]) - half, a[1].min(b[1]) - half],
                [a[0].max(b[0]) + half, a[1].max(b[1]) + half],
            ),
        }
    }
}

/// The yaw (rad about +y) a point obstacle's model is turned by, from
/// the direction of its way (`angle`): the model's +x across the way
/// (a gate's bars span the road), as the renderer draws it.
pub fn yaw_of_angle(angle: f64) -> f64 {
    -(angle + std::f64::consts::FRAC_PI_2)
}

/// The yaw of a street lamp's model facing `toward` (x, z): its +x, the
/// arm, points that way (as `ocw-render::street_lamps` draws it).
pub fn yaw_toward(toward: [f32; 2]) -> f64 {
    f64::from(-toward[1]).atan2(f64::from(toward[0]))
}

/// A street lamp's model yaw (its arm towards the road).
fn lamp_yaw(l: &crate::lamps::Lamp) -> f32 {
    yaw_toward(l.toward) as f32
}

/// A point of the model's footprint (mx, mz) turned by `yaw` about +y,
/// as world (x, z) offsets.
fn turned(yaw: f64, m: [f64; 2]) -> [f64; 2] {
    let (s, c) = yaw.sin_cos();
    [m[0] * c + m[1] * s, -m[0] * s + m[1] * c]
}

/// One part of a point obstacle's solid footprint in its model's frame
/// (metres from the point; x and z of the model file, before the yaw).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ShapePart {
    /// A post (a cylinder or capsule proxy): a circle of radius `half[0]`;
    /// else a box of half extents `half`.
    pub round: bool,
    pub centre: [f64; 2],
    pub half: [f64; 2],
}

impl ShapePart {
    pub fn circle(r: f64) -> Self {
        Self {
            round: true,
            centre: [0.0, 0.0],
            half: [r, r],
        }
    }

    /// The solid it is at `pos`, turned by `yaw`: a circle, or a box as a
    /// wall along its longer side (its half thickness the shorter half
    /// extent; a square box is a circle of that size).
    pub fn shape(&self, pos: [f64; 2], yaw: f64) -> Shape {
        let c = turned(yaw, self.centre);
        let c = [pos[0] + c[0], pos[1] + c[1]];
        if self.round {
            return Shape::Circle { c, r: self.half[0] };
        }
        let (long, short, axis) = if self.half[0] >= self.half[1] {
            (self.half[0], self.half[1], [1.0, 0.0])
        } else {
            (self.half[1], self.half[0], [0.0, 1.0])
        };
        let l = long - short;
        if l < 1e-3 {
            return Shape::Circle { c, r: short };
        }
        let d = turned(yaw, [axis[0] * l, axis[1] * l]);
        Shape::Segment {
            a: [c[0] - d[0], c[1] - d[1]],
            b: [c[0] + d[0], c[1] + d[1]],
            half: short,
        }
    }

    /// How far it reaches from the point (m).
    pub fn reach_m(&self) -> f64 {
        let r = if self.round {
            self.half[0]
        } else {
            self.half[0].hypot(self.half[1])
        };
        self.centre[0].hypot(self.centre[1]) + r
    }
}

/// The solid footprints of the point obstacles by kind (spec 15H / 15N:
/// from the collision proxies `col_*` of their model files, read by
/// `ocw_sim::collide::props_shapes`; the server and the client's
/// prediction use the same table). Kinds it does not list are circles of
/// [`ObstacleKind::radius_m`] – the hard-coded sizes of before, also
/// [`PointShapes::radii`].
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PointShapes {
    parts: BTreeMap<ObstacleKind, Vec<ShapePart>>,
}

impl PointShapes {
    /// Every kind a circle of its [`ObstacleKind::radius_m`] (the sizes
    /// before the files' proxies were read).
    pub fn radii() -> Self {
        Self::default()
    }

    /// The footprints of these kinds (the others: [`ObstacleKind::radius_m`]).
    pub fn new(parts: BTreeMap<ObstacleKind, Vec<ShapePart>>) -> Self {
        Self { parts }
    }

    /// The footprint of `kind`.
    pub fn parts(&self, kind: ObstacleKind) -> Vec<ShapePart> {
        match self.parts.get(&kind) {
            Some(p) if !p.is_empty() => p.clone(),
            _ => vec![ShapePart::circle(kind.radius_m())],
        }
    }

    /// How far `kind`'s footprint reaches from its point (m).
    pub fn reach_m(&self, kind: ObstacleKind) -> f64 {
        self.parts(kind)
            .iter()
            .map(ShapePart::reach_m)
            .fold(0.0, f64::max)
    }
}

/// A segment of a line obstacle, by tile.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Piece {
    a: [f32; 2],
    b: [f32; 2],
    kind: ObstacleKind,
    y: Option<[f32; 2]>,
    id: u64,
}

/// What a tile's index is built from.
#[derive(Debug, Clone, Default, PartialEq)]
struct TileSource {
    /// Solid OSM points.
    points: Vec<PointObstacle>,
    /// Line pieces reaching into the tile (OSM lines, railings, tunnel
    /// walls).
    pieces: Vec<Piece>,
    /// The rings of the buildings reaching into the tile (walls; kept as
    /// points, a fifth of their segments' memory): per ring its number and
    /// the range of its points in `ring_points`.
    rings: Vec<(u32, u32, u32)>,
    ring_points: Vec<[f32; 2]>,
    /// Deflated tree cells (the tile's own).
    trees: Vec<u8>,
    /// The street lamps placed in the tile (both kinds: OSM's and the
    /// rows along lit roads, [`crate::lamps::tile_lamps`]): foot (x, z)
    /// and the yaw of their model.
    lamps: Vec<([f32; 2], f32)>,
}

/// The static obstacles of a world, ready for the collision model: the
/// `OBST` section plus the buildings, bridge railings and tunnel walls
/// of the package. Pure and wasm-safe; shared (in an `Arc`) by the
/// server's session and the client's prediction.
#[derive(Debug, Clone, PartialEq)]
pub struct WorldObstacles {
    tile_size_m: f32,
    tree_side: usize,
    tiles: BTreeMap<TileKey, TileSource>,
    /// The street lamp poles of a streamed package's resident tiles (the
    /// browser; [`Self::add_streamed_lamps`]), after the package's own.
    streamed: StreamedLamps,
}

/// A line's id: its source, its number and the segment.
fn piece_id(source: u64, line: usize, seg: usize) -> u64 {
    source << 56 | (line as u64) << 20 | seg as u64
}

impl WorldObstacles {
    /// From a package (its base suffices: tiles are not needed). `None`
    /// when it has nothing a rider could hit.
    pub fn from_package(pkg: &WorldPackage) -> Option<Self> {
        let w = Self::base(pkg);
        (!w.tiles.is_empty()).then_some(w)
    }

    /// From a streamed package's base: [`Self::from_package`], also when
    /// nothing in the base is solid – the tiles' street lamps come as they
    /// stream in ([`Self::add_streamed_lamps`]).
    pub fn from_streamed_base(pkg: &WorldPackage) -> Self {
        Self::base(pkg)
    }

    fn base(pkg: &WorldPackage) -> Self {
        let mut w = Self {
            tile_size_m: pkg.tile_size_m,
            tree_side: crate::trees::cells_per_side(pkg.tile_size_m),
            tiles: BTreeMap::new(),
            streamed: StreamedLamps::default(),
        };
        if let Some(o) = &pkg.obstacles {
            w.tree_side = usize::from(o.tree_side);
            let mut n = 0usize;
            for (key, t) in &o.tiles {
                let src = w.tiles.entry(*key).or_default();
                src.points
                    .extend(t.points.iter().filter(|p| p.solid()).copied());
                src.trees = t.trees.clone();
                for l in &t.lines {
                    n += 1;
                    if l.kind.solid() {
                        let pts: Vec<[f32; 2]> = l.points.clone();
                        w.add_line(&pts, None, l.kind, 1, n);
                    }
                }
            }
        }
        if let Some(b) = &pkg.buildings {
            w.add_buildings(b);
        }
        if let Some(b) = &pkg.bridges {
            w.add_railings(b);
        }
        if let Some(g) = &pkg.graph {
            w.add_tunnel_walls(g);
        }
        // The street lamps as the apps place them (the package's tiles
        // needed: none from a streamed base).
        // Each pole in the tile its foot stands in (a strip's lamps can
        // stand a few metres into the next tile).
        let size = f64::from(pkg.tile_size_m);
        for lamps in crate::lamps::package_lamps(pkg).into_values() {
            for l in lamps {
                let at = [f64::from(l.foot[0]), f64::from(l.foot[2])];
                w.tiles
                    .entry(TileKey::containing(at, size))
                    .or_default()
                    .lamps
                    .push(([l.foot[0], l.foot[2]], lamp_yaw(&l)));
            }
        }
        w
    }

    /// The street lamps placed (poles a rider can hit), the streamed ones
    /// included.
    pub fn lamp_count(&self) -> usize {
        self.tiles.values().map(|t| t.lamps.len()).sum::<usize>() + self.streamed.count()
    }

    /// The street lamp poles standing in tile `key`: foot (x, z) and model
    /// yaw (the package's, then the streamed ones).
    pub fn tile_lamp_feet(&self, key: TileKey) -> Vec<Pole> {
        let mut out = self
            .tiles
            .get(&key)
            .map_or_else(Vec::new, |t| t.lamps.clone());
        out.extend(self.streamed.poles_in(key));
        out
    }

    /// The street lamps placed by a streamed tile `source` (spec 14 Phase
    /// 12 K3, the browser: [`crate::lamps::tile_lamps`] of the resident
    /// tile, as the renderer places them), replacing its earlier ones. The
    /// server and the prediction sharing this see them from now on.
    pub fn add_streamed_lamps(&self, source: TileKey, lamps: &[crate::lamps::Lamp]) {
        self.streamed
            .set(source, lamps, f64::from(self.tile_size_m), lamp_yaw);
    }

    /// Brings the streamed poles to the tiles of `wanted` (tiles a
    /// streamed package holds now – the browser: those near the rider):
    /// those that have none yet are placed with `place` (the lamps a
    /// tile's strips and mapped lamps place, [`crate::lamps::tile_lamps`]),
    /// the tiles no longer wanted dropped. Returns (placed, dropped) tiles.
    pub fn sync_streamed_lamps(
        &self,
        wanted: &[TileKey],
        place: impl Fn(TileKey) -> Vec<crate::lamps::Lamp>,
    ) -> (usize, usize) {
        let mut dropped = 0;
        for key in self.streamed.sources() {
            if !wanted.contains(&key) {
                self.streamed.drop_source(key);
                dropped += 1;
            }
        }
        let mut placed = 0;
        for &key in wanted {
            if !self.streamed.has_source(key) {
                self.add_streamed_lamps(key, &place(key));
                placed += 1;
            }
        }
        (placed, dropped)
    }

    /// Drops the street lamps of a streamed tile that left the memory.
    pub fn drop_streamed_lamps(&self, source: TileKey) {
        self.streamed.drop_source(source);
    }

    /// The streamed tiles whose lamps are placed.
    pub fn streamed_lamp_tiles(&self) -> Vec<TileKey> {
        self.streamed.sources()
    }

    pub fn has_streamed_lamps(&self, source: TileKey) -> bool {
        self.streamed.has_source(source)
    }

    /// The version of tile `key`'s solids: changes when streamed poles
    /// come into it or go (a cached index of another version is stale).
    pub fn tile_version(&self, key: TileKey) -> u64 {
        self.streamed.version(key)
    }

    /// Adds the segments of a line to every tile they reach.
    fn add_line(
        &mut self,
        pts: &[[f32; 2]],
        ys: Option<&[f32]>,
        kind: ObstacleKind,
        source: u64,
        line: usize,
    ) {
        let size = f64::from(self.tile_size_m);
        let m = kind.half_thickness_m() + 0.5;
        for (s, w) in pts.windows(2).enumerate() {
            let (a, b) = (w[0], w[1]);
            if a == b {
                continue;
            }
            let y = ys.map(|ys| {
                let (ya, yb) = (ys[s], ys[s + 1]);
                [ya.min(yb), ya.max(yb)]
            });
            let lo = TileKey::containing(
                [f64::from(a[0].min(b[0])) - m, f64::from(a[1].min(b[1])) - m],
                size,
            );
            let hi = TileKey::containing(
                [f64::from(a[0].max(b[0])) + m, f64::from(a[1].max(b[1])) + m],
                size,
            );
            for x in lo.x..=hi.x {
                for z in lo.z..=hi.z {
                    self.tiles
                        .entry(TileKey { x, z })
                        .or_default()
                        .pieces
                        .push(Piece {
                            a,
                            b,
                            kind,
                            y,
                            id: piece_id(source, line, s),
                        });
                }
            }
        }
    }

    fn add_buildings(&mut self, buildings: &Buildings) {
        let size = f64::from(self.tile_size_m);
        let mut n = 0u32;
        for list in buildings.values() {
            for b in list {
                for ring in std::iter::once(&b.outline).chain(&b.holes) {
                    n += 1;
                    if ring.len() < 2 {
                        continue;
                    }
                    let (mut lo, mut hi) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
                    for p in ring {
                        for k in 0..2 {
                            lo[k] = lo[k].min(f64::from(p[k]));
                            hi[k] = hi[k].max(f64::from(p[k]));
                        }
                    }
                    let a = TileKey::containing([lo[0] - 0.5, lo[1] - 0.5], size);
                    let b = TileKey::containing([hi[0] + 0.5, hi[1] + 0.5], size);
                    for x in a.x..=b.x {
                        for z in a.z..=b.z {
                            let src = self.tiles.entry(TileKey { x, z }).or_default();
                            let start = src.ring_points.len() as u32;
                            src.ring_points.extend_from_slice(ring);
                            src.rings.push((n, start, ring.len() as u32));
                        }
                    }
                }
            }
        }
    }

    fn add_railings(&mut self, bridges: &Bridges) {
        let mut n = 0;
        for list in bridges.values() {
            for b in list {
                if b.points.len() < 2 {
                    continue;
                }
                let inset = match b.railing {
                    Railing::Parapet => PARAPET_T_M / 2.0,
                    Railing::Rail => 0.1,
                };
                let half = b.width_m / 2.0 + KERB_M - inset;
                for side in [-1.0f32, 1.0] {
                    n += 1;
                    let (pts, ys): (Vec<[f32; 2]>, Vec<f32>) = (0..b.points.len())
                        .map(|i| {
                            let (c, r) = crate::bridges::frame(&b.points, i);
                            ([c[0] + r[0] * side * half, c[2] + r[2] * side * half], c[1])
                        })
                        .unzip();
                    self.add_line(&pts, Some(&ys), ObstacleKind::Railing, 3, n);
                }
            }
        }
    }

    fn add_tunnel_walls(&mut self, g: &RoadGraph) {
        let mut n = 0;
        for e in g.edges.iter().filter(|e| e.tunnel && e.points.len() >= 2) {
            let half = e.width_m / 2.0 + TUNNEL_SIDE_M;
            let mut along = 0.0;
            let mut samples = Vec::with_capacity(e.points.len());
            for (i, p) in e.points.iter().enumerate() {
                if i > 0 {
                    let q = e.points[i - 1];
                    along += (p[0] - q[0]).hypot(p[1] - q[1]);
                }
                let (a, b) = (
                    e.points[i.saturating_sub(1)],
                    e.points[(i + 1).min(e.points.len() - 1)],
                );
                let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
                let l = dx.hypot(dz).max(1e-9);
                samples.push((*p, [-dz / l, dx / l], e.height_at(along)));
            }
            for side in [-1.0, 1.0] {
                n += 1;
                let pts: Vec<[f32; 2]> = samples
                    .iter()
                    .map(|(p, r, _)| {
                        [
                            (p[0] + r[0] * side * half) as f32,
                            (p[1] + r[1] * side * half) as f32,
                        ]
                    })
                    .collect();
                let ys: Option<Vec<f32>> = samples
                    .iter()
                    .map(|(_, _, y)| y.map(|y| y as f32))
                    .collect();
                self.add_line(&pts, ys.as_deref(), ObstacleKind::TunnelWall, 4, n);
            }
        }
    }

    pub fn tile_size_m(&self) -> f32 {
        self.tile_size_m
    }

    /// The tree cells of tile `key` stored in the section (the grid's
    /// trees within wild mode's reach, [`crate::trees::tree_cells`]; one
    /// byte per candidate, `side`² of them), if any – the trunks made off
    /// the road leave these out ([`crate::woods`]).
    pub fn tile_tree_cells(&self, key: TileKey) -> Option<(usize, Vec<u8>)> {
        let src = self.tiles.get(&key)?;
        let side = self.tree_side;
        TileObstacles {
            trees: src.trees.clone(),
            ..Default::default()
        }
        .tree_cells(side)
        .map(|c| (side, c))
    }

    /// The feet (x, z) of the planted trees stored for tile `key` (orchard
    /// rows and mapped trees within wild mode's reach).
    pub fn tile_planted_feet(&self, key: TileKey) -> Vec<[f32; 2]> {
        self.tiles.get(&key).map_or_else(Vec::new, |t| {
            t.points
                .iter()
                .filter(|p| p.kind == ObstacleKind::PlantedTree)
                .map(|p| p.pos)
                .collect()
        })
    }

    /// Approximate memory (bytes), for the logs.
    pub fn approx_bytes(&self) -> usize {
        self.tiles
            .values()
            .map(|t| {
                t.points.len() * std::mem::size_of::<PointObstacle>()
                    + t.pieces.len() * std::mem::size_of::<Piece>()
                    + t.rings.len() * 12
                    + t.ring_points.len() * 8
                    + t.trees.len()
                    + t.lamps.len() * 12
                    + std::mem::size_of::<TileSource>()
            })
            .sum::<usize>()
            + self.streamed.approx_bytes()
    }

    /// Tiles with anything solid (those with streamed poles included).
    pub fn tiles(&self) -> impl Iterator<Item = TileKey> + '_ {
        let mut keys: Vec<TileKey> = self.tiles.keys().copied().collect();
        keys.extend(self.streamed.foot_tiles());
        keys.sort();
        keys.dedup();
        keys.into_iter()
    }

    /// The spatial index of a tile (`None`: nothing solid there), the
    /// point obstacles circles of [`ObstacleKind::radius_m`]
    /// ([`PointShapes::radii`]; the ride uses [`Self::tile_index_with`]
    /// and the model files' proxies).
    pub fn tile_index(&self, key: TileKey) -> Option<TileIndex> {
        self.tile_index_with(key, &PointShapes::radii())
    }

    /// The spatial index of a tile with the point obstacles' footprints
    /// of `shapes` (spec 15H: the model files' collision proxies), the
    /// street lamps' poles included.
    pub fn tile_index_with(&self, key: TileKey, shapes: &PointShapes) -> Option<TileIndex> {
        let empty = TileSource::default();
        let src = self.tiles.get(&key).unwrap_or(&empty);
        let mut solids = Vec::new();
        let tile_id = (key.x as u32 as u64 & 0xFFFF) << 40 | (key.z as u32 as u64 & 0xFFFF) << 24;
        let mut part = 0u64;
        for p in &src.points {
            let c = [f64::from(p.pos[0]), f64::from(p.pos[1])];
            // Planted trees hit as trees, their trunk as wide as stored.
            let tree = match p.kind {
                ObstacleKind::PlantedTree if p.width_m > 0.0 => Some(f64::from(p.width_m) / 2.0),
                ObstacleKind::PlantedTree => Some(p.kind.radius_m()),
                _ => None,
            };
            let (kind, parts) = match tree {
                Some(r) => (ObstacleKind::Tree, vec![ShapePart::circle(r)]),
                None => (p.kind, shapes.parts(p.kind)),
            };
            let yaw = yaw_of_angle(f64::from(p.angle));
            for sp in parts {
                solids.push(Solid {
                    shape: sp.shape(c, yaw),
                    kind,
                    y: None,
                    id: 5 << 56 | tile_id | part,
                });
                part += 1;
            }
        }
        let poles = shapes.parts(ObstacleKind::StreetLamp);
        let streamed = self.streamed.poles_in(key);
        for (i, (foot, yaw)) in src.lamps.iter().chain(&streamed).enumerate() {
            let c = [f64::from(foot[0]), f64::from(foot[1])];
            for (k, sp) in poles.iter().enumerate() {
                solids.push(Solid {
                    shape: sp.shape(c, f64::from(*yaw)),
                    kind: ObstacleKind::StreetLamp,
                    y: None,
                    id: 7 << 56 | tile_id | (i as u64) << 2 | (k as u64 & 3),
                });
            }
        }
        for &(n, start, len) in &src.rings {
            let ring = &src.ring_points[start as usize..(start + len) as usize];
            for (k, a) in ring.iter().enumerate() {
                let b = ring[(k + 1) % ring.len()];
                if *a == b {
                    continue;
                }
                solids.push(Solid {
                    shape: Shape::Segment {
                        a: [f64::from(a[0]), f64::from(a[1])],
                        b: [f64::from(b[0]), f64::from(b[1])],
                        half: ObstacleKind::Building.half_thickness_m(),
                    },
                    kind: ObstacleKind::Building,
                    y: None,
                    id: piece_id(2, n as usize, k),
                });
            }
        }
        for p in &src.pieces {
            solids.push(Solid {
                shape: Shape::Segment {
                    a: [f64::from(p.a[0]), f64::from(p.a[1])],
                    b: [f64::from(p.b[0]), f64::from(p.b[1])],
                    half: p.kind.half_thickness_m(),
                },
                kind: p.kind,
                y: p.y,
                id: p.id,
            });
        }
        if !src.trees.is_empty() {
            let side = self.tree_side;
            let cells = TileObstacles {
                trees: src.trees.clone(),
                ..Default::default()
            }
            .tree_cells(side);
            if let Some(cells) = cells {
                for (i, (c, r)) in crate::trees::trunks_from_cells(key, self.tile_size_m, &cells)
                    .into_iter()
                    .enumerate()
                {
                    solids.push(Solid {
                        shape: Shape::Circle { c, r },
                        kind: ObstacleKind::Tree,
                        y: None,
                        id: 6 << 56
                            | (key.x as u32 as u64 & 0xFFFF) << 40
                            | (key.z as u32 as u64 & 0xFFFF) << 24
                            | i as u64,
                    });
                }
            }
        }
        (!solids.is_empty()).then(|| {
            TileIndex::new(
                key.origin(f64::from(self.tile_size_m)),
                self.tile_size_m,
                solids,
            )
        })
    }
}

/// The solid obstacles of one tile in a grid of [`INDEX_CELL_M`] cells:
/// each solid is listed in every cell its box touches (within the tile).
#[derive(Debug, Clone, PartialEq)]
pub struct TileIndex {
    origin: [f64; 2],
    side: usize,
    solids: Vec<Solid>,
    /// Per cell the range of its entries in `items` (CSR).
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl TileIndex {
    pub fn new(origin: [f64; 2], tile_size_m: f32, solids: Vec<Solid>) -> Self {
        let side = (f64::from(tile_size_m) / INDEX_CELL_M).ceil() as usize;
        let cell =
            |v: f64, o: f64| (((v - o) / INDEX_CELL_M).floor().max(0.0) as usize).min(side - 1);
        let range = |lo: [f64; 2], hi: [f64; 2]| -> Option<([usize; 2], [usize; 2])> {
            let end = f64::from(tile_size_m);
            if hi[0] < origin[0]
                || hi[1] < origin[1]
                || lo[0] > origin[0] + end
                || lo[1] > origin[1] + end
            {
                return None;
            }
            Some((
                [cell(lo[0], origin[0]), cell(lo[1], origin[1])],
                [cell(hi[0], origin[0]), cell(hi[1], origin[1])],
            ))
        };
        let mut counts = vec![0u32; side * side + 1];
        let each = |f: &mut dyn FnMut(usize, u32)| {
            for (i, s) in solids.iter().enumerate() {
                let (lo, hi) = s.bounds();
                let Some((a, b)) = range(lo, hi) else {
                    continue;
                };
                for k in a[1]..=b[1] {
                    for j in a[0]..=b[0] {
                        if let Shape::Segment { a: p, b: q, half } = s.shape {
                            // Long walls: only the cells they pass.
                            let c = [
                                origin[0] + (j as f64 + 0.5) * INDEX_CELL_M,
                                origin[1] + (k as f64 + 0.5) * INDEX_CELL_M,
                            ];
                            if segment_distance(c, p, q) > half + INDEX_CELL_M * 0.71 {
                                continue;
                            }
                        }
                        f(k * side + j, i as u32);
                    }
                }
            }
        };
        each(&mut |c, _| counts[c + 1] += 1);
        for c in 0..side * side {
            counts[c + 1] += counts[c];
        }
        let mut fill = counts.clone();
        let mut items = vec![0u32; counts[side * side] as usize];
        each(&mut |c, i| {
            items[fill[c] as usize] = i;
            fill[c] += 1;
        });
        Self {
            origin,
            side,
            solids,
            starts: counts,
            items,
        }
    }

    pub fn len(&self) -> usize {
        self.solids.len()
    }

    pub fn is_empty(&self) -> bool {
        self.solids.is_empty()
    }

    pub fn solids(&self) -> &[Solid] {
        &self.solids
    }

    /// The solids whose cells overlap the box (lo, hi), each once.
    pub fn query(&self, lo: [f64; 2], hi: [f64; 2], out: &mut Vec<Solid>) {
        let end = self.side as f64 * INDEX_CELL_M;
        if hi[0] < self.origin[0]
            || hi[1] < self.origin[1]
            || lo[0] > self.origin[0] + end
            || lo[1] > self.origin[1] + end
        {
            return;
        }
        let cell = |v: f64, o: f64| {
            (((v - o) / INDEX_CELL_M).floor().max(0.0) as usize).min(self.side - 1)
        };
        let (a, b) = (
            [cell(lo[0], self.origin[0]), cell(lo[1], self.origin[1])],
            [cell(hi[0], self.origin[0]), cell(hi[1], self.origin[1])],
        );
        let first = out.len();
        for k in a[1]..=b[1] {
            for j in a[0]..=b[0] {
                let c = k * self.side + j;
                for &i in &self.items[self.starts[c] as usize..self.starts[c + 1] as usize] {
                    let s = self.solids[i as usize];
                    if !out[first..].iter().any(|o| o.id == s.id) {
                        out.push(s);
                    }
                }
            }
        }
    }
}

/// Distance from `p` to the segment `a`–`b`.
pub fn segment_distance(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dz * dz;
    let t = if l2 > 0.0 {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dz) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p[0] - a[0] - dx * t).hypot(p[1] - a[1] - dz * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_package() -> WorldPackage {
        WorldPackage {
            name: "Empty".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: BTreeMap::new(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
            traffic: None,
            aero: None,
        }
    }

    fn sample() -> Obstacles {
        let mut o = Obstacles::new(1000.0);
        let mut t = TileObstacles {
            points: vec![
                PointObstacle {
                    kind: ObstacleKind::Bollard,
                    flags: ROAD_BARRIER | ON_ROAD,
                    pos: [10.0, 20.0],
                    angle: 1.0,
                    width_m: 0.0,
                    height_m: 0.9,
                },
                PointObstacle {
                    kind: ObstacleKind::Post,
                    flags: 0,
                    pos: [30.5, 40.25],
                    angle: 0.0,
                    width_m: 0.0,
                    height_m: 7.0,
                },
            ],
            lines: vec![LineObstacle {
                kind: ObstacleKind::Fence,
                flags: 0,
                height_m: 1.2,
                points: vec![[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]],
            }],
            trees: Vec::new(),
        };
        let mut cells = vec![0u8; 125 * 125];
        cells[17] = crate::landuse::LandClass::Forest.code() + 1;
        t.set_tree_cells(&cells);
        o.tiles.insert(TileKey { x: 0, z: 0 }, t);
        o
    }

    /// 16H Q6 regression: a warning sign's face keeps its side through
    /// the section (the angle byte alone is modulo π – a sign facing north
    /// came back facing south, showing riders its back).
    #[test]
    fn a_signs_facing_survives_the_section() {
        let mut o = Obstacles::new(1000.0);
        for (i, a) in [-1.6_f32, 1.5, 3.5, 0.2].into_iter().enumerate() {
            o.add_point(
                PointObstacle {
                    kind: ObstacleKind::NoBikeSign,
                    flags: 0,
                    pos: [10.0 * i as f32, 5.0],
                    angle: a,
                    width_m: 0.0,
                    height_m: 2.3,
                },
                1000.0,
            );
        }
        let mut b = Vec::new();
        write_obstacles(&o, &mut b);
        let back = read_obstacles(&b).unwrap();
        let got = &back.tiles[&TileKey { x: 0, z: 0 }].points;
        for (p, q) in o.tiles[&TileKey { x: 0, z: 0 }].points.iter().zip(got) {
            let d = (p.angle - q.angle).rem_euclid(std::f32::consts::TAU);
            assert!(
                d.min(std::f32::consts::TAU - d) < 0.02,
                "{} -> {}",
                p.angle,
                q.angle
            );
            assert_eq!(q.flags, 0, "the stored flag is not kept in memory");
        }
    }

    #[test]
    fn section_round_trip() {
        let o = sample();
        let mut b = Vec::new();
        write_obstacles(&o, &mut b);
        let back = read_obstacles(&b).unwrap();
        let (t, u) = (
            &o.tiles[&TileKey { x: 0, z: 0 }],
            &back.tiles[&TileKey { x: 0, z: 0 }],
        );
        assert_eq!(t.lines, u.lines);
        assert_eq!(t.trees, u.trees);
        assert_eq!(u.points[1].pos, [30.5, 40.25]);
        assert!((u.points[0].angle - 1.0).abs() < 0.02);
        assert_eq!(u.points[0].flags, ROAD_BARRIER | ON_ROAD);
        assert_eq!(u.tree_cells(125).unwrap()[17], 4);
        // A newer layout is "not supported" (skipped by the package reader).
        b[0] = 9;
        assert!(read_obstacles(&b).unwrap_err().contains("not supported"));
        let (by, trees) = o.counts();
        assert_eq!((by[&ObstacleKind::Bollard], trees), (1, 1));
    }

    #[test]
    fn kinds_from_osm_tags() {
        let of = |k: &'static str, v: &'static str| {
            ObstacleKind::of_point(&move |key: &str| (key == k).then(|| v.to_string()))
        };
        assert_eq!(of("barrier", "bollard"), Some(ObstacleKind::Bollard));
        assert_eq!(of("barrier", "gate"), Some(ObstacleKind::Gate));
        assert_eq!(of("barrier", "lift_gate"), Some(ObstacleKind::LiftGate));
        assert_eq!(of("barrier", "block"), Some(ObstacleKind::Block));
        assert_eq!(
            of("barrier", "cycle_barrier"),
            Some(ObstacleKind::CycleBarrier)
        );
        assert_eq!(of("barrier", "chain"), Some(ObstacleKind::Barrier));
        assert_eq!(of("barrier", "entrance"), None);
        assert_eq!(of("power", "pole"), Some(ObstacleKind::Post));
        assert_eq!(of("power", "tower"), Some(ObstacleKind::Pylon));
        assert_eq!(of("man_made", "utility_pole"), Some(ObstacleKind::Post));
        assert_eq!(of("highway", "crossing"), None);
        assert_eq!(ObstacleKind::of_line("fence"), Some(ObstacleKind::Fence));
        assert_eq!(
            ObstacleKind::of_line("retaining_wall"),
            Some(ObstacleKind::Wall)
        );
        assert_eq!(ObstacleKind::of_line("hedge"), Some(ObstacleKind::Hedge));
        assert_eq!(
            ObstacleKind::of_line("guard_rail"),
            Some(ObstacleKind::GuardRail)
        );
        assert_eq!(ObstacleKind::of_line("kerb"), Some(ObstacleKind::Kerb));
        assert!(!ObstacleKind::Kerb.solid() && ObstacleKind::Fence.solid());
        for k in ObstacleKind::ALL {
            assert_eq!(ObstacleKind::from_code(k.code()), Some(k));
        }
    }

    #[test]
    fn the_index_finds_each_solid_once_and_road_barriers_are_not_solid() {
        let mut pkg = empty_package();
        pkg.obstacles = Some(sample());
        let w = WorldObstacles::from_package(&pkg).unwrap();
        let idx = w.tile_index(TileKey { x: 0, z: 0 }).unwrap();
        // The post, two fence segments and the tree; not the road bollard.
        assert_eq!(idx.len(), 4, "{:?}", idx.solids());
        assert!(idx.solids().iter().all(|s| s.kind != ObstacleKind::Bollard));
        let mut out = Vec::new();
        idx.query([45.0, -5.0], [55.0, 60.0], &mut out);
        let fences = out.iter().filter(|s| s.kind == ObstacleKind::Fence).count();
        assert_eq!(fences, 2, "{out:?}");
        out.clear();
        idx.query([28.0, 38.0], [32.0, 42.0], &mut out);
        assert!(out.iter().any(|s| s.kind == ObstacleKind::Post));
        out.clear();
        idx.query([500.0, 500.0], [510.0, 510.0], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn railings_and_tunnel_walls_come_from_their_sections() {
        use crate::bridges::{Bridge, BridgeMaterial, BridgeSource, BridgeStructure};
        let mut pkg = empty_package();
        let bridge = Bridge {
            edge: crate::graph::EdgeId(1),
            osm_way: 1,
            structure: BridgeStructure::Beam,
            material: BridgeMaterial::Concrete,
            source: BridgeSource::Tagged,
            railing: Railing::Parapet,
            width_m: 5.0,
            deck_m: 1.0,
            points: vec![[100.0, 50.0, 200.0], [160.0, 50.0, 200.0]],
            abutment_foot_y: [40.0, 40.0],
            piers: Vec::new(),
            arches: Vec::new(),
        };
        pkg.bridges = Some(BTreeMap::from([(TileKey { x: 0, z: 0 }, vec![bridge])]));
        let w = WorldObstacles::from_package(&pkg).unwrap();
        let idx = w.tile_index(TileKey { x: 0, z: 0 }).unwrap();
        let rails: Vec<&Solid> = idx
            .solids()
            .iter()
            .filter(|s| s.kind == ObstacleKind::Railing)
            .collect();
        assert_eq!(rails.len(), 2);
        for r in &rails {
            let Shape::Segment { a, .. } = r.shape else {
                panic!()
            };
            // 2.5 m + 0.6 m kerb − half the parapet beside the centre line.
            assert!(((a[1] - 200.0).abs() - (3.1 - 0.175)).abs() < 1e-3, "{a:?}");
            assert!(r.reaches(50.0) && !r.reaches(40.0));
        }
    }
}

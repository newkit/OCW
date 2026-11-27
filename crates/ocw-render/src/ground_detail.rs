//! Near-field ground detail (spec §4.6.2): the small things on the verges,
//! the shoulders and the ground beside the roads, where the rider looks –
//! road-edge gravel, pebbles, a few larger rocks, dirt and dust patches,
//! grass tufts, dry stalks, small shrubs, ferns, fallen leaves, needle
//! litter, cones, twigs and moss.
//!
//! **Placed by rule, not stored:** nothing of it is in the world package.
//! Every road strip of a tile is walked in steps of [`STEP_M`] along its
//! edges (the steps fixed in the road's own arc length, `dress.start_m`,
//! so they run on across the tiles); at each step and side the place –
//! the region, the land class there, the forest's leaf type (section
//! `FRST`), the altitude, the season's grass, leaf fall and snow – gives
//! the expected count of each [`Kind`] ([`mix`]), and a hash of the
//! step's world position draws the bits, their offset from the edge, size,
//! turn and shade. The same package gives the same bits on every client,
//! in every screenshot.
//!
//! **On the drawn ground:** every bit takes its height from the ground as
//! the tile draws it ([`TileGround`]: the terrain mesh after the junctions
//! lowered it) or from the road's shoulder where that lies higher, sinks a
//! little into it, and is left out on the road surface (but for a little
//! gravel within [`EDGE_IN_M`] of the edge), on bridge decks, in building
//! footprints, in water, and where the ground lies far off the road (cuts,
//! embankments, tunnels).
//!
//! **Cheap:** few low-poly models made in code (own work, no third-party
//! assets), merged per chunk of [`CHUNKS`]² a tile into one mesh in the
//! trees' material (it sways and takes the snow) plus one blended mesh for
//! the soft-edged patches; a chunk's meshes are built on the task pool
//! only when a camera comes near ([`lazy_ground`]) and dropped again
//! behind it, drawn up to the quality preset's distance and faded out
//! (dithered) before it, with the preset's share of the bits
//! ([`crate::quality::QualityPreset::ground_detail`]) or the setting's
//! (`display.ground_detail` auto / off / low / full, [`level`]). Lying snow
//! (the weather's cover from half on) hides all but the rocks and shrubs.
//!
//! **The answers of spec 4.6.2 (2026-11-19):** the bits of the first 7 m
//! band are spread over [`BAND_M`] (even to [`NEAR_BAND_M`], thinning out
//! beyond: the same count); very few lie on the road itself
//! ([`road_mix`]: grit, small stones, the odd twig, leaves in autumn),
//! never in the wheel tracks or a lane's middle ([`road_spot_free`]);
//! autumn adds leaf patches under the broadleaves and a few more leaves;
//! a wet road darkens the bits and makes them shine ([`wet_look`]).

use std::sync::{Arc, Weak};

use bevy::camera::visibility::VisibilityRange;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task, futures::check_ready};
use clap::{Arg, Command};
use ocw_sky::season::{LeafType, SeasonDay};
use ocw_world::flora::FloraRegion;
use ocw_world::landuse::LandClass;
use ocw_world::trees::hash;
use ocw_world::{RoadSurface, TileKey, WorldPackage};

use crate::camera::SolidFootprints;
use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::package::{Keep, MeshArrays, TileGround};

/// Chunks per tile side (25 m chunks for 1 km tiles).
pub const CHUNKS: u32 = 40;
/// The bits lie up to this far beyond the road's edge (m; spec 4.6.2 Q4:
/// the same number of bits as in the first 7 m band, spread wider) …
pub const BAND_M: f32 = 14.0;
/// … evenly up to here (the first band of v4.72), thinning out linearly
/// beyond ([`spread`]).
pub const NEAR_BAND_M: f32 = 7.0;
/// Steps along the road edges (m).
pub const STEP_M: f32 = 1.0;
/// Road-edge gravel reaches this far onto the road surface (m).
pub const EDGE_IN_M: f32 = 0.25;
/// Most bits of one kind at one step and side.
const MAX_PER_STEP: u32 = 12;
/// A chunk's meshes are built when a camera comes within the draw
/// distance and this margin of its centre (m) …
pub const BUILD_MARGIN_M: f32 = 30.0;
/// … and dropped beyond the draw distance and this one.
pub const DROP_MARGIN_M: f32 = 70.0;
/// [`lazy_ground`] looks at every chunk only every this many frames while
/// nothing is being built.
pub const SCAN_EVERY: u32 = 4;
/// Chunk meshes being built at a time.
pub const MAX_TASKS: usize = 6;
/// Points of a patch's rim.
pub const RING: usize = 8;
/// The inner ring of a patch lies this share of the way to its rim.
pub const INNER: f32 = 0.55;
/// A patch lies this far over the ground (m).
pub const PATCH_LIFT_M: f32 = 0.04;
/// The weather's snow cover from which only rocks and shrubs show.
pub const SNOW_HIDES_SMALL: f32 = 0.5;
/// The VR preset of Phase 11 (share, distance m; spec 4.6.2 Q2, decided
/// 2026-11-19: "modest 50 % up to 50 m"), between Medium and Low.
pub const VR_LEVEL: (f32, f32) = (0.5, 50.0);
/// `display.ground_detail low` (share, distance m): the Low preset's.
pub const LOW_LEVEL: (f32, f32) = (0.25, 35.0);
/// `display.ground_detail full`: the High preset's.
pub const FULL_LEVEL: (f32, f32) = (1.0, 80.0);
/// Bits on the road surface keep this far (m) from a lane's middle …
pub const LANE_MIDDLE_CLEAR_M: f32 = 0.45;
/// … and lie only where the wheel tracks
/// ([`ocw_world::road_wear::wheel_tracks`]) are below this.
pub const TRACK_CLEAR: f32 = 0.35;
/// A wet ground detail is this much darker at full wetness (the road's
/// asphalt 0.55, unpaved 0.72: stone, soil and plants in between) …
pub const WET_DARKEN: f32 = 0.38;
/// … and this rough (dry: 1): it shines like the wet road.
pub const WET_ROUGHNESS: f32 = 0.6;
/// The broadleaf share the autumn leaves count with at least in a wood
/// (mixed woods measured as mostly conifers still have beeches and maples
/// by the roads; spec 4.6.2 Q5) and the region's share where the package
/// has no forest type – shared with the physics of wet leaves
/// (`ocw_world::leaves`, spec 4.6.2 Q6).
pub use ocw_world::leaves::{LEAF_WOOD_MIN, region_broadleaf};

/// What lies by the road.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    /// Road-edge gravel and grit, at the edge and a little onto the road.
    Gravel,
    Pebble,
    Rock,
    /// A dirt or dust patch (soft-edged, blended).
    Dirt,
    /// A moss patch (blended).
    Moss,
    /// A patch of needle litter (blended).
    Needles,
    /// A fallen leaf.
    Leaf,
    Twig,
    Cone,
    Tuft,
    /// Dry stalks.
    Stalks,
    Fern,
    Shrub,
    /// A patch of fallen leaves under broadleaves (blended; spec 4.6.2
    /// Q5), autumn and winter. Last, so the other kinds keep their seeds.
    LeafLitter,
}

impl Kind {
    pub const ALL: [Kind; 14] = [
        Kind::Gravel,
        Kind::Pebble,
        Kind::Rock,
        Kind::Dirt,
        Kind::Moss,
        Kind::Needles,
        Kind::Leaf,
        Kind::Twig,
        Kind::Cone,
        Kind::Tuft,
        Kind::Stalks,
        Kind::Fern,
        Kind::Shrub,
        Kind::LeafLitter,
    ];

    /// What may lie on the road surface itself (spec 4.6.2 Q4, Q5): grit,
    /// small stones, the odd twig, leaves in autumn.
    pub const ON_ROAD: [Kind; 4] = [Kind::Gravel, Kind::Pebble, Kind::Twig, Kind::Leaf];

    pub fn name(self) -> &'static str {
        match self {
            Kind::Gravel => "gravel",
            Kind::Pebble => "pebble",
            Kind::Rock => "rock",
            Kind::Dirt => "dirt",
            Kind::Moss => "moss",
            Kind::Needles => "needles",
            Kind::Leaf => "leaf",
            Kind::Twig => "twig",
            Kind::Cone => "cone",
            Kind::Tuft => "tuft",
            Kind::Stalks => "stalks",
            Kind::Fern => "fern",
            Kind::Shrub => "shrub",
            Kind::LeafLitter => "leaf litter",
        }
    }

    /// Covered by lying snow (all but rocks and shrubs).
    pub fn small(self) -> bool {
        !matches!(self, Kind::Rock | Kind::Shrub)
    }

    /// A soft-edged patch in the blended mesh.
    pub fn patch(self) -> bool {
        matches!(
            self,
            Kind::Dirt | Kind::Moss | Kind::Needles | Kind::LeafLitter
        )
    }

    /// Where across the verge (m from the road edge): from, evenly to,
    /// thinning out to, and the share's skew (above 1 most lie near the
    /// start; [`spread`]). The ends of the even part are the first band's
    /// (v4.72, up to 7 m); the thinning tail doubles it (spec 4.6.2 Q4).
    pub fn band(self) -> (f32, f32, f32, f32) {
        match self {
            Kind::Gravel => (-EDGE_IN_M, 0.5, 0.5, 1.6),
            Kind::Pebble => (0.0, 3.5, 6.0, 1.8),
            Kind::Rock => (1.2, NEAR_BAND_M, BAND_M, 1.0),
            Kind::Dirt => (0.6, 5.0, 10.0, 1.3),
            Kind::Twig | Kind::Cone | Kind::Leaf => (0.1, 5.5, 11.0, 1.2),
            Kind::Needles | Kind::Moss | Kind::LeafLitter => (0.8, NEAR_BAND_M, BAND_M, 1.0),
            Kind::Tuft => (0.3, NEAR_BAND_M, BAND_M, 0.9),
            Kind::Stalks => (0.3, 5.0, 10.0, 1.0),
            Kind::Fern => (1.0, NEAR_BAND_M, BAND_M, 0.8),
            Kind::Shrub => (1.2, NEAR_BAND_M, BAND_M, 0.8),
        }
    }

    /// Size range (m: the radius of stones and patches, the height of
    /// plants, the length of twigs and leaves).
    fn size(self) -> (f32, f32) {
        match self {
            Kind::Gravel => (0.012, 0.03),
            Kind::Pebble => (0.025, 0.08),
            Kind::Rock => (0.18, 0.7),
            Kind::Dirt => (0.4, 1.3),
            Kind::Moss => (0.25, 0.7),
            Kind::Needles => (0.5, 1.4),
            // 8 – 15 cm since spec 4.6.2 Q5 (6 – 12 cm before).
            Kind::Leaf => (0.08, 0.15),
            Kind::Twig => (0.18, 0.6),
            Kind::Cone => (0.05, 0.1),
            Kind::Tuft => (0.15, 0.4),
            Kind::Stalks => (0.3, 0.7),
            Kind::Fern => (0.3, 0.65),
            Kind::Shrub => (0.25, 0.6),
            Kind::LeafLitter => (0.45, 1.2),
        }
    }
}

/// The distance from the road edge of a bit drawn with `u` (0 … 1) for a
/// band from `b0`, even to `f` and thinning out linearly to `b1` (the
/// inverse of the trapezoid's distribution; `f >= b1`: even to `b1`).
pub fn spread(b0: f32, f: f32, b1: f32, u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    let flat = (f - b0).max(0.0);
    let tail = (b1 - f).max(0.0);
    let total = flat + tail / 2.0;
    if total <= 0.0 {
        return b0;
    }
    let t = u * total;
    if t <= flat || tail <= 0.0 {
        return b0 + t.min(flat);
    }
    let r = ((t - flat) / (tail / 2.0)).clamp(0.0, 1.0);
    f + tail * (1.0 - (1.0 - r).sqrt())
}

/// One bit of ground detail (package metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bit {
    pub kind: Kind,
    /// Where it touches the ground (rocks: the lowest ground under them).
    pub pos: Vec3,
    pub size: f32,
    pub yaw: f32,
    /// The ground's normal there.
    pub normal: Vec3,
    /// Linear RGB.
    pub colour: [f32; 3],
    /// On the road surface (spec 4.6.2 Q4; not the edge gravel).
    pub road: bool,
    /// Patches: the rim's points (radius share, height over `pos`, and the
    /// height over `pos` of the inner ring at [`INNER`] of the way).
    pub ring: [[f32; 3]; RING],
}

/// The season's say on the ground detail at one altitude.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeasonFx {
    /// Grass green (0 dormant – 1 lush) and dried (summer straw).
    pub green: f32,
    pub dry: f32,
    /// How brown the high pastures are (`ocw_sky::season::Grass::high_brown`).
    pub high_brown: f32,
    /// Leaves lying on the ground under broadleaves (0 – 1).
    pub litter: f32,
    /// Freshly fallen, still coloured (autumn).
    pub autumn: f32,
    /// The leaf fall's own share of [`Self::litter`] (0 in summer, 1 when
    /// the leaves are down): the leaf patches and the leaves on the road.
    pub fall: f32,
    /// Climatological snow cover (0 – 1).
    pub snow: f32,
}

impl SeasonFx {
    /// How brown the grass is ([`crate::season::brown_grass`]).
    pub fn brown(&self) -> f32 {
        crate::season::brown_grass(self.green, self.dry, self.high_brown)
    }

    /// Without a season: summer.
    pub const SUMMER: SeasonFx = SeasonFx {
        green: 1.0,
        dry: 0.0,
        high_brown: 0.0,
        litter: 0.2,
        autumn: 0.0,
        fall: 0.0,
        snow: 0.0,
    };

    pub fn of(day: Option<&SeasonDay>, alt_m: f64) -> Self {
        let Some(d) = day else {
            return Self::SUMMER;
        };
        let g = d.grass(alt_m);
        let f = d.foliage(LeafType::Deciduous, alt_m, 0.0);
        // Leaves come down with the autumn colour and lie through the
        // winter; the fresh green of spring covers the last of them.
        // Where the broadleaves are evergreen (Tenerife) only the few
        // deciduous trees' share of it changes (spec §14 Phase 9 Q13).
        let k = d.leaf_fall_share();
        let fallen = (1.0 - f.leaves).clamp(0.0, 1.0);
        let litter = (0.15 + k * (0.5 * f.autumn + fallen)) * (1.0 - 0.6 * k * f.fresh);
        Self {
            green: g.green,
            dry: g.dry,
            high_brown: g.high_brown,
            litter: litter.clamp(0.0, 1.0),
            autumn: (k * f.autumn * (0.4 + fallen)).clamp(0.0, 1.0),
            // The same leaf fall as the physics of wet leaves (4.6.2 Q6).
            fall: d.leaf_fall(alt_m),
            snow: d.snow(alt_m),
        }
    }
}

/// What a place is, for the ground detail.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Place {
    pub region: FloraRegion,
    pub class: LandClass,
    pub alt_m: f64,
    /// Share of broadleaves among the trees (the measured forest type, or
    /// the region's).
    pub broadleaf: f32,
    pub surface: RoadSurface,
    /// A sidewalk on this side.
    pub sidewalk: bool,
}

/// Bits of each kind expected per metre of one road side (High).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Mix(pub [f32; 14]);

impl Mix {
    pub fn get(&self, k: Kind) -> f32 {
        self.0[k as usize]
    }

    fn set(&mut self, k: Kind, v: f32) {
        self.0[k as usize] = v.max(0.0);
    }

    fn scale(&mut self, k: Kind, f: f32) {
        self.0[k as usize] *= f.max(0.0);
    }

    pub fn total(&self) -> f32 {
        self.0.iter().sum()
    }
}

/// The ground detail expected at a place in a season (bits per metre of
/// road side, before the preset's share).
pub fn mix(p: &Place, s: &SeasonFx) -> Mix {
    use Kind::*;
    let mut m = Mix::default();
    let forest = matches!(p.class, LandClass::Forest);
    let open = matches!(
        p.class,
        LandClass::Default
            | LandClass::Meadow
            | LandClass::Park
            | LandClass::Farmland
            | LandClass::Orchard
            | LandClass::Vineyard
    );
    let urban = matches!(
        p.class,
        LandClass::Residential | LandClass::Commercial | LandClass::Industrial
    );
    let stony = matches!(
        p.class,
        LandClass::Rock | LandClass::Scree | LandClass::Bare
    );
    if matches!(
        p.class,
        LandClass::Water | LandClass::Wetland | LandClass::Glacier
    ) {
        return m;
    }
    let conifer = 1.0 - p.broadleaf.clamp(0.0, 1.0);
    // The common ground of every region.
    m.set(Gravel, 2.0);
    m.set(Pebble, 1.4);
    m.set(Rock, 0.03);
    m.set(Dirt, 0.12);
    m.set(Twig, 0.08);
    m.set(Tuft, 1.4);
    m.set(Stalks, 0.25);
    m.set(Shrub, 0.03);
    if forest {
        m.set(Tuft, 0.5);
        m.set(Twig, 0.45);
        m.set(Rock, 0.06);
        // A bit stronger since spec 4.6.2 Q5 (6 → 7.5), and patches of
        // fallen leaves under the broadleaves.
        m.set(Leaf, 7.5 * p.broadleaf * s.litter);
        m.set(LeafLitter, 0.5 * p.broadleaf.max(LEAF_WOOD_MIN) * s.fall);
        m.set(Needles, 0.45 * conifer);
        m.set(Cone, 0.35 * conifer);
        m.set(Fern, 0.25);
        m.set(Moss, 0.2);
        m.set(Dirt, 0.2);
    } else if open {
        // Leaves blown out of the hedges and roadside trees.
        m.set(Leaf, 0.65 * s.litter);
        m.set(LeafLitter, 0.02 * s.fall);
    }
    match p.region {
        FloraRegion::Tenerife => {
            // Volcanic: lava grit and scoria, more rocks, dust, sparse and
            // dry plants; no ferns or moss by the roads, no cones but the
            // Canary pine's.
            m.scale(Pebble, 1.8);
            m.scale(Rock, 3.0);
            m.scale(Dirt, 2.5);
            m.scale(Tuft, 0.3);
            m.scale(Stalks, 2.2);
            m.scale(Shrub, 3.5);
            m.set(Fern, 0.0);
            m.set(Moss, 0.0);
            if p.alt_m > 1800.0 {
                // Las Cañadas: pumice, lava blocks, broom.
                m.scale(Rock, 2.0);
                m.scale(Tuft, 0.3);
            }
        }
        FloraRegion::BlackForest => {
            // Wet granite country: ferns, moss and needles in the woods.
            m.scale(Fern, 2.0);
            m.scale(Moss, 2.0);
            m.scale(Needles, 1.3);
            m.scale(Tuft, 1.1);
        }
        FloraRegion::AlpineRhine => {
            // Limestone gravel, meadow tufts, alpine rocks higher up.
            m.scale(Gravel, 1.3);
            m.scale(Tuft, if open { 1.7 } else { 1.0 });
            m.scale(Fern, 0.8);
            if p.alt_m > 1200.0 {
                m.scale(Rock, 3.0);
                m.scale(Pebble, 1.4);
                m.scale(Shrub, 2.5);
            }
        }
        FloraRegion::Pyrenees => {
            // Granite and schist grit, dry pine needles and cones in the
            // pine woods, rocks and dwarf shrubs (rhododendron, juniper)
            // higher up.
            m.scale(Needles, 1.4);
            m.scale(Cone, 1.4);
            m.scale(Pebble, 1.3);
            m.scale(Fern, 0.6);
            if p.alt_m > 1600.0 {
                m.scale(Rock, 2.5);
                m.scale(Shrub, 2.0);
            }
        }
        FloraRegion::Other => {}
    }
    if urban {
        for k in [
            Tuft, Stalks, Shrub, Fern, Moss, Needles, Cone, Leaf, Twig, LeafLitter,
        ] {
            m.scale(k, 0.35);
        }
        m.scale(Pebble, 0.6);
        m.scale(Dirt, 0.5);
        m.scale(Rock, 0.15);
    }
    if stony {
        m.scale(Rock, 4.0);
        m.scale(Pebble, 2.5);
        m.scale(Tuft, 0.25);
        m.scale(Dirt, 1.5);
    }
    if matches!(p.class, LandClass::Farmland) {
        m.scale(Dirt, 2.0);
        m.scale(Tuft, 0.7);
    }
    match p.surface {
        RoadSurface::Gravel | RoadSurface::Dirt => m.scale(Gravel, 2.2),
        RoadSurface::Paving => m.scale(Gravel, 0.3),
        _ => {}
    }
    if p.sidewalk {
        m.set(Gravel, 0.0);
        for k in Kind::ALL {
            m.scale(k, 0.4);
        }
    }
    // Dry seasons: more straw, fewer green tufts; dormant grass lies low.
    m.scale(Stalks, 1.0 + 1.5 * s.dry + 0.8 * (1.0 - s.green));
    m.scale(Tuft, 1.0 - 0.4 * s.dry);
    // The climatological snow covers what is small.
    let cover = (1.0 - s.snow.clamp(0.0, 1.0)).powi(2);
    for k in Kind::ALL {
        if k.small() {
            m.scale(k, cover);
        }
    }
    m
}

/// What lies on the road surface itself (spec 4.6.2 Q4, Q5), bits per
/// metre of one side's half of the carriageway (High): very few – grit and
/// small stones (more on worn and unpaved roads, like the road damage's
/// loose stones of [`ocw_world::road_wear`]), the odd twig, leaves in
/// autumn and winter. `grade`: the road's wear grade (1 … 7); `leaves`:
/// the leaves per metre there ([`road_leaves`]: the same amount the
/// physics of wet leaves rides over, spec 4.6.2 Q6).
pub fn road_mix(p: &Place, s: &SeasonFx, grade: u8, leaves: f32) -> Mix {
    use Kind::*;
    let mut m = Mix::default();
    let forest = matches!(p.class, LandClass::Forest);
    let urban = matches!(
        p.class,
        LandClass::Residential | LandClass::Commercial | LandClass::Industrial
    );
    if matches!(
        p.class,
        LandClass::Water | LandClass::Wetland | LandClass::Glacier
    ) {
        return m;
    }
    let worn = 0.6 + 0.25 * f32::from(grade.clamp(1, 7) - 1);
    m.set(Gravel, 0.035 * worn);
    m.set(Pebble, 0.012 * worn);
    m.set(Twig, if forest { 0.012 } else { 0.003 });
    match p.surface {
        RoadSurface::Gravel | RoadSurface::Dirt => {
            m.scale(Gravel, 2.0);
            m.scale(Pebble, 2.0);
        }
        RoadSurface::Paving => m.scale(Gravel, 0.5),
        _ => {}
    }
    if urban || p.sidewalk {
        // Swept streets.
        for k in Kind::ON_ROAD {
            m.scale(k, 0.5);
        }
    }
    let cover = (1.0 - s.snow.clamp(0.0, 1.0)).powi(2);
    for k in Kind::ON_ROAD {
        m.scale(k, cover);
    }
    // Already for the town, the season and the snow.
    m.set(Leaf, leaves);
    m
}

/// The leaves per metre on one side's half of the road at (x, z) – the
/// verge's point the place is taken at – in the season `s` at `alt_m`:
/// [`ocw_world::leaves`]'s cell code read straight from the package and
/// its amount, the same function the server's physics of wet leaves uses
/// (spec 4.6.2 Q6).
pub fn road_leaves(pkg: &WorldPackage, x: f64, z: f64, alt_m: f64, s: &SeasonFx) -> f32 {
    ocw_world::leaves::per_metre(
        ocw_world::leaves::code_at(pkg, x, z),
        FloraRegion::at(pkg.origin.0, pkg.origin.1),
        alt_m,
        s.fall,
        s.snow,
    )
}

/// Whether a bit may lie `u` m across a carriageway of half width `half`
/// (lanes as the road's dress): off the wheel tracks and off the lanes'
/// middles, where the wheels run (spec 4.6.2 Q4; the road damage sweeps
/// its loose stones out of the tracks the same way).
pub fn road_spot_free(u: f32, half: f32, lanes: [u8; 2]) -> bool {
    use ocw_world::road_wear::{lane_middles, wheel_tracks};
    wheel_tracks(u, half, lanes) < TRACK_CLEAR
        && lane_middles(half, lanes)
            .iter()
            .all(|c| (u - c).abs() > LANE_MIDDLE_CLEAR_M)
}

/// The look of the ground detail at road wetness `wet` (0 … 1; spec 4.6.2
/// Q3): the colour factor and the roughness – darker and shining like the
/// wet road (Phase 9), no shadows of its own.
pub fn wet_look(wet: f32) -> (f32, f32) {
    let w = wet.clamp(0.0, 1.0);
    (1.0 - WET_DARKEN * w, 1.0 + (WET_ROUGHNESS - 1.0) * w)
}

fn lin3(c: [f32; 3]) -> [f32; 3] {
    let l = Color::srgb(c[0], c[1], c[2]).to_linear();
    [l.red, l.green, l.blue]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

/// The stone colours of a region (sRGB): the main rock, a second one,
/// the second's share.
pub fn stone_srgb(region: FloraRegion, alt_m: f64) -> ([f32; 3], [f32; 3], f32) {
    match region {
        // Basalt and red scoria; pale pumice in Las Cañadas.
        FloraRegion::Tenerife if alt_m > 1800.0 => ([0.20, 0.19, 0.18], [0.66, 0.60, 0.48], 0.35),
        FloraRegion::Tenerife => ([0.19, 0.18, 0.17], [0.38, 0.22, 0.16], 0.3),
        // Granite and gneiss.
        FloraRegion::BlackForest => ([0.53, 0.50, 0.47], [0.40, 0.38, 0.37], 0.4),
        // Limestone (grey and a warmer grey).
        FloraRegion::AlpineRhine => ([0.68, 0.66, 0.62], [0.56, 0.53, 0.49], 0.35),
        // Granite (light grey) and dark slaty schist.
        FloraRegion::Pyrenees => ([0.58, 0.56, 0.53], [0.34, 0.33, 0.32], 0.4),
        FloraRegion::Other => ([0.52, 0.50, 0.47], [0.44, 0.41, 0.38], 0.3),
    }
}

/// The soil colour of a region (sRGB): Tenerife's dust and ash, the Black
/// Forest's dark wet earth, the Alpine Rhine's brown loam.
pub fn soil_srgb(region: FloraRegion, h: f32) -> [f32; 3] {
    match region {
        FloraRegion::Tenerife => mix3([0.56, 0.46, 0.35], [0.30, 0.28, 0.27], h),
        FloraRegion::BlackForest => mix3([0.25, 0.19, 0.14], [0.20, 0.15, 0.11], h),
        FloraRegion::AlpineRhine => mix3([0.40, 0.32, 0.23], [0.48, 0.42, 0.33], h),
        // Brown mountain soil over granite and schist.
        FloraRegion::Pyrenees => mix3([0.38, 0.30, 0.22], [0.30, 0.25, 0.19], h),
        FloraRegion::Other => mix3([0.40, 0.32, 0.24], [0.33, 0.27, 0.20], h),
    }
}

/// How alpine the grass at a place is for its russet
/// ([`crate::season::russet_share`] by the region's tree line).
fn russet(p: &Place) -> f32 {
    crate::season::russet_share(p.alt_m, p.region.tree_line_m())
}

/// The colour of a bit (linear): `h` its own shade hash (0 … 1).
pub fn colour(k: Kind, p: &Place, s: &SeasonFx, h: f32) -> [f32; 3] {
    let shade = 0.85 + 0.3 * h;
    let c = match k {
        Kind::Gravel | Kind::Pebble | Kind::Rock => {
            let (a, b, share) = stone_srgb(p.region, p.alt_m);
            let c = if h < share { b } else { a };
            // Gravel at the edge is a little lighter (dust).
            let lift = if k == Kind::Gravel { 1.08 } else { 1.0 };
            c.map(|v| v * lift)
        }
        Kind::Dirt => soil_srgb(p.region, h),
        Kind::Moss => mix3([0.25, 0.37, 0.12], [0.36, 0.38, 0.18], 1.0 - s.green),
        Kind::Needles => match p.region {
            // The Canary pine's long needles, paler.
            FloraRegion::Tenerife => [0.52, 0.38, 0.24],
            _ => [0.40, 0.25, 0.14],
        },
        Kind::Leaf | Kind::LeafLitter => {
            let fresh = [[0.80, 0.42, 0.10], [0.86, 0.68, 0.16], [0.62, 0.22, 0.08]];
            let old = [[0.42, 0.30, 0.18], [0.36, 0.27, 0.17], [0.48, 0.36, 0.22]];
            let i = ((h * 3.0) as usize).min(2);
            let c = mix3(old[i], fresh[i], s.autumn);
            if k == Kind::LeafLitter {
                // The patch: the leaves with the soil between them.
                mix3(c, soil_srgb(p.region, h), 0.3)
            } else {
                c
            }
        }
        Kind::Twig => mix3([0.36, 0.28, 0.21], [0.46, 0.43, 0.39], h),
        Kind::Cone => [0.44, 0.29, 0.17],
        Kind::Tuft => {
            let base = match p.region {
                FloraRegion::Tenerife => lin3([0.42, 0.45, 0.24]),
                _ => lin3([0.30, 0.46, 0.17]),
            };
            let c = crate::season::grass_colour(base, s.green, s.dry);
            // Above the tree line dormant and dry tufts are russet, like
            // the turf around them (spec 4.6.3, v6.72).
            let c = crate::season::alpine_grass(c, s.brown(), russet(p), h);
            return c.map(|v| v * shade);
        }
        Kind::Stalks => {
            let straw = lin3(mix3([0.74, 0.64, 0.42], [0.52, 0.42, 0.29], h));
            let c = crate::season::alpine_grass(straw, 1.0, russet(p), h);
            return c.map(|v| v * shade);
        }
        Kind::Fern => mix3([0.24, 0.44, 0.15], [0.56, 0.36, 0.16], 1.0 - s.green),
        Kind::Shrub => match p.region {
            FloraRegion::Tenerife => [0.44, 0.48, 0.34],
            _ => mix3([0.20, 0.33, 0.14], [0.40, 0.33, 0.20], 1.0 - s.green),
        },
    };
    lin3(c).map(|v| v * shade)
}

/// What the placement needs of a tile.
pub struct Ctx<'a> {
    pub pkg: &'a WorldPackage,
    pub key: TileKey,
    /// The ground as the tile draws it.
    pub ground: &'a TileGround<'a>,
    pub solid: &'a SolidFootprints,
    pub season: Option<&'a SeasonDay>,
    /// Whether a point (package x, z) belongs to this member of the world
    /// set.
    pub keep: &'a dyn Fn(f64, f64) -> bool,
}

/// How much is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Params {
    /// The preset's share of the bits (0 – 1).
    pub density: f32,
    /// Lying snow: only rocks and shrubs.
    pub snow: bool,
}

impl Params {
    pub const FULL: Params = Params {
        density: 1.0,
        snow: false,
    };
}

/// A road segment for the on-road test.
struct Seg {
    a: [f64; 3],
    b: [f64; 3],
    half: f64,
}

/// The highest road surface centre line over (x, z) (`road_heights` over
/// the segments near the chunk).
fn road_y(segs: &[Seg], x: f64, z: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for s in segs {
        let (dx, dz) = (s.b[0] - s.a[0], s.b[2] - s.a[2]);
        let len2 = dx * dx + dz * dz;
        if len2 < 1e-12 {
            continue;
        }
        let t = ((x - s.a[0]) * dx + (z - s.a[2]) * dz) / len2;
        // A little beyond the segment's ends too: the outer side of a bend.
        let (cx, cz, t) = if (0.0..=1.0).contains(&t) {
            (s.a[0] + dx * t, s.a[2] + dz * t, t)
        } else {
            let t = t.clamp(0.0, 1.0);
            (s.a[0] + dx * t, s.a[2] + dz * t, t)
        };
        if (x - cx).powi(2) + (z - cz).powi(2) <= s.half * s.half {
            let y = s.a[1] + (s.b[1] - s.a[1]) * t;
            best = Some(best.map_or(y, |b| b.max(y)));
        }
    }
    best
}

fn h01(x: i64, z: i64, seed: u64) -> f32 {
    hash(x, z, seed)
}

/// The bits whose position lies in `rect` (package x0, z0, x1, z1) and in
/// tile `ctx.key`.
pub fn bits_in(ctx: &Ctx, rect: [f64; 4], p: &Params) -> Vec<Bit> {
    let pkg = ctx.pkg;
    let mut out = Vec::new();
    if p.density <= 0.0 {
        return out;
    }
    let Some(tile) = pkg.tiles.get(&ctx.key) else {
        return out;
    };
    let size = f64::from(pkg.tile_size_m);
    let o = ctx.key.origin(size);
    let rect = [
        rect[0].max(o[0]),
        rect[1].max(o[1]),
        rect[2].min(o[0] + size),
        rect[3].min(o[1] + size),
    ];
    if rect[0] >= rect[2] || rect[1] >= rect[3] {
        return out;
    }
    let inside = |x: f64, z: f64| x >= rect[0] && x < rect[2] && z >= rect[1] && z < rect[3];
    let overlaps = |lo: [f64; 2], hi: [f64; 2], m: f64| {
        lo[0] - m < rect[2] && hi[0] + m > rect[0] && lo[1] - m < rect[3] && hi[1] + m > rect[1]
    };
    let region = FloraRegion::at(pkg.origin.0, pkg.origin.1);
    let lift = crate::ROAD_SURFACE_LIFT_M;
    // The segments that can cover a point of the rect.
    let segs: Vec<Seg> = tile
        .roads
        .iter()
        .flat_map(|s| {
            let half = f64::from(s.width_m) / 2.0;
            s.points.windows(2).filter_map(move |w| {
                let (a, b) = (w[0].map(f64::from), w[1].map(f64::from));
                let lo = [a[0].min(b[0]), a[2].min(b[2])];
                let hi = [a[0].max(b[0]), a[2].max(b[2])];
                overlaps(lo, hi, half + 1.0).then_some(Seg { a, b, half })
            })
        })
        .collect();
    let decks = crate::bridges::bridges_near(pkg, ctx.key);
    let shoulder_ground = |x: f32, z: f32| {
        pkg.surface_height(f64::from(x), f64::from(z))
            .map(|h| h as f32)
    };
    let class_at = |x: f64, z: f64| {
        pkg.landuse
            .as_ref()
            .and_then(|lu| lu.class_at(size, x, z))
            .unwrap_or_default()
    };
    let broadleaf_at = |x: f64, z: f64, alt: f64| {
        pkg.look
            .forest
            .as_ref()
            .and_then(|f| f.cell(size, x, z))
            .and_then(|c| c.broadleaf)
            .map_or_else(|| region_broadleaf(region, alt), |b| f32::from(b) / 100.0)
    };
    for s in &tile.roads {
        if s.points.len() < 2 {
            continue;
        }
        let half = s.width_m / 2.0;
        let walk_max = s.dress.sidewalk_m[0].max(s.dress.sidewalk_m[1]);
        let grade = ocw_world::road_wear::wear_grade(s.class, s.surface, &s.condition);
        let reach = f64::from(half + walk_max + BAND_M + STEP_M + 1.0);
        let mut local = 0.0f32;
        for w in s.points.windows(2) {
            let (a, b) = (Vec3::from(w[0]), Vec3::from(w[1]));
            let len = (b - a).with_y(0.0).length();
            if len < 1e-3 {
                continue;
            }
            let lo = [f64::from(a.x.min(b.x)), f64::from(a.z.min(b.z))];
            let hi = [f64::from(a.x.max(b.x)), f64::from(a.z.max(b.z))];
            if !overlaps(lo, hi, reach) {
                local += len;
                continue;
            }
            let dir = (b - a).with_y(0.0) / len;
            let right = Vec3::new(-dir.z, 0.0, dir.x);
            let g0 = s.dress.start_m + local;
            let mut j = (g0 / STEP_M).ceil() as i64;
            loop {
                let along = j as f32 * STEP_M;
                if along >= g0 + len {
                    break;
                }
                j += 1;
                let t = (along - g0) / len;
                let q = a + (b - a) * t;
                let at_local = local + t * len;
                let narrow = s.dress.in_junction(at_local);
                // The step's seed: its world position (quarter metres).
                let (qx, qz) = ((q.x * 4.0).round() as i64, (q.z * 4.0).round() as i64);
                for (si, sign) in [-1.0f32, 1.0].into_iter().enumerate() {
                    let out_dir = right * sign;
                    let walk = s.dress.sidewalk_m[si];
                    let edge = q + out_dir * half;
                    let top = edge + Vec3::Y * lift;
                    if crate::bridges::on_deck(&decks, [edge.x, q.y, edge.z]) {
                        continue;
                    }
                    let sh = crate::road_mesh::shoulder(top, out_dir, &shoulder_ground, narrow);
                    let lip = top.y - crate::road_mesh::SHOULDER_STEP_M;
                    // The drawn surface `dist` m beyond the edge at (x, z):
                    // the ground, or the shoulder where it lies higher.
                    // (`v` m further along: the shoulder rises and falls
                    // with the road.)
                    let rise = (b.y - a.y) / len;
                    let surface = |x: f64, z: f64, dist: f32, v: f32| -> Option<f64> {
                        let g = ctx.ground.height(x, z)?;
                        if walk <= 0.0 && !sh.wall && (0.0..sh.width).contains(&dist) {
                            let y = lip + (sh.outer_y - lip) * dist / sh.width + rise * v;
                            Some(g.max(f64::from(y)))
                        } else {
                            Some(g)
                        }
                    };
                    let mid = edge + out_dir * (walk + 3.0);
                    let (mx, mz) = (f64::from(mid.x), f64::from(mid.z));
                    let alt = f64::from(q.y);
                    let place = Place {
                        region,
                        class: class_at(mx, mz),
                        alt_m: alt,
                        broadleaf: broadleaf_at(mx, mz, alt),
                        surface: s.surface,
                        sidewalk: walk > 0.0,
                    };
                    let fx = SeasonFx::of(ctx.season, alt);
                    let m = mix(&place, &fx);
                    for kind in Kind::ALL {
                        if p.snow && kind.small() {
                            continue;
                        }
                        if kind == Kind::Gravel && narrow {
                            continue;
                        }
                        let e = m.get(kind) * STEP_M * p.density;
                        if e <= 0.0 {
                            continue;
                        }
                        let seed = |slot: u32, field: u64| {
                            ((si as u64) << 60)
                                ^ ((kind as u64) << 52)
                                ^ (u64::from(slot) << 40)
                                ^ (field << 32)
                                ^ 0x6e65_6172 // "near"
                        };
                        let n = (e.floor() as u32
                            + u32::from(h01(qx, qz, seed(0, 15)) < e.fract()))
                        .min(MAX_PER_STEP);
                        let (b0, f, b1, skew) = kind.band();
                        let (s0, s1) = kind.size();
                        for slot in 0..n {
                            let u = h01(qx, qz, seed(slot, 1)).powf(skew);
                            let mut dist = spread(b0, f, b1, u);
                            if walk > 0.0 {
                                dist = dist.max(0.0) + walk + 0.3;
                            }
                            let v = h01(qx, qz, seed(slot, 2)) * STEP_M;
                            let at = edge + out_dir * dist + dir * v;
                            let (x, z) = (f64::from(at.x), f64::from(at.z));
                            if !inside(x, z) || !(ctx.keep)(x, z) {
                                continue;
                            }
                            let on_road = road_y(&segs, x, z);
                            let on_edge = kind == Kind::Gravel && dist < 0.0;
                            if on_road.is_some() && !on_edge {
                                continue;
                            }
                            if ctx.solid.top_at(x, z).is_some() {
                                continue;
                            }
                            if matches!(
                                class_at(x, z),
                                LandClass::Water | LandClass::Wetland | LandClass::Glacier
                            ) {
                                continue;
                            }
                            let size = s0 + (s1 - s0) * h01(qx, qz, seed(slot, 3)).powf(1.6);
                            let size = if kind == Kind::Rock && region == FloraRegion::Tenerife {
                                size * 1.3
                            } else {
                                size
                            };
                            // Patches lie beyond the shoulder (their rims
                            // would bend over its edge).
                            if kind.patch() && dist - 1.2 * size < sh.width {
                                continue;
                            }
                            let yaw = h01(qx, qz, seed(slot, 4)) * std::f32::consts::TAU;
                            let shade = h01(qx, qz, seed(slot, 5));
                            let (y, normal) = if on_edge {
                                let y = on_road.unwrap_or(f64::from(q.y)) + f64::from(lift);
                                (y, Vec3::Y)
                            } else {
                                let Some(y) = surface(x, z, dist, v) else {
                                    continue;
                                };
                                // Far off the road's height: a cut, an
                                // embankment's foot, a tunnel's roof.
                                if (y - f64::from(q.y)).abs() > f64::from(1.2 + 0.6 * dist.max(0.0))
                                {
                                    continue;
                                }
                                let d = 0.25;
                                let hx = |dx: f64, dz: f64| surface(x + dx, z + dz, dist, v);
                                let normal =
                                    match (hx(d, 0.0), hx(-d, 0.0), hx(0.0, d), hx(0.0, -d)) {
                                        (Some(e), Some(w), Some(n), Some(s)) => Vec3::new(
                                            ((w - e) / (2.0 * d)) as f32,
                                            1.0,
                                            ((s - n) / (2.0 * d)) as f32,
                                        )
                                        .normalize(),
                                        _ => Vec3::Y,
                                    };
                                (y, normal)
                            };
                            let mut ring = [[0.0f32; 3]; RING];
                            let mut y = y;
                            if kind.patch() {
                                let mut ok = true;
                                for (i, r) in ring.iter_mut().enumerate() {
                                    let share = 0.7 + 0.5 * h01(qx, qz, seed(slot, 6 + i as u64));
                                    let a = yaw + i as f32 / RING as f32 * std::f32::consts::TAU;
                                    let (rx, rz) = (
                                        x + f64::from(a.cos() * size * share),
                                        z + f64::from(a.sin() * size * share),
                                    );
                                    // The rim's distance from the edge.
                                    let rd = dist
                                        + (Vec3::new(a.cos(), 0.0, a.sin()) * size * share)
                                            .dot(out_dir);
                                    let (ix, iz) = (
                                        x + (rx - x) * f64::from(INNER),
                                        z + (rz - z) * f64::from(INNER),
                                    );
                                    let ri = dist + (rd - dist) * INNER;
                                    match (surface(rx, rz, rd, v), surface(ix, iz, ri, v)) {
                                        (Some(ry), Some(iy)) if road_y(&segs, rx, rz).is_none() => {
                                            *r = [share, (ry - y) as f32, (iy - y) as f32];
                                        }
                                        _ => ok = false,
                                    }
                                }
                                if !ok {
                                    continue;
                                }
                            } else if kind == Kind::Rock {
                                // The lowest ground under the rock.
                                for i in 0..8 {
                                    let a = i as f32 / 8.0 * std::f32::consts::TAU;
                                    let r = size * 0.8;
                                    let (rx, rz) =
                                        (x + f64::from(a.cos() * r), z + f64::from(a.sin() * r));
                                    if let Some(ry) = surface(rx, rz, dist, v) {
                                        y = y.min(ry);
                                    }
                                }
                            }
                            let pos = Vec3::new(at.x, y as f32, at.z);
                            out.push(Bit {
                                kind,
                                pos,
                                size,
                                yaw,
                                normal,
                                colour: colour(kind, &place, &fx, shade),
                                ring,
                                road: false,
                            });
                        }
                    }
                    // Very few bits on the road itself (spec 4.6.2 Q4,
                    // Q5): across this side's half of the carriageway,
                    // mostly near the edge, never in the wheel tracks or
                    // a lane's middle; not in junctions.
                    if narrow {
                        continue;
                    }
                    let leaves = road_leaves(pkg, mx, mz, alt, &fx);
                    let rm = road_mix(&place, &fx, grade, leaves);
                    for kind in Kind::ON_ROAD {
                        if p.snow && kind.small() {
                            continue;
                        }
                        let e = rm.get(kind) * STEP_M * p.density;
                        if e <= 0.0 {
                            continue;
                        }
                        let seed = |slot: u32, field: u64| {
                            ((si as u64) << 60)
                                ^ ((kind as u64) << 52)
                                ^ (u64::from(slot) << 40)
                                ^ (field << 32)
                                ^ 0x726f_6164 // "road"
                        };
                        let n = (e.floor() as u32
                            + u32::from(h01(qx, qz, seed(0, 15)) < e.fract()))
                        .min(MAX_PER_STEP);
                        for slot in 0..n {
                            // A few tries for a free spot.
                            let Some(inward) = (0..3u64)
                                .map(|t| {
                                    EDGE_IN_M
                                        + (half - EDGE_IN_M).max(0.0)
                                            * h01(qx, qz, seed(slot, 1 + 8 * t)).powf(2.0)
                                })
                                .find(|d| road_spot_free(half - d, half, s.dress.lanes))
                            else {
                                continue;
                            };
                            let v = h01(qx, qz, seed(slot, 2)) * STEP_M;
                            let at = edge - out_dir * inward + dir * v;
                            let (x, z) = (f64::from(at.x), f64::from(at.z));
                            if !inside(x, z) || !(ctx.keep)(x, z) {
                                continue;
                            }
                            if crate::bridges::on_deck(&decks, [at.x, q.y, at.z]) {
                                continue;
                            }
                            let Some(ry) = road_y(&segs, x, z) else {
                                continue;
                            };
                            // Only the smaller half of each kind's sizes
                            // (leaves: all).
                            let (s0, s1) = kind.size();
                            let part = if kind == Kind::Leaf { 1.0 } else { 0.5 };
                            let size = s0 + (s1 - s0) * part * h01(qx, qz, seed(slot, 3)).powf(1.6);
                            let yaw = h01(qx, qz, seed(slot, 4)) * std::f32::consts::TAU;
                            let shade = h01(qx, qz, seed(slot, 5));
                            out.push(Bit {
                                kind,
                                pos: Vec3::new(at.x, (ry + f64::from(lift)) as f32, at.z),
                                size,
                                yaw,
                                normal: Vec3::Y,
                                colour: colour(kind, &place, &fx, shade),
                                ring: [[0.0; 3]; RING],
                                road: true,
                            });
                        }
                    }
                }
            }
            local += len;
        }
    }
    out
}

/// All bits of the tile.
pub fn tile_bits(ctx: &Ctx, p: &Params) -> Vec<Bit> {
    let size = f64::from(ctx.pkg.tile_size_m);
    let o = ctx.key.origin(size);
    bits_in(ctx, [o[0], o[1], o[0] + size, o[1] + size], p)
}

// --- Models -------------------------------------------------------------------

fn rgba(c: [f32; 3], a: f32) -> [f32; 4] {
    [c[0], c[1], c[2], a]
}

/// How far a bit of horizontal reach `r` must sink on its slope so its
/// downhill side does not float.
fn slope_sink(n: Vec3, r: f32) -> f32 {
    let ny = n.y.max(0.3);
    r * (1.0 - ny * ny).sqrt() / ny
}

/// The ground plane's axes at a bit: along its turn and across.
fn plane_axes(n: Vec3, yaw: f32) -> (Vec3, Vec3) {
    let t = Vec3::new(yaw.cos(), 0.0, yaw.sin());
    let t = (t - n * t.dot(n)).normalize_or(Vec3::X);
    (t, n.cross(t))
}

/// A thin blade of grass or straw: one triangle, drawn from both sides,
/// its normal leaning up (lit like the grass around).
#[allow(clippy::too_many_arguments)]
fn blade3(
    m: &mut MeshArrays,
    root: Vec3,
    lean: Vec3,
    height: f32,
    width: f32,
    foot_y: f32,
    c: [f32; 4],
) {
    let side = Vec3::Y.cross(lean).normalize_or(Vec3::X) * (width / 2.0);
    let tip = root + lean * height + Vec3::Y * height;
    let n = (Vec3::Y + lean * 0.4).normalize();
    let b = m.positions.len() as u32;
    for p in [root - side, root + side, tip] {
        m.positions.push(p.to_array());
        m.normals.push(n.to_array());
        m.colors.push(c);
        m.uvs.push([p.y - foot_y, 0.0]);
    }
    m.indices.extend([b, b + 1, b + 2, b, b + 2, b + 1]);
}

#[allow(clippy::too_many_arguments)]
fn flat_quad(
    m: &mut MeshArrays,
    centre: Vec3,
    n: Vec3,
    t: Vec3,
    b: Vec3,
    l: f32,
    w: f32,
    c: [f32; 4],
) {
    let base = m.positions.len() as u32;
    let lift = n * 0.012;
    for (sl, sw, curl) in [
        (-1.0, -1.0, 0.004),
        (1.0, -1.0, 0.0),
        (1.0, 1.0, 0.006),
        (-1.0, 1.0, 0.0),
    ] {
        let p = centre + lift + t * (sl * l / 2.0) + b * (sw * w / 2.0) + n * curl;
        m.positions.push(p.to_array());
        m.normals.push(n.to_array());
        m.colors.push(c);
        m.uvs.push([0.0, 0.0]);
    }
    m.indices
        .extend([base, base + 2, base + 1, base, base + 3, base + 2]);
}

/// Adds a bit's model to the chunk's meshes (`solid` in the trees'
/// material, `patches` blended), relative to `origin`.
pub fn add_bit(solid: &mut MeshArrays, patches: &mut MeshArrays, b: &Bit, origin: Vec3) {
    let at = b.pos - origin;
    let foot = at.y;
    let h = |k: u32| {
        hash(
            (b.pos.x * 100.0) as i64,
            (b.pos.z * 100.0) as i64,
            0x67_726e_6400 + u64::from(k),
        )
    };
    match b.kind {
        Kind::Gravel | Kind::Pebble => {
            let r = b.size;
            let flat = if b.kind == Kind::Gravel {
                0.45
            } else {
                0.55 + 0.2 * h(1)
            };
            let base = at - Vec3::Y * slope_sink(b.normal, r);
            let seg = if b.kind == Kind::Gravel { 4 } else { 5 };
            crate::flora::blob(
                solid,
                base,
                Vec3::new(0.0, r * flat * 0.6, 0.0),
                Vec3::new(r, r * flat, r * (0.7 + 0.3 * h(2))),
                1.0,
                seg,
                1,
                b.colour,
                b.yaw,
            );
        }
        Kind::Rock => {
            let r = b.size;
            let flat = 0.5 + 0.25 * h(1);
            let base = at - Vec3::Y * (slope_sink(b.normal, r) * 0.5);
            let first = solid.positions.len();
            crate::flora::blob(
                solid,
                base,
                Vec3::new(0.0, r * flat * 0.55, 0.0),
                Vec3::new(r, r * flat, r * (0.65 + 0.3 * h(2))),
                1.0,
                7,
                3,
                b.colour,
                b.yaw,
            );
            // Lumps: each vertex pushed in or out from the rock's centre.
            let c = base + Vec3::Y * (r * flat * 0.55);
            for (i, p) in solid.positions[first..].iter_mut().enumerate() {
                let k = 0.82 + 0.3 * h(10 + i as u32);
                let q = c + (Vec3::from(*p) - c) * k;
                *p = q.to_array();
            }
        }
        Kind::Dirt | Kind::Moss | Kind::Needles | Kind::LeafLitter => {
            let alpha = match b.kind {
                Kind::Moss => 0.85,
                Kind::Needles | Kind::LeafLitter => 0.8,
                _ => 0.7,
            };
            let n = b.normal.to_array();
            let base = patches.positions.len() as u32;
            let centre = at + Vec3::Y * PATCH_LIFT_M;
            let push = |m: &mut MeshArrays, p: Vec3, a: f32| {
                m.positions.push(p.to_array());
                m.normals.push(n);
                m.colors.push(rgba(b.colour, a));
                m.uvs.push([0.0, 0.0]);
            };
            push(patches, centre, alpha);
            for (i, r) in b.ring.iter().enumerate() {
                let a = b.yaw + i as f32 / RING as f32 * std::f32::consts::TAU;
                let d = Vec3::new(a.cos(), 0.0, a.sin()) * (b.size * r[0]);
                let inner = at + d * INNER + Vec3::Y * (r[2] + PATCH_LIFT_M);
                push(patches, inner, alpha * 0.85);
            }
            for (i, r) in b.ring.iter().enumerate() {
                let a = b.yaw + i as f32 / RING as f32 * std::f32::consts::TAU;
                let d = Vec3::new(a.cos(), 0.0, a.sin()) * (b.size * r[0]);
                let outer = at + d + Vec3::Y * (r[1] + PATCH_LIFT_M);
                push(patches, outer, 0.0);
            }
            let ring = RING as u32;
            for i in 0..ring {
                let j = (i + 1) % ring;
                let (i0, j0) = (base + 1 + i, base + 1 + j);
                let (i1, j1) = (base + 1 + ring + i, base + 1 + ring + j);
                // Counter-clockwise seen from above (angles grow from +x
                // towards +z).
                patches.indices.extend([base, j0, i0]);
                patches.indices.extend([i0, j0, j1, i0, j1, i1]);
            }
        }
        Kind::Leaf => {
            let (t, bb) = plane_axes(b.normal, b.yaw);
            flat_quad(
                solid,
                at,
                b.normal,
                t,
                bb,
                b.size,
                b.size * 0.6,
                rgba(b.colour, 1.0),
            );
        }
        Kind::Twig => {
            let (t, _) = plane_axes(b.normal, b.yaw);
            let r = 0.008 + 0.008 * h(1);
            let lift = b.normal * (r * 0.6);
            let a = at - t * (b.size / 2.0) + lift;
            let e = at + t * (b.size / 2.0) + lift;
            crate::flora::tube(solid, a, e, r, r * 0.6, 3, foot, b.colour);
        }
        Kind::Cone => {
            let (t, _) = plane_axes(b.normal, b.yaw);
            let r = b.size * 0.32;
            let lift = b.normal * (r * 0.8);
            let a = at - t * (b.size / 2.0) + lift;
            let e = at + t * (b.size / 2.0) + lift;
            crate::flora::tube(solid, a, e, r, r * 0.45, 5, foot, b.colour);
        }
        Kind::Tuft | Kind::Stalks => {
            let (count, width, lean) = if b.kind == Kind::Tuft {
                (5 + (h(1) * 3.0) as u32, 0.035, 0.45)
            } else {
                (3 + (h(1) * 3.0) as u32, 0.012, 0.15)
            };
            let root = at - Vec3::Y * 0.03;
            let c = rgba(b.colour, 1.0);
            for i in 0..count {
                let a = b.yaw + i as f32 / count as f32 * std::f32::consts::TAU + h(2 + i) * 0.8;
                let dir = Vec3::new(a.cos(), 0.0, a.sin());
                let off = dir * (0.02 + 0.03 * h(20 + i));
                let height = b.size * (0.6 + 0.4 * h(40 + i));
                let k = 0.85 + 0.3 * h(60 + i);
                let shade = [c[0] * k, c[1] * k, c[2] * k, 1.0];
                blade3(
                    solid,
                    root + off,
                    dir * lean * (0.5 + h(80 + i)),
                    height,
                    width,
                    foot,
                    shade,
                );
            }
        }
        Kind::Fern => {
            let fronds = 5 + (h(1) * 3.0) as u32;
            for i in 0..fronds {
                let a = b.yaw + i as f32 / fronds as f32 * std::f32::consts::TAU;
                let dir = Vec3::new(a.cos(), 0.0, a.sin());
                let len = b.size * (0.8 + 0.4 * h(2 + i));
                crate::flora::blade(
                    solid,
                    at - Vec3::Y * 0.02,
                    dir,
                    1.1,
                    len,
                    len * 0.32,
                    len * 0.75,
                    3,
                    foot,
                    b.colour,
                );
            }
        }
        Kind::Shrub => {
            let parts = 2 + (h(1) * 2.0) as u32;
            let base = at - Vec3::Y * 0.05;
            for i in 0..parts {
                let a = b.yaw + i as f32 * 2.3;
                let off = Vec3::new(a.cos(), 0.0, a.sin()) * (b.size * 0.35 * h(2 + i));
                let r = b.size * (0.45 + 0.25 * h(10 + i));
                let k = 0.85 + 0.3 * h(20 + i);
                crate::flora::blob(
                    solid,
                    base + off,
                    Vec3::new(0.0, r * 0.85, 0.0),
                    Vec3::new(r, r * 0.85, r),
                    1.0,
                    6,
                    2,
                    b.colour.map(|v| v * k),
                    a,
                );
            }
        }
    }
}

/// The meshes of `bits` relative to `origin`: the solid one and the
/// blended patches.
pub fn meshes(bits: &[Bit], origin: Vec3) -> (MeshArrays, MeshArrays) {
    let (mut solid, mut patches) = (MeshArrays::default(), MeshArrays::default());
    for b in bits {
        add_bit(&mut solid, &mut patches, b, origin);
    }
    solid.shrink_to_fit();
    patches.shrink_to_fit();
    (solid, patches)
}

// --- Chunks of a tile -----------------------------------------------------------

/// Cells per chunk side of the kept mask (2.5 m cells for 25 m chunks).
const MASK: usize = 10;

/// A chunk of a tile with ground detail.
#[derive(Debug, Clone, PartialEq)]
pub struct DetailChunk {
    pub cell: [u16; 2],
    /// Its centre on the ground (package metres): the entity's place.
    pub centre: [f32; 3],
    /// Which 2.5 m cells the world set keeps (`None`: all).
    keep: Option<[u64; 2]>,
}

/// What a tile keeps to place its ground detail later, near a camera.
pub struct TileGroundDetail {
    pub key: TileKey,
    /// The tile's terrain mesh's vertices (positions only): the drawn
    /// ground.
    pub terrain: MeshArrays,
    pub solid: SolidFootprints,
    pub season: Option<Arc<SeasonDay>>,
    pub chunks: Vec<DetailChunk>,
}

impl Default for TileGroundDetail {
    fn default() -> Self {
        Self {
            key: TileKey { x: 0, z: 0 },
            terrain: MeshArrays::default(),
            solid: SolidFootprints::default(),
            season: None,
            chunks: Vec::new(),
        }
    }
}

impl TileGroundDetail {
    pub fn bytes(&self) -> usize {
        self.terrain.bytes() + self.chunks.capacity() * std::mem::size_of::<DetailChunk>()
    }
}

/// The chunk cell's rect (package x0, z0, x1, z1).
fn chunk_rect(pkg: &WorldPackage, key: TileKey, cell: [u16; 2]) -> [f64; 4] {
    let size = f64::from(pkg.tile_size_m);
    let cs = size / f64::from(CHUNKS);
    let o = key.origin(size);
    let x0 = o[0] + f64::from(cell[0]) * cs;
    let z0 = o[1] + f64::from(cell[1]) * cs;
    [x0, z0, x0 + cs, z0 + cs]
}

/// The chunks of tile `key` near its roads, with what the tile build
/// knows (`terrain`: its terrain mesh as drawn).
pub fn tile_detail(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    terrain: &MeshArrays,
    solid: &SolidFootprints,
    season: Option<Arc<SeasonDay>>,
) -> TileGroundDetail {
    let Some(tile) = pkg.tiles.get(&key) else {
        return TileGroundDetail::default();
    };
    let size = f64::from(pkg.tile_size_m);
    let cs = size / f64::from(CHUNKS);
    let o = key.origin(size);
    let n = CHUNKS as usize;
    let mut near = vec![false; n * n];
    for s in &tile.roads {
        let walk = s.dress.sidewalk_m[0].max(s.dress.sidewalk_m[1]);
        let reach = f64::from(s.width_m / 2.0 + walk + BAND_M + STEP_M + 1.0);
        for w in s.points.windows(2) {
            let (a, b) = (w[0], w[1]);
            let lo = [
                f64::from(a[0].min(b[0])) - reach,
                f64::from(a[2].min(b[2])) - reach,
            ];
            let hi = [
                f64::from(a[0].max(b[0])) + reach,
                f64::from(a[2].max(b[2])) + reach,
            ];
            let c = |v: f64, o: f64| ((v - o) / cs).floor().clamp(0.0, (n - 1) as f64) as usize;
            if hi[0] < o[0] || hi[1] < o[1] || lo[0] > o[0] + size || lo[1] > o[1] + size {
                continue;
            }
            // Only the cells the segment's band passes (a long diagonal
            // segment's box would take a square of cells).
            let (ax, az, bx, bz) = (
                f64::from(a[0]),
                f64::from(a[2]),
                f64::from(b[0]),
                f64::from(b[2]),
            );
            for k in c(lo[1], o[1])..=c(hi[1], o[1]) {
                for i in c(lo[0], o[0])..=c(hi[0], o[0]) {
                    let (cx, cz) = (o[0] + (i as f64 + 0.5) * cs, o[1] + (k as f64 + 0.5) * cs);
                    let (dx, dz) = (bx - ax, bz - az);
                    let l2 = (dx * dx + dz * dz).max(1e-9);
                    let t = (((cx - ax) * dx + (cz - az) * dz) / l2).clamp(0.0, 1.0);
                    let d = ((cx - ax - dx * t).powi(2) + (cz - az - dz * t).powi(2)).sqrt();
                    if d <= reach + cs * std::f64::consts::FRAC_1_SQRT_2 {
                        near[k * n + i] = true;
                    }
                }
            }
        }
    }
    let ground = TileGround::new(pkg, key, Some(terrain));
    let chunks = near
        .iter()
        .enumerate()
        .filter(|(_, on)| **on)
        .map(|(idx, _)| {
            let cell = [(idx % n) as u16, (idx / n) as u16];
            let r = chunk_rect(pkg, key, cell);
            let (cx, cz) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
            let cy = ground.height(cx, cz).unwrap_or(0.0);
            let keep = keep.map(|f| {
                let mut m = [0u64; 2];
                let c = (r[2] - r[0]) / MASK as f64;
                for j in 0..MASK {
                    for i in 0..MASK {
                        let p = [r[0] + (i as f64 + 0.5) * c, r[1] + (j as f64 + 0.5) * c];
                        if f(p) {
                            let b = j * MASK + i;
                            m[b / 64] |= 1 << (b % 64);
                        }
                    }
                }
                m
            });
            DetailChunk {
                cell,
                centre: [cx as f32, cy as f32, cz as f32],
                keep,
            }
        })
        .filter(|c| c.keep != Some([0, 0]))
        .collect();
    TileGroundDetail {
        key,
        terrain: MeshArrays {
            positions: terrain.positions.clone(),
            ..Default::default()
        },
        solid: solid.clone(),
        season,
        chunks,
    }
}

/// A tile's ground detail, shared by its chunks, with the package it was
/// built from – not kept alive by them (spec §14 Phase 10 B2: a streamed
/// package gets a new version whenever tiles come or go, and each tile
/// holding the version it was uploaded with kept up to 25 of them). When
/// that version is gone, the chunks take the current package of their
/// member of the world set ([`DetailSource::package`]).
pub struct DetailSource {
    pub pkg: Weak<WorldPackage>,
    /// The member of the world set the tile belongs to.
    pub member: usize,
    pub tile: TileGroundDetail,
}

impl DetailSource {
    /// The package to build from: the tile's own if it is still there,
    /// else the current one of its member that has the tile (`None`: the
    /// tile is no longer resident – nothing to build).
    pub fn package(
        &self,
        world: Option<&crate::package::PackageWorld>,
    ) -> Option<Arc<WorldPackage>> {
        self.pkg
            .upgrade()
            .or_else(|| world?.set.members().get(self.member)?.package().cloned())
            .filter(|p| p.tiles.contains_key(&self.tile.key))
    }

    /// The bits of a chunk.
    pub fn bits(&self, pkg: &WorldPackage, chunk: &DetailChunk, p: &Params) -> Vec<Bit> {
        let ground = TileGround::new(pkg, self.tile.key, Some(&self.tile.terrain));
        let rect = chunk_rect(pkg, self.tile.key, chunk.cell);
        let keep = |x: f64, z: f64| {
            let Some(m) = chunk.keep else {
                return true;
            };
            let c = (rect[2] - rect[0]) / MASK as f64;
            let i = (((x - rect[0]) / c).floor().max(0.0) as usize).min(MASK - 1);
            let j = (((z - rect[1]) / c).floor().max(0.0) as usize).min(MASK - 1);
            let b = j * MASK + i;
            m[b / 64] >> (b % 64) & 1 == 1
        };
        let ctx = Ctx {
            pkg,
            key: self.tile.key,
            ground: &ground,
            solid: &self.tile.solid,
            season: self.tile.season.as_deref(),
            keep: &keep,
        };
        bits_in(&ctx, rect, p)
    }

    /// A chunk's meshes (relative to its centre) and its bit count.
    pub fn build(
        &self,
        pkg: &WorldPackage,
        chunk: &DetailChunk,
        p: &Params,
    ) -> (MeshArrays, MeshArrays, usize) {
        let bits = self.bits(pkg, chunk, p);
        let (a, b) = meshes(&bits, Vec3::from(chunk.centre));
        (a, b, bits.len())
    }
}

// --- Drawing --------------------------------------------------------------------

/// How much ground detail is drawn (setting `display.ground_detail`,
/// console `ground_detail`; spec 4.6.2 Q1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GroundDetailMode {
    /// As the quality preset says (High all of it to 80 m, Medium 60 % to
    /// 55 m, Low 25 % to 35 m) – the default.
    #[default]
    Auto,
    Off,
    /// [`LOW_LEVEL`] whatever the preset.
    Low,
    /// [`FULL_LEVEL`] whatever the preset.
    Full,
}

impl GroundDetailMode {
    /// The setting's choices (`on`: the console's old word for `auto`).
    pub const CHOICES: [&'static str; 4] = ["auto", "off", "low", "full"];

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "auto" | "on" => Self::Auto,
            "off" => Self::Off,
            "low" => Self::Low,
            "full" => Self::Full,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Off => "off",
            Self::Low => "low",
            Self::Full => "full",
        }
    }
}

/// The ground detail's setting (console `ground_detail
/// auto|off|low|full`; `display.ground_detail`, default auto).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct GroundDetailSettings {
    pub mode: GroundDetailMode,
}

/// The materials: the trees' (swaying, snow on top) and a blended one for
/// the patches.
#[derive(Resource)]
pub struct GroundDetailMaterials {
    pub solid: Handle<crate::see_through::SeeThroughMaterial>,
    pub patches: Handle<StandardMaterial>,
}

/// The wetness the materials were last set for (spec 4.6.2 Q3).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct GroundDetailWet(pub f32);

/// What a chunk was built for.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BuildKey {
    pub density: f32,
    pub snow: bool,
}

/// A chunk of ground detail on its entity (placed at its centre).
#[derive(Component)]
pub struct LazyGround {
    pub src: Arc<DetailSource>,
    pub chunk: DetailChunk,
    pub built: Option<BuildKey>,
    /// Its blended patches' entity (a child).
    pub patches: Option<Entity>,
    pub bits: usize,
}

impl LazyGround {
    pub fn new(src: Arc<DetailSource>, chunk: DetailChunk) -> Self {
        Self {
            src,
            chunk,
            built: None,
            patches: None,
            bits: 0,
        }
    }
}

/// The patches' child of a chunk.
#[derive(Component)]
pub struct GroundPatches;

/// A chunk being built.
#[derive(Component)]
pub struct GroundBuild(Task<(MeshArrays, MeshArrays, usize)>, BuildKey);

/// What [`lazy_ground`] does with a chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroundStep {
    Build,
    Drop,
    Keep,
}

/// The step for a chunk `d` m from the nearest camera with the draw
/// distance `draw_m`: built as it is (`same`), built for other settings, or
/// not built.
pub fn ground_step(d: f32, draw_m: f32, built: bool, same: bool) -> GroundStep {
    if draw_m <= 0.0 {
        return if built {
            GroundStep::Drop
        } else {
            GroundStep::Keep
        };
    }
    if built && d > draw_m + DROP_MARGIN_M {
        GroundStep::Drop
    } else if (!built || !same) && d <= draw_m + BUILD_MARGIN_M {
        GroundStep::Build
    } else {
        GroundStep::Keep
    }
}

/// The visibility range of a chunk drawn up to `draw_m` from its centre,
/// faded out (dithered) over the last quarter.
pub fn detail_range(draw_m: f32) -> VisibilityRange {
    let end = draw_m.max(1.0);
    VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: 0.75 * end..end,
        use_aabb: false,
    }
}

/// The share and distance of the preset with the setting (spec 4.6.2 Q1:
/// `low` and `full` override the preset, `auto` follows it).
pub fn level(
    quality: &crate::quality::QualityPreset,
    settings: &GroundDetailSettings,
) -> (f32, f32) {
    match settings.mode {
        GroundDetailMode::Off => (0.0, 0.0),
        GroundDetailMode::Low => LOW_LEVEL,
        GroundDetailMode::Full => FULL_LEVEL,
        GroundDetailMode::Auto if quality.ground_detail <= 0.0 => (0.0, 0.0),
        GroundDetailMode::Auto => (quality.ground_detail, quality.ground_detail_m),
    }
}

/// Darkens the ground detail and makes it shine while the road is wet
/// (spec 4.6.2 Q3; [`wet_look`]), like the wet road of Phase 9: the two
/// materials' colour factor and roughness, changed only when the
/// (quantized) wetness changes.
pub fn wet_ground_detail(
    view: Option<Res<crate::RideView>>,
    materials: Option<Res<GroundDetailMaterials>>,
    mut last: ResMut<GroundDetailWet>,
    mut solid: ResMut<Assets<crate::see_through::SeeThroughMaterial>>,
    mut blended: ResMut<Assets<StandardMaterial>>,
) {
    let Some(materials) = materials else {
        return;
    };
    let wet = crate::weather::surface_uniform(view.as_ref().and_then(|v| v.weather.as_ref())).x;
    if wet == last.0 {
        return;
    }
    last.0 = wet;
    let (k, rough) = wet_look(wet);
    let c = Color::linear_rgb(k, k, k);
    if let Some(mut m) = solid.get_mut(&materials.solid) {
        m.base.base_color = c;
        m.base.perceptual_roughness = rough;
    }
    if let Some(mut m) = blended.get_mut(&materials.patches) {
        m.base_color = c;
        m.perceptual_roughness = rough;
    }
}

fn setup_materials(
    mut commands: Commands,
    mut solid: ResMut<Assets<crate::see_through::SeeThroughMaterial>>,
    mut blended: ResMut<Assets<StandardMaterial>>,
) {
    let base = StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 1.0,
        ..default()
    };
    commands.insert_resource(GroundDetailMaterials {
        solid: solid.add(crate::see_through::swaying(
            crate::see_through::see_through(base.clone()),
            None,
        )),
        patches: blended.add(StandardMaterial {
            alpha_mode: AlphaMode::Blend,
            reflectance: 0.2,
            ..base
        }),
    });
}

/// Builds the chunks' meshes near a camera on the task pool (nearest
/// first, at most [`MAX_TASKS`] at a time), again when the preset's share
/// or the snow changes, and drops them far away.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub fn lazy_ground(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    materials: Option<Res<GroundDetailMaterials>>,
    settings: Res<GroundDetailSettings>,
    quality: Option<Res<crate::quality::RenderQuality>>,
    view: Option<Res<crate::RideView>>,
    world: Option<Res<crate::package::PackageWorld>>,
    cameras: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    building: Query<(), With<GroundBuild>>,
    mut last: Local<(u32, Option<BuildKey>)>,
    mut chunks: Query<(
        Entity,
        Ref<GlobalTransform>,
        &mut LazyGround,
        Option<&mut GroundBuild>,
        &mut VisibilityRange,
    )>,
) {
    let Some(materials) = materials else {
        return;
    };
    let preset = quality.map(|q| q.0).unwrap_or_default();
    let (density, draw_m) = level(&preset, &settings);
    let snow = view
        .as_ref()
        .and_then(|v| v.weather.as_ref())
        .is_some_and(|w| w.ground.snow_cover() >= SNOW_HIDES_SMALL);
    let want = BuildKey { density, snow };
    // A full pass over the chunks (some 20,000 with the tiles of a world
    // set) every [`SCAN_EVERY`] frames while nothing is being built and
    // nothing changed: at 40 km/h the rider moves less than a metre.
    last.0 = last.0.wrapping_add(1);
    if last.1 == Some(want) && building.is_empty() && !last.0.is_multiple_of(SCAN_EVERY) {
        return;
    }
    last.1 = Some(want);
    let range = detail_range(draw_m);
    let eyes: Vec<Vec3> = cameras
        .iter()
        .filter(|(c, _)| c.is_active)
        .map(|(_, g)| g.translation())
        .collect();
    let mut todo: Vec<(f32, Entity)> = Vec::new();
    let mut busy = 0;
    for (e, g, mut lazy, task, mut vr) in &mut chunks {
        if g.is_added() {
            continue;
        }
        if *vr != range {
            *vr = range.clone();
        }
        let at = g.translation();
        let d = eyes
            .iter()
            .map(|p| p.distance(at))
            .fold(f32::INFINITY, f32::min);
        if let Some(mut task) = task {
            if d > draw_m + DROP_MARGIN_M || task.1 != want {
                commands.entity(e).remove::<GroundBuild>();
            } else if let Some((solid, patches, n)) = check_ready(&mut task.0) {
                let key = task.1;
                let mut ec = commands.entity(e);
                ec.remove::<GroundBuild>();
                if solid.indices.is_empty() {
                    ec.remove::<(
                        Mesh3d,
                        MeshMaterial3d<crate::see_through::SeeThroughMaterial>,
                    )>();
                } else {
                    ec.insert((
                        Mesh3d(meshes.add(solid.mesh())),
                        MeshMaterial3d(materials.solid.clone()),
                    ));
                }
                if let Some(old) = lazy.patches.take() {
                    commands.entity(old).despawn();
                }
                if !patches.indices.is_empty() {
                    let child = commands
                        .spawn((
                            Mesh3d(meshes.add(patches.mesh())),
                            MeshMaterial3d(materials.patches.clone()),
                            Transform::IDENTITY,
                            Visibility::default(),
                            NotShadowCaster,
                            range.clone(),
                            GroundPatches,
                            ChildOf(e),
                        ))
                        .id();
                    lazy.patches = Some(child);
                }
                lazy.built = Some(key);
                lazy.bits = n;
            } else {
                busy += 1;
            }
            continue;
        }
        let built = lazy.built.is_some();
        match ground_step(d, draw_m, built, lazy.built == Some(want)) {
            GroundStep::Build => todo.push((d, e)),
            GroundStep::Drop => {
                commands.entity(e).remove::<(
                    Mesh3d,
                    MeshMaterial3d<crate::see_through::SeeThroughMaterial>,
                )>();
                if let Some(old) = lazy.patches.take() {
                    commands.entity(old).despawn();
                }
                lazy.built = None;
                lazy.bits = 0;
            }
            GroundStep::Keep => {}
        }
    }
    todo.sort_by(|a, b| a.0.total_cmp(&b.0));
    for &(_, e) in todo.iter().take(MAX_TASKS.saturating_sub(busy)) {
        if let Ok((_, _, lazy, _, _)) = chunks.get(e) {
            let src = lazy.src.clone();
            // The tile is no longer resident: nothing to build.
            let Some(pkg) = src.package(world.as_deref()) else {
                continue;
            };
            let chunk = lazy.chunk.clone();
            let p = Params { density, snow };
            let task =
                AsyncComputeTaskPool::get().spawn(async move { src.build(&pkg, &chunk, &p) });
            commands.entity(e).insert(GroundBuild(task, want));
        }
    }
}

/// Spawns a tile's ground detail chunks as children of its entity.
pub fn spawn_chunks(
    c: &mut bevy::ecs::relationship::RelatedSpawnerCommands<ChildOf>,
    pkg: &Arc<WorldPackage>,
    member: usize,
    mut tile: TileGroundDetail,
) {
    let chunks = std::mem::take(&mut tile.chunks);
    if chunks.is_empty() {
        return;
    }
    let src = Arc::new(DetailSource {
        pkg: Arc::downgrade(pkg),
        member,
        tile,
    });
    for chunk in chunks {
        let at = Transform::from_translation(Vec3::from(chunk.centre));
        c.spawn((
            LazyGround::new(src.clone(), chunk),
            at,
            Visibility::default(),
            NotShadowCaster,
            crate::flight_world::TileDetail,
            detail_range(crate::quality::QualityPreset::default().ground_detail_m),
            Name::new("ground detail"),
        ));
    }
}

/// The setting and its console command alone (`ground_detail`; the
/// settings' tests add it without the renderer).
pub struct GroundDetailCommandPlugin;

impl Plugin for GroundDetailCommandPlugin {
    fn build(&self, app: &mut App) {
        register_command(app);
    }
}

/// The systems and the console command (added by the package plugin).
pub fn register(app: &mut App) {
    if !app.is_plugin_added::<GroundDetailCommandPlugin>() {
        app.add_plugins(GroundDetailCommandPlugin);
    }
    app.init_resource::<GroundDetailWet>()
        .add_systems(Startup, setup_materials)
        .add_systems(Update, wet_ground_detail);
}

fn register_command(app: &mut App) {
    app.init_resource::<GroundDetailSettings>()
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ground_detail")
                .about("Stones, dirt, tufts, leaves and plant bits beside and on the roads: auto (the quality preset's), off, low or full (setting display.ground_detail; on = auto; spec 4.6.2)")
                .arg(Arg::new("mode").value_parser(["auto", "on", "off", "low", "full"])),
            |world, m| {
                if let Some(v) = m.get_one::<String>("mode") {
                    world.resource_mut::<GroundDetailSettings>().mode =
                        GroundDetailMode::parse(v).ok_or_else(|| format!("auto, off, low or full, not {v}"))?;
                }
                let settings = *world.resource::<GroundDetailSettings>();
                let preset = world
                    .get_resource::<crate::quality::RenderQuality>()
                    .map(|q| q.0)
                    .unwrap_or_default();
                let (share, draw) = level(&preset, &settings);
                let mut q = world.query::<&LazyGround>();
                let (mut chunks, mut built, mut bits) = (0, 0, 0);
                for l in q.iter(world) {
                    chunks += 1;
                    if l.built.is_some() {
                        built += 1;
                        bits += l.bits;
                    }
                }
                Ok(format!(
                    "ground detail {}: share {share:.2} up to {draw:.0} m; {built} of {chunks} chunks built, {bits} bits",
                    settings.mode.name()
                ))
            },
        ));
}

#[cfg(test)]
mod tests;

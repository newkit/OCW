//! The big plants, each species with its own low-poly model (spec §4.6.1,
//! §14 Phase 8 T8): own work generated in code, like the rider and bike
//! models – no third-party meshes. Every model comes in two levels of
//! detail: **near** (the tiers of a spruce, the tufts of a Canary pine, the
//! fronds of a palm, about 40 – 250 triangles) and **far** (one crown and a
//! trunk, about 10 – 30 triangles), drawn by the tile's tree chunks with a
//! dithered cross-fade between them (`package::tile_tree_chunks`).
//!
//! The season of each plant ([`plant_look`]): leaves, fresh green and
//! autumn colour by its leaf type and its species' timing (birches early,
//! ashes and oaks late; `ocw_sky::season::Timing`), blossom (fruit trees in
//! April, Teide broom in May and June), the summer leaf drop of the
//! tabaibas in the dry season; colours per species ([`crown_colour`]):
//! beech copper, maple yellow and orange, larch gold, ash dull yellow-green.
//! Vertex `uv.x` is the height above the foot: the tree material sways with
//! it in the wind (spec §14 Phase 9 S4).

use bevy::prelude::*;
use ocw_sky::season::{LeafType, SeasonDay, Timing};
use ocw_world::flora::{Habit, Species};

use crate::package::MeshArrays;

/// Which level of detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Detail {
    Near,
    Far,
}

/// What a plant looks like on the day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantLook {
    /// How much of the crown is in leaf (0 bare – 1 full).
    pub leaves: f32,
    /// Young spring green.
    pub fresh: f32,
    /// Autumn colour.
    pub autumn: f32,
    /// In flower (fruit trees, broom).
    pub bloom: f32,
    /// Dried in the summer drought (tabaiba drops its leaves).
    pub dry: f32,
}

impl PlantLook {
    /// Full summer leaves, the look without a season.
    pub const SUMMER: PlantLook = PlantLook {
        leaves: 1.0,
        fresh: 0.0,
        autumn: 0.0,
        bloom: 0.0,
        dry: 0.0,
    };
}

/// The leaf type the seasons give a species.
pub fn leaf_type(sp: Species) -> LeafType {
    match sp.habit() {
        Habit::Deciduous => LeafType::Deciduous,
        Habit::Larch => LeafType::Larch,
        _ => LeafType::Evergreen,
    }
}

/// When a species leafs out and turns (spec §4.6.1 table 5).
pub fn timing(sp: Species) -> Timing {
    let (spring, autumn) = match sp {
        Species::Birch => (0.8, 0.95),
        Species::Willow => (0.75, 1.15),
        Species::Ash => (1.35, 0.85),
        Species::Oak => (1.25, 1.15),
        Species::SycamoreMaple => (0.95, 0.95),
        Species::Alder | Species::GreenAlder => (0.9, 1.15),
        Species::Lime => (0.95, 0.9),
        Species::FruitTree => (0.92, 1.0),
        Species::Chestnut => (1.15, 1.05),
        _ => (1.0, 1.0),
    };
    Timing { spring, autumn }
}

/// A bump from 0 to 1 and back over `lo … hi` (smooth edges a fifth wide).
fn bump(x: f64, lo: f64, hi: f64) -> f32 {
    let e = (hi - lo) / 5.0;
    let up = ((x - lo) / e).clamp(0.0, 1.0);
    let down = ((hi - x) / e).clamp(0.0, 1.0);
    (up.min(down)) as f32
}

/// The look of a plant of `sp` at `alt_m` on `day` (`jitter` −1 … 1, its
/// own: not every tree turns on the same day).
pub fn plant_look(day: Option<&SeasonDay>, sp: Species, alt_m: f64, jitter: f32) -> PlantLook {
    let Some(day) = day else {
        return PlantLook::SUMMER;
    };
    let leaf = leaf_type(sp);
    let f = day.foliage_timed(leaf, alt_m, jitter, timing(sp));
    let st = day.state(alt_m);
    let gdd = f64::from(st.gdd) * (1.0 + 0.08 * f64::from(jitter));
    let bloom = match sp {
        // Apple, pear and cherry blossom: from budburst to full leaf.
        Species::FruitTree => bump(gdd, 60.0, 240.0),
        // The Teide broom flowers in May and June.
        Species::Broom => bump(day.day + 6.0 * f64::from(jitter), 128.0, 178.0),
        _ => 0.0,
    };
    let grass = day.grass(alt_m);
    let dry = match sp {
        Species::Tabaiba => grass.dry,
        _ => 0.0,
    };
    PlantLook {
        leaves: f.leaves,
        fresh: f.fresh,
        autumn: f.autumn,
        bloom,
        dry,
    }
}

fn lin3(r: f32, g: f32, b: f32) -> [f32; 3] {
    let c = Color::srgb(r, g, b).to_linear();
    [c.red, c.green, c.blue]
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    let t = t.clamp(0.0, 1.0);
    std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t)
}

fn shade3(c: [f32; 3], k: f32) -> [f32; 3] {
    c.map(|v| v * k)
}

/// The summer crown colour of a species (sRGB).
fn summer_srgb(sp: Species) -> [f32; 3] {
    use Species::*;
    match sp {
        NorwaySpruce => [0.11, 0.25, 0.13],
        SilverFir => [0.11, 0.25, 0.17],
        DouglasFir => [0.17, 0.33, 0.17],
        ScotsPine => [0.20, 0.33, 0.17],
        MountainPine => [0.14, 0.27, 0.13],
        Larch => [0.36, 0.52, 0.22],
        Beech => [0.25, 0.43, 0.15],
        SycamoreMaple => [0.24, 0.42, 0.16],
        Ash => [0.28, 0.43, 0.18],
        Oak => [0.23, 0.38, 0.14],
        Alder => [0.17, 0.32, 0.12],
        Willow => [0.42, 0.52, 0.34],
        GreenAlder => [0.30, 0.48, 0.17],
        Birch => [0.36, 0.52, 0.20],
        Unknown | Lime => [0.25, 0.44, 0.17],
        FruitTree => [0.27, 0.45, 0.18],
        Chestnut => [0.24, 0.40, 0.14],
        CanaryPine => [0.30, 0.42, 0.17],
        Laurel => [0.13, 0.28, 0.11],
        FayaHeath => [0.21, 0.30, 0.14],
        DatePalm => [0.25, 0.40, 0.15],
        SlenderPalm => [0.28, 0.43, 0.18],
        DragonTree => [0.24, 0.36, 0.20],
        Broom => [0.38, 0.44, 0.26],
        Tabaiba => [0.40, 0.48, 0.24],
        Cardon => [0.30, 0.40, 0.22],
        PricklyPear => [0.33, 0.45, 0.24],
        Agave => [0.40, 0.50, 0.44],
        Eucalyptus => [0.36, 0.44, 0.32],
        Banana => [0.33, 0.52, 0.17],
        SubtropicalFruit => [0.15, 0.31, 0.12],
    }
}

/// The autumn colour of a species (sRGB; `hue` 0 – 1 its own).
fn autumn_srgb(sp: Species, hue: f32) -> [f32; 3] {
    use Species::*;
    match sp {
        // Beech: copper to orange-brown.
        Beech => {
            if hue < 0.5 {
                [0.72, 0.42, 0.14]
            } else {
                [0.62, 0.33, 0.12]
            }
        }
        // Sycamore: yellow to orange.
        SycamoreMaple => {
            if hue < 0.6 {
                [0.84, 0.66, 0.18]
            } else {
                [0.86, 0.46, 0.12]
            }
        }
        Larch => [0.86, 0.62, 0.20],
        Ash => [0.56, 0.56, 0.24],
        Oak => [0.58, 0.40, 0.18],
        Birch | Willow | Lime => [0.86, 0.72, 0.22],
        Alder | GreenAlder => [0.33, 0.38, 0.16],
        Chestnut => [0.74, 0.56, 0.18],
        FruitTree => {
            if hue < 0.5 {
                [0.82, 0.55, 0.16]
            } else {
                [0.72, 0.26, 0.12]
            }
        }
        _ => summer_srgb(sp),
    }
}

/// The colour of the bare twigs of a crown (linear).
pub fn twig() -> [f32; 3] {
    lin3(0.36, 0.31, 0.27)
}

/// The crown colour (linear) of a plant of `sp` with `look`; `shade` (0 –
/// 1) its own brightness, `hue` (0 – 1) its own autumn tone.
pub fn crown_colour(sp: Species, look: &PlantLook, shade: f32, hue: f32) -> [f32; 3] {
    let summer = summer_srgb(sp);
    let s = lin3(summer[0], summer[1], summer[2]);
    let mut c = shade3(s, 0.86 + 0.28 * shade);
    if leaf_type(sp) != LeafType::Evergreen {
        let fresh = lin3(0.46, 0.64, 0.24);
        c = mix3(c, fresh, look.fresh * 0.7);
        let a = autumn_srgb(sp, hue);
        c = mix3(c, lin3(a[0], a[1], a[2]), look.autumn);
    }
    match sp {
        Species::FruitTree => {
            // White (apple, pear, cherry) or pink (some apples) blossom.
            let b = if hue < 0.7 {
                lin3(0.93, 0.92, 0.88)
            } else {
                lin3(0.93, 0.76, 0.80)
            };
            c = mix3(c, b, look.bloom * 0.85);
        }
        Species::Broom => {
            c = mix3(c, lin3(0.92, 0.88, 0.86), look.bloom * 0.8);
        }
        Species::Tabaiba => {
            c = mix3(c, lin3(0.52, 0.48, 0.42), look.dry * 0.8);
        }
        _ => {}
    }
    c
}

/// The bark colour of a species (linear).
pub fn bark_colour(sp: Species) -> [f32; 3] {
    use Species::*;
    match sp {
        Beech | Ash | SycamoreMaple => lin3(0.45, 0.44, 0.41),
        Birch => lin3(0.86, 0.85, 0.80),
        ScotsPine => lin3(0.50, 0.32, 0.20),
        CanaryPine => lin3(0.38, 0.29, 0.22),
        Eucalyptus => lin3(0.70, 0.66, 0.58),
        DatePalm => lin3(0.44, 0.35, 0.24),
        SlenderPalm => lin3(0.52, 0.47, 0.40),
        DragonTree => lin3(0.58, 0.55, 0.48),
        Banana => lin3(0.40, 0.48, 0.20),
        Cardon | PricklyPear | Agave | Tabaiba => lin3(0.42, 0.40, 0.33),
        _ => lin3(0.35, 0.25, 0.17),
    }
}

// --- Primitives ---------------------------------------------------------------

fn rgba(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 1.0]
}

/// A solid of revolution about the vertical through `base + offset`:
/// rings of (height, radius) from bottom to top (both ends closed by an
/// apex, radius 0), squashed in x / z by `sx` / `sz`, turned by `turn`; all
/// dimensions × `s`. The normals come from the profile's slope.
#[allow(clippy::too_many_arguments)]
pub fn lathe(
    m: &mut MeshArrays,
    base: Vec3,
    offset: Vec3,
    s: f32,
    rings: &[(f32, f32)],
    seg: u32,
    colour: [f32; 3],
    turn: f32,
    (sx, sz): (f32, f32),
) {
    let colour = rgba(colour);
    let n = rings.len();
    let mut prev: Option<(u32, bool)> = None;
    for (idx, &(y, r)) in rings.iter().enumerate() {
        let b = m.positions.len() as u32;
        let apex = r <= 1e-4;
        let (ya, ra) = rings[idx.saturating_sub(1)];
        let (yb, rb) = rings[(idx + 1).min(n - 1)];
        // The profile's tangent (dr, dy): the normal is (dy, -dr).
        let (dr, dy) = (rb - ra, (yb - ya).max(1e-4));
        let count = if apex { 1 } else { seg };
        for j in 0..count {
            let a = turn + j as f32 / seg as f32 * std::f32::consts::TAU;
            let (cx, cz) = if apex { (0.0, 0.0) } else { (a.cos(), a.sin()) };
            let p = base + offset * s + Vec3::new(r * cx * sx, y, r * cz * sz) * s;
            m.positions.push(p.to_array());
            m.uvs.push([(y + offset.y) * s, 0.0]);
            let nrm = if apex {
                Vec3::new(0.0, if idx == 0 { -1.0 } else { 1.0 }, 0.0)
            } else {
                Vec3::new(cx * dy / sx.max(0.2), -dr, cz * dy / sz.max(0.2)).normalize_or(Vec3::Y)
            };
            m.normals.push(nrm.to_array());
            m.colors.push(colour);
        }
        if let Some((pb, papex)) = prev {
            match (papex, apex) {
                (true, true) => {}
                (true, false) => {
                    for j in 0..seg {
                        m.indices.extend([pb, b + j, b + (j + 1) % seg]);
                    }
                }
                (false, true) => {
                    for j in 0..seg {
                        m.indices.extend([pb + j, b, pb + (j + 1) % seg]);
                    }
                }
                (false, false) => {
                    for j in 0..seg {
                        let (a0, a1) = (pb + j, pb + (j + 1) % seg);
                        let (b0, b1) = (b + j, b + (j + 1) % seg);
                        m.indices.extend([a0, b0, a1, a1, b0, b1]);
                    }
                }
            }
        }
        prev = Some((b, apex));
    }
}

/// An ellipsoid blob centred `centre` above `base` (offsets × `s`) with
/// radii `r` (x, y, z, × `s`): a lumpy crown part.
#[allow(clippy::too_many_arguments)]
pub fn blob(
    m: &mut MeshArrays,
    base: Vec3,
    centre: Vec3,
    r: Vec3,
    s: f32,
    seg: u32,
    rings: u32,
    colour: [f32; 3],
    turn: f32,
) {
    let mut prof = Vec::with_capacity(rings as usize + 2);
    for i in 0..=rings + 1 {
        let t = i as f32 / (rings + 1) as f32 * std::f32::consts::PI;
        let y = -t.cos() * r.y;
        let rr = if i == 0 || i == rings + 1 {
            0.0
        } else {
            t.sin()
        };
        prof.push((y, rr));
    }
    let k = (r.x.max(1e-3), r.z.max(1e-3));
    lathe(m, base, centre, s, &prof, seg, colour, turn, k);
}

/// A tapering tube from `a` to `b` (package metres) with radii `ra` /
/// `rb`, closed at both ends: trunks and branches.
#[allow(clippy::too_many_arguments)]
pub fn tube(
    m: &mut MeshArrays,
    a: Vec3,
    b: Vec3,
    ra: f32,
    rb: f32,
    seg: u32,
    foot_y: f32,
    colour: [f32; 3],
) {
    let colour = rgba(colour);
    let d = (b - a).normalize_or(Vec3::Y);
    let u = d.any_orthonormal_vector();
    // (u, v) turn like the lathe's rings (+x towards +z about +y).
    let v = u.cross(d);
    let start = m.positions.len() as u32;
    // Caps' centres.
    for (p, nrm) in [(a, -d), (b, d)] {
        m.positions.push(p.to_array());
        m.uvs.push([p.y - foot_y, 0.0]);
        m.normals.push(nrm.to_array());
        m.colors.push(colour);
    }
    for (p, r) in [(a, ra), (b, rb)] {
        for j in 0..seg {
            let t = j as f32 / seg as f32 * std::f32::consts::TAU;
            let off = u * t.cos() + v * t.sin();
            m.positions.push((p + off * r).to_array());
            m.uvs.push([p.y - foot_y, 0.0]);
            m.normals.push(off.to_array());
            m.colors.push(colour);
        }
    }
    let (ca, cb, ra0, rb0) = (start, start + 1, start + 2, start + 2 + seg);
    for j in 0..seg {
        let (j1, j0) = ((j + 1) % seg, j);
        m.indices
            .extend([ra0 + j0, rb0 + j0, ra0 + j1, ra0 + j1, rb0 + j0, rb0 + j1]);
        // (Outward: see the lathe's winding.)
        m.indices.extend([ca, ra0 + j0, ra0 + j1]);
        m.indices.extend([cb, rb0 + j1, rb0 + j0]);
    }
}

/// A leaf blade (palm frond, banana leaf, agave leaf), both sides: from
/// `root` along `dir` (horizontal part, unit) rising `up` and drooping by
/// `droop` (m at the tip, quadratic), `length` long and `width` wide (the
/// widest at `belly` of its length), in `segs` pieces.
#[allow(clippy::too_many_arguments)]
pub fn blade(
    m: &mut MeshArrays,
    root: Vec3,
    dir: Vec3,
    up: f32,
    length: f32,
    width: f32,
    droop: f32,
    segs: u32,
    foot_y: f32,
    colour: [f32; 3],
) {
    let colour = rgba(colour);
    let side = Vec3::Y.cross(dir).normalize_or(Vec3::X);
    let at = |t: f32| root + dir * length * t + Vec3::Y * (up * length * t - droop * t * t);
    // Both faces are their own triangles with their own normals, each on
    // its triangles' front (spec 15G Q11 (a); before, each face carried the
    // other face's normal, which Blender turns round): the upper face
    // (`flip`) faces `tangent × side`, the lower face the opposite. The tip
    // is one vertex (no zero-area triangle there).
    for flip in [false, true] {
        let b = m.positions.len() as u32;
        for i in 0..=segs {
            let t = i as f32 / segs as f32;
            let c = at(t);
            let tangent = (at((t + 0.01).min(1.0)) - at((t - 0.01).max(0.0))).normalize_or(dir);
            let upper = tangent.cross(side).normalize_or(Vec3::Y);
            let nrm = if flip { upper } else { -upper };
            let w = width * (std::f32::consts::PI * t.powf(0.8)).sin().max(0.0) * 0.5;
            let tip = i == segs;
            let w = w.max(width * 0.05);
            let points: &[Vec3] = if tip {
                &[c]
            } else {
                &[c - side * w, c + side * w]
            };
            for p in points {
                m.positions.push(p.to_array());
                m.uvs.push([p.y - foot_y, 0.0]);
                m.normals.push(nrm.to_array());
                m.colors.push(colour);
            }
        }
        for i in 0..segs {
            let (a0, a1, b0, b1) = (b + 2 * i, b + 2 * i + 1, b + 2 * i + 2, b + 2 * i + 3);
            let last = i + 1 == segs;
            match (flip, last) {
                (true, false) => m.indices.extend([a0, b0, a1, a1, b0, b1]),
                (false, false) => m.indices.extend([a0, a1, b0, a1, b1, b0]),
                // The tip `b0`.
                (true, true) => m.indices.extend([a0, b0, a1]),
                (false, true) => m.indices.extend([a0, a1, b0]),
            }
        }
    }
}

/// A drooping tier of a conifer: from the apex at `top` down to the rim
/// of radius `r` at `bottom`, the underside back up to the trunk.
#[allow(clippy::too_many_arguments)]
fn tier(
    m: &mut MeshArrays,
    base: Vec3,
    s: f32,
    (bottom, top, r): (f32, f32, f32),
    droop: f32,
    seg: u32,
    colour: [f32; 3],
    turn: f32,
) {
    let h = top - bottom;
    // The underside (up to its apex inside the tier) and the upper side
    // meet at the rim: each has its own rim vertices with its own normal
    // (spec 15G Q11 (a); a shared rim normal pointed down, away from the
    // upper side's front, and Blender rewrote it).
    two_sided_cone(
        m,
        base,
        s,
        &[(bottom + h * droop, 0.0), (bottom, r)],
        &[(bottom, r), (bottom + h * 0.45, r * 0.62), (top, 0.0)],
        seg,
        colour,
        turn,
    );
}

/// A closed shell of revolution from `under` (from its apex down to the
/// rim) and `upper` (from the rim up to its apex), with the rim's vertices
/// twice: on each side with that side's normal.
#[allow(clippy::too_many_arguments)]
fn two_sided_cone(
    m: &mut MeshArrays,
    base: Vec3,
    s: f32,
    under: &[(f32, f32)],
    upper: &[(f32, f32)],
    seg: u32,
    colour: [f32; 3],
    turn: f32,
) {
    debug_assert_eq!(under.last(), upper.first());
    lathe(m, base, Vec3::ZERO, s, under, seg, colour, turn, (1.0, 1.0));
    lathe(m, base, Vec3::ZERO, s, upper, seg, colour, turn, (1.0, 1.0));
}

// --- The species --------------------------------------------------------------

/// A plant's place and its own numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlantAt {
    /// The foot of the trunk (package metres).
    pub base: Vec3,
    /// Its scale (× the species' size).
    pub scale: f32,
    /// Its turn about the vertical (radians).
    pub turn: f32,
    /// Its own brightness and autumn tone (0 – 1).
    pub shade: f32,
    pub hue: f32,
    /// Its own variety (0 – 1): an old fir's flat top, an agave's stalk.
    pub pick: f32,
}

fn h01(p: f32, k: u32) -> f32 {
    let x = (p * 1000.0 + k as f32 * 17.13).sin() * 43_758.547;
    x - x.floor()
}

/// Draws a plant of `sp` with `look` at `at` into `m` in `detail`: from
/// its model file with `OCW_FLORA=files` where the season keeps the file's
/// shape (spec §14 Phase 15, M1; [`crate::flora_files`]), else from the
/// generator ([`generate`]).
pub fn plant(m: &mut MeshArrays, sp: Species, at: &PlantAt, look: &PlantLook, detail: Detail) {
    if crate::flora_files::try_draw(m, sp, at, look, detail) {
        return;
    }
    generate(m, sp, at, look, detail);
}

/// The generator of the plants (what the files in
/// `artwork/3dmodels/flora/` were exported from).
pub fn generate(m: &mut MeshArrays, sp: Species, at: &PlantAt, look: &PlantLook, detail: Detail) {
    use Species::*;
    let s = at.scale;
    let base = at.base;
    let near = detail == Detail::Near;
    let crown = crown_colour(sp, look, at.shade, at.hue);
    let bark = bark_colour(sp);
    let h = sp.height_m();
    let r = sp.crown_radius_m();
    let seg = if near { 7 } else { 5 };
    let turn = at.turn;
    let foot = base.y;
    let bare = leaf_type(sp) != LeafType::Evergreen && look.leaves < 0.12;
    match sp {
        NorwaySpruce | SilverFir | DouglasFir | Larch => {
            let trunk_top = if near { 0.55 * h } else { 0.3 * h };
            tube(
                m,
                base,
                base + Vec3::Y * trunk_top * s,
                sp.trunk_radius_m() * s,
                0.006 * h * s,
                if near { 5 } else { 4 },
                foot,
                bark,
            );
            let (start, flat_top) = match sp {
                SilverFir => (0.16, at.pick > 0.55),
                DouglasFir => (0.2, false),
                Larch => (0.2, false),
                _ => (0.1, false),
            };
            let colour = if sp == Larch && bare { twig() } else { crown };
            let larch_k = if sp == Larch {
                0.45 + 0.55 * look.leaves.max(0.15)
            } else {
                1.0
            };
            if !near {
                let top = if flat_top { 0.94 } else { 1.0 };
                // The flat underside and the cone, each with its own rim.
                two_sided_cone(
                    m,
                    base,
                    s,
                    &[(start * h, 0.0), (start * h, r * larch_k)],
                    &[(start * h, r * larch_k), (top * h, 0.0)],
                    seg,
                    colour,
                    turn,
                );
                return;
            }
            let tiers: u32 = match sp {
                DouglasFir | Larch => 5,
                _ => 6,
            };
            let droop = match sp {
                NorwaySpruce => 0.42,
                SilverFir => 0.18,
                _ => 0.3,
            };
            let span = (1.0 - start) * h;
            for i in 0..tiers {
                // From the bottom tier up, each narrower; they overlap.
                let t0 = i as f32 / tiers as f32;
                let bottom = start * h + span * t0 * 0.92;
                let top = (bottom + span * (1.7 / tiers as f32)).min(h);
                let mut rr = r * larch_k * (1.0 - 0.82 * t0) * (0.9 + 0.2 * h01(at.pick, i));
                let mut top_y = top;
                if flat_top && i == tiers - 1 {
                    // The old fir's flat "stork's nest" top.
                    rr = r * 0.45;
                    top_y = bottom + (top - bottom) * 0.45;
                }
                tier(
                    m,
                    base,
                    s,
                    (bottom, top_y, rr),
                    droop,
                    seg,
                    colour,
                    turn + i as f32 * 0.7,
                );
            }
        }
        ScotsPine | CanaryPine => {
            // A tall bare trunk, the crown of tufts up high.
            let trunk_top = if sp == CanaryPine { 0.92 } else { 0.85 };
            tube(
                m,
                base,
                base + Vec3::new(0.3 * (at.pick - 0.5), trunk_top * h, 0.0) * s,
                sp.trunk_radius_m() * s,
                0.006 * h * s,
                if near { 6 } else { 4 },
                foot,
                bark,
            );
            if !near {
                blob(
                    m,
                    base,
                    Vec3::Y * 0.78 * h,
                    Vec3::new(r, 0.2 * h, r),
                    s,
                    seg,
                    2,
                    crown,
                    turn,
                );
                return;
            }
            let tufts = if sp == CanaryPine { 9 } else { 5 };
            for i in 0..tufts {
                let a = turn + i as f32 * 2.39;
                let k = i as f32 / tufts as f32;
                let y = if sp == CanaryPine {
                    (0.4 + 0.55 * k) * h
                } else {
                    (0.55 + 0.4 * k) * h
                };
                let reach = r * (0.75 - 0.45 * k) * (0.7 + 0.5 * h01(at.pick, i));
                let c = Vec3::new(reach * a.cos(), y, reach * a.sin());
                tube(
                    m,
                    base + Vec3::Y * (y - 0.6) * s,
                    base + c * s,
                    0.1 * s,
                    0.05 * s,
                    3,
                    foot,
                    bark,
                );
                if sp == CanaryPine {
                    // Long hanging needles: a drooping tuft.
                    let rr = r * (0.45 - 0.15 * k);
                    lathe(
                        m,
                        base,
                        c - Vec3::Y * 1.6,
                        s,
                        &[
                            (0.0, 0.0),
                            (0.2, rr * 0.8),
                            (1.0, rr),
                            (1.9, rr * 0.55),
                            (2.3, 0.0),
                        ],
                        6,
                        shade3(crown, 0.92 + 0.12 * h01(at.pick, i + 9)),
                        a,
                        (1.0, 1.0),
                    );
                } else {
                    blob(
                        m,
                        base,
                        c,
                        Vec3::new(r * 0.5, 0.11 * h, r * 0.5),
                        s,
                        6,
                        2,
                        crown,
                        a,
                    );
                }
            }
        }
        MountainPine | GreenAlder | Tabaiba | Broom => {
            // Low shrubs of a few domes.
            let n = match (sp, near) {
                (_, false) => 1,
                (Broom, _) => 1,
                (Tabaiba, _) => 3,
                _ => 4,
            };
            if sp == Tabaiba && near {
                for i in 0..5 {
                    let a = turn + i as f32 * 1.26;
                    tube(
                        m,
                        base,
                        base + Vec3::new(0.5 * r * a.cos(), 0.75 * h, 0.5 * r * a.sin()) * s,
                        0.05 * s,
                        0.03 * s,
                        3,
                        foot,
                        bark,
                    );
                }
            }
            let leafy = if sp == Tabaiba {
                1.0 - 0.6 * look.dry
            } else {
                1.0
            };
            for i in 0..n {
                let a = turn + i as f32 * 2.1;
                let off = if n == 1 { 0.0 } else { 0.45 * r };
                let (rx, ry) = match sp {
                    Broom => (r, 0.62 * h),
                    _ => (r * 0.62 * leafy, 0.5 * h),
                };
                let y = if sp == Broom { 0.25 * h } else { 0.52 * h };
                blob(
                    m,
                    base,
                    Vec3::new(off * a.cos(), y, off * a.sin()),
                    Vec3::new(rx, ry, rx * 0.9),
                    s,
                    if near { 7 } else { 5 },
                    if near { 3 } else { 1 },
                    shade3(crown, 0.92 + 0.15 * h01(at.pick, i)),
                    a,
                );
            }
        }
        Cardon => {
            let n = if near { 9 } else { 3 };
            for i in 0..n {
                let a = turn + i as f32 * 2.39;
                let d = if i == 0 {
                    0.0
                } else {
                    r * (0.3 + 0.6 * h01(at.pick, i))
                };
                let hh = h * (0.55 + 0.45 * h01(at.pick, i + 20));
                let p = base + Vec3::new(d * a.cos(), 0.0, d * a.sin()) * s;
                lathe(
                    m,
                    p,
                    Vec3::ZERO,
                    s,
                    &[(0.0, 0.0), (0.0, 0.16), (hh * 0.9, 0.15), (hh, 0.0)],
                    5,
                    crown,
                    a,
                    (1.0, 1.0),
                );
            }
        }
        PricklyPear => {
            let n = if near { 7 } else { 2 };
            for i in 0..n {
                let a = turn + i as f32 * 1.9;
                let y = 0.3 + (i as f32 / n as f32) * 0.6 * h;
                let d = 0.4 * r * h01(at.pick, i);
                blob(
                    m,
                    base,
                    Vec3::new(d * a.cos(), y, d * a.sin()),
                    Vec3::new(0.32, 0.38, 0.06),
                    s * 1.2,
                    6,
                    1,
                    crown,
                    a,
                );
            }
        }
        Agave => {
            let n = if near { 14 } else { 5 };
            for i in 0..n {
                let a = turn + i as f32 * 2.39;
                let dir = Vec3::new(a.cos(), 0.0, a.sin());
                let up = 0.6 + 0.9 * (i % 3) as f32 / 2.0;
                blade(
                    m,
                    base + Vec3::Y * 0.1,
                    dir,
                    up,
                    1.0 * s,
                    0.22 * s,
                    0.15 * s,
                    if near { 3 } else { 1 },
                    foot,
                    crown,
                );
            }
            if near && at.pick > 0.85 {
                // The flower stalk, once in a lifetime.
                tube(
                    m,
                    base,
                    base + Vec3::Y * 5.5 * s,
                    0.08 * s,
                    0.03 * s,
                    4,
                    foot,
                    lin3(0.45, 0.40, 0.25),
                );
            }
        }
        DatePalm | SlenderPalm => {
            let slender = sp == SlenderPalm;
            let lean = Vec3::new(0.04 * (at.pick - 0.5) * h, 0.0, 0.04 * (at.hue - 0.5) * h);
            let top = base + (Vec3::Y * h + lean) * s;
            let (r0, r1) = (sp.trunk_radius_m(), if slender { 0.16 } else { 0.4 });
            tube(
                m,
                base,
                top,
                r0 * s,
                r1 * s,
                if near { 7 } else { 4 },
                foot,
                bark,
            );
            // The crown: fronds radiating and arching.
            let fronds = match (slender, near) {
                (_, false) => 6,
                (true, true) => 12,
                (false, true) => 26,
            };
            let len = if slender { 2.6 } else { 4.4 };
            for i in 0..fronds {
                let a = turn + i as f32 * 2.39996;
                let tier = (i % 3) as f32;
                let dir = Vec3::new(a.cos(), 0.0, a.sin());
                let up = 0.9 - 0.45 * tier;
                let colour = if !slender && i >= fronds - 2 && near {
                    lin3(0.55, 0.46, 0.28)
                } else {
                    shade3(crown, 0.9 + 0.2 * h01(at.pick, i))
                };
                blade(
                    m,
                    top + Vec3::Y * 0.1 * s,
                    dir,
                    up,
                    len * s,
                    if slender { 0.9 } else { 1.15 } * s,
                    (1.4 + 0.8 * tier) * s,
                    if near { 4 } else { 2 },
                    foot,
                    colour,
                );
            }
            if !slender && near {
                // The pineapple-like knob under the crown.
                blob(
                    m,
                    base,
                    Vec3::Y * h + lean,
                    Vec3::new(0.65, 0.5, 0.65),
                    s,
                    6,
                    2,
                    lin3(0.40, 0.34, 0.22),
                    turn,
                );
            }
        }
        DragonTree => {
            let fork = 0.45 * h;
            let top = base + Vec3::Y * fork * s;
            tube(
                m,
                base,
                top,
                sp.trunk_radius_m() * s,
                0.4 * s,
                if near { 7 } else { 5 },
                foot,
                bark,
            );
            let branches = if near { 5 } else { 3 };
            for i in 0..branches {
                let a = turn + i as f32 * std::f32::consts::TAU / branches as f32;
                let d = Vec3::new(a.cos(), 0.0, a.sin());
                let mid = top + (d * 0.4 * r + Vec3::Y * 0.22 * h) * s;
                tube(m, top, mid, 0.3 * s, 0.22 * s, 4, foot, bark);
                let ends: &[f32] = if near { &[-0.45, 0.45] } else { &[0.0] };
                for &e in ends {
                    let d2 = Vec3::new((a + e).cos(), 0.0, (a + e).sin());
                    let tip = mid + (d2 * 0.55 * r + Vec3::Y * 0.2 * h) * s;
                    tube(m, mid, tip, 0.2 * s, 0.14 * s, 4, foot, bark);
                    // A tuft of stiff blue-green leaves at the tip.
                    blob(
                        m,
                        base,
                        (tip - base) / s + Vec3::Y * 0.25,
                        Vec3::new(0.9, 0.45, 0.9),
                        s,
                        if near { 6 } else { 4 },
                        1,
                        crown,
                        a,
                    );
                }
            }
        }
        Banana => {
            // Banana plants are only drawn near (plantations).
            let top = base + Vec3::Y * 0.6 * h * s;
            tube(m, base, top, 0.14 * s, 0.1 * s, 4, foot, bark);
            let n = if near { 6 } else { 4 };
            for i in 0..n {
                let a = turn + i as f32 * 2.39996;
                let dir = Vec3::new(a.cos(), 0.0, a.sin());
                let colour = if i == n - 1 {
                    // An old torn leaf.
                    lin3(0.52, 0.48, 0.24)
                } else {
                    shade3(crown, 0.9 + 0.2 * h01(at.pick, i))
                };
                blade(
                    m,
                    top,
                    dir,
                    0.75 - 0.2 * (i % 3) as f32,
                    2.2 * s,
                    0.95 * s,
                    1.1 * s,
                    2,
                    foot,
                    colour,
                );
            }
        }
        _ => broadleaf(m, sp, at, look, detail, crown, bark),
    }
}

/// The crown shape of a broadleaf: (trunk height, crown centre, radii
/// x / y, number of lumps, lump spread) as shares of the height and the
/// crown radius.
fn broadleaf_shape(sp: Species) -> (f32, f32, f32, f32, u32, f32) {
    use Species::*;
    match sp {
        Beech => (0.3, 0.62, 1.0, 0.36, 4, 0.45),
        SycamoreMaple => (0.32, 0.62, 0.95, 0.36, 4, 0.4),
        Ash => (0.38, 0.66, 0.9, 0.3, 5, 0.6),
        Oak => (0.25, 0.6, 1.0, 0.32, 5, 0.6),
        Alder => (0.2, 0.58, 0.7, 0.42, 3, 0.25),
        Willow => (0.22, 0.58, 1.0, 0.38, 4, 0.45),
        Birch => (0.3, 0.64, 0.75, 0.38, 3, 0.35),
        Unknown | Lime => (0.25, 0.6, 0.95, 0.42, 3, 0.3),
        FruitTree => (0.25, 0.6, 1.0, 0.38, 3, 0.4),
        Chestnut => (0.25, 0.58, 1.0, 0.38, 4, 0.45),
        Laurel => (0.12, 0.55, 1.0, 0.45, 5, 0.5),
        FayaHeath => (0.15, 0.58, 0.9, 0.42, 4, 0.5),
        Eucalyptus => (0.5, 0.75, 0.85, 0.24, 5, 0.6),
        SubtropicalFruit => (0.1, 0.5, 1.0, 0.48, 3, 0.35),
        _ => (0.25, 0.6, 1.0, 0.38, 3, 0.4),
    }
}

/// A broadleaf tree: a trunk and a lumpy crown that grows with its
/// leaves; bare (and while thin) its branches show.
fn broadleaf(
    m: &mut MeshArrays,
    sp: Species,
    at: &PlantAt,
    look: &PlantLook,
    detail: Detail,
    crown: [f32; 3],
    bark: [f32; 3],
) {
    let s = at.scale;
    let base = at.base;
    let foot = base.y;
    let near = detail == Detail::Near;
    let h = sp.height_m();
    let r = sp.crown_radius_m();
    let (trunk, centre, rx, ry, lumps, spread) = broadleaf_shape(sp);
    let evergreen = leaf_type(sp) == LeafType::Evergreen;
    let leaves = if evergreen { 1.0 } else { look.leaves };
    let multi = matches!(sp, Species::Willow | Species::Laurel | Species::FayaHeath);
    let stems = if multi && near { 3 } else { 1 };
    let cy = centre * h;
    for k in 0..stems {
        let a = at.turn + k as f32 * 2.1;
        let lean = if stems > 1 {
            Vec3::new(0.35 * r * a.cos(), 0.0, 0.35 * r * a.sin())
        } else {
            Vec3::ZERO
        };
        tube(
            m,
            base,
            base + (Vec3::Y * (cy * 0.9) + lean) * s,
            // One stem: `Species::trunk_radius_m` (the collisions' trunk).
            0.02 * h * s / stems as f32 + 0.05 * s,
            0.008 * h * s,
            if near { 6 } else { 4 },
            foot,
            bark,
        );
    }
    if leaves < 0.5 {
        // The branches of a bare or thin crown.
        let n = if near { 7 } else { 3 };
        for k in 0..n {
            let a = at.turn + k as f32 * 2.4;
            let y0 = (trunk + (centre - trunk) * k as f32 / n as f32) * h;
            let reach = rx * r * (0.9 - 0.3 * k as f32 / n as f32);
            let from = base + Vec3::Y * y0 * s;
            let to = base + Vec3::new(reach * a.cos(), y0 + ry * h * 1.1, reach * a.sin()) * s;
            tube(m, from, to, 0.012 * h * s, 0.003 * h * s, 3, foot, twig());
        }
    }
    // The crown is the same in both details (spec 4.6.1, the bug of
    // 2026-11-23: the trees turned bald when coming closer – the far model
    // kept a grey crown of twigs that the near one did not have): the same
    // form, size, colour and opacity by [`crown_form`], only the detail of
    // the mesh differs.
    let (k, opacity) = match crown_form(sp, look) {
        CrownForm::Twigs { opacity } => {
            // The fine twigs of a bare crown: a see-through grey-brown haze
            // around the branches (dithered by the tree material).
            let from = m.colors.len();
            blob(
                m,
                base,
                Vec3::Y * cy,
                Vec3::new(rx * r * TWIG_CROWN, ry * h * 0.9, rx * r * TWIG_CROWN),
                s,
                if near { 7 } else { 5 },
                if near { 3 } else { 1 },
                mix3(twig(), crown, leaves * 2.0),
                at.turn,
            );
            set_opacity(m, from, opacity);
            return;
        }
        CrownForm::Leaves { size, opacity } => (size, opacity),
    };
    let from = m.colors.len();
    if !near {
        blob(
            m,
            base,
            Vec3::Y * cy,
            Vec3::new(rx * r * k, ry * h * k, rx * r * k),
            s,
            6,
            2,
            crown,
            at.turn,
        );
        set_opacity(m, from, opacity);
        return;
    }
    // The main crown and its lumps.
    blob(
        m,
        base,
        Vec3::Y * cy,
        Vec3::new(rx * r * k * 0.8, ry * h * k * 0.85, rx * r * k * 0.8),
        s,
        7,
        3,
        crown,
        at.turn,
    );
    for i in 0..lumps {
        let a = at.turn + i as f32 * 2.39996;
        let d = spread * r * k;
        let up = (h01(at.pick, i) - 0.4) * ry * h * 0.8;
        let lr = (0.45 + 0.15 * h01(at.pick, i + 7)) * r * k;
        blob(
            m,
            base,
            Vec3::new(d * a.cos(), cy + up, d * a.sin()),
            Vec3::new(lr, lr * 0.85, lr),
            s,
            6,
            2,
            shade3(crown, 0.9 + 0.2 * h01(at.pick, i + 3)),
            a,
        );
    }
    set_opacity(m, from, opacity);
}

/// Below this many leaves a deciduous crown is bare: only its twigs.
pub const BARE_LEAVES: f32 = 0.12;
/// The bare crown of twigs: its width (share of the crown radius) and how
/// much of it covers what is behind (the rest shows through, dithered).
pub const TWIG_CROWN: f32 = 0.85;
pub const TWIG_OPACITY: f32 = 0.45;

/// The crown of a broadleaf on the day, the same in both details.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CrownForm {
    /// Bare: the haze of its twigs.
    Twigs { opacity: f32 },
    /// In leaf: the crown at `size` (share of the full crown), thinning
    /// crowns more see-through.
    Leaves { size: f32, opacity: f32 },
}

impl CrownForm {
    pub fn opacity(self) -> f32 {
        match self {
            CrownForm::Twigs { opacity } | CrownForm::Leaves { opacity, .. } => opacity,
        }
    }
}

/// The crown of a broadleaf of `sp` with `look` (both details draw it).
pub fn crown_form(sp: Species, look: &PlantLook) -> CrownForm {
    let leaves = if leaf_type(sp) == LeafType::Evergreen {
        1.0
    } else {
        look.leaves
    };
    let blossom = sp == Species::FruitTree && look.bloom > 0.3;
    if leaves < BARE_LEAVES && !blossom {
        return CrownForm::Twigs {
            opacity: TWIG_OPACITY,
        };
    }
    // Blossom before the leaves: the crown at least half grown.
    let size = (0.55 + 0.45 * leaves).max(if blossom { 0.8 } else { 0.0 });
    let opacity = if blossom {
        1.0
    } else {
        (0.35 + 1.3 * leaves).min(1.0)
    };
    CrownForm::Leaves { size, opacity }
}

/// Sets the opacity (vertex alpha: the tree material leaves that share of
/// the pixels, `see_through.wesl`) of the vertices from `from` on.
fn set_opacity(m: &mut MeshArrays, from: usize, opacity: f32) {
    if opacity >= 1.0 {
        return;
    }
    for c in &mut m.colors[from..] {
        c[3] = opacity;
    }
}

/// The conifers' canopy from afar (sRGB) in `region`: spruce and fir
/// (`far_terrain::CANOPY`), the Canary pine's open, lighter crowns on
/// Tenerife.
pub fn far_conifers(region: ocw_world::flora::FloraRegion) -> [f32; 3] {
    match region {
        ocw_world::flora::FloraRegion::Tenerife => [0.17, 0.27, 0.12],
        _ => crate::far_terrain::CANOPY,
    }
}

/// The top of a plant's crown and its radius at scale `scale` (m): the
/// flight's obstacles (spec §14 Phase 13B).
pub fn crown_top(sp: Species, scale: f32) -> (f32, f32) {
    (sp.height_m() * scale, sp.crown_radius_m() * scale)
}

// --- Rows and stands ----------------------------------------------------------

/// The look of a vineyard on the day: leaves (0 bare canes – 1 full) and
/// autumn colour (yellow, red for the red varieties).
pub fn vine_look(day: Option<&SeasonDay>, alt_m: f64, jitter: f32) -> PlantLook {
    match day {
        None => PlantLook::SUMMER,
        Some(d) => {
            // Vines break bud late (late April) and turn in October.
            let f = d.foliage_timed(
                LeafType::Deciduous,
                alt_m,
                jitter,
                Timing {
                    spring: 1.6,
                    autumn: 1.0,
                },
            );
            PlantLook {
                leaves: f.leaves,
                fresh: f.fresh,
                autumn: f.autumn,
                bloom: 0.0,
                dry: 0.0,
            }
        }
    }
}

/// The colour of a vine row (linear): green, yellow or red in autumn
/// (`red`: a red variety such as the Blauburgunder of Vaduz).
pub fn vine_colour(look: &PlantLook, red: bool, shade: f32) -> [f32; 3] {
    let green = lin3(0.30, 0.45, 0.17);
    // Red varieties turn from yellow-orange to deep red, white ones yellow;
    // never all the same: `shade` (the piece's own) holds some back.
    let autumn = if red {
        mix3(
            lin3(0.72, 0.45, 0.14),
            lin3(0.46, 0.12, 0.08),
            0.4 + 0.6 * shade,
        )
    } else {
        mix3(lin3(0.78, 0.66, 0.20), lin3(0.62, 0.55, 0.18), shade)
    };
    let c = mix3(green, lin3(0.46, 0.62, 0.24), look.fresh * 0.6);
    let t = look.autumn * (0.55 + 0.45 * shade);
    shade3(mix3(c, autumn, t), 0.88 + 0.24 * shade)
}

/// A row of vines on a wire trellis from `a` to `b` (feet on the ground,
/// package metres): posts every 5 m, the canopy a narrow wall (`height`
/// m high, its foliage from 0.6 m) that grows with the leaves; bare: the
/// canes along the wire.
pub fn vine_row(
    m: &mut MeshArrays,
    a: Vec3,
    b: Vec3,
    height: f32,
    look: &PlantLook,
    colour: [f32; 3],
) {
    let post = lin3(0.40, 0.36, 0.30);
    let len = (b - a).length();
    let dir = (b - a).normalize_or(Vec3::X);
    let side = Vec3::Y.cross(dir).normalize_or(Vec3::Z);
    let posts = (len / 5.0).floor() as u32 + 1;
    for i in 0..posts {
        let p = a + (b - a) * (i as f32 / (posts - 1).max(1) as f32);
        tube(
            m,
            p - Vec3::Y * 0.2,
            p + Vec3::Y * height,
            0.04,
            0.04,
            3,
            p.y,
            post,
        );
    }
    let leaves = look.leaves;
    let (lo, hi, half) = if leaves < 0.12 {
        // The canes on the wire.
        (0.75, 0.9, 0.06)
    } else {
        (
            0.55,
            0.6 + (height - 0.6) * leaves.max(0.3),
            0.22 + 0.18 * leaves,
        )
    };
    let c = if leaves < 0.12 {
        lin3(0.38, 0.31, 0.24)
    } else {
        colour
    };
    // A box along the row with its feet following the ground (the ends'
    // heights), split into 5 m pieces.
    let pieces = (len / 5.0).ceil().max(1.0) as u32;
    // Each piece's own height (the canopy's ragged top) and shade.
    let jag = |i: u32, k: u32| h01(a.x * 0.37 + b.z * 0.11 + i as f32 * 0.013, k);
    let mid = (lo + hi) / 2.0;
    for i in 0..pieces {
        let (t0, t1) = (i as f32 / pieces as f32, (i + 1) as f32 / pieces as f32);
        let (p0, p1) = (a + (b - a) * t0, a + (b - a) * t1);
        let top = |j: u32| hi - (hi - mid) * 0.25 * jag(j, 1);
        let rgba_c = rgba(shade3(c, 0.9 + 0.2 * jag(i, 2)));
        // The cross-section: foot, middle (widest), top on each side.
        let ring = |p: Vec3, j: u32| {
            [
                p + side * half * 0.55 + Vec3::Y * lo,
                p + side * half + Vec3::Y * mid,
                p + side * half * 0.45 + Vec3::Y * top(j),
                p - side * half * 0.45 + Vec3::Y * top(j),
                p - side * half + Vec3::Y * mid,
                p - side * half * 0.55 + Vec3::Y * lo,
            ]
        };
        let (q0, q1) = (ring(p0, i), ring(p1, i + 1));
        // Six faces around (the ends are open: rows continue).
        for k in 0..6usize {
            let (i0, i1) = (k, (k + 1) % 6);
            let n = {
                let e = q0[i1] - q0[i0];
                e.cross(dir).normalize_or(Vec3::Y)
            };
            let start = m.positions.len() as u32;
            for p in [q0[i0], q0[i1], q1[i0], q1[i1]] {
                m.positions.push(p.to_array());
                m.uvs.push([(p.y - a.y.min(b.y)).max(0.0), 0.0]);
                m.normals.push(n.to_array());
                m.colors.push(rgba_c);
            }
            m.indices
                .extend([start, start + 2, start + 1, start + 1, start + 2, start + 3]);
        }
    }
}

/// A field's stand (maize, cereal, sunflower, potato, rapeseed) over one
/// cell of `size` m at `centre`: its feet at the ground heights `ys` of the
/// four corners (x−z−, x+z−, x−z+, x+z+) and of the centre, its top `tops`
/// m above the ground at those five points (a bumpy canopy: neighbouring
/// cells share their corners), each top vertex shaded by `shades`; its
/// sides only where `open[side]` (x−, x+, z−, z+: the neighbour is not the
/// same stand), darker towards the ground.
///
/// The top is a fan of four triangles around the centre, so it follows the
/// ground wherever the ground is two triangles split along the x+z−/x−z+
/// diagonal with the centre on it: the rendered terrain's cells
/// (`package::terrain_arrays`) when the stand's cell is one of them and
/// `ys` are the rendered ground ([`crate::package::TileGround`]). Before
/// 2026-11-17 the centre stood at the corners' mean: on a terrace edge up
/// to 7 m above the drawn ground, a green slab floating over it. The top's
/// normals are the ground's, so a stand on a slope is lit like the slope.
#[allow(clippy::too_many_arguments)]
pub fn stand_cell(
    m: &mut MeshArrays,
    centre: Vec3,
    size: f32,
    ys: [f32; 5],
    tops: [f32; 5],
    shades: [f32; 5],
    open: [bool; 4],
    colour: [f32; 3],
) {
    let hs = size / 2.0;
    let xz = [[-hs, -hs], [hs, -hs], [-hs, hs], [hs, hs]];
    let top: Vec<Vec3> = (0..4)
        .map(|i| Vec3::new(centre.x + xz[i][0], ys[i] + tops[i], centre.z + xz[i][1]))
        .collect();
    let mid = Vec3::new(centre.x, ys[4] + tops[4], centre.z);
    let foot: Vec<Vec3> = (0..4)
        .map(|i| Vec3::new(centre.x + xz[i][0], ys[i] - 0.1, centre.z + xz[i][1]))
        .collect();
    // The ground's normals: each face of the ground's fan (centre, a, b)
    // adds its normal to its three vertices.
    let ground: Vec<Vec3> = (0..4)
        .map(|i| Vec3::new(xz[i][0], ys[i], xz[i][1]))
        .chain(std::iter::once(Vec3::new(0.0, ys[4], 0.0)))
        .collect();
    let mut normals = [Vec3::ZERO; 5];
    for (a, b) in [(1usize, 0usize), (3, 1), (2, 3), (0, 2)] {
        let n = (ground[a] - ground[4])
            .cross(ground[b] - ground[4])
            .normalize_or(Vec3::Y);
        for v in [a, b, 4] {
            normals[v] += n;
        }
    }
    // The top: a fan of four triangles around the centre.
    let start = m.positions.len() as u32;
    for (k, p) in top.iter().chain(std::iter::once(&mid)).enumerate() {
        m.positions.push(p.to_array());
        m.uvs.push([tops[k], 0.0]);
        m.normals.push(normals[k].normalize_or(Vec3::Y).to_array());
        m.colors.push(rgba(shade3(colour, shades[k])));
    }
    let c = start + 4;
    m.indices.extend([
        c,
        start + 1,
        start,
        c,
        start + 3,
        start + 1,
        c,
        start + 2,
        start + 3,
        c,
        start,
        start + 2,
    ]);
    // Sides: (corner a, corner b, outward normal).
    let sides = [
        (2usize, 0usize, Vec3::NEG_X),
        (1, 3, Vec3::X),
        (0, 1, Vec3::NEG_Z),
        (3, 2, Vec3::Z),
    ];
    let low = rgba(shade3(colour, 0.5));
    for (k, (ia, ib, n)) in sides.iter().enumerate() {
        if !open[k] {
            continue;
        }
        let s = m.positions.len() as u32;
        for (p, y, c) in [
            (foot[*ia], 0.0, low),
            (foot[*ib], 0.0, low),
            (top[*ia], tops[*ia], rgba(shade3(colour, 0.8 * shades[*ia]))),
            (top[*ib], tops[*ib], rgba(shade3(colour, 0.8 * shades[*ib]))),
        ] {
            m.positions.push(p.to_array());
            m.uvs.push([y, 0.0]);
            m.normals.push(n.to_array());
            m.colors.push(c);
        }
        m.indices.extend([s, s + 1, s + 2, s + 1, s + 3, s + 2]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn at(scale: f32) -> PlantAt {
        PlantAt {
            base: Vec3::new(10.0, 100.0, 20.0),
            scale,
            turn: 0.4,
            shade: 0.5,
            hue: 0.3,
            pick: 0.6,
        }
    }

    fn check(m: &MeshArrays) {
        let n = m.positions.len();
        assert_eq!(m.normals.len(), n);
        assert_eq!(m.colors.len(), n);
        assert_eq!(m.uvs.len(), n);
        assert_eq!(m.indices.len() % 3, 0);
        assert!(m.indices.iter().all(|i| (*i as usize) < n));
        for (p, q) in m.positions.iter().zip(&m.normals) {
            assert!(p.iter().all(|v| v.is_finite()));
            let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt();
            assert!((l - 1.0).abs() < 1e-3, "normal {q:?}");
        }
    }

    /// Every edge of a closed mesh is shared by exactly two triangles,
    /// once in each direction.
    fn closed(m: &MeshArrays) -> bool {
        let mut edges: HashMap<(u32, u32), i32> = HashMap::new();
        for t in m.indices.chunks(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                *edges.entry((a, b)).or_default() += 1;
            }
        }
        edges
            .iter()
            .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1))
    }

    #[test]
    fn primitives_are_closed_and_outward() {
        let mut m = MeshArrays::default();
        lathe(
            &mut m,
            Vec3::ZERO,
            Vec3::ZERO,
            1.0,
            &[(0.0, 0.0), (0.0, 2.0), (3.0, 1.0), (5.0, 0.0)],
            7,
            [0.2; 3],
            0.0,
            (1.0, 1.0),
        );
        check(&m);
        assert!(closed(&m));
        // Outward: each triangle's normal points away from the axis or up/down.
        for t in m.indices.chunks(3) {
            let p: Vec<Vec3> = t
                .iter()
                .map(|i| Vec3::from(m.positions[*i as usize]))
                .collect();
            let n = (p[1] - p[0]).cross(p[2] - p[0]);
            let c = (p[0] + p[1] + p[2]) / 3.0;
            let out = Vec3::new(c.x, c.y - 2.5, c.z);
            assert!(n.dot(out) > 0.0, "inward triangle {p:?}");
        }
        let mut m = MeshArrays::default();
        tube(
            &mut m,
            Vec3::ZERO,
            Vec3::new(0.5, 4.0, 0.2),
            0.3,
            0.1,
            5,
            0.0,
            [0.3; 3],
        );
        check(&m);
        assert!(closed(&m));
        let mut m = MeshArrays::default();
        blob(
            &mut m,
            Vec3::ZERO,
            Vec3::Y * 3.0,
            Vec3::new(2.0, 1.0, 1.5),
            1.0,
            6,
            3,
            [0.2; 3],
            0.0,
        );
        check(&m);
        assert!(closed(&m));
    }

    #[test]
    fn every_species_has_both_details_within_budget() {
        for sp in Species::ALL {
            let mut near = MeshArrays::default();
            plant(&mut near, sp, &at(1.0), &PlantLook::SUMMER, Detail::Near);
            let mut far = MeshArrays::default();
            plant(&mut far, sp, &at(1.0), &PlantLook::SUMMER, Detail::Far);
            check(&near);
            check(&far);
            let (tn, tf) = (near.indices.len() / 3, far.indices.len() / 3);
            assert!(tn > 0 && tf > 0, "{sp:?}");
            assert!(tn <= 900, "{sp:?}: {tn} near triangles");
            assert!(tf <= 160, "{sp:?}: {tf} far triangles");
            assert!(tf < tn || tn < 40, "{sp:?}: far {tf} not below near {tn}");
            // The plant stands at its foot and about as tall as the species.
            let top = near.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max) - 100.0;
            let want = sp.height_m();
            assert!(
                top > want * 0.6 && top < want * 1.45 + 1.0,
                "{sp:?}: top {top}, height {want}"
            );
            // The sway: uv.x is the height above the foot.
            for (p, uv) in near.positions.iter().zip(&near.uvs) {
                assert!(
                    (uv[0] - (p[1] - 100.0)).abs() < 0.6,
                    "{sp:?}: {uv:?} at {p:?}"
                );
            }
            // Deterministic.
            let mut again = MeshArrays::default();
            plant(&mut again, sp, &at(1.0), &PlantLook::SUMMER, Detail::Near);
            assert_eq!(again.positions, near.positions);
        }
    }

    #[test]
    fn scale_and_shapes_differ_by_species() {
        let width = |sp: Species| {
            let mut m = MeshArrays::default();
            plant(&mut m, sp, &at(1.0), &PlantLook::SUMMER, Detail::Near);
            let xs = m.positions.iter().map(|p| p[0]);
            let (lo, hi) = xs.fold((f32::MAX, f32::MIN), |(l, h), x| (l.min(x), h.max(x)));
            hi - lo
        };
        // A spruce is narrower than a fir and much narrower than a beech.
        assert!(width(Species::NorwaySpruce) < width(Species::SilverFir) + 0.5);
        assert!(width(Species::NorwaySpruce) < width(Species::Beech) * 0.7);
        // Bigger scale: bigger plant.
        let mut a = MeshArrays::default();
        plant(
            &mut a,
            Species::Beech,
            &at(1.4),
            &PlantLook::SUMMER,
            Detail::Near,
        );
        let top = a.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max) - 100.0;
        assert!(top > Species::Beech.height_m() * 1.1);
    }

    /// Regression (2026-11-23, "the trees turn bald all of a sudden when
    /// getting closer"): a tree has the same leaf state in its near and
    /// its far model on every day – in May, July, late October and late
    /// November, in the valley and up high: the same crown form, opacity,
    /// height and colour (the far model kept a grey crown of twigs that the
    /// near model did not have, and a thin autumn crown was a grey haze
    /// from afar but a coloured crown up close).
    #[test]
    fn near_and_far_models_have_the_same_leaves_every_season() {
        let li = ocw_sky::season::SeasonModel::for_place(47.14, 9.52);
        let bf = ocw_sky::season::SeasonModel::for_place(48.6, 8.2);
        // The crown: every vertex that is not trunk or branch (bark or
        // twig colour at full opacity).
        let crown = |sp: Species, m: &MeshArrays| -> (f32, f32, [f32; 3], usize) {
            let same = |c: &[f32; 4], d: [f32; 3]| (0..3).all(|i| (c[i] - d[i]).abs() < 1e-5);
            let parts: Vec<(&[f32; 3], &[f32; 4])> = m
                .positions
                .iter()
                .zip(&m.colors)
                .filter(|(_, c)| c[3] < 1.0 || !(same(c, twig()) || same(c, bark_colour(sp))))
                .collect();
            let top = parts.iter().map(|(p, _)| p[1]).fold(0.0f32, f32::max) - 100.0;
            let alpha = parts.iter().map(|(_, c)| c[3]).fold(1.0f32, f32::min);
            let n = parts.len().max(1) as f32;
            let colour = std::array::from_fn(|i| parts.iter().map(|(_, c)| c[i]).sum::<f32>() / n);
            (top, alpha, colour, parts.len())
        };
        let mut forms = std::collections::BTreeSet::new();
        for (model, alt) in [(&li, 455.0), (&li, 1300.0), (&bf, 700.0)] {
            // 10 May, 15 July, 28 October, 23 November.
            for day in [130.0, 196.0, 301.0, 327.0] {
                let d = model.day(day);
                for sp in [
                    Species::Beech,
                    Species::SycamoreMaple,
                    Species::Ash,
                    Species::Oak,
                    Species::Birch,
                    Species::Lime,
                    Species::FruitTree,
                    Species::Alder,
                    Species::Willow,
                    Species::Chestnut,
                    Species::Laurel,
                ] {
                    for jitter in [-1.0, 0.0, 1.0] {
                        let look = plant_look(Some(&d), sp, alt, jitter);
                        let form = crown_form(sp, &look);
                        forms.insert(match form {
                            CrownForm::Twigs { .. } => "bare",
                            CrownForm::Leaves { opacity, .. } if opacity < 1.0 => "thin",
                            CrownForm::Leaves { .. } => "full",
                        });
                        let mut near = MeshArrays::default();
                        plant(&mut near, sp, &at(1.0), &look, Detail::Near);
                        let mut far = MeshArrays::default();
                        plant(&mut far, sp, &at(1.0), &look, Detail::Far);
                        let (nt, na, nc, nn) = crown(sp, &near);
                        let (ft, fa, fc, fnn) = crown(sp, &far);
                        let what = format!("{sp:?} day {day} at {alt} m: {look:?}");
                        assert!(nn > 0 && fnn > 0, "{what}: a crown in both");
                        assert_eq!(na, fa, "{what}: opacity");
                        assert!((na - form.opacity()).abs() < 1e-6, "{what}");
                        assert!(
                            (nt - ft).abs() < 0.15 * nt.max(ft),
                            "{what}: top {nt} / {ft}"
                        );
                        for i in 0..3 {
                            assert!(
                                (nc[i] - fc[i]).abs() < 0.04,
                                "{what}: colour {nc:?} / {fc:?}"
                            );
                        }
                    }
                }
            }
        }
        assert!(
            forms.contains("bare") && forms.contains("full"),
            "{forms:?}"
        );
        // Late November in the valley: the beech is bare in both models,
        // a see-through crown of twigs; July: a full, opaque crown.
        let nov = plant_look(Some(&li.day(327.0)), Species::Beech, 455.0, 0.0);
        assert_eq!(
            crown_form(Species::Beech, &nov),
            CrownForm::Twigs {
                opacity: TWIG_OPACITY
            }
        );
        let jul = plant_look(Some(&li.day(196.0)), Species::Beech, 455.0, 0.0);
        assert_eq!(crown_form(Species::Beech, &jul).opacity(), 1.0);
        // Evergreen laurels never go bare.
        let lau = plant_look(Some(&li.day(20.0)), Species::Laurel, 455.0, 0.0);
        assert_eq!(crown_form(Species::Laurel, &lau).opacity(), 1.0);
    }

    #[test]
    fn bare_trees_and_seasonal_colours() {
        let bare = PlantLook {
            leaves: 0.0,
            ..PlantLook::SUMMER
        };
        let mut m = MeshArrays::default();
        plant(&mut m, Species::Beech, &at(1.0), &bare, Detail::Near);
        check(&m);
        let twig = twig();
        assert!(
            m.colors.iter().all(|c| c[0] < 0.6),
            "no crown colour when bare"
        );
        assert!(m.colors.iter().any(|c| (c[0] - twig[0]).abs() < 1e-6));
        // Spruces stay green in winter.
        let mut s = MeshArrays::default();
        plant(&mut s, Species::NorwaySpruce, &at(1.0), &bare, Detail::Near);
        let g = s.colors.iter().filter(|c| c[1] > c[0] * 1.3).count();
        assert!(g > s.colors.len() / 2);
        // Autumn: beech copper (red over blue), larch gold, maple yellow.
        let autumn = PlantLook {
            autumn: 1.0,
            ..PlantLook::SUMMER
        };
        let beech = crown_colour(Species::Beech, &autumn, 0.5, 0.3);
        let larch = crown_colour(Species::Larch, &autumn, 0.5, 0.3);
        let maple = crown_colour(Species::SycamoreMaple, &autumn, 0.5, 0.3);
        let spruce = crown_colour(Species::NorwaySpruce, &autumn, 0.5, 0.3);
        assert!(beech[0] > beech[1] && beech[0] > 3.0 * beech[2]);
        assert!(larch[0] > larch[2] * 3.0 && larch[1] > larch[2]);
        assert!(maple[1] > 0.3);
        assert!(spruce[1] > spruce[0]);
        // Blossom whitens a fruit tree; drought greys a tabaiba.
        let bloom = PlantLook {
            bloom: 1.0,
            ..PlantLook::SUMMER
        };
        let fruit = crown_colour(Species::FruitTree, &bloom, 0.5, 0.2);
        assert!(fruit.iter().all(|v| *v > 0.5), "{fruit:?}");
        let dry = PlantLook {
            dry: 1.0,
            ..PlantLook::SUMMER
        };
        let t = crown_colour(Species::Tabaiba, &dry, 0.5, 0.2);
        let g = crown_colour(Species::Tabaiba, &PlantLook::SUMMER, 0.5, 0.2);
        assert!(t[1] - t[0] < g[1] - g[0]);
    }

    /// Spec §14 Phase 9 Q13: Tenerife's forests – laurel forest,
    /// fayal-brezal, Canary pine, measured or by the region's mix – never
    /// show autumn colours or drop their leaves, at any height and on any
    /// day; their verges get no autumn litter.
    #[test]
    fn tenerifes_forests_stay_evergreen() {
        use ocw_sky::season::{CANARIES, MeasuredLeaf, TreeSetting};
        use ocw_world::flora::{FloraRegion, LeafGroup, Setting, Site, pick_species};
        use ocw_world::trees::DominantLeaf;
        let model = ocw_sky::season::SeasonModel::new(CANARIES);
        let days: Vec<_> = (1..=365)
            .step_by(8)
            .map(|d| model.day(f64::from(d)))
            .collect();
        for alt in [50.0, 400.0, 800.0, 1200.0, 1600.0, 2000.0, 2300.0] {
            for measured in [
                None,
                Some(MeasuredLeaf::Broadleaved),
                Some(MeasuredLeaf::Coniferous),
            ] {
                for windward in [true, false] {
                    for k in 0..20 {
                        let pick = (k as f32 + 0.5) / 20.0;
                        let leaf =
                            CANARIES
                                .trees
                                .leaf_type(TreeSetting::Forest, alt, measured, pick);
                        assert_eq!(leaf, LeafType::Evergreen, "{alt} {measured:?}");
                        let site = Site {
                            region: FloraRegion::Tenerife,
                            setting: Setting::Forest,
                            alt_m: alt,
                            measured: measured.map(|m| match m {
                                MeasuredLeaf::Broadleaved => DominantLeaf::Broadleaved,
                                MeasuredLeaf::Coniferous => DominantLeaf::Coniferous,
                            }),
                            windward,
                            near_water: false,
                        };
                        let sp = pick_species(&site, LeafGroup::Evergreen, pick);
                        assert_eq!(leaf_type(sp), LeafType::Evergreen, "{sp:?}");
                        for d in &days {
                            let l = plant_look(Some(d), sp, alt, pick * 2.0 - 1.0);
                            assert_eq!((l.leaves, l.autumn), (1.0, 0.0), "{sp:?} {alt}");
                        }
                    }
                }
            }
        }
        for d in &days {
            for alt in [50.0, 800.0, 1600.0] {
                let fx = crate::ground_detail::SeasonFx::of(Some(d), alt);
                // A tenth of a deciduous place's (Vaduz: up to 1).
                assert!(fx.autumn <= 0.15 && fx.fall <= 0.1 + 1e-6, "{fx:?}");
            }
        }
    }

    #[test]
    fn seasons_per_species_in_vaduz() {
        let model = ocw_sky::season::SeasonModel::for_place(47.14, 9.52);
        // Early May: birches are out before ashes.
        let may = model.day(122.0);
        let birch = plant_look(Some(&may), Species::Birch, 455.0, 0.0);
        let ash = plant_look(Some(&may), Species::Ash, 455.0, 0.0);
        assert!(birch.leaves > ash.leaves, "{birch:?} {ash:?}");
        // Mid-April: fruit trees in blossom.
        let apr = model.day(108.0);
        let fruit = plant_look(Some(&apr), Species::FruitTree, 455.0, 0.0);
        assert!(fruit.bloom > 0.5, "{fruit:?}");
        // July: no blossom; vines in full leaf; December: bare vines.
        let jul = model.day(195.0);
        assert_eq!(
            plant_look(Some(&jul), Species::FruitTree, 455.0, 0.0).bloom,
            0.0
        );
        assert!(vine_look(Some(&jul), 455.0, 0.0).leaves > 0.9);
        let dec = model.day(350.0);
        assert!(vine_look(Some(&dec), 455.0, 0.0).leaves < 0.1);
        // Late October: vines coloured.
        let oct = model.day(300.0);
        assert!(vine_look(Some(&oct), 455.0, 0.0).autumn > 0.5);
        let red = vine_colour(&vine_look(Some(&oct), 455.0, 0.0), true, 0.5);
        assert!(red[0] > red[1] * 1.5);
        // Without a season: summer.
        assert_eq!(
            plant_look(None, Species::Beech, 455.0, 0.0),
            PlantLook::SUMMER
        );
        // The Teide broom flowers in late May.
        let tf = ocw_sky::season::SeasonModel::for_place(28.27, -16.64);
        let b = plant_look(Some(&tf.day(150.0)), Species::Broom, 2100.0, 0.0);
        assert!(b.bloom > 0.9);
        assert_eq!(
            plant_look(Some(&tf.day(250.0)), Species::Broom, 2100.0, 0.0).bloom,
            0.0
        );
    }

    #[test]
    fn rows_and_stands() {
        let mut m = MeshArrays::default();
        let look = PlantLook::SUMMER;
        vine_row(
            &mut m,
            Vec3::new(0.0, 100.0, 0.0),
            Vec3::new(0.0, 104.0, 40.0),
            1.8,
            &look,
            vine_colour(&look, false, 0.5),
        );
        check(&m);
        // 9 posts of 12 triangles and 9 pieces of 6 faces of 2 triangles.
        assert_eq!(m.indices.len() / 3, 9 * 12 + 9 * 6 * 2);
        let top = m.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!(top <= 104.0 + 1.85 && top > 104.0 + 1.4, "{top}");
        let mut s = MeshArrays::default();
        stand_cell(
            &mut s,
            Vec3::new(5.0, 0.0, 5.0),
            10.0,
            [1.0, 1.2, 1.1, 1.3, 1.15],
            [2.5, 2.4, 2.6, 2.5, 2.7],
            [1.0, 0.9, 1.1, 1.0, 1.05],
            [true, false, false, true],
            [0.2, 0.4, 0.1],
        );
        check(&s);
        assert_eq!(s.indices.len() / 3, 4 + 2 * 2);
        // The centre stands highest here: 1.15 (its ground) + 2.7.
        let top = s.positions.iter().map(|p| p[1]).fold(f32::MIN, f32::max);
        assert!((top - 3.85).abs() < 1e-5, "{top}");
    }
}

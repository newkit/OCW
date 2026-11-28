//! The Models workspace's pure parts (spec §14 Phase 16, 16B; stage E3,
//! view only): the model browser (grouped by kind and folder, searched),
//! the orbit / turntable camera, the light presets from the Phase 9 light
//! model, the level of detail shown and its switching distance, the
//! gizmos of the contract's anchors, hotspots and collision proxies, the
//! node tree, the stats against the kind's budget and the try-on
//! placements (wear on the rider at an anchor for every body shape, a
//! part at a bike's hotspot, the rider on the bike in the riding poses).
//! The Bevy side is [`crate::models_view`], the panel
//! [`crate::models_ui`].

use bevy::math::{Affine3A, Quat, Vec3};
use ocw_assets::check::{collision_shape, contract_role};
use ocw_assets::kinds::Kind;
use ocw_assets::manifest::Entry;
use ocw_assets::model::Model;
use ocw_assets::pose::{self, BikeRig, BikeSizing, BodyFit, PoseParams, RiderRig};
use ocw_core::character::{BodyShape, Character};

/// One group of the browser: a kind's folder (`parts/<slot>/` its own).
#[derive(Debug, Clone, PartialEq)]
pub struct Group<'a> {
    /// `bikes`, `wear`, `parts/wheel`, …
    pub folder: String,
    pub kind: Kind,
    pub entries: Vec<&'a Entry>,
}

/// The folder an entry is listed under (its path without the file name).
pub fn folder_of(e: &Entry) -> String {
    e.file
        .rsplit_once('/')
        .map_or_else(|| e.kind.dir().to_owned(), |(d, _)| d.to_owned())
}

/// What an entry is found by: its id and the kind's own keys.
fn haystack(e: &Entry) -> String {
    [
        Some(e.id()),
        Some(e.kind.name()),
        e.profile.as_deref(),
        e.species.as_deref(),
        e.variant.as_deref(),
        e.anchor.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_lowercase()
}

/// The browser's list: the entries matching every word of `search` (any
/// case), grouped by folder in the order of [`Kind::ALL`], sorted by id.
pub fn browse<'a>(entries: &'a [Entry], search: &str) -> Vec<Group<'a>> {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    let mut groups: Vec<Group<'a>> = Vec::new();
    for kind in Kind::ALL {
        let mut hits: Vec<&Entry> = entries
            .iter()
            .filter(|e| e.kind == kind)
            .filter(|e| {
                let h = haystack(e);
                words.iter().all(|w| h.contains(w.as_str()))
            })
            .collect();
        hits.sort_by(|a, b| a.file.cmp(&b.file));
        for e in hits {
            let f = folder_of(e);
            match groups.iter_mut().find(|g| g.folder == f) {
                Some(g) => g.entries.push(e),
                None => groups.push(Group {
                    folder: f,
                    kind,
                    entries: vec![e],
                }),
            }
        }
    }
    groups
}

/// The orbit camera: around `target`, `yaw` about +Y (0: looking at the
/// model's front, which faces −Z), `pitch` above the horizon.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

/// The view's vertical field of view (rad).
pub const FOV_Y: f32 = 0.7;
/// Turntable speed (rad/s).
pub const TURNTABLE_RAD_S: f32 = 0.5;

impl Default for Orbit {
    fn default() -> Self {
        Self {
            target: Vec3::new(0.0, 0.5, 0.0),
            yaw: 0.6,
            pitch: 0.25,
            distance: 3.0,
        }
    }
}

impl Orbit {
    /// Frames bounds (min, max): the whole model in view from a
    /// three-quarter front view.
    pub fn framing(lo: Vec3, hi: Vec3) -> Self {
        let r = ((hi - lo).length() * 0.5).max(0.05);
        Self {
            target: (lo + hi) * 0.5,
            yaw: 0.6,
            pitch: 0.25,
            distance: r / (FOV_Y * 0.5).sin() * 1.1,
        }
    }

    /// The camera's position.
    pub fn eye(&self) -> Vec3 {
        // Yaw 0: in front of the model (its front is −Z).
        let dir = Vec3::new(
            self.yaw.sin() * self.pitch.cos(),
            self.pitch.sin(),
            -self.yaw.cos() * self.pitch.cos(),
        );
        self.target + dir * self.distance
    }

    /// A mouse drag of (dx, dy) pixels.
    pub fn drag(&mut self, dx: f32, dy: f32) {
        self.yaw -= dx * 0.008;
        self.pitch = (self.pitch + dy * 0.006).clamp(-1.45, 1.45);
    }

    /// A pan by (dx, dy) pixels in the view plane (shift + drag).
    pub fn pan(&mut self, dx: f32, dy: f32, view_height_px: f32) {
        let m_per_px = 2.0 * self.distance * (FOV_Y * 0.5).tan() / view_height_px.max(1.0);
        let fwd = (self.target - self.eye()).normalize_or_zero();
        let right = fwd.cross(Vec3::Y).normalize_or_zero();
        let up = right.cross(fwd);
        self.target += (-right * dx + up * dy) * m_per_px;
    }

    /// Wheel zoom: `steps` > 0 closer.
    pub fn zoom(&mut self, steps: f32) {
        self.distance = (self.distance * 0.9f32.powf(steps)).clamp(0.05, 2000.0);
    }

    /// The turntable's turn over `dt` seconds.
    pub fn turn(&mut self, dt: f32) {
        self.yaw = (self.yaw + TURNTABLE_RAD_S * dt).rem_euclid(std::f32::consts::TAU);
    }
}

/// The light presets of the view (the Phase 9 light model).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LightPreset {
    #[default]
    Day,
    Dusk,
    Night,
    Overcast,
}

impl LightPreset {
    pub const ALL: [Self; 4] = [Self::Day, Self::Dusk, Self::Night, Self::Overcast];

    pub fn name(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Dusk => "dusk",
            Self::Night => "night",
            Self::Overcast => "overcast",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }

    /// The sun's elevation (°).
    pub fn sun_elevation(self) -> f64 {
        match self {
            Self::Day | Self::Overcast => 42.0,
            Self::Dusk => 2.0,
            Self::Night => -16.0,
        }
    }
}

/// What the view's lights are set to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightRig {
    /// Towards the sun.
    pub sun_dir: Vec3,
    pub sun_lux: f32,
    pub sun_rgb: [f32; 3],
    pub ambient: f32,
    pub ambient_rgb: [f32; 3],
    pub ev100: f32,
    pub shadows: bool,
    /// The background (sRGB): the horizon colour.
    pub sky_srgb: [f32; 3],
}

/// The light of a preset: `ocw_sky::light` at the preset's sun, and for
/// overcast the cloud light of the weather preset `overcast`
/// (`ocw_weather::light`), as the ride's sky does.
pub fn light_rig(p: LightPreset) -> LightRig {
    let el = p.sun_elevation();
    let l = ocw_sky::light::light(el, ocw_sky::light::DEFAULT_NIGHT_FLOOR);
    let az = 135f32.to_radians();
    let e = (el as f32).to_radians();
    let sun_dir = Vec3::new(az.sin() * e.cos(), e.sin().max(0.05), -az.cos() * e.cos()).normalize();
    let mut rig = LightRig {
        sun_dir,
        sun_lux: l.sun_lux,
        sun_rgb: l.sun_rgb,
        ambient: l.ambient,
        ambient_rgb: l.ambient_rgb,
        ev100: l.ev100,
        shadows: l.shadows,
        sky_srgb: l.fog_srgb,
    };
    if p == LightPreset::Overcast {
        let w = ocw_weather::presets::preset("overcast").expect("overcast preset");
        let cl = ocw_weather::light::cloud_light(&w);
        rig.sun_lux *= cl.direct;
        rig.ambient *= cl.diffuse;
        rig.ev100 += cl.stops * 0.5;
        rig.shadows &= cl.shadows;
        let g = 0.62;
        rig.sky_srgb = rig.sky_srgb.map(|c| c + (g - c) * cl.grey);
    }
    rig
}

/// Which level of detail is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LodMode {
    /// By the camera's distance, as in a ride.
    #[default]
    Auto,
    Near,
    Far,
    Billboard,
}

impl LodMode {
    pub const ALL: [Self; 4] = [Self::Auto, Self::Near, Self::Far, Self::Billboard];

    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Near => "near",
            Self::Far => "lod1",
            Self::Billboard => "billboard",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "far" => Some(Self::Far),
            _ => Self::ALL.into_iter().find(|m| m.name() == s),
        }
    }
}

/// A node's level.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Near,
    Lod1,
    Billboard,
    /// `col_*`: never drawn (shown as gizmos).
    Collision,
}

/// The level node `i` belongs to.
pub fn node_level(m: &Model, i: usize) -> Level {
    let mut level = Level::Near;
    let mut at = Some(i);
    while let Some(n) = at {
        match m.nodes[n].name.as_str() {
            "lod1" | "lod2" => level = Level::Lod1,
            "billboard" => level = Level::Billboard,
            name if name.starts_with("col_") => return Level::Collision,
            _ => {}
        }
        at = m.nodes[n].parent;
    }
    level
}

/// Where the game switches a kind from its near model to `lod1` (m from
/// the camera): the game's own table, `ocw_render::lod` (spec 16H Q24
/// (a): bikes, riders, wear and the unicorn 40 m, props 60 m, plants
/// 270 m); `None` where the kind never switches (workshop pieces, parts).
pub fn lod_distance(kind: Kind) -> Option<f32> {
    ocw_render::lod::distance_m(kind)
}

/// The level shown for `mode` at camera distance `distance`.
pub fn shown_level(
    mode: LodMode,
    distance: f32,
    kind: Kind,
    has_lod1: bool,
    has_billboard: bool,
) -> Level {
    match mode {
        LodMode::Near => Level::Near,
        LodMode::Far if has_lod1 => Level::Lod1,
        LodMode::Billboard if has_billboard => Level::Billboard,
        LodMode::Far | LodMode::Billboard => Level::Near,
        LodMode::Auto => match lod_distance(kind) {
            Some(d) if has_lod1 && distance > d => Level::Lod1,
            _ => Level::Near,
        },
    }
}

/// What a gizmo marks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GizmoKind {
    Anchor,
    Hotspot,
    Collision,
}

/// A gizmo: a marked node of the contract.
#[derive(Debug, Clone, PartialEq)]
pub struct Gizmo {
    pub kind: GizmoKind,
    pub name: String,
    pub node: usize,
    /// The node's model-space transform.
    pub transform: Affine3A,
    /// Collision proxies: the shape and its half extents in the node's
    /// frame (its mesh's bounds when it has one, else the node's unit
    /// shape: Blender's empties of size 1).
    pub shape: Option<&'static str>,
    pub half: Vec3,
    pub centre: Vec3,
}

impl Gizmo {
    pub fn position(&self) -> Vec3 {
        self.transform.translation.into()
    }
}

/// The gizmos of a model: anchors `anchor_*`, hotspots `hs_*`, collision
/// proxies `col_*` (15B), at their nodes' model-space transforms.
pub fn gizmos(m: &Model) -> Vec<Gizmo> {
    let mut out = Vec::new();
    for (i, n) in m.nodes.iter().enumerate() {
        let kind = match contract_role(&n.name) {
            Some("anchor") => GizmoKind::Anchor,
            Some("hotspot") => GizmoKind::Hotspot,
            Some("collision proxy") => GizmoKind::Collision,
            _ => continue,
        };
        let (centre, half) = if kind == GizmoKind::Collision {
            proxy_extent(m, i)
        } else {
            (Vec3::ZERO, Vec3::ZERO)
        };
        out.push(Gizmo {
            kind,
            name: n.name.clone(),
            node: i,
            transform: m.world(i),
            shape: collision_shape(&n.name),
            half,
            centre,
        });
    }
    out
}

/// A proxy's centre and half extents in its node's frame.
fn proxy_extent(m: &Model, i: usize) -> (Vec3, Vec3) {
    let n = &m.nodes[i];
    let mut lo = Vec3::splat(f32::INFINITY);
    let mut hi = Vec3::splat(f32::NEG_INFINITY);
    for p in &n.primitives {
        for v in &p.positions {
            lo = lo.min(Vec3::from(*v));
            hi = hi.max(Vec3::from(*v));
        }
    }
    if lo.x.is_finite() {
        ((lo + hi) * 0.5, (hi - lo) * 0.5)
    } else {
        // The node's own scale is in its transform already.
        (Vec3::ZERO, Vec3::ONE)
    }
}

/// A row of the node tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub node: usize,
    pub depth: usize,
    pub name: String,
    /// The contract role (`anchor`, `hotspot`, …) if any.
    pub role: Option<&'static str>,
    /// Triangles of the node itself.
    pub triangles: usize,
    /// Its materials.
    pub materials: Vec<String>,
}

/// The node tree in file order (parents first), with depths.
pub fn tree(m: &Model) -> Vec<TreeRow> {
    m.nodes
        .iter()
        .enumerate()
        .map(|(i, n)| TreeRow {
            node: i,
            depth: std::iter::successors(n.parent, |p| m.nodes[*p].parent).count(),
            name: n.name.clone(),
            role: contract_role(&n.name),
            triangles: n.primitives.iter().map(|p| p.triangles()).sum(),
            materials: n
                .primitives
                .iter()
                .filter_map(|p| m.materials.get(p.material).map(|x| x.name.clone()))
                .collect(),
        })
        .collect()
}

/// The stats panel's numbers.
#[derive(Debug, Clone, PartialEq)]
pub struct Stats {
    pub nodes: usize,
    pub materials: usize,
    pub triangles: usize,
    pub vertices: usize,
    /// The near model's triangles (all but `lod1`, `billboard`, `col_*`).
    pub near_triangles: usize,
    pub lod1_triangles: Option<usize>,
    /// The kind's budgets (near, far).
    pub budget: Option<(usize, Option<usize>)>,
    pub bounds: Option<(Vec3, Vec3)>,
    pub file_bytes: usize,
}

impl Stats {
    pub fn of(m: &Model, kind: Option<Kind>, bytes: usize) -> Self {
        let lod1 = m.node("lod1").map(|l| m.triangles_below(Some(l)));
        let near = (0..m.nodes.len())
            .filter(|&i| node_level(m, i) == Level::Near)
            .map(|i| {
                m.nodes[i]
                    .primitives
                    .iter()
                    .map(|p| p.triangles())
                    .sum::<usize>()
            })
            .sum();
        Self {
            nodes: m.nodes.len(),
            materials: m.materials.len(),
            triangles: m.triangles(),
            vertices: m.vertices(),
            near_triangles: near,
            lod1_triangles: lod1,
            budget: kind.map(Kind::budget),
            bounds: m.near_bounds(),
            file_bytes: bytes,
        }
    }

    /// The near model's share of its budget (1 = at the budget). Bikes and
    /// riders count the whole file, as the validator does.
    pub fn budget_share(&self, kind: Kind) -> Option<f32> {
        let (near, _) = self.budget?;
        let n = if matches!(kind, Kind::Bike | Kind::Rider) {
            self.triangles
        } else {
            self.near_triangles
        };
        Some(n as f32 / near as f32)
    }

    pub fn lod1_share(&self) -> Option<f32> {
        let far = self.budget?.1?;
        Some(self.lod1_triangles? as f32 / far as f32)
    }
}

// --- Try it on (16B) ---

/// The rider's character for a try-on: shape and height (cm).
pub fn try_character(shape: BodyShape, height_cm: u16) -> Character {
    Character {
        shape,
        height_cm,
        ..Character::default()
    }
    .validated()
}

/// Where a wearable worn at rider node `anchor` sits on the rider of
/// character `c`, in the rider's model space (the rider's root scaled by
/// the height, the anchor's girth for its build): the wearable's root
/// transform. `None` when the rider has no such node.
pub fn wear_transform(rider: &Model, anchor: &str, c: &Character) -> Option<Affine3A> {
    let i = rider.node(anchor)?;
    let (girth, shift) = ocw_assets::rider::girth_transform(rider, i, c);
    let scale = Affine3A::from_scale(Vec3::splat(c.scale()));
    let local = Affine3A::from_scale_rotation_translation(girth, Quat::IDENTITY, shift);
    Some(scale * world_with_build(rider, i, c) * local)
}

/// Node `i`'s model-space transform with the build's node translations
/// (the upper arms move with the torso's girth).
pub fn world_with_build(m: &Model, i: usize, c: &Character) -> Affine3A {
    let n = &m.nodes[i];
    let local = Affine3A::from_scale_rotation_translation(
        n.scale,
        n.rotation,
        ocw_assets::rider::node_translation(m, i, c),
    );
    match n.parent {
        Some(p) => world_with_build(m, p, c) * local,
        None => local,
    }
}

/// The hotspot node of a part's slot (`parts/wheel/…` → `hs_wheels`):
/// the slot folder, or its plural.
pub fn hotspot_for(bike: &Model, slot: &str) -> Option<usize> {
    bike.node(&format!("hs_{slot}"))
        .or_else(|| bike.node(&format!("hs_{slot}s")))
}

/// The slot of a part file (`parts/<slot>/<name>.glb`).
pub fn part_slot(e: &Entry) -> Option<&str> {
    let mut p = e.file.split('/');
    (p.next()? == "parts").then_some(())?;
    p.next()
}

/// A part placed at its bike's hotspot: the part's root transform in the
/// bike's model space.
pub fn part_transform(bike: &Model, slot: &str) -> Option<Affine3A> {
    hotspot_for(bike, slot).map(|i| bike.world(i))
}

/// The riding poses of the try-on (`ocw_assets::pose`, §2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RidePose {
    #[default]
    Hoods,
    Tops,
    Drops,
    Tuck,
    Standing,
    ArmsUp,
    SitUp,
    Throw,
    Stop,
}

impl RidePose {
    pub const ALL: [Self; 9] = [
        Self::Hoods,
        Self::Tops,
        Self::Drops,
        Self::Tuck,
        Self::Standing,
        Self::ArmsUp,
        Self::SitUp,
        Self::Throw,
        Self::Stop,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Hoods => "hoods",
            Self::Tops => "tops",
            Self::Drops => "drops",
            Self::Tuck => "tuck",
            Self::Standing => "standing",
            Self::ArmsUp => "arms-up",
            Self::SitUp => "sit-up",
            Self::Throw => "throw",
            Self::Stop => "stop",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == s)
    }

    /// The pose's blend weights at crank angle `crank` (rad).
    pub fn params(self, crank: f32) -> PoseParams {
        let mut p = PoseParams {
            crank,
            ..Default::default()
        };
        match self {
            Self::Hoods => {}
            Self::Tops => p.tops = 1.0,
            Self::Drops => p.drops = 1.0,
            Self::Tuck => {
                p.drops = 1.0;
                p.tuck = 1.0;
            }
            Self::Standing => {
                p.stand = 1.0;
                p.rock = 0.12;
            }
            Self::ArmsUp => p.arms_up = 1.0,
            Self::SitUp => p.sit_up = 1.0,
            Self::Throw => {
                p.drops = 1.0;
                p.throw = 1.0;
            }
            Self::Stop => {
                p.crank = 0.0;
                p.straddle = 1.0;
                p.foot = 1.0;
            }
        }
        p
    }
}

/// The rider fitted to a bike for a character (the bike file's sizing).
pub fn ride_fit(rider: &Model, bike: &Model, c: &Character) -> Result<BodyFit, String> {
    let r = RiderRig::from_model(rider)?.for_body(c);
    let b = BikeRig::from_model(bike)?;
    Ok(BodyFit::sized(
        &r,
        &b,
        c.scale(),
        &BikeSizing::from_model(bike),
    ))
}

/// The local transforms the pose writes, by node name: the pelvis
/// (translation in the scaled rider root's frame, as the game does) and
/// every posed joint's rotation (rider and bike nodes: crank, pedals).
pub fn posed_nodes(fit: &BodyFit, p: &PoseParams) -> Vec<(&'static str, Option<Vec3>, Quat)> {
    let pose = pose::pose(&fit.rider, &fit.bike, &fit.fit, p);
    let mut out = vec![(
        pose::nodes::PELVIS,
        Some(pose.pelvis.0 / fit.scale),
        pose.pelvis.1,
    )];
    out.extend(pose.rotations().iter().map(|(n, q)| (*n, None, *q)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_assets::{glb, shipped};

    fn entries() -> Vec<Entry> {
        ocw_assets::manifest::Manifest::parse(shipped::MANIFEST)
            .unwrap()
            .assets
    }

    #[test]
    fn the_browser_groups_by_kind_and_folder_and_searches() {
        let all = entries();
        let groups = browse(&all, "");
        let listed: usize = groups.iter().map(|g| g.entries.len()).sum();
        assert_eq!(listed, all.len());
        // In the order of the kinds: bikes first, riders, wear, …
        assert_eq!(groups[0].folder, "bikes");
        assert_eq!(groups[0].kind, Kind::Bike);
        assert_eq!(groups[0].entries.len(), ocw_core::ProfileId::ALL.len());
        let order: Vec<&str> = groups.iter().map(|g| g.folder.as_str()).collect();
        let bikes = order.iter().position(|f| *f == "bikes").unwrap();
        let flora = order.iter().position(|f| *f == "flora").unwrap();
        assert!(bikes < flora);
        // Sorted within a group.
        for g in &groups {
            assert!(g.entries.windows(2).all(|w| w[0].file < w[1].file));
        }
        // Searching: every word, any case, ids and kind keys.
        let g = browse(&all, "HELMET aero");
        let ids: Vec<&str> = g
            .iter()
            .flat_map(|g| g.entries.iter().map(|e| e.id()))
            .collect();
        assert_eq!(ids, ["wear/helmet-aero-road"]);
        let g = browse(&all, "tt");
        assert!(
            g.iter()
                .any(|g| g.entries.iter().any(|e| e.id() == "bikes/bike-tt"))
        );
        // A plant by its species key, a wearable by its anchor.
        assert!(!browse(&all, "pricklypear").is_empty());
        assert!(
            browse(&all, "hand_l")
                .iter()
                .all(|g| g.kind == Kind::Wearable)
        );
        assert!(browse(&all, "no-such-model-xyz").is_empty());
        let e = &all.iter().find(|e| e.file.starts_with("bikes/")).unwrap();
        assert_eq!(folder_of(e), "bikes");
    }

    #[test]
    fn the_orbit_frames_and_turns() {
        let o = Orbit::framing(Vec3::new(-0.5, 0.0, -1.0), Vec3::new(0.5, 1.0, 1.0));
        assert_eq!(o.target, Vec3::new(0.0, 0.5, 0.0));
        let r = (Vec3::new(1.0, 1.0, 2.0)).length() * 0.5;
        assert!((o.eye().distance(o.target) - o.distance).abs() < 1e-4);
        assert!(o.distance > r);
        // Yaw 0 looks at the front (the model faces −Z: the eye is at −Z).
        let front = Orbit {
            yaw: 0.0,
            pitch: 0.0,
            ..o
        };
        assert!(front.eye().z < o.target.z - 0.9 * o.distance);
        let mut t = front;
        t.turn(1.0);
        assert!((t.yaw - TURNTABLE_RAD_S).abs() < 1e-6);
        t.drag(0.0, 10_000.0);
        assert!(t.pitch <= 1.45);
        let d = t.distance;
        t.zoom(1.0);
        assert!(t.distance < d);
        let before = t.target;
        t.pan(100.0, 0.0, 800.0);
        assert!(t.target.distance(before) > 0.0);
    }

    #[test]
    fn the_light_presets_follow_the_light_model() {
        let day = light_rig(LightPreset::Day);
        let dusk = light_rig(LightPreset::Dusk);
        let night = light_rig(LightPreset::Night);
        let grey = light_rig(LightPreset::Overcast);
        assert!(day.sun_lux > dusk.sun_lux && dusk.sun_lux > night.sun_lux);
        assert!(day.shadows && !night.shadows);
        // Overcast: much less direct sun, no hard shadows, a greyer sky.
        assert!(grey.sun_lux < 0.5 * day.sun_lux);
        assert!(!grey.shadows);
        assert!(grey.ambient > 0.0);
        assert!(day.sun_dir.y > 0.5 && (day.sun_dir.length() - 1.0).abs() < 1e-4);
        for p in LightPreset::ALL {
            assert_eq!(LightPreset::parse(p.name()), Some(p));
        }
    }

    #[test]
    fn the_level_of_detail_switches_at_the_games_distance() {
        let bike = shipped::bike_model(ocw_core::ProfileId::Road);
        let lod = bike.node("lod1").unwrap();
        assert_eq!(node_level(bike, lod), Level::Lod1);
        assert_eq!(node_level(bike, bike.node("crank").unwrap()), Level::Near);
        let d = lod_distance(Kind::Bike).unwrap();
        assert_eq!(d, 40.0);
        assert_eq!(
            shown_level(LodMode::Auto, d - 1.0, Kind::Bike, true, false),
            Level::Near
        );
        assert_eq!(
            shown_level(LodMode::Auto, d + 1.0, Kind::Bike, true, false),
            Level::Lod1
        );
        // The auto level follows the game's table for every kind (16H
        // Q24): props at 60 m, the unicorn at 40 m, workshop pieces never.
        for k in Kind::ALL {
            match ocw_render::lod::distance_m(k) {
                Some(d) => {
                    assert_eq!(lod_distance(k), Some(d), "{k:?}");
                    assert_eq!(
                        shown_level(LodMode::Auto, d - 1.0, k, true, false),
                        Level::Near
                    );
                    assert_eq!(
                        shown_level(LodMode::Auto, d + 1.0, k, true, false),
                        Level::Lod1
                    );
                    // Without a `lod1` the near model at any distance.
                    assert_eq!(
                        shown_level(LodMode::Auto, d + 1.0, k, false, false),
                        Level::Near
                    );
                }
                None => assert_eq!(
                    shown_level(LodMode::Auto, 1e4, k, true, false),
                    Level::Near,
                    "{k:?}"
                ),
            }
        }
        assert_eq!(lod_distance(Kind::Prop), Some(60.0));
        assert_eq!(lod_distance(Kind::Creature), Some(40.0));
        assert_eq!(lod_distance(Kind::Workshop), None);
        assert_eq!(
            shown_level(LodMode::Far, 1.0, Kind::Bike, true, false),
            Level::Lod1
        );
        // No billboard in the file: the near model.
        assert_eq!(
            shown_level(LodMode::Billboard, 1.0, Kind::Plant, true, false),
            Level::Near
        );
        assert!(lod_distance(Kind::Plant).unwrap() > 200.0);
        assert_eq!(LodMode::parse("far"), Some(LodMode::Far));
    }

    /// The gizmos sit at their nodes' transforms (16G).
    #[test]
    fn gizmo_positions_are_the_node_transforms() {
        for p in ocw_core::ProfileId::ALL {
            let bike = shipped::bike_model(p);
            let g = gizmos(bike);
            let hotspots: Vec<&Gizmo> = g.iter().filter(|g| g.kind == GizmoKind::Hotspot).collect();
            assert_eq!(hotspots.len(), ocw_assets::pose::bike_nodes::HOTSPOTS.len());
            for h in hotspots {
                let want = bike.position(&h.name).unwrap();
                assert!(h.position().distance(want) < 1e-6, "{}", h.name);
                assert_eq!(h.transform, bike.world(h.node));
            }
        }
        // The rider's pocket anchors.
        let rider = shipped::rider_model();
        let anchors: Vec<Gizmo> = gizmos(rider)
            .into_iter()
            .filter(|g| g.kind == GizmoKind::Anchor)
            .collect();
        assert!(anchors.iter().any(|a| a.name == "anchor_pocket_c"));
        for a in &anchors {
            assert!(a.position().distance(rider.position(&a.name).unwrap()) < 1e-6);
        }
        // A collision proxy with a mesh: its bounds; without: a unit shape.
        let mut m = Model::default();
        let root = m.add(ocw_assets::model::Node::new("post", None, Vec3::ZERO));
        let mut c = ocw_assets::model::Node::new("col_box", Some(root), Vec3::new(0.0, 0.5, 0.0));
        c.scale = Vec3::new(0.05, 0.5, 0.05);
        m.add(c);
        let g = gizmos(&m);
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].kind, GizmoKind::Collision);
        assert_eq!(g[0].shape, Some("box"));
        assert_eq!(g[0].half, Vec3::ONE);
        assert_eq!(g[0].position(), Vec3::new(0.0, 0.5, 0.0));
        let back = glb::read(&glb::write(&m)).unwrap();
        assert_eq!(gizmos(&back)[0].position(), Vec3::new(0.0, 0.5, 0.0));
    }

    #[test]
    fn the_tree_and_the_stats_count_against_the_budget() {
        let bike = shipped::bike_model(ocw_core::ProfileId::Road);
        let rows = tree(bike);
        assert_eq!(rows.len(), bike.nodes.len());
        assert_eq!(rows[0].depth, 0);
        assert!(rows.iter().any(|r| r.role == Some("hotspot")));
        assert_eq!(
            rows.iter().map(|r| r.triangles).sum::<usize>(),
            bike.triangles()
        );
        let s = Stats::of(bike, Some(Kind::Bike), 123);
        assert_eq!(s.triangles, bike.triangles());
        assert_eq!(s.vertices, bike.vertices());
        assert_eq!(s.near_triangles + s.lod1_triangles.unwrap(), s.triangles);
        assert_eq!(s.budget, Some((15_000, Some(4_000))));
        let share = s.budget_share(Kind::Bike).unwrap();
        assert!(share > 0.0 && share <= 1.0, "{share}");
        assert!(s.lod1_share().unwrap() <= 1.0);
        let (lo, hi) = s.bounds.unwrap();
        assert!(lo.y.abs() < 0.05 && hi.y > 0.8);
        assert_eq!(s.file_bytes, 123);
    }

    /// Try-on: the wear's root sits on its anchor for every body shape and
    /// height (16G).
    #[test]
    fn wear_sits_at_its_anchor_for_every_build() {
        let rider = shipped::rider_model();
        for shape in BodyShape::ALL {
            for h in [150, 178, 205] {
                let c = try_character(shape, h);
                let head = wear_transform(rider, "head", &c).unwrap();
                // The head does not change girth: the helmet's origin is
                // the scaled head node.
                let want = rider.position("head").unwrap() * c.scale();
                assert!(
                    Vec3::from(head.translation).distance(want) < 1e-4,
                    "{shape:?} {h}"
                );
                // The torso wear takes the torso's girth and its centre.
                let t = wear_transform(rider, "torso", &c).unwrap();
                let g = ocw_assets::rider::girth_of("torso", &c);
                let i = rider.node("torso").unwrap();
                let centre = ocw_assets::rider::girth_centre(rider, i);
                // The girth centre stays where it is on the torso.
                let world_centre = t.transform_point3(centre);
                let want = Affine3A::from_scale(Vec3::splat(c.scale()))
                    .transform_point3(world_with_build(rider, i, &c).transform_point3(centre));
                assert!(world_centre.distance(want) < 1e-4, "{shape:?} {h}");
                let sx = Vec3::from(t.matrix3.x_axis).length();
                assert!((sx - g.x * c.scale()).abs() < 1e-4, "{shape:?} {h}");
            }
        }
        assert!(wear_transform(rider, "tail", &Character::default()).is_none());
    }

    #[test]
    fn parts_sit_at_their_bikes_hotspots() {
        for p in ocw_core::ProfileId::ALL {
            let bike = shipped::bike_model(p);
            for slot in ["saddle", "wheels", "wheel", "pedals", "lights"] {
                let t = part_transform(bike, slot).unwrap_or_else(|| panic!("{p:?} {slot}"));
                let i = hotspot_for(bike, slot).unwrap();
                assert_eq!(t, bike.world(i));
            }
            assert!(part_transform(bike, "rocket").is_none());
        }
        let mut e = ocw_assets::export::entry("parts/wheel/wheel-carbon-deep.glb", Kind::Part, "x");
        assert_eq!(part_slot(&e), Some("wheel"));
        e.file = "props/post-square.glb".into();
        assert_eq!(part_slot(&e), None);
    }

    #[test]
    fn the_rider_rides_every_bike_in_every_pose() {
        let rider = shipped::rider_model();
        for p in ocw_core::ProfileId::ALL {
            let bike = shipped::bike_model(p);
            for shape in [BodyShape::Slim, BodyShape::Fat] {
                let c = try_character(shape, 190);
                let fit = ride_fit(rider, bike, &c).unwrap();
                for pose in RidePose::ALL {
                    let nodes = posed_nodes(&fit, &pose.params(1.0));
                    assert_eq!(nodes[0].0, "pelvis");
                    let pelvis = nodes[0].1.unwrap() * fit.scale;
                    // The hips above the bottom bracket's height, behind
                    // the bars.
                    assert!(pelvis.y > 0.5, "{p:?} {pose:?} {pelvis}");
                    assert!(nodes.iter().all(|(_, _, q)| q.is_finite()));
                    assert!(nodes.iter().any(|(n, _, _)| *n == "crank"));
                }
            }
        }
        for p in RidePose::ALL {
            assert_eq!(RidePose::parse(p.name()), Some(p));
        }
    }
}

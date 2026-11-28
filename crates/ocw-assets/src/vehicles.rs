//! The traffic's models (spec 14 Phase 18, 18B *Models*, 18E T6): cars in
//! three shapes (hatchback, saloon, SUV), a panel van, a rigid and an
//! articulated lorry, a city bus, a motorbike with its rider, a farm
//! tractor, and for the sky two narrow-body airliners (sharklets; split
//! winglets), a small high-wing plane, a glider, a light and a medium
//! transport helicopter, a hot-air balloon and a flying saucer. The
//! airliners and the medium helicopter are detailed from the user's own
//! apron photos (18G, `vehicles/aircraft.rs`, `docs/reference/aircraft.md`).
//!
//! All are our own generic shapes – no makes, no logos, no copies of a
//! real model – in the game's low-poly style: flat-shaded boxes, lofts
//! and lathes, every triangle with an area and the normal of its face (so
//! they survive Blender, 15M), positions on the writer's 1/65536 m grid.
//! `ocw-assets model export-all` writes them into
//! `artwork/3dmodels/vehicles/` (Git LFS); the tests keep the files equal
//! to this generator while it lives (Q6).
//!
//! The contract (`ocw_assets::kinds::VehicleClass`, the validator): the
//! origin on the ground under the centre of the wheelbase (aircraft,
//! balloon, UFO: under the body's centre), the front at −Z here (+Z in the
//! file); `extras.ocw.class`; the wheels `wheel_fl` … `wheel_rr` (and
//! `wheel_r2_*` … on the articulated lorry's trailer; `wheel_front` /
//! `wheel_rear` on the motorbike) at their axles; the lamps as their own
//! nodes drawn in lamp slots (`light_head_l/r`, `light_brake_l/r`,
//! `light_indicator_fl/fr/rl/rr`; aircraft `light_nav_l/r`, `light_tail`,
//! `light_beacon`, `light_strobe_l/r`; the burner, the UFO's rim); the
//! moving parts `rotor_main`, `rotor_tail`, `prop`, `door_*`, `trailer`,
//! `gear_*`; the rider placeholder `rider`; `lod1`; the collision boxes
//! `col_box*` (road vehicles); the paint slot `body` the game varies (a
//! bus: its made-up operator's `livery`).

use std::f32::consts::{PI, TAU};

use glam::{Quat, Vec3};
use serde_json::json;

use crate::kinds::{Kind, VehicleClass};
use crate::manifest::Entry;
use crate::mesh::{Parts, Shape};
use crate::model::{Model, Node, srgb};
use crate::slots;

mod aircraft;

/// The folder of the files.
pub const DIR: &str = "vehicles/";

/// The root node of every file.
pub const ROOT: &str = "vehicle";

/// One model of the traffic.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Vehicle {
    CarHatchback,
    CarSaloon,
    CarSuv,
    VanPanel,
    LorryRigid,
    LorryArticulated,
    BusCity,
    MotorbikeTouring,
    TractorFarm,
    AirlinerTwinjet,
    AirlinerTwinjetSplitWinglet,
    PlaneHighWing,
    GliderSailplane,
    HelicopterLight,
    HelicopterMediumTransport,
    BalloonHotAir,
    UfoSaucer,
}

impl Vehicle {
    pub const ALL: [Vehicle; 17] = [
        Vehicle::CarHatchback,
        Vehicle::CarSaloon,
        Vehicle::CarSuv,
        Vehicle::VanPanel,
        Vehicle::LorryRigid,
        Vehicle::LorryArticulated,
        Vehicle::BusCity,
        Vehicle::MotorbikeTouring,
        Vehicle::TractorFarm,
        Vehicle::AirlinerTwinjet,
        Vehicle::AirlinerTwinjetSplitWinglet,
        Vehicle::PlaneHighWing,
        Vehicle::GliderSailplane,
        Vehicle::HelicopterLight,
        Vehicle::HelicopterMediumTransport,
        Vehicle::BalloonHotAir,
        Vehicle::UfoSaucer,
    ];

    /// The file in `artwork/3dmodels/` (15H names: function, then shape).
    pub fn file(self) -> &'static str {
        match self {
            Vehicle::CarHatchback => "vehicles/car-hatchback.glb",
            Vehicle::CarSaloon => "vehicles/car-saloon.glb",
            Vehicle::CarSuv => "vehicles/car-suv.glb",
            Vehicle::VanPanel => "vehicles/van-panel.glb",
            Vehicle::LorryRigid => "vehicles/lorry-rigid-box.glb",
            Vehicle::LorryArticulated => "vehicles/lorry-articulated-box.glb",
            Vehicle::BusCity => "vehicles/bus-city.glb",
            Vehicle::MotorbikeTouring => "vehicles/motorbike-touring.glb",
            Vehicle::TractorFarm => "vehicles/tractor-farm.glb",
            Vehicle::AirlinerTwinjet => "vehicles/airliner-twinjet.glb",
            Vehicle::AirlinerTwinjetSplitWinglet => "vehicles/airliner-twinjet-split-winglet.glb",
            Vehicle::PlaneHighWing => "vehicles/plane-high-wing.glb",
            Vehicle::GliderSailplane => "vehicles/glider-sailplane.glb",
            Vehicle::HelicopterLight => "vehicles/helicopter-light.glb",
            Vehicle::HelicopterMediumTransport => "vehicles/helicopter-medium-transport.glb",
            Vehicle::BalloonHotAir => "vehicles/balloon-hot-air.glb",
            Vehicle::UfoSaucer => "vehicles/ufo-saucer.glb",
        }
    }

    pub fn class(self) -> VehicleClass {
        match self {
            Vehicle::CarHatchback | Vehicle::CarSaloon | Vehicle::CarSuv => VehicleClass::Car,
            Vehicle::VanPanel => VehicleClass::Van,
            Vehicle::LorryRigid | Vehicle::LorryArticulated => VehicleClass::Lorry,
            Vehicle::BusCity => VehicleClass::Bus,
            Vehicle::MotorbikeTouring => VehicleClass::Motorbike,
            Vehicle::TractorFarm => VehicleClass::Tractor,
            Vehicle::AirlinerTwinjet | Vehicle::AirlinerTwinjetSplitWinglet => {
                VehicleClass::Airliner
            }
            Vehicle::PlaneHighWing => VehicleClass::SmallPlane,
            Vehicle::GliderSailplane => VehicleClass::Glider,
            Vehicle::HelicopterLight | Vehicle::HelicopterMediumTransport => {
                VehicleClass::Helicopter
            }
            Vehicle::BalloonHotAir => VehicleClass::Balloon,
            Vehicle::UfoSaucer => VehicleClass::Ufo,
        }
    }

    /// The manifest entry.
    pub fn entry(self) -> Entry {
        let mut e = crate::export::entry(
            self.file(),
            Kind::Vehicle,
            &format!("ocw_assets::vehicles::Vehicle::{self:?}.model()"),
        );
        e.source =
            "own work, generic shape made by the OCW generator ocw_assets::vehicles (spec 14 Phase 18, 18E T6)"
                .into();
        e
    }

    /// The model, stamped (15B).
    pub fn model(self) -> Model {
        let mut b = Build::new();
        let f: fn(&mut Build) = match self {
            Vehicle::CarHatchback => |b| car(b, &HATCHBACK),
            Vehicle::CarSaloon => |b| car(b, &SALOON),
            Vehicle::CarSuv => |b| car(b, &SUV),
            Vehicle::VanPanel => van,
            Vehicle::LorryRigid => lorry_rigid,
            Vehicle::LorryArticulated => lorry_articulated,
            Vehicle::BusCity => bus,
            Vehicle::MotorbikeTouring => motorbike,
            Vehicle::TractorFarm => tractor,
            Vehicle::AirlinerTwinjet => |b| aircraft::airliner(b, &aircraft::AIRLINER_SHARKLET),
            Vehicle::AirlinerTwinjetSplitWinglet => {
                |b| aircraft::airliner(b, &aircraft::AIRLINER_SPLIT)
            }
            Vehicle::PlaneHighWing => small_plane,
            Vehicle::GliderSailplane => glider,
            Vehicle::HelicopterLight => helicopter,
            Vehicle::HelicopterMediumTransport => aircraft::helicopter_medium,
            Vehicle::BalloonHotAir => balloon,
            Vehicle::UfoSaucer => ufo,
        };
        for level in [Level::Near, Level::Far] {
            b.level = level;
            f(&mut b);
        }
        b.finish(self)
    }
}

// ---------------------------------------------------------------------
// The builder: nodes, the two levels, lamps and wheels.

/// The writer's grid (1/65536 m): positions are put on it before the
/// normals are taken, so the file's normals are its triangles' exactly.
fn q(v: Vec3) -> Vec3 {
    (v * 65536.0).round() / 65536.0
}

fn v3(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Level {
    Near,
    Far,
}

struct Build {
    m: Model,
    /// The shapes per node (index = node index).
    parts: Vec<Parts>,
    level: Level,
    lod1: usize,
}

impl Build {
    fn new() -> Self {
        let mut m = Model::default();
        m.add(Node::new(ROOT, None, Vec3::ZERO));
        let lod1 = m.add(Node::new("lod1", Some(0), Vec3::ZERO));
        Self {
            m,
            parts: vec![Parts::new(), Parts::new()],
            level: Level::Near,
            lod1,
        }
    }

    fn far(&self) -> bool {
        self.level == Level::Far
    }

    /// The node `name` below `parent`, at model position `at` (made once:
    /// in the near pass; nodes only move, never turn).
    fn node(&mut self, name: &str, parent: &str, at: Vec3) -> usize {
        if let Some(i) = self.m.node(name) {
            return i;
        }
        let p = self.m.node(parent).expect("parent node");
        let base = Vec3::from(self.m.world(p).translation);
        let i = self.m.add(Node::new(name, Some(p), q(at - base)));
        self.parts.push(Parts::new());
        i
    }

    /// Where the shapes of `name` go: the node near, `lod1` far.
    fn on(&mut self, name: &str) -> &mut Parts {
        let i = if self.far() {
            self.lod1
        } else {
            self.m.node(name).expect("node")
        };
        &mut self.parts[i]
    }

    /// The root's shapes (near), or `lod1`'s.
    fn root(&mut self) -> &mut Parts {
        self.on(ROOT)
    }

    /// A part of its own (a node near; merged into `lod1` far).
    fn part(&mut self, name: &str, parent: &str, at: Vec3) -> &mut Parts {
        if !self.far() {
            self.node(name, parent, at);
        }
        self.on(name)
    }

    /// The file's default colour of a slot (the slot's own otherwise).
    fn paint(&mut self, slot: &str, hex: u32) {
        if self.m.material(slot).is_none() {
            let mut mat = slots::material(slot);
            mat.color = srgb(hex);
            self.m.materials.push(mat);
        }
    }

    /// A lamp: its node at the lens's centre, the lens a box of half size
    /// `half` (near), or the lens's face towards `facing` in `lod1` (far).
    fn lamp(
        &mut self,
        name: &str,
        parent: &str,
        slot: &'static str,
        centre: Vec3,
        half: Vec3,
        facing: Vec3,
    ) {
        let far = self.far();
        let p = self.part(name, parent, centre);
        if far {
            lens_face(p.slot(slot), centre, half, facing);
        } else {
            cuboid(p, [slot; 6], centre, half);
        }
    }

    /// A road wheel: its node at the axle, the tyre and (near) the hub
    /// on the outer side (`side` −1: the left, outer face at −X).
    fn wheel(&mut self, name: &str, parent: &str, centre: Vec3, side: f32, r: f32, w: f32) {
        let far = self.far();
        let p = self.part(name, parent, centre);
        wheel(p, centre, side, r, w, if far { 6 } else { 12 }, !far);
    }

    fn finish(mut self, v: Vehicle) -> Model {
        for (i, parts) in std::mem::take(&mut self.parts).into_iter().enumerate() {
            parts.attach(&mut self.m, i);
        }
        let mut m = self.m;
        let class = v.class();
        if class.on_road() {
            // The collision boxes (12B): the vehicle without its trailer,
            // and the trailer on its own.
            let trailer = m.node("trailer");
            let keep_main = |m: &Model, i: usize| {
                !m.under_named(i, |n| {
                    n == "lod1" || n == "trailer" || n.starts_with("col_")
                })
            };
            // (The trailer's first: the nodes stay in the depth-first order
            // the reader gives back.)
            let main = m.bounds_where(|i| keep_main(&m, i)).expect("a mesh");
            if let Some(t) = trailer {
                let (lo, hi) = m
                    .bounds_where(|i| m.is_below(i, t) && !m.under_named(i, |n| n == "lod1"))
                    .expect("the trailer's mesh");
                add_box(&mut m, "col_box_trailer", t, lo, hi);
            }
            add_box(&mut m, "col_box", 0, main.0, main.1);
        }
        let (lo, hi) = m.near_bounds().expect("a mesh");
        let size = hi - lo;
        let r = |x: f32| (x * 100.0).round() / 100.0;
        let e = v.entry();
        crate::export::stamp(
            &mut m,
            &e,
            json!({
                "class": class.name(),
                "size_m": { "length": r(size.z), "width": r(size.x), "height": r(size.y) },
            }),
        );
        m
    }
}

/// An empty box proxy `name` below `parent` over the model-space box
/// `lo`–`hi` (the unit box scaled, as the props' proxies, 15N).
fn add_box(m: &mut Model, name: &str, parent: usize, lo: Vec3, hi: Vec3) {
    let base = Vec3::from(m.world(parent).translation);
    let mut n = Node::new(name, Some(parent), q((lo + hi) / 2.0 - base));
    n.scale = q((hi - lo) / 2.0);
    m.add(n);
}

// ---------------------------------------------------------------------
// Flat-shaded geometry.

/// A triangle (on the grid; none without area).
fn tri(s: &mut Shape, a: Vec3, b: Vec3, c: Vec3) {
    let (a, b, c) = (q(a), q(b), q(c));
    if (b - a).cross(c - a).length() < 1e-7 {
        return;
    }
    s.add_flat(&[a, b, c], &[[0, 1, 2]]);
}

/// A convex polygon, its front towards `out`.
fn poly(s: &mut Shape, pts: &[Vec3], out: Vec3) {
    let mut n = Vec3::ZERO;
    for (k, a) in pts.iter().enumerate() {
        let b = pts[(k + 1) % pts.len()];
        n += a.cross(b);
    }
    let flip = n.dot(out) < 0.0;
    for k in 1..pts.len() - 1 {
        if flip {
            tri(s, pts[0], pts[k + 1], pts[k]);
        } else {
            tri(s, pts[0], pts[k], pts[k + 1]);
        }
    }
}

/// The faces of a hexahedron: bottom, top, front (−Z), right (+X), rear,
/// left. Corners: the bottom ring front-left, front-right, rear-right,
/// rear-left, then the top ring in the same order.
const FACES: [[usize; 4]; 6] = [
    [0, 1, 2, 3],
    [4, 5, 6, 7],
    [0, 1, 5, 4],
    [1, 2, 6, 5],
    [2, 3, 7, 6],
    [3, 0, 4, 7],
];
const FRONT: usize = 2;
const RIGHT: usize = 3;
const REAR: usize = 4;
const LEFT: usize = 5;

/// A convex hexahedron, each face in its slot (`slots` in [`FACES`]'
/// order).
fn hexa(p: &mut Parts, slot: [&'static str; 6], c: &[Vec3; 8]) {
    let centre = c.iter().copied().sum::<Vec3>() / 8.0;
    for (f, idx) in FACES.iter().enumerate() {
        let pts = idx.map(|i| c[i]);
        let mid = pts.iter().copied().sum::<Vec3>() / 4.0;
        poly(p.slot(slot[f]), &pts, mid - centre);
    }
}

/// The corners of a slab: the bottom rectangle at `y0` over `z0..z1`,
/// `±w0` wide, the top at `y1` over `zt0..zt1`, `±w1`.
#[allow(clippy::too_many_arguments)]
fn slab(z0: f32, z1: f32, y0: f32, w0: f32, zt0: f32, zt1: f32, y1: f32, w1: f32) -> [Vec3; 8] {
    [
        v3(-w0, y0, z0),
        v3(w0, y0, z0),
        v3(w0, y0, z1),
        v3(-w0, y0, z1),
        v3(-w1, y1, zt0),
        v3(w1, y1, zt0),
        v3(w1, y1, zt1),
        v3(-w1, y1, zt1),
    ]
}

/// A box (half extents `half`), each face in its slot.
fn cuboid(p: &mut Parts, slot: [&'static str; 6], centre: Vec3, half: Vec3) {
    obox(p, slot, centre, half, Quat::IDENTITY);
}

/// A box turned by `rot`.
fn obox(p: &mut Parts, slot: [&'static str; 6], centre: Vec3, half: Vec3, rot: Quat) {
    let c = slab(
        -half.z, half.z, -half.y, half.x, -half.z, half.z, half.y, half.x,
    )
    .map(|k| centre + rot * k);
    hexa(p, slot, &c);
}

fn all(slot: &'static str) -> [&'static str; 6] {
    [slot; 6]
}

/// The point at (`u`, `v`) of a hexahedron's face (bilinear; `u` along
/// its first edge, `v` up its side).
fn on_face(c: &[Vec3; 8], face: usize, u: f32, v: f32) -> Vec3 {
    let [a, b, cc, d] = FACES[face].map(|i| c[i]);
    let lo = a.lerp(b, u);
    let hi = d.lerp(cc, u);
    lo.lerp(hi, v)
}

/// The outward normal of a face.
fn face_normal(c: &[Vec3; 8], face: usize) -> Vec3 {
    let centre = c.iter().copied().sum::<Vec3>() / 8.0;
    let [a, b, cc, d] = FACES[face].map(|i| c[i]);
    let n = (b - a).cross(d - a) + (cc - b).cross(cc - d);
    let n = n.normalize_or_zero();
    if n.dot((a + b + cc + d) / 4.0 - centre) < 0.0 {
        -n
    } else {
        n
    }
}

/// A flat patch on a face (windows, stripes, doors): the face's part
/// `u0..u1` × `v0..v1`, lifted `lift` off it.
#[allow(clippy::too_many_arguments)]
fn decal(s: &mut Shape, c: &[Vec3; 8], face: usize, u0: f32, u1: f32, v0: f32, v1: f32, lift: f32) {
    let n = face_normal(c, face);
    let pts =
        [(u0, v0), (u1, v0), (u1, v1), (u0, v1)].map(|(u, v)| on_face(c, face, u, v) + n * lift);
    poly(s, &pts, n);
}

/// The centre of a face's patch, lifted (where a lamp on it goes).
fn face_point(c: &[Vec3; 8], face: usize, u: f32, v: f32, lift: f32) -> Vec3 {
    on_face(c, face, u, v) + face_normal(c, face) * lift
}

/// The face of a lens box towards `facing` (an axis), for `lod1`.
fn lens_face(s: &mut Shape, centre: Vec3, half: Vec3, facing: Vec3) {
    let f = facing.normalize();
    let (a, b) = if f.z.abs() > 0.5 {
        (Vec3::X * half.x, Vec3::Y * half.y)
    } else if f.x.abs() > 0.5 {
        (Vec3::Z * half.z, Vec3::Y * half.y)
    } else {
        (Vec3::X * half.x, Vec3::Z * half.z)
    };
    let c = centre + f * (half * f.abs()).length();
    poly(s, &[c - a - b, c + a - b, c + a + b, c - a + b], f);
}

/// A surface of revolution about `axis` through `o`: rings at `t` along
/// the axis with radius `r` (`r` 0: a pole), lifted `lift` along `u`;
/// `u` scaled by `su`, `v = axis × u` by `sv`. The profile runs from the
/// −axis end to the +axis end; open ends get flat caps in `cap`.
struct Lathe {
    o: Vec3,
    axis: Vec3,
    u: Vec3,
    sides: u32,
    su: f32,
    sv: f32,
}

impl Lathe {
    fn new(o: Vec3, axis: Vec3, u: Vec3, sides: u32) -> Self {
        Self {
            o,
            axis: axis.normalize(),
            u: u.normalize(),
            sides,
            su: 1.0,
            sv: 1.0,
        }
    }

    fn squash(mut self, su: f32, sv: f32) -> Self {
        self.su = su;
        self.sv = sv;
        self
    }

    fn point(&self, t: f32, r: f32, lift: f32, j: u32) -> Vec3 {
        let v = self.axis.cross(self.u);
        let th = j as f32 * TAU / self.sides as f32;
        self.o
            + self.axis * t
            + self.u * lift
            + (self.u * (th.cos() * self.su) + v * (th.sin() * self.sv)) * r
    }

    /// Draws `prof` = (t, r, lift); `slot(i, j)` is the slot of the band
    /// from ring `i` to `i + 1`, side `j`.
    fn draw(
        &self,
        p: &mut Parts,
        prof: &[(f32, f32, f32)],
        slot: impl Fn(usize, u32) -> &'static str,
        cap: &'static str,
    ) {
        let n = self.sides;
        let ring = |i: usize| -> Vec<Vec3> {
            let (t, r, l) = prof[i];
            (0..n).map(|j| self.point(t, r, l, j)).collect()
        };
        for i in 0..prof.len() - 1 {
            let (a, b) = (ring(i), ring(i + 1));
            for j in 0..n as usize {
                let k = (j + 1) % n as usize;
                let s = p.slot(slot(i, j as u32));
                tri(s, a[j], a[k], b[k]);
                tri(s, a[j], b[k], b[j]);
            }
        }
        if prof[0].1 > 0.0 {
            poly(p.slot(cap), &ring(0), -self.axis);
        }
        if prof[prof.len() - 1].1 > 0.0 {
            poly(p.slot(cap), &ring(prof.len() - 1), self.axis);
        }
    }
}

/// A perpendicular of `d` (deterministic).
fn perp(d: Vec3) -> Vec3 {
    let d = d.normalize();
    let up = if d.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let side = d.cross(up).normalize();
    side.cross(d).normalize()
}

/// A round bar from `a` to `b` of radius `r` with flat ends.
fn rod(p: &mut Parts, slot: &'static str, a: Vec3, b: Vec3, r: f32, sides: u32) {
    let d = b - a;
    let l = Lathe::new(a, d, perp(d), sides);
    l.draw(
        p,
        &[
            (0.0, 0.0, 0.0),
            (0.0, r, 0.0),
            (d.length(), r, 0.0),
            (d.length(), 0.0, 0.0),
        ],
        |_, _| slot,
        slot,
    );
}

/// A tyre about the X axis at `centre` (`side`: the outer face's sign of
/// X), with a hub on the outer face when `hub`.
fn wheel(p: &mut Parts, centre: Vec3, side: f32, r: f32, w: f32, sides: u32, hub: bool) {
    let axis = Vec3::X * side;
    let l = Lathe::new(centre, axis, Vec3::Y, sides);
    let h = w / 2.0;
    l.draw(
        p,
        &[(-h, 0.0, 0.0), (-h, r, 0.0), (h, r, 0.0), (h, 0.0, 0.0)],
        |_, _| "tyre",
        "tyre",
    );
    if hub {
        let hub = Lathe::new(centre, axis, Vec3::Y, 8);
        let rh = r * 0.62;
        hub.draw(
            p,
            &[
                (h, 0.0, 0.0),
                (h, rh, 0.0),
                (h + 0.012, rh * 0.85, 0.0),
                (h + 0.012, 0.0, 0.0),
            ],
            |_, _| "rim",
            "rim",
        );
    }
}

/// A dark wheel arch on a body side at `x` (its outward sign `side`)
/// over the wheel at (`y`, `z`) of radius `r`: half a ring above it.
fn arch(s: &mut Shape, x: f32, side: f32, y: f32, z: f32, r: f32, width: f32) {
    let steps = 6;
    for k in 0..steps {
        let a0 = PI * k as f32 / steps as f32;
        let a1 = PI * (k + 1) as f32 / steps as f32;
        let p = |a: f32, rr: f32| v3(x, y + a.sin() * rr, z + a.cos() * rr);
        poly(
            s,
            &[
                p(a0, r + 0.01),
                p(a0, r + width),
                p(a1, r + width),
                p(a1, r + 0.01),
            ],
            Vec3::X * side,
        );
    }
}

/// A body loft through sections at `z` (front first): each an octagon
/// from `yb` to `yt`, `±w` wide, chamfered by `cb` at the bottom and `ct`
/// at the top (far: a hexagon without the bottom chamfer).
#[derive(Debug, Clone, Copy)]
struct Section {
    z: f32,
    yb: f32,
    yt: f32,
    w: f32,
}

const fn sec(z: f32, yb: f32, yt: f32, w: f32) -> Section {
    Section { z, yb, yt, w }
}

fn loft(s: &mut Shape, secs: &[Section], cb: f32, ct: f32, far: bool) {
    let ring = |c: &Section| -> Vec<Vec3> {
        let (w, z) = (c.w, c.z);
        if far {
            vec![
                v3(-w, c.yb, z),
                v3(w, c.yb, z),
                v3(w, c.yt - ct, z),
                v3(w - ct, c.yt, z),
                v3(-w + ct, c.yt, z),
                v3(-w, c.yt - ct, z),
            ]
        } else {
            vec![
                v3(-w + cb, c.yb, z),
                v3(w - cb, c.yb, z),
                v3(w, c.yb + cb, z),
                v3(w, c.yt - ct, z),
                v3(w - ct, c.yt, z),
                v3(-w + ct, c.yt, z),
                v3(-w, c.yt - ct, z),
                v3(-w, c.yb + cb, z),
            ]
        }
    };
    let rings: Vec<Vec<Vec3>> = secs.iter().map(ring).collect();
    let mid = |r: &[Vec3]| r.iter().copied().sum::<Vec3>() / r.len() as f32;
    for w in rings.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        let centre = (mid(a) + mid(b)) / 2.0;
        for k in 0..a.len() {
            let n = (k + 1) % a.len();
            let quad = [a[k], a[n], b[n], b[k]];
            let fm = quad.iter().copied().sum::<Vec3>() / 4.0;
            poly(s, &quad, fm - centre);
        }
    }
    let (first, last) = (&rings[0], &rings[rings.len() - 1]);
    poly(s, first, mid(first) - mid(&rings[1]));
    poly(s, last, mid(last) - mid(&rings[rings.len() - 2]));
}

/// A thin wing panel from a root chord to a tip chord: (x, y, z of the
/// leading edge, z of the trailing edge, thickness) at each end.
fn wing(p: &mut Parts, slot: [&'static str; 6], root: [f32; 5], tip: [f32; 5]) {
    let [rx, ry, rl, rt, rh] = root;
    let [tx, ty, tl, tt, th] = tip;
    let c = [
        v3(rx, ry - rh / 2.0, rl),
        v3(tx, ty - th / 2.0, tl),
        v3(tx, ty - th / 2.0, tt),
        v3(rx, ry - rh / 2.0, rt),
        v3(rx, ry + rh / 2.0, rl),
        v3(tx, ty + th / 2.0, tl),
        v3(tx, ty + th / 2.0, tt),
        v3(rx, ry + rh / 2.0, rt),
    ];
    hexa(p, slot, &c);
}

/// A vertical fin: its root chord at `y0` (z `l0..t0`), its tip at `y1`
/// (z `l1..t1`), thickness `h0` → `h1`, centred on `x`.
#[allow(clippy::too_many_arguments)]
fn fin(
    p: &mut Parts,
    slot: &'static str,
    x: f32,
    y0: f32,
    l0: f32,
    t0: f32,
    h0: f32,
    y1: f32,
    l1: f32,
    t1: f32,
    h1: f32,
) {
    let c = [
        v3(x - h0 / 2.0, y0, l0),
        v3(x + h0 / 2.0, y0, l0),
        v3(x + h0 / 2.0, y0, t0),
        v3(x - h0 / 2.0, y0, t0),
        v3(x - h1 / 2.0, y1, l1),
        v3(x + h1 / 2.0, y1, l1),
        v3(x + h1 / 2.0, y1, t1),
        v3(x - h1 / 2.0, y1, t1),
    ];
    hexa(p, all(slot), &c);
}

/// Both sides: `f(-1)` and `f(1)`.
fn sides(mut f: impl FnMut(f32)) {
    f(-1.0);
    f(1.0);
}

/// `l` / `r` for a side.
fn lr(side: f32) -> &'static str {
    if side < 0.0 { "l" } else { "r" }
}

// ---------------------------------------------------------------------
// Cars.

struct CarSpec {
    secs: [Section; 5],
    cb: f32,
    ct: f32,
    /// The greenhouse: bottom z0, z1, y, half width; top z0, z1, y, half
    /// width.
    cabin: [f32; 8],
    axle: f32,
    r: f32,
    tyre_w: f32,
    /// The wheels' outer faces (±x).
    track_out: f32,
    head_y: f32,
    brake_y: f32,
    roof_rails: bool,
    paint: u32,
}

const HATCHBACK: CarSpec = CarSpec {
    secs: [
        sec(-2.02, 0.30, 0.66, 0.80),
        sec(-1.78, 0.22, 0.80, 0.86),
        sec(-0.78, 0.20, 0.92, 0.875),
        sec(1.62, 0.22, 0.97, 0.875),
        sec(1.98, 0.32, 0.94, 0.84),
    ],
    cb: 0.05,
    ct: 0.07,
    cabin: [-0.82, 1.86, 0.90, 0.84, -0.06, 1.76, 1.47, 0.69],
    axle: 1.275,
    r: 0.31,
    tyre_w: 0.2,
    track_out: 0.90,
    head_y: 0.56,
    brake_y: 0.84,
    roof_rails: false,
    paint: 0x9E2B25,
};

const SALOON: CarSpec = CarSpec {
    secs: [
        sec(-2.40, 0.30, 0.66, 0.84),
        sec(-2.12, 0.23, 0.80, 0.89),
        sec(-0.95, 0.21, 0.90, 0.905),
        sec(1.95, 0.23, 0.95, 0.905),
        sec(2.35, 0.34, 0.93, 0.86),
    ],
    cb: 0.05,
    ct: 0.07,
    cabin: [-0.98, 1.62, 0.88, 0.87, -0.12, 0.98, 1.44, 0.70],
    axle: 1.425,
    r: 0.32,
    tyre_w: 0.21,
    track_out: 0.93,
    head_y: 0.56,
    brake_y: 0.82,
    roof_rails: false,
    paint: 0x2E3F5C,
};

const SUV: CarSpec = CarSpec {
    secs: [
        sec(-2.30, 0.44, 0.86, 0.86),
        sec(-2.06, 0.38, 1.00, 0.93),
        sec(-0.86, 0.36, 1.10, 0.94),
        sec(1.92, 0.38, 1.12, 0.94),
        sec(2.30, 0.46, 1.08, 0.90),
    ],
    cb: 0.07,
    ct: 0.08,
    cabin: [-0.90, 2.22, 1.08, 0.92, -0.16, 2.14, 1.72, 0.80],
    axle: 1.375,
    r: 0.37,
    tyre_w: 0.24,
    track_out: 0.97,
    head_y: 0.78,
    brake_y: 1.02,
    roof_rails: true,
    paint: 0x6B6F73,
};

fn car(b: &mut Build, s: &CarSpec) {
    let far = b.far();
    b.paint("body", s.paint);
    let front = s.secs[0];
    let rear = s.secs[4];
    // The body and the greenhouse (roof in the body colour, glass round).
    loft(b.root().slot("body"), &s.secs, s.cb, s.ct, far);
    let [z0, z1, y0, w0, tz0, tz1, y1, w1] = s.cabin;
    let cab = slab(z0, z1, y0, w0, tz0, tz1, y1, w1);
    hexa(
        b.root(),
        ["body", "body", "glass", "glass", "glass", "glass"],
        &cab,
    );
    if !far {
        // The pillars between the side windows.
        for face in [LEFT, RIGHT] {
            let u = if face == RIGHT { 0.47 } else { 0.53 };
            decal(
                b.root().slot("body"),
                &cab,
                face,
                u - 0.03,
                u + 0.03,
                0.0,
                1.0,
                0.004,
            );
        }
        // Bumpers, grille, plates, mirrors, the arches.
        let fb = v3(0.0, front.yb + 0.09, front.z - 0.025);
        cuboid(b.root(), all("trim"), fb, v3(front.w - 0.03, 0.08, 0.04));
        let rb = v3(0.0, rear.yb + 0.09, rear.z + 0.025);
        cuboid(b.root(), all("trim"), rb, v3(rear.w - 0.03, 0.08, 0.04));
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, s.head_y - 0.01, front.z - 0.008),
            v3(0.26, 0.05, 0.012),
        );
        cuboid(
            b.root(),
            all("plate"),
            v3(0.0, front.yb + 0.09, front.z - 0.07),
            v3(0.25, 0.055, 0.006),
        );
        cuboid(
            b.root(),
            all("plate"),
            v3(0.0, s.brake_y - 0.24, rear.z + 0.008),
            v3(0.25, 0.055, 0.006),
        );
        sides(|x| {
            cuboid(
                b.root(),
                ["trim", "body", "body", "body", "glass", "body"],
                v3(x * (w0 + 0.08), y0 + 0.1, z0 + 0.3),
                v3(0.07, 0.05, 0.035),
            );
            for z in [-s.axle, s.axle] {
                let side_w = s.secs[2].w;
                arch(
                    b.root().slot("trim"),
                    x * (side_w + 0.003),
                    x,
                    s.r,
                    z,
                    s.r,
                    0.06,
                );
            }
        });
        if s.roof_rails {
            sides(|x| {
                cuboid(
                    b.root(),
                    all("trim"),
                    v3(x * (w1 - 0.08), y1 + 0.03, (tz0 + tz1) / 2.0),
                    v3(0.025, 0.03, (tz1 - tz0) / 2.0 - 0.15),
                );
            });
        }
    }
    // Wheels.
    for (name, x, z) in [
        ("wheel_fl", -1.0, -s.axle),
        ("wheel_fr", 1.0, -s.axle),
        ("wheel_rl", -1.0, s.axle),
        ("wheel_rr", 1.0, s.axle),
    ] {
        let c = v3(x * (s.track_out - s.tyre_w / 2.0), s.r, z);
        b.wheel(name, ROOT, c, x, s.r, s.tyre_w);
    }
    // Lamps.
    sides(|x| {
        let side = lr(x);
        b.lamp(
            &format!("light_head_{side}"),
            ROOT,
            "light_head",
            v3(x * front.w * 0.62, s.head_y, front.z - 0.012),
            v3(0.15, 0.045, 0.014),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_f{side}"),
            ROOT,
            "light_indicator",
            v3(x * (front.w - 0.1), s.head_y, front.z - 0.01),
            v3(0.035, 0.03, 0.012),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_brake_{side}"),
            ROOT,
            "light_brake",
            v3(x * rear.w * 0.74, s.brake_y, rear.z + 0.012),
            v3(0.12, 0.05, 0.014),
            Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_r{side}"),
            ROOT,
            "light_indicator",
            v3(x * rear.w * 0.74, s.brake_y - 0.095, rear.z + 0.01),
            v3(0.08, 0.025, 0.012),
            Vec3::Z,
        );
    });
}

// ---------------------------------------------------------------------
// The panel van.

fn van(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xE6E7E3);
    let secs = [
        sec(-2.80, 0.34, 0.82, 0.92),
        sec(-2.55, 0.28, 1.00, 0.99),
        sec(-1.90, 0.27, 1.12, 1.00),
        sec(2.55, 0.27, 1.12, 1.00),
        sec(2.62, 0.32, 1.10, 0.98),
    ];
    loft(b.root().slot("body"), &secs, 0.05, 0.07, far);
    let upper = slab(-1.96, 2.62, 1.08, 1.0, -1.18, 2.62, 2.42, 1.0);
    hexa(
        b.root(),
        ["body", "body", "glass", "body", "body", "body"],
        &upper,
    );
    if !far {
        for face in [LEFT, RIGHT] {
            let (u0, u1) = if face == RIGHT {
                (0.0, 0.17)
            } else {
                (0.83, 1.0)
            };
            decal(
                b.root().slot("glass"),
                &upper,
                face,
                u0 + 0.01,
                u1 - 0.01,
                0.28,
                0.86,
                0.004,
            );
            // The sliding door's seam on the right.
            if face == RIGHT {
                decal(
                    b.root().slot("trim"),
                    &upper,
                    face,
                    0.25,
                    0.257,
                    0.02,
                    0.93,
                    0.004,
                );
                decal(
                    b.root().slot("trim"),
                    &upper,
                    face,
                    0.47,
                    0.477,
                    0.02,
                    0.93,
                    0.004,
                );
            }
        }
        // The rear doors' seam.
        decal(
            b.root().slot("trim"),
            &upper,
            REAR,
            0.497,
            0.503,
            0.0,
            0.95,
            0.004,
        );
        let front = secs[0];
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, front.yb + 0.08, front.z - 0.02),
            v3(front.w - 0.02, 0.09, 0.04),
        );
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.42, 2.66),
            v3(0.97, 0.09, 0.04),
        );
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.66, front.z - 0.006),
            v3(0.34, 0.08, 0.012),
        );
        cuboid(
            b.root(),
            all("plate"),
            v3(0.0, front.yb + 0.08, front.z - 0.065),
            v3(0.25, 0.055, 0.006),
        );
        cuboid(
            b.root(),
            all("plate"),
            v3(0.0, 0.42, 2.705),
            v3(0.25, 0.055, 0.006),
        );
        sides(|x| {
            cuboid(
                b.root(),
                ["trim", "trim", "trim", "trim", "glass", "trim"],
                v3(x * 1.1, 1.62, -1.62),
                v3(0.09, 0.12, 0.035),
            );
            for z in [-1.7, 1.7] {
                arch(b.root().slot("trim"), x * 1.003, x, 0.34, z, 0.34, 0.07);
            }
        });
    }
    for (name, x, z) in [
        ("wheel_fl", -1.0, -1.7),
        ("wheel_fr", 1.0, -1.7),
        ("wheel_rl", -1.0, 1.7),
        ("wheel_rr", 1.0, 1.7),
    ] {
        b.wheel(name, ROOT, v3(x * 0.92, 0.34, z), x, 0.34, 0.22);
    }
    sides(|x| {
        let side = lr(x);
        b.lamp(
            &format!("light_head_{side}"),
            ROOT,
            "light_head",
            v3(x * 0.6, 0.66, -2.812),
            v3(0.17, 0.06, 0.014),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_f{side}"),
            ROOT,
            "light_indicator",
            v3(x * 0.83, 0.66, -2.81),
            v3(0.04, 0.035, 0.012),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_brake_{side}"),
            ROOT,
            "light_brake",
            v3(x * 0.9, 0.92, 2.632),
            v3(0.06, 0.14, 0.012),
            Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_r{side}"),
            ROOT,
            "light_indicator",
            v3(x * 0.9, 0.7, 2.632),
            v3(0.06, 0.05, 0.012),
            Vec3::Z,
        );
    });
}

// ---------------------------------------------------------------------
// Lorries.

/// A lorry's cab (front at `zf`, back at `zb`, roof at `top`), its
/// bumper and front lamps; returns the cab's corners.
fn lorry_cab(b: &mut Build, zf: f32, zb: f32, top: f32) -> [Vec3; 8] {
    let far = b.far();
    let cab = slab(zf, zb, 0.55, 1.25, zf + 0.12, zb, top, 1.2);
    hexa(
        b.root(),
        ["trim", "body", "body", "body", "body", "body"],
        &cab,
    );
    let lift = 0.006;
    decal(
        b.root().slot("glass"),
        &cab,
        FRONT,
        0.06,
        0.94,
        0.5,
        0.86,
        lift,
    );
    if !far {
        decal(
            b.root().slot("trim"),
            &cab,
            FRONT,
            0.22,
            0.78,
            0.18,
            0.42,
            lift,
        );
        decal(
            b.root().slot("glass"),
            &cab,
            RIGHT,
            0.03,
            0.42,
            0.5,
            0.86,
            lift,
        );
        decal(
            b.root().slot("glass"),
            &cab,
            LEFT,
            0.58,
            0.97,
            0.5,
            0.86,
            lift,
        );
        // Steps and the door's seam.
        decal(
            b.root().slot("trim"),
            &cab,
            RIGHT,
            0.5,
            0.51,
            0.1,
            0.92,
            lift,
        );
        decal(
            b.root().slot("trim"),
            &cab,
            LEFT,
            0.49,
            0.5,
            0.1,
            0.92,
            lift,
        );
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.66, zf - 0.06),
            v3(1.24, 0.17, 0.06),
        );
        cuboid(
            b.root(),
            all("plate"),
            v3(0.0, 0.62, zf - 0.125),
            v3(0.26, 0.06, 0.006),
        );
        sides(|x| {
            // Mirrors on their arms.
            cuboid(
                b.root(),
                all("trim"),
                v3(x * 1.36, 2.05, zf + 0.25),
                v3(0.11, 0.02, 0.02),
            );
            cuboid(
                b.root(),
                ["trim", "trim", "trim", "trim", "glass", "trim"],
                v3(x * 1.48, 1.85, zf + 0.25),
                v3(0.05, 0.25, 0.04),
            );
        });
    }
    sides(|x| {
        let side = lr(x);
        b.lamp(
            &format!("light_head_{side}"),
            ROOT,
            "light_head",
            v3(x * 0.92, 0.7, zf - 0.135),
            v3(0.18, 0.07, 0.015),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_f{side}"),
            ROOT,
            "light_indicator",
            v3(x * 1.17, 0.7, zf - 0.133),
            v3(0.05, 0.06, 0.013),
            -Vec3::Z,
        );
    });
    cab
}

/// The rear lamp bar at `z` below `parent`, with the brake lamps and the
/// rear indicators.
fn rear_lamps(b: &mut Build, parent: &str, z: f32, y: f32) {
    if !b.far() {
        let p = b.on(parent);
        cuboid(p, all("trim"), v3(0.0, y, z - 0.03), v3(1.22, 0.1, 0.03));
        cuboid(
            p,
            all("plate"),
            v3(0.0, y - 0.2, z - 0.02),
            v3(0.26, 0.06, 0.006),
        );
        cuboid(
            p,
            all("trim"),
            v3(0.0, y - 0.2, z - 0.06),
            v3(1.0, 0.06, 0.04),
        );
    }
    sides(|x| {
        let side = lr(x);
        b.lamp(
            &format!("light_brake_{side}"),
            parent,
            "light_brake",
            v3(x * 0.98, y, z + 0.012),
            v3(0.16, 0.07, 0.014),
            Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_r{side}"),
            parent,
            "light_indicator",
            v3(x * 0.66, y, z + 0.012),
            v3(0.1, 0.07, 0.014),
            Vec3::Z,
        );
    });
}

/// A cargo box, `livery` round, with its reflectors and door seams.
fn cargo_box(b: &mut Build, parent: &str, z0: f32, z1: f32, y0: f32, y1: f32) {
    let far = b.far();
    let c = slab(z0, z1, y0, 1.27, z0, z1, y1, 1.27);
    let p = b.on(parent);
    hexa(
        p,
        ["trim", "livery", "livery", "livery", "livery", "livery"],
        &c,
    );
    if !far {
        decal(p.slot("trim"), &c, REAR, 0.497, 0.503, 0.02, 0.98, 0.004);
        for face in [LEFT, RIGHT] {
            decal(p.slot("trim"), &c, face, 0.0, 1.0, 0.0, 0.03, 0.004);
            for k in 0..4 {
                let u = 0.12 + 0.25 * k as f32;
                decal(
                    p.slot("reflector"),
                    &c,
                    face,
                    u,
                    u + 0.012,
                    0.05,
                    0.08,
                    0.006,
                );
            }
        }
    }
}

fn lorry_rigid(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xD9D9D4);
    b.paint("livery", 0xF2F2EE);
    lorry_cab(b, -3.8, -1.55, 3.05);
    cargo_box(b, ROOT, -1.45, 4.4, 1.0, 3.65);
    if !far {
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.72, 1.4),
            v3(0.45, 0.17, 2.95),
        );
        cuboid(
            b.root(),
            all("metal"),
            v3(-0.75, 0.72, 0.2),
            v3(0.28, 0.24, 0.5),
        );
        // Mudguards over the rear wheels.
        sides(|x| {
            cuboid(
                b.root(),
                all("trim"),
                v3(x * 1.13, 1.08, 2.5),
                v3(0.16, 0.025, 0.62),
            );
        });
    }
    rear_lamps(b, ROOT, 4.44, 0.78);
    for (name, x, z, w) in [
        ("wheel_fl", -1.0, -2.5, 0.32),
        ("wheel_fr", 1.0, -2.5, 0.32),
        ("wheel_rl", -1.0, 2.5, 0.5),
        ("wheel_rr", 1.0, 2.5, 0.5),
    ] {
        b.wheel(name, ROOT, v3(x * (1.28 - w / 2.0), 0.52, z), x, 0.52, w);
    }
}

fn lorry_articulated(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0x2F5D8A);
    b.paint("livery", 0xEDEDE8);
    lorry_cab(b, -3.15, -0.95, 3.25);
    // The roof fairing.
    let fairing = slab(-2.95, -0.95, 3.24, 1.16, -2.0, -0.95, 3.82, 1.1);
    hexa(b.root(), all("body"), &fairing);
    if !far {
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.78, 0.0),
            v3(0.45, 0.18, 2.9),
        );
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 1.17, 1.6),
            v3(0.62, 0.05, 0.62),
        );
        cuboid(
            b.root(),
            all("metal"),
            v3(-0.82, 0.78, -0.1),
            v3(0.3, 0.27, 0.55),
        );
        sides(|x| {
            cuboid(
                b.root(),
                all("trim"),
                v3(x * 1.13, 1.1, 1.85),
                v3(0.16, 0.025, 0.6),
            );
        });
    }
    for (name, x, w) in [("wheel_fl", -1.0, 0.32), ("wheel_fr", 1.0, 0.32)] {
        b.wheel(
            name,
            ROOT,
            v3(x * (1.28 - w / 2.0), 0.52, -1.85),
            x,
            0.52,
            w,
        );
    }
    for (name, x, w) in [("wheel_rl", -1.0, 0.5), ("wheel_rr", 1.0, 0.5)] {
        b.wheel(name, ROOT, v3(x * (1.28 - w / 2.0), 0.52, 1.85), x, 0.52, w);
    }
    // The semi-trailer turns about its kingpin over the fifth wheel.
    if !far {
        b.node("trailer", ROOT, v3(0.0, 1.25, 1.6));
    }
    cargo_box(b, "trailer", 0.0, 13.6, 1.3, 4.0);
    if !far {
        let p = b.on("trailer");
        cuboid(p, all("trim"), v3(0.0, 1.15, 11.0), v3(0.5, 0.15, 2.5));
        sides(|x| {
            rod(
                p,
                "metal",
                v3(x * 0.9, 1.3, 3.0),
                v3(x * 0.9, 0.12, 3.0),
                0.06,
                6,
            );
            cuboid(p, all("trim"), v3(x * 0.9, 0.08, 3.0), v3(0.12, 0.04, 0.12));
            cuboid(
                p,
                all("trim"),
                v3(x * 1.1, 1.08, 11.6),
                v3(0.18, 0.025, 1.95),
            );
        });
    }
    rear_lamps(b, "trailer", 13.64, 0.98);
    for (k, z) in [10.3f32, 11.6, 12.9].into_iter().enumerate() {
        sides(|x| {
            let name = format!("wheel_r{}_{}", k + 2, lr(x));
            b.wheel(
                &name,
                "trailer",
                v3(x * (1.27 - 0.17), 0.5, z),
                x,
                0.5,
                0.34,
            );
        });
    }
}

// ---------------------------------------------------------------------
// The city bus.

fn bus(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xE9E9E4);
    b.paint("livery", 0x1F7A8C);
    let c = slab(-5.7, 6.3, 0.32, 1.275, -5.62, 6.25, 3.05, 1.24);
    hexa(
        b.root(),
        ["trim", "body", "livery", "livery", "livery", "livery"],
        &c,
    );
    let lift = 0.006;
    decal(
        b.root().slot("glass"),
        &c,
        FRONT,
        0.04,
        0.96,
        0.3,
        0.9,
        lift,
    );
    decal(
        b.root().slot("glass"),
        &c,
        RIGHT,
        0.12,
        0.97,
        0.38,
        0.88,
        lift,
    );
    decal(
        b.root().slot("glass"),
        &c,
        LEFT,
        0.03,
        0.95,
        0.38,
        0.88,
        lift,
    );
    if !far {
        decal(
            b.root().slot("trim"),
            &c,
            FRONT,
            0.08,
            0.92,
            0.915,
            0.975,
            lift,
        );
        decal(b.root().slot("glass"), &c, REAR, 0.1, 0.9, 0.56, 0.88, lift);
        decal(b.root().slot("trim"), &c, REAR, 0.2, 0.8, 0.14, 0.36, lift);
        for face in [LEFT, RIGHT] {
            decal(b.root().slot("body"), &c, face, 0.0, 1.0, 0.0, 0.1, lift);
            // The window pillars.
            for k in 1..8 {
                let u = 0.12 + (0.97 - 0.12) * k as f32 / 8.0;
                let u = if face == LEFT { 1.0 - u } else { u };
                decal(
                    b.root().slot("livery"),
                    &c,
                    face,
                    u - 0.006,
                    u + 0.006,
                    0.38,
                    0.88,
                    lift + 0.002,
                );
            }
        }
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.48, -5.74),
            v3(1.25, 0.14, 0.05),
        );
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.48, 6.34),
            v3(1.25, 0.14, 0.05),
        );
        cuboid(
            b.root(),
            all("plate"),
            v3(0.0, 0.48, -5.795),
            v3(0.26, 0.06, 0.006),
        );
        cuboid(
            b.root(),
            all("plate"),
            v3(0.0, 0.48, 6.395),
            v3(0.26, 0.06, 0.006),
        );
        cuboid(
            b.root(),
            all("body"),
            v3(0.0, 3.16, 0.8),
            v3(0.8, 0.12, 1.4),
        );
        sides(|x| {
            cuboid(
                b.root(),
                all("trim"),
                v3(x * 1.38, 2.35, -5.55),
                v3(0.1, 0.02, 0.02),
            );
            cuboid(
                b.root(),
                ["trim", "trim", "trim", "trim", "glass", "trim"],
                v3(x * 1.47, 2.1, -5.55),
                v3(0.04, 0.26, 0.04),
            );
            for z in [-3.0, 3.0] {
                arch(b.root().slot("trim"), x * 1.278, x, 0.5, z, 0.5, 0.08);
            }
        });
    }
    // The doors on the right (the kerb side in DE, CH, LI, ES): glass
    // leaves the game opens.
    for (name, u0, u1) in [("door_front", 0.025, 0.11), ("door_mid", 0.44, 0.535)] {
        let at = face_point(&c, RIGHT, (u0 + u1) / 2.0, 0.45, 0.01);
        let p = b.part(name, ROOT, at);
        decal(p.slot("glass"), &c, RIGHT, u0, u1, 0.03, 0.88, 0.012);
        if !far {
            decal(
                p.slot("trim"),
                &c,
                RIGHT,
                (u0 + u1) / 2.0 - 0.002,
                (u0 + u1) / 2.0 + 0.002,
                0.03,
                0.88,
                0.014,
            );
            decal(p.slot("trim"), &c, RIGHT, u0, u1, 0.44, 0.46, 0.014);
        }
    }
    for (name, x, z, w) in [
        ("wheel_fl", -1.0, -3.0, 0.3),
        ("wheel_fr", 1.0, -3.0, 0.3),
        ("wheel_rl", -1.0, 3.0, 0.42),
        ("wheel_rr", 1.0, 3.0, 0.42),
    ] {
        b.wheel(name, ROOT, v3(x * (1.29 - w / 2.0), 0.5, z), x, 0.5, w);
    }
    sides(|x| {
        let side = lr(x);
        b.lamp(
            &format!("light_head_{side}"),
            ROOT,
            "light_head",
            v3(x * 0.9, 0.7, -5.72),
            v3(0.2, 0.07, 0.014),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_f{side}"),
            ROOT,
            "light_indicator",
            v3(x * 1.16, 0.7, -5.718),
            v3(0.05, 0.06, 0.012),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_brake_{side}"),
            ROOT,
            "light_brake",
            v3(x * 1.08, 1.05, 6.32),
            v3(0.1, 0.16, 0.014),
            Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_r{side}"),
            ROOT,
            "light_indicator",
            v3(x * 1.08, 0.78, 6.318),
            v3(0.1, 0.06, 0.012),
            Vec3::Z,
        );
    });
}

// ---------------------------------------------------------------------
// The motorbike and its rider.

fn motorbike(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0x2B4F7A);
    let head = v3(0.0, 0.98, -0.5);
    let axle_f = v3(0.0, 0.31, -0.725);
    if !far {
        b.node("wheel_front", ROOT, axle_f);
    }
    {
        let p = b.on("wheel_front");
        wheel(p, axle_f, 1.0, 0.31, 0.13, if far { 6 } else { 12 }, !far);
        if !far {
            let hub = Lathe::new(axle_f, -Vec3::X, Vec3::Y, 8);
            hub.draw(
                p,
                &[
                    (0.065, 0.0, 0.0),
                    (0.065, 0.18, 0.0),
                    (0.077, 0.15, 0.0),
                    (0.077, 0.0, 0.0),
                ],
                |_, _| "rim",
                "rim",
            );
        }
    }
    let axle_r = v3(0.0, 0.31, 0.725);
    b.wheel("wheel_rear", ROOT, axle_r, 1.0, 0.31, 0.17);
    let p = b.root();
    // The engine, the frame, the tank, the seat, the tail, the fairing.
    cuboid(p, all("metal"), v3(0.0, 0.45, -0.05), v3(0.18, 0.16, 0.25));
    let tank = slab(-0.5, 0.05, 0.76, 0.16, -0.42, -0.02, 1.0, 0.11);
    hexa(p, all("body"), &tank);
    let seat = slab(0.0, 0.62, 0.79, 0.15, 0.06, 0.58, 0.88, 0.13);
    hexa(p, all("trim"), &seat);
    let tail = slab(0.5, 0.92, 0.72, 0.1, 0.55, 0.88, 0.86, 0.07);
    hexa(p, all("body"), &tail);
    let fairing = slab(-0.9, -0.55, 0.7, 0.2, -0.8, -0.58, 1.12, 0.14);
    hexa(p, all("body"), &fairing);
    let screen = slab(-0.8, -0.74, 1.1, 0.13, -0.7, -0.64, 1.34, 0.11);
    hexa(p, all("glass"), &screen);
    if !far {
        sides(|x| {
            rod(
                p,
                "chrome",
                axle_f + Vec3::X * (x * 0.08),
                head + v3(x * 0.08, 0.0, 0.03),
                0.025,
                6,
            );
            rod(
                p,
                "trim",
                axle_r + Vec3::X * (x * 0.1),
                v3(x * 0.1, 0.42, 0.15),
                0.03,
                4,
            );
            cuboid(
                p,
                all("trim"),
                v3(x * 0.3, 1.2, -0.56),
                v3(0.05, 0.03, 0.015),
            );
            cuboid(
                p,
                all("trim"),
                v3(x * 0.2, 0.42, 0.12),
                v3(0.04, 0.015, 0.05),
            );
        });
        rod(p, "trim", head, v3(0.0, 0.76, 0.2), 0.035, 6);
        rod(
            p,
            "trim",
            v3(-0.33, 1.03, -0.48),
            v3(0.33, 1.03, -0.48),
            0.016,
            6,
        );
        rod(
            p,
            "metal",
            v3(0.17, 0.3, -0.05),
            v3(0.2, 0.46, 0.88),
            0.05,
            8,
        );
        let guard = slab(-0.92, -0.54, 0.63, 0.07, -0.88, -0.56, 0.67, 0.07);
        hexa(p, all("body"), &guard);
        cuboid(p, all("plate"), v3(0.0, 0.66, 0.95), v3(0.1, 0.06, 0.005));
    }
    b.lamp(
        "light_head",
        ROOT,
        "light_head",
        v3(0.0, 0.93, -0.905),
        v3(0.09, 0.06, 0.015),
        -Vec3::Z,
    );
    b.lamp(
        "light_brake",
        ROOT,
        "light_brake",
        v3(0.0, 0.8, 0.932),
        v3(0.07, 0.035, 0.014),
        Vec3::Z,
    );
    sides(|x| {
        let side = lr(x);
        b.lamp(
            &format!("light_indicator_f{side}"),
            ROOT,
            "light_indicator",
            v3(x * 0.17, 0.92, -0.8),
            v3(0.03, 0.02, 0.025),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_r{side}"),
            ROOT,
            "light_indicator",
            v3(x * 0.13, 0.78, 0.9),
            v3(0.03, 0.02, 0.025),
            Vec3::Z,
        );
    });
    rider(b, v3(0.0, 0.88, 0.25));
}

/// The rider placeholder (18B: our generic kit; tinted by the rider-look
/// slots `jersey`, `shorts`, `helmet`, `glove`, `shoe`), seated at
/// `seat`, leaning to the bars.
fn rider(b: &mut Build, seat: Vec3) {
    let far = b.far();
    let p = b.part("rider", ROOT, seat);
    let torso = slab(0.1, 0.38, 0.92, 0.17, -0.16, 0.08, 1.42, 0.2);
    hexa(p, all("jersey"), &torso);
    let n = if far { 4 } else { 6 };
    let helmet = Lathe::new(
        v3(0.0, 1.43, -0.15),
        Vec3::Y,
        Vec3::Z,
        if far { 5 } else { 10 },
    );
    helmet.draw(
        p,
        &[
            (0.0, 0.0, 0.0),
            (0.02, 0.11, 0.0),
            (0.12, 0.15, 0.0),
            (0.24, 0.12, 0.0),
            (0.3, 0.0, 0.0),
        ],
        |_, _| "helmet",
        "helmet",
    );
    if far {
        sides(|x| {
            rod(
                p,
                "shorts",
                v3(x * 0.2, 0.92, -0.2),
                v3(x * 0.2, 0.5, 0.08),
                0.06,
                n,
            );
        });
        return;
    }
    cuboid(
        p,
        ["glass", "glass", "glass", "glass", "helmet", "glass"],
        v3(0.0, 1.56, -0.28),
        v3(0.09, 0.045, 0.03),
    );
    sides(|x| {
        let shoulder = v3(x * 0.2, 1.36, -0.04);
        let elbow = v3(x * 0.27, 1.17, -0.27);
        let hand = v3(x * 0.31, 1.04, -0.47);
        rod(p, "jersey", shoulder, elbow, 0.055, n);
        rod(p, "jersey", elbow, hand, 0.045, n);
        cuboid(p, all("glove"), hand, v3(0.04, 0.045, 0.05));
        let hip = v3(x * 0.13, 0.95, 0.2);
        let knee = v3(x * 0.2, 0.92, -0.2);
        let ankle = v3(x * 0.2, 0.5, 0.08);
        rod(p, "shorts", hip, knee, 0.075, n);
        rod(p, "shorts", knee, ankle, 0.06, n);
        cuboid(
            p,
            all("shoe"),
            ankle + v3(0.0, -0.04, -0.04),
            v3(0.045, 0.05, 0.11),
        );
    });
}

// ---------------------------------------------------------------------
// The farm tractor.

fn tractor(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0x3F6E8C);
    b.paint("rim", 0x9A9DA0);
    let bonnet = slab(-2.25, -0.15, 0.62, 0.42, -2.15, -0.15, 1.38, 0.38);
    hexa(b.root(), all("body"), &bonnet);
    decal(
        b.root().slot("trim"),
        &bonnet,
        FRONT,
        0.15,
        0.85,
        0.1,
        0.62,
        0.005,
    );
    let cab = slab(-0.25, 1.35, 1.38, 0.62, -0.15, 1.3, 2.7, 0.6);
    hexa(
        b.root(),
        ["trim", "glass", "glass", "glass", "glass", "glass"],
        &cab,
    );
    let roof = slab(-0.32, 1.42, 2.7, 0.7, -0.28, 1.38, 2.86, 0.68);
    hexa(b.root(), all("body"), &roof);
    cuboid(
        b.root(),
        all("trim"),
        v3(0.0, 1.1, 0.55),
        v3(0.5, 0.3, 0.75),
    );
    sides(|x| {
        let fender =
            slab(0.48, 1.98, 1.62, 0.25, 0.6, 1.9, 1.72, 0.25).map(|c| c + Vec3::X * (x * 0.98));
        hexa(b.root(), all("body"), &fender);
    });
    if !far {
        // The pillars, the front weights, the exhaust, the hitch.
        for (x, z) in [(-1.0, -0.2), (1.0, -0.2), (-1.0, 1.32), (1.0, 1.32)] {
            cuboid(
                b.root(),
                all("body"),
                v3(x * 0.6, 2.04, z),
                v3(0.03, 0.66, 0.03),
            );
        }
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.56, -2.2),
            v3(0.36, 0.16, 0.22),
        );
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.62, -1.2),
            v3(0.3, 0.1, 1.0),
        );
        rod(
            b.root(),
            "metal",
            v3(0.28, 1.38, -0.4),
            v3(0.28, 2.95, -0.4),
            0.05,
            8,
        );
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.62, 1.95),
            v3(0.2, 0.08, 0.12),
        );
        sides(|x| {
            cuboid(
                b.root(),
                all("trim"),
                v3(x * 0.82, 1.0, -1.2),
                v3(0.15, 0.025, 0.32),
            );
            cuboid(
                b.root(),
                ["trim", "trim", "trim", "trim", "glass", "trim"],
                v3(x * 0.82, 2.25, -0.2),
                v3(0.03, 0.1, 0.06),
            );
        });
    }
    sides(|x| {
        let side = lr(x);
        let fx = x * (0.96 - 0.14);
        b.wheel(
            &format!("wheel_f{side}"),
            ROOT,
            v3(fx, 0.48, -1.2),
            x,
            0.48,
            0.28,
        );
        let rx = x * (1.21 - 0.23);
        b.wheel(
            &format!("wheel_r{side}"),
            ROOT,
            v3(rx, 0.82, 1.2),
            x,
            0.82,
            0.46,
        );
        b.lamp(
            &format!("light_head_{side}"),
            ROOT,
            "light_head",
            v3(x * 0.24, 1.18, -2.265),
            v3(0.08, 0.06, 0.015),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_f{side}"),
            ROOT,
            "light_indicator",
            v3(x * 0.64, 2.62, -0.33),
            v3(0.05, 0.04, 0.03),
            -Vec3::Z,
        );
        b.lamp(
            &format!("light_brake_{side}"),
            ROOT,
            "light_brake",
            v3(x * 1.05, 1.6, 2.0),
            v3(0.07, 0.06, 0.015),
            Vec3::Z,
        );
        b.lamp(
            &format!("light_indicator_r{side}"),
            ROOT,
            "light_indicator",
            v3(x * 0.86, 1.6, 2.0),
            v3(0.05, 0.05, 0.015),
            Vec3::Z,
        );
    });
}

// ---------------------------------------------------------------------
// Aircraft.

fn nav_lights(b: &mut Build, tip_l: Vec3, tip_r: Vec3, tail: Vec3, beacon: Vec3, size: f32) {
    let h = Vec3::splat(size);
    b.lamp("light_nav_l", ROOT, "light_nav_red", tip_l, h, -Vec3::X);
    b.lamp("light_nav_r", ROOT, "light_nav_green", tip_r, h, Vec3::X);
    b.lamp("light_tail", ROOT, "light_white", tail, h, Vec3::Z);
    b.lamp("light_beacon", ROOT, "light_nav_red", beacon, h, Vec3::Y);
}

fn small_plane(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xF1F1EC);
    b.paint("livery", 0xB3262E);
    let cy = 1.25;
    let n = if far { 6 } else { 10 };
    let f = Lathe::new(v3(0.0, cy, 0.0), Vec3::Z, Vec3::Y, n).squash(1.15, 0.95);
    let prof: Vec<(f32, f32, f32)> = if far {
        vec![
            (-3.85, 0.0, -0.05),
            (-3.0, 0.52, 0.0),
            (-0.2, 0.62, 0.08),
            (3.6, 0.17, 0.25),
            (4.3, 0.0, 0.3),
        ]
    } else {
        vec![
            (-3.85, 0.0, -0.05),
            (-3.75, 0.36, -0.05),
            (-3.0, 0.52, 0.0),
            (-1.9, 0.6, 0.08),
            (-0.2, 0.62, 0.08),
            (1.0, 0.5, 0.1),
            (3.6, 0.17, 0.25),
            (4.3, 0.0, 0.3),
        ]
    };
    f.draw(
        b.root(),
        &prof,
        |i, j| {
            if far {
                "body"
            } else if i == 2 && matches!(j, 0 | 1 | 9 | 8) || i == 3 && matches!(j, 2 | 7) {
                "glass"
            } else if j == 5 {
                "livery"
            } else {
                "body"
            }
        },
        "body",
    );
    let p = b.root();
    wing(
        p,
        all("body"),
        [-5.5, 2.06, -1.6, -0.15, 0.16],
        [5.5, 2.06, -1.6, -0.15, 0.16],
    );
    wing(
        p,
        all("body"),
        [-1.7, cy + 0.32, 3.4, 4.25, 0.08],
        [1.7, cy + 0.32, 3.55, 4.25, 0.06],
    );
    fin(
        p,
        "livery",
        0.0,
        cy + 0.2,
        3.0,
        4.32,
        0.1,
        2.85,
        3.9,
        4.42,
        0.06,
    );
    if !far {
        sides(|x| {
            rod(
                p,
                "metal",
                v3(x * 0.45, 0.85, -0.9),
                v3(x * 2.8, 2.0, -0.95),
                0.035,
                6,
            );
        });
        let pn = b.part("gear_nose", ROOT, v3(0.0, 0.22, -3.1));
        rod(
            pn,
            "metal",
            v3(0.0, 0.22, -3.1),
            v3(0.0, 0.85, -3.2),
            0.03,
            6,
        );
        wheel(pn, v3(0.0, 0.22, -3.1), 1.0, 0.22, 0.1, 10, true);
        // Wheel fairings (spats) over the upper wheels (18G).
        let spat = |p: &mut Parts, at: Vec3, r: f32| {
            let l = Lathe::new(at + Vec3::Y * 0.06, Vec3::Z, Vec3::Y, 8).squash(1.15, 0.55);
            l.draw(
                p,
                &[
                    (-0.42, 0.0, 0.0),
                    (-0.3, r * 0.6, 0.0),
                    (0.0, r * 0.85, 0.02),
                    (0.3, r * 0.6, 0.06),
                    (0.55, 0.0, 0.1),
                ],
                |_, _| "body",
                "body",
            );
        };
        spat(pn, v3(0.0, 0.22, -3.1), 0.22);
        sides(|x| {
            let at = v3(x * 1.15, 0.25, -0.4);
            let p = b.part(&format!("gear_main_{}", lr(x)), ROOT, at);
            rod(p, "metal", at, v3(x * 0.3, 0.75, -0.4), 0.035, 6);
            wheel(p, at, x, 0.25, 0.12, 10, true);
            spat(p, at, 0.25);
        });
        // The cowling's air inlets, the exhaust, the cabin door's outline.
        let p = b.root();
        sides(|x| {
            cuboid(
                p,
                all("trim"),
                v3(x * 0.25, cy - 0.2, -3.73),
                v3(0.11, 0.06, 0.02),
            );
        });
        rod(
            p,
            "metal",
            v3(0.22, cy - 0.5, -2.85),
            v3(0.26, cy - 0.62, -2.55),
            0.03,
            6,
        );
        let door = |p: &mut Parts, z0: f32, z1: f32, y0: f32, y1: f32| {
            let x = 0.57;
            for (a, bb, c, d) in [
                (z0, z1, y0, y0 + 0.02),
                (z0, z1, y1 - 0.02, y1),
                (z0, z0 + 0.02, y0, y1),
                (z1 - 0.02, z1, y0, y1),
            ] {
                poly(
                    p.slot("trim"),
                    &[v3(x, c, a), v3(x, c, bb), v3(x, d, bb), v3(x, d, a)],
                    Vec3::X,
                );
            }
        };
        door(p, -1.65, -0.75, cy - 0.22, cy + 0.22);
    }
    // The propeller: spinner and two blades, turning about Z.
    let hub = v3(0.0, cy - 0.05, -3.88);
    let p = b.part("prop", ROOT, hub);
    let spinner = Lathe::new(
        hub - Vec3::Z * 0.38,
        Vec3::Z,
        Vec3::Y,
        if far { 5 } else { 8 },
    );
    spinner.draw(
        p,
        &[(0.0, 0.0, 0.0), (0.3, 0.15, 0.0), (0.38, 0.17, 0.0)],
        |_, _| "paint_white",
        "paint_white",
    );
    if !far {
        for s in [-1.0f32, 1.0] {
            let rot = Quat::from_rotation_y(s * 0.2);
            obox(
                p,
                all("trim"),
                hub + v3(0.0, s * 0.55, -0.12),
                v3(0.04, 0.45, 0.012),
                rot,
            );
        }
    }
    nav_lights(
        b,
        v3(-5.52, 2.06, -1.0),
        v3(5.52, 2.06, -1.0),
        v3(0.0, 2.0, 4.45),
        v3(0.0, 2.9, 4.1),
        0.05,
    );
}

fn glider(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xF4F4F0);
    let cy = 0.62;
    let n = if far { 6 } else { 10 };
    let f = Lathe::new(v3(0.0, cy, 0.0), Vec3::Z, Vec3::Y, n).squash(1.1, 1.0);
    let prof: Vec<(f32, f32, f32)> = if far {
        vec![
            (-3.2, 0.0, -0.05),
            (-2.6, 0.36, 0.0),
            (0.4, 0.25, 0.1),
            (3.6, 0.0, 0.18),
        ]
    } else {
        vec![
            (-3.2, 0.0, -0.05),
            (-3.05, 0.22, -0.03),
            (-2.6, 0.36, 0.0),
            (-1.6, 0.38, 0.02),
            (-0.6, 0.36, 0.05),
            (0.4, 0.25, 0.1),
            (2.2, 0.13, 0.16),
            (3.4, 0.1, 0.18),
            (3.6, 0.0, 0.18),
        ]
    };
    f.draw(b.root(), &prof, |_, _| "body", "body");
    let canopy = Lathe::new(
        v3(0.0, cy + 0.25, 0.0),
        Vec3::Z,
        Vec3::Y,
        if far { 5 } else { 8 },
    )
    .squash(1.0, 0.75);
    canopy.draw(
        b.root(),
        &[
            (-2.75, 0.0, 0.0),
            (-2.4, 0.17, 0.0),
            (-1.6, 0.25, 0.0),
            (-0.6, 0.18, 0.0),
            (-0.3, 0.0, 0.0),
        ],
        |_, _| "glass",
        "glass",
    );
    let wy = cy + 0.24;
    sides(|x| {
        let p = b.root();
        wing(
            p,
            all("body"),
            [x * 0.2, wy, -0.75, 0.15, 0.14],
            [x * 4.6, wy + 0.13, -0.66, 0.04, 0.07],
        );
        wing(
            p,
            all("body"),
            [x * 4.6, wy + 0.13, -0.66, 0.04, 0.07],
            [x * 7.5, wy + 0.27, -0.5, -0.15, 0.03],
        );
    });
    let p = b.root();
    fin(
        p,
        "body",
        0.0,
        cy + 0.05,
        2.8,
        3.6,
        0.08,
        1.75,
        3.2,
        3.65,
        0.05,
    );
    wing(
        p,
        all("body"),
        [-1.3, 1.77, 3.2, 3.62, 0.05],
        [1.3, 1.77, 3.3, 3.62, 0.04],
    );
    if !far {
        // Winglets, the tow hook, the tail skid, the canopy's frame (18G).
        sides(|x| {
            let p = b.root();
            let c = [
                v3(x * 7.47, wy + 0.25, -0.5),
                v3(x * 7.6, wy + 0.62, -0.36),
                v3(x * 7.6, wy + 0.62, -0.2),
                v3(x * 7.47, wy + 0.25, -0.15),
                v3(x * 7.53, wy + 0.25, -0.5),
                v3(x * 7.64, wy + 0.62, -0.36),
                v3(x * 7.64, wy + 0.62, -0.2),
                v3(x * 7.53, wy + 0.25, -0.15),
            ];
            hexa(p, all("body"), &c);
        });
        let p = b.root();
        cuboid(
            p,
            all("trim"),
            v3(0.0, cy - 0.33, -2.95),
            v3(0.03, 0.03, 0.06),
        );
        cuboid(
            p,
            all("trim"),
            v3(0.0, cy - 0.12, 3.45),
            v3(0.025, 0.06, 0.12),
        );
        rod(
            p,
            "trim",
            v3(-0.26, cy + 0.27, -1.55),
            v3(0.26, cy + 0.27, -1.55),
            0.02,
            5,
        );
        let at = v3(0.0, 0.19, 0.0);
        let p = b.part("gear_main", ROOT, at);
        wheel(p, at, 1.0, 0.19, 0.1, 10, true);
        cuboid(
            b.root(),
            all("trim"),
            v3(0.0, 0.25, -2.3),
            v3(0.03, 0.05, 0.25),
        );
    }
}

fn helicopter(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xF2F2EE);
    b.paint("livery", 0xC8231E);
    let n = if far { 6 } else { 12 };
    let cabin = Lathe::new(v3(0.0, 1.45, -0.6), Vec3::Z, Vec3::Y, n).squash(1.0, 0.8);
    let prof: Vec<(f32, f32, f32)> = if far {
        vec![
            (-2.45, 0.0, -0.25),
            (-1.2, 1.0, 0.0),
            (1.2, 0.85, 0.15),
            (2.3, 0.0, 0.35),
        ]
    } else {
        vec![
            (-2.45, 0.0, -0.25),
            (-2.3, 0.45, -0.2),
            (-1.9, 0.8, -0.1),
            (-1.2, 1.0, 0.0),
            (0.2, 1.0, 0.05),
            (1.2, 0.85, 0.15),
            (1.9, 0.5, 0.3),
            (2.3, 0.0, 0.35),
        ]
    };
    cabin.draw(
        b.root(),
        &prof,
        |i, j| {
            if far {
                if (4..=8).contains(&(j * 2)) {
                    "livery"
                } else {
                    "body"
                }
            } else if (i == 1 || i == 2) && matches!(j, 0 | 1 | 2 | 10 | 11)
                || i == 3 && matches!(j, 2 | 3 | 8 | 9)
            {
                "glass"
            } else if (4..=8).contains(&j) {
                "livery"
            } else {
                "body"
            }
        },
        "body",
    );
    let p = b.root();
    let cowl = slab(-0.9, 1.0, 2.3, 0.42, -0.7, 0.85, 2.62, 0.32);
    hexa(p, all("body"), &cowl);
    let boom = Lathe::new(
        v3(0.0, 1.85, 0.0),
        Vec3::Z,
        Vec3::Y,
        if far { 5 } else { 8 },
    );
    boom.draw(
        p,
        &[(1.2, 0.38, 0.0), (4.0, 0.25, 0.12), (6.7, 0.14, 0.28)],
        |_, _| "body",
        "body",
    );
    fin(p, "livery", 0.0, 1.95, 6.1, 6.8, 0.1, 3.0, 6.6, 7.0, 0.06);
    wing(
        p,
        all("livery"),
        [-1.0, 2.0, 5.2, 5.6, 0.06],
        [1.0, 2.0, 5.2, 5.6, 0.06],
    );
    if !far {
        rod(p, "metal", v3(0.0, 2.6, -0.3), v3(0.0, 2.95, -0.3), 0.08, 8);
        sides(|x| {
            rod(
                p,
                "metal",
                v3(x * 1.0, 0.06, -1.8),
                v3(x * 1.0, 0.06, 1.4),
                0.05,
                6,
            );
            for z in [-1.0, 0.8] {
                rod(
                    p,
                    "metal",
                    v3(x * 1.0, 0.06, z),
                    v3(x * 0.55, 0.75, z),
                    0.04,
                    6,
                );
            }
        });
    } else {
        sides(|x| {
            cuboid(
                p,
                all("metal"),
                v3(x * 1.0, 0.05, -0.2),
                v3(0.04, 0.04, 1.6),
            )
        });
    }
    // The main rotor (three blades about Y) and the tail rotor (about X).
    let hub = v3(0.0, 2.95, -0.3);
    let p = b.part("rotor_main", ROOT, hub);
    if !far {
        let h = Lathe::new(hub - Vec3::Y * 0.08, Vec3::Y, Vec3::Z, 8);
        h.draw(
            p,
            &[
                (0.0, 0.0, 0.0),
                (0.0, 0.22, 0.0),
                (0.16, 0.18, 0.0),
                (0.2, 0.0, 0.0),
            ],
            |_, _| "metal",
            "metal",
        );
    }
    for k in 0..3 {
        let a = k as f32 * TAU / 3.0;
        let rot = Quat::from_rotation_y(a);
        obox(
            p,
            all("trim"),
            hub + rot * v3(0.0, 0.04, -2.75),
            v3(0.14, 0.025, 2.55),
            rot,
        );
    }
    let th = v3(0.24, 2.55, 6.78);
    let p = b.part("rotor_tail", ROOT, th);
    for s in [-1.0f32, 1.0] {
        cuboid(
            p,
            all("trim"),
            th + v3(0.0, s * 0.42, 0.0),
            v3(0.015, 0.4, 0.06),
        );
    }
    if !far {
        rod(p, "metal", th - Vec3::X * 0.1, th + Vec3::X * 0.05, 0.05, 6);
    }
    nav_lights(
        b,
        v3(-1.04, 2.0, 5.4),
        v3(1.04, 2.0, 5.4),
        v3(0.0, 2.2, 7.05),
        v3(0.0, 2.68, 0.7),
        0.05,
    );
}

// ---------------------------------------------------------------------
// The hot-air balloon.

fn balloon(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xE8B92E);
    b.paint("livery", 0xC2362B);
    let n = if far { 8 } else { 16 };
    let env = Lathe::new(Vec3::ZERO, Vec3::Y, Vec3::Z, n);
    let prof: Vec<(f32, f32, f32)> = if far {
        vec![
            (5.8, 1.1, 0.0),
            (9.0, 4.6, 0.0),
            (14.2, 8.3, 0.0),
            (19.2, 7.9, 0.0),
            (22.7, 3.8, 0.0),
            (23.4, 0.0, 0.0),
        ]
    } else {
        vec![
            (5.8, 1.1, 0.0),
            (7.0, 2.4, 0.0),
            (9.0, 4.6, 0.0),
            (11.5, 6.9, 0.0),
            (14.2, 8.3, 0.0),
            (16.8, 8.6, 0.0),
            (19.2, 7.9, 0.0),
            (21.3, 6.2, 0.0),
            (22.7, 3.8, 0.0),
            (23.3, 1.4, 0.0),
            (23.4, 0.0, 0.0),
        ]
    };
    let last = prof.len() - 2;
    env.draw(
        b.root(),
        &prof,
        |i, j| {
            if i == last || j % 2 == 0 {
                "body"
            } else {
                "livery"
            }
        },
        "trim",
    );
    let p = b.root();
    let basket = slab(-0.65, 0.65, 0.0, 0.65, -0.68, 0.68, 1.1, 0.68);
    hexa(p, all("wood"), &basket);
    let frame = slab(-0.38, 0.38, 2.95, 0.38, -0.38, 0.38, 3.08, 0.38);
    hexa(p, all("steel"), &frame);
    for (x, z) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
        if !far {
            rod(
                p,
                "steel",
                v3(x * 0.6, 1.1, z * 0.6),
                v3(x * 0.36, 2.95, z * 0.36),
                0.025,
                4,
            );
            rod(
                p,
                "steel",
                v3(x * 0.36, 3.08, z * 0.36),
                v3(x * 0.78, 5.82, z * 0.78),
                0.02,
                4,
            );
        }
    }
    if !far {
        let rim = slab(-0.7, 0.7, 1.1, 0.7, -0.7, 0.7, 1.16, 0.7);
        hexa(p, all("trim"), &rim);
    }
    b.lamp(
        "light_burner",
        ROOT,
        "light_glow",
        v3(0.0, 3.3, 0.0),
        v3(0.16, 0.2, 0.16),
        Vec3::Y,
    );
}

// ---------------------------------------------------------------------
// The flying saucer.

fn ufo(b: &mut Build) {
    let far = b.far();
    b.paint("body", 0xA9B1BA);
    b.paint("glass", 0x5FB8C9);
    let n = if far { 10 } else { 24 };
    let hull = Lathe::new(Vec3::ZERO, Vec3::Y, Vec3::Z, n);
    let prof: Vec<(f32, f32, f32)> = if far {
        vec![
            (1.0, 0.0, 0.0),
            (1.0, 1.4, 0.0),
            (1.85, 5.6, 0.0),
            (2.05, 6.0, 0.0),
            (2.35, 6.0, 0.0),
            (3.15, 3.4, 0.0),
            (3.5, 2.1, 0.0),
            (4.75, 1.0, 0.0),
            (4.95, 0.0, 0.0),
        ]
    } else {
        vec![
            (1.0, 0.0, 0.0),
            (1.0, 1.4, 0.0),
            (1.35, 3.6, 0.0),
            (1.85, 5.6, 0.0),
            (2.05, 6.0, 0.0),
            (2.35, 6.0, 0.0),
            (2.6, 5.4, 0.0),
            (3.15, 3.4, 0.0),
            (3.45, 2.3, 0.0),
            (3.5, 2.1, 0.0),
            (4.2, 1.75, 0.0),
            (4.75, 1.0, 0.0),
            (4.95, 0.0, 0.0),
        ]
    };
    let rim = if far { 3 } else { 4 };
    let dome = if far { 6 } else { 9 };
    hull.draw(
        b.root(),
        &prof,
        |i, _| {
            if i == 0 {
                "trim"
            } else if i == rim {
                if far { "light_glow" } else { "chrome" }
            } else if i >= dome {
                "glass"
            } else if i == dome - 1 || i == rim - 1 {
                "chrome"
            } else {
                "body"
            }
        },
        "body",
    );
    if !far {
        // Three landing legs with their pads.
        for k in 0..3 {
            let a = k as f32 * TAU / 3.0 + PI / 6.0;
            let d = v3(a.sin(), 0.0, a.cos());
            rod(
                b.root(),
                "metal",
                d * 2.4 + Vec3::Y * 1.3,
                d * 3.2 + Vec3::Y * 0.12,
                0.08,
                6,
            );
            let pad = Lathe::new(d * 3.2, Vec3::Y, Vec3::Z, 8);
            pad.draw(
                b.root(),
                &[
                    (0.0, 0.0, 0.0),
                    (0.0, 0.3, 0.0),
                    (0.12, 0.26, 0.0),
                    (0.12, 0.0, 0.0),
                ],
                |_, _| "metal",
                "metal",
            );
        }
        // The beam's lens under the hull.
        let p = b.part("light_beam", ROOT, v3(0.0, 0.97, 0.0));
        let lens = Lathe::new(Vec3::ZERO, Vec3::Y, Vec3::Z, 16);
        lens.draw(
            p,
            &[
                (0.95, 0.0, 0.0),
                (0.95, 1.1, 0.0),
                (0.995, 1.1, 0.0),
                (0.995, 0.0, 0.0),
            ],
            |_, _| "light_glow",
            "light_glow",
        );
        // The rim of coloured lights.
        const COLOURS: [&str; 4] = [
            "light_nav_red",
            "light_white",
            "light_nav_green",
            "light_indicator",
        ];
        for k in 0..12 {
            let a = (k as f32 + 0.5) * TAU / 12.0;
            let d = v3(a.sin(), 0.0, a.cos());
            let at = d * 6.03 + Vec3::Y * 2.2;
            let p = b.part(&format!("light_rim_{k}"), ROOT, at);
            obox(
                p,
                all(COLOURS[k % 4]),
                at,
                v3(0.16, 0.1, 0.06),
                Quat::from_rotation_y(a),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check;

    fn checked(v: Vehicle) -> Vec<check::Finding> {
        let m = v.model();
        let bytes = crate::glb::write(&m);
        check::check(&v.entry(), &m, bytes.len())
    }

    /// Every model passes the validator without an error or a warning,
    /// and is within its class's budgets.
    #[test]
    fn every_vehicle_is_valid_and_within_its_budget() {
        for v in Vehicle::ALL {
            let f = checked(v);
            assert!(
                f.is_empty(),
                "{v:?}: {:#?}",
                f.iter().map(|f| f.to_string()).collect::<Vec<_>>()
            );
            let m = v.model();
            let (near, far) = v.class().budget();
            let lod = m.triangles_below(m.node("lod1"));
            let own = m.triangles() - lod;
            assert!(own <= near && lod <= far && lod > 0, "{v:?}: {own} / {lod}");
            assert!(
                own > 2 * lod,
                "{v:?}: the far model is much lighter ({own} / {lod})"
            );
            eprintln!("{}: {own} triangles near, {lod} in lod1", v.file());
        }
    }

    /// Two builds give the same bytes (15B: deterministic files).
    #[test]
    fn the_files_are_deterministic() {
        for v in Vehicle::ALL {
            assert_eq!(
                crate::glb::write(&v.model()),
                crate::glb::write(&v.model()),
                "{v:?}"
            );
        }
    }

    /// Every triangle has an area and the normal of its face (what Blender
    /// keeps, 15M).
    #[test]
    fn every_triangle_has_an_area_and_its_face_normal() {
        for v in Vehicle::ALL {
            let m = v.model();
            for n in &m.nodes {
                for p in &n.primitives {
                    for t in p.indices.chunks(3) {
                        let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(p.positions[t[k] as usize]));
                        let face = (b - a).cross(c - a);
                        assert!(
                            face.length() > 1e-7,
                            "{v:?} {}: a triangle without area",
                            n.name
                        );
                        let face = face.normalize();
                        for &k in t {
                            let nn = Vec3::from(p.normals[k as usize]);
                            assert!(
                                nn.dot(face) > 0.999,
                                "{v:?} {}: a normal off its face",
                                n.name
                            );
                        }
                    }
                }
            }
        }
    }

    /// The closed shapes face outwards: a box and a lathed ball.
    #[test]
    fn boxes_and_lathes_face_out() {
        let mut p = Parts::new();
        cuboid(&mut p, all("body"), v3(1.0, 2.0, 3.0), v3(0.5, 0.2, 0.3));
        let ball = Lathe::new(v3(-2.0, 0.0, 0.0), Vec3::Z, Vec3::Y, 9).squash(1.3, 0.7);
        ball.draw(
            &mut p,
            &[
                (-1.0, 0.0, 0.0),
                (-0.5, 0.8, 0.0),
                (0.5, 0.9, 0.1),
                (1.0, 0.0, 0.0),
            ],
            |_, _| "glass",
            "glass",
        );
        let mut m = Model::default();
        let root = m.add(Node::new("x", None, Vec3::ZERO));
        p.attach(&mut m, root);
        for prim in &m.nodes[0].primitives {
            let centre = if m.materials[prim.material].name == "body" {
                v3(1.0, 2.0, 3.0)
            } else {
                v3(-2.0, 0.0, 0.0)
            };
            for t in prim.indices.chunks(3) {
                let pts = [0, 1, 2].map(|k| Vec3::from(prim.positions[t[k] as usize]));
                let mid = (pts[0] + pts[1] + pts[2]) / 3.0;
                let n = Vec3::from(prim.normals[t[0] as usize]);
                assert!(
                    n.dot(mid - centre) > 0.0,
                    "{}",
                    m.materials[prim.material].name
                );
            }
        }
    }

    /// The validator catches what the contract forbids: a missing wheel,
    /// a lamp in a body slot, a wheel off its pivot, physics or driving
    /// keys, a missing class, the paint slot gone, a budget overrun, nav
    /// lights swapped, the front at +Z.
    #[test]
    fn the_validator_catches_broken_vehicles() {
        let codes = |m: &Model, e: &Entry| -> Vec<&'static str> {
            let bytes = crate::glb::write(m);
            check::check(e, m, bytes.len())
                .iter()
                .filter(|f| f.is_error())
                .map(|f| f.code)
                .collect()
        };
        let v = Vehicle::CarHatchback;
        let e = v.entry();
        let good = v.model();
        assert!(codes(&good, &e).is_empty());

        let mut m = good.clone();
        let i = m.node("wheel_rr").unwrap();
        m.nodes[i].name = "wheel_x".into();
        assert!(codes(&m, &e).contains(&"node.missing"));

        let mut m = good.clone();
        let i = m.node("light_head_l").unwrap();
        let body = m.material("body").unwrap();
        m.nodes[i].primitives[0].material = body;
        assert!(codes(&m, &e).contains(&"vehicle.light-slot"));

        let mut m = good.clone();
        let i = m.node("light_brake_r").unwrap();
        m.nodes[i].primitives.clear();
        assert!(codes(&m, &e).contains(&"vehicle.light-mesh"));

        let mut m = good.clone();
        let i = m.node("wheel_fl").unwrap();
        for p in &mut m.nodes[i].primitives {
            for v in &mut p.positions {
                v[1] += 0.1;
            }
        }
        let c = codes(&m, &e);
        assert!(
            c.contains(&"vehicle.wheel-pivot") && c.contains(&"vehicle.wheel-ground"),
            "{c:?}"
        );

        for key in ["mass_kg", "max_speed", "power_kw"] {
            let mut m = good.clone();
            m.extras["ocw"][key] = json!(1200);
            assert!(codes(&m, &e).contains(&"extras.physics"), "{key}");
            let mut m = good.clone();
            let i = m.node("wheel_fl").unwrap();
            m.nodes[i].extras = json!({ key: 3 });
            assert!(codes(&m, &e).contains(&"extras.physics"), "node {key}");
        }

        let mut m = good.clone();
        m.extras["ocw"].as_object_mut().unwrap().remove("class");
        assert!(codes(&m, &e).contains(&"vehicle.class"));
        let mut m = good.clone();
        m.extras["ocw"]["class"] = json!("tank");
        assert!(codes(&m, &e).contains(&"vehicle.class"));

        let mut m = good.clone();
        let i = m.material("body").unwrap();
        m.materials[i].name = "paint_dark".into();
        assert!(codes(&m, &e).contains(&"vehicle.paint-slot"));

        // Too many triangles near and far; a balloon without its burner.
        let mut m = good.clone();
        let extra = m.nodes[m.node("wheel_fl").unwrap()].primitives.clone();
        let lod = m.node("lod1").unwrap();
        for _ in 0..25 {
            m.nodes[0].primitives.extend(extra.iter().cloned());
            m.nodes[lod].primitives.extend(extra.iter().cloned());
        }
        let c = codes(&m, &e);
        assert!(
            c.contains(&"budget.triangles") && c.contains(&"lod.budget"),
            "{c:?}"
        );
        let mut m = good.clone();
        m.extras["ocw"]["class"] = json!("balloon");
        assert!(codes(&m, &e).contains(&"node.missing"));

        // Turned round: the front wheels behind the rear ones.
        let mut m = good.clone();
        for n in &mut m.nodes {
            if n.parent.is_some() {
                n.translation.z = -n.translation.z;
            }
        }
        assert!(codes(&m, &e).contains(&"vehicle.front"));

        // Moved along: the origin is not under the wheelbase's centre.
        let mut m = good.clone();
        for n in &mut m.nodes {
            if n.parent == Some(0) {
                n.translation.z += 0.5;
            }
        }
        assert!(codes(&m, &e).contains(&"vehicle.origin"));

        // The airliner's nav lights swapped.
        let a = Vehicle::AirlinerTwinjet;
        let mut m = a.model();
        let (l, r) = (
            m.node("light_nav_l").unwrap(),
            m.node("light_nav_r").unwrap(),
        );
        let (tl, tr) = (m.nodes[l].translation, m.nodes[r].translation);
        m.nodes[l].translation = tr;
        m.nodes[r].translation = tl;
        assert!(codes(&m, &a.entry()).contains(&"vehicle.lights"));

        // Without lod1.
        let mut m = good.clone();
        let i = m.node("lod1").unwrap();
        m.nodes[i].name = "far".into();
        assert!(codes(&m, &e).contains(&"lod.missing"));
    }

    /// Plausible sizes (m): length, width, height per model, the wheels
    /// on the ground, and the real-world dimensions they stand for.
    #[test]
    fn the_proportions_are_plausible() {
        for (v, len, wid, hgt) in [
            (Vehicle::CarHatchback, 3.8..4.3, 1.7..2.0, 1.4..1.6),
            (Vehicle::CarSaloon, 4.5..5.0, 1.75..2.05, 1.35..1.55),
            (Vehicle::CarSuv, 4.4..4.9, 1.8..2.15, 1.6..1.9),
            (Vehicle::VanPanel, 5.0..5.8, 1.95..2.45, 2.2..2.6),
            (Vehicle::LorryRigid, 7.5..9.5, 2.45..3.1, 3.3..3.9),
            (Vehicle::LorryArticulated, 15.5..17.5, 2.45..3.1, 3.7..4.1),
            (Vehicle::BusCity, 11.5..12.5, 2.5..3.1, 3.0..3.4),
            (Vehicle::MotorbikeTouring, 2.0..2.4, 0.6..0.95, 1.3..1.8),
            (Vehicle::TractorFarm, 4.0..4.6, 2.2..2.6, 2.7..3.1),
            (Vehicle::AirlinerTwinjet, 36.0..40.0, 33.0..36.0, 11.0..12.5),
            (
                Vehicle::AirlinerTwinjetSplitWinglet,
                38.0..40.0,
                34.0..36.5,
                11.0..12.5,
            ),
            (Vehicle::PlaneHighWing, 7.5..9.0, 10.0..11.5, 2.5..3.0),
            (Vehicle::GliderSailplane, 6.0..7.5, 14.5..15.5, 1.5..1.9),
            (Vehicle::HelicopterLight, 9.0..13.0, 9.0..11.0, 2.8..3.5),
            (
                Vehicle::HelicopterMediumTransport,
                19.0..22.5,
                16.0..19.5,
                5.5..6.5,
            ),
            (Vehicle::BalloonHotAir, 16.0..18.0, 16.0..18.0, 22.0..24.5),
            (Vehicle::UfoSaucer, 11.5..12.5, 11.5..12.5, 4.5..5.2),
        ] {
            let m = v.model();
            let (lo, hi) = m.near_bounds().unwrap();
            let s = hi - lo;
            assert!(
                len.contains(&s.z) && wid.contains(&s.x) && hgt.contains(&s.y),
                "{v:?}: {s}"
            );
            assert!(lo.y.abs() < 0.05, "{v:?} stands on its origin: {lo}");
        }
    }

    /// The shipped files are this generator's (byte for byte where they
    /// were exported; elsewhere within the writer's grid: `sin` and `cos`
    /// may differ in the last bit between C libraries).
    #[test]
    fn the_shipped_files_are_the_generator_s() {
        for v in Vehicle::ALL {
            let shipped = crate::library::embedded_files()
                .iter()
                .find(|f| f.0 == v.file())
                .map(|f| f.1)
                .unwrap_or_else(|| panic!("{} is shipped", v.file()));
            let made = crate::glb::write(&v.model());
            if shipped == made.as_slice() {
                continue;
            }
            let a = crate::glb::read(shipped).unwrap();
            let b = v.model();
            assert_eq!(a.nodes.len(), b.nodes.len(), "{v:?}");
            for (na, nb) in a.nodes.iter().zip(&b.nodes) {
                assert_eq!(na.name, nb.name);
                assert_eq!(
                    na.primitives.len(),
                    nb.primitives.len(),
                    "{v:?} {}",
                    na.name
                );
                for (pa, pb) in na.primitives.iter().zip(&nb.primitives) {
                    assert_eq!(pa.indices, pb.indices, "{v:?} {}", na.name);
                    for (x, y) in pa.positions.iter().zip(&pb.positions) {
                        assert!(
                            (Vec3::from(*x) - Vec3::from(*y)).length() < 1e-4,
                            "{v:?} {}",
                            na.name
                        );
                    }
                }
            }
        }
    }
}

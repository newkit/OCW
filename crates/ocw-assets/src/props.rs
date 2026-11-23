//! Props placed in the world from their model files (spec §14 Phase 15,
//! 15E / 15H MX: the point obstacles and the street lamps): how a file is
//! fitted to one instance – its height, the width it spans, the reach of
//! its arm – and set on the ground, as merged vertex arrays (the renderer
//! merges them per tile; pure, no Bevy).
//!
//! **The fit contract** (written by the files, read here):
//! - `asset.extras.ocw.fit = { height_m, sink_m, span_m?, reach_m? }`: the
//!   file's reference height above the ground, how deep it is set into the
//!   ground (the mesh starts at the origin and is lowered by `sink_m`, so
//!   the file keeps its origin on the ground, 15B), the reference width
//!   across (model X) of props that span a way (gates), the reference
//!   reach (model +X) of an arm (street lamps).
//! - An instance of height `h` is scaled vertically by
//!   `(h + sink) / (height + sink)` (its foot stays `sink_m` deep).
//! - Node extras `ocw.span`: `"stretch"` (the default of a spanning prop:
//!   the node's mesh is stretched across by `span / span_m`) or `"move"`
//!   (only moved: a gate's posts keep their thickness).
//! - Node extras `ocw.reach`: `"stretch"` (the arm: stretched along +X
//!   from the node's origin until its tip moves by `reach − reach_m`) or
//!   `"move"` (the lamp head and its light: moved by that much).
//! - Node extras `ocw.ground`: `"model"` (the default: the node stands on
//!   the ground under the model's origin), `"node"` (on the ground under
//!   its own origin: a gate post at either end of a sloping road) or
//!   `"drape"` (each vertex on the ground under it: the bars and rails
//!   between two posts follow the slope).
//!
//! Roles are inherited by child nodes. Collision proxies `col_*`, anchors
//! and far levels are not drawn; anchors are placed like their node
//! ([`Placed::anchors`]: the lamp's `anchor_light`).

use glam::{Mat3, Quat, Vec3};
use serde_json::Value;

use crate::model::{Model, is_far_level};

/// The fit of a file (`extras.ocw.fit`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileFit {
    pub height_m: f32,
    pub sink_m: f32,
    pub span_m: Option<f32>,
    pub reach_m: Option<f32>,
}

impl FileFit {
    /// The fit of model `m` (`None` without `extras.ocw.fit.height_m`).
    pub fn of(m: &Model) -> Option<Self> {
        let fit = m.ocw()?.get("fit")?;
        let num = |k: &str| fit.get(k).and_then(Value::as_f64).map(|v| v as f32);
        Some(Self {
            height_m: num("height_m")?,
            sink_m: num("sink_m").unwrap_or(0.0),
            span_m: num("span_m"),
            reach_m: num("reach_m"),
        })
    }

    /// As `extras.ocw.fit` (rounded to 0.1 mm).
    pub fn to_json(&self) -> Value {
        let r = |v: f32| f64::from((v * 10_000.0).round() / 10_000.0);
        let mut o = serde_json::Map::new();
        o.insert("height_m".into(), r(self.height_m).into());
        o.insert("sink_m".into(), r(self.sink_m).into());
        if let Some(s) = self.span_m {
            o.insert("span_m".into(), r(s).into());
        }
        if let Some(s) = self.reach_m {
            o.insert("reach_m".into(), r(s).into());
        }
        Value::Object(o)
    }
}

/// One placed instance's sizes (`None`: the file's).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Instance {
    pub height_m: Option<f32>,
    pub span_m: Option<f32>,
    pub reach_m: Option<f32>,
}

/// How a node follows the span or the reach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Along {
    Stretch,
    Move,
}

/// Which ground a node stands on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ground {
    Model,
    Node,
    Drape,
}

/// Node extras keys.
pub const SPAN: &str = "span";
pub const REACH: &str = "reach";
pub const GROUND: &str = "ground";

/// Node extras `{"ocw": {key: value, …}}`.
pub fn node_extras(pairs: &[(&str, &str)]) -> Value {
    let mut o = serde_json::Map::new();
    for (k, v) in pairs {
        o.insert((*k).into(), (*v).into());
    }
    serde_json::json!({ "ocw": Value::Object(o) })
}

/// The role `key` of node `i` (or its nearest ancestor that sets it).
fn role<'a>(m: &'a Model, mut i: usize, key: &str) -> Option<&'a str> {
    loop {
        if let Some(v) = m.nodes[i]
            .extras
            .get("ocw")
            .and_then(|o| o.get(key))
            .and_then(Value::as_str)
        {
            return Some(v);
        }
        i = m.nodes[i].parent?;
    }
}

fn along(m: &Model, i: usize, key: &str) -> Option<Along> {
    match role(m, i, key)? {
        "stretch" => Some(Along::Stretch),
        "move" => Some(Along::Move),
        _ => None,
    }
}

fn ground_of(m: &Model, i: usize) -> Ground {
    match role(m, i, GROUND) {
        Some("node") => Ground::Node,
        Some("drape") => Ground::Drape,
        _ => Ground::Model,
    }
}

/// Whether node `i` is drawn (not a proxy, an anchor or a far level).
fn drawn(m: &Model, i: usize) -> bool {
    !m.under_named(i, |n| {
        is_far_level(n) || n.starts_with("col_") || n.starts_with("anchor_")
    })
}

/// One material's placed triangles.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PlacedPart {
    /// The material's paint slot.
    pub slot: String,
    /// Linear RGBA of every vertex (the material's colour × the vertex
    /// colour).
    pub colors: Vec<[f32; 4]>,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Counter-clockwise triangles.
    pub indices: Vec<u32>,
}

/// A placed instance.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Placed {
    /// Per drawn primitive.
    pub parts: Vec<PlacedPart>,
    /// The anchors `anchor_*` placed (world positions).
    pub anchors: Vec<(String, Vec3)>,
}

impl Placed {
    pub fn anchor(&self, name: &str) -> Option<Vec3> {
        self.anchors
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, p)| *p)
    }
}

/// The placement of one node: model space → fitted model space.
struct NodeFit {
    /// Model-space origin of the node.
    origin: Vec3,
    /// x' = x · sx + dx (fitted, before the turn).
    sx: f32,
    dx: f32,
    ground: Ground,
}

/// Places model `m` (fit [`FileFit::of`]) for `inst`: its origin at
/// `foot` (x, z; `foot.y` the ground under it), turned by `yaw` about +Y
/// (Bevy's `Quat::from_rotation_y`), nodes on `ground(x, z)` by their role.
/// A model without a fit is placed as it is (height 1:1, no sink).
pub fn place(
    m: &Model,
    inst: &Instance,
    foot: Vec3,
    yaw: f32,
    ground: &dyn Fn(f32, f32) -> f32,
) -> Placed {
    let fit = FileFit::of(m).unwrap_or(FileFit {
        height_m: 1.0,
        sink_m: 0.0,
        span_m: None,
        reach_m: None,
    });
    let sy = inst
        .height_m
        .map_or(1.0, |h| (h + fit.sink_m) / (fit.height_m + fit.sink_m));
    let span = match (fit.span_m, inst.span_m) {
        (Some(r), Some(s)) if r > 0.0 => s / r,
        _ => 1.0,
    };
    let reach = match (fit.reach_m, inst.reach_m) {
        (Some(r), Some(s)) => s - r,
        _ => 0.0,
    };
    let turn = Quat::from_rotation_y(yaw);
    let world: Vec<glam::Affine3A> = (0..m.nodes.len()).map(|i| m.world(i)).collect();
    let node_fit = |i: usize| -> NodeFit {
        let origin = Vec3::from(world[i].translation);
        let (mut sx, mut dx) = (1.0, 0.0);
        if fit.span_m.is_some() {
            match along(m, i, SPAN).unwrap_or(Along::Stretch) {
                Along::Stretch => sx = span,
                Along::Move => dx = origin.x * (span - 1.0),
            }
        }
        if reach != 0.0 {
            match along(m, i, REACH) {
                Some(Along::Stretch) if !m.nodes[i].primitives.is_empty() => {
                    // The tip (the mesh's largest x) moves by `reach`.
                    let tip = m.nodes[i]
                        .primitives
                        .iter()
                        .flat_map(|p| &p.positions)
                        .map(|v| world[i].transform_point3(Vec3::from(*v)).x)
                        .fold(f32::MIN, f32::max);
                    let len = tip - origin.x;
                    if len > 1e-4 {
                        let k = (len + reach) / len;
                        // x' = o + (x − o) k (after the span's sx, dx).
                        dx = dx * k + origin.x * (1.0 - k);
                        sx *= k;
                    }
                }
                Some(_) => dx += reach,
                None => {}
            }
        }
        NodeFit {
            origin,
            sx,
            dx,
            ground: ground_of(m, i),
        }
    };
    let fitted = |f: &NodeFit, p: Vec3| -> Vec3 {
        let q = Vec3::new(p.x * f.sx + f.dx, p.y * sy - fit.sink_m, p.z);
        turn * q + Vec3::new(foot.x, 0.0, foot.z)
    };
    let base = |f: &NodeFit, at: Vec3| -> f32 {
        match f.ground {
            Ground::Model => foot.y,
            Ground::Node => {
                let o = fitted(f, Vec3::new(f.origin.x, 0.0, f.origin.z));
                ground(o.x, o.z)
            }
            Ground::Drape => ground(at.x, at.z),
        }
    };
    let mut out = Placed::default();
    for (i, n) in m.nodes.iter().enumerate() {
        if n.name.starts_with("anchor_") && drawn_parent(m, i) {
            let f = node_fit(i);
            let p = fitted(&f, f.origin);
            out.anchors
                .push((n.name.clone(), p + Vec3::Y * base(&f, p)));
            continue;
        }
        if n.primitives.is_empty() || !drawn(m, i) {
            continue;
        }
        let f = node_fit(i);
        let w = world[i];
        // Normals: the node's linear part, then the fit's scale (inverse
        // transpose), then the turn.
        let lin = Mat3::from(w.matrix3).inverse().transpose();
        for p in &n.primitives {
            let mat = &m.materials[p.material];
            let mut part = PlacedPart {
                slot: mat.name.clone(),
                ..Default::default()
            };
            for (k, v) in p.positions.iter().enumerate() {
                let q = fitted(&f, w.transform_point3(Vec3::from(*v)));
                part.positions.push((q + Vec3::Y * base(&f, q)).to_array());
                let n0 = lin * Vec3::from(p.normals.get(k).copied().unwrap_or([0.0, 1.0, 0.0]));
                let n1 = Vec3::new(n0.x / f.sx, n0.y / sy, n0.z).normalize_or_zero();
                part.normals.push((turn * n1).to_array());
                let c = p.colors.get(k).copied().unwrap_or([1.0; 4]);
                part.colors.push([
                    mat.color[0] * c[0],
                    mat.color[1] * c[1],
                    mat.color[2] * c[2],
                    mat.color[3] * c[3],
                ]);
            }
            part.indices = p.indices.clone();
            out.parts.push(part);
        }
    }
    out
}

/// Whether anchor node `i` hangs under drawn geometry (not a proxy or far
/// level).
fn drawn_parent(m: &Model, i: usize) -> bool {
    !m.under_named(i, |n| is_far_level(n) || n.starts_with("col_"))
}

/// A collision proxy `col_*` (15B, Phase 12) in model space.
#[derive(Debug, Clone, PartialEq)]
pub struct Proxy {
    pub name: String,
    /// `box`, `cylinder` or `capsule`.
    pub shape: &'static str,
    /// Centre and half extents in model space (a cylinder's radius is
    /// `half.x`, its axis +Y).
    pub centre: Vec3,
    pub half: Vec3,
}

/// The collision proxies of `m`, read in **both** forms the checker
/// accepts (15H Q9 is open): a node with a mesh gives its mesh's bounds;
/// an empty node is the unit shape (half extents 1, like Blender's empties)
/// scaled by its transform.
pub fn proxies(m: &Model) -> Vec<Proxy> {
    let mut out = Vec::new();
    for (i, n) in m.nodes.iter().enumerate() {
        let Some(shape) = crate::check::collision_shape(&n.name) else {
            continue;
        };
        let w = m.world(i);
        let (centre, half) = if n.primitives.iter().any(|p| !p.positions.is_empty()) {
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for p in &n.primitives {
                for v in &p.positions {
                    let q = w.transform_point3(Vec3::from(*v));
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
            }
            ((lo + hi) * 0.5, (hi - lo) * 0.5)
        } else {
            // The unit shape's corners through the transform.
            let mut lo = Vec3::splat(f32::INFINITY);
            let mut hi = Vec3::splat(f32::NEG_INFINITY);
            for c in 0..8 {
                let s = |b: u32| if c & b == 0 { -1.0 } else { 1.0 };
                let q = w.transform_point3(Vec3::new(s(1), s(2), s(4)));
                lo = lo.min(q);
                hi = hi.max(q);
            }
            ((lo + hi) * 0.5, (hi - lo) * 0.5)
        };
        out.push(Proxy {
            name: n.name.clone(),
            shape,
            centre,
            half,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Material, Node, Primitive};

    fn quad_box(lo: Vec3, hi: Vec3) -> Primitive {
        // Only the corners matter for these tests (positions, normals up).
        let mut p = Primitive::default();
        for c in 0..8u32 {
            let pick = |b: u32, a: f32, z: f32| if c & b == 0 { a } else { z };
            p.positions.push([
                pick(1, lo.x, hi.x),
                pick(2, lo.y, hi.y),
                pick(4, lo.z, hi.z),
            ]);
            p.normals.push([0.0, 1.0, 0.0]);
        }
        p.indices = vec![0, 1, 2];
        p
    }

    /// A gate-like model: two posts (moved), a bar between them (stretched,
    /// draped), a lamp-like arm (reach) and head with an anchor.
    fn gate() -> Model {
        let mut m = Model::default();
        m.materials.push(Material {
            name: "steel".into(),
            color: [0.5, 0.5, 0.5, 1.0],
            metallic: 0.0,
            roughness: 0.5,
            emissive: [0.0; 3],
        });
        let root = m.add(Node::new("gate", None, Vec3::ZERO));
        for (name, x) in [("post_l", -1.5), ("post_r", 1.5)] {
            let mut n = Node::new(name, Some(root), Vec3::new(x, 0.0, 0.0));
            n.extras = node_extras(&[(SPAN, "move"), (GROUND, "node")]);
            n.primitives.push(quad_box(
                Vec3::new(-0.1, 0.0, -0.1),
                Vec3::new(0.1, 1.3, 0.1),
            ));
            let i = m.add(n);
            let mut c = Node::new("col_cylinder", Some(i), Vec3::new(0.0, 0.65, 0.0));
            c.scale = Vec3::new(0.1, 0.65, 0.1);
            m.add(c);
        }
        let mut bar = Node::new("bar", Some(root), Vec3::ZERO);
        bar.extras = node_extras(&[(GROUND, "drape")]);
        bar.primitives.push(quad_box(
            Vec3::new(-1.5, 1.0, -0.03),
            Vec3::new(1.5, 1.1, 0.03),
        ));
        m.add(bar);
        m.extras = serde_json::json!({ "ocw": { "fit": FileFit {
            height_m: 1.2, sink_m: 0.1, span_m: Some(3.0), reach_m: None }.to_json() } });
        m
    }

    fn xs(p: &Placed, part: usize) -> (f32, f32) {
        p.parts[part]
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v[0]), b.max(v[0])))
    }

    #[test]
    fn the_span_moves_the_posts_and_stretches_the_bar() {
        let m = gate();
        let flat = |_: f32, _: f32| 5.0;
        let p = place(
            &m,
            &Instance::default(),
            Vec3::new(0.0, 5.0, 0.0),
            0.0,
            &flat,
        );
        assert_eq!(p.parts.len(), 3, "proxies are not drawn");
        let (a, b) = xs(&p, 0);
        assert!((a + 1.6).abs() < 1e-5 && (b + 1.4).abs() < 1e-5, "{a} {b}");
        // The sink: the posts start 0.1 m below the ground.
        let low = p.parts[0]
            .positions
            .iter()
            .map(|v| v[1])
            .fold(f32::MAX, f32::min);
        assert!((low - 4.9).abs() < 1e-5, "{low}");
        let wide = Instance {
            span_m: Some(6.0),
            ..Default::default()
        };
        let p = place(&m, &wide, Vec3::new(0.0, 5.0, 0.0), 0.0, &flat);
        let (a, b) = xs(&p, 0);
        assert!(
            (a + 3.1).abs() < 1e-5 && (b + 2.9).abs() < 1e-5,
            "post keeps 0.2 m: {a} {b}"
        );
        let (a, b) = xs(&p, 2);
        assert!(
            (a + 3.0).abs() < 1e-5 && (b - 3.0).abs() < 1e-5,
            "bar across 6 m: {a} {b}"
        );
    }

    #[test]
    fn nodes_stand_on_their_ground_and_the_height_scales() {
        let m = gate();
        // A slope across: 0.1 m per metre in x.
        let slope = |x: f32, _: f32| 10.0 + 0.1 * x;
        let p = place(
            &m,
            &Instance::default(),
            Vec3::new(0.0, 10.0, 0.0),
            0.0,
            &slope,
        );
        let low = |k: usize| {
            p.parts[k]
                .positions
                .iter()
                .map(|v| v[1])
                .fold(f32::MAX, f32::min)
        };
        // Posts on the ground under their own origins (±1.5 m).
        assert!((low(0) - (10.0 - 0.15 - 0.1)).abs() < 1e-4, "{}", low(0));
        assert!((low(1) - (10.0 + 0.15 - 0.1)).abs() < 1e-4, "{}", low(1));
        // The bar's ends follow the slope.
        for v in &p.parts[2].positions {
            let want = 10.0
                + 0.1 * v[0]
                + if v[1] - 10.0 - 0.1 * v[0] > 0.95 {
                    1.0
                } else {
                    0.9
                };
            assert!((v[1] - want).abs() < 1e-4, "{v:?} {want}");
        }
        // Height 2.5 m: (2.5 + 0.1) / (1.2 + 0.1) vertically, the foot as deep.
        let tall = Instance {
            height_m: Some(2.5),
            ..Default::default()
        };
        let flat = |_: f32, _: f32| 0.0;
        let p = place(&m, &tall, Vec3::ZERO, 0.0, &flat);
        let top = p.parts[0]
            .positions
            .iter()
            .map(|v| v[1])
            .fold(f32::MIN, f32::max);
        let bottom = p.parts[0]
            .positions
            .iter()
            .map(|v| v[1])
            .fold(f32::MAX, f32::min);
        assert!((top - (1.3 * 2.6 / 1.3 - 0.1)).abs() < 1e-4, "{top}");
        assert!((bottom + 0.1).abs() < 1e-5, "{bottom}");
    }

    #[test]
    fn the_reach_stretches_the_arm_and_moves_the_head_and_its_anchor() {
        let mut m = Model::default();
        m.materials.push(Material {
            name: "bulb".into(),
            color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.4,
            emissive: [0.0; 3],
        });
        let root = m.add(Node::new("lamp", None, Vec3::ZERO));
        let mut arm = Node::new("arm", Some(root), Vec3::new(-0.05, 0.0, 0.0));
        arm.extras = node_extras(&[(REACH, "stretch")]);
        arm.primitives.push(quad_box(
            Vec3::new(0.0, 6.0, -0.04),
            Vec3::new(1.6, 6.1, 0.04),
        ));
        m.add(arm);
        let mut head = Node::new("head", Some(root), Vec3::new(1.5, 6.0, 0.0));
        head.extras = node_extras(&[(REACH, "move")]);
        head.primitives.push(quad_box(
            Vec3::new(-0.28, -0.06, -0.14),
            Vec3::new(0.28, 0.06, 0.14),
        ));
        let h = m.add(head);
        m.add(Node::new(
            "anchor_light",
            Some(h),
            Vec3::new(0.0, -0.3, 0.0),
        ));
        m.extras = serde_json::json!({ "ocw": { "fit": FileFit {
            height_m: 6.0, sink_m: 0.0, span_m: None, reach_m: Some(1.5) }.to_json() } });
        let flat = |_: f32, _: f32| 0.0;
        let short = Instance {
            reach_m: Some(0.6),
            ..Default::default()
        };
        // Turned by 90°: +X goes to −Z.
        let p = place(&m, &short, Vec3::ZERO, std::f32::consts::FRAC_PI_2, &flat);
        let zs = p.parts[0]
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v[2]), b.max(v[2])));
        assert!(
            (zs.0 + 0.65).abs() < 1e-4 && (zs.1 - 0.05).abs() < 1e-4,
            "{zs:?}"
        );
        let light = p.anchor("anchor_light").unwrap();
        assert!(light.distance(Vec3::new(0.0, 5.7, -0.6)) < 1e-4, "{light}");
    }

    #[test]
    fn proxies_come_from_meshes_or_from_unit_shapes() {
        let m = gate();
        let p = proxies(&m);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].shape, "cylinder");
        assert!(p[0].centre.distance(Vec3::new(-1.5, 0.65, 0.0)) < 1e-5);
        assert!(p[0].half.distance(Vec3::new(0.1, 0.65, 0.1)) < 1e-5);
        // The same proxy as a modelled mesh.
        let mut m = m;
        let i = m.node("col_cylinder").unwrap();
        m.nodes[i].scale = Vec3::ONE;
        m.nodes[i].primitives.push(quad_box(
            Vec3::new(-0.1, -0.65, -0.1),
            Vec3::new(0.1, 0.65, 0.1),
        ));
        let q = proxies(&m);
        assert!(q[0].centre.distance(p[0].centre) < 1e-5 && q[0].half.distance(p[0].half) < 1e-5);
        // Both survive the file.
        let back = crate::glb::read(&crate::glb::write(&m)).unwrap();
        assert_eq!(proxies(&back).len(), 2);
    }
}

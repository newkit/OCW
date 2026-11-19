//! The one-time export of the generated props into model files (spec §14
//! Phase 15, 15H MX / M2): the point obstacles of [`crate::obstacles`] and
//! the street lamps of [`crate::street_lamps`], each kind at its default
//! size as `artwork/3dmodels/props/*.glb` with its parts as nodes (the fit
//! roles of [`ocw_assets::props`]), collision proxies `col_*` and anchors
//! (`cargo run -p ocw-render --example export-props`). After the export
//! the files are the source and the generators are removed (Q6).

use bevy::prelude::*;
use ocw_assets::kinds::Kind;
use ocw_assets::manifest::Entry;
use ocw_assets::model::{Material, Model, Node, Primitive};
use ocw_assets::props::{FileFit, GROUND, REACH, SPAN, node_extras};
use ocw_world::obstacles::ObstacleKind;
use serde_json::json;

use crate::obstacles::{Palette, beam, prism};
use crate::package::MeshArrays;

/// `source` of the exported prop files.
pub const SOURCE: &str = "own work, exported once from the OCW generators (ocw-render obstacles / street_lamps, spec §14 Phase 15 MX); the file is the source (Q6)";

/// A model being built: parts in world space at angle 0 (obstacles: X
/// east is along the way, Z south across it) turned into model space.
struct Build {
    m: Model,
    root: usize,
    sink: f32,
    /// Obstacles: model x = world z (across), model z = −world x (along).
    swap: bool,
}

impl Build {
    fn new(root: &str, sink: f32, swap: bool) -> Self {
        let mut m = Model::default();
        let root = m.add(Node::new(root, None, Vec3::ZERO));
        Self {
            m,
            root,
            sink,
            swap,
        }
    }

    fn to_model(&self, p: [f32; 3]) -> Vec3 {
        if self.swap {
            Vec3::new(p[2], p[1], -p[0])
        } else {
            Vec3::from(p)
        }
    }

    /// A node at `origin` (model space, on the ground: y 0) with roles.
    fn node(&mut self, name: &str, origin: Vec3, roles: &[(&str, &str)]) -> usize {
        let mut n = Node::new(name, Some(self.root), origin);
        if !roles.is_empty() {
            n.extras = node_extras(roles);
        }
        self.m.add(n)
    }

    /// Adds world-space arrays (one colour) to node `i` as slot `slot`.
    fn mesh(&mut self, i: usize, slot: &str, a: &MeshArrays) {
        let colour = a.colors.first().copied().unwrap_or([1.0; 4]);
        let base = ocw_assets::slots::material(slot);
        let mat = Material {
            name: slot.into(),
            color: colour,
            metallic: base.metallic,
            roughness: base.roughness,
        };
        let k = match self.m.materials.iter().position(|m| *m == mat) {
            Some(k) => k,
            None => {
                assert!(
                    self.m.material(slot).is_none(),
                    "one colour per slot in a file"
                );
                self.m.materials.push(mat);
                self.m.materials.len() - 1
            }
        };
        let origin = Vec3::from(self.m.world(i).translation);
        let mut prim = Primitive {
            material: k,
            ..Default::default()
        };
        for (p, n) in a.positions.iter().zip(&a.normals) {
            let q = self.to_model(*p) + Vec3::Y * self.sink - origin;
            prim.positions.push(q.to_array());
            prim.normals.push(self.to_model(*n).to_array());
        }
        prim.indices = a.indices.clone();
        let node = &mut self.m.nodes[i];
        match node.primitives.iter_mut().find(|p| p.material == k) {
            Some(p) => {
                let off = p.positions.len() as u32;
                p.positions.extend(prim.positions);
                p.normals.extend(prim.normals);
                p.indices.extend(prim.indices.iter().map(|v| v + off));
            }
            None => node.primitives.push(prim),
        }
    }

    /// A collision proxy (an empty: the unit shape scaled, 15H Q9's form
    /// (c)/(a)) under node `i`: `centre` in model space above the ground.
    fn proxy(&mut self, i: usize, name: &str, centre: Vec3, half: Vec3) {
        let origin = Vec3::from(self.m.world(i).translation);
        let mut n = Node::new(name, Some(i), centre + Vec3::Y * self.sink - origin);
        n.scale = half;
        self.m.add(n);
    }

    fn anchor(&mut self, i: usize, name: &str, at: Vec3) {
        let origin = Vec3::from(self.m.world(i).translation);
        self.m
            .add(Node::new(name, Some(i), at + Vec3::Y * self.sink - origin));
    }
}

fn arrays(f: impl FnOnce(&mut MeshArrays)) -> MeshArrays {
    let mut a = MeshArrays::default();
    f(&mut a);
    a
}

/// The file of an obstacle kind's model (15H names).
pub fn obstacle_file(kind: ObstacleKind) -> &'static str {
    crate::obstacles::model_id_of(kind)
}

/// The default span of a gate (m; the old generator's width without one).
pub const SPAN_M: f32 = 3.0;
/// The reference arm of a street lamp (m).
pub const REACH_M: f32 = 1.5;

/// The obstacle kinds with a file.
pub const OBSTACLES: [ObstacleKind; 8] = [
    ObstacleKind::Bollard,
    ObstacleKind::Post,
    ObstacleKind::Pylon,
    ObstacleKind::Block,
    ObstacleKind::Gate,
    ObstacleKind::LiftGate,
    ObstacleKind::CycleBarrier,
    ObstacleKind::Barrier,
];

/// The OSM tag a kind stands for (`extras.ocw.osm`).
fn osm(kind: ObstacleKind) -> &'static str {
    match kind {
        ObstacleKind::Bollard => "barrier=bollard",
        ObstacleKind::Post => "power=pole",
        ObstacleKind::Pylon => "power=tower",
        ObstacleKind::Block => "barrier=block",
        ObstacleKind::Gate => "barrier=gate",
        ObstacleKind::LiftGate => "barrier=lift_gate",
        ObstacleKind::CycleBarrier => "barrier=cycle_barrier",
        _ => "barrier=*",
    }
}

/// An obstacle kind's model, at its default height (and the default span).
pub fn obstacle_model(kind: ObstacleKind) -> Model {
    let c = Palette::new();
    let h = kind.default_height_m() as f32;
    let o = [0.0f32; 3];
    let across = |w: f32| [0.0, 0.0, w];
    let sink = match kind {
        ObstacleKind::Post | ObstacleKind::Pylon | ObstacleKind::Gate | ObstacleKind::LiftGate => {
            0.2
        }
        _ => 0.1,
    };
    let name = obstacle_file(kind)
        .trim_start_matches("props/")
        .replace('-', "_");
    let mut b = Build::new(&name, sink, true);
    let root = b.root;
    let mut span = None;
    let cyl = |r: f32, top: f32| (Vec3::new(0.0, top / 2.0, 0.0), Vec3::new(r, top / 2.0, r));
    match kind {
        ObstacleKind::Bollard => {
            b.mesh(root, "paint_dark", &arrays(|m| prism(m, o, 0.1, 0.09, -0.1, h, 8, c.dark)));
            b.mesh(root, "paint_white", &arrays(|m| {
                prism(m, o, 0.101, 0.101, h - 0.25, h - 0.15, 8, c.white)
            }));
            let (at, half) = cyl(0.1, h);
            b.proxy(root, "col_cylinder", at, half);
        }
        ObstacleKind::Post => {
            b.mesh(root, "wood", &arrays(|m| prism(m, o, 0.13, 0.1, -0.2, h, 6, c.wood)));
            let (at, half) = cyl(0.13, h);
            b.proxy(root, "col_cylinder", at, half);
        }
        ObstacleKind::Pylon => {
            b.mesh(root, "steel", &arrays(|m| prism(m, o, 2.5, 0.4, -0.2, h, 4, c.steel)));
            let (at, half) = cyl(2.5, h);
            b.proxy(root, "col_cylinder", at, half);
        }
        ObstacleKind::Block => {
            let n = b.node("block", Vec3::ZERO, &[(GROUND, "drape")]);
            b.mesh(n, "concrete", &arrays(|m| {
                beam(m, across(-0.6), across(0.6), 0.3, -0.1, h, c.concrete)
            }));
            b.proxy(n, "col_box", Vec3::new(0.0, h / 2.0, 0.0), Vec3::new(0.6, h / 2.0, 0.3));
        }
        ObstacleKind::Gate => {
            span = Some(SPAN_M);
            let (a, e) = (across(-SPAN_M / 2.0), across(SPAN_M / 2.0));
            for (nm, p, x) in [("post_l", a, -SPAN_M / 2.0), ("post_r", e, SPAN_M / 2.0)] {
                let n = b.node(nm, Vec3::new(x, 0.0, 0.0), &[(SPAN, "move"), (GROUND, "node")]);
                b.mesh(n, "steel", &arrays(|m| prism(m, p, 0.08, 0.08, -0.2, h + 0.1, 6, c.steel)));
                let (at, half) = cyl(0.08, h + 0.1);
                let side = nm.trim_start_matches("post_");
                b.proxy(n, &format!("col_cylinder_{side}"), at + Vec3::X * x, half);
            }
            let n = b.node("bars", Vec3::ZERO, &[(GROUND, "drape")]);
            b.mesh(n, "steel", &arrays(|m| {
                for k in [0.35, 0.65, 0.95] {
                    beam(m, a, e, 0.03, h * k - 0.03, h * k + 0.03, c.steel);
                }
            }));
            let (lo, hi) = (h * 0.35 - 0.03, h * 0.95 + 0.03);
            b.proxy(
                n,
                "col_box",
                Vec3::new(0.0, (lo + hi) / 2.0, 0.0),
                Vec3::new(SPAN_M / 2.0, (hi - lo) / 2.0, 0.03),
            );
        }
        ObstacleKind::LiftGate => {
            span = Some(SPAN_M);
            let (a, e) = (across(-SPAN_M / 2.0), across(SPAN_M / 2.0));
            let x = -SPAN_M / 2.0;
            let post = b.node("post_l", Vec3::new(x, 0.0, 0.0), &[(SPAN, "move"), (GROUND, "node")]);
            b.mesh(post, "paint_white", &arrays(|m| {
                prism(m, a, 0.15, 0.15, -0.2, h + 0.15, 6, c.white)
            }));
            let (at, half) = cyl(0.15, h + 0.15);
            b.proxy(post, "col_cylinder", at + Vec3::X * x, half);
            // The arm stands on the post's ground (flat at the post).
            let arm = b.node("arm", Vec3::new(x, 0.0, 0.0), &[(GROUND, "node")]);
            let bar = h - 0.05;
            let n = (SPAN_M / 0.5).ceil().max(1.0) as usize;
            let (mut red, mut white) = (MeshArrays::default(), MeshArrays::default());
            for k in 0..n {
                let (t0, t1) = (k as f32 / n as f32, (k + 1) as f32 / n as f32);
                let p0 = [a[0] + (e[0] - a[0]) * t0, a[1], a[2] + (e[2] - a[2]) * t0];
                let p1 = [a[0] + (e[0] - a[0]) * t1, a[1], a[2] + (e[2] - a[2]) * t1];
                if k % 2 == 0 {
                    beam(&mut red, p0, p1, 0.04, bar - 0.05, bar + 0.05, c.red);
                } else {
                    beam(&mut white, p0, p1, 0.04, bar - 0.05, bar + 0.05, c.white);
                }
            }
            b.mesh(arm, "paint_red", &red);
            b.mesh(arm, "paint_white", &white);
            b.proxy(
                arm,
                "col_box",
                Vec3::new(0.0, bar, 0.0),
                Vec3::new(SPAN_M / 2.0, 0.05, 0.04),
            );
        }
        ObstacleKind::CycleBarrier => {
            // Two staggered rails, each across most of the way (`off`
            // along the way: model z = −off).
            for (rail, (off, from, to)) in [(-0.6f32, -1.0f32, 0.4f32), (0.6, -0.4, 1.0)]
                .into_iter()
                .enumerate()
            {
                let q = |w: f32| [off, 0.0, w];
                let (a, e) = (q(from), q(to));
                let tag = if rail == 0 { "a" } else { "b" };
                let n = b.node(&format!("rail_{tag}"), Vec3::new((from + to) / 2.0, 0.0, -off), &[(GROUND, "drape")]);
                b.mesh(n, "steel", &arrays(|m| beam(m, a, e, 0.03, h - 0.06, h, c.steel)));
                b.proxy(
                    n,
                    &format!("col_box_{tag}"),
                    Vec3::new((from + to) / 2.0, h / 2.0, -off),
                    Vec3::new((to - from) / 2.0 + 0.03, h / 2.0, 0.03),
                );
                for (end, p, w) in [("l", a, from), ("r", e, to)] {
                    let n = b.node(&format!("post_{tag}_{end}"), Vec3::new(w, 0.0, -off), &[(GROUND, "node")]);
                    b.mesh(n, "steel", &arrays(|m| prism(m, p, 0.03, 0.03, -0.1, h, 6, c.steel)));
                }
            }
        }
        _ => {
            b.mesh(root, "paint_dark", &arrays(|m| prism(m, o, 0.25, 0.2, -0.1, h, 6, c.dark)));
            let (at, half) = cyl(0.25, h);
            b.proxy(root, "col_cylinder", at, half);
        }
    }
    let mut m = b.m;
    let e = entry(&format!("{}.glb", obstacle_file(kind)));
    let fit = FileFit {
        height_m: h,
        sink_m: sink,
        span_m: span,
        reach_m: None,
    };
    ocw_assets::export::stamp(&mut m, &e, json!({ "osm": osm(kind), "fit": fit.to_json() }));
    m
}

/// A manifest entry of an exported prop.
pub fn entry(file: &str) -> Entry {
    let mut e = ocw_assets::export::entry(file, Kind::Prop, "");
    e.generator = None;
    e.source = SOURCE.into();
    e
}

/// A street lamp's model: head height `h`, the reference arm.
pub fn lamp_model(file: &str, h: f32) -> Model {
    let grey = [0.16, 0.17, 0.18, 1.0];
    let head = [0.75, 0.74, 0.70, 1.0];
    let sink = 0.1;
    let mut b = Build::new("lamp_street", sink, false);
    let arm = REACH_M;
    let pole = b.node("pole", Vec3::ZERO, &[]);
    b.mesh(pole, "paint_dark", &arrays(|m| {
        crate::street_lamps::push_box(
            m,
            Vec3::Y * (h / 2.0),
            Vec3::new(0.07, h / 2.0 + 0.1, 0.07),
            Quat::IDENTITY,
            grey,
        )
    }));
    b.proxy(pole, "col_box", Vec3::Y * (h / 2.0), Vec3::new(0.07, h / 2.0, 0.07));
    let n = b.node("arm", Vec3::new(-0.05, 0.0, 0.0), &[(REACH, "stretch")]);
    b.mesh(n, "paint_dark", &arrays(|m| {
        crate::street_lamps::push_box(
            m,
            Vec3::Y * (h + 0.05) + Vec3::X * (arm / 2.0),
            Vec3::new(arm / 2.0 + 0.05, 0.04, 0.04),
            Quat::IDENTITY,
            grey,
        )
    }));
    let n = b.node("head", Vec3::new(arm, 0.0, 0.0), &[(REACH, "move")]);
    b.mesh(n, "bulb", &arrays(|m| {
        crate::street_lamps::push_box(
            m,
            Vec3::Y * (h - 0.02) + Vec3::X * arm,
            Vec3::new(0.28, 0.06, 0.14),
            Quat::IDENTITY,
            head,
        )
    }));
    b.anchor(n, "anchor_light", Vec3::new(arm, h - 0.32, 0.0));
    let mut m = b.m;
    let e = entry(file);
    let fit = FileFit {
        height_m: h,
        sink_m: sink,
        span_m: None,
        reach_m: Some(arm),
    };
    ocw_assets::export::stamp(&mut m, &e, json!({ "osm": "highway=street_lamp", "fit": fit.to_json() }));
    m
}

/// Every exported prop: (manifest entry, model).
pub fn models() -> Vec<(Entry, Model)> {
    let mut out: Vec<(Entry, Model)> = OBSTACLES
        .iter()
        .map(|&k| (entry(&format!("{}.glb", obstacle_file(k))), obstacle_model(k)))
        .collect();
    for (file, h) in crate::street_lamps::LAMP_FILES {
        out.push((entry(&format!("{file}.glb")), lamp_model(&format!("{file}.glb"), h)));
    }
    out
}

#[cfg(test)]
#[path = "prop_export_tests.rs"]
mod tests;

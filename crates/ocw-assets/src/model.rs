//! The in-memory model of a glTF asset: a node hierarchy of rigid parts
//! with triangle meshes and named materials (spec §8.1).
//!
//! Coordinates are the app's (Bevy's): metres, Y up, the model faces **−Z**,
//! +X is its right. glTF files face +Z (glTF 2.0 §3.4); [`crate::glb`]
//! turns models by 180° about Y when writing and reading, so code never
//! sees the file convention.

use glam::{Affine3A, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// A material, named after its paint slot (see [`crate::slots`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Material {
    pub name: String,
    /// Linear RGBA (glTF `baseColorFactor`).
    pub color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    /// Linear RGB (glTF `emissiveFactor`; written only when not black).
    /// The default glow of a light's slot (`bulb`), set in OCW Studio
    /// (16B *Materials*).
    #[serde(default)]
    pub emissive: [f32; 3],
}

/// Converts an sRGB `0xRRGGBB` colour to linear RGBA.
pub fn srgb(hex: u32) -> [f32; 4] {
    let ch = |s: u32| {
        let c = ((hex >> s) & 0xFF) as f32 / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [ch(16), ch(8), ch(0), 1.0]
}

/// Triangles in one material.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Primitive {
    pub material: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Counter-clockwise triangles (front faces).
    pub indices: Vec<u32>,
    /// Vertex colours, linear RGBA (glTF `COLOR_0`, multiplied with the
    /// material's base colour); empty: none.
    pub colors: Vec<[f32; 4]>,
    /// Texture coordinates (glTF `TEXCOORD_0`); empty: none. The plants
    /// keep the height above the stem base in `u` (the wind sway).
    pub uvs: Vec<[f32; 2]>,
}

impl Primitive {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A node: a rigid part (or an empty anchor) at a transform relative to
/// its parent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub primitives: Vec<Primitive>,
    /// glTF node `extras` (Blender's custom properties); `Null`: none.
    pub extras: serde_json::Value,
}

impl Node {
    pub fn new(name: &str, parent: Option<usize>, translation: Vec3) -> Self {
        Self {
            name: name.into(),
            parent,
            translation,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
            primitives: Vec::new(),
            extras: serde_json::Value::Null,
        }
    }

    pub fn local(&self) -> Affine3A {
        Affine3A::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// Whether a node of this name holds a far level of detail (15B: `lod1`,
/// and for plants `lod2` and `billboard`).
pub fn is_far_level(name: &str) -> bool {
    matches!(name, "lod1" | "lod2" | "billboard")
}

/// The `extras.ocw` key that marks a model held in the hand (`true`): it
/// has no ground, its origin is where the hand holds it, so the validator
/// does not check its height against its origin (spec 15B, Phase 15 Q10
/// (a); the phone of the status signs).
pub const HELD: &str = "held";

/// The `extras.ocw` key of the factor the vertex colours are drawn with
/// ([`Model::shade_scale`], spec 15G Q11 (a)).
pub const SHADE_SCALE: &str = "shade_scale";
/// The values `extras.ocw.shade_scale` may take (the validator's range):
/// 1 does nothing, 2 doubles the colours – more would be a file whose
/// shading does not fit its paint colour.
pub const SHADE_SCALE_RANGE: (f32, f32) = (1.0, 2.0);

/// A vertex colour `c` as drawn with the model's shade scale `s`: the
/// colour times `s`, the alpha as it is.
pub fn shaded(c: [f32; 4], s: f32) -> [f32; 4] {
    [c[0] * s, c[1] * s, c[2] * s, c[3]]
}

/// A whole asset.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub materials: Vec<Material>,
    /// Parents come before their children.
    pub nodes: Vec<Node>,
    /// glTF `asset.copyright`.
    pub copyright: String,
    /// glTF `asset.generator`.
    pub generator: String,
    /// glTF `asset.extras` (spec §14 Phase 15, 15B: `{"ocw": {"schema",
    /// "kind", "id", …}}`); `Null`: none.
    pub extras: serde_json::Value,
}

impl Model {
    /// The model's `extras.ocw` table (15B), if any.
    pub fn ocw(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.extras.get("ocw")?.as_object()
    }

    /// Index of the node called `name`.
    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
    }

    /// Whether it is held in the hand (`extras.ocw.held = true`, [`HELD`]).
    pub fn is_held(&self) -> bool {
        self.ocw()
            .and_then(|o| o.get(HELD))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    }

    /// The factor its vertex colours are drawn with (spec 15G Q11 (a)):
    /// `extras.ocw.shade_scale` – the exporter kept the colours at most 1
    /// (Blender stores them as 8 bits, clamped to 1) by dividing them by it,
    /// and the renderer multiplies them back. 1 without the key; a value
    /// out of [`SHADE_SCALE_RANGE`] is clamped into it (the validator
    /// reports it), anything that is not a number counts as 1.
    pub fn shade_scale(&self) -> f32 {
        let (lo, hi) = SHADE_SCALE_RANGE;
        self.ocw()
            .and_then(|o| o.get(SHADE_SCALE))
            .and_then(serde_json::Value::as_f64)
            .filter(|s| s.is_finite())
            .map_or(1.0, |s| (s as f32).clamp(lo, hi))
    }

    /// Scales the vertex colours down to at most 1 where they are brighter
    /// and writes the factor as `extras.ocw.shade_scale` (rounded up to
    /// 1/1000, so every colour stays ≤ 1); [`Model::shade_scale`] times a
    /// stored colour is the colour as it was (to the float's precision).
    /// Colours at most 1 are left alone and no key is written. Returns the
    /// factor (1: none). Alpha is not scaled.
    pub fn fit_shading(&mut self) -> f32 {
        let max = self
            .nodes
            .iter()
            .flat_map(|n| &n.primitives)
            .flat_map(|p| &p.colors)
            .map(|c| c[0].max(c[1]).max(c[2]))
            .fold(0.0f32, f32::max);
        if max <= 1.0 {
            return 1.0;
        }
        // (Less a hair, so that an f32 just above 1.08 gives 1.08, not 1.081;
        // the clamp below keeps the colours ≤ 1.)
        let s = (f64::from(max) * 1000.0 - 1e-3).ceil() / 1000.0;
        assert!(
            s <= f64::from(SHADE_SCALE_RANGE.1),
            "vertex colours up to {max}: more than the shade scale allows"
        );
        let k = s as f32;
        for c in self
            .nodes
            .iter_mut()
            .flat_map(|n| &mut n.primitives)
            .flat_map(|p| &mut p.colors)
        {
            for ch in &mut c[..3] {
                *ch = (*ch / k).min(1.0);
            }
        }
        if !self.extras.is_object() {
            self.extras = serde_json::json!({});
        }
        let root = self.extras.as_object_mut().expect("an object");
        let ocw = root.entry("ocw").or_insert_with(|| serde_json::json!({}));
        if !ocw.is_object() {
            *ocw = serde_json::json!({});
        }
        ocw.as_object_mut()
            .expect("an object")
            .insert(SHADE_SCALE.into(), serde_json::json!(s));
        k
    }

    /// Index of the material called `name`.
    pub fn material(&self, name: &str) -> Option<usize> {
        self.materials.iter().position(|m| m.name == name)
    }

    /// The model-space transform of node `i`.
    pub fn world(&self, i: usize) -> Affine3A {
        let n = &self.nodes[i];
        match n.parent {
            Some(p) => self.world(p) * n.local(),
            None => n.local(),
        }
    }

    /// Model-space position of the node called `name`.
    pub fn position(&self, name: &str) -> Option<Vec3> {
        self.node(name)
            .map(|i| Vec3::from(self.world(i).translation))
    }

    /// Whether node `i` is `ancestor` or below it.
    pub fn is_below(&self, mut i: usize, ancestor: usize) -> bool {
        loop {
            if i == ancestor {
                return true;
            }
            match self.nodes[i].parent {
                Some(p) => i = p,
                None => return false,
            }
        }
    }

    /// Triangles of the nodes below `root` (all nodes for `None`).
    pub fn triangles_below(&self, root: Option<usize>) -> usize {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| root.is_none_or(|r| self.is_below(*i, r)))
            .map(|(_, n)| n.primitives.iter().map(Primitive::triangles).sum::<usize>())
            .sum()
    }

    pub fn triangles(&self) -> usize {
        self.triangles_below(None)
    }

    /// Vertices of the nodes below `root` (all nodes for `None`).
    pub fn vertices_below(&self, root: Option<usize>) -> usize {
        self.nodes
            .iter()
            .enumerate()
            .filter(|(i, _)| root.is_none_or(|r| self.is_below(*i, r)))
            .map(|(_, n)| {
                n.primitives
                    .iter()
                    .map(|p| p.positions.len())
                    .sum::<usize>()
            })
            .sum()
    }

    pub fn vertices(&self) -> usize {
        self.vertices_below(None)
    }

    /// The model-space bounds (min, max) of the meshes of the nodes `keep`
    /// accepts; `None` without a vertex.
    pub fn bounds_where(&self, keep: impl Fn(usize) -> bool) -> Option<(Vec3, Vec3)> {
        let mut lo = Vec3::splat(f32::INFINITY);
        let mut hi = Vec3::splat(f32::NEG_INFINITY);
        for (i, n) in self.nodes.iter().enumerate() {
            if n.primitives.is_empty() || !keep(i) {
                continue;
            }
            let w = self.world(i);
            for p in &n.primitives {
                for v in &p.positions {
                    let q = w.transform_point3(Vec3::from(*v));
                    lo = lo.min(q);
                    hi = hi.max(q);
                }
            }
        }
        lo.x.is_finite().then_some((lo, hi))
    }

    /// The node `i` or one of its ancestors is called like `pred` accepts.
    pub fn under_named(&self, i: usize, pred: impl Fn(&str) -> bool) -> bool {
        let mut at = Some(i);
        while let Some(n) = at {
            if pred(&self.nodes[n].name) {
                return true;
            }
            at = self.nodes[n].parent;
        }
        false
    }

    /// The bounds of the near model: every mesh except the far levels
    /// (`lod1`, `lod2`, `billboard`) and the collision proxies (`col_*`).
    pub fn near_bounds(&self) -> Option<(Vec3, Vec3)> {
        self.bounds_where(|i| !self.under_named(i, |n| is_far_level(n) || n.starts_with("col_")))
    }

    /// Adds a node and returns its index.
    pub fn add(&mut self, node: Node) -> usize {
        debug_assert!(node.parent.is_none_or(|p| p < self.nodes.len()));
        self.nodes.push(node);
        self.nodes.len() - 1
    }

    /// The material index for `name`, added with these values if new.
    pub fn material_or_add(&mut self, m: Material) -> usize {
        match self.material(&m.name) {
            Some(i) => i,
            None => {
                self.materials.push(m);
                self.materials.len() - 1
            }
        }
    }
}

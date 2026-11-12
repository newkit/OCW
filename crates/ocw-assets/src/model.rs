//! The in-memory model of a glTF asset: a node hierarchy of rigid parts
//! with triangle meshes and named materials (spec §8.1).
//!
//! Coordinates are the app's (Bevy's): metres, Y up, the model faces **−Z**,
//! +X is its right. glTF files face +Z (glTF 2.0 §3.4); [`crate::glb`]
//! turns models by 180° about Y when writing and reading, so code never
//! sees the file convention.

use glam::{Affine3A, Quat, Vec3};

/// A material, named after its paint slot (see [`crate::slots`]).
#[derive(Debug, Clone, PartialEq)]
pub struct Material {
    pub name: String,
    /// Linear RGBA (glTF `baseColorFactor`).
    pub color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
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
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Primitive {
    pub material: usize,
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    /// Counter-clockwise triangles (front faces).
    pub indices: Vec<u32>,
}

impl Primitive {
    pub fn triangles(&self) -> usize {
        self.indices.len() / 3
    }
}

/// A node: a rigid part (or an empty anchor) at a transform relative to
/// its parent.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    pub name: String,
    pub parent: Option<usize>,
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
    pub primitives: Vec<Primitive>,
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
        }
    }

    pub fn local(&self) -> Affine3A {
        Affine3A::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }
}

/// A whole asset.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Model {
    pub materials: Vec<Material>,
    /// Parents come before their children.
    pub nodes: Vec<Node>,
    /// glTF `asset.copyright`.
    pub copyright: String,
    /// glTF `asset.generator`.
    pub generator: String,
}

impl Model {
    /// Index of the node called `name`.
    pub fn node(&self, name: &str) -> Option<usize> {
        self.nodes.iter().position(|n| n.name == name)
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

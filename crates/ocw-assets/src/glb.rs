//! Binary glTF 2.0 (`.glb`) writing and reading for [`Model`]s.
//!
//! Written files are plain glTF 2.0: one scene, nodes with translation /
//! rotation, one mesh per node with one primitive per material (POSITION,
//! NORMAL, indices), metallic-roughness materials, `asset.copyright` with
//! the licence. Reading uses the `gltf` crate (the one Bevy's loader
//! uses) and accepts any triangle-mesh glTF with embedded buffers; only
//! node TRS transforms are supported (no skins, morph targets, textures –
//! the app's assets are rigid parts, spec §2.6 decision).

use glam::{Quat, Vec3};
use serde_json::{Value, json};

use crate::model::{Material, Model, Node, Primitive};

/// glTF files face +Z, the app's models −Z: the same 180° turn about Y in
/// both directions.
fn flip(v: [f32; 3]) -> [f32; 3] {
    [-v[0], v[1], -v[2]]
}

fn flip_quat(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, q.y, -q.z, q.w)
}

/// Errors reading a `.glb`.
#[derive(Debug)]
pub struct GlbError(pub String);

impl std::fmt::Display for GlbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for GlbError {}

fn err(s: impl Into<String>) -> GlbError {
    GlbError(s.into())
}

/// Rounds to 1/65536 m: the files do not change with the last bits of
/// `sin`/`cos` on another platform.
fn q(v: f32) -> f32 {
    (v * 65536.0).round() / 65536.0
}

/// Writes `model` as a `.glb`.
pub fn write(model: &Model) -> Vec<u8> {
    let mut bin: Vec<u8> = Vec::new();
    let mut views = Vec::new();
    let mut accessors = Vec::new();
    let mut add_view = |bin: &mut Vec<u8>, bytes: &[u8], target: u32| -> usize {
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let offset = bin.len();
        bin.extend_from_slice(bytes);
        views.push(json!({
            "buffer": 0, "byteOffset": offset, "byteLength": bytes.len(), "target": target
        }));
        views.len() - 1
    };
    let mut meshes = Vec::new();
    let mut nodes: Vec<Value> = Vec::new();
    for node in &model.nodes {
        let mut n = json!({ "name": node.name });
        let t = flip(node.translation.to_array()).map(q);
        if t != [0.0; 3] {
            n["translation"] = json!(t);
        }
        if node.rotation != Quat::IDENTITY {
            n["rotation"] = json!(
                flip_quat(node.rotation)
                    .to_array()
                    .map(|v| q(v * 4.0) / 4.0)
            );
        }
        if node.scale != Vec3::ONE {
            n["scale"] = json!(node.scale.to_array());
        }
        if !node.primitives.is_empty() {
            let mut prims = Vec::new();
            for p in &node.primitives {
                let pos: Vec<[f32; 3]> = p.positions.iter().map(|v| flip(*v).map(q)).collect();
                let nrm: Vec<[f32; 3]> = p
                    .normals
                    .iter()
                    .map(|v| flip(*v).map(|c| (c * 32767.0).round() / 32767.0))
                    .collect();
                let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                for v in &pos {
                    for k in 0..3 {
                        min[k] = min[k].min(v[k]);
                        max[k] = max[k].max(v[k]);
                    }
                }
                let bytes: Vec<u8> = pos.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
                let pv = add_view(&mut bin, &bytes, 34962);
                accessors.push(json!({
                    "bufferView": pv, "componentType": 5126, "count": pos.len(),
                    "type": "VEC3", "min": min, "max": max
                }));
                let pa = accessors.len() - 1;
                let bytes: Vec<u8> = nrm.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
                let nv = add_view(&mut bin, &bytes, 34962);
                accessors.push(json!({
                    "bufferView": nv, "componentType": 5126, "count": nrm.len(), "type": "VEC3"
                }));
                let na = accessors.len() - 1;
                let (bytes, ty): (Vec<u8>, u32) = if pos.len() <= u16::MAX as usize {
                    (
                        p.indices
                            .iter()
                            .flat_map(|i| (*i as u16).to_le_bytes())
                            .collect(),
                        5123,
                    )
                } else {
                    (
                        p.indices.iter().flat_map(|i| i.to_le_bytes()).collect(),
                        5125,
                    )
                };
                let iv = add_view(&mut bin, &bytes, 34963);
                accessors.push(json!({
                    "bufferView": iv, "componentType": ty, "count": p.indices.len(), "type": "SCALAR"
                }));
                let ia = accessors.len() - 1;
                prims.push(json!({
                    "attributes": { "POSITION": pa, "NORMAL": na },
                    "indices": ia, "material": p.material, "mode": 4
                }));
            }
            meshes.push(json!({ "name": node.name, "primitives": prims }));
            n["mesh"] = json!(meshes.len() - 1);
        }
        nodes.push(n);
    }
    for (i, node) in model.nodes.iter().enumerate() {
        if let Some(p) = node.parent {
            let children = nodes[p]
                .as_object_mut()
                .expect("node object")
                .entry("children")
                .or_insert_with(|| json!([]));
            children.as_array_mut().expect("children").push(json!(i));
        }
    }
    let roots: Vec<usize> = (0..model.nodes.len())
        .filter(|&i| model.nodes[i].parent.is_none())
        .collect();
    let materials: Vec<Value> = model
        .materials
        .iter()
        .map(|m| {
            json!({
                "name": m.name,
                "pbrMetallicRoughness": {
                    "baseColorFactor": m.color.map(|c| q(c)),
                    "metallicFactor": m.metallic,
                    "roughnessFactor": m.roughness
                }
            })
        })
        .collect();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let mut doc = json!({
        "asset": { "version": "2.0", "generator": model.generator },
        "scene": 0,
        "scenes": [{ "nodes": roots }],
        "nodes": nodes,
        "materials": materials,
        "buffers": [{ "byteLength": bin.len() }],
    });
    if !model.copyright.is_empty() {
        doc["asset"]["copyright"] = json!(model.copyright);
    }
    if !meshes.is_empty() {
        doc["meshes"] = json!(meshes);
        doc["bufferViews"] = json!(views);
        doc["accessors"] = json!(accessors);
    }
    let mut text = serde_json::to_vec(&doc).expect("JSON");
    while text.len() % 4 != 0 {
        text.push(b' ');
    }
    let total = 12 + 8 + text.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&text);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(b"BIN\0");
    out.extend_from_slice(&bin);
    out
}

/// Reads a `.glb` (or `.gltf` with embedded buffers) into a [`Model`].
pub fn read(bytes: &[u8]) -> Result<Model, GlbError> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| err(format!("not glTF 2.0: {e}")))?;
    let blob = gltf.blob.as_deref();
    let doc = &gltf.document;
    let asset = doc.as_json().asset.clone();
    let materials = doc
        .materials()
        .map(|m| {
            let pbr = m.pbr_metallic_roughness();
            Material {
                name: m.name().unwrap_or("").into(),
                color: pbr.base_color_factor(),
                metallic: pbr.metallic_factor(),
                roughness: pbr.roughness_factor(),
            }
        })
        .collect();
    // Parents first: walk the default scene's hierarchy depth-first.
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .ok_or_else(|| err("no scene"))?;
    let mut model = Model {
        materials,
        nodes: Vec::new(),
        copyright: asset.copyright.unwrap_or_default(),
        generator: asset.generator.unwrap_or_default(),
    };
    let mut stack: Vec<(gltf::Node, Option<usize>)> = scene.nodes().map(|n| (n, None)).collect();
    stack.reverse();
    let mut seen = std::collections::BTreeSet::new();
    while let Some((n, parent)) = stack.pop() {
        if !seen.insert(n.index()) {
            return Err(err("node graph is not a tree"));
        }
        let (t, r, s) = n.transform().decomposed();
        let mut node = Node::new(n.name().unwrap_or(""), parent, Vec3::from(flip(t)));
        node.rotation = flip_quat(Quat::from_array(r)).normalize();
        node.scale = Vec3::from(s);
        if n.skin().is_some() {
            return Err(err(format!("node {}: skins are not supported", node.name)));
        }
        if let Some(mesh) = n.mesh() {
            for p in mesh.primitives() {
                if p.mode() != gltf::mesh::Mode::Triangles {
                    return Err(err(format!("node {}: only triangle meshes", node.name)));
                }
                let reader = p.reader(|b| match b.source() {
                    gltf::buffer::Source::Bin => blob,
                    gltf::buffer::Source::Uri(_) => None,
                });
                let positions: Vec<[f32; 3]> = reader
                    .read_positions()
                    .ok_or_else(|| err(format!("node {}: no POSITION", node.name)))?
                    .map(flip)
                    .collect();
                let normals: Vec<[f32; 3]> = reader
                    .read_normals()
                    .ok_or_else(|| err(format!("node {}: no NORMAL", node.name)))?
                    .map(flip)
                    .collect();
                let indices: Vec<u32> = match reader.read_indices() {
                    Some(i) => i.into_u32().collect(),
                    None => (0..positions.len() as u32).collect(),
                };
                if normals.len() != positions.len()
                    || indices.len() % 3 != 0
                    || indices.iter().any(|&i| i as usize >= positions.len())
                {
                    return Err(err(format!("node {}: broken primitive", node.name)));
                }
                let material = p.material().index().ok_or_else(|| {
                    err(format!("node {}: primitive without material", node.name))
                })?;
                node.primitives.push(Primitive {
                    material,
                    positions,
                    normals,
                    indices,
                });
            }
        }
        let i = model.add(node);
        for c in n.children().collect::<Vec<_>>().into_iter().rev() {
            stack.push((c, Some(i)));
        }
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Parts;

    fn sample() -> Model {
        let mut m = Model {
            copyright: "test".into(),
            generator: "unit test".into(),
            ..Default::default()
        };
        let root = m.add(Node::new("root", None, Vec3::new(0.0, 1.0, -2.0)));
        let mut child = Node::new("child", Some(root), Vec3::new(0.5, 0.0, 0.0));
        child.rotation = Quat::from_rotation_x(0.3);
        let c = m.add(child);
        let mut p = Parts::new();
        p.slot("frame")
            .cuboid(Vec3::new(0.5, 1.0, -2.0), Vec3::splat(0.1), Quat::IDENTITY);
        p.slot("tyre").tube(Vec3::ZERO, Vec3::Y, 0.1, 0.1, 6, true);
        p.attach(&mut m, c);
        m.add(Node::new("anchor", Some(root), Vec3::new(0.0, 0.2, -0.1)));
        m
    }

    #[test]
    fn a_written_model_reads_back_the_same() {
        let m = sample();
        let bytes = write(&m);
        assert_eq!(&bytes[..4], b"glTF");
        assert_eq!(bytes.len() % 4, 0);
        let back = read(&bytes).unwrap();
        assert_eq!(back.copyright, "test");
        assert_eq!(back.nodes.len(), 3);
        assert_eq!(back.materials.len(), 2);
        for (a, b) in m.nodes.iter().zip(&back.nodes) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.parent, b.parent);
            assert!(a.translation.distance(b.translation) < 1e-4);
            assert!(a.rotation.angle_between(b.rotation) < 1e-3);
            assert_eq!(a.primitives.len(), b.primitives.len());
            for (pa, pb) in a.primitives.iter().zip(&b.primitives) {
                assert_eq!(pa.indices, pb.indices);
                assert_eq!(pa.material, pb.material);
                for (x, y) in pa.positions.iter().zip(&pb.positions) {
                    assert!(Vec3::from(*x).distance(Vec3::from(*y)) < 1e-4);
                }
            }
        }
        // World positions survive the file's +Z convention.
        let a = m.position("anchor").unwrap();
        assert!(a.distance(back.position("anchor").unwrap()) < 1e-4);
        assert!(a.distance(Vec3::new(0.0, 1.2, -2.1)) < 1e-5);
    }

    #[test]
    fn the_file_faces_plus_z() {
        let bytes = write(&sample());
        let g = gltf::Gltf::from_slice(&bytes).unwrap();
        let root = g
            .document
            .nodes()
            .find(|n| n.name() == Some("root"))
            .unwrap();
        let (t, _, _) = root.transform().decomposed();
        // The app's -Z (forward) is +Z in the file.
        assert_eq!(t, [0.0, 1.0, 2.0]);
    }

    #[test]
    fn broken_files_are_refused() {
        assert!(read(b"not a glb").is_err());
        let mut bytes = write(&sample());
        bytes.truncate(40);
        assert!(read(&bytes).is_err());
    }
}

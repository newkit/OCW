//! Binary glTF 2.0 (`.glb`) writing and reading for [`Model`]s.
//!
//! Written files are plain glTF 2.0: one scene, nodes with translation /
//! rotation, one mesh per node with one primitive per material (POSITION,
//! NORMAL, optional COLOR_0 and TEXCOORD_0, indices), metallic-roughness
//! materials, `asset.copyright` with the licence, `extras` on the asset
//! and the nodes (spec §14 Phase 15, 15B: `asset.extras.ocw`). Reading
//! uses the `gltf` crate (the one Bevy's loader uses) and accepts any
//! triangle-mesh glTF: a `.glb`, or a `.gltf` whose buffers are data URIs
//! or files next to it ([`read_with`]); only node TRS transforms are
//! supported (no skins, morph targets or textures yet – the app's assets
//! are rigid parts, spec §2.6 decision), and an extension in
//! `extensionsRequired` is an error naming it.

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
        while !bin.len().is_multiple_of(4) {
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
        if !node.extras.is_null() {
            n["extras"] = node.extras.clone();
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
                let mut attributes = json!({ "POSITION": pa, "NORMAL": na });
                if p.colors.len() == p.positions.len() && !p.colors.is_empty() {
                    let bytes: Vec<u8> = p
                        .colors
                        .iter()
                        .flatten()
                        .flat_map(|f| q(*f).to_le_bytes())
                        .collect();
                    let cv = add_view(&mut bin, &bytes, 34962);
                    accessors.push(json!({
                        "bufferView": cv, "componentType": 5126, "count": p.colors.len(), "type": "VEC4"
                    }));
                    attributes["COLOR_0"] = json!(accessors.len() - 1);
                }
                if p.uvs.len() == p.positions.len() && !p.uvs.is_empty() {
                    let bytes: Vec<u8> = p
                        .uvs
                        .iter()
                        .flatten()
                        .flat_map(|f| q(*f).to_le_bytes())
                        .collect();
                    let uv = add_view(&mut bin, &bytes, 34962);
                    accessors.push(json!({
                        "bufferView": uv, "componentType": 5126, "count": p.uvs.len(), "type": "VEC2"
                    }));
                    attributes["TEXCOORD_0"] = json!(accessors.len() - 1);
                }
                prims.push(json!({
                    "attributes": attributes,
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
                    "baseColorFactor": m.color.map(q),
                    "metallicFactor": m.metallic,
                    "roughnessFactor": m.roughness
                }
            })
        })
        .collect();
    while !bin.len().is_multiple_of(4) {
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
    if !model.extras.is_null() {
        doc["asset"]["extras"] = model.extras.clone();
    }
    if !meshes.is_empty() {
        doc["meshes"] = json!(meshes);
        doc["bufferViews"] = json!(views);
        doc["accessors"] = json!(accessors);
    }
    let mut text = serde_json::to_vec(&doc).expect("JSON");
    while !text.len().is_multiple_of(4) {
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

/// Reads a `.glb` (or a `.gltf` whose buffers are embedded or data URIs)
/// into a [`Model`].
pub fn read(bytes: &[u8]) -> Result<Model, GlbError> {
    read_with(bytes, &|_| None)
}

/// Reads a `.gltf` file and the buffers it names next to it (Blender's
/// "glTF Separate" export, spec §14 Phase 15, 15D), or a `.glb`.
#[cfg(not(target_arch = "wasm32"))]
pub fn read_file(path: &std::path::Path) -> Result<Model, GlbError> {
    let bytes = std::fs::read(path).map_err(|e| err(format!("{}: {e}", path.display())))?;
    let dir = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    read_with(&bytes, &|uri| {
        // Only plain relative names: never leave the model's folder.
        let uri = uri.replace("%20", " ");
        let rel = std::path::Path::new(&uri);
        let plain = rel
            .components()
            .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !plain {
            return None;
        }
        std::fs::read(dir.join(rel)).ok()
    })
}

/// The JSON of a `.glb` (its first chunk) or of a `.gltf` (all of it).
fn json_of(bytes: &[u8]) -> Option<Value> {
    if bytes.starts_with(b"glTF") {
        let len = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
        serde_json::from_slice(bytes.get(20..20 + len)?).ok()
    } else {
        serde_json::from_slice(bytes).ok()
    }
}

/// Decodes a `data:…;base64,` URI.
fn data_uri(uri: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let rest = uri.strip_prefix("data:")?;
    let (_, data) = rest.split_once(";base64,")?;
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

/// The extensions this reader supports when a file requires them.
pub const SUPPORTED_REQUIRED: [&str; 0] = [];

/// [`read`] with `resolve` giving the bytes of an external buffer URI
/// (`None`: not available).
pub fn read_with(
    bytes: &[u8],
    resolve: &dyn Fn(&str) -> Option<Vec<u8>>,
) -> Result<Model, GlbError> {
    let json = json_of(bytes).unwrap_or(Value::Null);
    if let Some(list) = json["extensionsRequired"].as_array() {
        for ext in list.iter().filter_map(Value::as_str) {
            if !SUPPORTED_REQUIRED.contains(&ext) {
                return Err(err(format!(
                    "the file requires the glTF extension {ext}, which is not supported"
                )));
            }
        }
    }
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| err(format!("not glTF 2.0: {e}")))?;
    let blob = gltf.blob.as_deref();
    let doc = &gltf.document;
    // External and data-URI buffers, by index.
    let mut external: Vec<Option<Vec<u8>>> = Vec::new();
    for b in doc.buffers() {
        external.push(match b.source() {
            gltf::buffer::Source::Bin => None,
            gltf::buffer::Source::Uri(uri) => {
                let data = data_uri(uri)
                    .or_else(|| resolve(uri))
                    .ok_or_else(|| err(format!("buffer {}: cannot read `{uri}`", b.index())))?;
                if data.len() < b.length() {
                    return Err(err(format!("buffer {}: `{uri}` is too short", b.index())));
                }
                Some(data)
            }
        });
    }
    let node_extras = |i: usize| -> Value {
        json["nodes"][i]
            .get("extras")
            .cloned()
            .unwrap_or(Value::Null)
    };
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
        extras: json["asset"].get("extras").cloned().unwrap_or(Value::Null),
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
        node.extras = node_extras(n.index());
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
                    gltf::buffer::Source::Uri(_) => external[b.index()].as_deref(),
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
                    || !indices.len().is_multiple_of(3)
                    || indices.iter().any(|&i| i as usize >= positions.len())
                {
                    return Err(err(format!("node {}: broken primitive", node.name)));
                }
                let colors: Vec<[f32; 4]> = reader
                    .read_colors(0)
                    .map(|c| c.into_rgba_f32().collect())
                    .unwrap_or_default();
                let uvs: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|t| t.into_f32().collect())
                    .unwrap_or_default();
                if (!colors.is_empty() && colors.len() != positions.len())
                    || (!uvs.is_empty() && uvs.len() != positions.len())
                {
                    return Err(err(format!("node {}: broken vertex attributes", node.name)));
                }
                let material = p.material().index().ok_or_else(|| {
                    err(format!("node {}: primitive without material", node.name))
                })?;
                node.primitives.push(Primitive {
                    material,
                    positions,
                    normals,
                    indices,
                    colors,
                    uvs,
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

    /// Spec §14 Phase 15, 15F: vertex colours, texture coordinates and
    /// `extras` (the asset's and the nodes') survive a round trip, and a
    /// re-write is byte-identical.
    #[test]
    fn colours_uvs_and_extras_round_trip() {
        let mut m = sample();
        m.extras = json!({"ocw": {"schema": 1, "kind": "prop", "id": "props/test-cube"}});
        m.nodes[1].extras = json!({"role": "lamp", "lumen": 800});
        let p = &mut m.nodes[1].primitives[0];
        let n = p.positions.len();
        p.colors = (0..n)
            .map(|i| [i as f32 / n as f32, 0.5, 0.25, 1.0])
            .collect();
        p.uvs = (0..n).map(|i| [i as f32 * 0.1, 1.0]).collect();
        let bytes = write(&m);
        let back = read(&bytes).unwrap();
        assert_eq!(back.extras, m.extras);
        assert_eq!(back.nodes[1].extras, m.nodes[1].extras);
        assert_eq!(back.nodes[0].extras, Value::Null);
        let (a, b) = (&m.nodes[1].primitives[0], &back.nodes[1].primitives[0]);
        assert_eq!(b.colors.len(), n);
        assert_eq!(b.uvs.len(), n);
        for (x, y) in a.colors.iter().zip(&b.colors) {
            assert!((0..4).all(|k| (x[k] - y[k]).abs() < 1e-4));
        }
        for (x, y) in a.uvs.iter().zip(&b.uvs) {
            assert!((0..2).all(|k| (x[k] - y[k]).abs() < 1e-4));
        }
        assert!(back.nodes[1].primitives[1].colors.is_empty());
        assert_eq!(write(&back), bytes);
    }

    /// The `.gltf` form (Blender's "glTF Separate"): the buffer as a data
    /// URI or as a file next to it.
    #[test]
    fn gltf_text_files_with_separate_buffers_read() {
        use base64::Engine;
        let bytes = write(&sample());
        let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut doc: Value = serde_json::from_slice(&bytes[20..20 + len]).unwrap();
        let bin = bytes[20 + len + 8..].to_vec();
        doc["buffers"][0]["uri"] = json!("sample.bin");
        let text = serde_json::to_vec(&doc).unwrap();
        let resolve = |uri: &str| (uri == "sample.bin").then(|| bin.clone());
        let a = read_with(&text, &resolve).unwrap();
        assert_eq!(a, read(&bytes).unwrap());
        assert!(read(&text).unwrap_err().0.contains("sample.bin"));
        let data = base64::engine::general_purpose::STANDARD.encode(&bin);
        doc["buffers"][0]["uri"] = json!(format!("data:application/octet-stream;base64,{data}"));
        let b = read(&serde_json::to_vec(&doc).unwrap()).unwrap();
        assert_eq!(b, a);
    }

    /// An extension the reader does not know in `extensionsRequired` is an
    /// error naming it (15D).
    #[test]
    fn required_extensions_are_named() {
        let bytes = write(&sample());
        let len = u32::from_le_bytes(bytes[12..16].try_into().unwrap()) as usize;
        let mut doc: Value = serde_json::from_slice(&bytes[20..20 + len]).unwrap();
        doc["extensionsUsed"] = json!(["KHR_draco_mesh_compression"]);
        doc["extensionsRequired"] = json!(["KHR_draco_mesh_compression"]);
        doc["buffers"][0]["uri"] = json!("x.bin");
        let e = read(&serde_json::to_vec(&doc).unwrap()).unwrap_err();
        assert!(e.0.contains("KHR_draco_mesh_compression"), "{e}");
    }
}

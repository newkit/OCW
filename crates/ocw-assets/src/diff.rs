//! The structural diff of two models (spec §14 Phase 15, M2; 15D `model
//! diff A B`): what a review of a binary `.glb` needs – nodes added,
//! removed, moved or re-parented, meshes with their triangle and vertex
//! counts (and whether the geometry changed at the same counts),
//! materials, `extras` (flattened to paths), the asset's copyright and
//! generator, and the manifest entries. Two identical models have an
//! empty diff; a canonical re-export of a Blender round trip should too
//! (15F, `make test-blender`).
//!
//! The geometry is compared as what is drawn: the triangles with their
//! corners (position, normal, colour, UV), in any order, from any corner,
//! with the same winding. A mesh whose vertices were only re-indexed,
//! split or merged (Blender's exporter does both) is the same mesh; its
//! vertex count shows in the totals line but is no change.

use std::collections::BTreeMap;

use glam::Vec3;
use serde::Serialize;
use serde_json::Value;

use crate::manifest::Entry;
use crate::model::{Model, Node, Primitive};

/// Positions, colours, UVs and transforms closer than this count as the
/// same (the writer rounds to 1/65536).
pub const TOLERANCE: f32 = 1e-4;

/// Normal components closer than this (about 0.1°) count as the same:
/// Blender keeps custom normals at a lower precision (a round trip moves
/// them by up to 7e-4, measured on the committed models, 2026-11-19).
pub const NORMAL_TOLERANCE: f32 = 2e-3;

/// Vertex colour components closer than this count as the same: Blender
/// keeps vertex colours as 8-bit sRGB (`BYTE_COLOR`), so a round trip moves
/// them by up to half a step (0.003 measured, 2026-11-19).
pub const COLOR_TOLERANCE: f32 = 5e-3;

/// The tolerance of a [`Corner`] component.
fn corner_tolerance(i: usize) -> f32 {
    match i {
        3..6 => NORMAL_TOLERANCE,
        6..10 => COLOR_TOLERANCE,
        _ => TOLERANCE,
    }
}

/// A mesh's counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeshStats {
    pub triangles: usize,
    pub vertices: usize,
    /// The materials of its primitives.
    pub materials: Vec<String>,
}

/// One difference.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum Change {
    NodeAdded {
        name: String,
        parent: Option<String>,
    },
    NodeRemoved {
        name: String,
    },
    NodeParent {
        name: String,
        before: Option<String>,
        after: Option<String>,
    },
    NodeTransform {
        name: String,
        field: &'static str,
        before: [f32; 4],
        after: [f32; 4],
    },
    Mesh {
        node: String,
        before: MeshStats,
        after: MeshStats,
        /// The geometry (positions, normals, indices, colours, UVs) moved
        /// although the counts may be the same.
        geometry: bool,
        /// What alone explains the move, when the triangles are the same
        /// otherwise: `normals`, `colours` (both); `positions` when the
        /// shape itself (positions, winding or UVs) moved.
        #[serde(skip_serializing_if = "Vec::is_empty")]
        what: Vec<&'static str>,
    },
    NodeExtras {
        name: String,
        before: Value,
        after: Value,
    },
    MaterialAdded {
        name: String,
    },
    MaterialRemoved {
        name: String,
    },
    MaterialChanged {
        name: String,
        field: &'static str,
        before: [f32; 4],
        after: [f32; 4],
    },
    Asset {
        field: &'static str,
        before: String,
        after: String,
    },
    Extras {
        path: String,
        before: Option<Value>,
        after: Option<Value>,
    },
    Manifest {
        key: String,
        before: Option<String>,
        after: Option<String>,
    },
}

fn v4(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|x| format!("{x:.4}")).collect();
    format!("({})", parts.join(", "))
}

fn opt<T: std::fmt::Display>(v: &Option<T>) -> String {
    v.as_ref().map_or_else(|| "–".into(), ToString::to_string)
}

impl std::fmt::Display for Change {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Change::NodeAdded { name, parent } => match parent {
                Some(p) => write!(f, "+ node {name} (under {p})"),
                None => write!(f, "+ node {name} (a root)"),
            },
            Change::NodeRemoved { name } => write!(f, "- node {name}"),
            Change::NodeParent {
                name,
                before,
                after,
            } => write!(f, "~ node {name}: parent {} → {}", opt(before), opt(after)),
            Change::NodeTransform {
                name,
                field,
                before,
                after,
            } => {
                let n = if *field == "rotation" { 4 } else { 3 };
                write!(
                    f,
                    "~ node {name}: {field} {} → {}",
                    v4(&before[..n]),
                    v4(&after[..n])
                )
            }
            Change::Mesh {
                node,
                before,
                after,
                geometry,
                what,
            } => {
                write!(f, "~ mesh {node}:")?;
                let mut any = false;
                if before.triangles != after.triangles {
                    write!(f, " {} → {} triangles", before.triangles, after.triangles)?;
                    any = true;
                }
                if before.vertices != after.vertices {
                    write!(
                        f,
                        "{} {} → {} vertices",
                        if any { "," } else { "" },
                        before.vertices,
                        after.vertices
                    )?;
                    any = true;
                }
                if before.materials != after.materials {
                    write!(
                        f,
                        "{} materials [{}] → [{}]",
                        if any { "," } else { "" },
                        before.materials.join(", "),
                        after.materials.join(", ")
                    )?;
                    any = true;
                }
                if !any && *geometry {
                    write!(f, " geometry changed (same counts)")?;
                } else if *geometry && before.triangles == after.triangles {
                    // Only the vertex count shows: say that the triangles moved.
                    write!(f, ", geometry changed")?;
                }
                if *geometry && !what.is_empty() {
                    write!(f, ": {}", what.join(", "))?;
                }
                Ok(())
            }
            Change::NodeExtras {
                name,
                before,
                after,
            } => write!(f, "~ node {name}: extras {before} → {after}"),
            Change::MaterialAdded { name } => write!(f, "+ material {name}"),
            Change::MaterialRemoved { name } => write!(f, "- material {name}"),
            Change::MaterialChanged {
                name,
                field,
                before,
                after,
            } => write!(
                f,
                "~ material {name}: {field} {} → {}",
                v4(before),
                v4(after)
            ),
            Change::Asset {
                field,
                before,
                after,
            } => write!(f, "~ asset.{field}: \"{before}\" → \"{after}\""),
            Change::Extras {
                path,
                before,
                after,
            } => write!(f, "~ extras {path}: {} → {}", opt(before), opt(after)),
            Change::Manifest { key, before, after } => {
                write!(f, "~ manifest {key}: {} → {}", opt(before), opt(after))
            }
        }
    }
}

/// The counts of a whole model.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Totals {
    pub nodes: usize,
    pub materials: usize,
    pub triangles: usize,
    pub vertices: usize,
    /// Near-model bounds (min, max).
    pub bounds: Option<([f32; 3], [f32; 3])>,
}

impl Totals {
    pub fn of(m: &Model) -> Self {
        Self {
            nodes: m.nodes.len(),
            materials: m.materials.len(),
            triangles: m.triangles(),
            vertices: m.vertices(),
            bounds: m.near_bounds().map(|(a, b)| (a.to_array(), b.to_array())),
        }
    }
}

/// The diff of two models.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelDiff {
    pub before: Totals,
    pub after: Totals,
    pub changes: Vec<Change>,
}

impl ModelDiff {
    /// No structural change.
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }

    /// The report: a totals line, then one line per change.
    pub fn lines(&self) -> Vec<String> {
        let (a, b) = (&self.before, &self.after);
        let d = |x: usize, y: usize| {
            if x == y {
                format!("{x}")
            } else {
                format!("{x} → {y} ({:+})", y as i64 - x as i64)
            }
        };
        let mut out = vec![format!(
            "nodes {}, materials {}, triangles {}, vertices {}",
            d(a.nodes, b.nodes),
            d(a.materials, b.materials),
            d(a.triangles, b.triangles),
            d(a.vertices, b.vertices)
        )];
        if a.bounds != b.bounds {
            let s = |x: &Option<([f32; 3], [f32; 3])>| {
                x.map_or_else(
                    || "none".into(),
                    |(lo, hi)| format!("{} … {}", v4(&lo), v4(&hi)),
                )
            };
            out.push(format!("~ bounds {} → {}", s(&a.bounds), s(&b.bounds)));
        }
        out.extend(self.changes.iter().map(ToString::to_string));
        out
    }
}

/// Node keys: the name and its occurrence (unnamed or duplicate nodes).
fn keys(m: &Model) -> Vec<(String, usize)> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    m.nodes
        .iter()
        .map(|n| {
            let k = seen.entry(n.name.as_str()).or_default();
            *k += 1;
            (n.name.clone(), *k - 1)
        })
        .collect()
}

fn label((name, nth): &(String, usize)) -> String {
    let name = if name.is_empty() { "(unnamed)" } else { name };
    if *nth == 0 {
        name.to_owned()
    } else {
        format!("{name}#{}", nth + 1)
    }
}

fn mesh_stats(m: &Model, n: &Node) -> MeshStats {
    MeshStats {
        triangles: n.primitives.iter().map(|p| p.triangles()).sum(),
        vertices: n.primitives.iter().map(|p| p.positions.len()).sum(),
        materials: n
            .primitives
            .iter()
            .map(|p| {
                m.materials
                    .get(p.material)
                    .map_or_else(|| format!("#{}", p.material), |x| x.name.clone())
            })
            .collect(),
    }
}

fn close(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() <= TOLERANCE)
}

/// Attributes left out of a comparison (to say what moved).
#[derive(Clone, Copy, PartialEq, Eq)]
struct Skip {
    normals: bool,
    colours: bool,
}

const ALL: Skip = Skip {
    normals: false,
    colours: false,
};

fn same_geometry(a: &Node, b: &Node, skip: Skip) -> bool {
    a.primitives.len() == b.primitives.len()
        && a.primitives
            .iter()
            .zip(&b.primitives)
            .all(|(p, q)| same_primitive(p, q, skip))
}

/// `None` when the meshes draw the same; else what alone explains the
/// change (see [`Change::Mesh`]; empty when the primitives differ).
fn geometry_change(a: &Node, b: &Node) -> Option<Vec<&'static str>> {
    if same_geometry(a, b, ALL) {
        return None;
    }
    if a.primitives.len() != b.primitives.len()
        || a.primitives
            .iter()
            .zip(&b.primitives)
            .any(|(p, q)| p.indices.len() != q.indices.len())
    {
        return Some(Vec::new());
    }
    let only = |normals, colours| same_geometry(a, b, Skip { normals, colours });
    Some(if only(true, false) {
        vec!["normals"]
    } else if only(false, true) {
        vec!["colours"]
    } else if only(true, true) {
        vec!["normals", "colours"]
    } else {
        vec!["positions"]
    })
}

/// One triangle corner: position, normal, colour, UV (missing ones zero).
type Corner = [f32; 12];

fn corner(p: &Primitive, i: u32) -> Option<Corner> {
    let i = i as usize;
    let pos = p.positions.get(i)?;
    let n = p.normals.get(i).copied().unwrap_or_default();
    let c = p.colors.get(i).copied().unwrap_or_default();
    let uv = p.uvs.get(i).copied().unwrap_or_default();
    Some([
        pos[0], pos[1], pos[2], n[0], n[1], n[2], c[0], c[1], c[2], c[3], uv[0], uv[1],
    ])
}

/// A triangle's bucket: its three corner positions on a 1 mm grid, in
/// any order (the attributes and the winding are compared after).
fn bucket(t: &[Corner; 3]) -> [[i64; 3]; 3] {
    let mut k = t
        .each_ref()
        .map(|c| [0, 1, 2].map(|i| (c[i] / 1e-3).round() as i64));
    k.sort_unstable();
    k
}

/// The triangles of a primitive; `None` for an index out of range.
fn triangles(p: &Primitive) -> Option<Vec<[Corner; 3]>> {
    p.indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|t| Some([corner(p, t[0])?, corner(p, t[1])?, corner(p, t[2])?]))
        .collect()
}

/// The same triangle: every corner within the tolerances, starting from
/// any corner, with the same winding.
fn same_triangle(a: &[Corner; 3], b: &[Corner; 3], skip: Skip) -> bool {
    (0..3).any(|r| (0..3).all(|i| same_corner(&a[i], &b[(i + r) % 3], skip)))
}

/// Every triangle of `a` matches its own triangle of `b` (positions on a
/// grid line can fall into neighbouring buckets: then it reports a change,
/// never hides one).
fn same_triangles(a: &[[Corner; 3]], b: &[[Corner; 3]], skip: Skip) -> bool {
    let mut left: BTreeMap<[[i64; 3]; 3], Vec<&[Corner; 3]>> = BTreeMap::new();
    for t in b {
        left.entry(bucket(t)).or_default().push(t);
    }
    a.iter().all(|t| {
        let Some(cands) = left.get_mut(&bucket(t)) else {
            return false;
        };
        match cands.iter().position(|c| same_triangle(t, c, skip)) {
            Some(i) => {
                cands.swap_remove(i);
                true
            }
            None => false,
        }
    })
}

fn same_corner(a: &Corner, b: &Corner, skip: Skip) -> bool {
    a.iter().zip(b).enumerate().all(|(i, (x, y))| {
        (skip.normals && (3..6).contains(&i))
            || (skip.colours && (6..10).contains(&i))
            || (x - y).abs() <= corner_tolerance(i)
    })
}

fn same_primitive(p: &Primitive, q: &Primitive, skip: Skip) -> bool {
    if p.indices.len() != q.indices.len()
        || p.normals.is_empty() != q.normals.is_empty()
        || p.colors.is_empty() != q.colors.is_empty()
        || p.uvs.is_empty() != q.uvs.is_empty()
    {
        return false;
    }
    // The canonical files of one writer: the same arrays.
    if skip == ALL
        && p.indices == q.indices
        && close(p.positions.as_flattened(), q.positions.as_flattened())
        && p.colors.len() == q.colors.len()
        && p.colors
            .as_flattened()
            .iter()
            .zip(q.colors.as_flattened())
            .all(|(x, y)| (x - y).abs() <= COLOR_TOLERANCE)
        && close(p.uvs.as_flattened(), q.uvs.as_flattened())
        && p.normals.len() == q.normals.len()
        && p.normals
            .as_flattened()
            .iter()
            .zip(q.normals.as_flattened())
            .all(|(x, y)| (x - y).abs() <= NORMAL_TOLERANCE)
    {
        return true;
    }
    // Re-indexed, split or merged vertices: the drawn triangles.
    match (triangles(p), triangles(q)) {
        (Some(a), Some(b)) => same_triangles(&a, &b, skip),
        _ => false,
    }
}

/// Flattens JSON into `path → leaf` (`ocw.frame.stack_m`).
fn flatten(prefix: &str, v: &Value, out: &mut BTreeMap<String, Value>) {
    match v {
        Value::Object(o) if !o.is_empty() => {
            for (k, x) in o {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                flatten(&p, x, out);
            }
        }
        Value::Null if prefix.is_empty() => {}
        _ => {
            out.insert(prefix.to_owned(), v.clone());
        }
    }
}

fn vec4(v: Vec3) -> [f32; 4] {
    [v.x, v.y, v.z, 0.0]
}

/// The structural diff of `a` (before) and `b` (after).
pub fn diff(a: &Model, b: &Model) -> ModelDiff {
    let mut changes = Vec::new();
    let (ka, kb) = (keys(a), keys(b));
    let index_b: BTreeMap<&(String, usize), usize> =
        kb.iter().enumerate().map(|(i, k)| (k, i)).collect();
    let index_a: BTreeMap<&(String, usize), usize> =
        ka.iter().enumerate().map(|(i, k)| (k, i)).collect();
    let parent =
        |m: &Model, k: &[(String, usize)], i: usize| m.nodes[i].parent.map(|p| label(&k[p]));
    for (i, k) in ka.iter().enumerate() {
        let Some(&j) = index_b.get(k) else {
            changes.push(Change::NodeRemoved { name: label(k) });
            continue;
        };
        let (na, nb) = (&a.nodes[i], &b.nodes[j]);
        let name = label(k);
        let (pa, pb) = (parent(a, &ka, i), parent(b, &kb, j));
        if pa != pb {
            changes.push(Change::NodeParent {
                name: name.clone(),
                before: pa,
                after: pb,
            });
        }
        for (field, x, y) in [
            ("translation", vec4(na.translation), vec4(nb.translation)),
            ("scale", vec4(na.scale), vec4(nb.scale)),
        ] {
            if !close(&x, &y) {
                changes.push(Change::NodeTransform {
                    name: name.clone(),
                    field,
                    before: x,
                    after: y,
                });
            }
        }
        // q and −q are the same rotation.
        if na.rotation.dot(nb.rotation).abs() < 1.0 - TOLERANCE {
            changes.push(Change::NodeTransform {
                name: name.clone(),
                field: "rotation",
                before: na.rotation.to_array(),
                after: nb.rotation.to_array(),
            });
        }
        let (sa, sb) = (mesh_stats(a, na), mesh_stats(b, nb));
        let change = geometry_change(na, nb);
        let geometry = change.is_some();
        // The vertex count alone is no change (same triangles, see the module).
        if geometry || sa.triangles != sb.triangles || sa.materials != sb.materials {
            changes.push(Change::Mesh {
                node: name.clone(),
                before: sa,
                after: sb,
                geometry,
                what: change.unwrap_or_default(),
            });
        }
        if na.extras != nb.extras {
            changes.push(Change::NodeExtras {
                name,
                before: na.extras.clone(),
                after: nb.extras.clone(),
            });
        }
    }
    for (j, k) in kb.iter().enumerate() {
        if !index_a.contains_key(k) {
            changes.push(Change::NodeAdded {
                name: label(k),
                parent: parent(b, &kb, j),
            });
        }
    }
    for ma in &a.materials {
        match b.materials.iter().find(|m| m.name == ma.name) {
            None => changes.push(Change::MaterialRemoved {
                name: ma.name.clone(),
            }),
            Some(mb) => {
                for (field, x, y) in [
                    ("color", ma.color, mb.color),
                    (
                        "metallic",
                        [ma.metallic, 0.0, 0.0, 0.0],
                        [mb.metallic, 0.0, 0.0, 0.0],
                    ),
                    (
                        "roughness",
                        [ma.roughness, 0.0, 0.0, 0.0],
                        [mb.roughness, 0.0, 0.0, 0.0],
                    ),
                    (
                        "emissive",
                        [ma.emissive[0], ma.emissive[1], ma.emissive[2], 0.0],
                        [mb.emissive[0], mb.emissive[1], mb.emissive[2], 0.0],
                    ),
                ] {
                    if !close(&x, &y) {
                        changes.push(Change::MaterialChanged {
                            name: ma.name.clone(),
                            field,
                            before: x,
                            after: y,
                        });
                    }
                }
            }
        }
    }
    for mb in &b.materials {
        if a.material(&mb.name).is_none() {
            changes.push(Change::MaterialAdded {
                name: mb.name.clone(),
            });
        }
    }
    for (field, x, y) in [
        ("copyright", &a.copyright, &b.copyright),
        ("generator", &a.generator, &b.generator),
    ] {
        if x != y {
            changes.push(Change::Asset {
                field,
                before: x.clone(),
                after: y.clone(),
            });
        }
    }
    let (mut ea, mut eb) = (BTreeMap::new(), BTreeMap::new());
    flatten("", &a.extras, &mut ea);
    flatten("", &b.extras, &mut eb);
    let paths: std::collections::BTreeSet<&String> = ea.keys().chain(eb.keys()).collect();
    for p in paths {
        let (x, y) = (ea.get(p), eb.get(p));
        if x != y {
            changes.push(Change::Extras {
                path: p.clone(),
                before: x.cloned(),
                after: y.cloned(),
            });
        }
    }
    ModelDiff {
        before: Totals::of(a),
        after: Totals::of(b),
        changes,
    }
}

/// The differences of two manifest entries (`None`: not listed).
pub fn diff_entries(a: Option<&Entry>, b: Option<&Entry>) -> Vec<Change> {
    let fields = |e: Option<&Entry>| -> Vec<(&'static str, Option<String>)> {
        vec![
            ("file", e.map(|e| e.file.clone())),
            ("kind", e.map(|e| e.kind.name().to_owned())),
            ("profile", e.and_then(|e| e.profile.clone())),
            ("species", e.and_then(|e| e.species.clone())),
            ("variant", e.and_then(|e| e.variant.clone())),
            ("anchor", e.and_then(|e| e.anchor.clone())),
            ("generator", e.and_then(|e| e.generator.clone())),
            ("license", e.map(|e| e.license.clone())),
            ("source", e.map(|e| e.source.clone())),
            ("author", e.map(|e| e.author.clone())),
        ]
    };
    fields(a)
        .into_iter()
        .zip(fields(b))
        .filter(|((_, x), (_, y))| x != y)
        .map(|((key, before), (_, after))| Change::Manifest {
            key: key.into(),
            before,
            after,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{glb, slots};
    use glam::Quat;

    fn bike() -> Model {
        crate::shipped::bike_model(ocw_core::ProfileId::Road).clone()
    }

    #[test]
    fn a_model_against_itself_is_empty() {
        let m = bike();
        let d = diff(&m, &m);
        assert!(d.is_empty(), "{:#?}", d.changes);
        assert_eq!(d.lines().len(), 1);
        // Also after a write and a read (the canonical bytes).
        let back = glb::read(&glb::write(&m)).unwrap();
        assert!(diff(&m, &back).is_empty());
    }

    #[test]
    fn nodes_meshes_materials_and_extras_are_reported() {
        let a = bike();
        let mut b = a.clone();
        let saddle = b.node("saddle").unwrap();
        b.nodes[saddle].translation.y += 0.02;
        let crank = b.node("crank").unwrap();
        b.nodes[crank].rotation = Quat::from_rotation_x(0.5) * b.nodes[crank].rotation;
        b.nodes[saddle].name = "saddle_new".into();
        let lamp = b.add(Node::new("anchor_lamp", Some(0), Vec3::ZERO));
        b.nodes[lamp].extras = serde_json::json!({"note": "test"});
        let i = b.material("frame").unwrap();
        b.materials[i].color = [1.0, 0.0, 0.0, 1.0];
        b.materials.push(slots::material("decal"));
        // A mesh with one triangle less.
        let wheel = b.node("wheel_front").unwrap();
        let p = &mut b.nodes[wheel].primitives[0];
        p.indices.truncate(p.indices.len() - 3);
        b.extras["ocw"]["profile"] = serde_json::json!("tt");
        let d = diff(&a, &b);
        let lines = d.lines().join("\n");
        assert!(lines.contains("- node saddle\n"), "{lines}");
        assert!(lines.contains("+ node saddle_new (under "), "{lines}");
        assert!(lines.contains("+ node anchor_lamp"), "{lines}");
        assert!(lines.contains("~ node crank: rotation"), "{lines}");
        assert!(lines.contains("~ mesh wheel_front:"), "{lines}");
        assert!(lines.contains("~ material frame: color"), "{lines}");
        assert!(
            lines.contains("~ extras ocw.profile: \"road\" → \"tt\""),
            "{lines}"
        );
        assert!(lines.starts_with("nodes "), "{lines}");
        assert!(lines.contains("(-1)"), "{lines}");
        // The same counts but moved geometry.
        let mut c = a.clone();
        let w = c.node("wheel_rear").unwrap();
        c.nodes[w].primitives[0].positions[0][0] += 0.01;
        let d = diff(&a, &c);
        assert_eq!(d.changes.len(), 1);
        assert_eq!(
            d.changes[0].to_string(),
            "~ mesh wheel_rear: geometry changed (same counts): positions"
        );
        // Machine-readable.
        let j = serde_json::to_value(&d).unwrap();
        assert_eq!(j["changes"][0]["change"], "mesh");
        assert_eq!(j["changes"][0]["geometry"], true);
    }

    /// What Blender's exporter does to a mesh: the triangles in another
    /// order, each from another corner, the vertices re-indexed and some
    /// split (duplicated), normals a little off – the same mesh.
    #[test]
    fn reindexed_and_split_vertices_are_the_same_mesh() {
        let a = bike();
        let w = a.node("wheel_front").unwrap();
        let p = &a.nodes[w].primitives[0];
        let mut q = Primitive {
            material: p.material,
            ..Default::default()
        };
        let tris: Vec<&[u32; 3]> = p.indices.as_chunks::<3>().0.iter().rev().collect();
        for (k, t) in tris.iter().enumerate() {
            // Every triangle gets its own three vertices (split), turned.
            let turn = k % 3;
            for c in 0..3 {
                let i = t[(c + turn) % 3] as usize;
                q.indices.push(q.positions.len() as u32);
                q.positions.push(p.positions[i]);
                let mut n = p.normals[i];
                n[0] += 5e-4;
                q.normals.push(n);
                if !p.colors.is_empty() {
                    q.colors.push(p.colors[i]);
                }
                if !p.uvs.is_empty() {
                    q.uvs.push(p.uvs[i]);
                }
            }
        }
        // And a vertex no triangle uses (Blender leaves some, too).
        q.positions.push([9.0; 3]);
        q.normals.push([0.0, 1.0, 0.0]);
        if !q.colors.is_empty() {
            q.colors.push([1.0; 4]);
        }
        if !q.uvs.is_empty() {
            q.uvs.push([0.0; 2]);
        }
        let mut b = a.clone();
        b.nodes[w].primitives[0] = q.clone();
        let d = diff(&a, &b);
        assert!(d.is_empty(), "{:#?}", d.changes);
        assert_ne!(d.before.vertices, d.after.vertices, "split vertices");
        // A vertex moved by 1 mm, a flipped triangle, a normal 1° off:
        // changes (on a triangle with three distinct corners: flipping a
        // degenerate one changes nothing).
        let k = q
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .position(|t| {
                let [x, y, z] = [0, 1, 2].map(|i| Vec3::from(q.positions[t[i] as usize]));
                (y - x).cross(z - x).length() > 1e-6
            })
            .expect("a triangle with an area")
            * 3;
        for broken in 0..3 {
            let mut r = q.clone();
            match broken {
                0 => r.positions[r.indices[k + 1] as usize][1] += 1e-3,
                1 => r.indices.swap(k, k + 1),
                _ => r.normals[r.indices[k + 2] as usize][2] += 0.02,
            }
            let mut c = a.clone();
            c.nodes[w].primitives[0] = r;
            let d = diff(&a, &c);
            assert_eq!(d.changes.len(), 1, "case {broken}: {:#?}", d.changes);
            let want = ["positions", "positions", "normals"][broken];
            assert!(
                matches!(&d.changes[0], Change::Mesh { node, geometry: true, what, .. }
                    if node == "wheel_front" && what == &[want]),
                "case {broken}: {:#?}",
                d.changes
            );
        }
        // An index out of range is a change, not a panic.
        let mut r = q;
        r.indices[0] = 1_000_000;
        let mut c = a.clone();
        c.nodes[w].primitives[0] = r;
        assert!(!diff(&a, &c).is_empty());
    }

    #[test]
    fn manifest_entries_are_compared() {
        let a = crate::export::entry("props/post-square.glb", crate::kinds::Kind::Prop, "x");
        let mut b = a.clone();
        b.license = "CC0-1.0".into();
        let d = diff_entries(Some(&a), Some(&b));
        assert_eq!(d.len(), 1);
        assert!(d[0].to_string().starts_with("~ manifest license:"));
        assert!(diff_entries(Some(&a), Some(&a)).is_empty());
        assert_eq!(diff_entries(None, Some(&a)).len(), 6);
    }
}

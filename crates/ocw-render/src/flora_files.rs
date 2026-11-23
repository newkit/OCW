//! The plants as model files (spec §14 Phase 15, M1; Q8): every species
//! of [`crate::flora::plant`] exported as at least three shape variants
//! (`-a`, `-b`, `-c`; more for the main forest trees), each with its near
//! model and its `lod1`, into `artwork/3dmodels/flora/`
//! (`cargo run -p ocw-render --example export-flora [-- --check]`).
//!
//! A file is the generator's plant at its stem base, unscaled and unturned,
//! with the summer leaves. Its materials are paint slots: the crown
//! (`leaf`, or `needle` for conifers) with the summer crown colour and the
//! shading as vertex colours, `bark` in the species' bark colour, `dry`
//! parts (dead fronds, stalks) in their vertex colours. Drawing a file
//! ([`draw`]) places, scales and turns it per plant and tints the crown by
//! the plant's own shade and the season, as the generator does.
//!
//! The switch (`OCW_FLORA=files`, [`files_enabled`]): the renderer draws
//! the plants from the embedded files instead of the generator wherever
//! the season leaves the shape alone (full leaves, no summer drought);
//! bare, thin and dry crowns still come from the generator – their shapes
//! are not in the files yet (an open point of Phase 15).

use bevy::prelude::*;
use ocw_assets::manifest::Entry;
use ocw_assets::model::{Material, Model, Node, Primitive};
use ocw_sky::season::LeafType;
use ocw_world::flora::{Habit, Species};

use crate::flora::{
    Detail, PlantAt, PlantLook, bark_colour, crown_colour, generate as plant, leaf_type,
};
use crate::package::MeshArrays;

/// How many shape variants a species gets (Q8: three, more for the main
/// forest trees of the regions, seen most and up close).
pub fn variant_count(sp: Species) -> usize {
    match sp {
        Species::NorwaySpruce | Species::SilverFir | Species::Beech | Species::CanaryPine => 5,
        Species::ScotsPine | Species::Larch | Species::Oak | Species::Laurel => 4,
        _ => 3,
    }
}

/// The `pick` (the generator's own variety, 0 – 1) of variant `i` of `n`.
pub fn variant_pick(i: usize, n: usize) -> f32 {
    (i as f32 + 0.5) / n as f32
}

/// The variant a plant's `pick` falls in.
pub fn variant_of(pick: f32, n: usize) -> usize {
    ((pick.clamp(0.0, 1.0) * n as f32) as usize).min(n - 1)
}

/// The file name's function and shape per species (15H).
pub fn stem(sp: Species) -> &'static str {
    use Species::*;
    match sp {
        Unknown => "tree-broadleaf-generic",
        NorwaySpruce => "tree-spruce-conical",
        SilverFir => "tree-fir-conical",
        DouglasFir => "tree-douglas-conical",
        ScotsPine => "tree-pine-scots",
        MountainPine => "shrub-pine-mountain",
        Larch => "tree-larch-conical",
        Beech => "tree-beech-round",
        SycamoreMaple => "tree-maple-round",
        Ash => "tree-ash-oval",
        Oak => "tree-oak-broad",
        Alder => "tree-alder-oval",
        Willow => "tree-willow-round",
        GreenAlder => "shrub-alder-mountain",
        Birch => "tree-birch-slender",
        Lime => "tree-lime-broad",
        FruitTree => "tree-fruit-round",
        Chestnut => "tree-chestnut-broad",
        CanaryPine => "tree-pine-canary",
        Laurel => "tree-laurel-round",
        FayaHeath => "shrub-faya-heath",
        DatePalm => "palm-date-canary",
        SlenderPalm => "palm-slender-tall",
        DragonTree => "tree-dragon-umbrella",
        Broom => "shrub-broom-teide",
        Tabaiba => "shrub-tabaiba-cushion",
        Cardon => "shrub-cardon-candelabra",
        PricklyPear => "cactus-pear-paddle",
        Agave => "succulent-agave-rosette",
        Eucalyptus => "tree-eucalyptus-tall",
        Banana => "plant-banana-fronds",
        SubtropicalFruit => "tree-fruit-subtropical",
    }
}

/// The file of variant `i` of `sp`.
pub fn file(sp: Species, i: usize) -> String {
    format!("flora/{}-{}.glb", stem(sp), (b'a' + i as u8) as char)
}

/// The model id of variant `i` of `sp`.
pub fn id(sp: Species, i: usize) -> String {
    file(sp, i).trim_end_matches(".glb").to_owned()
}

/// The crown's slot of a species.
pub fn crown_slot(sp: Species) -> &'static str {
    match sp.habit() {
        Habit::Conifer | Habit::Larch => "needle",
        _ => "leaf",
    }
}

const SHADES: (f32, f32) = (0.2, 0.8);
const HUE: f32 = 0.5;
/// The shade whose crown colour a file's crown material carries.
pub const FILE_SHADE: f32 = 0.5;

fn canonical(pick: f32, shade: f32) -> PlantAt {
    PlantAt {
        base: Vec3::ZERO,
        scale: 1.0,
        turn: 0.0,
        shade,
        hue: HUE,
        pick,
    }
}

fn rgb_close(a: [f32; 4], b: [f32; 3], tol: f32) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() <= tol)
}

/// One level of detail split by slot: (material, primitive).
fn split(sp: Species, pick: f32, detail: Detail) -> Result<Vec<(Material, Primitive)>, String> {
    let look = PlantLook::SUMMER;
    let mut a = MeshArrays::default();
    let mut b = MeshArrays::default();
    plant(&mut a, sp, &canonical(pick, SHADES.0), &look, detail);
    plant(&mut b, sp, &canonical(pick, SHADES.1), &look, detail);
    let n = a.positions.len();
    if b.positions != a.positions || b.indices != a.indices {
        return Err(format!("{sp:?}: the shade changes the shape"));
    }
    if a.colors.len() != n || a.normals.len() != n || a.uvs.len() != n {
        return Err(format!("{sp:?}: incomplete vertex arrays"));
    }
    let crown_a = crown_colour(sp, &look, SHADES.0, HUE);
    let crown_b = crown_colour(sp, &look, SHADES.1, HUE);
    let crown_file = crown_colour(sp, &look, FILE_SHADE, HUE);
    let bark = bark_colour(sp);
    // Per vertex: (slot, vertex colour).
    let mut tag: Vec<(usize, [f32; 4])> = Vec::with_capacity(n);
    for i in 0..n {
        let (ca, cb) = (a.colors[i], b.colors[i]);
        let constant = (0..4).all(|k| (ca[k] - cb[k]).abs() < 1e-6);
        if constant {
            if rgb_close(ca, bark, 1e-6) {
                tag.push((1, [1.0; 4]));
            } else {
                tag.push((2, ca));
            }
            continue;
        }
        // A crown vertex: the crown colour times a shading factor.
        let ch = (0..3)
            .max_by(|&x, &y| crown_a[x].total_cmp(&crown_a[y]))
            .unwrap_or(0);
        let k = ca[ch] / crown_a[ch].max(1e-6);
        let fits = (0..3).all(|c| {
            (ca[c] - crown_a[c] * k).abs() < 2e-5 && (cb[c] - crown_b[c] * k).abs() < 2e-5
        });
        if !fits {
            return Err(format!(
                "{sp:?}: vertex {i} colour {ca:?} is neither the crown's shade nor fixed"
            ));
        }
        tag.push((0, [k, k, k, 1.0]));
    }
    let materials = [
        Material {
            name: crown_slot(sp).into(),
            color: [crown_file[0], crown_file[1], crown_file[2], 1.0],
            metallic: 0.0,
            roughness: 0.85,
            emissive: [0.0; 3],
        },
        Material {
            name: "bark".into(),
            color: [bark[0], bark[1], bark[2], 1.0],
            metallic: 0.0,
            roughness: 0.9,
            emissive: [0.0; 3],
        },
        Material {
            name: "dry".into(),
            color: [1.0; 4],
            metallic: 0.0,
            roughness: 0.9,
            emissive: [0.0; 3],
        },
    ];
    let mut out = Vec::new();
    for (slot, mat) in materials.into_iter().enumerate() {
        // The slot's triangles, with their vertices renumbered.
        let mut map = vec![u32::MAX; n];
        let mut p = Primitive::default();
        for tri in a.indices.chunks(3) {
            let s = tag[tri[0] as usize].0;
            if tri.iter().any(|&v| tag[v as usize].0 != s) {
                return Err(format!("{sp:?}: a triangle across two slots"));
            }
            if s != slot {
                continue;
            }
            for &v in tri {
                let v = v as usize;
                if map[v] == u32::MAX {
                    map[v] = p.positions.len() as u32;
                    p.positions.push(a.positions[v]);
                    p.normals.push(a.normals[v]);
                    p.uvs.push(a.uvs[v]);
                    p.colors.push(tag[v].1);
                }
                p.indices.push(map[v]);
            }
        }
        if !p.indices.is_empty() {
            out.push((mat, p));
        }
    }
    Ok(out)
}

/// The model of variant `i` of `sp` (near on the root `plant`, far on
/// `lod1`), stamped with its manifest entry.
pub fn model(sp: Species, i: usize) -> Result<(Entry, Model), String> {
    let n = variant_count(sp);
    let pick = variant_pick(i, n);
    let mut m = Model::default();
    let root = m.add(Node::new("plant", None, Vec3::ZERO));
    let lod = m.add(Node::new("lod1", Some(root), Vec3::ZERO));
    for (node, detail) in [(root, Detail::Near), (lod, Detail::Far)] {
        for (mat, mut p) in split(sp, pick, detail)? {
            p.material = m.material_or_add(mat);
            m.nodes[node].primitives.push(p);
        }
    }
    let mut e = ocw_assets::export::entry(
        &file(sp, i),
        ocw_assets::kinds::Kind::Plant,
        &format!("ocw_render::flora::plant(Species::{sp:?}, pick = {pick})"),
    );
    e.species = Some(format!("{sp:?}"));
    e.variant = Some(((b'a' + i as u8) as char).to_string());
    ocw_assets::export::stamp(
        &mut m,
        &e,
        serde_json::json!({
            "species": format!("{sp:?}"),
            "variant": e.variant,
            "pick": pick,
            "height_m": sp.height_m(),
        }),
    );
    // The crown shading reaches 1.1 × the paint colour; Blender keeps
    // vertex colours ≤ 1, so the file keeps them ≤ 1 and says by how much
    // to brighten them (`extras.ocw.shade_scale`, spec 15G Q11 (a)).
    m.fit_shading();
    Ok((e, m))
}

/// Every plant file: (entry, model).
pub fn models() -> Result<Vec<(Entry, Model)>, String> {
    let mut out = Vec::new();
    for sp in Species::ALL {
        for i in 0..variant_count(sp) {
            out.push(model(sp, i)?);
        }
    }
    Ok(out)
}

/// Whether a plant with `look` keeps the shape of its file (full leaves,
/// no drought): only then it can be drawn from the file.
pub fn shape_in_file(sp: Species, look: &PlantLook) -> bool {
    let full = leaf_type(sp) == LeafType::Evergreen || look.leaves >= 0.999;
    full && look.dry <= 0.0
}

/// Draws the file `model` of `sp` at `at` with `look` into `m` (`detail`:
/// the root's primitives or `lod1`'s).
pub fn draw(
    m: &mut MeshArrays,
    model: &Model,
    sp: Species,
    at: &PlantAt,
    look: &PlantLook,
    detail: Detail,
) {
    let lod = model.node("lod1");
    let node = match detail {
        Detail::Near => 0,
        Detail::Far => lod.unwrap_or(0),
    };
    let crown = crown_colour(sp, look, at.shade, at.hue);
    // The generator turns its rings from +x towards +z.
    let rot = Quat::from_rotation_y(-at.turn);
    let s = at.scale;
    // The file's colours are kept ≤ 1; drawn × its shade scale (Q11 (a)).
    let shade = model.shade_scale();
    for p in &model.nodes[node].primitives {
        let mat = &model.materials[p.material];
        let tint = if mat.name == crown_slot(sp) {
            crown
        } else {
            [mat.color[0], mat.color[1], mat.color[2]]
        };
        let b = m.positions.len() as u32;
        for v in 0..p.positions.len() {
            let pos = at.base + rot * (Vec3::from(p.positions[v]) * s);
            m.positions.push(pos.to_array());
            m.normals.push((rot * Vec3::from(p.normals[v])).to_array());
            let uv = p.uvs.get(v).copied().unwrap_or([pos.y - at.base.y, 0.0]);
            m.uvs.push([uv[0] * s, uv[1]]);
            let c = p
                .colors
                .get(v)
                .map_or([1.0; 4], |c| ocw_assets::model::shaded(*c, shade));
            m.colors
                .push([tint[0] * c[0], tint[1] * c[1], tint[2] * c[2], 1.0]);
        }
        m.indices.extend(p.indices.iter().map(|i| b + i));
    }
}

/// Whether the plants are drawn from the files (`OCW_FLORA=files`; native).
pub fn files_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        #[cfg(not(target_arch = "wasm32"))]
        {
            std::env::var("OCW_FLORA").is_ok_and(|v| v == "files")
        }
        #[cfg(target_arch = "wasm32")]
        {
            false
        }
    })
}

/// The file model of the variant `at.pick` falls in.
pub fn file_model(sp: Species, pick: f32) -> Option<std::sync::Arc<Model>> {
    static CACHE: std::sync::OnceLock<Vec<Vec<Option<std::sync::Arc<Model>>>>> =
        std::sync::OnceLock::new();
    let all = CACHE.get_or_init(|| {
        let lib = ocw_assets::library::library();
        Species::ALL
            .iter()
            .map(|sp| {
                (0..variant_count(*sp))
                    .map(|i| lib.get(&id(*sp, i)).ok())
                    .collect()
            })
            .collect()
    });
    let list = &all[Species::ALL.iter().position(|s| *s == sp)?];
    list[variant_of(pick, list.len())].clone()
}

/// Draws a plant from its file when the switch is on and the season
/// keeps the file's shape; returns whether it did.
pub fn try_draw(
    m: &mut MeshArrays,
    sp: Species,
    at: &PlantAt,
    look: &PlantLook,
    detail: Detail,
) -> bool {
    if !files_enabled() || !shape_in_file(sp, look) {
        return false;
    }
    let Some(model) = file_model(sp, at.pick) else {
        return false;
    };
    draw(m, &model, sp, at, look, detail);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two meshes are the same set of triangles (the files group them
    /// by slot, so the order differs): every per-triangle quantity –
    /// centroid, area, colour, sway height, normal – sorted, element by
    /// element within the file's precision (a matching within a tolerance
    /// keeps the sorted sequences within it).
    fn close(a: &MeshArrays, b: &MeshArrays, what: &str, shape_tol: f32) {
        assert_eq!(a.indices.len(), b.indices.len(), "{what}: indices");
        assert_eq!(a.positions.len(), b.positions.len(), "{what}: vertices");
        let stats = |m: &MeshArrays| -> Vec<Vec<f32>> {
            let mut out = vec![Vec::new(); 11];
            for t in m.indices.chunks(3) {
                let p = [0, 1, 2].map(|k| Vec3::from(m.positions[t[k] as usize]));
                let c = (p[0] + p[1] + p[2]) / 3.0;
                let area = (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5;
                let col = m.colors[t[0] as usize];
                let n = Vec3::from(m.normals[t[0] as usize]);
                let vals = [
                    c.x,
                    c.y,
                    c.z,
                    area,
                    col[0],
                    col[1],
                    col[2],
                    m.uvs[t[0] as usize][0],
                    n.x,
                    n.y,
                    n.z,
                ];
                for (o, v) in out.iter_mut().zip(vals) {
                    o.push(v);
                }
            }
            for o in &mut out {
                o.sort_by(f32::total_cmp);
            }
            out
        };
        let names = [
            "centroid x",
            "centroid y",
            "centroid z",
            "area",
            "red",
            "green",
            "blue",
            "sway",
            "normal x",
            "normal y",
            "normal z",
        ];
        let st = shape_tol;
        let tol = [st, st, st, st, 2e-3, 2e-3, 2e-3, st, 2e-2, 2e-2, 2e-2];
        for (k, (x, y)) in stats(a).iter().zip(stats(b)).enumerate() {
            let worst = x
                .iter()
                .zip(&y)
                .map(|(u, v)| (u - v).abs())
                .fold(0.0f32, f32::max);
            assert!(worst <= tol[k], "{what}: {} differs by {worst}", names[k]);
        }
    }

    /// Every species has at least three variants (Q8), every file passes
    /// the contract, and the export is deterministic.
    #[test]
    fn every_species_exports_its_variants() {
        let all = models().unwrap_or_else(|e| panic!("{e}"));
        for sp in Species::ALL {
            let n = all
                .iter()
                .filter(|(e, _)| e.species.as_deref() == Some(&format!("{sp:?}")))
                .count();
            assert!(n >= 3, "{sp:?}: {n} variants");
        }
        let files: Vec<(String, Vec<u8>)> = all
            .iter()
            .map(|(e, m)| (e.file.clone(), ocw_assets::glb::write(m)))
            .collect();
        let entries: Vec<Entry> = all.iter().map(|(e, _)| e.clone()).collect();
        let text = ocw_assets::export::manifest_text(&entries);
        let checked =
            ocw_assets::manifest::validate(&text, &files).unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(checked.len(), all.len());
        let again = models().unwrap();
        for ((_, a), (_, b)) in all.iter().zip(&again) {
            assert_eq!(ocw_assets::glb::write(a), ocw_assets::glb::write(b));
        }
    }

    /// A file drawn at a plant's place is the generator's mesh for that
    /// variant – any shade, scale and foot (turn 0: the generator's turn
    /// is not a rotation of the whole plant for every species) – and the
    /// shipped files are the export: the same primitives, triangles and
    /// slots, the shapes within a few centimetres (the generator's variety
    /// hash amplifies the last bits of `sin`, which differ between C
    /// libraries; byte for byte is `export-flora --check` on marge, where
    /// they were exported).
    #[test]
    fn plants_from_files_are_the_generators_plants() {
        let lib = ocw_assets::library::ModelLibrary::embedded();
        for (e, model) in models().unwrap() {
            let shipped = lib
                .get(e.id())
                .unwrap_or_else(|err| panic!("{err}: run the flora export"));
            assert_eq!(shipped.nodes.len(), model.nodes.len(), "{}", e.file);
            assert_eq!(shipped.materials.len(), model.materials.len(), "{}", e.file);
            for (a, b) in shipped.nodes.iter().zip(&model.nodes) {
                assert_eq!(a.primitives.len(), b.primitives.len(), "{}", e.file);
                for (p, q) in a.primitives.iter().zip(&b.primitives) {
                    assert_eq!(p.indices, q.indices, "{} is stale: export-flora", e.file);
                    assert_eq!(
                        shipped.materials[p.material].name,
                        model.materials[q.material].name
                    );
                    let off = p
                        .positions
                        .iter()
                        .zip(&q.positions)
                        .map(|(u, v)| Vec3::from(*u).distance(Vec3::from(*v)))
                        .fold(0.0f32, f32::max);
                    assert!(off < 0.05, "{} is stale: export-flora ({off} m)", e.file);
                }
            }
            let sp = *Species::ALL
                .iter()
                .find(|s| Some(format!("{s:?}")) == e.species)
                .unwrap();
            let i = (e.variant.as_ref().unwrap().as_bytes()[0] - b'a') as usize;
            let pick = variant_pick(i, variant_count(sp));
            // Through the file format, as the app reads it.
            let file = ocw_assets::glb::read(&ocw_assets::glb::write(&model)).unwrap();
            for at in [
                PlantAt {
                    base: Vec3::ZERO,
                    scale: 1.0,
                    turn: 0.0,
                    shade: 0.37,
                    hue: HUE,
                    pick,
                },
                PlantAt {
                    base: Vec3::new(5.0, 2.0, -3.0),
                    scale: 1.3,
                    turn: 0.0,
                    shade: 0.9,
                    hue: HUE,
                    pick,
                },
            ] {
                for detail in [Detail::Near, Detail::Far] {
                    let mut g = MeshArrays::default();
                    plant(&mut g, sp, &at, &PlantLook::SUMMER, detail);
                    let mut f = MeshArrays::default();
                    draw(&mut f, &file, sp, &at, &PlantLook::SUMMER, detail);
                    // Unscaled exact; scaled within 6 cm: the generator does
                    // not scale every part with the plant (the agave's
                    // leaf thickness), the files scale uniformly.
                    let tol = if at.scale == 1.0 { 2e-3 } else { 0.06 };
                    close(&g, &f, &format!("{} {detail:?} {at:?}", e.file), tol);
                }
            }
        }
    }

    /// Spec 15G Q11 (a), what Blender keeps: no vertex colour above 1 in
    /// any plant file – the export and the shipped files – with the shading
    /// in `extras.ocw.shade_scale` where it reaches above the paint colour,
    /// and drawn × it the crown is as bright as the generator's (the
    /// sunlit leaves up to 10 % above the paint colour, as before).
    #[test]
    fn no_plant_file_has_a_vertex_colour_above_one() {
        let lib = ocw_assets::library::ModelLibrary::embedded();
        let mut scaled = 0;
        for (e, model) in models().unwrap() {
            let shipped = lib.get(e.id()).unwrap();
            for m in [&model, &*shipped] {
                let mut max = 0.0f32;
                for p in m.nodes.iter().flat_map(|n| &n.primitives) {
                    for c in &p.colors {
                        assert!(c.iter().all(|v| *v <= 1.0), "{}: {c:?}", e.file);
                        max = max.max(c[0].max(c[1]).max(c[2]));
                    }
                }
                let k = m.shade_scale();
                assert!(
                    (ocw_assets::model::SHADE_SCALE_RANGE.0..=1.1).contains(&k),
                    "{}: {k}",
                    e.file
                );
                if k > 1.0 {
                    // The brightest vertex fills the range (no headroom
                    // left beyond the 1/1000 rounding).
                    assert!(max > 1.0 - 2e-3, "{}: {max} × {k}", e.file);
                }
                let findings = ocw_assets::check::check_model(&e, m, 1000);
                assert!(
                    findings
                        .iter()
                        .all(|f| f.code != "mesh.color-above-one" && f.code != "extras.shade-scale"),
                    "{}: {findings:#?}",
                    e.file
                );
            }
            // The generator's maths may differ in the last bit between CPUs
            // (aarch64 fuses multiply-adds), which can move the rounded-up
            // scale by one step of 1/1000; the shipped file is the truth.
            assert!(
                (model.shade_scale() - shipped.shade_scale()).abs() <= 1.001e-3,
                "{}: {} vs {}",
                e.file,
                model.shade_scale(),
                shipped.shade_scale()
            );
            scaled += usize::from(model.shade_scale() > 1.0);
        }
        // The 63 files whose shading went above 1 (15M).
        assert_eq!(scaled, 63);
    }

    /// Spec 15G Q11 (a): every leaf triangle (the crown's `leaf` /
    /// `needle`, the `dry` fronds) has its normals on its front side and an
    /// area – both faces of a leaf are their own triangles with their own
    /// normals – in the export and in the shipped files; the bark keeps its
    /// own (the caps' rims are at right angles, which Blender keeps).
    #[test]
    fn leaf_normals_are_on_the_triangles_front() {
        let lib = ocw_assets::library::ModelLibrary::embedded();
        for (e, model) in models().unwrap() {
            let shipped = lib.get(e.id()).unwrap();
            for m in [&model, &*shipped] {
                for (node, p) in m
                    .nodes
                    .iter()
                    .flat_map(|n| n.primitives.iter().map(move |p| (&n.name, p)))
                {
                    let slot = m.materials[p.material].name.as_str();
                    if slot == "bark" {
                        continue;
                    }
                    for t in p.indices.chunks(3) {
                        let v = [0, 1, 2].map(|k| Vec3::from(p.positions[t[k] as usize]));
                        let face = (v[1] - v[0]).cross(v[2] - v[0]);
                        assert!(
                            face.length() > 1e-6,
                            "{} {node}/{slot}: a triangle without area at {v:?}",
                            e.file
                        );
                        let face = face.normalize();
                        for &k in t {
                            let n = Vec3::from(p.normals[k as usize]);
                            assert!(
                                n.dot(face) > 0.05,
                                "{} {node}/{slot}: normal {n} behind the face {face} at {:?}",
                                e.file,
                                p.positions[k as usize]
                            );
                        }
                    }
                }
            }
        }
    }

    /// Every plant's lowest point is its stem base (15B, Phase 15 Q10 (a)):
    /// near and far, in the export and the shipped files, within the
    /// validator's 5 cm – the Teide broom (0.67 m below before), the
    /// prickly pear (0.10 m below), the agave (0.10 m above) and the
    /// mountain alder (0.05 m above) included.
    #[test]
    fn every_plant_stands_on_its_stem_base() {
        let lib = ocw_assets::library::ModelLibrary::embedded();
        let tol = ocw_assets::check::GROUND_TOLERANCE_M;
        for (e, model) in models().unwrap() {
            let shipped = lib.get(e.id()).unwrap();
            for (what, m) in [("export", &model), ("file", &*shipped)] {
                let (lo, _) = m.near_bounds().unwrap();
                assert!(
                    lo.y.abs() <= tol,
                    "{} {what}: lowest point {}",
                    e.file,
                    lo.y
                );
                let Some(far) = m.node("lod1") else { continue };
                let low = (0..m.nodes.len())
                    .filter(|&i| m.is_below(i, far))
                    .flat_map(|i| {
                        let w = m.world(i);
                        m.nodes[i].primitives.iter().flat_map(move |p| {
                            p.positions
                                .iter()
                                .map(move |q| w.transform_point3(Vec3::from(*q)).y)
                        })
                    })
                    .fold(f32::MAX, f32::min);
                assert!(
                    low.abs() <= tol,
                    "{} {what}: lod1 lowest point {low}",
                    e.file
                );
            }
        }
    }

    /// The triangle budgets per level (15B: a plant 2,000 in all, 200 in
    /// `lod1`) still hold with both faces of every leaf.
    #[test]
    fn plants_stay_within_their_budget() {
        let (near, far) = ocw_assets::kinds::Kind::Plant.budget();
        for (e, m) in models().unwrap() {
            assert!(m.triangles() <= near, "{}: {}", e.file, m.triangles());
            let lod = m.triangles_below(m.node("lod1"));
            assert!(lod <= far.unwrap(), "{}: lod1 {lod}", e.file);
        }
    }

    #[test]
    fn variants_cover_the_picks() {
        for n in [3, 4, 5] {
            for i in 0..n {
                assert_eq!(variant_of(variant_pick(i, n), n), i);
            }
            assert_eq!(variant_of(0.0, n), 0);
            assert_eq!(variant_of(1.0, n), n - 1);
        }
        let bare = PlantLook {
            leaves: 0.0,
            ..PlantLook::SUMMER
        };
        assert!(shape_in_file(Species::Beech, &PlantLook::SUMMER));
        assert!(!shape_in_file(Species::Beech, &bare));
        assert!(shape_in_file(Species::NorwaySpruce, &bare));
    }
}

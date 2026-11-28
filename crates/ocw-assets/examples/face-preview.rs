//! Pictures of the generated rider's head without the app (spec 14 Phase
//! 4, 4N *Faces*): a small software renderer for shaping the face – the
//! near model's head with a look's hair, facial hair and glasses, from
//! the front, three-quarter and the side, in the look's colours, with the
//! vertex colours used as `ocw-render`'s gear shader uses them (a shade
//! on skin, eyes and lips; the hair's density on brows and beards).
//!
//! `cargo run -p ocw-assets --example face-preview -- OUT_DIR` writes
//! `face-<n>-<name>.png` per look and prints the triangles of the face's
//! nodes. Made-up looks only.

use glam::Vec3;
use ocw_assets::model::Model;
use ocw_assets::pose::nodes::HEAD;
use ocw_assets::rider::{self, Detail};
use ocw_core::character::{Age, BEARDS, EYES, HAIR_STYLES, MOUSTACHES, hair_parse, list_parse};
use ocw_core::{BaseBody, Character, Rgb};

const SIZE: usize = 520;
/// Half the height of the picture (m).
const HALF: f32 = 0.165;

fn linear(c: Rgb) -> Vec3 {
    let f = |v: u8| {
        let s = f32::from(v) / 255.0;
        if s <= 0.04045 { s / 12.92 } else { ((s + 0.055) / 1.055).powf(2.4) }
    };
    let [r, g, b] = c.channels();
    Vec3::new(f(r), f(g), f(b))
}

fn srgb(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0 + 0.5) as u8
}

fn hash(p: Vec3) -> f32 {
    let q = (p * 1400.0).floor();
    let n = (q.x * 127.1 + q.y * 311.7 + q.z * 74.7).sin() * 43_758.547;
    n - n.floor()
}

struct Tri {
    p: [Vec3; 3],
    n: [Vec3; 3],
    c: [f32; 3],
    paint: Vec3,
    hair: bool,
    glossy: bool,
}

#[allow(clippy::too_many_arguments)] // one made-up look per line
fn look(
    name: &'static str,
    age: Age,
    skin: u8,
    hair: &str,
    hair_colour: u32,
    moustache: &str,
    beard: &str,
    eyes: &str,
    glasses: u8,
    helmet: bool,
) -> (&'static str, Character, bool) {
    let c = Character {
        base: BaseBody::A,
        age,
        skin,
        hair: hair_parse(hair).unwrap_or_else(|| panic!("hair {hair}: {HAIR_STYLES:?}")),
        hair_colour: Rgb(hair_colour),
        moustache: list_parse(&MOUSTACHES, moustache).expect("moustache"),
        beard: list_parse(&BEARDS, beard).expect("beard"),
        eyes: list_parse(&EYES, eyes).expect("eyes"),
        glasses,
        ..Default::default()
    };
    (name, c, helmet)
}

fn triangles(m: &Model, c: &Character, helmet: bool) -> Vec<Tri> {
    let head = m.node(HEAD).expect("head");
    let variants: &[&str] = if helmet { &[] } else { &[rider::HELMET_OFF] };
    let shown = rider::visible(c, variants);
    let optional = rider::optional();
    let skin = linear(c.skin_colour());
    let mut out = Vec::new();
    for (i, node) in m.nodes.iter().enumerate() {
        if !(i == head || m.is_below(i, head)) {
            continue;
        }
        // An optional node, or one below it, shows only with the look.
        let hidden = (0..m.nodes.len()).any(|j| {
            (j == i || m.is_below(i, j))
                && optional.contains(&m.nodes[j].name.as_str())
                && !shown.contains(&m.nodes[j].name.as_str())
        });
        if hidden {
            continue;
        }
        let w = m.world(i);
        for p in &node.primitives {
            let mat = &m.materials[p.material];
            let paint = match mat.name.as_str() {
                "skin" => skin,
                "hair" => linear(c.hair_shown()),
                "facial_hair" => linear(c.facial_hair_shown()),
                "skin_line" => linear(c.skin_line_colour()),
                "lips" => linear(c.lip_colour()),
                "iris" => linear(c.eye_colour()),
                _ => Vec3::new(mat.color[0], mat.color[1], mat.color[2]),
            };
            let hair = matches!(mat.name.as_str(), "hair" | "facial_hair");
            let glossy = matches!(mat.name.as_str(), "iris" | "eye_white" | "lens" | "eye_glint");
            for t in p.indices.chunks(3) {
                let v = |k: usize| t[k] as usize;
                out.push(Tri {
                    p: [0, 1, 2].map(|k| w.transform_point3(Vec3::from(p.positions[v(k)]))),
                    n: [0, 1, 2]
                        .map(|k| w.transform_vector3(Vec3::from(p.normals[v(k)])).normalize()),
                    c: [0, 1, 2].map(|k| p.colors.get(v(k)).map_or(1.0, |c| c[0])),
                    paint,
                    hair,
                    glossy,
                });
            }
        }
    }
    out
}

fn render(tris: &[Tri], skin: Vec3, yaw: f32, pitch: f32, img: &mut [u8], x0: usize, width: usize) {
    let centre = rider::skull_centre() + Vec3::new(0.0, -0.035, 0.0);
    let v = Vec3::new(yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos());
    let r = Vec3::Y.cross(v).normalize();
    let up = v.cross(r);
    let light = (v * 0.8 + up * 0.55 - r * 0.45).normalize();
    let fill = (v * 0.6 + r * 0.8).normalize();
    let mut depth = vec![f32::MIN; SIZE * SIZE];
    let scale = SIZE as f32 / (2.0 * HALF);
    for t in tris {
        let s = t.p.map(|p| {
            let q = p - centre;
            (
                SIZE as f32 / 2.0 + q.dot(r) * scale,
                SIZE as f32 / 2.0 - q.dot(up) * scale,
                q.dot(v),
            )
        });
        // Back faces are culled, as in the game (y grows downwards).
        let area = (s[1].0 - s[0].0) * (s[2].1 - s[0].1) - (s[2].0 - s[0].0) * (s[1].1 - s[0].1);
        if area >= 0.0 {
            continue;
        }
        let (min_x, max_x) = (
            s.iter().map(|p| p.0).fold(f32::MAX, f32::min).floor().max(0.0) as usize,
            s.iter().map(|p| p.0).fold(f32::MIN, f32::max).ceil().min(SIZE as f32 - 1.0) as usize,
        );
        let (min_y, max_y) = (
            s.iter().map(|p| p.1).fold(f32::MAX, f32::min).floor().max(0.0) as usize,
            s.iter().map(|p| p.1).fold(f32::MIN, f32::max).ceil().min(SIZE as f32 - 1.0) as usize,
        );
        for y in min_y..=max_y {
            for x in min_x..=max_x {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let e = |a: (f32, f32, f32), b: (f32, f32, f32)| {
                    (b.0 - a.0) * (py - a.1) - (b.1 - a.1) * (px - a.0)
                };
                let w = [e(s[1], s[2]) / area, e(s[2], s[0]) / area, e(s[0], s[1]) / area];
                if w.iter().any(|k| *k < 0.0) {
                    continue;
                }
                let z = w[0] * s[0].2 + w[1] * s[1].2 + w[2] * s[2].2;
                if z <= depth[y * SIZE + x] {
                    continue;
                }
                depth[y * SIZE + x] = z;
                let n = (t.n[0] * w[0] + t.n[1] * w[1] + t.n[2] * w[2]).normalize();
                let at = t.p[0] * w[0] + t.p[1] * w[1] + t.p[2] * w[2];
                let shade = t.c[0] * w[0] + t.c[1] * w[1] + t.c[2] * w[2];
                let albedo = if t.hair {
                    let dots = hash(at) * 0.9;
                    let cover = ((shade * 1.15 - dots) / 0.25).clamp(0.0, 1.0);
                    skin + (t.paint - skin) * cover
                } else {
                    t.paint * shade
                };
                let lit = 0.28 + 0.95 * n.dot(light).max(0.0) + 0.25 * n.dot(fill).max(0.0);
                let half = (light + v).normalize();
                let spec = if t.glossy { 0.6 * n.dot(half).max(0.0).powf(120.0) } else { 0.0 };
                let c = albedo * lit + Vec3::splat(spec);
                let o = (y * width + x0 + x) * 3;
                img[o] = srgb(c.x);
                img[o + 1] = srgb(c.y);
                img[o + 2] = srgb(c.z);
            }
        }
    }
}

fn main() {
    let dir = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    std::fs::create_dir_all(&dir).expect("the output folder");
    let m = rider::build(Detail::High);
    for name in ["head", rider::FACE, rider::LIPS, rider::LIP_LOWER]
        .into_iter()
        .chain(rider::opt::BEARD)
        .chain(rider::opt::MOUSTACHE)
        .chain(rider::opt::FACE_LINES)
        .chain([rider::opt::FACE_JAW])
    {
        let i = m.node(name).expect("node");
        let tris: usize = m.nodes[i].primitives.iter().map(|p| p.triangles()).sum();
        println!("{name}: {tris} triangles");
    }
    println!("mouth (from the skull's centre): {}", ocw_assets::faces::mouth() - rider::skull_centre());
    // How far the helmets' straps are from the face's skin (m).
    let f = ocw_assets::faces::Face::get();
    for h in ocw_assets::helmet::Helmet::ALL {
        let i = m.node(h.node()).expect("helmet");
        let w = m.world(i);
        let mut near: Vec<(i32, i32, f32)> = Vec::new();
        for p in &m.nodes[i].primitives {
            if m.materials[p.material].name != ocw_assets::helmet::DETAIL {
                continue;
            }
            for v in &p.positions {
                let q = w.transform_point3(Vec3::from(*v));
                let rel = q - rider::skull_centre();
                let az = rel.x.atan2(-(rel.z + 0.012)).to_degrees();
                if rel.y < 0.0 && az.abs() < 79.0 {
                    near.push((az.abs() as i32 / 10 * 10, (rel.y * 100.0) as i32, f.over(q)));
                }
            }
        }
        near.sort_by(|a, b| a.partial_cmp(b).expect("numbers"));
        near.dedup_by(|b, a| a.0 == b.0 && a.1 == b.1);
        println!("{h:?} straps (angle, height cm, mm over the skin):");
        for (a, y, over) in near {
            println!("  {a:3} {y:4} {:6.1}", over * 1e3);
        }
    }
    let looks = [
        look("ada", Age::Twenties, 1, "long", 0x7E3A24, "none", "none", "green", 0, false),
        look("cyd", Age::Forties, 11, "crew-cut", 0x3B2A20, "none", "short-boxed", "dark-brown", 0, false),
        look("dag", Age::Seventies, 5, "bald", 0x9C9A96, "classic", "full", "blue", 2, false),
        look("eve", Age::Sixties, 14, "low-bun", 0x15110F, "none", "none", "dark-brown", 2, false),
        look("finn", Age::Thirties, 8, "short-textured", 0x6B4426, "none", "stubble", "hazel", 0, false),
        look("gus", Age::Fifties, 3, "side-part", 0xA0703F, "pencil", "chin-strap", "grey", 1, true),
        look("hal", Age::Thirties, 9, "buzz-cut", 0x15110F, "handlebar", "goatee", "brown", 0, false),
        look("ike", Age::Forties, 6, "shaved", 0xB3482A, "chevron", "long", "amber", 0, false),
        look("jo", Age::Twenties, 15, "afro", 0x15110F, "horseshoe", "short-stubble-beard", "dark-brown", 0, true),
    ];
    let views = [(180f32, 0f32), (140.0, -4.0), (90.0, 0.0), (165.0, -28.0)];
    for (k, (name, c, helmet)) in looks.iter().enumerate() {
        let tris = triangles(&m, c, *helmet);
        let width = SIZE * views.len();
        let mut img = vec![0u8; width * SIZE * 3];
        for px in img.chunks_mut(3) {
            px.copy_from_slice(&[150, 170, 190]);
        }
        for (j, (yaw, pitch)) in views.iter().enumerate() {
            // 180° is the front, as `camera.closeup`'s yaw.
            render(
                &tris,
                linear(c.skin_colour()),
                (yaw - 180.0).to_radians(),
                pitch.to_radians(),
                &mut img,
                j * SIZE,
                width,
            );
        }
        let path = format!("{dir}/face-{}-{name}.png", k + 1);
        image::save_buffer(&path, &img, width as u32, SIZE as u32, image::ExtendedColorType::Rgb8)
            .expect("the picture");
        println!("{path}: {} triangles", tris.len());
    }
}

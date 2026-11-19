use super::*;
use ocw_assets::props::{Instance, place};
use ocw_world::lamps::Lamp;
use ocw_world::obstacles::PointObstacle;

/// Triangles as (centroid key, vertices, normals, colour), sorted.
type Tri = ([i64; 3], [[f32; 3]; 3], [[f32; 3]; 3], [f32; 4]);

fn tris(pos: &[[f32; 3]], nor: &[[f32; 3]], col: &[[f32; 4]], idx: &[u32]) -> Vec<Tri> {
    let mut out: Vec<Tri> = idx
        .chunks(3)
        .map(|t| {
            let v = [pos[t[0] as usize], pos[t[1] as usize], pos[t[2] as usize]];
            let n = [nor[t[0] as usize], nor[t[1] as usize], nor[t[2] as usize]];
            let c: [f32; 3] = std::array::from_fn(|k| (v[0][k] + v[1][k] + v[2][k]) / 3.0);
            let key = c.map(|x| (x * 1e3).round() as i64);
            (key, v, n, col[t[0] as usize])
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

fn placed_arrays(p: &ocw_assets::props::Placed, only: Option<&str>) -> MeshArrays {
    let mut m = MeshArrays::default();
    for part in p.parts.iter().filter(|q| only.is_none_or(|s| q.slot == s)) {
        let base = m.positions.len() as u32;
        m.positions.extend(&part.positions);
        m.normals.extend(&part.normals);
        m.colors.extend(&part.colors);
        m.indices.extend(part.indices.iter().map(|i| i + base));
    }
    m
}

fn same(old: &MeshArrays, new: &MeshArrays, tol: f32, what: &str) {
    let (a, b) = (
        tris(&old.positions, &old.normals, &old.colors, &old.indices),
        tris(&new.positions, &new.normals, &new.colors, &new.indices),
    );
    assert_eq!(a.len(), b.len(), "{what}: triangles");
    for (x, y) in a.iter().zip(&b) {
        for k in 0..3 {
            let d = Vec3::from(x.1[k]).distance(Vec3::from(y.1[k]));
            assert!(d <= tol, "{what}: vertex {:?} vs {:?}", x.1[k], y.1[k]);
            let dn = Vec3::from(x.2[k]).distance(Vec3::from(y.2[k]));
            assert!(dn <= 1e-3 + tol, "{what}: normal {:?} vs {:?}", x.2[k], y.2[k]);
        }
        let dc = Vec4::from(x.3).distance(Vec4::from(y.3));
        assert!(dc < 1e-4, "{what}: colour {:?} vs {:?}", x.3, y.3);
    }
}

fn file(m: &Model) -> Model {
    ocw_assets::glb::read(&ocw_assets::glb::write(m)).expect("reads back")
}

fn obstacle(kind: ObstacleKind, angle: f32, width_m: f32) -> PointObstacle {
    PointObstacle {
        kind,
        flags: 0,
        pos: [12.0, -30.0],
        angle,
        width_m,
        height_m: kind.default_height_m() as f32,
    }
}

fn from_file(m: &Model, p: &PointObstacle, ground: &dyn Fn(f32, f32) -> f32) -> MeshArrays {
    let span = if p.width_m > 0.5 { p.width_m } else { SPAN_M };
    let inst = Instance {
        height_m: Some(p.height_m.max(0.2)),
        span_m: Some(span),
        reach_m: None,
    };
    let foot = Vec3::new(p.pos[0], ground(p.pos[0], p.pos[1]), p.pos[1]);
    placed_arrays(
        &place(
            m,
            &inst,
            foot,
            -(p.angle + std::f32::consts::FRAC_PI_2),
            ground,
        ),
        None,
    )
}

fn bounds(a: &MeshArrays) -> (Vec3, Vec3) {
    a.positions.iter().fold(
        (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)),
        |(lo, hi), v| (lo.min(Vec3::from(*v)), hi.max(Vec3::from(*v))),
    )
}

/// Every obstacle file, drawn on flat ground, is the generator's mesh
/// (vertices, normals, colours within 0.1 mm); on a slope the parts stand
/// on the ground as before (within the slope over the parts' thickness);
/// at other heights, widths and angles its bounds agree.
#[test]
fn obstacle_files_are_the_generators_meshes() {
    let c = crate::obstacles::Palette::new();
    for kind in OBSTACLES {
        let m = file(&obstacle_model(kind));
        let flat = |_: f32, _: f32| 7.0;
        let p = obstacle(kind, 0.0, 0.0);
        let mut old = MeshArrays::default();
        crate::obstacles::point_mesh(&mut old, &p, &c, &flat);
        same(&old, &from_file(&m, &p, &flat), 1e-4, &format!("{kind:?} flat"));
        let slope = |x: f32, z: f32| 7.0 + 0.1 * x - 0.08 * z;
        let mut old = MeshArrays::default();
        crate::obstacles::point_mesh(&mut old, &p, &c, &slope);
        same(&old, &from_file(&m, &p, &slope), 0.04, &format!("{kind:?} slope"));
        // A gate across a 6 m road: the stretched gate is the generator's.
        if kind == ObstacleKind::Gate {
            let p = obstacle(kind, 0.0, 6.0);
            let mut old = MeshArrays::default();
            crate::obstacles::point_mesh(&mut old, &p, &c, &flat);
            same(&old, &from_file(&m, &p, &flat), 1e-4, "gate 6 m");
        }
        if kind == ObstacleKind::Pylon {
            continue; // square to the world before, turned with its way now
        }
        for (angle, width, height) in [(0.7f32, 0.0, None), (2.2, 5.0, Some(1.6f32))] {
            let mut p = obstacle(kind, angle, width);
            if let Some(h) = height {
                p.height_m = h;
            }
            let mut old = MeshArrays::default();
            crate::obstacles::point_mesh(&mut old, &p, &c, &flat);
            let ((lo0, hi0), (lo1, hi1)) = (bounds(&old), bounds(&from_file(&m, &p, &flat)));
            // Taller than the file: the whole model is scaled, so parts
            // the generator kept at a fixed offset (the lift gate's post
            // top 0.15 m over the arm) move a little (7.5 cm at 1.6 m).
            assert!(
                lo0.distance(lo1) < 0.08 && hi0.distance(hi1) < 0.08,
                "{kind:?} at {angle}: {lo0} {hi0} vs {lo1} {hi1}"
            );
        }
    }
}

/// Every lamp file is the generator's lamp (pole and arm, the head, the
/// light under it) for any arm of the placement (0.6 – 2 m).
#[test]
fn lamp_files_are_the_generators_lamps() {
    for (id, h) in crate::street_lamps::LAMP_FILES {
        let m = file(&lamp_model(&format!("{id}.glb"), h));
        for arm in [0.6, 1.5, 2.0] {
            let lamp = Lamp {
                foot: [10.0, 500.0, 20.0],
                toward: [1.0, 0.0],
                height_m: h,
                arm_m: arm,
            };
            let old = crate::street_lamps::lamp_arrays(&[(lamp, 499.5)]);
            let inst = Instance {
                height_m: Some(h),
                span_m: None,
                reach_m: Some(arm),
            };
            let flat = |_: f32, _: f32| 0.0;
            let p = place(&m, &inst, Vec3::new(10.0, 499.5, 20.0), 0.0, &flat);
            same(&old.poles, &placed_arrays(&p, Some("paint_dark")), 1e-4, id);
            same(&old.heads, &placed_arrays(&p, Some("bulb")), 1e-4, id);
            let light = p.anchor("anchor_light").unwrap();
            assert!(
                light.distance(Vec3::from(old.spots[0])) < 1e-4,
                "{id}: {light}"
            );
        }
    }
}

/// The exported files pass the checker without a finding (names, slots,
/// proxies, the origin on the ground) and the export is deterministic.
#[test]
fn the_export_passes_the_checker() {
    let a = models();
    assert_eq!(a.len(), OBSTACLES.len() + 3);
    for ((e, m), (_, m2)) in a.iter().zip(models()) {
        let bytes = ocw_assets::glb::write(m);
        assert_eq!(bytes, ocw_assets::glb::write(&m2), "{}", e.file);
        let back = file(m);
        let f = ocw_assets::check::check(e, &back, bytes.len());
        assert!(f.is_empty(), "{}: {f:#?}", e.file);
        assert!(
            !ocw_assets::props::proxies(&back).is_empty(),
            "{}",
            e.file
        );
    }
}

//! The Models workspace's edit gizmo and picking (spec §14 Phase 16, 16B
//! *Node edits*, stage E4), pure: which handle of the move / rotate /
//! scale gizmo the pointer is on, what a drag from one screen point to
//! another does to the node's transform (with snapping), and which node a
//! click in the 3D view hits (its triangles, or a contract point's
//! marker). The screen projection comes in as a function, so all of it is
//! tested without a window.
//!
//! The axes: **move** and **rotate** use the parent's frame (the node's
//! translation and rotation are in it), **scale** the node's own axes
//! (its scale is along them).

use bevy::math::{Affine3A, Quat, Vec2, Vec3};
use ocw_assets::model::Model;
use ocw_edit::model::Trs;

/// The gizmo's mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GizmoMode {
    #[default]
    Move,
    Rotate,
    Scale,
}

impl GizmoMode {
    pub const ALL: [GizmoMode; 3] = [GizmoMode::Move, GizmoMode::Rotate, GizmoMode::Scale];

    pub fn name(self) -> &'static str {
        match self {
            GizmoMode::Move => "move",
            GizmoMode::Rotate => "rotate",
            GizmoMode::Scale => "scale",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == s)
    }
}

/// A drag of a gizmo handle: what it started from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GizmoDrag {
    pub node: ocw_edit::model::NodeId,
    pub axis: usize,
    pub mode: GizmoMode,
    /// The frame, the transform and the pointer at the start.
    pub frame: Frame,
    pub start: Trs,
    pub parent_rot: Quat,
    pub from: Vec2,
}

/// Snapping steps (16B: "transform nodes with a move / rotate / scale
/// gizmo and snapping").
pub const SNAP_MOVE_M: f32 = 0.005;
pub const SNAP_ROTATE_RAD: f32 = std::f32::consts::PI / 24.0; // 7.5°
pub const SNAP_SCALE: f32 = 0.05;

/// How near the pointer must be to a handle (logical pixels).
pub const HANDLE_TOLERANCE_PX: f32 = 9.0;

/// The gizmo of a node in the scene: its origin and the three axes
/// (unit vectors in the scene) it acts along.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub origin: Vec3,
    pub axes: [Vec3; 3],
    /// The handles' length (m).
    pub len: f32,
}

/// The gizmo frame of node `i` of `m`, placed in the scene by `offset`
/// (the model's root transform), for `mode`.
pub fn frame(m: &Model, i: usize, offset: Affine3A, mode: GizmoMode, len: f32) -> Frame {
    let world = offset * m.world(i);
    let basis = match mode {
        GizmoMode::Scale => world,
        _ => {
            offset
                * match m.nodes[i].parent {
                    Some(p) => m.world(p),
                    None => Affine3A::IDENTITY,
                }
        }
    };
    let axes = [Vec3::X, Vec3::Y, Vec3::Z].map(|a| {
        let v = basis.transform_vector3(a);
        v.normalize_or(a)
    });
    Frame {
        origin: world.translation.into(),
        axes,
        len,
    }
}

/// Points of the rotate handle of `axis` (a ring around it).
pub fn ring(f: &Frame, axis: usize, segments: usize) -> Vec<Vec3> {
    let a = f.axes[(axis + 1) % 3];
    let b = f.axes[(axis + 2) % 3];
    (0..=segments)
        .map(|k| {
            let t = k as f32 / segments as f32 * std::f32::consts::TAU;
            f.origin + (a * t.cos() + b * t.sin()) * f.len
        })
        .collect()
}

fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let t = if ab.length_squared() > 0.0 {
        ((p - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// The handle (axis 0 – 2) under the pointer at `mouse`, if any.
pub fn pick_handle(
    f: &Frame,
    mode: GizmoMode,
    project: impl Fn(Vec3) -> Option<Vec2>,
    mouse: Vec2,
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for axis in 0..3 {
        let d = match mode {
            GizmoMode::Move | GizmoMode::Scale => {
                let (Some(a), Some(b)) = (
                    project(f.origin + f.axes[axis] * f.len * 0.15),
                    project(f.origin + f.axes[axis] * f.len),
                ) else {
                    continue;
                };
                segment_distance(mouse, a, b)
            }
            GizmoMode::Rotate => {
                let pts: Vec<Vec2> = ring(f, axis, 48).into_iter().filter_map(&project).collect();
                pts.windows(2)
                    .map(|w| segment_distance(mouse, w[0], w[1]))
                    .fold(f32::INFINITY, f32::min)
            }
        };
        if d <= HANDLE_TOLERANCE_PX && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((axis, d));
        }
    }
    best.map(|(a, _)| a)
}

fn snap(v: f32, step: f32, on: bool) -> f32 {
    if on && step > 0.0 {
        (v / step).round() * step
    } else {
        v
    }
}

/// A drag of handle `axis` from `from` to `to` (screen points): the
/// node's transform from the one it had when the drag began (`start`).
/// `parent_rot`: the rotation of the node's parent frame in the scene
/// (move and rotate act in it); `eye`: the camera's position (a rotation
/// turns as the pointer goes round the ring, seen from it).
#[allow(clippy::too_many_arguments)] // the drag's whole state, pure
pub fn drag(
    f: &Frame,
    mode: GizmoMode,
    axis: usize,
    start: Trs,
    parent_rot: Quat,
    eye: Vec3,
    project: impl Fn(Vec3) -> Option<Vec2>,
    from: Vec2,
    to: Vec2,
    snapping: bool,
) -> Trs {
    let mut t = start;
    let (Some(o), Some(tip)) = (project(f.origin), project(f.origin + f.axes[axis] * f.len)) else {
        return t;
    };
    match mode {
        GizmoMode::Move | GizmoMode::Scale => {
            let s = tip - o;
            if s.length_squared() < 1.0 {
                return t;
            }
            // Metres along the axis for the pointer's way along its image.
            let along = (to - from).dot(s) / s.length_squared() * f.len;
            if mode == GizmoMode::Move {
                let local_axis = parent_rot.inverse() * f.axes[axis];
                // The scene's axis in the parent frame, in metres of the
                // parent frame (its scale divides out).
                let v = snap(along, SNAP_MOVE_M, snapping);
                t.translation = start.translation + local_axis.normalize_or_zero() * v;
                if snapping {
                    let c = &mut t.translation;
                    *c = (*c / SNAP_MOVE_M).round() * SNAP_MOVE_M;
                }
            } else {
                let factor = (1.0 + along / f.len).max(0.01);
                let mut s = start.scale;
                s[axis] = snap(start.scale[axis] * factor, SNAP_SCALE, snapping).max(1e-4);
                t.scale = s;
            }
        }
        GizmoMode::Rotate => {
            let a = (from - o).to_angle();
            let b = (to - o).to_angle();
            let mut angle = b - a;
            if angle > std::f32::consts::PI {
                angle -= std::f32::consts::TAU;
            } else if angle < -std::f32::consts::PI {
                angle += std::f32::consts::TAU;
            }
            // Screen y points down: a turn on the screen is the other way
            // round; and seen from behind the axis, once more.
            let towards = (eye - f.origin).dot(f.axes[axis]) > 0.0;
            if towards {
                angle = -angle;
            }
            let angle = snap(angle, SNAP_ROTATE_RAD, snapping);
            let local_axis = (parent_rot.inverse() * f.axes[axis]).normalize_or_zero();
            t.rotation = (Quat::from_axis_angle(local_axis, angle) * start.rotation).normalize();
        }
    }
    t
}

/// Ray – triangle (Möller–Trumbore): the distance along the ray.
fn ray_triangle(o: Vec3, d: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let e1 = b - a;
    let e2 = c - a;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = o - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// The node a click hits: the nearest triangle along the ray of the nodes
/// `shown` (the level drawn; proxies are not), else a contract point
/// (anchor, hotspot, proxy) whose marker is within the tolerance on the
/// screen. `offset`: the model's root in the scene.
#[allow(clippy::too_many_arguments)] // the click's whole state, pure
pub fn pick_node(
    m: &Model,
    offset: Affine3A,
    origin: Vec3,
    dir: Vec3,
    shown: impl Fn(usize) -> bool,
    points: &[(usize, Vec3)],
    project: impl Fn(Vec3) -> Option<Vec2>,
    mouse: Vec2,
) -> Option<usize> {
    // Markers first: they are small and drawn over the mesh.
    let marker = points
        .iter()
        .filter_map(|(i, p)| Some((*i, project(*p)?.distance(mouse))))
        .filter(|(_, d)| *d <= HANDLE_TOLERANCE_PX)
        .min_by(|a, b| a.1.total_cmp(&b.1));
    if let Some((i, _)) = marker {
        return Some(i);
    }
    let mut best: Option<(usize, f32)> = None;
    for (i, n) in m.nodes.iter().enumerate() {
        if n.primitives.is_empty() || !shown(i) {
            continue;
        }
        let w = offset * m.world(i);
        for p in &n.primitives {
            let v: Vec<Vec3> = p
                .positions
                .iter()
                .map(|x| w.transform_point3(Vec3::from(*x)))
                .collect();
            for tri in p.indices.as_chunks::<3>().0 {
                let (Some(a), Some(b), Some(c)) = (
                    v.get(tri[0] as usize),
                    v.get(tri[1] as usize),
                    v.get(tri[2] as usize),
                ) else {
                    continue;
                };
                if let Some(t) = ray_triangle(origin, dir, *a, *b, *c)
                    && best.is_none_or(|(_, bt)| t < bt)
                {
                    best = Some((i, t));
                }
            }
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_assets::model::{Node, Primitive};

    /// A camera looking down −Z from z = 5: x right, y up, 100 px a metre.
    fn project(p: Vec3) -> Option<Vec2> {
        Some(Vec2::new(400.0 + p.x * 100.0, 300.0 - p.y * 100.0))
    }

    fn model() -> Model {
        let mut m = Model::default();
        let root = m.add(Node::new("root", None, Vec3::new(1.0, 0.0, 0.0)));
        let mut child = Node::new("child", Some(root), Vec3::new(0.0, 1.0, 0.0));
        child.rotation = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let p = Primitive {
            positions: vec![[-0.5, -0.5, 0.0], [0.5, -0.5, 0.0], [0.0, 0.5, 0.0]],
            normals: vec![[0.0, 0.0, 1.0]; 3],
            indices: vec![0, 1, 2],
            ..Default::default()
        };
        child.primitives.push(p);
        m.add(child);
        m
    }

    #[test]
    fn the_handles_are_found_on_screen() {
        let m = model();
        let f = frame(&m, 1, Affine3A::IDENTITY, GizmoMode::Move, 1.0);
        assert!(f.origin.distance(Vec3::new(1.0, 1.0, 0.0)) < 1e-5);
        // The x handle runs from (1.15, 1) to (2, 1) m.
        let on_x = project(Vec3::new(1.6, 1.0, 0.0)).unwrap();
        assert_eq!(pick_handle(&f, GizmoMode::Move, project, on_x), Some(0));
        let on_y = project(Vec3::new(1.0, 1.7, 0.0)).unwrap() + Vec2::new(4.0, 0.0);
        assert_eq!(pick_handle(&f, GizmoMode::Move, project, on_y), Some(1));
        let off = project(Vec3::new(1.5, 1.5, 0.0)).unwrap();
        assert_eq!(pick_handle(&f, GizmoMode::Move, project, off), None);
        // Scale handles follow the node's own (turned) axes.
        let s = frame(&m, 1, Affine3A::IDENTITY, GizmoMode::Scale, 1.0);
        assert!(s.axes[0].distance(Vec3::Y) < 1e-5);
        let up = project(Vec3::new(1.0, 1.6, 0.0)).unwrap();
        assert_eq!(pick_handle(&s, GizmoMode::Scale, project, up), Some(0));
        // The z ring of rotate is a circle on the screen.
        let r = frame(&m, 1, Affine3A::IDENTITY, GizmoMode::Rotate, 1.0);
        let at = project(Vec3::new(
            1.0 + std::f32::consts::FRAC_1_SQRT_2,
            1.0 + std::f32::consts::FRAC_1_SQRT_2,
            0.0,
        ))
        .unwrap();
        assert_eq!(pick_handle(&r, GizmoMode::Rotate, project, at), Some(2));
    }

    #[test]
    fn a_drag_moves_rotates_and_scales_with_snapping() {
        let m = model();
        let start = Trs::of(&m.nodes[1]);
        let f = frame(&m, 1, Affine3A::IDENTITY, GizmoMode::Move, 1.0);
        let from = Vec2::new(500.0, 200.0);
        // 23 px right = 0.23 m along x (the parent's frame is the scene's).
        let t = drag(
            &f,
            GizmoMode::Move,
            0,
            start,
            Quat::IDENTITY,
            Vec3::new(0.0, 0.0, 5.0),
            project,
            from,
            from + Vec2::new(23.0, 9.0),
            false,
        );
        assert!((t.translation - start.translation).distance(Vec3::new(0.23, 0.0, 0.0)) < 1e-4);
        let t = drag(
            &f,
            GizmoMode::Move,
            0,
            start,
            Quat::IDENTITY,
            Vec3::new(0.0, 0.0, 5.0),
            project,
            from,
            from + Vec2::new(23.2, 0.0),
            true,
        );
        assert!((t.translation.x - (start.translation.x + 0.23)).abs() < 1e-5);
        // Scale along the node's x (the scene's y): 50 px up = × 1.5.
        let s = frame(&m, 1, Affine3A::IDENTITY, GizmoMode::Scale, 1.0);
        let t = drag(
            &s,
            GizmoMode::Scale,
            0,
            start,
            Quat::IDENTITY,
            Vec3::new(0.0, 0.0, 5.0),
            project,
            from,
            from + Vec2::new(0.0, -50.0),
            false,
        );
        assert!((t.scale.x - 1.5).abs() < 1e-4 && t.scale.y == 1.0);
        // A quarter turn round the z ring, seen from +z: +90° about z.
        let r = frame(&m, 1, Affine3A::IDENTITY, GizmoMode::Rotate, 1.0);
        let o = project(r.origin).unwrap();
        let t = drag(
            &r,
            GizmoMode::Rotate,
            2,
            start,
            Quat::IDENTITY,
            Vec3::new(0.0, 0.0, 5.0),
            project,
            o + Vec2::new(100.0, 0.0),
            o + Vec2::new(0.0, -100.0),
            true,
        );
        let want = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2) * start.rotation;
        assert!(t.rotation.dot(want).abs() > 1.0 - 1e-6, "{:?}", t.rotation);
    }

    #[test]
    fn a_click_picks_the_triangle_or_the_marker() {
        let m = model();
        // The child's triangle (turned 90°) around (1, 1, 0).
        let hit = pick_node(
            &m,
            Affine3A::IDENTITY,
            Vec3::new(1.1, 1.0, 5.0),
            Vec3::NEG_Z,
            |_| true,
            &[],
            project,
            Vec2::ZERO,
        );
        assert_eq!(hit, Some(1));
        let miss = pick_node(
            &m,
            Affine3A::IDENTITY,
            Vec3::new(3.0, 3.0, 5.0),
            Vec3::NEG_Z,
            |_| true,
            &[],
            project,
            Vec2::ZERO,
        );
        assert_eq!(miss, None);
        // Not drawn: not picked.
        let hidden = pick_node(
            &m,
            Affine3A::IDENTITY,
            Vec3::new(1.1, 1.0, 5.0),
            Vec3::NEG_Z,
            |_| false,
            &[],
            project,
            Vec2::ZERO,
        );
        assert_eq!(hidden, None);
        // A marker near the pointer wins.
        let p = Vec3::new(1.0, 0.0, 0.0);
        let marker = pick_node(
            &m,
            Affine3A::IDENTITY,
            Vec3::new(1.1, 1.0, 5.0),
            Vec3::NEG_Z,
            |_| true,
            &[(0, p)],
            project,
            project(p).unwrap() + Vec2::new(3.0, 3.0),
        );
        assert_eq!(marker, Some(0));
    }
}

//! The GPS bike computer of VR (spec §14 Phase 11, 11B; v4.52): an
//! original, unbranded head unit for an out-front mount – a compact,
//! wedge-like body (rounded sides, a slightly lower nose), a flat screen
//! window on top, a row of LEDs above the screen's rear edge, two side
//! buttons and a lap button on the nose, the twist-lock puck underneath.
//! No maker's name, logo or shape: the out-front GPS unit is only the idea.
//!
//! Exported as `parts/computer/computer-head-unit-wedge.glb` (a part of the
//! `computer` slot; origin: the unit's centre, at real size: 52 × 20 ×
//! 78 mm). App convention: −Z forward (the unit's nose), +Y up. Nodes: the
//! root `computer`, `anchor_screen` (the screen's centre: the renderer lays
//! the live screen there, facing +Y, its picture's top toward −Z) and
//! `anchor_mount` (the puck's underside, where the mount's arm ends).
//! The mount's arm is drawn by the renderer: its length depends on the bar
//! and the size the rider chose (`ocw_xr::computer`).

use glam::{Quat, Vec3};

use crate::mesh::{Cap, Parts, Ring};
use crate::model::{Model, Node};

/// The file (15H: function first, then shape).
pub const FILE: &str = "parts/computer/computer-head-unit-wedge.glb";
/// The node of the screen's centre.
pub const SCREEN_ANCHOR: &str = "anchor_screen";
/// The node under the puck.
pub const MOUNT_ANCHOR: &str = "anchor_mount";

/// Half the body: width, height, length (m; `ocw_xr::computer::BODY_M`).
const HALF: Vec3 = Vec3::new(0.026, 0.010, 0.039);
/// The screen window: half width, half length, its centre ahead of the
/// body's centre (`ocw_xr::computer::SCREEN_M`, `SCREEN_AHEAD_M`).
const SCREEN_HALF: (f32, f32) = (0.020, 0.029);
const SCREEN_AHEAD: f32 = 0.004;

/// Builds the unit.
pub fn build() -> Model {
    let mut m = Model::default();
    let root = m.add(Node::new("computer", None, Vec3::ZERO));
    let mut p = Parts::new();
    // The body: rounded cross-sections lofted from the rear (+Z, toward
    // the rider) to the nose, which narrows and drops a little.
    let sides = 16;
    let section = |z: f32, w: f32, h: f32, drop: f32| {
        Ring::ellipse(
            Vec3::new(0.0, -drop, z),
            Vec3::NEG_Z,
            Vec3::X,
            HALF.x * w,
            HALF.y * h,
        )
    };
    let rings = [
        section(HALF.z, 0.86, 0.80, 0.0),
        section(HALF.z * 0.92, 0.98, 0.97, 0.0),
        section(HALF.z * 0.55, 1.0, 1.0, 0.0),
        section(-HALF.z * 0.45, 0.99, 0.97, 0.0005),
        section(-HALF.z * 0.88, 0.92, 0.86, 0.0012),
        section(-HALF.z, 0.80, 0.72, 0.0016),
    ];
    p.slot("computer_body")
        .loft(&rings, sides, false, Cap::Flat, Cap::Flat);
    // A raised bezel around the screen and the glass on it; the renderer
    // draws the live screen just above the glass.
    let top = HALF.y * 0.97;
    let screen = Vec3::new(0.0, top + 0.0004, -SCREEN_AHEAD);
    p.slot("component").cuboid(
        screen - Vec3::Y * 0.0002,
        Vec3::new(SCREEN_HALF.0 + 0.0022, 0.0006, SCREEN_HALF.1 + 0.0024),
        Quat::IDENTITY,
    );
    p.slot("computer_screen").cuboid(
        screen + Vec3::Y * 0.0002,
        Vec3::new(SCREEN_HALF.0, 0.0003, SCREEN_HALF.1),
        Quat::IDENTITY,
    );
    // The LED bar behind the screen (toward the rider): seven lamps.
    for i in 0..7 {
        let x = (i as f32 - 3.0) * 0.0042;
        p.slot("computer_led").cuboid(
            Vec3::new(x, top + 0.0003, SCREEN_HALF.1 - SCREEN_AHEAD + 0.0045),
            Vec3::new(0.0015, 0.0004, 0.0011),
            Quat::IDENTITY,
        );
    }
    // Two side buttons near the rear, a lap button on the nose.
    for side in [-1.0f32, 1.0] {
        p.slot("component").cuboid(
            Vec3::new(side * (HALF.x - 0.0006), 0.001, HALF.z * 0.45),
            Vec3::new(0.0012, 0.0025, 0.006),
            Quat::IDENTITY,
        );
    }
    p.slot("component").cuboid(
        Vec3::new(0.0, -0.002, -HALF.z + 0.0002),
        Vec3::new(0.008, 0.0022, 0.0012),
        Quat::IDENTITY,
    );
    // The twist-lock puck underneath.
    let puck_top = -HALF.y * 0.95;
    let puck = puck_top - 0.0045;
    p.slot("component").tube(
        Vec3::new(0.0, puck_top, 0.0),
        Vec3::new(0.0, puck, 0.0),
        0.0125,
        0.0115,
        14,
        true,
    );
    p.attach(&mut m, root);
    m.add(Node::new(
        SCREEN_ANCHOR,
        Some(root),
        screen + Vec3::Y * 0.0006,
    ));
    m.add(Node::new(
        MOUNT_ANCHOR,
        Some(root),
        Vec3::new(0.0, puck, 0.0),
    ));
    m
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kinds::{self, Kind};

    #[test]
    fn the_unit_is_small_light_and_has_its_anchors() {
        let m = build();
        let (budget, _) = Kind::Part.budget();
        assert!(m.triangles() <= budget, "{} triangles", m.triangles());
        assert!(m.triangles() > 200, "not a plain box");
        let screen = m.position(SCREEN_ANCHOR).expect("screen anchor");
        let mount = m.position(MOUNT_ANCHOR).expect("mount anchor");
        assert!(screen.y > HALF.y * 0.9 && screen.z < 0.0, "{screen}");
        assert!(mount.y < -HALF.y, "{mount}");
        // Every vertex inside the real size (52 × 20 × 78 mm, the puck
        // below, the buttons proud of it).
        for n in &m.nodes {
            for prim in &n.primitives {
                for v in &prim.positions {
                    let v = Vec3::from(*v);
                    // The buttons stand out by a millimetre.
                    assert!(v.x.abs() <= HALF.x + 0.0015, "{v}");
                    assert!(v.z.abs() <= HALF.z + 0.0015, "{v}");
                    assert!(v.y <= HALF.y + 0.0015 && v.y >= mount.y - 1e-4, "{v}");
                }
            }
        }
        // Only known paint slots.
        for mat in &m.materials {
            assert!(crate::slots::get(&mat.name).is_some(), "{}", mat.name);
        }
        assert!(kinds::check_name(FILE, Kind::Part).is_ok());
    }

    /// The numbers the VR placement uses are the model's.
    #[test]
    fn the_sizes_match_the_vr_placement() {
        assert_eq!(HALF * 2.0, ocw_xr_sizes().0);
        assert_eq!((SCREEN_HALF.0 * 2.0, SCREEN_HALF.1 * 2.0), ocw_xr_sizes().1);
    }

    /// `ocw_xr::computer`'s constants (no dependency on ocw-xr here: the
    /// renderer's tests compare both too).
    fn ocw_xr_sizes() -> (Vec3, (f32, f32)) {
        (Vec3::new(0.052, 0.020, 0.078), (0.040, 0.058))
    }
}

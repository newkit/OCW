//! Physics spike (spec §14 Phase 12 Q5): a chain of ten capsules on
//! spherical joints falls onto the ground with Rapier (no Bevy plugin).
use rapier3d::prelude::*;

pub fn world() -> (PhysicsWorld, Vec<RigidBodyHandle>) {
    let mut w = PhysicsWorld::new();
    let ground = w.bodies.insert(RigidBodyBuilder::fixed());
    w.colliders.insert_with_parent(
        ColliderBuilder::cuboid(50.0, 0.1, 50.0).translation(Vec3::new(0.0, -0.1, 0.0)),
        ground,
        &mut w.bodies,
    );
    let mut out: Vec<RigidBodyHandle> = Vec::new();
    for i in 0..10 {
        let b = w.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vec3::new(i as f32 * 0.42, 1.5, 0.0))
                .linvel(Vec3::new(3.0, 0.0, 1.0)),
        );
        w.colliders
            .insert_with_parent(ColliderBuilder::capsule_x(0.15, 0.06), b, &mut w.bodies);
        if let Some(p) = out.last().copied() {
            let j = SphericalJointBuilder::new()
                .local_anchor1(Vec3::new(0.21, 0.0, 0.0))
                .local_anchor2(Vec3::new(-0.21, 0.0, 0.0))
                .build()
                .data;
            w.impulse_joints.insert(p, b, j, true);
        }
        out.push(b);
    }
    (w, out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ragdoll_chain_falls_and_settles() {
        let (mut w, bodies) = world();
        let t = std::time::Instant::now();
        for _ in 0..300 {
            w.step();
        }
        let per = t.elapsed().as_secs_f64() / 300.0 * 1000.0;
        let ys: Vec<f32> = bodies
            .iter()
            .map(|b| w.bodies[*b].translation().y)
            .collect();
        println!("rapier: {per:.3} ms per step, heights {ys:?}");
        assert!(ys.iter().all(|y| *y < 0.5 && *y > -0.1), "{ys:?}");
    }
}

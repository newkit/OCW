//! Physics spike (spec §14 Phase 12 Q5): a chain of ten capsules on
//! spherical joints falls onto the ground with Avian.
use avian3d::prelude::*;
use bevy::prelude::*;

pub fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        MinimalPlugins,
        TransformPlugin,
        bevy::asset::AssetPlugin::default(),
        bevy::mesh::MeshPlugin,
        PhysicsPlugins::default(),
    ));
    app.insert_resource(Time::<Fixed>::from_hz(60.0));
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f64(1.0 / 60.0),
    ));
    app.finish();
    app.cleanup();
    app
}

pub fn spawn(world: &mut World) -> Vec<Entity> {
    world.spawn((
        RigidBody::Static,
        Collider::half_space(Vec3::Y),
        Transform::default(),
    ));
    let mut out = Vec::new();
    let mut prev: Option<Entity> = None;
    for i in 0..10 {
        let e = world
            .spawn((
                RigidBody::Dynamic,
                Collider::capsule(0.06, 0.3),
                Transform::from_xyz(i as f32 * 0.42, 1.5, 0.0)
                    .with_rotation(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                LinearVelocity(Vec3::new(3.0, 0.0, 1.0)),
            ))
            .id();
        if let Some(p) = prev {
            world.spawn(
                SphericalJoint::new(p, e)
                    .with_local_anchor1(Vec3::new(0.0, -0.21, 0.0))
                    .with_local_anchor2(Vec3::new(0.0, 0.21, 0.0))
                    .with_swing_limits(-1.0, 1.0),
            );
        }
        prev = Some(e);
        out.push(e);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ragdoll_chain_falls_and_settles() {
        let mut a = app();
        let bodies = spawn(a.world_mut());
        a.update();
        let t = std::time::Instant::now();
        for _ in 0..300 {
            a.update();
        }
        let per = t.elapsed().as_secs_f64() / 300.0 * 1000.0;
        let ys: Vec<f32> = bodies
            .iter()
            .map(|e| a.world().get::<Transform>(*e).unwrap().translation.y)
            .collect();
        println!("avian: {per:.3} ms per frame, heights {ys:?}");
        assert!(ys.iter().all(|y| *y < 0.5 && *y > -0.1), "{ys:?}");
    }
}

//! Camera that follows the local rider.

use bevy::prelude::*;

use crate::riders::rider_transform;
use crate::{RideView, world};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CameraMode>()
            .add_systems(Startup, spawn_camera)
            .add_systems(PostUpdate, follow.before(TransformSystems::Propagate));
    }
}

/// Camera perspective, switchable with the console command `camera.mode`.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CameraMode {
    /// Behind and above the rider.
    #[default]
    Chase,
    /// From the side of the road.
    Side,
    /// High above, looking down.
    Top,
}

impl std::str::FromStr for CameraMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "chase" => Ok(Self::Chase),
            "side" => Ok(Self::Side),
            "top" => Ok(Self::Top),
            other => Err(format!("unknown camera mode '{other}' (chase, side, top)")),
        }
    }
}

#[derive(Component)]
pub struct FollowCamera;

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(0.0, 10.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
        world::fog(),
        FollowCamera,
    ));
}

fn follow(
    view: Res<RideView>,
    mode: Res<CameraMode>,
    time: Res<Time>,
    mut camera: Query<&mut Transform, With<FollowCamera>>,
) {
    let Some(local) = view.local() else { return };
    let Ok(mut cam) = camera.single_mut() else {
        return;
    };
    let rider = rider_transform(&view, local.state.distance, local.state.lateral_offset);
    let p = view.route.sample(local.state.distance);
    let forward = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let right = Vec3::new(-forward.z, 0.0, forward.x);
    let (eye, target) = match *mode {
        CameraMode::Chase => (
            rider.translation - forward * 6.5 + Vec3::Y * 2.6,
            rider.translation + forward * 6.0 + Vec3::Y * 1.0,
        ),
        CameraMode::Side => (
            rider.translation + right * 7.0 + Vec3::Y * 1.5,
            rider.translation + Vec3::Y * 0.9,
        ),
        CameraMode::Top => (
            rider.translation - forward * 30.0 + Vec3::Y * 60.0,
            rider.translation,
        ),
    };
    // Smooth the eye position; snap if far away (teleport, mode switch at start).
    let k = 1.0 - (-8.0 * time.delta_secs()).exp();
    cam.translation = if cam.translation.distance(eye) > 50.0 {
        eye
    } else {
        cam.translation.lerp(eye, k)
    };
    cam.look_at(target, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn camera_mode_parses() {
        assert_eq!("chase".parse::<CameraMode>(), Ok(CameraMode::Chase));
        assert_eq!("side".parse::<CameraMode>(), Ok(CameraMode::Side));
        assert_eq!("top".parse::<CameraMode>(), Ok(CameraMode::Top));
        assert!("Top".parse::<CameraMode>().is_err());
        assert!("".parse::<CameraMode>().is_err());
        assert_eq!(CameraMode::default(), CameraMode::Chase);
    }
}

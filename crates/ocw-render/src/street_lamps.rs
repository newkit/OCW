//! Street lamps (spec §14 Phase 9, S2): poles with lamp heads along lit
//! roads (OSM `lit`, else roads in towns; placement in
//! [`ocw_world::lamps`]), built with the package tiles; the heads glow
//! from dusk to dawn, and a small pool of point lights follows the camera
//! to the nearest lamps – the warm light pools on the road, cheap enough
//! for any GPU (no shadows, a dozen lights). Also the light pollution at
//! the rider for the night sky ([`crate::night_sky::LightPollution`]).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use ocw_world::lamps::Lamp;
use ocw_world::package::{TileKey, WorldPackage};

use crate::RideView;
use crate::building_material::night_factor;
use crate::camera::FollowCamera;
use crate::package::{Keep, MeshArrays};

/// Point lights in the pool around the camera.
pub const LAMP_LIGHTS: usize = 12;
/// How far from the camera a lamp gets one of them (m).
pub const LAMP_LIGHT_RANGE_M: f32 = 220.0;
/// Light of one street lamp (lumens, scene units) and its reach (m).
pub const LAMP_LUMENS: f32 = 3_500.0;
const LAMP_REACH_M: f32 = 26.0;
/// Warm white of LED and sodium street lights.
pub const LAMP_RGB: [f32; 3] = [1.0, 0.80, 0.55];
/// Emissive of a glowing lamp head (physical units, scaled by exposure).
const HEAD_GLOW: f32 = 90.0;

/// The lamps of a tile, ready to upload.
#[derive(Default)]
pub struct TileLamps {
    /// Poles and arms (vertex-coloured).
    pub poles: MeshArrays,
    /// The lamp heads (glowing at night).
    pub heads: MeshArrays,
    /// Where the light comes from (package metres, just under the heads).
    pub spots: Vec<[f32; 3]>,
}

/// The lamp head positions of a streamed tile in its package's frame (a
/// component of the tile's entity).
#[derive(Component, Debug, Clone, Default)]
pub struct LampSpots(pub Vec<Vec3>);

/// One of the point lights near the camera.
#[derive(Component)]
pub struct LampLight;

/// The lamp materials.
#[derive(Resource)]
pub struct LampMaterials {
    pub heads: Handle<StandardMaterial>,
    /// The night factor the heads glow for.
    night: f32,
}

fn push_box(m: &mut MeshArrays, centre: Vec3, half: Vec3, rot: Quat, colour: [f32; 4]) {
    // Six faces, four vertices each, outward normals.
    let faces = [
        (Vec3::X, Vec3::Y, Vec3::Z),
        (-Vec3::X, Vec3::Y, -Vec3::Z),
        (Vec3::Y, Vec3::Z, Vec3::X),
        (-Vec3::Y, Vec3::Z, -Vec3::X),
        (Vec3::Z, Vec3::X, Vec3::Y),
        (-Vec3::Z, Vec3::X, -Vec3::Y),
    ];
    for (n, u, v) in faces {
        let b = m.positions.len() as u32;
        let c = n * half;
        let (du, dv) = (u * half, v * half);
        for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            let p = centre + rot * (c + du * su + dv * sv);
            m.positions.push(p.to_array());
            m.normals.push((rot * n).to_array());
            m.colors.push(colour);
        }
        // Counter-clockwise seen from outside: u × v = n.
        m.indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
}

/// Meshes of lamps standing at `feet` heights (package metres).
pub fn lamp_arrays(lamps: &[(Lamp, f32)]) -> TileLamps {
    let mut out = TileLamps::default();
    let grey = [0.16, 0.17, 0.18, 1.0];
    let head = [0.75, 0.74, 0.70, 1.0];
    for (l, foot_y) in lamps {
        let foot = Vec3::new(l.foot[0], *foot_y, l.foot[2]);
        let toward = Vec3::new(l.toward[0], 0.0, l.toward[1]);
        let yaw = Quat::from_rotation_y((-toward.z).atan2(toward.x));
        let h = l.height_m;
        // The pole, the arm over the road, the head at its end.
        push_box(
            &mut out.poles,
            foot + Vec3::Y * (h / 2.0),
            Vec3::new(0.07, h / 2.0 + 0.1, 0.07),
            Quat::IDENTITY,
            grey,
        );
        push_box(
            &mut out.poles,
            foot + Vec3::Y * (h + 0.05) + toward * (l.arm_m / 2.0),
            Vec3::new(l.arm_m / 2.0 + 0.05, 0.04, 0.04),
            yaw,
            grey,
        );
        let at = foot + Vec3::Y * (h - 0.02) + toward * l.arm_m;
        push_box(&mut out.heads, at, Vec3::new(0.28, 0.06, 0.14), yaw, head);
        out.spots.push((at - Vec3::Y * 0.3).to_array());
    }
    out
}

/// The lamps of a package tile with the heights of their feet: on the
/// terrain beside the road, or at the road's height where the ground is far
/// off (on bridges, along walls and cuttings).
pub fn tile_lamps(pkg: &WorldPackage, key: TileKey, keep: Keep) -> TileLamps {
    let lamps: Vec<(Lamp, f32)> = ocw_world::lamps::tile_lamps(pkg, key)
        .into_iter()
        .filter(|l| keep.is_none_or(|f| f([f64::from(l.foot[0]), f64::from(l.foot[2])])))
        .map(|l| {
            let road = l.foot[1];
            let ground = pkg
                .surface_height(f64::from(l.foot[0]), f64::from(l.foot[2]))
                .map(|h| h as f32)
                .filter(|h| (h - road).abs() < 1.5)
                .unwrap_or(road);
            (l, ground)
        })
        .collect();
    lamp_arrays(&lamps)
}

pub struct StreetLampsPlugin;

impl Plugin for StreetLampsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_lamps)
            .add_systems(Update, (light_lamps, update_light_pollution));
    }
}

fn setup_lamps(mut commands: Commands, materials: Option<ResMut<Assets<StandardMaterial>>>) {
    let Some(mut materials) = materials else {
        return;
    };
    commands.insert_resource(LampMaterials {
        heads: materials.add(StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.4,
            ..default()
        }),
        night: -1.0,
    });
    for _ in 0..LAMP_LIGHTS {
        commands.spawn((
            LampLight,
            PointLight {
                color: Color::linear_rgb(LAMP_RGB[0], LAMP_RGB[1], LAMP_RGB[2]),
                intensity: 0.0,
                range: LAMP_REACH_M,
                radius: 0.15,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::default(),
            Visibility::Hidden,
            NotShadowCaster,
        ));
    }
}

/// The `n` spots nearest to `camera` within `range` (world positions).
pub fn nearest_spots(spots: &[Vec3], camera: Vec3, n: usize, range: f32) -> Vec<Vec3> {
    let mut v: Vec<(f32, Vec3)> = spots
        .iter()
        .map(|&p| (p.distance_squared(camera), p))
        .filter(|(d, _)| *d <= range * range)
        .collect();
    v.sort_by(|a, b| a.0.total_cmp(&b.0));
    v.into_iter().take(n).map(|(_, p)| p).collect()
}

#[allow(clippy::type_complexity)]
fn light_lamps(
    view: Option<Res<RideView>>,
    lamps: Option<ResMut<LampMaterials>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    tiles: Query<(&LampSpots, &GlobalTransform)>,
    cameras: Query<&GlobalTransform, With<FollowCamera>>,
    mut lights: Query<(&mut Transform, &mut PointLight, &mut Visibility), With<LampLight>>,
) {
    let night = view
        .as_ref()
        .and_then(|v| v.sky.as_ref())
        .map_or(0.0, |s| night_factor(s.sky.sun.elevation_deg));
    // The heads glow (changed in 2 % steps: a changed material is uploaded).
    if let (Some(mut lamps), Some(mut materials)) = (lamps, materials)
        && (lamps.night - night).abs() > 0.02
    {
        lamps.night = night;
        if let Some(mut m) = materials.get_mut(&lamps.heads) {
            let g = HEAD_GLOW * night;
            m.emissive = LinearRgba::rgb(LAMP_RGB[0] * g, LAMP_RGB[1] * g, LAMP_RGB[2] * g);
        }
    }
    let camera = cameras.iter().next().map(GlobalTransform::translation);
    let near = match (camera, night > 0.0) {
        (Some(c), true) => {
            let mut all = Vec::new();
            for (spots, tf) in &tiles {
                all.extend(spots.0.iter().map(|p| tf.transform_point(*p)));
            }
            nearest_spots(&all, c, LAMP_LIGHTS, LAMP_LIGHT_RANGE_M)
        }
        _ => Vec::new(),
    };
    for (i, (mut tf, mut light, mut vis)) in lights.iter_mut().enumerate() {
        match near.get(i) {
            Some(&p) => {
                if tf.translation != p {
                    tf.translation = p;
                }
                let want = LAMP_LUMENS * night;
                if light.intensity != want {
                    light.intensity = want;
                }
                vis.set_if_neq(Visibility::Inherited);
            }
            None => {
                vis.set_if_neq(Visibility::Hidden);
            }
        }
    }
}

/// Light pollution at the rider from the home package's buildings,
/// recomputed when the rider has moved 250 m.
fn update_light_pollution(
    view: Option<Res<RideView>>,
    package: Option<Res<crate::package::PackageWorld>>,
    mut pollution: ResMut<crate::night_sky::LightPollution>,
    mut last: Local<Option<[f64; 2]>>,
) {
    let (Some(view), Some(package)) = (view, package) else {
        if pollution.0 != 0.0 {
            pollution.0 = 0.0;
        }
        return;
    };
    let Some(at) = crate::package::rider_position(&view) else {
        return;
    };
    if last.is_some_and(|l| (l[0] - at[0]).hypot(l[1] - at[1]) < 250.0) {
        return;
    }
    *last = Some(at);
    let v = ocw_world::lamps::light_pollution(package.home(), at) as f32;
    pollution.set_if_neq(crate::night_sky::LightPollution(v));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lamp_meshes_stand_on_their_feet() {
        let lamp = Lamp {
            foot: [10.0, 500.0, 20.0],
            toward: [-1.0, 0.0],
            height_m: 6.0,
            arm_m: 1.5,
        };
        let t = lamp_arrays(&[(lamp, 499.5)]);
        assert_eq!(t.poles.positions.len(), 48, "pole and arm");
        assert_eq!(t.heads.positions.len(), 24);
        let min_y = t
            .poles
            .positions
            .iter()
            .map(|p| p[1])
            .fold(f32::MAX, f32::min);
        assert!((min_y - 499.4).abs() < 0.05, "{min_y}");
        // The head hangs over the road (towards -x), at the lamp height.
        let head = t.spots[0];
        assert!((head[0] - 8.5).abs() < 1e-4 && head[1] > 505.0, "{head:?}");
        // Faces point outwards.
        for (k, n) in t.heads.normals.iter().enumerate().step_by(4) {
            let p = Vec3::from(t.heads.positions[k]);
            let c = Vec3::new(8.5, 499.5 + 5.98, 20.0);
            assert!((p - c).dot(Vec3::from(*n)) > 0.0);
        }
    }

    #[test]
    fn the_nearest_lamps_get_the_lights() {
        let spots: Vec<Vec3> = (0..50)
            .map(|i| Vec3::new(i as f32 * 30.0, 6.0, 0.0))
            .collect();
        let near = nearest_spots(&spots, Vec3::new(400.0, 2.0, 0.0), 4, 220.0);
        assert_eq!(near.len(), 4);
        assert!(near.iter().all(|p| (p.x - 400.0).abs() <= 50.0), "{near:?}");
        assert!(nearest_spots(&spots, Vec3::new(0.0, 0.0, 5000.0), 4, 220.0).is_empty());
    }
}

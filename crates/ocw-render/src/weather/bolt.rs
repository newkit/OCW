//! The lightning channel (spec §14 Phase 9, S4): for a strike within
//! [`BOLT_RANGE_M`] its jagged channel (`ocw_weather::lightning::bolt`)
//! from the cloud base to the ground, as crossed glowing ribbons, while the
//! flash lasts. The flash's light itself is the sky's (`sky.rs`).

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;

use super::{WeatherFx, clouds};
use crate::RideView;

/// Strikes farther away show no channel (m): it would be hidden in the
/// rain and haze anyway, only the flash in the clouds is seen.
pub const BOLT_RANGE_M: f32 = 9000.0;
/// The channel shows while the flash is brighter than this.
const SHOW_FLASH: f32 = 0.03;

/// Marks the channel.
#[derive(Component)]
pub struct Bolt(pub u32);

/// The channel's mesh: every segment of `lines` as two crossed quads of
/// `width` (the main channel) or half of it (the branches).
pub fn bolt_mesh(lines: &[Vec<[f32; 3]>], width: f32) -> Mesh {
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    for (k, line) in lines.iter().enumerate() {
        let w = if k == 0 { width } else { width * 0.45 };
        for seg in line.windows(2) {
            let (a, b) = (Vec3::from(seg[0]), Vec3::from(seg[1]));
            for side in [Vec3::X, Vec3::Z] {
                let s = side * (w * 0.5);
                let base = positions.len() as u32;
                for p in [a - s, a + s, b + s, b - s] {
                    positions.push(p.to_array());
                    normals.push(side.cross(Vec3::Y).to_array());
                }
                indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
            }
        }
    }
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_indices(Indices::U32(indices));
    m
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub(crate) fn update(
    mut commands: Commands,
    view: Option<Res<RideView>>,
    fx: Res<WeatherFx>,
    ground: Option<Res<crate::Ground>>,
    cloud: Res<clouds::CloudState>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut material: Local<Option<Handle<StandardMaterial>>>,
    mut bolts: Query<(Entity, &Bolt, &mut Visibility)>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let Some(view) = view else { return };
    let now = view.elapsed_s;
    // The brightest strike within range whose flash is on now.
    let on = view.weather.as_ref().filter(|_| fx.bolts).and_then(|w| {
        w.strikes
            .iter()
            .filter(|s| s.strike.distance_m <= BOLT_RANGE_M)
            .map(|s| {
                let f = ocw_weather::lightning::flash((now - s.flash_at_s) as f32, &s.strike);
                (s, f)
            })
            .filter(|(_, f)| *f > SHOW_FLASH)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(s, _)| (*s, w.state))
    });
    let mut have = None;
    for (e, b, mut vis) in &mut bolts {
        match on {
            Some((s, _)) if s.strike.id == b.0 => {
                have = Some(e);
                if *vis != Visibility::Visible {
                    *vis = Visibility::Visible;
                }
            }
            _ => commands.entity(e).despawn(),
        }
    }
    let (Some((s, state)), None) = (on, have) else {
        return;
    };
    let Some(rider) = clouds::rider_position(&view) else {
        return;
    };
    let o = s.strike.offset();
    let (x, z) = (rider.x + o[0], rider.z + o[1]);
    let foot = ground
        .as_ref()
        .and_then(|g| (g.0)(f64::from(x), f64::from(z)))
        .map_or(rider.y, |h| h as f32);
    let ground_y = cloud.ground_y.unwrap_or(rider.y);
    let base = (ground_y + clouds::layer_shape(&state).0).max(foot + 300.0);
    let lines = ocw_weather::lightning::bolt(s.strike.seed, base - foot);
    // Wider far away, so a distant channel is still a line on the screen.
    let width = 1.5 + s.strike.distance_m * 0.0012;
    let m = material
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: LinearRgba::rgb(9.0, 9.5, 12.0).into(),
                unlit: true,
                fog_enabled: false,
                alpha_mode: AlphaMode::Add,
                ..default()
            })
        })
        .clone();
    commands.spawn((
        Bolt(s.strike.id),
        Mesh3d(meshes.add(bolt_mesh(&lines, width))),
        MeshMaterial3d(m),
        Transform::from_xyz(x, foot, z),
        Visibility::Visible,
        NoFrustumCulling,
        NotShadowCaster,
        NotShadowReceiver,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_channel_mesh_has_two_quads_per_segment() {
        let lines = ocw_weather::lightning::bolt(7, 800.0);
        let segments: usize = lines.iter().map(|l| l.len() - 1).sum();
        let m = bolt_mesh(&lines, 2.0);
        assert_eq!(m.count_vertices(), segments * 8);
        let Some(Indices::U32(idx)) = m.indices() else {
            panic!()
        };
        assert_eq!(idx.len(), segments * 12);
    }
}

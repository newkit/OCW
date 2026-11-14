//! Spray behind the wheels on wet roads (spec §14 Phase 9, S4: "spray
//! behind riders in heavy rain"): a small plume of mist puffs at every
//! rider's rear wheel (`spray.wesl`), its size by how wet the road is and
//! how fast the rider goes ([`amount`]). One mesh and one material for all
//! riders (batched).

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPipeline, MaterialPipelineKey};
use bevy::prelude::*;
use bevy::render::render_resource::{
    AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
};
use bevy::shader::ShaderRef;

use super::{SceneLight, WeatherFx};
use crate::RideView;
use crate::riders::{Avatar, GHOST_ID};

/// Puffs per plume.
pub const PUFFS: u32 = 48;

#[derive(Debug, Clone, Copy, PartialEq, Default, ShaderType)]
pub struct SprayUniform {
    /// rgb: colour (before exposure), w: opacity.
    pub colour: Vec4,
    /// x: time (s).
    pub misc: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct SprayMaterial {
    #[uniform(0)]
    pub spray: SprayUniform,
}

impl Material for SprayMaterial {
    fn vertex_shader() -> ShaderRef {
        shader()
    }
    fn fragment_shader() -> ShaderRef {
        shader()
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        let vertex = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
        ])?;
        descriptor.vertex.buffers = vec![vertex];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(AssetPath::from_path_buf(embedded_path!("spray.wesl")).with_source("embedded"))
}

/// A rider's plume (a child of the avatar).
#[derive(Component)]
pub struct Spray;

/// The avatar has its plume.
#[derive(Component)]
pub struct HasSpray;

/// How much spray a rider throws up (0–1): only on a wet road, growing
/// with the speed from 3 m/s (~11 km/h) to 12 m/s (~43 km/h), more with
/// standing water.
pub fn amount(wetness: f32, puddles: f32, speed_mps: f32) -> f32 {
    let speed = ((speed_mps - 3.0) / 9.0).clamp(0.0, 1.0);
    (wetness.clamp(0.0, 1.0).powi(2) * speed * (0.55 + 0.45 * puddles.clamp(0.0, 1.0)))
        .clamp(0.0, 1.0)
}

/// The plume mesh: [`PUFFS`] seeded quads.
pub fn plume_mesh() -> Mesh {
    let mut seeds = Vec::new();
    let mut corners = Vec::new();
    let mut indices = Vec::new();
    let r = |i: u32, k: u64| ocw_weather::lightning::unit(0x5B4A, u64::from(i), k) as f32;
    for i in 0..PUFFS {
        let s = [
            i as f32 / PUFFS as f32 + r(i, 1) / PUFFS as f32,
            r(i, 2),
            r(i, 3),
        ];
        let b = i * 4;
        for c in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            seeds.push(s);
            corners.push(c);
        }
        indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, seeds);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, corners);
    m.insert_indices(Indices::U32(indices));
    m
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub(crate) fn update(
    mut commands: Commands,
    view: Option<Res<RideView>>,
    fx: Res<WeatherFx>,
    light: Res<SceneLight>,
    time: Res<Time>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<SprayMaterial>>>,
    mut cache: Local<Option<(Handle<Mesh>, Handle<SprayMaterial>)>>,
    new_avatars: Query<(Entity, &Avatar), Without<HasSpray>>,
    avatars: Query<&Avatar>,
    mut plumes: Query<(&ChildOf, &mut Transform, &mut Visibility), With<Spray>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let w = view.as_ref().and_then(|v| v.weather.as_ref());
    let wet = w.map_or(0.0, |w| w.ground.wetness());
    if wet <= 0.0 && cache.is_none() {
        return;
    }
    let (mesh, material) = cache
        .get_or_insert_with(|| {
            (
                meshes.add(plume_mesh()),
                materials.add(SprayMaterial {
                    spray: SprayUniform::default(),
                }),
            )
        })
        .clone();
    for (e, a) in &new_avatars {
        commands.entity(e).insert(HasSpray);
        if a.id == GHOST_ID {
            continue;
        }
        commands.spawn((
            Spray,
            Mesh3d(mesh.clone()),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            Visibility::Hidden,
            NoFrustumCulling,
            NotShadowCaster,
            NotShadowReceiver,
            ChildOf(e),
        ));
    }
    let puddles = w.map_or(0.0, |w| w.ground.puddles());
    for (parent, mut t, mut vis) in &mut plumes {
        let speed = avatars
            .get(parent.parent())
            .ok()
            .and_then(|a| view.as_ref()?.riders.iter().find(|r| r.id == a.id))
            .map_or(0.0, |r| r.state.speed as f32);
        let k = if fx.spray {
            amount(wet, puddles, speed)
        } else {
            0.0
        };
        let want = if k > 0.03 {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        let s = Vec3::splat(0.4 + 0.6 * k);
        if t.scale != s {
            t.scale = s;
        }
    }
    let colour = 0.6 * light.ambient_rgb * light.ambient
        + 0.2 * light.key_rgb * light.key_lux * light.towards.y.max(0.0) / std::f32::consts::PI;
    if let Some(mut m) = materials.get_mut(&material) {
        m.spray = SprayUniform {
            colour: colour.extend(0.07 + 0.08 * puddles),
            misc: Vec4::new(time.elapsed_secs() % 3600.0, 0.0, 0.0, 0.0),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spray_needs_a_wet_road_and_speed() {
        assert_eq!(amount(0.0, 0.0, 12.0), 0.0, "dry road");
        assert_eq!(amount(1.0, 1.0, 2.0), 0.0, "walking pace");
        let wet = amount(1.0, 0.0, 10.0);
        let puddles = amount(1.0, 1.0, 10.0);
        assert!(wet > 0.3 && puddles > wet);
        assert!(amount(1.0, 1.0, 20.0) <= 1.0);
        assert!(
            amount(0.5, 0.0, 10.0) < wet / 2.0,
            "a damp road throws little"
        );
        assert_eq!(plume_mesh().count_vertices(), PUFFS as usize * 4);
    }
}

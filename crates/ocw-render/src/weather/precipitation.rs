//! Rain, drizzle, snow, sleet and hail around the camera (spec §14 Phase 9,
//! S4): one mesh of seeded quads, drawn in one call. The vertex shader
//! (`precipitation.wesl`) places every particle in a box that moves with
//! the camera – particles are fixed in the world and wrap around the box
//! as they fall and drift, so riding through the rain streams it past –,
//! stretches drops into streaks along their motion relative to the camera,
//! and collapses the quads beyond the count the intensity asks for
//! ([`ocw_weather::particles::particles`]). Snow flakes tumble, hail falls
//! fast and bright. Nothing on the CPU per particle, nothing but a uniform
//! per frame: GPU-friendly and WebGPU-compatible.

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
use ocw_weather::Precip;
use ocw_weather::particles::Particles;

use super::{FLASH_AMBIENT, SceneLight, WeatherFx, wind_vector};
use crate::RideView;
use crate::camera::FollowCamera;

/// The most particles a mesh holds.
pub const MAX_PARTICLES: u32 = 100_000;

/// Uniform of `precipitation.wesl`.
#[derive(Debug, Clone, Copy, PartialEq, Default, ShaderType)]
pub struct PrecipUniform {
    /// xyz: the particles' accumulated motion (m, wrapped to the box);
    /// w: how many are drawn.
    pub offset: Vec4,
    /// x: half box (x / z), y: half box (y), z: width, w: streak length (m).
    pub shape: Vec4,
    /// xyz: velocity relative to the camera (m/s); w: opacity.
    pub motion: Vec4,
    /// rgb: colour (linear, before exposure); w: kind (0 rain, 1 drizzle,
    /// 2 snow, 3 sleet, 4 hail).
    pub colour: Vec4,
    /// x: time (s, wrapped), y: sway (m), z: visibility (m), w: quads in
    /// the mesh.
    pub misc: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PrecipMaterial {
    #[uniform(0)]
    pub precip: PrecipUniform,
}

impl Material for PrecipMaterial {
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
        // The seed (position), the corner (uv) and the index (uv_b).
        let vertex = layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
            Mesh::ATTRIBUTE_UV_1.at_shader_location(2),
        ])?;
        descriptor.vertex.buffers = vec![vertex];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

fn shader() -> ShaderRef {
    ShaderRef::Path(
        AssetPath::from_path_buf(embedded_path!("precipitation.wesl")).with_source("embedded"),
    )
}

/// Marks the precipitation mesh.
#[derive(Component)]
pub struct Precipitation;

/// The kind's code in the shader.
pub fn kind_code(p: Precip) -> f32 {
    match p {
        Precip::None | Precip::Rain => 0.0,
        Precip::Drizzle => 1.0,
        Precip::Snow => 2.0,
        Precip::Sleet => 3.0,
        Precip::Hail => 4.0,
    }
}

/// A mesh of `n` particle quads: position = the particle's seed in the
/// unit cube, uv = the corner, uv_b = (index / n, a random number).
pub fn particle_mesh(n: u32) -> Mesh {
    let n = n.clamp(1, MAX_PARTICLES);
    let mut seeds = Vec::with_capacity(n as usize * 4);
    let mut corners = Vec::with_capacity(n as usize * 4);
    let mut extra = Vec::with_capacity(n as usize * 4);
    let mut indices = Vec::with_capacity(n as usize * 6);
    let r = |i: u32, k: u64| ocw_weather::lightning::unit(0x5EED, u64::from(i), k) as f32;
    for i in 0..n {
        let s = [r(i, 1), r(i, 2), r(i, 3)];
        let e = [i as f32 / n as f32, r(i, 4)];
        let b = i * 4;
        for c in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
            seeds.push(s);
            corners.push(c);
            extra.push(e);
        }
        indices.extend([b, b + 1, b + 2, b, b + 2, b + 3]);
    }
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, seeds);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, corners);
    m.insert_attribute(Mesh::ATTRIBUTE_UV_1, extra);
    m.insert_indices(Indices::U32(indices));
    m
}

/// The colour of a particle in this light (linear, before the exposure):
/// drops show the sky light they refract, flakes and hail are white and
/// lit like a surface.
pub fn particle_colour(kind: Precip, light: &SceneLight) -> Vec3 {
    let sun = light.key_rgb * light.key_lux * light.towards.y.max(0.0) / std::f32::consts::PI;
    let amb = light.ambient_rgb * light.ambient;
    let flash = Vec3::new(0.8, 0.85, 1.0) * FLASH_AMBIENT * light.flash;
    match kind {
        Precip::Snow | Precip::Hail | Precip::Sleet => 0.9 * (amb + 0.6 * sun) + flash,
        _ => 0.75 * amb + 0.08 * sun + flash,
    }
}

/// Wraps `x` into [0, m).
fn wrap(x: f32, m: f32) -> f32 {
    x.rem_euclid(m)
}

/// The particles' motion state.
#[derive(Default)]
pub(crate) struct PrecipState {
    offset: Vec3,
    budget: u32,
    material: Option<Handle<PrecipMaterial>>,
    mesh: Option<Handle<Mesh>>,
    last_eye: Option<Vec3>,
    eye_velocity: Vec3,
}

/// The uniform for particles `p` moving `offset`, the camera at
/// `eye_velocity`.
#[allow(clippy::too_many_arguments)] // the uniform's inputs, one each
pub fn uniform(
    p: &Particles,
    wind: Vec2,
    offset: Vec3,
    eye_velocity: Vec3,
    light: &SceneLight,
    visibility_m: f32,
    time_s: f32,
    quads: u32,
) -> PrecipUniform {
    let v = Vec3::new(wind.x * p.drift, -p.fall_mps, wind.y * p.drift);
    PrecipUniform {
        offset: offset.extend(p.count as f32),
        shape: Vec4::new(p.half_box[0], p.half_box[1], p.width_m, p.streak_m),
        motion: (v - eye_velocity).extend(p.opacity),
        colour: particle_colour(p.kind, light).extend(kind_code(p.kind)),
        misc: Vec4::new(time_s % 3600.0, p.sway_m, visibility_m, quads as f32),
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub(crate) fn update(
    mut commands: Commands,
    view: Option<Res<RideView>>,
    fx: Res<WeatherFx>,
    light: Res<SceneLight>,
    time: Res<Time>,
    mut state: Local<PrecipState>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<PrecipMaterial>>>,
    cameras: Query<&GlobalTransform, With<FollowCamera>>,
    mut drawn: Query<(&mut Visibility, &mut Mesh3d), With<Precipitation>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let w = view.as_ref().and_then(|v| v.weather.as_ref());
    let p = w.and_then(|w| {
        ocw_weather::particles::particles(w.state.precip, w.state.precip_mm_h, fx.particles)
    });
    let dt = time.delta_secs().max(1e-4);
    let eye = cameras.iter().next().map(GlobalTransform::translation);
    if let Some(eye) = eye {
        if let Some(last) = state.last_eye {
            let v = (eye - last) / dt;
            // A camera cut is no motion.
            let v = if v.length() > 80.0 { Vec3::ZERO } else { v };
            let ev = state.eye_velocity;
            state.eye_velocity = ev + (v - ev) * (1.0 - (-dt / 0.15).exp());
        }
        state.last_eye = Some(eye);
    }
    let show = p.is_some();
    if show && (state.material.is_none() || state.budget != fx.particles) {
        let mesh = meshes.add(particle_mesh(fx.particles));
        state.budget = fx.particles;
        if let Some(old) = state.mesh.replace(mesh.clone()) {
            meshes.remove(&old);
        }
        if state.material.is_none() {
            let m = materials.add(PrecipMaterial {
                precip: PrecipUniform::default(),
            });
            commands.spawn((
                Precipitation,
                Mesh3d(mesh),
                MeshMaterial3d(m.clone()),
                Transform::default(),
                Visibility::Hidden,
                NoFrustumCulling,
                NotShadowCaster,
                NotShadowReceiver,
            ));
            state.material = Some(m);
        } else {
            for (_, mut m) in &mut drawn {
                m.0 = mesh.clone();
            }
        }
    }
    for (mut vis, _) in &mut drawn {
        let want = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
    }
    let (Some(p), Some(w), Some(h)) = (p, w, state.material.clone()) else {
        return;
    };
    let wind = wind_vector(w);
    let step = Vec3::new(wind.x * p.drift, -p.fall_mps, wind.y * p.drift) * dt;
    let box_xz = 2.0 * p.half_box[0];
    let box_y = 2.0 * p.half_box[1];
    state.offset = Vec3::new(
        wrap(state.offset.x + step.x, box_xz),
        wrap(state.offset.y + step.y, box_y),
        wrap(state.offset.z + step.z, box_xz),
    );
    let u = uniform(
        &p,
        wind,
        state.offset,
        state.eye_velocity,
        &light,
        w.state.visibility_m(),
        time.elapsed_secs(),
        state.budget,
    );
    if let Some(mut m) = materials.get_mut(&h) {
        m.precip = u;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_mesh_holds_seeded_quads_with_their_index() {
        let m = particle_mesh(1000);
        assert_eq!(m.count_vertices(), 4000);
        let Some(Indices::U32(idx)) = m.indices() else {
            panic!()
        };
        assert_eq!(idx.len(), 6000);
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(e)) =
            m.attribute(Mesh::ATTRIBUTE_UV_1)
        else {
            panic!()
        };
        assert_eq!(e[0][0], 0.0);
        assert!((e[3996][0] - 0.999).abs() < 1e-6);
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(s)) =
            m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        assert!(s.iter().all(|p| p.iter().all(|c| (0.0..1.0).contains(c))));
        assert_ne!(s[0], s[4], "every particle its own place");
    }

    #[test]
    fn streaks_follow_the_motion_relative_to_the_camera_and_flakes_are_lit() {
        let rain = ocw_weather::particles::particles(Precip::Rain, 4.0, 40_000).unwrap();
        let light = SceneLight::default();
        let still = uniform(
            &rain,
            Vec2::ZERO,
            Vec3::ZERO,
            Vec3::ZERO,
            &light,
            9000.0,
            1.0,
            40_000,
        );
        assert!(still.motion.y < -4.0 && still.motion.x == 0.0);
        // Riding at 10 m/s north (-z): the drops come towards the rider.
        let riding = uniform(
            &rain,
            Vec2::ZERO,
            Vec3::ZERO,
            Vec3::new(0.0, 0.0, -10.0),
            &light,
            9000.0,
            1.0,
            40_000,
        );
        assert!(riding.motion.z > 9.0);
        assert_eq!(riding.offset.w, rain.count as f32);
        let snow = ocw_weather::particles::particles(Precip::Snow, 2.0, 40_000).unwrap();
        let s = particle_colour(Precip::Snow, &light);
        let r = particle_colour(Precip::Rain, &light);
        assert!(
            s.length() > r.length(),
            "flakes are white, drops see-through"
        );
        let night = SceneLight {
            key_lux: 0.0,
            ambient: 30.0,
            ..light
        };
        assert!(particle_colour(Precip::Snow, &night).length() < s.length() / 10.0);
        // A flash lights them.
        let flash = SceneLight {
            flash: 1.0,
            ..night
        };
        assert!(
            particle_colour(Precip::Rain, &flash).length()
                > 10.0 * particle_colour(Precip::Rain, &night).length()
        );
        assert_eq!(kind_code(snow.kind), 2.0);
        assert_eq!(wrap(-1.0, 10.0), 9.0);
    }
}

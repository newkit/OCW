//! Clouds (spec §14 Phase 9, S4): a sky dome around the camera whose
//! fragment shader (`clouds.wesl`) intersects each view ray with the cloud
//! layer (from below its base, or from above its top on high mountains) and
//! draws layered value noise shaped per cloud type – heaps of cumulus,
//! a grey stratus deck, streaky cirrus, dark cumulonimbus – self-shadowed
//! towards the sun, fading into the haze towards the horizon, lit up by
//! lightning. The layer drifts with the wind aloft.
//!
//! Cheap: one full-screen-ish transparent draw, a few noise taps per pixel,
//! no textures and no compute (WebGPU-compatible). The same noise runs on
//! the CPU ([`density`]): the scene's sun light dims when a heap passes in
//! front of the sun ([`sun_shade`]), so the shadows of passing clouds sweep
//! over the rider. The sky behind dense fog is drawn in the fog's colour
//! (the atmosphere would show a blue sky there).

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_path};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::Material;
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use ocw_weather::{CloudType, WeatherState};

use super::{SceneLight, WeatherFx, WeatherView};
use crate::RideView;
use crate::camera::FollowCamera;

/// Radius of the dome (m): beyond the far terrain (120 km), in front of the
/// moon and planets (400 km).
pub const DOME_RADIUS_M: f32 = 150_000.0;
/// Sort the dome after the sky objects (moon, planets at −400 km) and
/// before everything else that is transparent.
const DOME_SORT_BIAS: f32 = -200_000.0;

/// Uniform of `clouds.wesl`.
#[derive(Debug, Clone, Copy, PartialEq, Default, ShaderType)]
pub struct CloudUniform {
    /// x: cover (0–1), y: base altitude (scene y, m), z: thickness (m),
    /// w: type (0 cumulus, 1 stratus, 2 cirrus, 3 cumulonimbus).
    pub layer: Vec4,
    /// x, z: the drift (m), y: feature scale (m), w: darkness (0–1).
    pub drift: Vec4,
    /// xyz: towards the key light; w: its illuminance above the clouds.
    pub sun: Vec4,
    /// rgb: the key light's colour (linear); w: the flash (0 – ~1).
    pub sun_rgb: Vec4,
    /// rgb: the ambient light (colour × brightness) under the clouds;
    /// w: the wind direction (rad, cirrus streaks).
    pub ambient: Vec4,
    /// rgb: the fog colour (linear); w: the visibility (m).
    pub fog: Vec4,
    /// xyz: towards the flash; w: unused.
    pub flash: Vec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct CloudMaterial {
    #[uniform(0)]
    pub clouds: CloudUniform,
}

impl Material for CloudMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("clouds.wesl")).with_source("embedded"),
        )
    }
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        DOME_SORT_BIAS
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
}

/// Marks the cloud dome.
#[derive(Component)]
pub struct CloudDome;

/// The layer's drift (m) and the ground height it floats over (scene y,
/// following the rider slowly).
#[derive(Resource, Default)]
pub struct CloudState {
    pub drift: Vec2,
    pub ground_y: Option<f32>,
    material: Option<Handle<CloudMaterial>>,
}

/// The type's code in the shader.
pub fn type_code(t: CloudType) -> f32 {
    match t {
        CloudType::Cumulus => 0.0,
        CloudType::Stratus => 1.0,
        CloudType::Cirrus => 2.0,
        CloudType::Cumulonimbus => 3.0,
    }
}

/// Base height above the ground (m) and thickness (m) the layer is drawn
/// with: cirrus is high whatever the state says; a thunder cloud towers.
pub fn layer_shape(w: &WeatherState) -> (f32, f32) {
    match w.cloud_type {
        CloudType::Cumulus => (w.cloud_base_m.max(300.0), 900.0),
        CloudType::Stratus => (w.cloud_base_m.max(60.0), 600.0),
        CloudType::Cirrus => (w.cloud_base_m.max(7000.0), 400.0),
        CloudType::Cumulonimbus => (w.cloud_base_m.max(300.0), 7000.0),
    }
}

/// Size of the largest noise features (m).
pub fn feature_scale(t: CloudType) -> f32 {
    match t {
        CloudType::Cumulus => 1600.0,
        CloudType::Stratus => 4000.0,
        CloudType::Cirrus => 3000.0,
        CloudType::Cumulonimbus => 3500.0,
    }
}

// --- the noise, the CPU twin of clouds.wesl -------------------------------

fn hash(x: i32, y: i32) -> f32 {
    let mut h = (x as u32)
        .wrapping_mul(374_761_393)
        .wrapping_add((y as u32).wrapping_mul(668_265_263));
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    (h & 0x00FF_FFFF) as f32 / 16_777_216.0
}

fn vnoise(p: Vec2) -> f32 {
    let i = p.floor();
    let f = p - i;
    let u = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let (x, y) = (i.x as i32, i.y as i32);
    let a = hash(x, y);
    let b = hash(x + 1, y);
    let c = hash(x, y + 1);
    let d = hash(x + 1, y + 1);
    let ab = a + (b - a) * u.x;
    let cd = c + (d - c) * u.x;
    ab + (cd - ab) * u.y
}

/// Fractal noise in about [0, 1] (mean 0.5).
pub fn fbm(p: Vec2, octaves: u32) -> f32 {
    let mut s = 0.0;
    let mut a = 0.5;
    let mut q = p;
    let mut norm = 0.0;
    for _ in 0..octaves {
        s += a * vnoise(q);
        norm += a;
        // Rotate and scale (no axis-aligned artefacts).
        q = Vec2::new(0.8 * q.x - 0.6 * q.y, 0.6 * q.x + 0.8 * q.y) * 2.03 + Vec2::new(17.1, 3.7);
        a *= 0.5;
    }
    s / norm
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Cloud density (0–1) of the layer at the horizontal point `q` (m, in
/// the drifting layer's frame), for `cover`, type code `kind`, feature
/// scale and wind direction (rad). Mirrors `cloud_density` in clouds.wesl.
pub fn density(q: Vec2, cover: f32, kind: f32, scale: f32, wind_rad: f32) -> f32 {
    let thr = 0.5 - (cover - 0.5) * 0.7;
    let k = kind.round() as i32;
    let (n, soft) = match k {
        1 => (fbm(q / scale, 4), 0.55),
        2 => {
            let (s, c) = wind_rad.sin_cos();
            let r = Vec2::new(c * q.x + s * q.y, -s * q.x + c * q.y);
            (fbm(Vec2::new(r.x / (scale * 5.0), r.y / scale), 4), 0.3)
        }
        3 => (fbm(q / scale, 5), 0.25),
        _ => (fbm(q / scale, 5), 0.14),
    };
    let d = smoothstep(thr - soft * 0.5, thr + soft * 0.5, n);
    if k == 2 { d * 0.7 } else { d }
}

/// How much of the direct sun reaches the rider at `rider` (scene m) under
/// a broken layer: 1 in a gap, low behind a heap (the cloud's shadow).
/// Full or no cover leave it to the average of `cloud_light`.
pub fn sun_shade(
    w: &WeatherState,
    rider: Vec3,
    ground_y: f32,
    towards_sun: Vec3,
    drift: Vec2,
) -> f32 {
    if towards_sun.y < 0.05 || !matches!(w.cloud_type, CloudType::Cumulus | CloudType::Cumulonimbus)
    {
        return 1.0;
    }
    let (base, _) = layer_shape(w);
    let base_y = ground_y + base;
    let up = (base_y - rider.y).max(0.0);
    let p = Vec2::new(rider.x, rider.z)
        + Vec2::new(towards_sun.x, towards_sun.z) * (up / towards_sun.y);
    let d = density(
        p - drift,
        w.cloud_cover,
        type_code(w.cloud_type),
        feature_scale(w.cloud_type),
        0.0,
    );
    1.0 - d * w.cloud_type.opacity()
}

/// The visibility (m) inside the cloud layer: riding into low cloud on a
/// climb is fog (spec 9E); `None` below or above it.
pub fn in_cloud_visibility(w: &WeatherState, height_above_base_m: f32) -> Option<f32> {
    let (_, thick) = layer_shape(w);
    if w.cloud_cover < 0.6 || !(0.0..=thick).contains(&height_above_base_m) {
        return None;
    }
    Some(60.0 + (1.0 - w.cloud_cover) * 2500.0)
}

/// The dome: a sphere seen from inside (radius 1, scaled).
pub fn dome_mesh() -> Mesh {
    let (rings, sectors) = (24u32, 48u32);
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    for r in 0..=rings {
        let theta = std::f32::consts::PI * r as f32 / rings as f32;
        for s in 0..=sectors {
            let phi = std::f32::consts::TAU * s as f32 / sectors as f32;
            let p = Vec3::new(
                theta.sin() * phi.cos(),
                theta.cos(),
                theta.sin() * phi.sin(),
            );
            positions.push(p.to_array());
            normals.push((-p).to_array());
        }
    }
    let mut indices = Vec::new();
    let row = sectors + 1;
    for r in 0..rings {
        for s in 0..sectors {
            let a = r * row + s;
            let b = a + row;
            // Wound to face inwards.
            indices.extend([a, b, a + 1, a + 1, b, b + 1]);
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

/// The local rider's position (scene m), if riding.
pub(crate) fn rider_position(view: &RideView) -> Option<Vec3> {
    let l = view.local()?;
    let p = view.track(l.id).sample(l.state.distance).position;
    Some(crate::to_vec3(p))
}

/// The uniform for weather `w` in the light `light`.
pub fn uniform(w: &WeatherView, light: &SceneLight, ground_y: f32, drift: Vec2) -> CloudUniform {
    let s = &w.state;
    let (base, thick) = layer_shape(s);
    let cl = ocw_weather::light::cloud_light(s);
    let wind_rad = {
        let v = s.wind_velocity(1.0);
        v[1].atan2(v[0])
    };
    CloudUniform {
        layer: Vec4::new(
            s.cloud_cover,
            ground_y + base,
            thick,
            type_code(s.cloud_type),
        ),
        drift: Vec4::new(drift.x, feature_scale(s.cloud_type), drift.y, cl.darkness),
        sun: light.towards.extend(light.key_lux),
        sun_rgb: light.key_rgb.extend(light.flash),
        ambient: (light.ambient_rgb * light.ambient).extend(wind_rad),
        fog: light.fog.extend(s.visibility_m()),
        flash: light.flash_towards.extend(0.0),
    }
}

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
pub(crate) fn update(
    mut commands: Commands,
    view: Option<Res<RideView>>,
    fx: Res<WeatherFx>,
    light: Res<SceneLight>,
    time: Res<Time>,
    mut state: ResMut<CloudState>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<CloudMaterial>>>,
    cameras: Query<&GlobalTransform, With<FollowCamera>>,
    mut domes: Query<(&mut Transform, &mut Visibility), With<CloudDome>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let w = view.as_ref().and_then(|v| v.weather.as_ref());
    let show = fx.clouds && w.is_some();
    let dt = time.delta_secs();
    if let Some(w) = w {
        // The layer drifts with the wind aloft (stronger than at the
        // ground).
        let v = w.state.wind_velocity(w.state.wind_mps * 1.8 + 2.0);
        state.drift += Vec2::new(v[0], v[1]) * dt;
        state.drift = Vec2::new(state.drift.x % 1.0e6, state.drift.y % 1.0e6);
        if let Some(p) = view.as_deref().and_then(rider_position) {
            let g = state.ground_y.get_or_insert(p.y);
            *g += (p.y - *g) * (1.0 - (-dt / 30.0).exp());
        }
    }
    if state.material.is_none() && show {
        let handle = materials.add(CloudMaterial {
            clouds: CloudUniform::default(),
        });
        commands.spawn((
            CloudDome,
            Mesh3d(meshes.add(dome_mesh())),
            MeshMaterial3d(handle.clone()),
            Transform::from_scale(Vec3::splat(DOME_RADIUS_M)),
            Visibility::Hidden,
            NoFrustumCulling,
            NotShadowCaster,
            NotShadowReceiver,
        ));
        state.material = Some(handle);
    }
    let eye = cameras.iter().next().map(GlobalTransform::translation);
    for (mut t, mut vis) in &mut domes {
        let want = if show {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        if *vis != want {
            *vis = want;
        }
        if let Some(eye) = eye {
            t.translation = eye;
        }
    }
    let (Some(w), Some(h)) = (w, state.material.clone()) else {
        return;
    };
    if !show {
        return;
    }
    let u = uniform(w, &light, state.ground_y.unwrap_or(0.0), state.drift);
    if let Some(mut m) = materials.get_mut(&h)
        && m.clouds != u
    {
        m.clouds = u;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_weather::presets::preset;

    #[test]
    fn the_noise_is_smooth_and_the_cover_sets_the_share() {
        // Mean about 0.5, within [0, 1].
        let mut sum = 0.0;
        let n = 4000;
        for i in 0..n {
            let p = Vec2::new(i as f32 * 0.731, (i * 7 % 101) as f32 * 1.37);
            let v = fbm(p, 5);
            assert!((0.0..=1.0).contains(&v));
            sum += v;
        }
        assert!((sum / n as f32 - 0.5).abs() < 0.05);
        // The share of the sky covered grows with the cover.
        let share = |cover: f32, kind: f32| {
            let mut c = 0.0;
            for i in 0..80 {
                for j in 0..80 {
                    let q = Vec2::new(i as f32 * 211.0, j as f32 * 197.0);
                    c += density(q, cover, kind, 1600.0, 0.0);
                }
            }
            c / 6400.0
        };
        let (few, half, full) = (share(0.1, 0.0), share(0.5, 0.0), share(1.0, 0.0));
        assert!(
            few < 0.1 && (0.3..0.7).contains(&half) && full > 0.9,
            "{few} {half} {full}"
        );
        assert!(share(1.0, 1.0) > 0.95, "a closed stratus deck");
        assert!(share(0.05, 0.0) < 0.03, "clear: hardly a cloud");
    }

    #[test]
    fn passing_heaps_shade_the_sun_and_low_cloud_is_fog() {
        let fair = preset("cloudy").unwrap();
        let sun = Vec3::new(0.3, 0.8, 0.5).normalize();
        // Along a line the sun goes in and out.
        let shades: Vec<f32> = (0..400)
            .map(|i| {
                sun_shade(
                    &fair,
                    Vec3::new(i as f32 * 50.0, 0.0, 0.0),
                    0.0,
                    sun,
                    Vec2::ZERO,
                )
            })
            .collect();
        assert!(shades.iter().any(|&s| s > 0.95), "gaps");
        assert!(shades.iter().any(|&s| s < 0.4), "heaps");
        // Overcast and low sun: left to the average.
        assert_eq!(
            sun_shade(
                &preset("overcast").unwrap(),
                Vec3::ZERO,
                0.0,
                sun,
                Vec2::ZERO
            ),
            1.0
        );
        assert_eq!(sun_shade(&fair, Vec3::ZERO, 0.0, Vec3::X, Vec2::ZERO), 1.0);
        // Riding into the stratus deck on a climb: fog.
        let overcast = preset("overcast").unwrap();
        assert!(in_cloud_visibility(&overcast, 100.0).unwrap() < 100.0);
        assert!(in_cloud_visibility(&overcast, -10.0).is_none());
        assert!(in_cloud_visibility(&preset("fair").unwrap(), 100.0).is_none());
        // Cirrus is high.
        let cirrus = WeatherState {
            cloud_type: CloudType::Cirrus,
            ..fair
        };
        assert!(layer_shape(&cirrus).0 >= 7000.0);
    }

    #[test]
    fn the_dome_faces_inwards() {
        let m = dome_mesh();
        let Some(bevy::mesh::VertexAttributeValues::Float32x3(p)) =
            m.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!()
        };
        let Some(Indices::U32(idx)) = m.indices() else {
            panic!()
        };
        // Every triangle's normal (by winding) points to the centre.
        for t in idx.chunks(3).take(500) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| Vec3::from(p[i as usize]));
            let n = (b - a).cross(c - a);
            if n.length() > 1e-6 {
                assert!(n.dot((a + b + c) / 3.0) < 0.0);
            }
        }
    }
}

//! The night sky without stars (spec §14 Phase 9, S2; decision Q7): the
//! moon as a disc with its phase, bright limb and earthshine, the planets
//! as points of light that fade in with the twilight, the Milky Way as a
//! procedural band turning with the sidereal time, and the sky glow of
//! nearby towns. The astronomy and the images are pure (`ocw_sky::moon`,
//! `planets`, `milky_way`, `night`, `moon_disc`); this module places them.
//!
//! Everything is drawn additively, unlit, without fog, far beyond the far
//! terrain (the camera's projection is infinite, so they are not clipped;
//! the terrain hides them behind mountains), centred on the camera: the
//! sky at infinity. The moonlight itself is the scene's one directional
//! light at night (`sky.rs`).

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::{NotShadowCaster, NotShadowReceiver};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use ocw_sky::moon_disc::{self, DiscParams};

use crate::RideView;
use crate::camera::FollowCamera;
use crate::sky::sun_direction;

/// Distances of the sky objects from the camera (m): beyond the far
/// terrain (120 km).
const MOON_DISTANCE_M: f32 = 400_000.0;
const PLANET_DISTANCE_M: f32 = 410_000.0;
const SKY_DISTANCE_M: f32 = 420_000.0;
/// The moon is drawn larger than its 0.5°, as the eye perceives it.
pub const MOON_SCALE: f32 = 2.0;
/// Moon brightness (linear, before tonemapping) at night and by day.
const MOON_NIGHT: f32 = 1.5;
const MOON_DAY: f32 = 0.55;
/// Planet sprite size (°) and gain.
const PLANET_SIZE_DEG: f32 = 0.7;
const PLANET_GAIN: f32 = 2.5;
/// Milky Way brightness at full visibility (linear, additive).
pub const MILKY_WAY_GAIN: f32 = 0.045;
/// Sky glow at full light pollution (linear, additive).
pub const GLOW_GAIN: f32 = 0.06;
/// Moon disc texture size.
const MOON_TEXTURE: u32 = 256;

/// Light pollution at the rider (0–1), set by the package world
/// (`street_lamps::update_light_pollution`); 0 without one.
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq)]
pub struct LightPollution(pub f32);

#[derive(Component)]
pub struct MoonDisc;

#[derive(Component)]
pub struct PlanetDot(pub usize);

#[derive(Component)]
pub struct MilkyWay;

#[derive(Component)]
pub struct SkyGlow;

/// Handles of the sky objects' assets.
#[derive(Resource)]
struct NightSkyAssets {
    moon_image: Handle<Image>,
    moon: Handle<StandardMaterial>,
    planets: Vec<Handle<StandardMaterial>>,
    milky_way: Handle<StandardMaterial>,
    glow: Handle<StandardMaterial>,
    /// The parameters the moon image was drawn for.
    drawn: Option<DiscParams>,
}

pub struct NightSkyPlugin;

impl Plugin for NightSkyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<LightPollution>()
            .add_systems(Update, (spawn_night_sky, place_night_sky).chain());
    }
}

fn sky_material(base: Color, texture: Option<Handle<Image>>) -> StandardMaterial {
    StandardMaterial {
        base_color: base,
        base_color_texture: texture,
        unlit: true,
        alpha_mode: AlphaMode::Add,
        fog_enabled: false,
        double_sided: true,
        cull_mode: None,
        ..default()
    }
}

fn rgba_image(size: u32, pixels: Vec<[u8; 4]>) -> Image {
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        pixels.into_iter().flatten().collect(),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// The Milky Way band as a mesh in equatorial (J2000) coordinates on the
/// unit sphere: galactic latitude ±32°, vertex colours the band's
/// brightness (cool white, warmer towards the centre).
pub fn milky_way_mesh() -> Mesh {
    let (nl, nb) = (360usize, 65usize);
    let mut positions = Vec::with_capacity(nl * nb);
    let mut colors = Vec::with_capacity(nl * nb);
    for i in 0..nl {
        let l = i as f64 * 360.0 / nl as f64;
        for j in 0..nb {
            let b = -32.0 + 64.0 * j as f64 / (nb - 1) as f64;
            let v = ocw_sky::milky_way::galactic_vector(l, b);
            positions.push([v[0] as f32, v[1] as f32, v[2] as f32]);
            // Fading to nothing at the mesh edge (no seam).
            let edge = 1.0 - ((b.abs() - 24.0) / 8.0).clamp(0.0, 1.0) as f32;
            let s = ocw_sky::milky_way::brightness(l, b) as f32 * edge;
            let warm = (-(ocw_sky::astro::wrap180(l) / 40.0).powi(2)).exp() as f32;
            colors.push([
                s * (0.92 + 0.08 * warm),
                s * 0.94,
                s * (1.0 - 0.12 * warm),
                1.0,
            ]);
        }
    }
    let mut indices = Vec::with_capacity(nl * (nb - 1) * 6);
    for i in 0..nl {
        let i1 = (i + 1) % nl;
        for j in 0..nb - 1 {
            let (a, b, c, d) = (i * nb + j, i1 * nb + j, i1 * nb + j + 1, i * nb + j + 1);
            indices.extend([a, b, c, a, c, d].map(|x| x as u32));
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    m.insert_indices(Indices::U32(indices));
    m
}

/// The sky glow dome (scene axes, unit sphere): vertex colours the glow's
/// shape above the horizon ([`ocw_sky::night::sky_glow`] at pollution 1).
pub fn glow_mesh() -> Mesh {
    let rows = [
        -8.0, -3.0, 0.0, 2.0, 4.0, 7.0, 10.0, 14.0, 19.0, 26.0, 35.0, 50.0, 70.0, 90.0,
    ];
    let n = 72usize;
    let mut positions = Vec::new();
    let mut colors = Vec::new();
    for &el in &rows {
        let g = ocw_sky::night::sky_glow(1.0, el, 1.0) as f32;
        let c = ocw_sky::night::GLOW_RGB;
        for i in 0..n {
            let az = i as f64 * 360.0 / n as f64;
            let d = sun_direction(az, el);
            positions.push(d.to_array());
            colors.push([c[0] * g, c[1] * g, c[2] * g, 1.0]);
        }
    }
    let mut indices = Vec::new();
    for r in 0..rows.len() - 1 {
        for i in 0..n {
            let i1 = (i + 1) % n;
            let (a, b, c, d) = (r * n + i, r * n + i1, (r + 1) * n + i1, (r + 1) * n + i);
            indices.extend([a, b, c, a, c, d].map(|x| x as u32));
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    m.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    m.insert_indices(Indices::U32(indices));
    m
}

fn spawn_night_sky(
    mut commands: Commands,
    existing: Option<Res<NightSkyAssets>>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
) {
    if existing.is_some() {
        return;
    }
    let (Some(mut meshes), Some(mut materials), Some(mut images)) = (meshes, materials, images)
    else {
        return;
    };
    let moon_image = images.add(rgba_image(
        MOON_TEXTURE,
        moon_disc::disc(
            MOON_TEXTURE,
            DiscParams {
                phase_angle_deg: 0.0,
                limb_dir_deg: 0.0,
                earthshine: 0.0,
                day: 0.0,
            },
        ),
    ));
    let sprite = images.add(rgba_image(32, moon_disc::point_sprite(32)));
    let moon = materials.add(sky_material(Color::BLACK, Some(moon_image.clone())));
    let quad = meshes.add(Rectangle::new(1.0, 1.0));
    let common = || {
        (
            Visibility::Hidden,
            NotShadowCaster,
            NotShadowReceiver,
            NoFrustumCulling,
        )
    };
    commands.spawn((
        MoonDisc,
        Mesh3d(quad.clone()),
        MeshMaterial3d(moon.clone()),
        Transform::default(),
        common(),
    ));
    let mut planets = Vec::new();
    for i in 0..ocw_sky::Planet::ALL.len() {
        let m = materials.add(sky_material(Color::BLACK, Some(sprite.clone())));
        commands.spawn((
            PlanetDot(i),
            Mesh3d(quad.clone()),
            MeshMaterial3d(m.clone()),
            Transform::default(),
            common(),
        ));
        planets.push(m);
    }
    let milky_way = materials.add(sky_material(Color::BLACK, None));
    commands.spawn((
        MilkyWay,
        Mesh3d(meshes.add(milky_way_mesh())),
        MeshMaterial3d(milky_way.clone()),
        Transform::default(),
        common(),
    ));
    let glow = materials.add(sky_material(Color::BLACK, None));
    commands.spawn((
        SkyGlow,
        Mesh3d(meshes.add(glow_mesh())),
        MeshMaterial3d(glow.clone()),
        Transform::default(),
        common(),
    ));
    commands.insert_resource(NightSkyAssets {
        moon_image,
        moon,
        planets,
        milky_way,
        glow,
        drawn: None,
    });
}

/// The rotation of the Milky Way's equatorial frame into the scene (x
/// east, y up, z south) at local sidereal time `lst` and latitude `lat`.
pub fn equatorial_rotation(lst: f64, lat: f64) -> Quat {
    let m = ocw_sky::astro::equatorial_to_enu(lst, lat);
    // Columns: where the equatorial axes go, as (east, up, -north).
    let col = |k: usize| Vec3::new(m[0][k] as f32, m[2][k] as f32, -(m[1][k] as f32));
    Quat::from_mat3(&Mat3::from_cols(col(0), col(1), col(2))).normalize()
}

/// Rotation of a quad (its +z towards the camera) at direction `towards`
/// from the camera, its +y along `up` projected on the sky.
fn facing(towards: Vec3, up: Vec3) -> Quat {
    let back = -towards.normalize();
    let mut up = (up - back * up.dot(back)).normalize_or_zero();
    if up == Vec3::ZERO {
        up = (Vec3::Y - back * Vec3::Y.dot(back)).normalize_or(Vec3::Z);
    }
    let right = up.cross(back);
    Quat::from_mat3(&Mat3::from_cols(right, up, back)).normalize()
}

/// The direction of the bright limb in the moon's image (°, 0 right, 90
/// up) for a quad facing the camera at `towards` with `up`, lit from
/// `sun` (all scene directions).
pub fn limb_direction(towards: Vec3, up: Vec3, sun: Vec3) -> f32 {
    let rot = facing(towards, up);
    let (right, up) = (rot * Vec3::X, rot * Vec3::Y);
    sun.dot(up).atan2(sun.dot(right)).to_degrees()
}

fn set_colour(materials: &mut Assets<StandardMaterial>, h: &Handle<StandardMaterial>, c: Color) {
    if let Some(m) = materials.get(h)
        && m.base_color == c
    {
        return;
    }
    if let Some(mut m) = materials.get_mut(h) {
        m.base_color = c;
    }
}

type SkyObjects<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut Visibility,
        Has<MoonDisc>,
        Option<&'static PlanetDot>,
        Has<MilkyWay>,
    ),
    (
        Without<FollowCamera>,
        Or<(
            With<MoonDisc>,
            With<PlanetDot>,
            With<MilkyWay>,
            With<SkyGlow>,
        )>,
    ),
>;

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn place_night_sky(
    view: Option<Res<RideView>>,
    pollution: Res<LightPollution>,
    assets: Option<ResMut<NightSkyAssets>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    images: Option<ResMut<Assets<Image>>>,
    cameras: Query<&GlobalTransform, With<FollowCamera>>,
    mut objects: SkyObjects,
) {
    let (Some(mut assets), Some(mut materials), Some(mut images)) = (assets, materials, images)
    else {
        return;
    };
    let sky = view.as_ref().and_then(|v| v.sky.as_ref()).map(|s| &s.sky);
    let camera = cameras.iter().next().map(GlobalTransform::translation);
    let (Some(sky), Some(camera)) = (sky, camera) else {
        for (_, mut v, ..) in &mut objects {
            v.set_if_neq(Visibility::Hidden);
        }
        return;
    };
    let pollution = f64::from(pollution.0);
    let (lat, _) = sky.place;
    let sun = sun_direction(sky.sun.azimuth_deg, sky.sun.elevation_deg);
    let moon = &sky.moon;
    let moon_dir = sun_direction(moon.azimuth_deg, moon.elevation_deg);
    let pole = Vec3::new(
        0.0,
        lat.to_radians().sin() as f32,
        -(lat.to_radians().cos() as f32),
    );
    let day = 1.0 - f64::from(sky.light.darkness);
    let limit = ocw_sky::night::limiting_magnitude(
        sky.sun.elevation_deg,
        moon.elevation_deg,
        moon.illuminated,
        pollution,
    );
    // The moon's image for its phase and the bright limb's direction.
    let params = DiscParams {
        phase_angle_deg: moon.phase_angle_deg,
        limb_dir_deg: f64::from(limb_direction(moon_dir, pole, sun)),
        earthshine: DiscParams::earthshine_for(moon.phase_angle_deg),
        day,
    }
    .quantised();
    if assets.drawn != Some(params) {
        if let Some(mut img) = images.get_mut(&assets.moon_image) {
            *img = rgba_image(MOON_TEXTURE, moon_disc::disc(MOON_TEXTURE, params));
        }
        assets.drawn = Some(params);
        // Re-set the material so the new image is picked up.
        if let Some(mut m) = materials.get_mut(&assets.moon) {
            m.base_color_texture = Some(assets.moon_image.clone());
        }
    }
    // Reddened and dimmed near the horizon, like the sun.
    let t = ocw_sky::light::sun_transmittance(moon.elevation_deg.max(0.0));
    let z = ocw_sky::light::sun_transmittance(90.0);
    let gain = MOON_DAY + (MOON_NIGHT - MOON_DAY) * (1.0 - day as f32);
    let moon_colour = Color::linear_rgb(
        gain * (t[0] / z[0]) as f32,
        gain * (t[1] / z[1]) as f32,
        gain * (t[2] / z[2]) as f32,
    );
    set_colour(&mut materials, &assets.moon, moon_colour);
    for (i, p) in sky.planets.iter().enumerate() {
        let vis = ocw_sky::night::point_visibility(p.magnitude(), limit)
            * (p.elevation_deg > -1.0) as u8 as f64;
        let b = (ocw_sky::night::point_brightness(p.magnitude()) * vis) as f32 * PLANET_GAIN;
        let c = p.planet.colour();
        if let Some(h) = assets.planets.get(i) {
            set_colour(
                &mut materials,
                h,
                Color::linear_rgb(c[0] * b, c[1] * b, c[2] * b),
            );
        }
    }
    let mw = ocw_sky::milky_way::visibility(
        sky.sun.elevation_deg,
        moon.elevation_deg,
        moon.illuminated,
        pollution,
    ) as f32
        * MILKY_WAY_GAIN;
    // Quantised: the material is uploaded again on every change.
    let mw = (mw * 400.0).round() / 400.0;
    set_colour(
        &mut materials,
        &assets.milky_way,
        Color::linear_rgb(mw, mw, mw),
    );
    let glow = ((pollution as f32 * sky.light.darkness * GLOW_GAIN) * 400.0).round() / 400.0;
    set_colour(
        &mut materials,
        &assets.glow,
        Color::linear_rgb(glow, glow, glow),
    );
    let mw_rotation = equatorial_rotation(sky.lst_deg(), lat);
    let moon_size = (moon.diameter_deg as f32 * MOON_SCALE).to_radians() * MOON_DISTANCE_M;
    let planet_size = PLANET_SIZE_DEG.to_radians() * PLANET_DISTANCE_M;
    for (mut tf, mut vis, is_moon, planet, is_mw) in &mut objects {
        let (new, show) = if is_moon {
            (
                Transform {
                    translation: camera + moon_dir * MOON_DISTANCE_M,
                    rotation: facing(moon_dir, pole),
                    scale: Vec3::splat(moon_size),
                },
                moon.elevation_deg > -2.0,
            )
        } else if let Some(&PlanetDot(i)) = planet {
            let Some(p) = sky.planets.get(i) else {
                continue;
            };
            let d = sun_direction(p.azimuth_deg, p.elevation_deg);
            let lit = ocw_sky::night::point_visibility(p.magnitude(), limit) > 0.0;
            (
                Transform {
                    translation: camera + d * PLANET_DISTANCE_M,
                    rotation: facing(d, Vec3::Y),
                    scale: Vec3::splat(planet_size),
                },
                lit && p.elevation_deg > -1.0,
            )
        } else if is_mw {
            (
                Transform {
                    translation: camera,
                    rotation: mw_rotation,
                    scale: Vec3::splat(SKY_DISTANCE_M),
                },
                mw > 0.0,
            )
        } else {
            (
                Transform {
                    translation: camera,
                    rotation: Quat::IDENTITY,
                    scale: Vec3::splat(SKY_DISTANCE_M * 0.98),
                },
                glow > 0.0,
            )
        };
        if *tf != new {
            *tf = new;
        }
        vis.set_if_neq(if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-4
    }

    /// The Milky Way's frame: the celestial pole stands at the latitude's
    /// height in the north; the galactic centre over Tenerife on a summer
    /// night is low in the south.
    #[test]
    fn the_equatorial_frame_turns_with_the_sky() {
        let lat = 28.3;
        let r = equatorial_rotation(123.0, lat);
        let pole = r * Vec3::Z;
        assert!(close(pole, sun_direction(0.0, lat)), "{pole}");
        // The galactic centre (α 266.4°) on the meridian: LST = α.
        let eq = ocw_sky::milky_way::galactic_to_equatorial(0.0, 0.0);
        let gc = ocw_sky::milky_way::galactic_vector(0.0, 0.0);
        let gc = equatorial_rotation(eq.ra_deg, lat)
            * Vec3::new(gc[0] as f32, gc[1] as f32, gc[2] as f32);
        let want = sun_direction(180.0, 90.0 - lat + eq.dec_deg);
        assert!(close(gc, want), "{gc} vs {want}");
    }

    #[test]
    fn quads_face_the_camera_with_north_up() {
        let d = sun_direction(200.0, 30.0);
        let r = facing(d, Vec3::Y);
        assert!(close(r * Vec3::Z, -d));
        assert!((r * Vec3::Y).y > 0.5);
        // A half moon lit from the right (west, the sun set) as seen
        // facing south: the limb at 0° (right).
        let towards = sun_direction(180.0, 20.0);
        let sun = sun_direction(270.0, -5.0);
        let a = limb_direction(towards, Vec3::Y, sun);
        assert!(a.abs() < 25.0, "{a}");
        let below = limb_direction(towards, Vec3::Y, -Vec3::Y);
        assert!((below + 90.0).abs() < 1.0, "{below}");
    }

    #[test]
    fn camera_look_stands_behind_the_rider() {
        let (eye, dir) = crate::camera::sky_look_eye(Vec3::ZERO, 90.0, 0.0);
        assert!(close(dir, Vec3::X));
        assert!(close(eye, Vec3::new(-6.0, 2.2, 0.0)), "{eye}");
        let (eye, dir) = crate::camera::sky_look_eye(Vec3::ONE, 180.0, 90.0);
        assert!(close(dir, Vec3::Y) && eye.y > 3.0);
    }

    #[test]
    fn meshes_cover_the_band_and_the_dome() {
        let mw = milky_way_mesh();
        assert_eq!(mw.count_vertices(), 360 * 65);
        let glow = glow_mesh();
        assert!(glow.count_vertices() > 500);
    }

    /// Headless: with a night sky the moon, the visible planets and the
    /// Milky Way are placed around the camera; by day the Milky Way hides.
    #[test]
    fn sky_objects_follow_the_sky_state() {
        let zone = ocw_sky::Zone::get("Atlantic/Canary").unwrap();
        let place = (28.27, -16.64);
        // A moonless summer night on the Teide plateau.
        let (ms, _) = zone.instant((2026, 8, 12), (23, 30, 0)).unwrap();
        let sky = ocw_sky::Sky::at(ms, &zone, place, 0.1);
        let mut view = RideView::new(ocw_world::Route::test_loop());
        view.sky = Some(crate::sky::SkyView {
            hud: sky.hud_clock(),
            sky,
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<Image>()
            .insert_resource(view)
            .add_plugins(NightSkyPlugin);
        app.world_mut().spawn((
            FollowCamera,
            Transform::from_xyz(10.0, 2000.0, 5.0),
            GlobalTransform::from_translation(Vec3::new(10.0, 2000.0, 5.0)),
        ));
        app.update();
        app.update();
        let w = app.world_mut();
        let (mw_tf, mw_vis) = w
            .query_filtered::<(&Transform, &Visibility), With<MilkyWay>>()
            .single(w)
            .unwrap();
        assert_eq!(*mw_vis, Visibility::Inherited, "a dark sky shows it");
        assert_eq!(mw_tf.translation, Vec3::new(10.0, 2000.0, 5.0));
        let moon_vis = *w
            .query_filtered::<&Visibility, With<MoonDisc>>()
            .single(w)
            .unwrap();
        assert_eq!(moon_vis, Visibility::Hidden, "new moon below the horizon");
        let planets = w
            .query::<(&PlanetDot, &Visibility)>()
            .iter(w)
            .filter(|(_, v)| **v == Visibility::Inherited)
            .count();
        assert!(planets >= 1, "Saturn and Jupiter are up in August");
        // Noon: no Milky Way, no planets but (maybe) Venus.
        let (noon, _) = zone.instant((2026, 8, 12), (13, 30, 0)).unwrap();
        let sky = ocw_sky::Sky::at(noon, &zone, place, 0.1);
        app.world_mut().resource_mut::<RideView>().sky = Some(crate::sky::SkyView {
            hud: sky.hud_clock(),
            sky,
        });
        app.update();
        let w = app.world_mut();
        let v = *w
            .query_filtered::<&Visibility, With<MilkyWay>>()
            .single(w)
            .unwrap();
        assert_eq!(v, Visibility::Hidden);
    }
}

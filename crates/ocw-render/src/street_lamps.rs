//! Street lamps (spec §14 Phase 9, S2): poles with lamp heads along lit
//! roads (OSM `lit`, else roads in towns; placement in
//! [`ocw_world::lamps`]), drawn from their model files
//! (`artwork/3dmodels/props/lamp-street-*`, spec §14 Phase 15, 15H MX) and
//! merged with the package tiles; the heads glow
//! from dusk to dawn, and a small pool of point lights follows the camera
//! to the nearest lamps – the warm light pools on the road, cheap enough
//! for any GPU (no shadows, a dozen lights). Also the light pollution at
//! the rider for the night sky ([`crate::night_sky::LightPollution`]).

use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use ocw_world::lamps::Lamp;
use ocw_world::package::{TileKey, WorldPackage};

use ocw_assets::props::{Instance, Level, has_lod1, place_level};

use crate::RideView;
use crate::building_material::night_factor;
use crate::camera::FollowCamera;
use crate::junction_mesh::PackageJunctions;
use crate::obstacles::{LodCell, LodCells, append};
use crate::package::{Keep, MeshArrays, TileGround};

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

/// The street lamp files by the head's height (spec §14 Phase 15, 15H:
/// `props/lamp-street-<shape>-<road>`): main roads 8 m, other roads 6 m,
/// paths and tracks 4.5 m ([`ocw_world::lamps::lamp_height_m`]).
pub const LAMP_FILES: [(&str, f32); 3] = [
    ("props/lamp-street-arm-main", 8.0),
    ("props/lamp-street-arm-road", 6.0),
    ("props/lamp-street-arm-path", 4.5),
];

/// The slot of the glowing lamp heads.
pub const BULB: &str = "bulb";
/// The anchor the light comes from (just under the head).
pub const LIGHT_ANCHOR: &str = "anchor_light";

/// The lamps of a tile, ready to upload.
#[derive(Default)]
pub struct TileLamps {
    /// Poles and arms (vertex-coloured).
    pub poles: MeshArrays,
    /// The lamp heads (glowing at night).
    pub heads: MeshArrays,
    /// Where the light comes from (package metres, just under the heads).
    pub spots: Vec<[f32; 3]>,
    /// Lamps whose file has a `lod1`: near and far by cell (layers: the
    /// poles, the heads), switched at 60 m (spec 16H Q24 (a),
    /// [`crate::lod::PROP`]).
    pub lod: Vec<LodCell>,
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

/// Meshes of lamps standing at `feet` heights (package metres): each
/// lamp the file of its height ([`LAMP_FILES`]) with its arm fitted
/// (`ocw_assets::props`: the arm stretched, the head and its light moved),
/// turned towards the road; the `bulb` parts are the heads.
pub fn lamp_arrays(lamps: &[(Lamp, f32)]) -> TileLamps {
    let files: Vec<(std::sync::Arc<ocw_assets::model::Model>, f32)> = LAMP_FILES
        .iter()
        .map(|(id, h)| (ocw_assets::shipped::model(id), *h))
        .collect();
    lamp_arrays_from(&files, lamps)
}

/// [`lamp_arrays`] with these files (model, head height): a file with a
/// `lod1` goes to [`TileLamps::lod`], near and far.
pub fn lamp_arrays_from(
    files: &[(std::sync::Arc<ocw_assets::model::Model>, f32)],
    lamps: &[(Lamp, f32)],
) -> TileLamps {
    let mut out = TileLamps::default();
    let mut cells = LodCells::default();
    for (l, foot_y) in lamps {
        let Some((model, _)) = files.iter().min_by(|a, b| {
            (a.1 - l.height_m)
                .abs()
                .total_cmp(&(b.1 - l.height_m).abs())
        }) else {
            continue;
        };
        let foot = Vec3::new(l.foot[0], *foot_y, l.foot[2]);
        let yaw = (-l.toward[1]).atan2(l.toward[0]);
        let inst = Instance {
            height_m: Some(l.height_m),
            span_m: None,
            reach_m: Some(l.arm_m),
        };
        let y = *foot_y;
        let ground = |_: f32, _: f32| y;
        let p = place_level(model, &inst, foot, yaw, &ground, Level::Near);
        if has_lod1(model) {
            let far = place_level(model, &inst, foot, yaw, &ground, Level::Lod1);
            let (n, f) = cells.at(foot);
            append(&mut n[0], &p, |s| s != BULB);
            append(&mut n[1], &p, |s| s == BULB);
            append(&mut f[0], &far, |s| s != BULB);
            append(&mut f[1], &far, |s| s == BULB);
        } else {
            append(&mut out.poles, &p, |s| s != BULB);
            append(&mut out.heads, &p, |s| s == BULB);
        }
        let light = p
            .anchor(LIGHT_ANCHOR)
            .unwrap_or(foot + Vec3::Y * (l.height_m - 0.32));
        out.spots.push(light.to_array());
    }
    out.lod = cells.finish();
    out
}

/// A lamp pole's foot sinks this far (m) below the lowest drawn ground
/// round it (spec §4.6.1).
pub const POLE_SINK_M: f32 = 0.05;

/// The lamps of a package tile with the heights of their feet: on the
/// `ground` as the tile draws it beside the road (the lowest point under
/// the pole, [`POLE_SINK_M`] into it), or at the road's height where the
/// ground is far off (on bridges, along walls and cuttings). Never in a
/// road (the rule of 2026-11-22, [`ocw_world::lamps::LampSpace`]): with
/// the package's `junctions` the lamps keep off every road of the graph
/// and the junction surfaces; without, off the tile's own strips.
pub fn tile_lamps(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    ground: &TileGround,
    junctions: Option<&PackageJunctions>,
) -> TileLamps {
    let lamps: Vec<(Lamp, f32)> = placed_lamps(pkg, key, junctions)
        .into_iter()
        .filter(|l| keep.is_none_or(|f| f([f64::from(l.foot[0]), f64::from(l.foot[2])])))
        .map(|l| {
            let road = l.foot[1];
            let ground = ground
                .foot(f64::from(l.foot[0]), f64::from(l.foot[2]), 0.1)
                .map(|h| h as f32 - POLE_SINK_M)
                .filter(|h| (h - road).abs() < 1.5)
                .unwrap_or(road);
            (l, ground)
        })
        .collect();
    lamp_arrays(&lamps)
}

/// The lamps of a package tile, before they are fitted to the drawn
/// ground: the mapped ones (OSM `highway=street_lamp`, section `OBST`)
/// and the regular rows, kept out of the roads
/// ([`ocw_world::lamps::tile_lamps`]) – the same lamps the server's
/// obstacles have ([`ocw_world::lamps::package_lamps`]).
pub fn placed_lamps(
    pkg: &WorldPackage,
    key: TileKey,
    junctions: Option<&PackageJunctions>,
) -> Vec<Lamp> {
    let edges = junctions.map_or_else(Vec::new, |pj| {
        ocw_world::lamps::ring_edges(key, |k| pj.tile_edges(k))
    });
    let space = ocw_world::lamps::LampSpace::new(pkg, key, &edges, junctions.map(|pj| &pj.set));
    ocw_world::lamps::tile_lamps(pkg, key, &space)
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

    /// Lamps whose file has a `lod1` (spec 16H Q24 (a)): drawn near up to
    /// 60 m and with their `lod1` beyond, by cell, the light where it was.
    #[test]
    fn lamps_with_a_lod1_switch_at_the_props_distance() {
        let red = |m: &ocw_assets::model::Material| {
            if m.name == BULB {
                m.clone()
            } else {
                ocw_assets::model::Material {
                    name: "paint_red".into(),
                    color: [1.0, 0.0, 0.0, 1.0],
                    ..m.clone()
                }
            }
        };
        let files: Vec<_> = LAMP_FILES
            .iter()
            .map(|(id, h)| {
                let m = ocw_assets::props::with_copied_lod1(&ocw_assets::shipped::model(id), &red);
                (std::sync::Arc::new(m), *h)
            })
            .collect();
        let lamp = |x: f32| Lamp {
            foot: [x, 500.0, 20.0],
            toward: [-1.0, 0.0],
            height_m: 6.0,
            arm_m: 1.5,
        };
        let plain = lamp_arrays(&[(lamp(10.0), 499.5)]);
        let t = lamp_arrays_from(&files, &[(lamp(10.0), 499.5), (lamp(40.0), 499.5)]);
        assert!(t.poles.indices.is_empty() && t.heads.indices.is_empty());
        assert_eq!(t.lod.len(), 2, "one cell each (30 m apart)");
        assert_eq!(t.spots[0], plain.spots[0]);
        let c = &t.lod[0];
        assert_eq!(c.origin, [10.0, 499.5, 20.0]);
        // Near: the file's near model; far: its lod1 (here the same shape
        // in another paint), both relative to the cell's origin.
        let back = |m: &MeshArrays| -> Vec<[f32; 3]> {
            m.positions
                .iter()
                .map(|p| [p[0] + c.origin[0], p[1] + c.origin[1], p[2] + c.origin[2]])
                .collect()
        };
        let same = |a: Vec<[f32; 3]>, b: &[[f32; 3]]| {
            a.len() == b.len()
                && a.iter()
                    .zip(b)
                    .all(|(p, q)| Vec3::from(*p).distance(Vec3::from(*q)) < 1e-3)
        };
        assert!(same(back(&c.near[0]), &plain.poles.positions));
        assert!(same(back(&c.near[1]), &plain.heads.positions));
        assert_eq!(back(&c.far[0]).len(), plain.poles.positions.len());
        assert_ne!(
            c.far[0].colors, c.near[0].colors,
            "lod1 is drawn, not the near model"
        );
        assert_eq!(c.far[1].indices.len(), plain.heads.indices.len());
        // The ranges the tile gives them: the props' switch.
        let near = crate::lod::PROP.near_range();
        let far = crate::lod::PROP.far_range(None);
        assert_eq!(near.end_margin, 54.0..66.0);
        assert_eq!(far.start_margin, 54.0..66.0);
        // The shipped lamps have no lod1: one mesh, drawn at any distance.
        assert!(plain.lod.is_empty());
    }

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

    /// Every lamp file is there with its proxy, its light anchor, its fit
    /// and the old generator's sizes: the pole 0.14 m square from 0.1 m
    /// below the foot, the arm to 0.05 m past the head's centre, the head
    /// 0.56 × 0.12 × 0.28 m, the light 0.3 m under it – for every arm.
    #[test]
    fn lamps_are_drawn_from_their_files_at_the_old_sizes() {
        for (id, h) in LAMP_FILES {
            let m = ocw_assets::shipped::model(id);
            assert!(m.node(LIGHT_ANCHOR).is_some(), "{id}");
            assert!(!ocw_assets::props::proxies(&m).is_empty(), "{id}");
            let fit = ocw_assets::props::FileFit::of(&m).unwrap();
            assert_eq!(fit.height_m, h, "{id}");
            assert_eq!(
                h,
                ocw_world::lamps::lamp_height_m(match h {
                    8.0 => ocw_world::graph::RoadClass::Primary,
                    6.0 => ocw_world::graph::RoadClass::Residential,
                    _ => ocw_world::graph::RoadClass::Path,
                })
            );
            for arm in [0.6f32, 1.25, 2.0] {
                let lamp = Lamp {
                    foot: [0.0, 0.0, 0.0],
                    toward: [1.0, 0.0],
                    height_m: h,
                    arm_m: arm,
                };
                let t = lamp_arrays(&[(lamp, 0.0)]);
                let b = |a: &MeshArrays| {
                    a.positions
                        .iter()
                        .fold((Vec3::MAX, Vec3::MIN), |(lo, hi), v| {
                            (lo.min(Vec3::from(*v)), hi.max(Vec3::from(*v)))
                        })
                };
                let (lo, hi) = b(&t.poles);
                let want_lo = Vec3::new(-0.07, -0.1, -0.07);
                let want_hi = Vec3::new(arm + 0.05, h + 0.1, 0.07);
                assert!(
                    lo.distance(want_lo) < 1e-3 && hi.distance(want_hi) < 1e-3,
                    "{id} {arm}: {lo} {hi}"
                );
                let (lo, hi) = b(&t.heads);
                let want_lo = Vec3::new(arm - 0.28, h - 0.08, -0.14);
                let want_hi = Vec3::new(arm + 0.28, h + 0.04, 0.14);
                assert!(
                    lo.distance(want_lo) < 1e-3 && hi.distance(want_hi) < 1e-3,
                    "{id} {arm}: {lo} {hi}"
                );
                assert!(Vec3::from(t.spots[0]).distance(Vec3::new(arm, h - 0.32, 0.0)) < 1e-3);
            }
        }
    }

    /// The renderer places the lamps the server's obstacles have (both
    /// kinds: OSM's mapped lamps and the rows along lit roads), from the
    /// whole package and from a streamed version holding only the tile
    /// and its ring.
    #[test]
    fn the_drawn_lamps_are_the_servers() {
        use ocw_world::junction::fixtures;
        use ocw_world::obstacles::{ObstacleKind, Obstacles, PointObstacle, TileObstacles};
        use std::collections::BTreeMap;
        let mut g = fixtures::t_junction(ocw_world::graph::RoadClass::Primary, 0.0, 0.0);
        for v in &mut g.vertices {
            *v = [v[0] + 500.0, v[1] + 500.0];
        }
        for e in &mut g.edges {
            for p in &mut e.points {
                *p = [p[0] + 500.0, p[1] + 500.0];
            }
        }
        let key = TileKey { x: 0, z: 0 };
        let roads = fixtures::strips(&g)
            .into_iter()
            .map(|(_, mut s)| {
                s.lit = ocw_world::RoadLit::Yes;
                s
            })
            .collect();
        let lamp = |x: f32, z: f32| PointObstacle {
            kind: ObstacleKind::StreetLamp,
            flags: 0,
            pos: [x, z],
            angle: 0.0,
            width_m: 0.0,
            height_m: 0.0,
        };
        let mut o = Obstacles::new(1000.0);
        o.tiles.insert(
            key,
            TileObstacles {
                // Beside the main road, on it, and at the junction.
                points: vec![lamp(450.0, 507.0), lamp(560.0, 501.0), lamp(502.0, 503.0)],
                ..Default::default()
            },
        );
        let pkg = WorldPackage {
            name: "lamps".into(),
            origin: (47.0, 9.0),
            tile_size_m: 1000.0,
            samples: 2,
            tiles: BTreeMap::from([(
                key,
                ocw_world::Tile {
                    key,
                    heights: vec![0.0; 4],
                    roads,
                },
            )]),
            graph: Some(std::sync::Arc::new(g)),
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: Some(o),
            places: None,
            junctions: None,
        };
        let pj = PackageJunctions::new(&pkg);
        let drawn = placed_lamps(&pkg, key, pj.as_ref());
        let server = ocw_world::lamps::package_lamps(&pkg);
        assert_eq!(server.get(&key), Some(&drawn));
        assert!(drawn.len() > 3, "{drawn:?}");
        // The mapped ones come first, out of the roads.
        assert_eq!([drawn[0].foot[0], drawn[0].foot[2]], [450.0, 507.0]);
        assert!(
            drawn[1].foot[2] > 504.0,
            "moved to the kerb: {:?}",
            drawn[1]
        );
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

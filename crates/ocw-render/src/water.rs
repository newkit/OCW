//! Water surfaces (spec 4.6 *Water surfaces*, v6.58): the material of
//! lakes, rivers and the sea, and the ribbons of the rivers and streams
//! along their course (package section `WATR`).
//!
//! - **The material** (`water.wesl`): the standard PBR light on a rippled
//!   normal (directional waves moving with the wind, a noise layer; short
//!   waves fade out with the distance so nothing shimmers), the water's
//!   own colour by depth and kind (shallow water shows its bed, deep water
//!   dark blue-green; rivers greener, the sea bluer), the sky reflected by
//!   the Fresnel law (the fog's horizon colour turning bluer towards the
//!   zenith), a broken band of foam on the shallowest water and the
//!   surface fading out at the shore, so land and water meet softly.
//!   No textures: the same in WebGPU, WebGL 2 and the Quest.
//! - **Vertex data:** `uv = (depth m, kind)` ([`KIND_LAKE`], [`KIND_RIVER`],
//!   [`KIND_SEA`]); the grid water of the land use (lakes, riverbank
//!   areas, the sea: `package::water_arrays`) and the ribbons
//!   ([`ribbon_arrays`]).
//! - **Ribbons:** each river or stream of the tile as a strip of its width
//!   along its points: across it three vertices (bank, middle, bank), the
//!   depth 0 on the banks; a wide river lies flat at its level (its channel
//!   is carved below it by the world builder), a stream on the drawn ground
//!   ([`RIBBON_LIFT_M`] above it, its stored level lies below the ground).

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use ocw_world::package::{TileKey, WorldPackage};

use crate::package::{Keep, MeshArrays};

/// The water's material: standard PBR with the water shader.
pub type WaterMaterial = ExtendedMaterial<StandardMaterial, WaterExtension>;

/// Kinds of water (`uv.y`).
pub const KIND_LAKE: f32 = 0.0;
pub const KIND_RIVER: f32 = 1.0;
pub const KIND_SEA: f32 = 2.0;

/// A ribbon lies this far above the drawn ground where the ground is above
/// its level (streams, the banks of a river).
pub const RIBBON_LIFT_M: f32 = 0.08;
/// The depth of a stream's water in the middle of its ribbon (m; the
/// colour and the softness of its banks).
pub const STREAM_DEPTH_M: f32 = 0.35;

/// The water material's parameters (`water.wesl` `WaterLook`).
#[derive(Clone, Copy, Debug, ShaderType, Reflect, PartialEq)]
pub struct WaterLook {
    /// x: ripple strength, y: wind (m/s), z: the direction the wind blows
    /// to (rad, from +x towards +z), w: reflection strength.
    pub params: Vec4,
}

impl Default for WaterLook {
    fn default() -> Self {
        Self {
            params: Vec4::new(1.0, 3.0, 0.6, 1.0),
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, Default)]
pub struct WaterExtension {
    #[uniform(100)]
    pub look: WaterLook,
    /// The mountains' shadow (`terrain_shadow`, bindings 120 – 122).
    #[uniform(120)]
    pub terrain_shadow: crate::terrain_shadow::TerrainShadowUniform,
    #[texture(121)]
    #[sampler(122)]
    pub terrain_shadow_map: Option<Handle<Image>>,
}

impl MaterialExtension for WaterExtension {
    fn fragment_shader() -> ShaderRef {
        // The bisect switch (10G): Bevy's standard shader.
        crate::switches::shader_or_default(
            crate::switches::plain_other(),
            ShaderRef::Path(
                AssetPath::from_path_buf(embedded_path!("water.wesl")).with_source("embedded"),
            ),
        )
    }
}

/// The water material (blended: its shallow edge fades out).
pub fn water_material() -> WaterMaterial {
    ExtendedMaterial {
        base: StandardMaterial {
            base_color: Color::WHITE,
            perceptual_roughness: 0.06,
            reflectance: 0.35,
            alpha_mode: AlphaMode::Blend,
            ..default()
        },
        extension: WaterExtension::default(),
    }
}

/// The wind of the weather as the water's parameters: speed (m/s) and the
/// direction it blows to in the world's x / z (the meteorological
/// direction it comes from: degrees clockwise from north, −z).
pub fn wind_params(wind_mps: f32, wind_from_deg: f32) -> Vec4 {
    let to = (wind_from_deg + 180.0).to_radians();
    // North is −z, east +x: a heading θ points along (sin θ, −cos θ).
    let (x, z) = (to.sin(), -to.cos());
    Vec4::new(1.0, wind_mps.max(0.0), z.atan2(x), 1.0)
}

pub struct WaterPlugin;

impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "water.wesl");
        app.add_plugins(MaterialPlugin::<WaterMaterial>::default())
            .add_systems(Update, follow_wind);
    }
}

/// The ripples follow the weather's wind (changed only when it moves by a
/// tenth of a m/s or a degree: a changed material is uploaded again).
fn follow_wind(
    view: Option<Res<crate::RideView>>,
    materials: Option<Res<crate::package::PackageMaterials>>,
    mut water: ResMut<Assets<WaterMaterial>>,
) {
    let (Some(view), Some(materials)) = (view, materials) else {
        return;
    };
    let Some(w) = view.weather.as_ref() else {
        return;
    };
    let want = wind_params(w.wind_mps, w.state.wind_from_deg);
    let Some(m) = water.get(&materials.water) else {
        return;
    };
    let now = m.extension.look.params;
    if ((now.y - want.y).abs() > 0.1 || (now.z - want.z).abs() > 0.02)
        && let Some(mut m) = water.get_mut(&materials.water)
    {
        m.extension.look.params = want;
    }
}

/// The ribbons of the rivers and streams of tile `key` (section `WATR`),
/// on the cells `keep` accepts; `ground` is the drawn ground at (x, z).
pub fn ribbon_arrays(
    pkg: &WorldPackage,
    key: TileKey,
    keep: Keep,
    ground: &dyn Fn(f64, f64) -> Option<f64>,
) -> MeshArrays {
    let mut m = MeshArrays::default();
    let Some(lines) = pkg
        .look
        .water_lines
        .as_ref()
        .and_then(|w| w.tiles.get(&key))
    else {
        return m;
    };
    for l in lines {
        let n = l.points.len();
        if n < 2 {
            continue;
        }
        let wide = f64::from(l.width_m) >= 8.0;
        let base = m.positions.len() as u32;
        for j in 0..n {
            let p = l.points[j];
            let (a, b) = (l.points[j.saturating_sub(1)], l.points[(j + 1).min(n - 1)]);
            let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
            let len = (dx * dx + dz * dz).sqrt().max(1e-6);
            let side = [-dz / len, dx / len];
            let half = p[3] / 2.0;
            for (s, across) in [(-1.0f32, 0.0f32), (0.0, 1.0), (1.0, 0.0)] {
                let (x, z) = (p[0] + side[0] * half * s, p[2] + side[1] * half * s);
                let g = ground(f64::from(x), f64::from(z)).map(|g| g as f32);
                let y = g.map_or(p[1], |g| p[1].max(g + RIBBON_LIFT_M));
                let depth = if wide {
                    // The water over the channel; 0 on the banks.
                    g.map_or(1.0, |g| (p[1] - g).max(0.0)) * across
                } else {
                    STREAM_DEPTH_M * across
                };
                m.positions.push([x, y, z]);
                m.normals.push([0.0, 1.0, 0.0]);
                m.colors.push([1.0; 4]);
                m.uvs.push([depth - (1.0 - across) * 0.02, KIND_RIVER]);
            }
        }
        for j in 0..n - 1 {
            let mid = [
                f64::from(l.points[j][0] + l.points[j + 1][0]) / 2.0,
                f64::from(l.points[j][2] + l.points[j + 1][2]) / 2.0,
            ];
            if keep.is_some_and(|f| !f(mid)) {
                continue;
            }
            let r = base + 3 * j as u32;
            let q = r + 3;
            // Two quads across: left bank – middle – right bank.
            m.indices.extend([
                r,
                q,
                r + 1,
                r + 1,
                q,
                q + 1,
                r + 1,
                q + 1,
                r + 2,
                r + 2,
                q + 1,
                q + 2,
            ]);
        }
    }
    m
}

/// Appends `b` to `a` (indices shifted).
pub fn append(a: &mut MeshArrays, b: MeshArrays) {
    let base = a.positions.len() as u32;
    a.positions.extend(b.positions);
    a.normals.extend(b.normals);
    a.colors.extend(b.colors);
    a.uvs.extend(b.uvs);
    a.indices.extend(b.indices.into_iter().map(|i| i + base));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_world::water_lines::{WaterLine, WaterLines};

    /// One flat 100 m tile at 400 m with these rivers.
    fn package(lines: Vec<WaterLine>) -> WorldPackage {
        use ocw_world::package::Tile;
        let key = TileKey { x: 0, z: 0 };
        let mut p = WorldPackage {
            name: "t".into(),
            origin: (47.0, 9.0),
            tile_size_m: 100.0,
            samples: 11,
            tiles: [(
                key,
                Tile {
                    key,
                    heights: vec![400.0; 121],
                    roads: Vec::new(),
                },
            )]
            .into(),
            graph: None,
            landuse: None,
            buildings: None,
            bridges: None,
            routes: Vec::new(),
            region: None,
            time_zone: None,
            look: Default::default(),
            obstacles: None,
            places: None,
            junctions: None,
            traffic: None,
            aero: None,
        };
        p.look.water_lines = Some(WaterLines::from_lines(&lines, 100.0));
        p
    }

    #[test]
    fn a_wide_river_lies_flat_at_its_level_and_a_stream_on_the_ground() {
        let river = WaterLine {
            width_m: 12.0,
            points: vec![
                [10.0, 401.0, 50.0, 12.0],
                [50.0, 400.8, 52.0, 12.0],
                [90.0, 400.6, 50.0, 12.0],
            ],
        };
        let stream = WaterLine {
            width_m: 3.0,
            points: vec![[10.0, 395.0, 20.0, 3.0], [90.0, 395.0, 25.0, 3.0]],
        };
        let pkg = package(vec![river, stream]);
        // The ground: 400 m, the river's channel 2 m deeper within 4 m of z 50.
        let ground = |x: f64, z: f64| -> Option<f64> {
            let _ = x;
            Some(if (z - 51.0).abs() < 4.0 { 398.0 } else { 400.0 })
        };
        let m = ribbon_arrays(&pkg, TileKey { x: 0, z: 0 }, None, &ground);
        // 3 + 2 points, three vertices across each; two quads per segment.
        assert_eq!(m.positions.len(), 15);
        assert_eq!(m.indices.len(), (2 + 1) * 12);
        assert_eq!(m.uvs.len(), 15);
        // The river's middle lies at its level over the channel, its depth
        // there 2.x m; its banks on the ground (at the level or above).
        let mid = m.positions[1];
        assert!((mid[1] - 401.0).abs() < 1e-4, "{mid:?}");
        assert!(m.uvs[1][0] > 2.5 && m.uvs[1][1] == KIND_RIVER);
        assert!(m.uvs[0][0] <= 0.0 && m.uvs[2][0] <= 0.0);
        // The stream is draped: 8 cm over the ground, not at its level.
        let s = m.positions[10];
        assert!((s[1] - (400.0 + RIBBON_LIFT_M)).abs() < 1e-4, "{s:?}");
        assert!((m.uvs[10][0] - STREAM_DEPTH_M).abs() < 1e-6);
        // Banks are 12 m (river) and 3 m (stream) apart.
        let w = |a: [f32; 3], b: [f32; 3]| (a[0] - b[0]).hypot(a[2] - b[2]);
        assert!((w(m.positions[0], m.positions[2]) - 12.0).abs() < 0.01);
        assert!((w(m.positions[9], m.positions[11]) - 3.0).abs() < 0.01);
        // Cells the package does not draw here are left out.
        let none = |_: [f64; 2]| false;
        let cut = ribbon_arrays(&pkg, TileKey { x: 0, z: 0 }, Some(&none), &ground);
        assert!(cut.indices.is_empty());
    }

    #[test]
    fn the_ripples_follow_the_wind() {
        // A west wind (from 270°) blows to the east: +x.
        let p = wind_params(6.0, 270.0);
        assert!((p.y - 6.0).abs() < 1e-6);
        assert!(p.z.cos() > 0.99 && p.z.sin().abs() < 0.01, "{p:?}");
        // From the north it blows to the south: +z.
        let s = wind_params(2.0, 0.0);
        assert!(s.z.sin() > 0.99, "{s:?}");
    }
}

crate::terrain_shadowed!(WaterExtension);

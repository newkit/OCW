//! Terrain materials (spec §4.6, Phase 8 T2): the land classes of the
//! splat map drawn with the tiling materials of the library
//! ([`crate::materials`]) instead of flat colours.
//!
//! - **Splat weights:** every land class is a mix of the nine ground
//!   materials ([`class_weights`]). Per tile the weights are stored at the
//!   *corners* of the 5 m cells ([`splat_corners`], the mean of the four
//!   cells around each corner, neighbour tiles included), as two RGBA8
//!   textures of (cells + 1)² texels sampled bilinearly: borders between
//!   classes blend over a cell instead of showing the 5 m grid, and
//!   neighbouring tiles agree exactly on their shared edge.
//! - **In the shader** (`ground.wesl`, [`blend_weights`] is its CPU twin):
//!   the lookup is warped by noise (organic borders), the weights are
//!   sharpened by a per-material noise ("height blend", so a 50/50 mix is
//!   patches, not a smear), steep slopes turn to rock ([`rock_weight`],
//!   triplanar there), and each material is sampled at two scales (macro
//!   variation hides the tiling; with distance the coarse scale takes over).
//!   Beyond [`TerrainLook::fade`] the ground fades to the land-use colour
//!   map, whose class colours are the materials' mean colours
//!   ([`class_palette`]), so near and far agree.
//! - **Regional tint (T3 hook):** [`TerrainLook::material_tint`] (near) and
//!   the per-class `tint` of [`class_palette`] (far); neutral by default.
//!
//! The material is independent of the light setup: it fills the standard
//! PBR input and calls Bevy's own lighting.

use bevy::asset::{AssetPath, embedded_path};
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use ocw_world::landuse::LandClass;

use crate::tunnel_cut::{MAX_CUT_PIECES, TunnelCut};

/// The terrain's material: standard PBR with the ground materials and the
/// tunnel tubes cut out.
pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, GroundExtension>;

/// Number of ground materials (texture array layers), in the order of
/// `ocw_assets::materials::GROUND_IDS`.
pub const GROUND_LAYERS: usize = 9;
pub const MEADOW: usize = 0;
pub const FARMLAND: usize = 1;
pub const FOREST_FLOOR: usize = 2;
pub const ROCK: usize = 3;
pub const SCREE: usize = 4;
pub const GRAVEL: usize = 5;
pub const SAND: usize = 6;
pub const SNOW: usize = 7;
pub const WETLAND: usize = 8;

/// The mix of ground materials a land class is drawn with (sums to 1).
pub fn class_weights(class: LandClass) -> [f32; GROUND_LAYERS] {
    let mut w = [0.0; GROUND_LAYERS];
    let mut put = |i: usize, v: f32| w[i] = v;
    match class {
        LandClass::Default | LandClass::Meadow | LandClass::Orchard | LandClass::Park => {
            put(MEADOW, 1.0)
        }
        LandClass::Farmland => {
            put(FARMLAND, 0.55);
            put(MEADOW, 0.45);
        }
        LandClass::Vineyard => {
            put(FARMLAND, 0.6);
            put(MEADOW, 0.4);
        }
        // Needles and litter with moss (first renders: needles alone read
        // as a bright orange desert between the trees).
        LandClass::Forest => {
            put(FOREST_FLOOR, 0.6);
            put(WETLAND, 0.4);
        }
        LandClass::Scrub => {
            put(MEADOW, 0.5);
            put(FOREST_FLOOR, 0.3);
            put(WETLAND, 0.2);
        }
        LandClass::Residential => {
            put(MEADOW, 0.8);
            put(GRAVEL, 0.2);
        }
        LandClass::Commercial => {
            put(GRAVEL, 0.6);
            put(MEADOW, 0.4);
        }
        LandClass::Industrial => {
            put(GRAVEL, 0.7);
            put(MEADOW, 0.3);
        }
        LandClass::Water => {
            put(GRAVEL, 0.5);
            put(SAND, 0.5);
        }
        LandClass::Wetland => put(WETLAND, 1.0),
        LandClass::Rock => put(ROCK, 1.0),
        LandClass::Scree => put(SCREE, 1.0),
        LandClass::Glacier => put(SNOW, 1.0),
        LandClass::Bare => {
            put(GRAVEL, 0.6);
            put(SAND, 0.4);
        }
    }
    w
}

/// The ground-layer mix of every land class, by class code: the default
/// ([`default_mix`]) or a regional set's (`assets/materials.toml`,
/// `[[region]]` `classes`).
pub type ClassMix = [[f32; GROUND_LAYERS]; 17];

/// [`class_weights`] of every class.
pub fn default_mix() -> ClassMix {
    std::array::from_fn(|i| class_weights(LandClass::ALL[i]))
}

/// The mix of a regional set: its class overrides (by name, see
/// `ocw_assets::materials::LAND_CLASS_NAMES`), the default elsewhere.
pub fn region_mix(region: &ocw_assets::materials::Region) -> ClassMix {
    let mut m = default_mix();
    for (i, name) in ocw_assets::materials::LAND_CLASS_NAMES.iter().enumerate() {
        if let Some(w) = region.class_mix(name) {
            m[i] = w;
        }
    }
    m
}

/// Slope (rise over run) where rock starts to show, and where it is all
/// rock: 75 % … 120 % (37° … 50°). The flat colour map used 60 … 100 %,
/// but with the limestone texture the steep alpine meadows of the 10 m
/// grid read as bare cliffs (first renders above Vaduz); grass holds on
/// slopes of up to about 40°.
pub const ROCK_SLOPE: (f32, f32) = (0.75, 1.2);

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of the ground is bare rock at a slope (rise over run).
pub fn rock_weight(slope: f32) -> f32 {
    smoothstep(ROCK_SLOPE.0, ROCK_SLOPE.1, slope)
}

/// Forests hold on steep slopes: the rock weight is reduced where the
/// splat says forest floor (`forest`, its weight 0 … 1).
pub fn rock_weight_under(slope: f32, forest: f32) -> f32 {
    rock_weight(slope) * (1.0 - FOREST_KEEPS * forest.clamp(0.0, 1.0))
}

/// How much of the rock a full forest floor keeps out.
pub const FOREST_KEEPS: f32 = 0.7;

/// Albedo factor of the ground under a full forest canopy (first renders:
/// the sunlit needle litter read as a bright desert between the trees).
pub const CANOPY_SHADE: f32 = 0.55;

/// The canopy-shade factor for a forest-floor weight 0 … 1.
pub fn canopy_shade(forest: f32) -> f32 {
    1.0 + (CANOPY_SHADE - 1.0) * forest.clamp(0.0, 1.0)
}

/// The final weights at a point (CPU twin of `ground.wesl`): the splat
/// weights `w`, rock on steep slopes, sharpened by per-material `noise`
/// (0 … 1; "height blend"): materials with a higher weight + noise win
/// within `contrast`, so mixes become patches. Normalised.
#[allow(clippy::needless_range_loop)] // parallel weight arrays
pub fn blend_weights(
    w: [f32; GROUND_LAYERS],
    slope: f32,
    noise: [f32; GROUND_LAYERS],
    contrast: f32,
) -> [f32; GROUND_LAYERS] {
    let r = rock_weight_under(slope, w[FOREST_FLOOR]);
    let mut v = w.map(|x| x * (1.0 - r));
    v[ROCK] += r;
    let h: Vec<f32> = (0..GROUND_LAYERS)
        .map(|i| {
            if v[i] > 0.0 {
                v[i] + noise[i] * 0.5
            } else {
                -1.0
            }
        })
        .collect();
    let top = h.iter().copied().fold(f32::MIN, f32::max);
    let mut out = [0.0; GROUND_LAYERS];
    for i in 0..GROUND_LAYERS {
        out[i] = (h[i] - (top - contrast)).max(0.0) * v[i].max(0.0);
    }
    let sum: f32 = out.iter().sum();
    if sum <= 0.0 {
        return v;
    }
    out.map(|x| x / sum)
}

/// Splat weights at the cell corners of a tile: `(cells + 1)²` corners,
/// each the mean of the class weights of the four cells around it.
/// `class(c, r)` is the class of cell (column, row) of this tile, also for
/// −1 and `cells` (the neighbour tiles' cells; `None` if there is no
/// neighbour: the nearest cell of this tile counts instead). Returns two
/// RGBA8 planes: layers 0–3 and 4–7 (layer 8, wetland, is the rest).
#[allow(clippy::needless_range_loop)] // parallel weight arrays
pub fn splat_corners(
    cells: usize,
    mix: &ClassMix,
    class: impl Fn(i64, i64) -> Option<LandClass>,
) -> (Vec<[u8; 4]>, Vec<[u8; 4]>) {
    let n = cells as i64;
    let at = |c: i64, r: i64| {
        class(c, r)
            .unwrap_or_else(|| class(c.clamp(0, n - 1), r.clamp(0, n - 1)).unwrap_or_default())
    };
    let mut a = Vec::with_capacity((cells + 1).pow(2));
    let mut b = Vec::with_capacity((cells + 1).pow(2));
    for r in 0..=n {
        for c in 0..=n {
            let mut w = [0.0f32; GROUND_LAYERS];
            for (dc, dr) in [(-1, -1), (0, -1), (-1, 0), (0, 0)] {
                let cw = mix[at(c + dc, r + dr) as usize];
                for i in 0..GROUND_LAYERS {
                    w[i] += cw[i] / 4.0;
                }
            }
            let q = |x: f32| (x * 255.0).round().clamp(0.0, 255.0) as u8;
            a.push([q(w[0]), q(w[1]), q(w[2]), q(w[3])]);
            b.push([q(w[4]), q(w[5]), q(w[6]), q(w[7])]);
        }
    }
    (a, b)
}

/// The far colour of every land class (sRGB): the class's material mix of
/// the materials' mean colours (`means`, linear RGB, by layer) times the
/// class tint (`tint`, the T3 hook, 1 = neutral).
pub fn class_palette(
    means: &[[f32; 3]; GROUND_LAYERS],
    mix: &ClassMix,
    tint: impl Fn(LandClass) -> [f32; 3],
) -> Vec<(LandClass, [f32; 3])> {
    LandClass::ALL
        .iter()
        .map(|&c| {
            let w = mix[c as usize];
            let t = tint(c);
            let shade = canopy_shade(w[FOREST_FLOOR]);
            let lin: [f32; 3] = std::array::from_fn(|ch| {
                (0..GROUND_LAYERS).map(|i| w[i] * means[i][ch]).sum::<f32>() * t[ch] * shade
            });
            let srgb = Color::linear_rgb(lin[0], lin[1], lin[2]).to_srgba();
            (c, [srgb.red, srgb.green, srgb.blue])
        })
        .collect()
}

/// Look parameters of the terrain (uniform of `ground.wesl`).
#[derive(Debug, Clone, Copy, PartialEq, Reflect, ShaderType)]
pub struct TerrainLook {
    /// x: 1 = textured (library loaded); y: land-use cells per tile side;
    /// z, w: the textures fade to the colour map between these distances (m).
    pub mode: Vec4,
    /// Repeat size (m) of the nine layers, packed four per vector.
    pub sizes: [Vec4; 3],
    /// Near-texture tint per layer (rgb; the T3 hook, neutral = 1).
    pub material_tint: [Vec4; GROUND_LAYERS],
    /// x: normal-map strength; y: macro scale factor; z: noise contrast of
    /// the height blend; w: border warp (m).
    pub params: Vec4,
    /// x: tile size (m); y: canopy shade – the albedo factor where the
    /// splat says forest floor (the ground under the canopy gets less
    /// light than our low-poly trees' shadows give it).
    pub tile: Vec4,
    /// The orthophoto corridor (spec §14 Phase 8 T7, [`crate::ortho`]):
    /// x: 1 = a corridor texture is bound; y: its pixel size (m); z, w: it
    /// fades out with the view distance between these (m).
    pub ortho: Vec4,
    /// x, y: the sharp photo replaces the blurred one in the materials'
    /// tint between these distances (m); z: its share there; w: the tint
    /// strength.
    pub ortho_blend: Vec4,
}

impl Default for TerrainLook {
    fn default() -> Self {
        let ortho = crate::ortho::OrthoLook::default().uniforms(false, 0.5);
        Self {
            mode: Vec4::new(0.0, 200.0, 90.0, 350.0),
            sizes: [Vec4::splat(2.0); 3],
            material_tint: [Vec4::ONE; GROUND_LAYERS],
            params: Vec4::new(0.8, 7.3, 0.35, 2.0),
            tile: Vec4::new(1000.0, CANOPY_SHADE, 0.0, 0.0),
            ortho: ortho.0,
            ortho_blend: ortho.1,
        }
    }
}

impl TerrainLook {
    pub fn textured(&self) -> bool {
        self.mode.x > 0.5
    }

    /// Fade distances (start, end).
    pub fn fade(&self) -> (f32, f32) {
        (self.mode.z, self.mode.w)
    }

    pub fn size(&self, layer: usize) -> f32 {
        self.sizes[layer / 4][layer % 4]
    }

    pub fn set_size(&mut self, layer: usize, m: f32) {
        self.sizes[layer / 4][layer % 4] = m;
    }
}

/// The terrain's material extension: the tunnel cut (binding 100), the
/// look (101), the tile's splat weights (102 – 104), the ground
/// material arrays (105 – 107) and the tile's orthophoto corridor (108 –
/// 109, spec §14 Phase 8 T7).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct GroundExtension {
    /// x: number of cut pieces; y: tube wall height; z: crown height (m).
    #[uniform(100)]
    pub cut_shape: Vec4,
    /// Two per piece, see [`TunnelCut`].
    #[uniform(100)]
    pub cut_pieces: [Vec4; 2 * MAX_CUT_PIECES],
    #[uniform(101)]
    pub look: TerrainLook,
    #[texture(102)]
    #[sampler(103)]
    pub splat_a: Option<Handle<Image>>,
    #[texture(104)]
    pub splat_b: Option<Handle<Image>>,
    #[texture(105, dimension = "2d_array")]
    #[sampler(106)]
    pub albedo: Option<Handle<Image>>,
    #[texture(107, dimension = "2d_array")]
    pub normal: Option<Handle<Image>>,
    /// The orthophoto corridor of the tile (sRGB, corridor weight in alpha;
    /// [`crate::ortho::tile_texture`]); only near the rider.
    #[texture(108)]
    #[sampler(109)]
    pub ortho: Option<Handle<Image>>,
}

impl GroundExtension {
    /// Untextured (flat colours), cut by `cut`.
    pub fn new(cut: &TunnelCut) -> Self {
        Self {
            cut_shape: cut.shape,
            cut_pieces: cut.pieces,
            look: TerrainLook::default(),
            splat_a: None,
            splat_b: None,
            albedo: None,
            normal: None,
            ortho: None,
        }
    }

    pub fn cut(&self) -> TunnelCut {
        TunnelCut {
            shape: self.cut_shape,
            pieces: self.cut_pieces,
        }
    }

    pub fn set_cut(&mut self, cut: &TunnelCut) {
        self.cut_shape = cut.shape;
        self.cut_pieces = cut.pieces;
    }
}

impl MaterialExtension for GroundExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("ground.wesl")).with_source("embedded"),
        )
    }
}

/// A terrain material on top of `base`, cut by `cut`.
pub fn terrain_material(base: StandardMaterial, cut: &TunnelCut) -> TerrainMaterial {
    ExtendedMaterial {
        base,
        extension: GroundExtension::new(cut),
    }
}

#[allow(clippy::needless_range_loop)] // parallel weight arrays
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_class_is_a_full_mix_of_ground_materials() {
        for c in LandClass::ALL {
            let w = class_weights(c);
            let sum: f32 = w.iter().sum();
            assert!((sum - 1.0).abs() < 1e-6, "{c:?}");
            assert!(w.iter().all(|x| *x >= 0.0));
        }
        assert!(class_weights(LandClass::Forest)[FOREST_FLOOR] > 0.5);
        assert_eq!(class_weights(LandClass::Glacier)[SNOW], 1.0);
        assert_eq!(class_weights(LandClass::Rock)[ROCK], 1.0);
        assert!(class_weights(LandClass::Farmland)[FARMLAND] > 0.5);
    }

    /// A regional set changes the mix of its classes only (data-driven:
    /// `[[region]]` `classes` in the manifest).
    #[test]
    fn regional_class_mixes_override_the_default() {
        let m =
            ocw_assets::materials::MaterialManifest::parse(ocw_assets::materials::SHIPPED).unwrap();
        let canary = m.region_for(Some("Atlantic/Canary")).unwrap();
        let mix = region_mix(canary);
        let def = default_mix();
        let bare = LandClass::Bare as usize;
        assert!((mix[bare][SAND] - 0.5).abs() < 1e-6);
        assert!(mix[LandClass::Default as usize][GRAVEL] > 0.5);
        assert_eq!(
            mix[LandClass::Glacier as usize],
            def[LandClass::Glacier as usize]
        );
        // The names match the land classes.
        for c in LandClass::ALL {
            assert_eq!(
                ocw_assets::materials::LAND_CLASS_NAMES[c as usize],
                format!("{c:?}").to_lowercase()
            );
        }
        // The slot ids are the renderer's layers.
        assert_eq!(ocw_assets::materials::GROUND_IDS[ROCK], "rock");
        assert_eq!(ocw_assets::materials::GROUND_IDS[GRAVEL], "gravel");
    }

    #[test]
    fn steep_slopes_turn_to_rock() {
        assert_eq!(rock_weight(0.0), 0.0);
        assert_eq!(rock_weight(0.7), 0.0);
        assert_eq!(rock_weight(1.25), 1.0);
        let mid = rock_weight(0.975);
        assert!(mid > 0.3 && mid < 0.7, "{mid}");
        // Monotonic.
        let mut last = 0.0;
        for i in 0..200 {
            let r = rock_weight(i as f32 * 0.01);
            assert!(r >= last);
            last = r;
        }
        let meadow = class_weights(LandClass::Meadow);
        let flat = blend_weights(meadow, 0.1, [0.5; GROUND_LAYERS], 0.35);
        assert!((flat[MEADOW] - 1.0).abs() < 1e-6);
        let cliff = blend_weights(meadow, 1.5, [0.5; GROUND_LAYERS], 0.35);
        assert!((cliff[ROCK] - 1.0).abs() < 1e-6);
        // A steep forest stays mostly forest.
        let mut forest = [0.0; GROUND_LAYERS];
        forest[FOREST_FLOOR] = 1.0;
        assert!((rock_weight_under(1.5, 1.0) - (1.0 - FOREST_KEEPS)).abs() < 1e-6);
        let steep_forest = blend_weights(forest, 1.5, [0.5; GROUND_LAYERS], 0.35);
        assert!(steep_forest[FOREST_FLOOR] > 0.5, "{steep_forest:?}");
        let half = blend_weights(meadow, 0.95, [0.5; GROUND_LAYERS], 0.35);
        assert!(half[ROCK] > 0.0 && half[MEADOW] > 0.0);
        assert!((half.iter().sum::<f32>() - 1.0).abs() < 1e-5);
    }

    /// Height blend: a 55 / 45 mix becomes mostly one material where its
    /// noise is high, the other where the other's is – patches, not a smear.
    #[test]
    fn noise_turns_mixes_into_patches() {
        let farm = class_weights(LandClass::Farmland);
        let mut n = [0.5; GROUND_LAYERS];
        n[FARMLAND] = 1.0;
        n[MEADOW] = 0.0;
        let a = blend_weights(farm, 0.0, n, 0.35);
        assert!(a[FARMLAND] > 0.95, "{a:?}");
        n[FARMLAND] = 0.0;
        n[MEADOW] = 1.0;
        let b = blend_weights(farm, 0.0, n, 0.35);
        assert!(b[MEADOW] > 0.95, "{b:?}");
        // Equal noise keeps a soft mix.
        let c = blend_weights(farm, 0.0, [0.5; GROUND_LAYERS], 0.35);
        assert!(c[FARMLAND] > 0.5 && c[MEADOW] > 0.2, "{c:?}");
    }

    fn decode(a: &[u8; 4], b: &[u8; 4]) -> [f32; GROUND_LAYERS] {
        let mut w = [0.0; GROUND_LAYERS];
        for i in 0..4 {
            w[i] = f32::from(a[i]) / 255.0;
            w[i + 4] = f32::from(b[i]) / 255.0;
        }
        w[WETLAND] = (1.0 - w.iter().sum::<f32>()).max(0.0);
        w
    }

    /// Border smoothing: corners inside a class are pure, corners on a
    /// border are the mean of the cells around them; the weights at a
    /// tile's edge only depend on the cells on both sides, so the
    /// neighbouring tile has the same values there (no seam).
    #[test]
    fn splat_corners_smooth_borders_and_match_neighbours() {
        let cells = 8usize;
        // Global classes: forest for x < 4 cells (of the first tile),
        // meadow beyond; tile A covers columns 0..8, tile B 8..16.
        let global = |gc: i64, _gr: i64| {
            if gc < 4 {
                LandClass::Forest
            } else {
                LandClass::Meadow
            }
        };
        let tile = |x0: i64| {
            move |c: i64, r: i64| (r >= -1 && r <= cells as i64).then(|| global(x0 + c, r))
        };
        let mix = default_mix();
        let (a0, b0) = splat_corners(cells, &mix, tile(0));
        let (a1, b1) = splat_corners(cells, &mix, tile(cells as i64));
        let n = cells + 1;
        let w =
            |a: &[[u8; 4]], b: &[[u8; 4]], c: usize, r: usize| decode(&a[r * n + c], &b[r * n + c]);
        // Pure inside, a 50/50 corner on the border (column 4).
        assert!((w(&a0, &b0, 1, 3)[FOREST_FLOOR] - 0.6).abs() < 0.01);
        assert_eq!(w(&a0, &b0, 6, 3)[MEADOW], 1.0);
        let border = w(&a0, &b0, 4, 3);
        assert!((border[FOREST_FLOOR] - 0.3).abs() < 0.01, "{border:?}");
        assert!((border[MEADOW] - 0.5).abs() < 0.01);
        // Shared edge: column 8 of A = column 0 of B.
        for r in 0..n {
            assert_eq!(a0[r * n + cells], a1[r * n]);
            assert_eq!(b0[r * n + cells], b1[r * n]);
        }
        // Weights sum to 1 (wetland is the rest).
        for i in 0..a0.len() {
            let s: f32 = decode(&a0[i], &b0[i]).iter().sum();
            assert!((s - 1.0).abs() < 0.02);
        }
        // Without neighbours the edge repeats the tile's own cells.
        let (lone, _) = splat_corners(cells, &mix, |c, r| {
            (c >= 0 && r >= 0 && c < cells as i64 && r < cells as i64).then_some(LandClass::Rock)
        });
        assert_eq!(lone[0][ROCK], 255);
    }

    #[test]
    fn the_far_palette_is_the_material_mix_and_the_tint_hook_is_neutral() {
        let mut means = [[0.1, 0.1, 0.1]; GROUND_LAYERS];
        means[MEADOW] = [0.1, 0.3, 0.05];
        means[FARMLAND] = [0.3, 0.2, 0.1];
        let p = class_palette(&means, &default_mix(), |_| [1.0; 3]);
        assert_eq!(p.len(), 17);
        let meadow = p.iter().find(|(c, _)| *c == LandClass::Meadow).unwrap().1;
        let want = Color::linear_rgb(0.1, 0.3, 0.05).to_srgba();
        assert!((meadow[1] - want.green).abs() < 1e-5);
        let tinted = class_palette(&means, &default_mix(), |c| {
            if c == LandClass::Meadow {
                [0.5, 1.0, 1.0]
            } else {
                [1.0; 3]
            }
        });
        let m2 = tinted
            .iter()
            .find(|(c, _)| *c == LandClass::Meadow)
            .unwrap()
            .1;
        assert!(m2[0] < meadow[0] && (m2[1] - meadow[1]).abs() < 1e-6);
        // Forests are shaded.
        // Forest: the 0.1 grey of its materials, darkened by the canopy.
        let forest = p.iter().find(|(c, _)| *c == LandClass::Forest).unwrap().1;
        let plain = Color::linear_rgb(0.1, 0.1, 0.1).to_srgba().red;
        let shaded = Color::linear_rgb(
            0.1 * canopy_shade(class_weights(LandClass::Forest)[FOREST_FLOOR]),
            0.0,
            0.0,
        )
        .to_srgba()
        .red;
        assert!((forest[0] - shaded).abs() < 1e-4 && forest[0] < plain);
        assert!(canopy_shade(1.0) < 0.7 && canopy_shade(0.0) == 1.0);
        // Defaults: neutral tint, fade after the near tiles, untextured.
        let look = TerrainLook::default();
        assert!(!look.textured());
        assert!(look.material_tint.iter().all(|t| *t == Vec4::ONE));
        let (a, b) = look.fade();
        assert!(a > 50.0 && b > a && b < 1000.0);
        let mut l = look;
        l.set_size(5, 1.5);
        assert_eq!(l.size(5), 1.5);
    }
}

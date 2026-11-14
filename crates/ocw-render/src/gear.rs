//! Rider and bike materials (spec §14 Phase 8 T6): every part of the near
//! models gets a real material instead of a flat paint slot – carbon weave
//! under a clear coat, spun / brushed metal, rubber with a tread, grips,
//! saddle leather, jersey knit with mesh side panels, lycra, plastics with
//! a gloss or matte finish – and the garments carry the jersey designer's
//! pattern, colours and race number (`ocw_core::kit`). One material type,
//! [`GearMaterial`]: the standard PBR material with the `gear.wesl`
//! fragment shader.
//!
//! The kind of a part comes from its paint slot and node ([`kind_of`]),
//! its colours and finish from the look ([`surface`]). The far LOD (the
//! bike's `lod1`, past `riders::LOD_DISTANCE_M`) keeps plain
//! [`StandardMaterial`]s in the kit's colours (the jersey in its dominant
//! colour): cheap at a distance. Meshes for the shader carry their rest
//! coordinates ([`gear_mesh`]). With the material library's gear layers
//! (`ocw_assets::materials::GEAR_IDS`, CC0) knit, carbon and leather are
//! textured; without them they are procedural.

use bevy::asset::{AssetPath, RenderAssetUsages, embedded_asset, embedded_path};
use bevy::math::Affine3A;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use ocw_assets::model::{Model, Primitive};
use ocw_assets::slots;
use ocw_core::kit::Garment;
use ocw_core::{Finish, KitPattern};

use crate::materials::MaterialLibrary;
use crate::riders::{AvatarColours, tint_color};

pub type GearMaterial = ExtendedMaterial<StandardMaterial, GearExtension>;

/// `OCW_GEAR=off`: riders and bikes in plain materials everywhere, as
/// before T6 (frame-time comparisons, slow GPUs).
pub fn off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var("OCW_GEAR").is_ok_and(|v| v == "off"))
}

/// What a part is made of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GearKind {
    /// Skin, hair, lenses, lights, small details: the slot's colour.
    Plain,
    /// Painted frame and decals: a clear coat.
    Paint,
    /// Bare carbon: the weave under a clear coat (frames, rims, bars).
    Carbon,
    /// Brushed / spun metal (cassette, chainrings, spokes, hubs) and the
    /// anodised components (stems, posts, derailleurs).
    Metal,
    /// Tyres: the tread and the walls.
    Tyre,
    /// Bar tape and grips.
    Grip,
    /// The saddle.
    Leather,
    /// Jersey, socks: knit.
    Knit,
    /// Shorts, skinsuit, gloves, overshoes: lycra.
    Lycra,
    /// Helmet shell, shoe uppers, bottles: gloss or matte.
    Plastic,
    /// The padded palm of gel gloves.
    Pad,
}

impl GearKind {
    /// The shader's code (`gear.wesl`, KIND_*).
    pub fn code(self) -> u32 {
        self as u32
    }
}

/// The material of a mesh in paint slot `slot` on the model node `node`.
pub fn kind_of(slot: &str, node: &str) -> GearKind {
    match slot {
        "frame" | "decal" => GearKind::Paint,
        "carbon" | "rim" => GearKind::Carbon,
        "metal" => GearKind::Metal,
        "component" if node == ocw_assets::pose::bike_nodes::SADDLE => GearKind::Leather,
        "component" => GearKind::Metal,
        "tyre" | "tyre_wall" => GearKind::Tyre,
        "bar_tape" => GearKind::Grip,
        "jersey" if node.starts_with("suit_") => GearKind::Lycra,
        "jersey" | "sock" | "sock_aero" => GearKind::Knit,
        "shorts" | "overshoe" | "glove" => GearKind::Lycra,
        "glove_pad" => GearKind::Pad,
        "helmet" | "shoe" | "shoe_race" | "bottle" => GearKind::Plastic,
        _ => GearKind::Plain,
    }
}

/// The garment whose pattern a slot carries.
pub fn garment_of(slot: &str) -> Option<Garment> {
    match slot {
        "jersey" => Some(Garment::Jersey),
        "shorts" => Some(Garment::Shorts),
        "helmet" => Some(Garment::Helmet),
        _ => None,
    }
}

/// Uniform of `gear.wesl`.
#[derive(Debug, Clone, Copy, PartialEq, Reflect, ShaderType)]
pub struct GearLook {
    /// x: kind; y: garment (0 none, 1 jersey, 2 shorts, 3 helmet); z:
    /// pattern; w: flags (1 three colours, 2 textured, 4 mesh panels).
    pub mode: Vec4,
    /// x: race number; y: detail strength; z: detail size (m); w:
    /// anisotropy.
    pub params: Vec4,
    /// Linear colours: the garment's own, first, second, third.
    pub base: Vec4,
    pub first: Vec4,
    pub second: Vec4,
    pub third: Vec4,
    /// Per texture layer: mean albedo (linear), repeat size (m).
    pub layers: [Vec4; 3],
}

pub const FLAG_THREE: u32 = 1;
pub const FLAG_TEXTURED: u32 = 2;
pub const FLAG_MESH_PANELS: u32 = 4;

/// The layers' repeats without the library (the shader fades by them).
const DEFAULT_LAYERS: [Vec4; 3] = [
    Vec4::new(0.3, 0.3, 0.3, 0.06),
    Vec4::new(0.05, 0.05, 0.05, 0.04),
    Vec4::new(0.05, 0.05, 0.05, 0.15),
];

impl GearLook {
    pub fn kind(&self) -> u32 {
        self.mode.x as u32
    }

    pub fn flags(&self) -> u32 {
        self.mode.w as u32
    }

    fn set_flag(&mut self, flag: u32, on: bool) {
        let f = if on {
            self.flags() | flag
        } else {
            self.flags() & !flag
        };
        self.mode.w = f as f32;
    }
}

/// The shader's part of [`GearMaterial`]. Bindless where the platform
/// has it (`gear.wesl`: the index table at binding 100, the looks at 101;
/// bindings 50–53 otherwise): the materials of all riders share a slab, so
/// their draws batch – with a bind group per rider and part, 30 riders
/// cost several milliseconds per frame.
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
#[data(50, GearLook, binding_array(101))]
#[bindless(index_table(range(50..54), binding(100)))]
pub struct GearExtension {
    pub look: GearLook,
    #[texture(51, dimension = "2d_array")]
    #[sampler(52)]
    pub albedo: Option<Handle<Image>>,
    #[texture(53, dimension = "2d_array")]
    pub normal: Option<Handle<Image>>,
}

impl From<&GearExtension> for GearLook {
    fn from(e: &GearExtension) -> Self {
        e.look
    }
}

impl MaterialExtension for GearExtension {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("gear.wesl")).with_source("embedded"),
        )
    }
}

/// What one part is drawn with: the base material's values and the
/// shader's uniform (Bevy-free values; tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GearSurface {
    pub kind: GearKind,
    pub garment: Option<Garment>,
    pub base: Color,
    pub metallic: f32,
    pub roughness: f32,
    pub clearcoat: f32,
    pub clearcoat_roughness: f32,
    pub look: GearLook,
}

fn lin(c: Color) -> Vec4 {
    let l = c.to_linear();
    Vec4::new(l.red, l.green, l.blue, 1.0)
}

/// The surface of a part in slot `slot` on node `node` (model colour
/// `model_color`, linear RGBA) for a rider in `colours`.
pub fn surface(
    slot: &str,
    node: &str,
    model_color: [f32; 4],
    colours: &AvatarColours,
) -> GearSurface {
    let s = slots::get(slot).unwrap_or(&slots::ALL[0]);
    // A frame painted "bare carbon" shows its weave (T6).
    let kind = if slot == "frame" && is_bare_carbon(colours.frame) {
        GearKind::Carbon
    } else {
        kind_of(slot, node)
    };
    let kit = &colours.kit;
    let mut base = tint_color(slot, model_color, colours);
    let mut metallic = s.metallic;
    let mut roughness = s.roughness;
    let (mut clearcoat, mut clearcoat_roughness) = (0.0, 0.5);
    let finish = |f: Finish| match f {
        Finish::Gloss => (0.28, 0.85, 0.08),
        Finish::Matte => (0.7, 0.0, 0.5),
    };
    // Detail strength, size (m), anisotropy.
    let (mut strength, mut size, mut aniso) = (0.0, 0.01, 0.0);
    match kind {
        GearKind::Paint => {
            roughness = 0.38;
            clearcoat = 1.0;
            clearcoat_roughness = 0.06;
        }
        GearKind::Carbon => {
            metallic = 0.0;
            roughness = 0.4;
            clearcoat = 1.0;
            clearcoat_roughness = 0.05;
            (strength, size) = (1.0, 0.003);
        }
        GearKind::Metal => {
            let anodised = slot == "component";
            (strength, size, aniso) = if anodised {
                (0.6, 0.0008, 0.35)
            } else {
                (1.0, 0.0008, 0.6)
            };
        }
        GearKind::Tyre => {
            roughness = roughness.max(0.82);
            (strength, size) = (1.0, 0.004);
        }
        GearKind::Grip => {
            roughness = 0.82;
            (strength, size) = (1.0, 0.0025);
        }
        GearKind::Leather => {
            roughness = 0.5;
            metallic = 0.0;
            (strength, size) = (1.0, 0.002);
        }
        GearKind::Knit => {
            roughness = 0.8;
            (strength, size) = (1.0, 0.004);
        }
        GearKind::Lycra => {
            roughness = 0.42;
            (strength, size) = (1.0, 0.003);
        }
        GearKind::Pad => {
            roughness = 0.75;
            (strength, size) = (1.0, 0.004);
        }
        GearKind::Plastic => {
            let f = match slot {
                "helmet" => Some(kit.helmet_finish),
                "shoe" | "shoe_race" => Some(kit.shoe_finish),
                _ => None,
            };
            if let Some(f) = f {
                (roughness, clearcoat, clearcoat_roughness) = finish(f);
                metallic = 0.0;
            }
        }
        GearKind::Plain => {}
    }
    let garment = garment_of(slot);
    let jersey = colours.jersey;
    let mut pattern = KitPattern::Plain;
    let mut number = 0;
    let mut first = jersey;
    match garment {
        Some(Garment::Jersey) => {
            pattern = kit.pattern;
            number = kit.number;
            base = jersey;
        }
        Some(Garment::Shorts) => {
            pattern = kit.pattern;
            number = kit.number;
        }
        Some(Garment::Helmet) => {
            if kit.helmet_pattern {
                pattern = kit.pattern;
            } else {
                first = base;
            }
        }
        None => {}
    }
    let mut flags = 0;
    if kit.three && garment.is_some() {
        flags |= FLAG_THREE;
    }
    // Only garments carry the design's colours: everything else is shared
    // by every rider in the same colour (one material, batched draws; a
    // material per rider and part cost 10× the frame time with 30 riders).
    let (first, second, third) = if garment.is_some() {
        (lin(first), lin(kit.second), lin(kit.third))
    } else {
        (Vec4::ZERO, Vec4::ZERO, Vec4::ZERO)
    };
    if kind == GearKind::Knit && garment == Some(Garment::Jersey) {
        flags |= FLAG_MESH_PANELS;
    }
    GearSurface {
        kind,
        garment,
        base,
        metallic,
        roughness,
        clearcoat,
        clearcoat_roughness,
        look: GearLook {
            mode: Vec4::new(
                kind.code() as f32,
                garment.map_or(0.0, |g| (g.wire() + 1) as f32),
                pattern.wire() as f32,
                flags as f32,
            ),
            params: Vec4::new(f32::from(number), strength, size, aniso),
            base: lin(base),
            first,
            second,
            third,
            layers: DEFAULT_LAYERS,
        },
    }
}

/// Whether a frame colour is [`ocw_core::bike::BARE_CARBON`].
pub fn is_bare_carbon(c: Color) -> bool {
    let [r, g, b] = ocw_core::bike::BARE_CARBON.channels();
    c.to_srgba().to_u8_array()[..3] == [r, g, b]
}

impl GearSurface {
    /// The material (`glow`: lamps lit, the workshop's light set).
    pub fn material(&self, glow: bool) -> GearMaterial {
        GearMaterial {
            base: StandardMaterial {
                base_color: self.base,
                metallic: self.metallic,
                perceptual_roughness: self.roughness,
                clearcoat: self.clearcoat,
                clearcoat_perceptual_roughness: self.clearcoat_roughness,
                // Turns on the anisotropic lighting; the shader sets the
                // direction and strength.
                anisotropy_strength: self.look.params.w,
                emissive: if glow {
                    LinearRgba::from(self.base) * 8.0
                } else {
                    LinearRgba::BLACK
                },
                ..default()
            },
            extension: GearExtension {
                look: self.look,
                albedo: None,
                normal: None,
            },
        }
    }

    /// A key for sharing materials: equal surfaces, equal keys.
    pub fn key(&self, glow: bool) -> Vec<u32> {
        let l = &self.look;
        let mut k: Vec<u32> = [l.mode, l.params, l.base, l.first, l.second, l.third]
            .iter()
            .flat_map(|v| v.to_array())
            .map(f32::to_bits)
            .collect();
        k.extend(
            [
                self.metallic,
                self.roughness,
                self.clearcoat,
                self.clearcoat_roughness,
            ]
            .map(f32::to_bits),
        );
        k.extend(lin(self.base).to_array().map(f32::to_bits));
        k.push(u32::from(glow));
        k
    }

    /// Whether the part needs the gear shader: its detail (carbon, metal,
    /// rubber, fabrics, leather) or the design's pattern. Painted, plain
    /// and plastic parts are standard materials with a clear coat or
    /// finish: they batch across riders (a material of its own per rider
    /// and part costs a draw of its own in every pass).
    pub fn needs_shader(&self) -> bool {
        match self.kind {
            GearKind::Plain | GearKind::Paint => false,
            GearKind::Plastic => self.garment == Some(Garment::Helmet) && self.look.mode.z != 0.0,
            _ => true,
        }
    }

    /// The standard material of a part that needs no shader.
    pub fn plain_material(&self) -> StandardMaterial {
        StandardMaterial {
            base_color: self.base,
            metallic: self.metallic,
            perceptual_roughness: self.roughness,
            clearcoat: self.clearcoat,
            clearcoat_perceptual_roughness: self.clearcoat_roughness,
            ..default()
        }
    }

    /// The far LOD's plain material in the same colours.
    pub fn far_material(&self, colours: &AvatarColours, slot: &str) -> StandardMaterial {
        let base = if slot == "jersey" {
            colours.far_jersey
        } else {
            self.base
        };
        StandardMaterial {
            base_color: base,
            metallic: self.metallic,
            perceptual_roughness: self.roughness,
            ..default()
        }
    }
}

/// The rest coordinates of model node `i` for the shader: the rider's
/// rest pose (model space) and its body region; bike parts their node's
/// own coordinates.
pub fn rest_frame(model: &Model, i: usize, rider: bool) -> (Affine3A, u32) {
    if rider {
        (
            model.world(i),
            ocw_assets::rider::body_region(model, i).code(),
        )
    } else {
        (Affine3A::IDENTITY, 0)
    }
}

/// A mesh of `p` with the gear shader's coordinates: uv = rest (x, y),
/// uv_b = (rest z, body region).
pub fn gear_mesh(p: &Primitive, rest: Affine3A, region: u32) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    let r: Vec<Vec3> = p
        .positions
        .iter()
        .map(|v| rest.transform_point3(Vec3::from(*v)))
        .collect();
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_0,
        r.iter().map(|v| [v.x, v.y]).collect::<Vec<_>>(),
    );
    m.insert_attribute(
        Mesh::ATTRIBUTE_UV_1,
        r.iter().map(|v| [v.z, region as f32]).collect::<Vec<_>>(),
    );
    m.insert_indices(Indices::U32(p.indices.clone()));
    m
}

/// The library's gear layers, for materials made from now on.
#[derive(Resource, Clone)]
pub struct GearTextures {
    pub albedo: Handle<Image>,
    pub normal: Handle<Image>,
    pub layers: [Vec4; 3],
}

impl GearTextures {
    /// Gives a material the textures (knit, carbon and leather only).
    pub fn apply(&self, m: &mut GearMaterial) {
        let kind = m.extension.look.kind();
        let textured = [GearKind::Knit, GearKind::Carbon, GearKind::Leather]
            .iter()
            .any(|k| k.code() == kind);
        if !textured {
            return;
        }
        m.extension.albedo = Some(self.albedo.clone());
        m.extension.normal = Some(self.normal.clone());
        m.extension.look.layers = self.layers;
        m.extension.look.set_flag(FLAG_TEXTURED, true);
    }
}

pub struct GearMaterialPlugin;

impl Plugin for GearMaterialPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "gear.wesl");
        app.add_plugins(MaterialPlugin::<GearMaterial>::default())
            .add_systems(PostUpdate, sync_gear_textures);
    }
}

/// Gives every gear material the library's layers once they are loaded
/// (materials made before and after).
fn sync_gear_textures(
    mut commands: Commands,
    library: Option<Res<MaterialLibrary>>,
    textures: Option<Res<GearTextures>>,
    mut materials: ResMut<Assets<GearMaterial>>,
    mut events: MessageReader<AssetEvent<GearMaterial>>,
) {
    let Some(textures) = textures else {
        if let Some(g) = library.as_ref().and_then(|l| l.gear.as_ref()) {
            let t = GearTextures {
                albedo: g.albedo.clone(),
                normal: g.normal.clone(),
                layers: std::array::from_fn(|i| {
                    let m = g.means[i];
                    Vec4::new(m[0], m[1], m[2], g.sizes[i])
                }),
            };
            for (_, m) in materials.iter_mut() {
                t.apply(m);
            }
            commands.insert_resource(t);
        }
        events.clear();
        return;
    };
    let added: Vec<AssetId<GearMaterial>> = events
        .read()
        .filter_map(|e| match e {
            AssetEvent::Added { id } => Some(*id),
            _ => None,
        })
        .collect();
    for id in added {
        let bare = materials
            .get(id)
            .is_some_and(|m| m.extension.albedo.is_none());
        if bare && let Some(mut m) = materials.get_mut(id) {
            textures.apply(&mut m);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riders::avatar_colours;
    use ocw_core::{KitDesign, Rgb, RiderId, RiderLook};

    /// Every slot of the shipped models has its material kind, by node
    /// where the slot is shared (the saddle among the components, the
    /// skinsuit among the jersey's parts).
    #[test]
    fn materials_are_assigned_per_slot_and_node() {
        use GearKind::*;
        for (slot, node, want) in [
            ("frame", "bike", Paint),
            ("carbon", "bike", Carbon),
            ("rim", "wheel_front", Carbon),
            ("metal", "crank", Metal),
            ("metal", "wheel_rear", Metal),
            ("component", "handlebar", Metal),
            ("component", ocw_assets::pose::bike_nodes::SADDLE, Leather),
            ("tyre", "wheel_front", Tyre),
            ("tyre_wall", "wheel_rear", Tyre),
            ("bar_tape", "handlebar", Grip),
            ("jersey", "torso", Knit),
            ("jersey", "suit_pelvis", Lycra),
            ("jersey", "sleeve_upper_l", Knit),
            ("shorts", "pelvis", Lycra),
            ("sock", "shin_l", Knit),
            ("glove", "glove_full_r", Lycra),
            ("glove_pad", "glove_gel_l", Pad),
            ("helmet", "helmet_road", Plastic),
            ("shoe_race", "shoe_race_l", Plastic),
            ("skin", "head", Plain),
            ("lens", "glasses_sport", Plain),
            ("light_front", "bike", Plain),
        ] {
            assert_eq!(kind_of(slot, node), want, "{slot} on {node}");
        }
        // Every paint slot has a kind (Plain is a choice, not a gap).
        for s in slots::ALL {
            let _ = kind_of(s.name, "x");
        }
        assert_eq!(garment_of("jersey"), Some(Garment::Jersey));
        assert_eq!(garment_of("accent"), None);
    }

    fn colours(look: &RiderLook) -> AvatarColours {
        avatar_colours(RiderId(1), true, Some(look))
    }

    /// The surfaces carry the look: the jersey's pattern, colours and
    /// number, the shorts' colour with the pattern's side panels, the
    /// helmet's finish and (optionally) the pattern, the free kit colours;
    /// carbon and paint under a clear coat, metal anisotropic.
    #[test]
    fn surfaces_carry_the_design() {
        let look = RiderLook {
            jersey: Rgb(0x1E88E5),
            helmet: Rgb(0xF2F2EE),
            pattern: KitPattern::Chevrons,
            design: KitDesign {
                second: Some(Rgb(0xFDD835)),
                third: Some(Rgb(0x1E1E22)),
                number: 42,
                shorts: Some(Rgb(0x3949AB)),
                gloves: Some(Rgb(0xE53935)),
                helmet_finish: Finish::Matte,
                ..Default::default()
            },
            ..Default::default()
        };
        let c = colours(&look);
        let j = surface("jersey", "torso", [1.0; 4], &c);
        assert_eq!(j.look.mode.y, 1.0, "jersey");
        assert_eq!(j.look.mode.z, KitPattern::Chevrons.wire() as f32);
        assert_eq!(j.look.params.x, 42.0);
        assert_eq!(j.look.flags() & FLAG_THREE, FLAG_THREE);
        assert_eq!(j.look.flags() & FLAG_MESH_PANELS, FLAG_MESH_PANELS);
        assert_eq!(j.look.base, lin(Color::srgb_u8(0x1E, 0x88, 0xE5)));
        assert_eq!(j.look.second, lin(Color::srgb_u8(0xFD, 0xD8, 0x35)));
        let s = surface("shorts", "pelvis", [0.01, 0.01, 0.01, 1.0], &c);
        assert_eq!(s.look.base, lin(Color::srgb_u8(0x39, 0x49, 0xAB)));
        assert_eq!(s.look.params.x, 42.0, "the hip numbers");
        let h = surface("helmet", "helmet_road", [1.0; 4], &c);
        assert_eq!(h.clearcoat, 0.0, "matte");
        assert!(h.roughness > 0.6);
        assert_eq!(h.look.mode.z, 0.0, "the pattern stays off the helmet");
        let g = surface("glove", "glove_mitt_l", [0.01, 0.01, 0.01, 1.0], &c);
        assert_eq!(g.base, Color::srgb_u8(0xE5, 0x39, 0x35));
        // Gloss helmet with the pattern.
        let both = RiderLook {
            design: KitDesign {
                helmet_pattern: true,
                helmet_finish: Finish::Gloss,
                ..look.design
            },
            ..look
        };
        let h = surface("helmet", "helmet_road", [1.0; 4], &colours(&both));
        assert!(h.clearcoat > 0.5 && h.roughness < 0.4);
        assert_eq!(h.look.mode.z, KitPattern::Chevrons.wire() as f32);
        assert_eq!(h.look.first, j.look.base, "the shell in the jersey colour");
        // Bike parts.
        let carbon = surface("carbon", "bike", [0.0, 0.0, 0.0, 1.0], &c);
        assert_eq!(carbon.clearcoat, 1.0);
        let frame = surface("frame", "bike", [0.0, 0.0, 0.0, 1.0], &c);
        assert_eq!(frame.clearcoat, 1.0);
        assert_eq!(frame.kind, GearKind::Paint);
        assert!(!frame.needs_shader() && frame.plain_material().clearcoat == 1.0);
        assert!(j.needs_shader() && carbon.needs_shader());
        // Painted "bare carbon": the weave.
        let bare = RiderLook {
            frame: ocw_core::bike::BARE_CARBON,
            ..look
        };
        let frame = surface("frame", "bike", [0.0, 0.0, 0.0, 1.0], &colours(&bare));
        assert_eq!(frame.kind, GearKind::Carbon);
        assert_eq!(
            surface("decal", "bike", [1.0; 4], &colours(&bare)).kind,
            GearKind::Paint
        );
        let metal = surface("metal", "crank", [0.6, 0.6, 0.6, 1.0], &c);
        assert!(metal.look.params.w > 0.0 && metal.material(false).base.anisotropy_strength > 0.0);
        // Equal surfaces share a material; another design does not.
        assert_eq!(
            j.key(false),
            surface("jersey", "torso", [1.0; 4], &c).key(false)
        );
        let renumbered = RiderLook {
            design: KitDesign {
                number: 43,
                ..look.design
            },
            ..look
        };
        assert_ne!(
            j.key(false),
            surface("jersey", "torso", [1.0; 4], &colours(&renumbered)).key(false)
        );
        // Parts without a garment are the same for every design (shared
        // materials: one draw batch for all riders).
        for (slot, node) in [
            ("metal", "crank"),
            ("tyre", "wheel_front"),
            ("skin", "head"),
        ] {
            assert_eq!(
                surface(slot, node, [0.5; 4], &c).key(false),
                surface(slot, node, [0.5; 4], &colours(&RiderLook::default())).key(false),
                "{slot}"
            );
        }
        // The helmet's design does not touch the jersey's material.
        assert_eq!(
            j.key(false),
            surface("jersey", "torso", [1.0; 4], &colours(&both)).key(false)
        );
    }

    /// The default (and anonymous) look: plain kit, no number, model
    /// colours; the far jersey is the dominant colour of the pattern.
    #[test]
    fn plain_looks_stay_plain() {
        for look in [RiderLook::default(), RiderLook::anonymous()] {
            let c = colours(&look);
            let j = surface("jersey", "torso", [1.0; 4], &c);
            assert_eq!(j.look.mode.z, 0.0);
            assert_eq!(j.look.params.x, 0.0);
            let s = surface("shorts", "pelvis", ocw_assets::model::srgb(0x141416), &c);
            assert_eq!(s.base.to_srgba().to_u8_array()[..3], [0x14, 0x14, 0x16]);
            assert_eq!(c.far_jersey, c.jersey);
        }
    }

    #[test]
    fn meshes_carry_their_rest_coordinates_and_region() {
        let rider = ocw_assets::shipped::rider_model();
        let i = rider.node(ocw_assets::pose::nodes::TORSO).unwrap();
        let (rest, region) = rest_frame(rider, i, true);
        assert_eq!(region, ocw_core::kit::BodyRegion::Torso.code());
        let p = &rider.nodes[i].primitives[0];
        let m = gear_mesh(p, rest, region);
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uv)) =
            m.attribute(Mesh::ATTRIBUTE_UV_0)
        else {
            panic!("uv");
        };
        let w = rest.transform_point3(Vec3::from(p.positions[0]));
        assert_eq!(uv[0], [w.x, w.y]);
        // The torso's points are in the rider's rest pose: above the hips.
        assert!(uv.iter().all(|v| v[1] > -0.1 && v[1] < 0.7));
        let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvb)) =
            m.attribute(Mesh::ATTRIBUTE_UV_1)
        else {
            panic!("uv_b");
        };
        assert!(uvb.iter().all(|v| v[1] == region as f32));
        let bike = ocw_assets::shipped::bike_model(ocw_core::ProfileId::Road);
        let w = bike
            .node(ocw_assets::pose::bike_nodes::WHEEL_FRONT)
            .unwrap();
        assert_eq!(rest_frame(bike, w, false), (Affine3A::IDENTITY, 0));
    }
}

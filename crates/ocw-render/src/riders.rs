//! Rider avatars (spec §2.5, §2.6, §8): the shipped glTF bike of the
//! rider's physics profile and the rider model, in the rider's colours,
//! animated from the ride (wheels from the speed, cranks and legs from
//! the cadence, standing on steep climbs and in sprints, cranks level
//! when freewheeling, the bike rocking under a standing rider). The
//! models, the pose and the state selection are Bevy-free
//! (`ocw_assets`); this module only spawns entities and writes
//! transforms. Far riders switch to the models' one-node `lod1`.
//!
//! The character and the kit (spec §14 Phase 4, 4C, W4): the rider model's
//! optional nodes the look shows (`ocw_assets::rider::visible`: worn kit
//! variants, hair, face, glasses), the skin and hair colours, the body
//! height (the whole rider scaled; the saddle moved along the seat tube
//! so the legs fit, `ocw_assets::pose::BodyFit`) and the build (girth of
//! the torso and limbs).
//!
//! Materials (spec §14 Phase 8 T6, [`crate::gear`]): the near models are
//! drawn with the gear shader – carbon, metal, rubber, fabrics, plastics,
//! the jersey designer's pattern and race number; the far model keeps plain
//! materials in the kit's colours.

use std::collections::{BTreeMap, HashMap};

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::VisibilityRange;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ocw_assets::anim::{self, AnimInput, Riding, Selector};
use ocw_assets::model::Model;
use ocw_assets::pose::{self, BikeRig, Fit, PoseParams, RiderRig, bike_nodes, nodes};
use ocw_assets::slots::{self, Tint};
use ocw_core::{ProfileId, RiderId, RiderLook};

use ocw_world::Track;

use crate::gear::{self, GearMaterial};
use crate::{Ground, RideView, to_vec3};

pub struct RidersPlugin;

impl Plugin for RidersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RiderEntities>().add_systems(
            PostUpdate,
            (sync_riders, ghost_casts_no_shadow, animate, lod_shadows)
                .chain()
                .before(TransformSystems::Propagate),
        );
    }
}

/// Marks a rider avatar entity.
#[derive(Component)]
pub struct Avatar {
    pub id: RiderId,
    pub is_local: bool,
}

/// Marks a ghost avatar whose parts no longer take part in shadows.
#[derive(Component)]
struct ShadowsOff;

/// The translucent ghost neither casts nor receives shadows: a solid
/// shadow under a see-through bike looks wrong, and its own parts would
/// darken each other.
fn ghost_casts_no_shadow(
    mut commands: Commands,
    ghosts: Query<(Entity, &Avatar), Without<ShadowsOff>>,
    children: Query<&Children>,
    meshes: Query<(), With<Mesh3d>>,
) {
    for (e, a) in &ghosts {
        if a.id != GHOST_ID {
            continue;
        }
        for d in children.iter_descendants(e) {
            if meshes.contains(d) {
                commands
                    .entity(d)
                    .insert((bevy::light::NotShadowCaster, bevy::light::NotShadowReceiver));
            }
        }
        commands.entity(e).insert(ShadowsOff);
    }
}

/// A mesh of an avatar's model: of the near model (posed, close by) or
/// of the far model (`lod1`).
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LodPart {
    pub far: bool,
}

/// Which of an avatar's two models currently casts its shadow.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShadowLod {
    pub far: bool,
}

/// Whether an avatar at `distance` m from the camera casts its shadow
/// with the far model (else with the near one).
///
/// The two models cross-fade between `LOD_DISTANCE_M ∓ LOD_FADE_M`; Bevy
/// dithers that fade only in the camera's passes. The shadow pass of an
/// opaque mesh has no fragment shader, so there both models would cast a
/// full shadow: two shadows of one rider (differently posed, the far one
/// seated and still). So exactly one model casts, switched at the middle
/// of the fade, where both are in range.
pub fn shadow_from_far_model(distance: f32) -> bool {
    distance >= LOD_DISTANCE_M
}

/// Lets exactly one of each avatar's two models cast a shadow
/// ([`shadow_from_far_model`]). The ghost casts none at all
/// ([`ghost_casts_no_shadow`]).
fn lod_shadows(
    mut commands: Commands,
    cameras: Query<&GlobalTransform, With<crate::camera::FollowCamera>>,
    avatars: Query<(Entity, &Avatar, &GlobalTransform, Option<&ShadowLod>)>,
    children: Query<&Children>,
    parts: Query<&LodPart>,
) {
    let eye = cameras.iter().next().map(GlobalTransform::translation);
    for (e, avatar, at, current) in &avatars {
        if avatar.id == GHOST_ID {
            continue;
        }
        let far = eye.is_some_and(|eye| shadow_from_far_model(eye.distance(at.translation())));
        if current.is_some_and(|c| c.far == far) {
            continue;
        }
        for d in children.iter_descendants(e) {
            let Ok(part) = parts.get(d) else { continue };
            if part.far == far {
                commands.entity(d).remove::<bevy::light::NotShadowCaster>();
            } else {
                commands.entity(d).insert(bevy::light::NotShadowCaster);
            }
        }
        commands.entity(e).insert(ShadowLod { far });
    }
}

#[derive(Resource, Default)]
pub(crate) struct RiderEntities(pub(crate) BTreeMap<RiderId, Entity>);

/// Despawns all avatars (a ride switch, [`crate::rebuild_scene`]): the
/// riders of the new session are spawned afresh.
pub(crate) fn reset_avatars(world: &mut World) {
    let all = world
        .get_resource_mut::<RiderEntities>()
        .map(|mut r| std::mem::take(&mut r.0))
        .unwrap_or_default();
    for e in all.into_values() {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    crate::ragdoll::reset(world);
}

/// The time trial's ghost (spec §2.3) is drawn as a rider with this id.
pub const GHOST_ID: RiderId = RiderId(u32::MAX);
/// The ghost's colour: a pale, translucent blue.
pub const GHOST_COLOR: Color = Color::srgba(0.55, 0.8, 1.0, 0.35);

/// Jersey colour of the local rider: #FF1472.
pub const LOCAL_JERSEY: Color = Color::srgb(1.0, 20.0 / 255.0, 114.0 / 255.0);

/// Jersey colour: the local rider's own colour, greens to violets for others.
pub fn jersey_color(id: RiderId, is_local: bool) -> Color {
    if is_local {
        LOCAL_JERSEY
    } else {
        // Green to violet only (150°–290°), never the local rider's pink.
        Color::hsl(150.0 + (id.0 as f32 * 47.0) % 140.0, 0.6, 0.45)
    }
}

/// Position and orientation of a rider on the route.
pub fn rider_transform(track: &dyn Track, distance: f64, lateral_offset: f64) -> Transform {
    let p = track.sample(distance);
    let pos = to_vec3(p.offset_position(lateral_offset));
    // Bevy models face -Z; the route direction is (x, z).
    let forward = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let pitch = (p.grade as f32).atan();
    let yaw = Transform::default().looking_to(forward, Vec3::Y).rotation;
    Transform::from_translation(pos).with_rotation(yaw * Quat::from_rotation_x(pitch))
}

/// Distance beside the road over which the avatar goes from the road's
/// height onto the ground (m).
pub const OFF_ROAD_BLEND_M: f64 = 2.0;

/// Largest lean angle (radians, ~35°).
const MAX_LEAN: f64 = 0.61;

/// Lean into a curve (radians, positive = to the right): a bike in a
/// steady turn leans by atan(v² κ / g).
pub fn lean_angle(curvature: f64, speed_mps: f64) -> f64 {
    (speed_mps * speed_mps * curvature / 9.81)
        .atan()
        .clamp(-MAX_LEAN, MAX_LEAN)
}

/// Rotation (about the travel direction) that leans the avatar; Bevy
/// models face -Z, so a lean to the right is a negative rotation about Z.
pub fn lean_rotation(lean: f64) -> Quat {
    Quat::from_rotation_z(-lean as f32)
}

/// Height of the surface under a point `along` metres ahead of the rider
/// (along its heading): the road surface as drawn there (the profile plus
/// [`crate::ROAD_SURFACE_LIFT_M`]), blended onto the ground beside the
/// road over [`OFF_ROAD_BLEND_M`] (no jump at the edge, no wheels in an
/// embankment).
fn contact_height(
    track: &dyn Track,
    state: &ocw_sim::RiderState,
    ground: Option<&Ground>,
    along: f64,
) -> f32 {
    let d = state.distance + along * state.heading.cos();
    let lateral = state.lateral_offset + along * state.heading.sin();
    let p = to_vec3(track.sample(d).offset_position(lateral));
    let beyond = lateral.abs() - track.road_width_at(d) / 2.0;
    let mut y = p.y + crate::ROAD_SURFACE_LIFT_M;
    if beyond > 0.0
        && let Some(h) = ground.and_then(|g| (g.0)(f64::from(p.x), f64::from(p.z)))
    {
        let w = (beyond / OFF_ROAD_BLEND_M).min(1.0) as f32;
        y += (h as f32 - y) * w;
    }
    y
}

/// Height of a wheel's hub `along` metres ahead of the rider when the
/// wheel (radius `r`) rests on the surface: the highest support of the
/// circle by the surface within ±r (over a crest the tyre touches the
/// crest, in a dip one side of it), not the surface straight under the hub.
fn hub_height(
    track: &dyn Track,
    state: &ocw_sim::RiderState,
    ground: Option<&Ground>,
    along: f64,
    r: f64,
) -> f32 {
    let lift = |t: f64| {
        let dx = t - along;
        f64::from(contact_height(track, state, ground, t)) + (r * r - dx * dx).max(0.0).sqrt()
    };
    // Coarse, then fine around the best coarse sample.
    let best = |from: f64, step: f64, n: i32| {
        (0..=n)
            .map(|k| from + f64::from(k) * step)
            .map(|t| (t, lift(t)))
            .fold(
                (along, f64::NEG_INFINITY),
                |a, b| if b.1 > a.1 { b } else { a },
            )
    };
    let coarse = r / 16.0;
    let (t, _) = best(along - r, coarse, 32);
    let fine = coarse / 16.0;
    let (_, h) = best((t - coarse).max(along - r), fine, 32);
    h as f32
}

/// The avatar: turned by the rider's heading relative to the road, leaning
/// for the curve it rides, and with both wheels of `bike` resting on the
/// surface (road, or the ground beside it; [`hub_height`]): over a bump
/// the front wheel rises first, then the rear. The bike leans about its
/// tyres' crown line ([`BikeRig::tyre_crown`] above the contact), so a
/// leaning tyre rolls onto its side instead of cutting into the road.
pub fn avatar_transform(
    track: &dyn Track,
    state: &ocw_sim::RiderState,
    ground: Option<&Ground>,
    bike: &BikeRig,
) -> Transform {
    let mut t = rider_transform(track, state.distance, state.lateral_offset);
    let r = f64::from(bike.wheel_radius);
    let wheelbase = f64::from(bike.wheelbase);
    // The hubs are a wheelbase apart along the pitched bike: horizontally
    // a little less on a slope (a few rounds settle it).
    let mut pitch = 0.0f64;
    let (mut front, mut rear) = (0.0, 0.0);
    for _ in 0..3 {
        let half = 0.5 * wheelbase * pitch.cos();
        front = hub_height(track, state, ground, half, r);
        rear = hub_height(track, state, ground, -half, r);
        pitch = (f64::from(front - rear) / wheelbase)
            .clamp(-1.0, 1.0)
            .asin();
    }
    let p = track.sample(state.distance);
    let road = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let yaw = Transform::default().looking_to(road, Vec3::Y).rotation;
    // Positive heading = right; Bevy turns left for a positive Y rotation.
    let upright =
        Quat::from_rotation_y(-state.heading as f32) * yaw * Quat::from_rotation_x(pitch as f32);
    // The origin (the contacts' midpoint) lies a wheel radius below the
    // hubs' midpoint along the pitched bike's up direction.
    let mid = Vec3::new(t.translation.x, 0.5 * (front + rear), t.translation.z);
    t.translation = mid - upright * (Vec3::Y * bike.wheel_radius);
    let lean = lean_rotation(lean_angle(state.turn_curvature, state.speed));
    t.rotation = upright * lean;
    t.translation += upright * crown_shift(lean, bike.tyre_crown);
    t
}

/// Shift that turns a rotation about the contact line into one about the
/// tyre crown's centre line `crown` above it.
fn crown_shift(roll: Quat, crown: f32) -> Vec3 {
    let c = Vec3::Y * crown;
    c - roll * c
}

/// The sway node's transform: the bike rocking by `roll` (rad) about its
/// tyres' crown line, like the lean ([`avatar_transform`]).
pub fn sway_transform(roll: f32, bike: &BikeRig) -> Transform {
    let rotation = Quat::from_rotation_z(roll);
    Transform::from_translation(crown_shift(rotation, bike.tyre_crown)).with_rotation(rotation)
}

/// The colours an avatar is drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarColours {
    pub frame: Color,
    pub jersey: Color,
    pub helmet: Color,
    /// The kit pattern's colour: white on dark jerseys, black on light.
    pub accent: Color,
    /// The character's skin and hair colours (spec 4C).
    pub skin: Color,
    pub hair: Color,
    /// The kit design (spec §14 Phase 8 T6).
    pub kit: KitColours,
    /// The jersey of the far model: the pattern's dominant colour.
    pub far_jersey: Color,
}

/// The jersey designer's choices as drawn (spec §14 Phase 8 T6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KitColours {
    pub pattern: ocw_core::KitPattern,
    /// The pattern's second and third colour (the third is the second in
    /// a two-colour design).
    pub second: Color,
    pub third: Color,
    pub three: bool,
    pub number: u16,
    pub helmet_pattern: bool,
    /// Free kit colours (`None`: the model's).
    pub shorts: Option<Color>,
    pub socks: Option<Color>,
    pub shoes: Option<Color>,
    pub gloves: Option<Color>,
    pub helmet_finish: ocw_core::Finish,
    pub shoe_finish: ocw_core::Finish,
}

fn rgb(c: ocw_core::Rgb) -> Color {
    let [r, g, b] = c.channels();
    Color::srgb_u8(r, g, b)
}

/// The colours of a rider: their look, or (an old server without looks)
/// the jersey colour by id with a dark frame.
pub fn avatar_colours(id: RiderId, is_local: bool, look: Option<&RiderLook>) -> AvatarColours {
    let (frame, jersey, helmet) = match look {
        Some(l) => (rgb(l.frame), rgb(l.jersey), rgb(l.helmet)),
        None => {
            let j = jersey_color(id, is_local);
            (Color::srgb(0.08, 0.08, 0.09), j, j)
        }
    };
    let s = jersey.to_srgba();
    let luminance = 0.2126 * s.red + 0.7152 * s.green + 0.0722 * s.blue;
    let accent = if luminance > 0.55 {
        Color::srgb(0.06, 0.06, 0.07)
    } else {
        Color::srgb(0.96, 0.96, 0.94)
    };
    let character = look.map(|l| l.character).unwrap_or_default();
    let (kit, far_jersey) = match look {
        Some(l) => {
            let d = l.design.validated();
            let [_, second, third] = l.kit_colours();
            (
                KitColours {
                    pattern: l.pattern,
                    second: rgb(second),
                    third: rgb(third),
                    three: d.third.is_some(),
                    number: d.number,
                    helmet_pattern: d.helmet_pattern,
                    shorts: d.shorts.map(rgb),
                    socks: d.socks.map(rgb),
                    shoes: d.shoes.map(rgb),
                    gloves: d.gloves.map(rgb),
                    helmet_finish: d.helmet_finish,
                    shoe_finish: d.shoe_finish,
                },
                rgb(ocw_core::kit::dominant_jersey_colour(l)),
            )
        }
        None => (
            KitColours {
                pattern: ocw_core::KitPattern::Plain,
                second: accent,
                third: accent,
                three: false,
                number: 0,
                helmet_pattern: false,
                shorts: None,
                socks: None,
                shoes: None,
                gloves: None,
                helmet_finish: Default::default(),
                shoe_finish: Default::default(),
            },
            jersey,
        ),
    };
    AvatarColours {
        frame,
        jersey,
        helmet,
        // The zip and trims: the design's second colour.
        accent: if look.is_some() { kit.second } else { accent },
        skin: rgb(character.skin_colour()),
        hair: rgb(character.validated().hair_colour),
        kit,
        far_jersey,
    }
}

/// The colour of paint slot `slot` (model colour `model_color`, linear
/// RGBA) for a rider in `colours`.
pub fn tint_color(slot: &str, model_color: [f32; 4], colours: &AvatarColours) -> Color {
    let s = slots::get(slot).unwrap_or(&slots::ALL[0]);
    let model = Color::linear_rgba(
        model_color[0],
        model_color[1],
        model_color[2],
        model_color[3],
    );
    match s.tint {
        Tint::Fixed => model,
        Tint::Shorts => colours.kit.shorts.unwrap_or(model),
        Tint::Socks => colours.kit.socks.unwrap_or(model),
        Tint::Shoes => colours.kit.shoes.unwrap_or(model),
        Tint::Gloves => colours.kit.gloves.unwrap_or(model),
        Tint::Frame => colours.frame,
        Tint::Jersey => colours.jersey,
        Tint::Helmet => colours.helmet,
        Tint::Accent => colours.accent,
        Tint::Skin => colours.skin,
        Tint::Hair => colours.hair,
    }
}

/// The rider model's nodes a look leaves out (spec 4C): the optional
/// nodes ([`ocw_assets::rider::optional`]) it does not show.
pub fn hidden_rider_nodes(look: Option<&RiderLook>, kit: &[String]) -> Vec<&'static str> {
    let l = look.copied().unwrap_or_default();
    let ids: Vec<&str> = kit.iter().map(String::as_str).collect();
    let shown = ocw_assets::rider::visible(&l.character, &ids);
    ocw_assets::rider::optional()
        .into_iter()
        .filter(|n| !shown.contains(n))
        .collect()
}

/// Whether rider `id` is a unicorn now (god mode, spec §14 Phase 13C):
/// the avatar is hidden once the transformation has turned the rider.
pub fn flying(view: &RideView, id: RiderId) -> bool {
    view.unicorns.iter().any(|u| u.id == id && u.turned())
}

/// What an avatar was built from: rebuilt when it changes.
#[derive(Component, Debug, Clone, PartialEq)]
struct AvatarLook {
    bike: ProfileId,
    look: Option<RiderLook>,
    /// The model variants of the fitted parts ([`part_variants`]).
    variants: Vec<String>,
    /// The rider variants of the worn kit ([`kit_variants`]).
    kit: Vec<String>,
}

/// The bike model variants (`ocw_assets::bikes::VARIANTS`) of the parts
/// `parts` (catalogue ids; spec §14 Phase 4, 4B): base parts and frames
/// are the profile's own model; unknown ids are skipped. Sorted.
pub fn part_variants(parts: &[String]) -> Vec<String> {
    let cat = ocw_assets::shipped::catalogue();
    let mut v: Vec<String> = parts
        .iter()
        .filter_map(|id| cat.get(id))
        .filter(|p| !p.slot.is_kit())
        .map(|p| p.model.clone())
        .filter(|m| m != "stock" && !m.starts_with("bike-"))
        .collect();
    v.sort();
    v.dedup();
    v
}

/// The rider model variants (`ocw_assets::rider::VARIANTS`) of the worn
/// kit among `parts` (catalogue ids; spec 4C): base items and unknown ids
/// are skipped. Sorted.
pub fn kit_variants(parts: &[String]) -> Vec<String> {
    let cat = ocw_assets::shipped::catalogue();
    let mut v: Vec<String> = parts
        .iter()
        .filter_map(|id| cat.get(id))
        .filter(|p| p.slot.is_kit() && !p.is_base())
        .map(|p| p.model.clone())
        .filter(|m| ocw_assets::rider::variant(m).is_some())
        .collect();
    v.sort();
    v.dedup();
    v
}

#[allow(clippy::too_many_arguments)]
fn sync_riders(
    mut commands: Commands,
    view: Res<RideView>,
    mut entities: ResMut<RiderEntities>,
    mut avatars: Query<(&mut Transform, &AvatarLook, &mut Visibility), With<Avatar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut gear_materials: ResMut<Assets<GearMaterial>>,
    ground: Option<Res<Ground>>,
    ragdolls: Option<Res<crate::ragdoll::Ragdolls>>,
    mut library: Local<Option<Library>>,
) {
    let lib = library.get_or_insert_with(|| Library::new(&mut meshes));
    // The ghost rides on the local rider's track.
    let ghost = view
        .ghost
        .zip(view.local())
        .map(|(g, l)| (GHOST_ID, g, l.id));
    // Remove riders that left.
    let present: Vec<RiderId> = view
        .riders
        .iter()
        .map(|r| r.id)
        .chain(ghost.map(|g| g.0))
        .collect();
    entities.0.retain(|id, e| {
        let keep = present.contains(id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    // The ghost: the local rider's bike, all translucent.
    let all = view
        .riders
        .iter()
        .map(|r| {
            let variants = part_variants(&r.parts);
            let kit = kit_variants(&r.parts);
            (
                r.id, r.state, r.id, r.is_local, r.bike, r.look, variants, kit, r.wobble,
            )
        })
        .chain(ghost.and_then(|(id, state, on)| {
            let local = view.local()?;
            Some((
                id,
                state,
                on,
                false,
                local.bike,
                None,
                Vec::new(),
                Vec::new(),
                0.0,
            ))
        }));
    for (id, state, track_of, is_local, bike, look, variants, kit, wobble) in all {
        let rig = &lib.bike_rigs[profile_index(bike)];
        let mut t = avatar_transform(view.track(track_of), &state, ground.as_deref(), rig);
        // The wobble after a soft bump (spec §14 Phase 12B): a lean swing.
        if wobble != 0.0 {
            t.rotation *= lean_rotation(wobble);
        }
        // Falling or down as a ragdoll, or remounting: the ragdoll shows
        // the rider (spec §14 Phase 12C).
        let ragdoll = ragdolls.as_ref().is_some_and(|r| r.hidden.contains(&id));
        let wanted = AvatarLook {
            bike,
            look,
            variants,
            kit,
        };
        if let Some(e) = entities.0.get(&id).copied() {
            match avatars.get_mut(e) {
                Ok((mut tr, built, mut vis)) if *built == wanted => {
                    *tr = t;
                    // God mode (spec §14 Phase 13C): the unicorn is the rider.
                    let shown = if flying(&view, id) || ragdoll {
                        Visibility::Hidden
                    } else {
                        Visibility::Inherited
                    };
                    vis.set_if_neq(shown);
                    continue;
                }
                // Another bike or look: build the avatar again.
                _ => commands.entity(e).despawn(),
            }
        }
        let e = spawn_avatar(
            &mut commands,
            lib,
            &mut meshes,
            &mut Mats {
                standard: &mut materials,
                gear: &mut gear_materials,
            },
            id,
            is_local,
            wanted,
            t,
        );
        if ragdoll {
            // A fresh avatar for the remount: hidden until it takes over.
            commands.entity(e).insert(Visibility::Hidden);
        }
        entities.0.insert(id, e);
    }
}

/// The ghost's material: translucent, lit only a little (it glows
/// faintly), so it reads as "not really there".
pub fn ghost_material() -> StandardMaterial {
    StandardMaterial {
        base_color: GHOST_COLOR,
        emissive: LinearRgba::rgb(0.08, 0.16, 0.25),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.8,
        ..default()
    }
}

/// Distance from the camera at which an avatar switches to its far model
/// (`lod1`: bike and seated rider in a few meshes), m. Beyond it the
/// avatar is not posed either (only its wheels' and cranks' angles go on).
pub const LOD_DISTANCE_M: f32 = 40.0;
/// Width of the cross-fade between the two models (m).
const LOD_FADE_M: f32 = 4.0;

fn near_range() -> VisibilityRange {
    VisibilityRange {
        start_margin: 0.0..0.0,
        end_margin: LOD_DISTANCE_M - LOD_FADE_M..LOD_DISTANCE_M + LOD_FADE_M,
        use_aabb: false,
    }
}

fn far_range() -> VisibilityRange {
    VisibilityRange {
        start_margin: LOD_DISTANCE_M - LOD_FADE_M..LOD_DISTANCE_M + LOD_FADE_M,
        end_margin: 1.0e7..1.0e7,
        use_aabb: false,
    }
}

/// A model's meshes, uploaded once: `(mesh, material index)` per primitive,
/// per node. Near parts carry their rest coordinates for the gear shader
/// (spec §14 Phase 8 T6; `rider`: the rider model's rest pose and body
/// regions), the far LOD only positions and normals.
struct Meshes(Vec<Vec<(Handle<Mesh>, usize)>>);

impl Meshes {
    fn new(model: &Model, meshes: &mut Assets<Mesh>, rider: bool) -> Self {
        let far = model.node(bike_nodes::LOD1);
        Self(
            model
                .nodes
                .iter()
                .enumerate()
                .map(|(i, n)| {
                    let is_far = far.is_some_and(|f| model.is_below(i, f));
                    let (rest, region) = gear::rest_frame(model, i, rider);
                    n.primitives
                        .iter()
                        .map(|p| {
                            let m = if is_far {
                                plain_mesh(p)
                            } else {
                                gear::gear_mesh(p, rest, region)
                            };
                            (meshes.add(m), p.material)
                        })
                        .collect()
                })
                .collect(),
        )
    }
}

/// A mesh with positions and normals only (the far LOD).
fn plain_mesh(p: &ocw_assets::model::Primitive) -> Mesh {
    let mut m = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
    m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
    m.insert_indices(Indices::U32(p.indices.clone()));
    m
}

/// The material assets an avatar is made with.
struct Mats<'a> {
    standard: &'a mut Assets<StandardMaterial>,
    gear: &'a mut Assets<GearMaterial>,
}

/// A part's material: the gear shader up close, a plain one far away and
/// for the ghost.
#[derive(Debug, Clone, PartialEq)]
pub enum PartMaterial {
    Gear(Handle<GearMaterial>),
    Plain(Handle<StandardMaterial>),
}

/// The shipped models on the GPU, their rigs, and the materials by slot
/// and colour (riders in the same colours share them).
struct Library {
    rider: Meshes,
    bikes: Vec<Meshes>,
    rider_rig: RiderRig,
    bike_rigs: Vec<BikeRig>,
    /// Bikes with fitted parts (the others' and the own, spec §14 Phase 4,
    /// 4G), built and uploaded once per profile and variant set.
    fitted: HashMap<(ProfileId, Vec<String>), (std::sync::Arc<Model>, Meshes)>,
    /// Far materials by slot and colour; gear materials by surface.
    materials: HashMap<(&'static str, [u8; 4]), Handle<StandardMaterial>>,
    gear: HashMap<Vec<u32>, Handle<GearMaterial>>,
    /// Near parts without a shader of their own, by surface.
    plain_near: HashMap<Vec<u32>, Handle<StandardMaterial>>,
    ghost: Option<Handle<StandardMaterial>>,
}

impl Library {
    fn new(meshes: &mut Assets<Mesh>) -> Self {
        let rider_rig = ocw_assets::shipped::rider_rig();
        let bike_rigs: Vec<BikeRig> = ProfileId::ALL
            .iter()
            .map(|p| ocw_assets::shipped::bike_rig(*p))
            .collect();
        Self {
            rider: Meshes::new(ocw_assets::shipped::rider_model(), meshes, true),
            bikes: ProfileId::ALL
                .iter()
                .map(|p| Meshes::new(ocw_assets::shipped::bike_model(*p), meshes, false))
                .collect(),
            rider_rig,
            bike_rigs,
            fitted: HashMap::new(),
            materials: HashMap::new(),
            gear: HashMap::new(),
            plain_near: HashMap::new(),
            ghost: None,
        }
    }

    /// The material of a part in paint slot `slot` on node `node` (model
    /// colour `model_color`, linear RGBA) for a rider in `colours`: the
    /// gear shader near, a plain material on the far LOD and the ghost.
    #[allow(clippy::too_many_arguments)]
    fn material(
        &mut self,
        mats: &mut Mats,
        slot: &str,
        node: &str,
        model_color: [f32; 4],
        colours: &AvatarColours,
        ghost: bool,
        far: bool,
    ) -> PartMaterial {
        if ghost {
            return PartMaterial::Plain(
                self.ghost
                    .get_or_insert_with(|| mats.standard.add(ghost_material()))
                    .clone(),
            );
        }
        let surface = gear::surface(slot, node, model_color, colours);
        if far || gear::off() {
            let s = slots::get(slot).unwrap_or(&slots::ALL[0]);
            let m = surface.far_material(colours, slot);
            return PartMaterial::Plain(
                self.materials
                    .entry((s.name, m.base_color.to_srgba().to_u8_array()))
                    .or_insert_with(|| mats.standard.add(m))
                    .clone(),
            );
        }
        // Painted, plain and plastic parts need no shader of their own:
        // the standard material (clear coat, finish) batches across riders.
        if !surface.needs_shader() {
            return PartMaterial::Plain(
                self.plain_near
                    .entry(surface.key(false))
                    .or_insert_with(|| mats.standard.add(surface.plain_material()))
                    .clone(),
            );
        }
        PartMaterial::Gear(
            self.gear
                .entry(surface.key(false))
                .or_insert_with(|| mats.gear.add(surface.material(false)))
                .clone(),
        )
    }
}

impl Library {
    /// The bike of `profile` with the part `variants` (none: the shipped
    /// model) and its meshes.
    fn bike(
        &mut self,
        profile: ProfileId,
        variants: &[String],
        meshes: &mut Assets<Mesh>,
    ) -> (Option<std::sync::Arc<Model>>, Meshes) {
        if variants.is_empty() {
            return (None, Meshes(self.bikes[profile_index(profile)].0.clone()));
        }
        let (m, list) = self
            .fitted
            .entry((profile, variants.to_vec()))
            .or_insert_with(|| {
                static LOW: std::sync::OnceLock<Model> = std::sync::OnceLock::new();
                let low =
                    LOW.get_or_init(|| ocw_assets::rider::build(ocw_assets::rider::Detail::Low));
                let ids: Vec<&str> = variants.iter().map(String::as_str).collect();
                let m = ocw_assets::bikes::build_with(
                    profile,
                    ocw_assets::shipped::rider_model(),
                    low,
                    &ids,
                );
                let list = Meshes::new(&m, meshes, false);
                (std::sync::Arc::new(m), list)
            });
        (Some(m.clone()), Meshes(list.0.clone()))
    }
}

fn profile_index(p: ProfileId) -> usize {
    ProfileId::ALL.iter().position(|q| *q == p).unwrap_or(0)
}

/// The entities an avatar's pose is written to, and its animation state.
#[derive(Component)]
pub struct AvatarRig {
    /// Rocks the bike and rider (bike roll about the ground contact line).
    sway: Entity,
    wheels: [Entity; 2],
    pelvis: Entity,
    /// In the order of [`ocw_assets::pose::Pose::rotations`].
    posed: Vec<Entity>,
    bike: BikeRig,
    /// The rider's rig at the body height, and that scale (spec 4C).
    rider: RiderRig,
    scale: f32,
    fit: Fit,
    selector: Selector,
    params: PoseParams,
    wheel_angle: f32,
    bias: f64,
}

impl AvatarRig {
    /// The riding state shown (tests, HUD).
    pub fn riding(&self) -> Riding {
        self.selector.state
    }

    pub fn params(&self) -> PoseParams {
        self.params
    }
}

/// Spawns `model`'s nodes (but not the subtrees of `skip`) under `parent`;
/// returns the entity of every node (PLACEHOLDER for skipped ones).
#[allow(clippy::too_many_arguments)]
fn spawn_model(
    commands: &mut Commands,
    parent: Entity,
    model: &Model,
    meshes: &Meshes,
    skip: &[usize],
    hidden: &[usize],
    far: Option<usize>,
    build: ocw_core::BodyBuild,
    mut material: impl FnMut(&str, &str, [f32; 4], bool) -> PartMaterial,
) -> Vec<Entity> {
    let mut out = vec![Entity::PLACEHOLDER; model.nodes.len()];
    for (i, node) in model.nodes.iter().enumerate() {
        if skip.iter().any(|&s| model.is_below(i, s)) {
            continue;
        }
        let up = node.parent.map_or(parent, |p| out[p]);
        let visibility = if hidden.contains(&i) {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        let e = commands
            .spawn((
                Transform {
                    translation: node.translation,
                    rotation: node.rotation,
                    scale: node.scale,
                },
                visibility,
                ChildOf(up),
                // The ragdoll finds the segments by name (spec §14 Phase 12C).
                Name::new(node.name.clone()),
            ))
            .id();
        out[i] = e;
        let is_far = far.is_some_and(|f| model.is_below(i, f));
        let range = if is_far { far_range() } else { near_range() };
        // The body build's girth (the rider model; 1 for bikes).
        let girth = ocw_assets::rider::girth(model, i, build);
        for (mesh, mat) in &meshes.0[i] {
            let m = &model.materials[*mat];
            let mut part = commands.spawn((
                Mesh3d(mesh.clone()),
                Transform::from_scale(girth),
                range.clone(),
                LodPart { far: is_far },
                ChildOf(e),
            ));
            match material(&m.name, &node.name, m.color, is_far) {
                PartMaterial::Gear(h) => part.insert(MeshMaterial3d(h)),
                PartMaterial::Plain(h) => part.insert(MeshMaterial3d(h)),
            };
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn spawn_avatar(
    commands: &mut Commands,
    lib: &mut Library,
    meshes: &mut Assets<Mesh>,
    materials: &mut Mats,
    id: RiderId,
    is_local: bool,
    built: AvatarLook,
    transform: Transform,
) -> Entity {
    let colours = avatar_colours(id, is_local, built.look.as_ref());
    let ghost = id == GHOST_ID;
    let b = profile_index(built.bike);
    // The fitted parts' variants (the stock bike: the shipped model).
    let (fitted, bike_list) = lib.bike(built.bike, &built.variants, meshes);
    let bike_model: &Model = fitted
        .as_deref()
        .unwrap_or_else(|| ocw_assets::shipped::bike_model(built.bike));
    let rider_model = ocw_assets::shipped::rider_model();
    let character = built.look.map(|l| l.character).unwrap_or_default();
    // The body height (spec 4C): the rider scaled, the saddle moved so the
    // legs fit (the bikes are made for the 1.78 m rider).
    let body = pose::BodyFit::new(&lib.rider_rig, &lib.bike_rigs[b], character.scale());
    let bike = body.bike;
    let root = commands
        .spawn((
            transform,
            Visibility::default(),
            Avatar { id, is_local },
            built.clone(),
        ))
        .id();
    let sway = commands
        .spawn((Transform::default(), Visibility::default(), ChildOf(root)))
        .id();
    let far = bike_model.node(bike_nodes::LOD1);
    // The ghost needs no far model: it is never far from the local rider.
    let skip: Vec<usize> = if ghost {
        far.into_iter().collect()
    } else {
        Vec::new()
    };
    // Handles are cheap to clone; the copies free `lib` for the materials.
    let rider_list = Meshes(lib.rider.0.clone());
    let bike_nodes_e = spawn_model(
        commands,
        sway,
        bike_model,
        &bike_list,
        &skip,
        &[],
        far,
        ocw_core::BodyBuild::Medium,
        |slot, node, c, is_far| lib.material(materials, slot, node, c, &colours, ghost, is_far),
    );
    // The saddle (and its post) at the rider's height.
    if body.saddle_offset != 0.0
        && let Some(i) = bike_model.node(bike_nodes::SADDLE)
    {
        let to = bike_model.nodes[i].translation + body.bike.seat_dir() * body.saddle_offset;
        commands
            .entity(bike_nodes_e[i])
            .insert(Transform::from_translation(to));
    }
    // The look's optional nodes (spec 4C): the worn kit (no item: the base
    // kit), the kit pattern, hair, face and glasses.
    let skip_rider: Vec<usize> = hidden_rider_nodes(built.look.as_ref(), &built.kit)
        .into_iter()
        .filter_map(|n| rider_model.node(n))
        .collect();
    let rider_e = spawn_model(
        commands,
        sway,
        rider_model,
        &rider_list,
        &skip_rider,
        &[],
        None,
        character.validated().build,
        |slot, node, c, is_far| lib.material(materials, slot, node, c, &colours, ghost, is_far),
    );
    // The body height: the whole rider scaled about the hips.
    if body.scale != 1.0
        && let Some(i) = rider_model.node("rider")
    {
        commands
            .entity(rider_e[i])
            .insert(Transform::from_scale(Vec3::splat(body.scale)));
    }
    let find = |name: &str| -> Entity {
        if let Some(i) = rider_model.node(name) {
            return rider_e[i];
        }
        bike_model
            .node(name)
            .map_or(Entity::PLACEHOLDER, |i| bike_nodes_e[i])
    };
    let posed = pose::pose(&body.rider, &bike, &body.fit, &PoseParams::default())
        .rotations()
        .iter()
        .map(|(name, _)| find(name))
        .collect();
    let rig = AvatarRig {
        sway,
        wheels: [find(bike_nodes::WHEEL_FRONT), find(bike_nodes::WHEEL_REAR)],
        pelvis: find(nodes::PELVIS),
        posed,
        bike,
        rider: body.rider,
        scale: body.scale,
        fit: body.fit,
        selector: Selector::default(),
        params: PoseParams::default(),
        wheel_angle: 0.0,
        bias: anim::bias(id.0),
    };
    commands.entity(root).insert(rig);
    root
}

/// Animates the avatars from the ride: riding state, cranks, wheels, pose.
fn animate(
    time: Res<Time>,
    view: Res<RideView>,
    mut avatars: Query<
        (&Avatar, &mut AvatarRig, &GlobalTransform),
        Without<crate::ragdoll::Ragdolled>,
    >,
    cameras: Query<&GlobalTransform, With<crate::camera::FollowCamera>>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    let eye = cameras.iter().next().map(GlobalTransform::translation);
    for (avatar, mut rig, at) in &mut avatars {
        let input = if avatar.id == GHOST_ID {
            let Some(g) = view.ghost else { continue };
            let grade = view
                .local()
                .map_or(0.0, |l| view.track(l.id).sample(g.distance).grade);
            AnimInput {
                speed_mps: g.speed,
                cadence_rpm: None,
                power_w: if g.speed > 1.0 { 200.0 } else { 0.0 },
                grade,
                bias: 0.0,
            }
        } else {
            let Some(r) = view.riders.iter().find(|r| r.id == avatar.id) else {
                continue;
            };
            AnimInput {
                speed_mps: r.state.speed,
                cadence_rpm: r.cadence_rpm,
                power_w: r.power_w,
                grade: view.track(r.id).sample(r.state.distance).grade,
                bias: rig.bias,
            }
        };
        let rig = &mut *rig;
        let state = rig.selector.update(&input, f64::from(dt));
        let target = anim::targets(state, input.speed_mps, input.grade, rig.bike.aero_bars);
        anim::blend(&mut rig.params, &target, dt);
        let cadence = anim::cadence(state, input.cadence_rpm);
        rig.params.crank = anim::advance_crank(rig.params.crank, cadence, dt);
        rig.wheel_angle =
            anim::advance_wheel(rig.wheel_angle, input.speed_mps, rig.bike.wheel_radius, dt);
        let spin = Quat::from_rotation_x(-rig.wheel_angle);
        for w in rig.wheels {
            if let Ok(mut t) = transforms.get_mut(w) {
                t.rotation = spin;
            }
        }
        // Far away only the lod1 model shows: no pose needed.
        if eye.is_some_and(|e| e.distance(at.translation()) > LOD_DISTANCE_M + LOD_FADE_M) {
            continue;
        }
        let p = pose::pose(&rig.rider, &rig.bike, &rig.fit, &rig.params);
        if let Ok(mut t) = transforms.get_mut(rig.sway) {
            *t = sway_transform(p.bike_roll, &rig.bike);
        }
        if let Ok(mut t) = transforms.get_mut(rig.pelvis) {
            // The pelvis in the bike frame, under the scaled rider root.
            t.translation = p.pelvis.0 / rig.scale;
            t.rotation = p.pelvis.1;
        }
        for ((_, q), e) in p.rotations().iter().zip(&rig.posed) {
            if let Ok(mut t) = transforms.get_mut(*e) {
                t.rotation = *q;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use ocw_world::Route;

    use super::*;

    fn road_rig() -> BikeRig {
        ocw_assets::shipped::bike_rig(ProfileId::Road)
    }

    fn view(points: Vec<[f64; 3]>) -> RideView {
        RideView::new(Route::from_points("t", "T", points, false, 6.0).unwrap())
    }

    fn close(a: Vec3, b: Vec3) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn the_ghost_is_a_translucent_rider_on_the_local_track() {
        let m = ghost_material();
        assert_eq!(m.alpha_mode, AlphaMode::Blend);
        assert!(m.base_color.alpha() < 0.5);
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<GearMaterial>>()
            .insert_resource(view(vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0]]))
            .add_plugins(RidersPlugin);
        let local = crate::RiderView {
            id: RiderId(1),
            name: "me".into(),
            state: ocw_sim::RiderState {
                distance: 10.0,
                ..Default::default()
            },
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: true,
            bike: Default::default(),
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
        };
        {
            let mut v = app.world_mut().resource_mut::<RideView>();
            v.riders = vec![local];
            v.ghost = Some(ocw_sim::RiderState {
                distance: 42.0,
                lateral_offset: 1.0,
                speed: 9.0,
                ..Default::default()
            });
        }
        app.update();
        let mut q = app.world_mut().query::<(&Avatar, &Transform)>();
        let ghost: Vec<Vec3> = q
            .iter(app.world())
            .filter(|(a, _)| a.id == GHOST_ID)
            .map(|(_, t)| t.translation)
            .collect();
        assert_eq!(ghost.len(), 1);
        assert!(
            close(ghost[0], Vec3::new(42.0, crate::ROAD_SURFACE_LIFT_M, 1.0)),
            "{ghost:?}"
        );
        // No ghost part casts or receives a shadow; the local rider's
        // receive one, and its near model (no camera: close by) casts it.
        let mut parts = app.world_mut().query_filtered::<(
            &ChildOf,
            Has<bevy::light::NotShadowCaster>,
            Has<bevy::light::NotShadowReceiver>,
            &LodPart,
        ), With<Mesh3d>>();
        let mut avatars = app.world_mut().query::<&Avatar>();
        let root = |w: &World, mut e: Entity| {
            while let Some(p) = w.get::<ChildOf>(e) {
                e = p.parent();
            }
            e
        };
        let (mut ghost_parts, mut own_parts) = (0, 0);
        let rows: Vec<_> = parts
            .iter(app.world())
            .map(|(c, caster, receiver, lod)| (c.parent(), caster, receiver, lod.far))
            .collect();
        for (parent, caster, receiver, far) in rows {
            let a = avatars.get(app.world(), root(app.world(), parent)).unwrap();
            if a.id == GHOST_ID {
                assert!(caster && receiver);
                ghost_parts += 1;
            } else {
                assert_eq!(caster, far, "only one model casts");
                assert!(!receiver);
                own_parts += 1;
            }
        }
        assert!(
            ghost_parts > 10 && own_parts > 10,
            "{ghost_parts} {own_parts}"
        );
        // Gone with the view's ghost.
        app.world_mut().resource_mut::<RideView>().ghost = None;
        app.update();
        let mut q = app.world_mut().query::<&Avatar>();
        assert_eq!(q.iter(app.world()).count(), 1);
    }

    #[test]
    fn local_rider_wears_ff1472() {
        let c = jersey_color(RiderId(7), true).to_srgba();
        let hex = (
            (c.red * 255.0).round() as u8,
            (c.green * 255.0).round() as u8,
            (c.blue * 255.0).round() as u8,
        );
        assert_eq!(hex, (0xFF, 0x14, 0x72));
        // Other riders are never pink/red (hue 150°–290°).
        for id in 0..200 {
            let hue = Hsla::from(jersey_color(RiderId(id), false)).hue;
            assert!((150.0..=290.0).contains(&hue), "rider {id}: hue {hue}");
        }
    }

    #[test]
    fn riders_wear_their_look() {
        let look = RiderLook {
            jersey: ocw_core::Rgb(0x1E88E5),
            frame: ocw_core::Rgb(0xFDD835),
            ..Default::default()
        };
        let c = avatar_colours(RiderId(3), false, Some(&look));
        assert_eq!(c.jersey, Color::srgb_u8(0x1E, 0x88, 0xE5));
        assert_eq!(c.frame, Color::srgb_u8(0xFD, 0xD8, 0x35));
        // White pattern on the dark blue jersey, black on a light one
        // (the design's automatic second colour, ocw_core::kit::contrast).
        assert_eq!(c.accent.to_srgba().to_u8_array(), [0xF5, 0xF5, 0xF0, 0xFF]);
        let light = RiderLook {
            jersey: ocw_core::Rgb(0xF2F2EE),
            ..look
        };
        assert_eq!(
            avatar_colours(RiderId(3), false, Some(&light))
                .accent
                .to_srgba()
                .to_u8_array(),
            [0x0F, 0x0F, 0x12, 0xFF]
        );
        // A chosen second colour is the accent; the kit colours apply to
        // their slots, the others keep the model's.
        let designed = RiderLook {
            design: ocw_core::KitDesign {
                second: Some(ocw_core::Rgb(0xE53935)),
                socks: Some(ocw_core::Rgb(0x43A047)),
                ..Default::default()
            },
            ..look
        };
        let d = avatar_colours(RiderId(3), false, Some(&designed));
        assert_eq!(d.accent, Color::srgb_u8(0xE5, 0x39, 0x35));
        assert_eq!(
            tint_color("sock", [1.0; 4], &d),
            Color::srgb_u8(0x43, 0xA0, 0x47)
        );
        assert_eq!(
            tint_color("shorts", [0.1, 0.1, 0.1, 1.0], &d),
            Color::linear_rgba(0.1, 0.1, 0.1, 1.0)
        );
        // No look (old server): the colour by id, as before.
        assert_eq!(
            avatar_colours(RiderId(3), false, None).jersey,
            jersey_color(RiderId(3), false)
        );
        // The default look is the local rider's #FF1472.
        assert_eq!(
            avatar_colours(RiderId(1), true, Some(&RiderLook::default())).jersey,
            LOCAL_JERSEY
        );
    }

    /// A changed bike or look rebuilds the avatar; the same one keeps it.
    #[test]
    fn avatars_are_rebuilt_when_the_look_changes() {
        let route = Route::test_loop();
        let mut view = RideView::new(route);
        view.riders.push(crate::RiderView {
            id: RiderId(1),
            name: "a".into(),
            state: ocw_sim::RiderState {
                distance: 10.0,
                ..Default::default()
            },
            power_w: 0.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: false,
            bike: ProfileId::Road,
            look: None,
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<GearMaterial>()
            .insert_resource(view)
            .add_plugins(RidersPlugin);
        app.update();
        let entity = |app: &App| app.world().resource::<RiderEntities>().0[&RiderId(1)];
        let first = entity(&app);
        app.update();
        assert_eq!(entity(&app), first);
        app.world_mut().resource_mut::<RideView>().riders[0].bike = ProfileId::Mountain;
        app.update();
        let second = entity(&app);
        assert_ne!(second, first);
        let built = app.world().get::<AvatarLook>(second).unwrap().clone();
        assert_eq!(built.bike, ProfileId::Mountain);
        assert!(built.variants.is_empty());
        // Fitted parts (spec §14 Phase 4, 4G): the others see them – a
        // new avatar with the parts' variants, other meshes than stock.
        let stock_meshes = shown_meshes(&mut app, second).len();
        app.world_mut().resource_mut::<RideView>().riders[0].parts = vec![
            "wheels.mtb.base".into(),
            "tyres.mtb.xc".into(),
            "lights.set".into(),
            "nonsense".into(),
        ];
        app.update();
        let third = entity(&app);
        assert_ne!(third, second);
        let built = app.world().get::<AvatarLook>(third).unwrap().clone();
        assert_eq!(
            built.variants,
            part_variants(&app.world().resource::<RideView>().riders[0].parts)
        );
        assert!(!built.variants.is_empty());
        assert_ne!(shown_meshes(&mut app, third).len(), stock_meshes);
        // The same parts again: kept.
        app.update();
        assert_eq!(entity(&app), third);
    }

    /// Part ids map to the generator's variants; base parts and frames
    /// are the profile's own model.
    #[test]
    fn parts_map_to_their_model_variants() {
        let cat = ocw_assets::shipped::catalogue();
        let v = part_variants(&[
            "tyres.road.race".into(),
            "tyres.road.base".into(),
            "frame.road-aero.top".into(),
            "wheels.road-carbon.top".into(),
            "wheels.road-carbon.top".into(),
            "rocket".into(),
        ]);
        let expect = |id: &str| cat.get(id).unwrap().model.clone();
        let mut want = vec![expect("tyres.road.race"), expect("wheels.road-carbon.top")];
        want.sort();
        assert_eq!(v, want);
        assert!(part_variants(&[]).is_empty());
        assert!(part_variants(&["frame.road.base".into()]).is_empty());
    }

    /// New snapshots move the riders every frame (multiplayer: 10 Hz
    /// snapshots, interpolated): the avatars are moved, never respawned,
    /// and no mesh or material is added after the first frame.
    #[test]
    fn moving_riders_keep_their_avatars_and_assets() {
        let mut view = RideView::new(Route::test_loop());
        view.riders = (1..=9)
            .map(|i| rider(i, ProfileId::ALL[i as usize % ProfileId::ALL.len()], i == 1))
            .collect();
        let mut app = app(view);
        app.update();
        let entities = app.world().resource::<RiderEntities>().0.clone();
        let counts = |app: &mut App| {
            let w = app.world_mut();
            let spawned = w.query::<Entity>().iter(w).count();
            (
                w.resource::<Assets<Mesh>>().len(),
                w.resource::<Assets<StandardMaterial>>().len(),
                spawned,
            )
        };
        let before = counts(&mut app);
        for frame in 0..60 {
            for r in &mut app.world_mut().resource_mut::<RideView>().riders {
                r.state.distance += 0.2;
                r.state.lateral_offset = (f64::from(frame) * 0.1).sin();
            }
            app.update();
        }
        assert_eq!(app.world().resource::<RiderEntities>().0, entities);
        assert_eq!(counts(&mut app), before);
    }

    fn rider(id: u32, bike: ProfileId, is_local: bool) -> crate::RiderView {
        crate::RiderView {
            id: RiderId(id),
            name: format!("r{id}"),
            state: ocw_sim::RiderState {
                distance: 10.0 + f64::from(id) * 3.0,
                speed: 8.0,
                ..Default::default()
            },
            power_w: 200.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local,
            bike,
            look: Some(RiderLook {
                pattern: ocw_core::KitPattern::Band,
                ..Default::default()
            }),
            parts: Vec::new(),
            crash: None,
            wobble: 0.0,
        }
    }

    /// A headless app with the riders plugin and a fixed 20 ms time step.
    fn app(view: RideView) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .init_asset::<GearMaterial>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(20),
            ))
            .insert_resource(view)
            .add_plugins(RidersPlugin);
        app
    }

    /// The mesh entities of an avatar that are not under a hidden node.
    fn shown_meshes(app: &mut App, avatar: Entity) -> Vec<Handle<Mesh>> {
        let w = app.world_mut();
        let mut out = Vec::new();
        let mut stack = vec![avatar];
        while let Some(e) = stack.pop() {
            if w.get::<Visibility>(e) == Some(&Visibility::Hidden) {
                continue;
            }
            if let Some(m) = w.get::<Mesh3d>(e) {
                out.push(m.0.clone());
            }
            if let Some(c) = w.get::<Children>(e) {
                stack.extend(c.iter());
            }
        }
        out
    }

    /// Regression (user report 2026-11-13, "two shadows of the rider"):
    /// at every camera distance exactly one of an avatar's two models
    /// (near or far LOD) casts a shadow, all of its parts, and the switch
    /// lies where both models are in range.
    #[test]
    fn one_model_casts_the_shadow_at_every_distance() {
        assert!(near_range().end_margin.end > LOD_DISTANCE_M);
        assert!(far_range().start_margin.start < LOD_DISTANCE_M);
        assert!(!shadow_from_far_model(near_range().end_margin.start - 0.01));
        assert!(shadow_from_far_model(far_range().start_margin.end));
        let mut view = RideView::new(Route::test_loop());
        view.riders.push(rider(1, ProfileId::Road, true));
        view.riders.push(rider(2, ProfileId::TimeTrial, false));
        let mut app = app(view);
        app.add_plugins(TransformPlugin);
        let cam = app
            .world_mut()
            .spawn((
                Camera3d::default(),
                Transform::default(),
                crate::camera::FollowCamera,
            ))
            .id();
        app.update();
        let entities = app.world().resource::<RiderEntities>().0.clone();
        for d in [3.0, 20.0, 36.5, 39.9, 40.1, 43.5, 60.0, 400.0, 10.0] {
            // One avatar at a time at distance d (the others elsewhere).
            for (i, e) in entities.values().enumerate() {
                let at = app
                    .world()
                    .get::<GlobalTransform>(*e)
                    .unwrap()
                    .translation();
                app.world_mut()
                    .get_mut::<Transform>(cam)
                    .unwrap()
                    .translation = at + Vec3::new(0.0, 0.0, d);
                app.update();
                app.update();
                let mut parts =
                    app.world_mut()
                        .query::<(Entity, &LodPart, Has<bevy::light::NotShadowCaster>)>();
                let w = app.world();
                let rows: Vec<(LodPart, bool)> = parts
                    .iter(w)
                    .filter(|(p, _, _)| {
                        let mut r = *p;
                        while let Some(c) = w.get::<ChildOf>(r) {
                            r = c.parent();
                        }
                        r == *e
                    })
                    .map(|(_, l, off)| (*l, !off))
                    .collect();
                let casting = |far: bool| rows.iter().filter(|(l, c)| l.far == far && *c).count();
                let all = |far: bool| rows.iter().filter(|(l, _)| l.far == far).count();
                let far = shadow_from_far_model(d);
                assert!(all(true) > 0 && all(false) > 0, "avatar {i}");
                assert_eq!(casting(far), all(far), "d {d}: the whole model casts");
                assert_eq!(casting(!far), 0, "d {d}: the other model casts none");
                assert_eq!(w.get::<ShadowLod>(*e), Some(&ShadowLod { far }));
            }
        }
    }

    /// Every bike type is its shipped model with the rider on it; head
    /// wear and kit follow the rider and the bike.
    #[test]
    fn avatars_are_the_shipped_models() {
        let mut view = RideView::new(Route::test_loop());
        for (i, p) in ProfileId::ALL.iter().enumerate() {
            view.riders.push(rider(i as u32 + 1, *p, i == 0));
        }
        let mut app = app(view);
        app.update();
        let entities = app.world().resource::<RiderEntities>().0.clone();
        assert_eq!(entities.len(), 6);
        let rider = ocw_assets::shipped::rider_model();
        let prims = |name: &str| {
            let i = rider.node(name).unwrap();
            (0..rider.nodes.len())
                .filter(|j| rider.is_below(*j, i))
                .map(|j| rider.nodes[j].primitives.len())
                .sum::<usize>()
        };
        let full = |m: &Model| m.nodes.iter().map(|n| n.primitives.len()).sum::<usize>();
        let look = RiderLook {
            pattern: ocw_core::KitPattern::Band,
            ..Default::default()
        };
        let hidden: usize = hidden_rider_nodes(Some(&look), &[])
            .iter()
            .map(|n| prims(n))
            .sum();
        for (i, p) in ProfileId::ALL.iter().enumerate() {
            let e = entities[&RiderId(i as u32 + 1)];
            let shown = shown_meshes(&mut app, e).len();
            let bike = full(ocw_assets::shipped::bike_model(*p));
            // The whole bike (with its far LOD), the rider in the base kit
            // (the road helmet on every bike) with the band.
            let expect = bike + full(rider) - hidden;
            assert_eq!(shown, expect, "{p:?}");
            assert!(app.world().get::<AvatarRig>(e).is_some());
        }
        assert!(hidden_rider_nodes(Some(&look), &[]).contains(&nodes::HELMET_AERO));
        assert!(!hidden_rider_nodes(Some(&look), &[]).contains(&nodes::HELMET_ROAD));
    }

    /// Materials per node and LOD (spec §14 Phase 8 T6): every near part
    /// is drawn with the gear shader – its kind from slot and node, the
    /// jersey with the design's pattern and number –, every far part and
    /// the ghost with a plain material, the far jersey in the pattern's
    /// dominant colour; near meshes carry their rest coordinates.
    #[test]
    fn materials_follow_the_node_and_the_lod() {
        let mut view = RideView::new(Route::test_loop());
        let mut r = rider(1, ProfileId::Road, false);
        let look = RiderLook {
            jersey: ocw_core::Rgb(0x1E88E5),
            pattern: ocw_core::KitPattern::Stripes,
            design: ocw_core::KitDesign {
                second: Some(ocw_core::Rgb(0xF2F2EE)),
                number: 21,
                ..Default::default()
            },
            ..Default::default()
        };
        r.look = Some(look);
        view.riders.push(r);
        let mut app = app(view);
        app.update();
        let w = app.world_mut();
        let mut q = w.query::<(
            &ChildOf,
            &LodPart,
            &Mesh3d,
            Option<&MeshMaterial3d<GearMaterial>>,
            Option<&MeshMaterial3d<StandardMaterial>>,
        )>();
        let rows: Vec<_> = q
            .iter(w)
            .map(|(c, l, m, g, s)| (c.parent(), l.far, m.0.clone(), g.cloned(), s.cloned()))
            .collect();
        let (mut near, mut far, mut jerseys, mut far_jerseys) = (0, 0, 0, 0);
        let (mut plain_near, mut clear_coated) = (0, 0);
        let mut kinds = std::collections::BTreeSet::new();
        let mut saddle = std::collections::BTreeSet::new();
        for (parent, is_far, mesh, g, s) in rows {
            let node = w
                .get::<Name>(parent)
                .map(|n| n.to_string())
                .unwrap_or_default();
            let has_uv = w
                .resource::<Assets<Mesh>>()
                .get(&mesh)
                .unwrap()
                .attribute(Mesh::ATTRIBUTE_UV_1)
                .is_some();
            if is_far {
                far += 1;
                assert!(g.is_none() && !has_uv, "{node}: far parts are plain");
                let m = w
                    .resource::<Assets<StandardMaterial>>()
                    .get(&s.expect("a plain material").0)
                    .unwrap();
                if m.base_color == Color::srgb_u8(0x1E, 0x88, 0xE5) {
                    far_jerseys += 1;
                }
            } else {
                near += 1;
                assert!(has_uv, "{node}: near meshes carry their rest coordinates");
                let Some(g) = g else {
                    // Painted, plain and plastic parts: the standard material.
                    let m = w
                        .resource::<Assets<StandardMaterial>>()
                        .get(&s.expect("a material").0)
                        .unwrap();
                    plain_near += 1;
                    if m.clearcoat > 0.0 {
                        clear_coated += 1;
                    }
                    continue;
                };
                let m = w.resource::<Assets<GearMaterial>>().get(&g.0).unwrap();
                let l = m.extension.look;
                kinds.insert(l.kind());
                if l.mode.y == 1.0 {
                    jerseys += 1;
                    assert_eq!(l.mode.z, ocw_core::KitPattern::Stripes.wire() as f32);
                    assert_eq!(l.params.x, 21.0, "the race number");
                }
                if node == ocw_assets::pose::bike_nodes::SADDLE {
                    saddle.insert(l.kind());
                }
            }
        }
        assert!(near > 20 && far > 3, "{near} {far}");
        // The saddle: leather (its rails: metal).
        assert!(
            saddle.contains(&gear::GearKind::Leather.code()),
            "{saddle:?}"
        );
        assert!(jerseys > 0 && far_jerseys > 0);
        // Skin, hair, the painted frame (clear coat), the helmet without
        // the pattern: standard materials.
        assert!(
            plain_near > 5 && clear_coated > 0,
            "{plain_near} {clear_coated}"
        );
        for k in [
            gear::GearKind::Metal,
            gear::GearKind::Tyre,
            gear::GearKind::Knit,
            gear::GearKind::Lycra,
            gear::GearKind::Leather,
            gear::GearKind::Grip,
        ] {
            assert!(
                kinds.contains(&k.code()),
                "{k:?} on the road bike and rider"
            );
        }
    }

    /// Others see your look (spec 4C): the worn kit's variants, the
    /// character's skin and hair, the body height (scaled rider, saddle
    /// moved) and build (girth) on their avatar.
    #[test]
    fn avatars_wear_the_kit_and_the_character() {
        let mut view = RideView::new(Route::test_loop());
        let mut r = rider(1, ProfileId::Road, false);
        r.look = Some(RiderLook {
            character: ocw_core::Character {
                height_cm: 195,
                build: ocw_core::BodyBuild::Strong,
                skin: 10,
                hair: 3,
                ..Default::default()
            },
            ..Default::default()
        });
        r.parts = vec!["helmet.tt.top".into(), "wheels.road-carbon.top".into()];
        view.riders.push(r);
        let mut app = app(view);
        for _ in 0..3 {
            app.update();
        }
        let e = app.world().resource::<RiderEntities>().0[&RiderId(1)];
        let look = app.world().get::<AvatarLook>(e).unwrap().clone();
        assert_eq!(look.kit, vec!["helmet-tt".to_string()]);
        assert_eq!(
            look.variants,
            part_variants(&["wheels.road-carbon.top".into()])
        );
        assert!(!look.variants.is_empty());
        let rig = app.world().get::<AvatarRig>(e).unwrap();
        assert!((rig.scale - 195.0 / 178.0).abs() < 1e-5);
        let base = ocw_assets::shipped::bike_rig(ProfileId::Road);
        assert!(rig.bike.saddle.y > base.saddle.y + 0.03, "a higher saddle");
        // Named nodes: the TT helmet and the ponytail are there, the road
        // helmet is not.
        let mut names = app.world_mut().query::<&Name>();
        let names: Vec<String> = names.iter(app.world()).map(|n| n.to_string()).collect();
        assert!(names.iter().any(|n| n == nodes::HELMET_AERO));
        assert!(names.iter().any(|n| n == ocw_assets::rider::opt::HAIR_TAIL));
        assert!(!names.iter().any(|n| n == nodes::HELMET_ROAD));
        let c = avatar_colours(RiderId(1), false, look.look.as_ref());
        let [r8, g8, b8] = ocw_core::character::SKIN_TONES[10].channels();
        assert_eq!(c.skin, Color::srgb_u8(r8, g8, b8));
        assert_eq!(
            tint_color("skin", [1.0; 4], &c),
            c.skin,
            "the skin slot takes the look's tone"
        );
    }

    /// The pose follows the ride: standing on a steep climb, wheels
    /// turning, cranks level and still when freewheeling.
    #[test]
    fn animation_follows_the_ride() {
        // A 10 % climb.
        let mut view = view(vec![[0.0, 0.0, 0.0], [0.0, 50.0, -500.0]]);
        let mut r = rider(1, ProfileId::Road, false);
        r.power_w = 320.0;
        r.state.speed = 4.0;
        view.riders.push(r);
        let mut app = app(view);
        for _ in 0..60 {
            app.update();
        }
        let e = app.world().resource::<RiderEntities>().0[&RiderId(1)];
        let rig = |app: &App| {
            let r = app.world().get::<AvatarRig>(e).unwrap();
            (r.riding(), r.params(), r.pelvis, r.wheels[0])
        };
        let (state, params, pelvis, wheel) = rig(&app);
        assert_eq!(state, Riding::Standing);
        assert!(params.stand > 0.9, "{params:?}");
        // Standing: the hips are well above the saddle.
        let saddle = ocw_assets::shipped::bike_rig(ProfileId::Road).saddle;
        let y = app.world().get::<Transform>(pelvis).unwrap().translation.y;
        assert!(y > saddle.y + 0.12, "{y} vs saddle {}", saddle.y);
        // The wheels turn: 4 m/s for 20 ms on a 0.34 m wheel.
        let a = app.world().get::<Transform>(wheel).unwrap().rotation;
        app.update();
        let b = app.world().get::<Transform>(wheel).unwrap().rotation;
        assert!(a.angle_between(b) > 0.2, "{}", a.angle_between(b));
        // Freewheeling (no power): seated, cranks level and still.
        app.world_mut().resource_mut::<RideView>().riders[0].power_w = 0.0;
        for _ in 0..100 {
            app.update();
        }
        let (state, params, ..) = rig(&app);
        assert_eq!(state, Riding::Coasting);
        assert!(params.stand < 0.05);
        let level = (params.crank - std::f32::consts::FRAC_PI_2).abs() < 0.01
            || (params.crank - 3.0 * std::f32::consts::FRAC_PI_2).abs() < 0.01;
        assert!(level, "crank {}", params.crank);
    }

    #[test]
    fn faces_along_the_road_and_offsets_to_the_right() {
        // Road heading east (+x); right of east is south (+z).
        let v = view(vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]);
        let t = rider_transform(&v.route, 10.0, 0.0);
        assert!(close(t.translation, Vec3::new(10.0, 0.0, 0.0)));
        assert!(close(t.forward().as_vec3(), Vec3::X));
        let right = rider_transform(&v.route, 10.0, 1.5);
        assert!(close(right.translation, Vec3::new(10.0, 0.0, 1.5)));
        assert!(close(right.right().as_vec3(), Vec3::Z));
    }

    #[test]
    fn leans_into_curves() {
        // A circle of radius 50 m, clockwise seen from above (x east, z
        // south): heading east, then turning towards south = right.
        let r = 50.0;
        let pts: Vec<[f64; 3]> = (0..120)
            .map(|i| {
                let a = f64::from(i) / 120.0 * std::f64::consts::TAU;
                [r * a.sin(), 0.0, r - r * a.cos()]
            })
            .collect();
        let v = RideView::new(Route::from_points("c", "C", pts, true, 6.0).unwrap());
        let k = v.route.curvature(80.0);
        assert!((k - 1.0 / r).abs() < 0.002, "curvature {k}");
        // 10 m/s on R 50: atan(100 / (50 * 9.81)) = 11.5° to the right.
        let lean = lean_angle(k, 10.0);
        assert!(
            (lean.to_degrees() - 11.5).abs() < 0.6,
            "{}",
            lean.to_degrees()
        );
        let mut t = rider_transform(&v.route, 80.0, 0.0);
        let up_before = t.up().as_vec3();
        t.rotation *= lean_rotation(lean);
        // The top of the rider moves to the right (towards the centre).
        let shift = (t.up().as_vec3() - up_before).dot(t.right().as_vec3());
        assert!(shift > 0.15, "shift {shift}");
        // Straight road and standing still: upright; capped when too fast.
        let straight = view(vec![[0.0, 0.0, 0.0], [100.0, 0.0, 0.0]]);
        assert!(straight.route.curvature(50.0).abs() < 1e-9);
        assert_eq!(lean_angle(k, 0.0), 0.0);
        assert_eq!(lean_angle(k, 40.0), MAX_LEAN);
        assert_eq!(lean_angle(-k, 40.0), -MAX_LEAN);
    }

    #[test]
    fn lean_steering_turns_and_tilts_the_avatar() {
        let v = view(vec![[0.0, 0.0, 0.0], [500.0, 0.0, 0.0]]);
        let state = ocw_sim::RiderState {
            distance: 100.0,
            speed: 10.0,
            heading: 0.2,
            turn_curvature: 0.02,
            ..Default::default()
        };
        let t = avatar_transform(&v.route, &state, None, &road_rig());
        // Heading east, turned 0.2 rad to the right (towards +z, south).
        let f = t.forward().as_vec3();
        assert!((f.z - 0.2f32.sin()).abs() < 1e-3, "forward {f:?}");
        // Leaning right: the top tilts towards the rider's right.
        let upright = avatar_transform(
            &v.route,
            &ocw_sim::RiderState {
                turn_curvature: 0.0,
                ..state
            },
            None,
            &road_rig(),
        );
        let shift = (t.up().as_vec3() - upright.up().as_vec3()).dot(upright.right().as_vec3());
        assert!(shift > 0.15, "shift {shift}");
        // Off the road (wild mode), on the ground instead of the road plane.
        let ground = Ground(std::sync::Arc::new(|_, _| Some(12.0)));
        let off = ocw_sim::RiderState {
            lateral_offset: 20.0,
            ..state
        };
        // (Leaning about the tyre's crown lifts the origin by a fraction of
        // a millimetre.)
        let y_at = |s: &ocw_sim::RiderState| {
            avatar_transform(&v.route, s, Some(&ground), &road_rig())
                .translation
                .y
        };
        assert!((y_at(&off) - 12.0).abs() < 1e-3, "{}", y_at(&off));
        let on = y_at(&state);
        assert!((on - crate::ROAD_SURFACE_LIFT_M).abs() < 1e-3, "{on}");
        // Just beside the road (1 m of the 2 m blend): halfway onto the ground.
        let edge = ocw_sim::RiderState {
            lateral_offset: v.route.road_width_m() / 2.0 + 1.0,
            ..state
        };
        let y = avatar_transform(&v.route, &edge, Some(&ground), &road_rig())
            .translation
            .y;
        // (The wheels rest on the steep blend: a little higher than the
        // surface under the hubs.)
        let part = (y - crate::ROAD_SURFACE_LIFT_M) / (12.0 - crate::ROAD_SURFACE_LIFT_M);
        assert!((0.45..0.6).contains(&part), "{y}");
    }

    #[test]
    fn front_wheel_climbs_a_bump_first() {
        // Flat road at 0 m with a 0.3 m step up at x = 50 (5 cm ramp).
        let pts: Vec<[f64; 3]> = (0..=400)
            .map(|i| {
                let x = f64::from(i) * 0.25;
                [x, if x >= 50.0 { 0.3 } else { 0.0 }, 0.0]
            })
            .collect();
        let v = view(pts);
        let at = |d: f64| {
            let s = ocw_sim::RiderState {
                distance: d,
                speed: 5.0,
                ..Default::default()
            };
            let t = avatar_transform(&v.route, &s, None, &road_rig());
            (
                t.translation.y - crate::ROAD_SURFACE_LIFT_M,
                t.forward().as_vec3().y,
            )
        };
        // Before the step: level. Front wheel on it: nose up, half lifted.
        let (y0, f0) = at(48.0);
        assert!(y0.abs() < 1e-4 && f0.abs() < 1e-4, "{y0} {f0}");
        let (y1, f1) = at(50.0);
        assert!(f1 > 0.2 && (y1 - 0.15).abs() < 0.03, "{y1} {f1}");
        // Both wheels up: level again, 0.3 m higher.
        let (y2, f2) = at(52.0);
        assert!((y2 - 0.3).abs() < 1e-4 && f2.abs() < 1e-4, "{y2} {f2}");
    }

    #[test]
    fn pitches_up_on_climbs() {
        // 10 % climb heading north (-z).
        let v = view(vec![[0.0, 0.0, 0.0], [0.0, 10.0, -100.0]]);
        let t = rider_transform(&v.route, 50.0, 0.0);
        let f = t.forward().as_vec3();
        assert!(f.y > 0.09 && f.y < 0.11, "forward {f:?}");
        assert!(f.z < -0.99);
        assert!((t.translation.y - 5.0).abs() < 0.1);
    }
}

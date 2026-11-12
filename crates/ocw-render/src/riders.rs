//! Rider avatars (spec §2.5, §2.6, §8): the shipped glTF bike of the
//! rider's physics profile and the rider model, in the rider's colours,
//! animated from the ride (wheels from the speed, cranks and legs from
//! the cadence, standing on steep climbs and in sprints, cranks level
//! when freewheeling, the bike rocking under a standing rider). The
//! models, the pose and the state selection are Bevy-free
//! (`ocw_assets`); this module only spawns entities and writes
//! transforms. Far riders switch to the models' one-node `lod1`.

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

use crate::{Ground, RideView, to_vec3};

pub struct RidersPlugin;

impl Plugin for RidersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RiderEntities>().add_systems(
            PostUpdate,
            (sync_riders, ghost_casts_no_shadow, animate)
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

#[derive(Resource, Default)]
struct RiderEntities(BTreeMap<RiderId, Entity>);

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

/// Rotation (about the travel direction, through the tyre contact) that
/// leans the avatar; Bevy models face -Z, so a lean to the right is a
/// negative rotation about Z.
pub fn lean_rotation(lean: f64) -> Quat {
    Quat::from_rotation_z(-lean as f32)
}

/// Half the wheelbase of the avatar (wheel contacts at ±this along the bike).
pub const HALF_WHEELBASE_M: f64 = 0.55;

/// Height of the surface under a point `along` metres ahead of the rider
/// (along its heading): the road there, blended onto the ground beside
/// the road over [`OFF_ROAD_BLEND_M`] (no jump at the edge, no wheels in
/// an embankment).
fn contact_height(
    track: &dyn Track,
    state: &ocw_sim::RiderState,
    ground: Option<&Ground>,
    along: f64,
) -> (Vec3, f32) {
    let d = state.distance + along * state.heading.cos();
    let lateral = state.lateral_offset + along * state.heading.sin();
    let p = to_vec3(track.sample(d).offset_position(lateral));
    let beyond = lateral.abs() - track.road_width_at(d) / 2.0;
    let mut y = p.y;
    if beyond > 0.0
        && let Some(h) = ground.and_then(|g| (g.0)(f64::from(p.x), f64::from(p.z)))
    {
        let w = (beyond / OFF_ROAD_BLEND_M).min(1.0) as f32;
        y += (h as f32 - y) * w;
    }
    (p, y)
}

/// The avatar: turned by the rider's heading relative to the road, leaning
/// for the curve it rides, and standing on the surface under both wheels
/// (road, or the ground beside it): over a bump the front wheel rises
/// first, then the rear.
pub fn avatar_transform(
    track: &dyn Track,
    state: &ocw_sim::RiderState,
    ground: Option<&Ground>,
) -> Transform {
    let mut t = rider_transform(track, state.distance, state.lateral_offset);
    let (front_p, front) = contact_height(track, state, ground, HALF_WHEELBASE_M);
    let (rear_p, rear) = contact_height(track, state, ground, -HALF_WHEELBASE_M);
    t.translation.y = 0.5 * (front + rear);
    let run = Vec2::new(front_p.x - rear_p.x, front_p.z - rear_p.z)
        .length()
        .max(1e-3);
    let pitch = (front - rear).atan2(run);
    let p = track.sample(state.distance);
    let road = Vec3::new(p.direction[0] as f32, 0.0, p.direction[1] as f32);
    let yaw = Transform::default().looking_to(road, Vec3::Y).rotation;
    // Positive heading = right; Bevy turns left for a positive Y rotation.
    t.rotation = Quat::from_rotation_y(-state.heading as f32)
        * yaw
        * Quat::from_rotation_x(pitch)
        * lean_rotation(lean_angle(state.turn_curvature, state.speed));
    t
}

/// The colours an avatar is drawn with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvatarColours {
    pub frame: Color,
    pub jersey: Color,
    pub helmet: Color,
    /// The kit pattern's colour: white on dark jerseys, black on light.
    pub accent: Color,
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
    AvatarColours {
        frame,
        jersey,
        helmet,
        accent,
    }
}

/// What an avatar was built from: rebuilt when it changes.
#[derive(Component, Debug, Clone, Copy, PartialEq)]
struct AvatarLook {
    bike: ProfileId,
    look: Option<RiderLook>,
}

#[allow(clippy::too_many_arguments)]
fn sync_riders(
    mut commands: Commands,
    view: Res<RideView>,
    mut entities: ResMut<RiderEntities>,
    mut avatars: Query<(&mut Transform, &AvatarLook), With<Avatar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    ground: Option<Res<Ground>>,
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
        .map(|r| (r.id, r.state, r.id, r.is_local, r.bike, r.look))
        .chain(ghost.and_then(|(id, state, on)| {
            let local = view.local()?;
            Some((id, state, on, false, local.bike, None))
        }));
    for (id, state, track_of, is_local, bike, look) in all {
        let t = avatar_transform(view.track(track_of), &state, ground.as_deref());
        let wanted = AvatarLook { bike, look };
        if let Some(e) = entities.0.get(&id).copied() {
            match avatars.get_mut(e) {
                Ok((mut tr, built)) if *built == wanted => {
                    *tr = t;
                    continue;
                }
                // Another bike or look: build the avatar again.
                _ => commands.entity(e).despawn(),
            }
        }
        let e = spawn_avatar(&mut commands, lib, &mut materials, id, is_local, wanted, t);
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
/// per node.
struct Meshes(Vec<Vec<(Handle<Mesh>, usize)>>);

impl Meshes {
    fn new(model: &Model, meshes: &mut Assets<Mesh>) -> Self {
        Self(
            model
                .nodes
                .iter()
                .map(|n| {
                    n.primitives
                        .iter()
                        .map(|p| {
                            let mut m = Mesh::new(
                                PrimitiveTopology::TriangleList,
                                RenderAssetUsages::default(),
                            );
                            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, p.positions.clone());
                            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, p.normals.clone());
                            m.insert_indices(Indices::U32(p.indices.clone()));
                            (meshes.add(m), p.material)
                        })
                        .collect()
                })
                .collect(),
        )
    }
}

/// The shipped models on the GPU, their rigs, and the materials by slot
/// and colour (riders in the same colours share them).
struct Library {
    rider: Meshes,
    bikes: Vec<Meshes>,
    rider_rig: RiderRig,
    bike_rigs: Vec<BikeRig>,
    fits: Vec<Fit>,
    materials: HashMap<(&'static str, [u8; 4]), Handle<StandardMaterial>>,
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
            rider: Meshes::new(ocw_assets::shipped::rider_model(), meshes),
            bikes: ProfileId::ALL
                .iter()
                .map(|p| Meshes::new(ocw_assets::shipped::bike_model(*p), meshes))
                .collect(),
            fits: bike_rigs.iter().map(|b| Fit::new(&rider_rig, b)).collect(),
            rider_rig,
            bike_rigs,
            materials: HashMap::new(),
            ghost: None,
        }
    }

    /// The material of paint slot `slot` (model colour `model_color`,
    /// linear RGBA) for a rider in `colours`.
    fn material(
        &mut self,
        materials: &mut Assets<StandardMaterial>,
        slot: &str,
        model_color: [f32; 4],
        colours: &AvatarColours,
        ghost: bool,
    ) -> Handle<StandardMaterial> {
        if ghost {
            return self
                .ghost
                .get_or_insert_with(|| materials.add(ghost_material()))
                .clone();
        }
        let s = slots::get(slot).unwrap_or(&slots::ALL[0]);
        let color = match s.tint {
            Tint::Fixed => Color::linear_rgba(
                model_color[0],
                model_color[1],
                model_color[2],
                model_color[3],
            ),
            Tint::Frame => colours.frame,
            Tint::Jersey => colours.jersey,
            Tint::Helmet => colours.helmet,
            Tint::Accent => colours.accent,
        };
        self.materials
            .entry((s.name, color.to_srgba().to_u8_array()))
            .or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: color,
                    metallic: s.metallic,
                    perceptual_roughness: s.roughness,
                    ..default()
                })
            })
            .clone()
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
    mut material: impl FnMut(&str, [f32; 4]) -> Handle<StandardMaterial>,
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
            ))
            .id();
        out[i] = e;
        let range = if far.is_some_and(|f| model.is_below(i, f)) {
            far_range()
        } else {
            near_range()
        };
        for (mesh, mat) in &meshes.0[i] {
            let m = &model.materials[*mat];
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material(&m.name, m.color)),
                range.clone(),
                ChildOf(e),
            ));
        }
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn spawn_avatar(
    commands: &mut Commands,
    lib: &mut Library,
    materials: &mut Assets<StandardMaterial>,
    id: RiderId,
    is_local: bool,
    built: AvatarLook,
    transform: Transform,
) -> Entity {
    let colours = avatar_colours(id, is_local, built.look.as_ref());
    let ghost = id == GHOST_ID;
    let b = profile_index(built.bike);
    let bike_model = ocw_assets::shipped::bike_model(built.bike);
    let rider_model = ocw_assets::shipped::rider_model();
    let bike = lib.bike_rigs[b];
    let root = commands
        .spawn((
            transform,
            Visibility::default(),
            Avatar { id, is_local },
            built,
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
    let bike_list = Meshes(lib.bikes[b].0.clone());
    let rider_list = Meshes(lib.rider.0.clone());
    let bike_nodes_e = spawn_model(
        commands,
        sway,
        bike_model,
        &bike_list,
        &skip,
        &[],
        far,
        |slot, c| lib.material(materials, slot, c, &colours, ghost),
    );
    // Head wear: a cotton cap for the local rider, an aero helmet on the
    // TT bike, else a road helmet; the kit pattern's band or sash.
    let wear = if is_local {
        nodes::CAP
    } else if bike.aero_bars {
        nodes::HELMET_AERO
    } else {
        nodes::HELMET_ROAD
    };
    let pattern = built
        .look
        .map_or(ocw_core::KitPattern::Plain, |l| l.pattern);
    let kit = match pattern {
        ocw_core::KitPattern::Plain => None,
        ocw_core::KitPattern::Band => Some(nodes::KIT_BAND),
        ocw_core::KitPattern::Sash => Some(nodes::KIT_SASH),
    };
    let skip_rider: Vec<usize> = [
        nodes::CAP,
        nodes::HELMET_AERO,
        nodes::HELMET_ROAD,
        nodes::KIT_BAND,
        nodes::KIT_SASH,
    ]
    .into_iter()
    .filter(|n| *n != wear && Some(*n) != kit)
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
        |slot, c| lib.material(materials, slot, c, &colours, ghost),
    );
    let find = |name: &str| -> Entity {
        if let Some(i) = rider_model.node(name) {
            return rider_e[i];
        }
        bike_model
            .node(name)
            .map_or(Entity::PLACEHOLDER, |i| bike_nodes_e[i])
    };
    let posed = pose::pose(&lib.rider_rig, &bike, &lib.fits[b], &PoseParams::default())
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
        fit: lib.fits[b],
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
    mut avatars: Query<(&Avatar, &mut AvatarRig, &GlobalTransform)>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    mut transforms: Query<&mut Transform>,
) {
    let dt = time.delta_secs();
    let eye = cameras.iter().next().map(GlobalTransform::translation);
    let rider_rig = ocw_assets::shipped::rider_rig();
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
        let p = pose::pose(&rider_rig, &rig.bike, &rig.fit, &rig.params);
        if let Ok(mut t) = transforms.get_mut(rig.sway) {
            t.rotation = Quat::from_rotation_z(p.bike_roll);
        }
        if let Ok(mut t) = transforms.get_mut(rig.pelvis) {
            t.translation = p.pelvis.0;
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
        assert!(close(ghost[0], Vec3::new(42.0, 0.0, 1.0)), "{ghost:?}");
        // No ghost part casts or receives a shadow; the local rider's do.
        let mut parts = app.world_mut().query_filtered::<(
            &ChildOf,
            Has<bevy::light::NotShadowCaster>,
            Has<bevy::light::NotShadowReceiver>,
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
            .map(|(c, caster, receiver)| (c.parent(), caster, receiver))
            .collect();
        for (parent, caster, receiver) in rows {
            let a = avatars.get(app.world(), root(app.world(), parent)).unwrap();
            if a.id == GHOST_ID {
                assert!(caster && receiver);
                ghost_parts += 1;
            } else {
                assert!(!caster && !receiver);
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
        // White pattern on the dark blue jersey, black on a light one.
        assert_eq!(c.accent, Color::srgb(0.96, 0.96, 0.94));
        let light = RiderLook {
            jersey: ocw_core::Rgb(0xF2F2EE),
            ..look
        };
        assert_eq!(
            avatar_colours(RiderId(3), false, Some(&light)).accent,
            Color::srgb(0.06, 0.06, 0.07)
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
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
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
        let built = *app.world().get::<AvatarLook>(second).unwrap();
        assert_eq!(built.bike, ProfileId::Mountain);
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
        }
    }

    /// A headless app with the riders plugin and a fixed 20 ms time step.
    fn app(view: RideView) -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
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
        let prims = |name: &str| rider.nodes[rider.node(name).unwrap()].primitives.len();
        let all_wear = prims(nodes::CAP) + prims(nodes::HELMET_ROAD) + prims(nodes::HELMET_AERO);
        let all_kit = prims(nodes::KIT_BAND) + prims(nodes::KIT_SASH);
        let full = |m: &Model| m.nodes.iter().map(|n| n.primitives.len()).sum::<usize>();
        for (i, p) in ProfileId::ALL.iter().enumerate() {
            let e = entities[&RiderId(i as u32 + 1)];
            let shown = shown_meshes(&mut app, e).len();
            let bike = full(ocw_assets::shipped::bike_model(*p));
            let wear = if i == 0 {
                prims(nodes::CAP)
            } else if *p == ProfileId::TimeTrial {
                prims(nodes::HELMET_AERO)
            } else {
                prims(nodes::HELMET_ROAD)
            };
            // The whole bike (with its far LOD), the rider with one head
            // wear and the band.
            let expect = bike + full(rider) - all_wear - all_kit + wear + prims(nodes::KIT_BAND);
            assert_eq!(shown, expect, "{p:?}");
            assert!(app.world().get::<AvatarRig>(e).is_some());
        }
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
        let t = avatar_transform(&v.route, &state, None);
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
        );
        let shift = (t.up().as_vec3() - upright.up().as_vec3()).dot(upright.right().as_vec3());
        assert!(shift > 0.15, "shift {shift}");
        // Off the road (wild mode), on the ground instead of the road plane.
        let ground = Ground(std::sync::Arc::new(|_, _| Some(12.0)));
        let off = ocw_sim::RiderState {
            lateral_offset: 20.0,
            ..state
        };
        assert_eq!(
            avatar_transform(&v.route, &off, Some(&ground))
                .translation
                .y,
            12.0
        );
        assert_eq!(
            avatar_transform(&v.route, &state, Some(&ground))
                .translation
                .y,
            0.0
        );
        // Just beside the road (1 m of the 2 m blend): halfway onto the ground.
        let edge = ocw_sim::RiderState {
            lateral_offset: v.route.road_width_m() / 2.0 + 1.0,
            ..state
        };
        let y = avatar_transform(&v.route, &edge, Some(&ground))
            .translation
            .y;
        assert!((y - 6.0).abs() < 1e-4, "{y}");
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
            let t = avatar_transform(&v.route, &s, None);
            (t.translation.y, t.forward().as_vec3().y)
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

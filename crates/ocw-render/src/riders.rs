//! Rider avatars: procedural bikes per bike type and riders in their
//! colours (spec §2.5; placeholder geometry until the glTF assets of §8).

use std::collections::BTreeMap;

use bevy::prelude::*;
use ocw_core::{ProfileId, RiderId, RiderLook};

use ocw_world::Track;

use crate::{Ground, RideView, to_vec3};

pub struct RidersPlugin;

impl Plugin for RidersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RiderEntities>().add_systems(
            PostUpdate,
            (sync_riders, pedal)
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

#[derive(Resource, Default)]
struct RiderEntities(BTreeMap<RiderId, Entity>);

/// Crank angle of an avatar in radians.
#[derive(Component, Default)]
struct Crank(f32);

/// A leg, rotating about the hip with the crank; `phase` is π for the right leg.
#[derive(Component)]
struct PedalLeg {
    phase: f32,
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

/// Cadence assumed for riders without cadence data while they move.
pub const DEFAULT_CADENCE_RPM: f64 = 85.0;
/// Maximum leg swing about the hip, in radians.
const LEG_SWING: f32 = 0.6;

/// Crank speed in rad/s: the measured cadence, or a default cadence while
/// the rider moves (others' cadence is not sent over the network).
pub fn crank_rate(cadence_rpm: Option<f64>, speed_mps: f64) -> f32 {
    let rpm = cadence_rpm.unwrap_or(if speed_mps > 1.0 {
        DEFAULT_CADENCE_RPM
    } else {
        0.0
    });
    (rpm / 60.0 * std::f64::consts::TAU) as f32
}

/// Leg rotation about the hip for a crank angle.
pub fn leg_rotation(crank: f32, phase: f32) -> Quat {
    Quat::from_rotation_x((crank + phase).sin() * LEG_SWING)
}

/// Turns the cranks and swings the legs.
fn pedal(
    time: Res<Time>,
    view: Res<RideView>,
    mut avatars: Query<(&Avatar, &mut Crank)>,
    mut legs: Query<(&PedalLeg, &ChildOf, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (avatar, mut crank) in &mut avatars {
        let rate = if avatar.id == GHOST_ID {
            view.ghost.map(|g| crank_rate(None, g.speed))
        } else {
            view.riders
                .iter()
                .find(|r| r.id == avatar.id)
                .map(|r| crank_rate(r.cadence_rpm, r.state.speed))
        };
        if let Some(rate) = rate {
            crank.0 = (crank.0 + rate * dt) % std::f32::consts::TAU;
        }
    }
    for (leg, parent, mut t) in &mut legs {
        if let Ok((_, crank)) = avatars.get(parent.parent()) {
            t.rotation = leg_rotation(crank.0, leg.phase);
        }
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

/// Handlebars of a bike model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bars {
    /// Road and gravel: a bar with drops curving down.
    Drop,
    /// Time trial: a base bar with two extensions pointing forward.
    Aero,
    /// Mountain bike: a wide straight bar.
    Flat,
}

/// The procedural bike model of a physics profile (spec §2.5: the bike
/// type picks the model; placeholder geometry until the glTF assets of
/// §8): what makes the silhouettes of the bike types distinguishable.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BikeModel {
    pub bars: Bars,
    /// Tyre width (m): road 25 mm, gravel 40 mm, MTB 60 mm.
    pub tyre_m: f32,
    /// Outer wheel radius incl. the tyre (m).
    pub wheel_radius_m: f32,
    /// Rim depth (m): deep aero rims show as a band inside the tyre.
    pub rim_depth_m: f32,
    /// A closed disc rear wheel (time trial).
    pub rear_disc: bool,
    /// Frame tube thickness (m); aero frames are deeper.
    pub tube_m: f32,
    /// Suspension fork (thicker, two legs; MTB).
    pub suspension: bool,
    /// Height of the saddle and top tube above the ground (m): sloping
    /// MTB frames are lower at the back.
    pub seat_height_m: f32,
    /// Forward lean of the rider's torso from upright (rad): flat on the
    /// TT bike, upright on the MTB.
    pub torso_lean: f32,
}

/// The model of a bike.
pub fn bike_model(bike: ProfileId) -> BikeModel {
    let road = BikeModel {
        bars: Bars::Drop,
        tyre_m: 0.025,
        wheel_radius_m: 0.34,
        rim_depth_m: 0.02,
        rear_disc: false,
        tube_m: 0.035,
        suspension: false,
        seat_height_m: 0.88,
        torso_lean: 1.0,
    };
    match bike {
        ProfileId::Road => road,
        ProfileId::RoadAero => BikeModel {
            rim_depth_m: 0.06,
            tube_m: 0.045,
            ..road
        },
        ProfileId::RoadClimbing => BikeModel {
            tube_m: 0.03,
            ..road
        },
        ProfileId::TimeTrial => BikeModel {
            bars: Bars::Aero,
            rim_depth_m: 0.08,
            rear_disc: true,
            tube_m: 0.055,
            torso_lean: 1.3,
            ..road
        },
        ProfileId::Gravel => BikeModel {
            tyre_m: 0.04,
            wheel_radius_m: 0.35,
            tube_m: 0.04,
            torso_lean: 0.9,
            ..road
        },
        ProfileId::Mountain => BikeModel {
            bars: Bars::Flat,
            tyre_m: 0.06,
            wheel_radius_m: 0.37,
            tube_m: 0.05,
            suspension: true,
            seat_height_m: 0.8,
            torso_lean: 0.7,
            ..road
        },
    }
}

/// The transform that stretches a Y-axis primitive of length 1 (a
/// [`Cylinder`] of height 1) from `a` to `b`, and the length.
pub fn tube_between(a: Vec3, b: Vec3) -> (f32, Transform) {
    let d = b - a;
    let len = d.length().max(1e-4);
    let rot = Quat::from_rotation_arc(Vec3::Y, d / len);
    (
        len,
        Transform::from_translation((a + b) * 0.5)
            .with_rotation(rot)
            .with_scale(Vec3::new(1.0, len, 1.0)),
    )
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

fn sync_riders(
    mut commands: Commands,
    view: Res<RideView>,
    mut entities: ResMut<RiderEntities>,
    mut avatars: Query<(&mut Transform, &AvatarLook), With<Avatar>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    ground: Option<Res<Ground>>,
) {
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
        let e = spawn_avatar(
            &mut commands,
            &mut meshes,
            &mut materials,
            id,
            is_local,
            wanted,
            t,
        );
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

/// Spawns a part: `mesh` with `material` at `transform`.
fn part(
    a: &mut ChildSpawnerCommands<'_>,
    mesh: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    transform: Transform,
) {
    a.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(material.clone()),
        transform,
    ));
}

/// A tube of radius `r` from `a` to `b` (a unit cylinder stretched).
fn tube(
    a: &mut ChildSpawnerCommands<'_>,
    unit: &Handle<Mesh>,
    material: &Handle<StandardMaterial>,
    (from, to): (Vec3, Vec3),
    r: f32,
) {
    let (_, t) = tube_between(from, to);
    part(
        a,
        unit,
        material,
        t.with_scale(t.scale * Vec3::new(r, 1.0, r)),
    );
}

fn spawn_avatar(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    id: RiderId,
    is_local: bool,
    built: AvatarLook,
    transform: Transform,
) -> Entity {
    let model = bike_model(built.bike);
    let colours = avatar_colours(id, is_local, built.look.as_ref());
    let mat = |materials: &mut Assets<StandardMaterial>, c: Color, rough: f32| {
        materials.add(StandardMaterial {
            base_color: c,
            perceptual_roughness: rough,
            ..default()
        })
    };
    let jersey = mat(materials, colours.jersey, 0.8);
    let frame = mat(materials, colours.frame, 0.35);
    let helmet = mat(materials, colours.helmet, 0.5);
    let accent = mat(materials, colours.accent, 0.8);
    let dark = mat(materials, Color::srgb(0.08, 0.08, 0.09), 0.5);
    let rim = mat(materials, Color::srgb(0.18, 0.18, 0.2), 0.3);
    let skin = mat(materials, Color::srgb(0.85, 0.68, 0.55), 0.7);
    // The ghost (spec §2.3): one translucent material for every part.
    let (jersey, frame, helmet, accent, dark, rim, skin) = if id == GHOST_ID {
        let g = materials.add(ghost_material());
        (
            g.clone(),
            g.clone(),
            g.clone(),
            g.clone(),
            g.clone(),
            g.clone(),
            g,
        )
    } else {
        (jersey, frame, helmet, accent, dark, rim, skin)
    };
    // A unit cylinder (radius 1, height 1) for all tubes.
    let unit = meshes.add(Cylinder::new(1.0, 1.0));
    let r = model.wheel_radius_m;
    let tyre = meshes.add(Torus::new(r - model.tyre_m, r));
    let rim_mesh = meshes.add(Torus::new(
        r - model.tyre_m - model.rim_depth_m,
        r - model.tyre_m + 0.004,
    ));
    let disc = meshes.add(Cylinder::new(r - model.tyre_m, 0.025));
    let rear_hub = Vec3::new(0.0, r, 0.55);
    let front_hub = Vec3::new(0.0, r, -0.55);
    let bb = Vec3::new(0.0, 0.3, 0.08);
    let seat = Vec3::new(0.0, model.seat_height_m, 0.28);
    let head_top = Vec3::new(0.0, 0.86, -0.4);
    let head_bottom = Vec3::new(0.0, 0.7, -0.44);
    let tube_r = model.tube_m * 0.5;
    commands
        .spawn((
            transform,
            Visibility::default(),
            Avatar { id, is_local },
            built,
            Crank::default(),
        ))
        .with_children(|a| {
            // Wheels (front at -Z), lying in the YZ plane.
            let wheel_rot = Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
            for hub in [front_hub, rear_hub] {
                part(
                    a,
                    &tyre,
                    &dark,
                    Transform::from_translation(hub).with_rotation(wheel_rot),
                );
                // Deep rims: a band inside the tyre, flattened sideways.
                part(
                    a,
                    &rim_mesh,
                    &rim,
                    Transform::from_translation(hub)
                        .with_rotation(wheel_rot)
                        .with_scale(Vec3::new(1.0, 0.5, 1.0)),
                );
            }
            if model.rear_disc {
                part(
                    a,
                    &disc,
                    &rim,
                    Transform::from_translation(rear_hub).with_rotation(wheel_rot),
                );
            }
            // Diamond frame in the frame colour.
            for ends in [(seat, head_top), (bb, head_bottom), (bb, seat)] {
                tube(a, &unit, &frame, ends, tube_r);
            }
            for x in [-0.045, 0.045] {
                let hub = rear_hub + Vec3::X * x;
                tube(a, &unit, &frame, (bb, hub), tube_r * 0.6);
                tube(
                    a,
                    &unit,
                    &frame,
                    (seat - Vec3::new(0.0, 0.05, 0.0), hub),
                    tube_r * 0.55,
                );
                // Fork legs (thick suspension legs on the MTB).
                let fork_r = if model.suspension {
                    0.022
                } else {
                    tube_r * 0.6
                };
                tube(
                    a,
                    &unit,
                    &frame,
                    (head_bottom + Vec3::X * x, front_hub + Vec3::X * x),
                    fork_r,
                );
            }
            tube(a, &unit, &frame, (head_bottom, head_top), tube_r);
            // Stem, saddle and seat post.
            let stem = head_top + Vec3::new(0.0, 0.06, -0.06);
            tube(a, &unit, &dark, (head_top, stem), 0.016);
            let saddle = seat + Vec3::new(0.0, 0.08, 0.0);
            tube(a, &unit, &dark, (seat, saddle), 0.014);
            part(
                a,
                &meshes.add(Cuboid::new(0.13, 0.04, 0.26)),
                &dark,
                Transform::from_translation(saddle + Vec3::new(0.0, 0.02, -0.03)),
            );
            // Handlebars; `hands` are where the arms end.
            let hands = match model.bars {
                Bars::Drop => {
                    let w = 0.21;
                    tube(
                        a,
                        &unit,
                        &dark,
                        (stem + Vec3::X * -w, stem + Vec3::X * w),
                        0.013,
                    );
                    for x in [-w, w] {
                        let top = stem + Vec3::X * x;
                        let low = top + Vec3::new(0.0, -0.13, 0.06);
                        tube(
                            a,
                            &unit,
                            &dark,
                            (top, top + Vec3::new(0.0, -0.05, -0.07)),
                            0.012,
                        );
                        tube(
                            a,
                            &unit,
                            &dark,
                            (top + Vec3::new(0.0, -0.05, -0.07), low),
                            0.012,
                        );
                    }
                    [
                        stem + Vec3::new(-w, 0.0, -0.02),
                        stem + Vec3::new(w, 0.0, -0.02),
                    ]
                }
                Bars::Aero => {
                    let w = 0.2;
                    tube(
                        a,
                        &unit,
                        &dark,
                        (stem + Vec3::X * -w, stem + Vec3::X * w),
                        0.014,
                    );
                    let mut hands = [Vec3::ZERO; 2];
                    for (i, x) in [-0.07f32, 0.07].into_iter().enumerate() {
                        let base = stem + Vec3::new(x, 0.04, 0.0);
                        let tip = base + Vec3::new(0.0, 0.03, -0.34);
                        tube(a, &unit, &dark, (base, tip), 0.011);
                        // Arm pads.
                        part(
                            a,
                            &meshes.add(Cuboid::new(0.08, 0.02, 0.1)),
                            &dark,
                            Transform::from_translation(base + Vec3::new(0.0, 0.02, 0.02)),
                        );
                        hands[i] = tip;
                    }
                    hands
                }
                Bars::Flat => {
                    let w = 0.36;
                    let bar = stem + Vec3::new(0.0, 0.05, 0.0);
                    tube(a, &unit, &dark, (stem, bar), 0.016);
                    tube(
                        a,
                        &unit,
                        &dark,
                        (bar + Vec3::X * -w, bar + Vec3::X * w),
                        0.013,
                    );
                    [bar + Vec3::X * -(w - 0.06), bar + Vec3::X * (w - 0.06)]
                }
            };
            // Torso, leaning forward from the hips over the saddle.
            let hip = saddle + Vec3::new(0.0, 0.1, 0.0);
            let dir = Vec3::new(0.0, model.torso_lean.cos(), -model.torso_lean.sin());
            let torso_rot = Quat::from_rotation_arc(Vec3::Y, dir);
            let torso_at = hip + dir * 0.3;
            part(
                a,
                &meshes.add(Capsule3d::new(0.17, 0.4)),
                &jersey,
                Transform::from_translation(torso_at).with_rotation(torso_rot),
            );
            // Kit pattern: a band or a diagonal sash around the torso.
            let pattern = built
                .look
                .map_or(ocw_core::KitPattern::Plain, |l| l.pattern);
            let ring = meshes.add(Cylinder::new(0.176, 0.075));
            match pattern {
                ocw_core::KitPattern::Plain => {}
                ocw_core::KitPattern::Band => part(
                    a,
                    &ring,
                    &accent,
                    Transform::from_translation(torso_at + dir * 0.05).with_rotation(torso_rot),
                ),
                ocw_core::KitPattern::Sash => part(
                    a,
                    &ring,
                    &accent,
                    Transform::from_translation(torso_at)
                        .with_rotation(torso_rot * Quat::from_rotation_z(0.7)),
                ),
            }
            // Arms from the shoulders to the hands.
            let shoulder = hip + dir * 0.55;
            for (x, hand) in [-0.17f32, 0.17].into_iter().zip(hands) {
                tube(a, &unit, &jersey, (shoulder + Vec3::X * x, hand), 0.045);
            }
            // Legs: pivot at the hip, swinging with the crank.
            let leg = meshes.add(Capsule3d::new(0.07, 0.5));
            for (x, phase) in [(-0.1, 0.0), (0.1, std::f32::consts::PI)] {
                a.spawn((
                    Transform::from_translation(hip + Vec3::new(x, -0.05, 0.0)),
                    Visibility::default(),
                    PedalLeg { phase },
                ))
                .with_children(|h| {
                    h.spawn((
                        Mesh3d(leg.clone()),
                        MeshMaterial3d(dark.clone()),
                        Transform::from_xyz(0.0, -0.32, -0.05),
                    ));
                });
            }
            // Head.
            let head = hip + dir * 0.72 + Vec3::new(0.0, 0.07, -0.02);
            part(
                a,
                &meshes.add(Sphere::new(0.12)),
                &skin,
                Transform::from_translation(head),
            );
            let top = head + Vec3::new(0.0, 0.05, 0.0);
            if is_local {
                // Classic cotton cycling cap: crown in the helmet colour
                // (white by default), band in the jersey colour, short
                // brim flipped up.
                part(
                    a,
                    &meshes.add(Sphere::new(0.126)),
                    &helmet,
                    Transform::from_translation(top).with_scale(Vec3::new(1.0, 0.62, 1.05)),
                );
                part(
                    a,
                    &meshes.add(Cylinder::new(0.127, 0.035)),
                    &jersey,
                    Transform::from_translation(top - Vec3::new(0.0, 0.01, 0.0)),
                );
                part(
                    a,
                    &meshes.add(Cuboid::new(0.17, 0.012, 0.075)),
                    &jersey,
                    Transform::from_translation(top + Vec3::new(0.0, 0.0, -0.15))
                        .with_rotation(Quat::from_rotation_x(0.45)),
                );
            } else {
                // Helmet; an aero helmet with a long tail on the TT bike.
                let scale = if model.bars == Bars::Aero {
                    Vec3::new(1.0, 0.7, 1.7)
                } else {
                    Vec3::new(1.0, 0.7, 1.2)
                };
                part(
                    a,
                    &meshes.add(Sphere::new(0.135)),
                    &helmet,
                    Transform::from_translation(top + Vec3::new(0.0, 0.0, 0.02)).with_scale(scale),
                );
            }
        })
        .id()
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

    /// The bike types have distinguishable silhouettes (spec §2.5): each
    /// differs from the others in bars, tyres or wheels.
    #[test]
    fn bike_types_look_different() {
        use ocw_core::BikeType;
        let models: Vec<BikeModel> = BikeType::ALL
            .iter()
            .map(|b| bike_model(b.default_profile()))
            .collect();
        for (i, a) in models.iter().enumerate() {
            for b in &models[i + 1..] {
                assert!(
                    a.bars != b.bars
                        || (a.tyre_m - b.tyre_m).abs() > 0.01
                        || a.rear_disc != b.rear_disc,
                    "{a:?} vs {b:?}"
                );
            }
        }
        let tt = bike_model(ProfileId::TimeTrial);
        assert!(tt.rear_disc && tt.bars == Bars::Aero);
        let mtb = bike_model(ProfileId::Mountain);
        assert!(mtb.bars == Bars::Flat && mtb.suspension && mtb.tyre_m > 0.05);
        assert!(bike_model(ProfileId::Gravel).tyre_m > bike_model(ProfileId::Road).tyre_m);
        // Aero road: deeper rims than the standard road bike.
        assert!(
            bike_model(ProfileId::RoadAero).rim_depth_m > bike_model(ProfileId::Road).rim_depth_m
        );
        // The TT rider is lowest, the MTB rider most upright.
        assert!(tt.torso_lean > bike_model(ProfileId::Road).torso_lean);
        assert!(mtb.torso_lean < bike_model(ProfileId::Road).torso_lean);
    }

    #[test]
    fn tubes_join_their_ends() {
        let (a, b) = (Vec3::new(0.0, 0.3, 0.1), Vec3::new(0.2, 0.9, -0.4));
        let (len, t) = tube_between(a, b);
        assert!((len - (b - a).length()).abs() < 1e-6);
        // The unit cylinder's ends (y = ±0.5) land on a and b.
        assert!(close(t.transform_point(Vec3::new(0.0, -0.5, 0.0)), a));
        assert!(close(t.transform_point(Vec3::new(0.0, 0.5, 0.0)), b));
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

    #[test]
    fn cranks_turn_with_cadence_or_movement() {
        // 90 rpm = 1.5 turns per second.
        assert!((crank_rate(Some(90.0), 0.0) - 3.0 * std::f32::consts::PI).abs() < 1e-5);
        // Measured 0 rpm: legs stand still even when coasting.
        assert_eq!(crank_rate(Some(0.0), 10.0), 0.0);
        // Others (no cadence): default cadence while moving, still when stopped.
        assert!(crank_rate(None, 8.0) > 0.0);
        assert_eq!(crank_rate(None, 0.5), 0.0);
        // Left and right legs swing in opposite directions.
        let (l, r) = (
            leg_rotation(1.0, 0.0),
            leg_rotation(1.0, std::f32::consts::PI),
        );
        assert!((l.to_euler(EulerRot::XYZ).0 + r.to_euler(EulerRot::XYZ).0).abs() < 1e-5);
        assert!(l.to_euler(EulerRot::XYZ).0.abs() <= LEG_SWING + 1e-6);
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

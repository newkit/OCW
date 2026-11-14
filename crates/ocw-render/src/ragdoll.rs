//! Ragdoll falls (spec §14 Phase 12C, Q5): when the server says a rider
//! crashed, the avatar's own body segments and bike become rigid bodies of
//! the physics engine – **Avian** (`avian3d`, MIT OR Apache-2.0, chosen by
//! the K2 spike, spec 12I) – joined at the hips, knees, shoulders, elbows,
//! waist and neck, the bike a frame with two wheels on hinges. They start
//! with the rider's speed and the contact's push from the server, fall,
//! slide and settle on a ground plane under the rider. On *Remount* the
//! pieces are eased back onto a fresh avatar in the riding pose (the rider
//! picks up the bike and clips in), which then takes over.
//!
//! Visual only: the crash, its timing and the time down are the server's
//! (12B / 12C); every client simulates its own, plausible fall, so riders
//! see similar but not identical falls (no determinism needed). Other
//! riders' falls get a crash marker over them.

use std::collections::{BTreeMap, BTreeSet};

use avian3d::prelude::*;
use bevy::prelude::*;
use ocw_assets::pose::{bike_nodes, nodes};
use ocw_core::RiderId;
use ocw_sim::collide::{self, CrashPhase};

use crate::RideView;
use crate::riders::{Avatar, RiderEntities};

pub struct RagdollPlugin;

impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<PhysicsSchedulePlugin>() {
            app.add_plugins(PhysicsPlugins::default());
        }
        app.init_resource::<Ragdolls>()
            .add_systems(Update, (drive_ragdolls, dress_markers).chain());
    }
}

/// Marks an avatar whose segments the ragdoll drives (the avatar's own
/// animation leaves it alone).
#[derive(Component)]
pub struct Ragdolled;

/// A ragdoll piece: one body segment or bike part.
#[derive(Component, Debug, Clone, Copy)]
pub struct Piece {
    pub rider: RiderId,
}

/// The crash marker over another rider's fall.
#[derive(Component)]
pub struct CrashMarker;

/// The rider segments that become bodies: (node, the child joint the
/// segment reaches to, collider radius).
const SEGMENTS: [(&str, Option<&str>, f32); 11] = [
    (nodes::PELVIS, Some(nodes::TORSO), 0.12),
    (nodes::TORSO, Some(nodes::HEAD), 0.14),
    (nodes::HEAD, None, 0.12),
    (nodes::THIGH[0], Some(nodes::SHIN[0]), 0.07),
    (nodes::THIGH[1], Some(nodes::SHIN[1]), 0.07),
    (nodes::SHIN[0], Some(nodes::FOOT[0]), 0.05),
    (nodes::SHIN[1], Some(nodes::FOOT[1]), 0.05),
    (nodes::UPPER_ARM[0], Some(nodes::FOREARM[0]), 0.05),
    (nodes::UPPER_ARM[1], Some(nodes::FOREARM[1]), 0.05),
    (nodes::FOREARM[0], Some(nodes::HAND[0]), 0.04),
    (nodes::FOREARM[1], Some(nodes::HAND[1]), 0.04),
];

/// The rider's joints: (parent, child, swing limit, rad).
const JOINTS: [(&str, &str, f32); 10] = [
    (nodes::PELVIS, nodes::TORSO, 0.7),
    (nodes::TORSO, nodes::HEAD, 0.8),
    (nodes::PELVIS, nodes::THIGH[0], 1.4),
    (nodes::PELVIS, nodes::THIGH[1], 1.4),
    (nodes::THIGH[0], nodes::SHIN[0], 1.3),
    (nodes::THIGH[1], nodes::SHIN[1], 1.3),
    (nodes::TORSO, nodes::UPPER_ARM[0], 1.7),
    (nodes::TORSO, nodes::UPPER_ARM[1], 1.7),
    (nodes::UPPER_ARM[0], nodes::FOREARM[0], 1.4),
    (nodes::UPPER_ARM[1], nodes::FOREARM[1], 1.4),
];

/// The bike's pieces: the frame (the bike's root node) and its wheels.
const BIKE: [&str; 3] = [
    bike_nodes::ROOT,
    bike_nodes::WHEEL_FRONT,
    bike_nodes::WHEEL_REAR,
];

/// One rider's ragdoll.
#[derive(Debug)]
struct Doll {
    /// When the crash began (s, client time minus the server's elapsed):
    /// a new crash of the same rider builds a new doll.
    start_s: f64,
    /// The avatar it was made from (its segments are the pieces).
    avatar: Entity,
    /// The pieces by node name.
    pieces: Vec<(String, Entity)>,
    /// Joints, the ground plane and the crash marker.
    extra: Vec<Entity>,
    /// Remounting: where the pieces lay and the fresh avatar they ease onto.
    remount: Option<Remount>,
}

#[derive(Debug)]
struct Remount {
    from: Vec<Transform>,
    /// The fresh avatar (once spawned by the riders plugin).
    avatar: Option<Entity>,
}

/// The ragdolls, and the riders whose avatars are hidden meanwhile.
#[derive(Resource, Debug, Default)]
pub struct Ragdolls {
    dolls: BTreeMap<RiderId, Doll>,
    pub hidden: BTreeSet<RiderId>,
}

impl Ragdolls {
    /// Riders falling or down as a ragdoll, or remounting.
    pub fn active(&self) -> impl Iterator<Item = RiderId> + '_ {
        self.dolls.keys().copied()
    }

    /// The pieces of a rider's ragdoll (tests).
    pub fn pieces(&self, id: RiderId) -> Vec<Entity> {
        self.dolls
            .get(&id)
            .map(|d| d.pieces.iter().map(|p| p.1).collect())
            .unwrap_or_default()
    }

    pub fn remounting(&self, id: RiderId) -> bool {
        self.dolls.get(&id).is_some_and(|d| d.remount.is_some())
    }
}

/// Despawns every ragdoll (a ride switch, with the avatars).
pub(crate) fn reset(world: &mut World) {
    let Some(mut dolls) = world.get_resource_mut::<Ragdolls>() else {
        return;
    };
    let all = std::mem::take(&mut dolls.dolls);
    dolls.hidden.clear();
    for d in all.into_values() {
        let es = d
            .pieces
            .iter()
            .map(|p| p.1)
            .chain(d.extra)
            .chain([d.avatar])
            .chain(d.remount.and_then(|r| r.avatar));
        for e in es {
            if let Ok(e) = world.get_entity_mut(e) {
                e.despawn();
            }
        }
    }
}

/// The entity named `name` below `root`.
fn find(
    root: Entity,
    name: &str,
    children: &Query<&Children>,
    names: &Query<&Name>,
) -> Option<Entity> {
    if names.get(root).is_ok_and(|n| n.as_str() == name) {
        return Some(root);
    }
    let list = children.get(root).ok()?;
    list.iter().find_map(|c| find(c, name, children, names))
}

/// A capsule from a segment's origin to `to` (both in its local frame).
fn segment_collider(to: Vec3, radius: f32) -> Collider {
    let len = to.length();
    if len < 1e-3 {
        return Collider::sphere(radius);
    }
    let rot = Quat::from_rotation_arc(Vec3::Y, to / len);
    Collider::compound(vec![(
        Position(to * 0.5),
        Rotation::from(rot),
        Collider::capsule(radius, (len - 2.0 * radius).max(0.02)),
    )])
}

#[allow(clippy::too_many_arguments, clippy::type_complexity, clippy::map_entry)]
fn drive_ragdolls(
    mut commands: Commands,
    view: Res<RideView>,
    time: Res<Time>,
    mut dolls: ResMut<Ragdolls>,
    mut entities: ResMut<RiderEntities>,
    avatars: Query<&Transform, (With<Avatar>, Without<Piece>)>,
    globals: Query<&GlobalTransform>,
    children: Query<&Children>,
    names: Query<&Name>,
    mut pieces: Query<&mut Transform, (With<Piece>, Without<Avatar>)>,
) {
    let now = time.elapsed_secs_f64();
    let wanted: BTreeMap<RiderId, collide::CrashView> = view
        .riders
        .iter()
        .filter_map(|r| r.crash.map(|c| (r.id, c)))
        .collect();
    // Riders riding again (or gone): their dolls go.
    let done: Vec<RiderId> = dolls
        .dolls
        .iter()
        .filter(|(id, d)| match wanted.get(id) {
            None => true,
            Some(c) => ((now - c.elapsed_s) - d.start_s).abs() > 0.75 && c.on_the_ground(),
        })
        .map(|(id, _)| *id)
        .collect();
    for id in done {
        let d = dolls.dolls.remove(&id).expect("listed");
        for (_, e) in &d.pieces {
            commands.entity(*e).despawn();
        }
        for e in &d.extra {
            commands.entity(*e).despawn();
        }
        if d.remount.is_some() {
            // The old avatar was replaced by the fresh one.
            commands.entity(d.avatar).despawn();
        } else if let Some(e) = entities.0.get(&id).copied()
            && e == d.avatar
        {
            // Never remounted (e.g. a rider who left): build anew.
            entities.0.remove(&id);
            commands.entity(e).despawn();
        }
        dolls.hidden.remove(&id);
    }
    for (id, c) in wanted {
        let start_s = now - c.elapsed_s;
        let rider = view.riders.iter().find(|r| r.id == id).expect("listed");
        if !dolls.dolls.contains_key(&id) {
            if !c.on_the_ground() {
                continue;
            }
            let Some(avatar) = entities.0.get(&id).copied() else {
                continue;
            };
            let Ok(root) = avatars.get(avatar) else {
                continue;
            };
            if let Some(doll) = spawn_doll(
                &mut commands,
                avatar,
                *root,
                &c,
                rider.state.speed,
                rider.is_local,
                &globals,
                &children,
                &names,
                id,
                start_s,
            ) {
                dolls.dolls.insert(id, doll);
                dolls.hidden.insert(id);
            }
            continue;
        }
        let doll = dolls.dolls.get_mut(&id).expect("checked");
        if c.phase != CrashPhase::Remount {
            continue;
        }
        // Remount: the pieces stop being physical and ease onto a fresh
        // avatar in the riding pose.
        if doll.remount.is_none() {
            let mut from = Vec::new();
            for (_, e) in &doll.pieces {
                commands
                    .entity(*e)
                    .remove::<(RigidBody, Collider, LinearVelocity, AngularVelocity)>();
                from.push(pieces.get(*e).copied().unwrap_or_default());
            }
            for e in doll.extra.drain(..) {
                commands.entity(e).despawn();
            }
            // The riders plugin spawns a fresh avatar (hidden meanwhile).
            entities.0.remove(&id);
            doll.remount = Some(Remount { from, avatar: None });
            continue;
        }
        let remount = doll.remount.as_mut().expect("set");
        if remount.avatar.is_none() {
            remount.avatar = entities.0.get(&id).copied().filter(|e| *e != doll.avatar);
            continue;
        }
        let fresh = remount.avatar.expect("set");
        let into = c.elapsed_s - collide::FALL_S - c.down_s;
        let t = (into / (collide::REMOUNT_S * 0.85)).clamp(0.0, 1.0) as f32;
        // Ease in and out: picking the bike up, swinging a leg over.
        let k = t * t * (3.0 - 2.0 * t);
        for ((name, e), from) in doll.pieces.iter().zip(&remount.from) {
            let Some(target) = find(fresh, name, &children, &names)
                .and_then(|n| globals.get(n).ok())
                .map(GlobalTransform::compute_transform)
            else {
                continue;
            };
            if let Ok(mut tr) = pieces.get_mut(*e) {
                tr.translation = from.translation.lerp(target.translation, k);
                tr.rotation = from.rotation.slerp(target.rotation, k);
            }
        }
        if t >= 1.0 {
            // Back on the bike: the fresh avatar takes over.
            dolls.hidden.remove(&id);
        }
    }
}

/// Turns an avatar into a ragdoll: its segments and bike parts become
/// rigid bodies in place, joined, pushed by the crash.
#[allow(clippy::too_many_arguments)]
fn spawn_doll(
    commands: &mut Commands,
    avatar: Entity,
    root: Transform,
    crash: &collide::CrashView,
    speed: f64,
    is_local: bool,
    globals: &Query<&GlobalTransform>,
    children: &Query<&Children>,
    names: &Query<&Name>,
    id: RiderId,
    start_s: f64,
) -> Option<Doll> {
    let node = |name: &str| {
        let e = find(avatar, name, children, names)?;
        Some((e, globals.get(e).ok()?.compute_transform()))
    };
    // Every segment must be there (and placed: a fresh avatar's first
    // frame has no global transforms yet).
    let segs: Vec<(&str, Entity, Transform)> = SEGMENTS
        .iter()
        .map(|(n, _, _)| node(n).map(|(e, t)| (*n, e, t)))
        .collect::<Option<_>>()?;
    let bike: Vec<(&str, Entity, Transform)> = BIKE
        .iter()
        .map(|n| node(n).map(|(e, t)| (*n, e, t)))
        .collect::<Option<_>>()?;
    if segs.iter().all(|s| s.2.translation == Vec3::ZERO) {
        return None;
    }
    // The rider's frame: Bevy models face -Z.
    let forward = root.rotation * Vec3::NEG_Z;
    let right = root.rotation * Vec3::X;
    let speed = speed.max(crash.speed) as f32;
    let push = forward * crash.impulse[0] as f32 + right * crash.impulse[1] as f32;
    let side = if crash.impulse[1] < 0.0 { -1.0 } else { 1.0 };
    let v_rider = forward * speed + push + Vec3::Y * 0.6;
    // Tipping over towards the side of the push.
    let w_rider = forward * (-side * 2.2) + right * 0.8;
    let mut pieces = Vec::new();
    let mut extra = Vec::new();
    let global = |n: &str| {
        segs.iter()
            .chain(bike.iter())
            .find(|s| s.0 == n)
            .map(|s| s.2)
            .or_else(|| node(n).map(|x| x.1))
    };
    for (name, to, radius) in SEGMENTS {
        let (_, e, g) = segs
            .iter()
            .find(|s| s.0 == name)
            .copied()
            .expect("all found");
        let collider = match to {
            Some(child) => {
                let c = global(child).map_or(Vec3::Y * 0.3, |cg| {
                    g.rotation.inverse() * (cg.translation - g.translation) / g.scale
                });
                segment_collider(c, radius)
            }
            None => Collider::compound(vec![(
                Position(Vec3::Y * 0.08),
                Rotation::default(),
                Collider::sphere(radius),
            )]),
        };
        commands.entity(e).remove::<ChildOf>().insert((
            Transform::from_translation(g.translation).with_rotation(g.rotation),
            RigidBody::Dynamic,
            collider,
            LinearVelocity(v_rider),
            AngularVelocity(w_rider),
            LinearDamping(0.2),
            AngularDamping(1.0),
            Friction::new(0.9),
            Restitution::new(0.05),
            TransformInterpolation,
            Piece { rider: id },
        ));
        pieces.push((name.to_owned(), e));
    }
    for (parent, child, swing) in JOINTS {
        let p = segs.iter().find(|s| s.0 == parent).expect("segment");
        let c = segs.iter().find(|s| s.0 == child).expect("segment");
        let anchor = p.2.rotation.inverse() * (c.2.translation - p.2.translation);
        let basis = p.2.rotation.inverse() * c.2.rotation;
        let j = commands
            .spawn((
                SphericalJoint::new(p.1, c.1)
                    .with_local_anchor1(anchor)
                    .with_local_anchor2(Vec3::ZERO)
                    .with_local_basis1(basis)
                    .with_swing_limits(-swing, swing)
                    .with_twist_limits(-0.6, 0.6),
                JointCollisionDisabled,
                JointDamping {
                    linear: 0.5,
                    angular: 2.0,
                },
            ))
            .id();
        extra.push(j);
    }
    // The bike: the frame between the hubs, two wheels on hinges; it
    // slides away from the rider.
    let (_, frame_e, frame_g) = bike[0];
    let hubs: Vec<Vec3> = bike[1..]
        .iter()
        .map(|b| frame_g.rotation.inverse() * (b.2.translation - frame_g.translation))
        .collect();
    let wheel_r = hubs
        .iter()
        .map(|h| h.y)
        .fold(0.3_f32, f32::max)
        .clamp(0.2, 0.4);
    let mid = (hubs[0] + hubs[1]) * 0.5;
    let length = (hubs[0] - hubs[1]).length().max(0.8);
    let v_bike = forward * speed * 0.9 - right * side * 0.8;
    commands.entity(frame_e).remove::<ChildOf>().insert((
        Transform::from_translation(frame_g.translation).with_rotation(frame_g.rotation),
        RigidBody::Dynamic,
        Collider::compound(vec![(
            Position(mid + Vec3::Y * 0.12),
            Rotation::default(),
            Collider::cuboid(0.06, 0.32, length * 0.8),
        )]),
        LinearVelocity(v_bike),
        AngularVelocity(forward * (side * 1.5)),
        LinearDamping(0.3),
        AngularDamping(0.8),
        Friction::new(0.6),
        TransformInterpolation,
        Piece { rider: id },
    ));
    pieces.push((bike_nodes::ROOT.to_owned(), frame_e));
    for ((name, e, g), hub) in bike[1..].iter().zip(&hubs) {
        commands.entity(*e).remove::<ChildOf>().insert((
            Transform::from_translation(g.translation).with_rotation(g.rotation),
            RigidBody::Dynamic,
            Collider::compound(vec![(
                Position(Vec3::ZERO),
                Rotation::from(Quat::from_rotation_z(std::f32::consts::FRAC_PI_2)),
                Collider::cylinder(wheel_r, 0.03),
            )]),
            LinearVelocity(v_bike),
            AngularVelocity(right * (-speed / wheel_r)),
            AngularDamping(0.3),
            Friction::new(0.8),
            TransformInterpolation,
            Piece { rider: id },
        ));
        pieces.push(((*name).to_owned(), *e));
        let j = commands
            .spawn((
                RevoluteJoint::new(frame_e, *e)
                    .with_local_anchor1(*hub)
                    .with_local_anchor2(Vec3::ZERO)
                    .with_hinge_axis(Vec3::X),
                JointCollisionDisabled,
            ))
            .id();
        extra.push(j);
    }
    // The ground under the rider: the road's plane (pitched with it).
    let ground = commands
        .spawn((
            Transform::from_translation(root.translation - root.rotation * (Vec3::Y * 0.1))
                .with_rotation(root.rotation),
            RigidBody::Static,
            Collider::cuboid(60.0, 0.2, 60.0),
            Friction::new(0.9),
        ))
        .id();
    extra.push(ground);
    if !is_local {
        extra.push(
            commands
                .spawn((
                    CrashMarker,
                    Transform::from_translation(root.translation + Vec3::Y * 2.3),
                    Visibility::default(),
                ))
                .id(),
        );
    }
    commands.entity(avatar).insert(Ragdolled);
    Some(Doll {
        start_s,
        avatar,
        pieces,
        extra,
        remount: None,
    })
}

/// The crash marker's mesh and material.
type MarkerLook = (Handle<Mesh>, Handle<StandardMaterial>);

/// Draws the crash markers: a red, glowing cone pointing down at the
/// fallen rider.
#[allow(clippy::type_complexity)]
fn dress_markers(
    mut commands: Commands,
    mut markers: Query<(Entity, &mut Transform), (With<CrashMarker>, Without<Mesh3d>)>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut cache: Local<Option<MarkerLook>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    for (e, mut t) in &mut markers {
        let (mesh, material) = cache
            .get_or_insert_with(|| {
                (
                    meshes.add(Cone::new(0.22, 0.45)),
                    materials.add(StandardMaterial {
                        base_color: Color::srgb(0.95, 0.12, 0.1),
                        emissive: LinearRgba::rgb(3.0, 0.3, 0.2),
                        unlit: true,
                        ..default()
                    }),
                )
            })
            .clone();
        t.rotation = Quat::from_rotation_x(std::f32::consts::PI);
        commands
            .entity(e)
            .insert((Mesh3d(mesh), MeshMaterial3d(material)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::riders::RidersPlugin;
    use ocw_core::ProfileId;
    use ocw_sim::collide::{ContactKind, CrashView};
    use ocw_world::Route;

    const ID: RiderId = RiderId(2);

    fn app() -> App {
        let pts = (0..=40).map(|i| [f64::from(i) * 10.0, 0.0, 0.0]).collect();
        let mut view = RideView::new(Route::from_points("t", "T", pts, false, 6.0).unwrap());
        view.riders.push(crate::RiderView {
            id: ID,
            name: "Bot".into(),
            state: ocw_sim::RiderState {
                distance: 50.0,
                speed: 8.0,
                ..Default::default()
            },
            power_w: 200.0,
            cadence_rpm: None,
            heart_rate_bpm: None,
            is_local: false,
            bike: ProfileId::Road,
            look: None,
            crash: None,
            wobble: 0.0,
        });
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_millis(20),
            ))
            .insert_resource(Time::<Fixed>::from_hz(50.0))
            .insert_resource(view)
            .add_plugins((RidersPlugin, RagdollPlugin));
        app.finish();
        app.cleanup();
        app
    }

    fn set_crash(app: &mut App, c: Option<CrashView>) {
        let mut v = app.world_mut().resource_mut::<RideView>();
        v.riders[0].crash = c;
        if c.is_some() {
            v.riders[0].state.speed = 0.0;
        }
    }

    fn crash(elapsed_s: f64, down_s: f64) -> CrashView {
        CrashView {
            phase: collide::crash_phase(elapsed_s, down_s).unwrap(),
            elapsed_s,
            down_s,
            kind: ContactKind::WheelTouch,
            by: Some(RiderId(3)),
            impulse: [-1.0, 1.5],
            speed: 8.0,
        }
    }

    #[test]
    fn a_crash_becomes_a_ragdoll_that_settles_and_remounts() {
        let mut app = app();
        for _ in 0..3 {
            app.update();
        }
        let avatar = app.world().resource::<RiderEntities>().0[&ID];
        let root_y = app.world().get::<Transform>(avatar).unwrap().translation.y;
        let down = 3.0;
        let mut t = 0.0;
        set_crash(&mut app, Some(crash(t, down)));
        app.update();
        let dolls = app.world().resource::<Ragdolls>();
        assert!(dolls.hidden.contains(&ID), "the avatar is hidden");
        let pieces = dolls.pieces(ID);
        assert_eq!(pieces.len(), SEGMENTS.len() + BIKE.len());
        app.update();
        assert!(
            pieces
                .iter()
                .all(|p| app.world().get::<RigidBody>(*p) == Some(&RigidBody::Dynamic))
        );
        let start: Vec<Vec3> = pieces
            .iter()
            .map(|p| app.world().get::<Transform>(*p).unwrap().translation)
            .collect();
        // Fall, slide and settle.
        while t < collide::FALL_S + down - 0.1 {
            t += 0.02;
            set_crash(&mut app, Some(crash(t, down)));
            app.update();
        }
        let w = app.world();
        let end: Vec<Vec3> = pieces
            .iter()
            .map(|p| w.get::<Transform>(*p).unwrap().translation)
            .collect();
        for (p, e) in pieces.iter().zip(&end) {
            assert!(e.is_finite(), "{e:?}");
            assert!(
                e.y < root_y + 0.75 && e.y > root_y - 0.3,
                "lying on the road: {e:?} (road {root_y})"
            );
            let v = w.get::<LinearVelocity>(*p).map_or(0.0, |v| v.0.length());
            assert!(v < 1.0, "settled: {v}");
        }
        // Slid along the road from where it happened.
        let moved = end
            .iter()
            .zip(&start)
            .map(|(e, s)| (e - s).length())
            .sum::<f32>()
            / end.len() as f32;
        assert!(moved > 0.5, "{moved}");
        // Remount: eased back onto a fresh avatar, which then shows.
        while t < collide::FALL_S + down + collide::REMOUNT_S - 0.02 {
            t += 0.02;
            set_crash(&mut app, Some(crash(t, down)));
            app.update();
        }
        let dolls = app.world().resource::<Ragdolls>();
        assert!(dolls.remounting(ID));
        assert!(!dolls.hidden.contains(&ID), "back on the bike");
        let fresh = app.world().resource::<RiderEntities>().0[&ID];
        assert_ne!(fresh, avatar);
        set_crash(&mut app, None);
        app.update();
        app.update();
        let dolls = app.world().resource::<Ragdolls>();
        assert_eq!(dolls.active().count(), 0);
        assert!(pieces.iter().all(|p| app.world().get_entity(*p).is_err()));
        assert_eq!(
            app.world().get::<Visibility>(fresh),
            Some(&Visibility::Inherited)
        );
    }
}

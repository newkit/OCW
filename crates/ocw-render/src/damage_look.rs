//! Visible damage (spec §14 Phase 12E, K5): simple and cheap marks on the
//! avatars from [`crate::RiderView::damage`] – scratches and scuffs on the
//! frame, a dropped chain hanging below the chainring, dust and a torn kit
//! on the rider – a few small boxes in the bike's frame, spawned once per
//! damaged avatar and shown as the damage grows. The bent wheel wobbles in
//! the avatar's animation ([`wheel_wobble`], `riders::animate`).

use bevy::prelude::*;
use ocw_sim::damage::Visible;

use crate::riders::{Avatar, AvatarRig};

pub struct DamageLookPlugin;

impl Plugin for DamageLookPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, sync_decor);
    }
}

/// How far a badly bent wheel turns out of its plane as it spins (rad).
pub const WOBBLE_RAD: f32 = 0.07;

/// The rotation of a wheel at `angle` (its spin) when bent by `bent`
/// (0–1): the rim swings sideways once per turn.
pub fn wheel_wobble(angle: f32, bent: f32) -> Quat {
    let spin = Quat::from_rotation_x(-angle);
    if bent <= 0.0 {
        return spin;
    }
    Quat::from_rotation_y(WOBBLE_RAD * bent * angle.sin()) * spin
}

/// The scuffs drawn for a scuff level (0–1).
pub const SCUFFS: usize = 4;

pub fn scuffs_shown(level: f32) -> usize {
    if level <= 0.0 {
        0
    } else {
        ((level * SCUFFS as f32).ceil() as usize).clamp(1, SCUFFS)
    }
}

/// The marks of one avatar.
#[derive(Component)]
pub struct DamageDecor {
    pub chain: Entity,
    pub scuffs: [Entity; SCUFFS],
    pub dust: [Entity; 2],
    pub torn: [Entity; 2],
    pub shown: Visible,
}

#[derive(Resource)]
struct DecorAssets {
    cube: Handle<Mesh>,
    chain: Handle<StandardMaterial>,
    scuff: Handle<StandardMaterial>,
    dust: Handle<StandardMaterial>,
    skin: Handle<StandardMaterial>,
}

fn assets(
    have: &mut Option<DecorAssets>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
) -> DecorAssets {
    let a = have.get_or_insert_with(|| DecorAssets {
        cube: meshes.add(Cuboid::new(1.0, 1.0, 1.0)),
        chain: materials.add(StandardMaterial {
            base_color: Color::srgb(0.07, 0.07, 0.08),
            metallic: 0.8,
            perceptual_roughness: 0.45,
            ..default()
        }),
        scuff: materials.add(StandardMaterial {
            base_color: Color::srgb(0.78, 0.78, 0.74),
            perceptual_roughness: 0.95,
            ..default()
        }),
        dust: materials.add(StandardMaterial {
            base_color: Color::srgba(0.55, 0.45, 0.30, 0.6),
            alpha_mode: AlphaMode::Blend,
            perceptual_roughness: 1.0,
            ..default()
        }),
        skin: materials.add(StandardMaterial {
            base_color: Color::srgb(0.80, 0.60, 0.50),
            perceptual_roughness: 0.8,
            ..default()
        }),
    });
    DecorAssets {
        cube: a.cube.clone(),
        chain: a.chain.clone(),
        scuff: a.scuff.clone(),
        dust: a.dust.clone(),
        skin: a.skin.clone(),
    }
}

/// A box from `a` to `b` (bike frame), `w` × `h` across.
fn segment(a: Vec3, b: Vec3, w: f32, h: f32) -> Transform {
    let d = b - a;
    let len = d.length().max(1e-3);
    Transform {
        translation: (a + b) * 0.5,
        rotation: Quat::from_rotation_arc(Vec3::Z, d / len),
        scale: Vec3::new(w, h, len),
    }
}

/// Where the marks sit on a bike (`rig`: its points, front = −Z).
pub fn decor_transforms(rig: &ocw_assets::pose::BikeRig) -> DecorPlaces {
    let rear_hub = Vec3::new(0.0, rig.wheel_radius, rig.wheelbase * 0.5);
    let right = 0.055;
    // The chain hangs off the chainring, sagging between it and the hub.
    let ring = rig.crank + Vec3::new(right, -0.10, 0.0);
    let hub = rear_hub + Vec3::new(right, -0.02, 0.0);
    let sag = (ring + hub) * 0.5 + Vec3::new(0.0, -0.14, 0.0);
    let chain = [
        segment(ring, sag, 0.012, 0.012),
        segment(sag, hub, 0.012, 0.012),
    ];
    // Scuffs along the down tube and the top tube, both sides.
    let head = (rig.hoods[0] + rig.hoods[1]) * 0.5 + Vec3::new(0.0, -0.12, 0.06);
    let mut scuffs = [Transform::IDENTITY; SCUFFS];
    for (i, s) in scuffs.iter_mut().enumerate() {
        let t = 0.25 + 0.17 * i as f32;
        let side = if i % 2 == 0 { 0.028 } else { -0.028 };
        let on_down_tube = rig.crank.lerp(head, t) + Vec3::new(side, 0.0, 0.0);
        *s = Transform {
            translation: on_down_tube,
            rotation: Quat::from_rotation_arc(Vec3::Z, (head - rig.crank).normalize_or(Vec3::Z)),
            scale: Vec3::new(0.004, 0.022, 0.06),
        };
    }
    // Dust on the hip and the back, a tear at both hips.
    let hip = rig.saddle + Vec3::new(0.0, 0.05, 0.02);
    let back = rig.saddle.lerp(head, 0.45) + Vec3::new(0.0, 0.38, 0.0);
    let dust = [
        Transform::from_translation(hip + Vec3::new(0.158, -0.02, 0.0))
            .with_scale(Vec3::new(0.008, 0.07, 0.09)),
        Transform::from_translation(back).with_scale(Vec3::new(0.14, 0.012, 0.12)),
    ];
    let torn = [
        Transform::from_translation(hip + Vec3::new(0.16, -0.07, -0.07))
            .with_scale(Vec3::new(0.008, 0.035, 0.05)),
        Transform::from_translation(hip + Vec3::new(-0.16, -0.07, -0.07))
            .with_scale(Vec3::new(0.008, 0.035, 0.05)),
    ];
    DecorPlaces {
        chain,
        scuffs,
        dust,
        torn,
    }
}

/// The marks' places on one bike.
pub struct DecorPlaces {
    pub chain: [Transform; 2],
    pub scuffs: [Transform; SCUFFS],
    pub dust: [Transform; 2],
    pub torn: [Transform; 2],
}

fn shown(v: bool) -> Visibility {
    if v {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    }
}

#[allow(clippy::type_complexity)]
fn sync_decor(
    mut commands: Commands,
    view: Res<crate::RideView>,
    mut have: Local<Option<DecorAssets>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut avatars: Query<(Entity, &Avatar, &AvatarRig, Option<&mut DamageDecor>)>,
    mut vis: Query<&mut Visibility>,
) {
    for (e, avatar, rig, decor) in &mut avatars {
        let want = view
            .riders
            .iter()
            .find(|r| r.id == avatar.id && avatar.id != crate::riders::GHOST_ID)
            .and_then(|r| r.damage)
            .unwrap_or_default();
        match decor {
            None if want.any() => {
                let a = assets(&mut have, &mut meshes, &mut materials);
                let places = decor_transforms(rig.bike_rig());
                let sway = rig.sway_entity();
                // The chain: two links of one entity each, under a parent.
                let chain = commands
                    .spawn((Transform::IDENTITY, Visibility::Hidden, ChildOf(sway)))
                    .id();
                for t in places.chain {
                    commands.spawn((
                        Mesh3d(a.cube.clone()),
                        MeshMaterial3d(a.chain.clone()),
                        t,
                        Visibility::Inherited,
                        ChildOf(chain),
                    ));
                }
                let mut spawn = |t: Transform, m: &Handle<StandardMaterial>| {
                    commands
                        .spawn((
                            Mesh3d(a.cube.clone()),
                            MeshMaterial3d(m.clone()),
                            t,
                            Visibility::Hidden,
                            ChildOf(sway),
                        ))
                        .id()
                };
                let scuffs = places.scuffs.map(|t| spawn(t, &a.scuff));
                let dust = places.dust.map(|t| spawn(t, &a.dust));
                let torn = places.torn.map(|t| spawn(t, &a.skin));
                commands.entity(e).insert(DamageDecor {
                    chain,
                    scuffs,
                    dust,
                    torn,
                    shown: Visible::default(),
                });
            }
            None => {}
            Some(mut d) => {
                if d.shown == want {
                    continue;
                }
                d.shown = want;
                let mut set = |e: Entity, on: bool| {
                    if let Ok(mut v) = vis.get_mut(e) {
                        let s = shown(on);
                        if *v != s {
                            *v = s;
                        }
                    }
                };
                set(d.chain, want.chain_dropped);
                let n = scuffs_shown(want.scuffs);
                for (i, s) in d.scuffs.iter().enumerate() {
                    set(*s, i < n);
                }
                set(d.dust[0], want.dust > 0.15);
                set(d.dust[1], want.dust > 0.45);
                for t in d.torn {
                    set(t, want.torn);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bent_wheel_swings_once_per_turn_and_a_true_one_does_not() {
        let true_wheel = wheel_wobble(1.3, 0.0);
        assert!(true_wheel.angle_between(Quat::from_rotation_x(-1.3)) < 1e-6);
        // A quarter turn: the rim out of its plane by the wobble.
        let bent = wheel_wobble(std::f32::consts::FRAC_PI_2, 1.0);
        let axle = bent * Vec3::X;
        assert!((axle.z.abs() - WOBBLE_RAD.sin()).abs() < 1e-3, "{axle:?}");
        let half = wheel_wobble(std::f32::consts::PI, 1.0) * Vec3::X;
        assert!(half.z.abs() < 1e-3, "back in its plane after half a turn");
    }

    #[test]
    fn scuffs_grow_with_the_damage() {
        assert_eq!(scuffs_shown(0.0), 0);
        assert_eq!(scuffs_shown(0.05), 1);
        assert_eq!(scuffs_shown(0.6), 3);
        assert_eq!(scuffs_shown(1.0), SCUFFS);
    }

    #[test]
    fn the_chain_hangs_below_the_chainring_on_the_right() {
        let rig = ocw_assets::pose::BikeRig::from_model(ocw_assets::shipped::bike_model(
            ocw_core::ProfileId::Road,
        ))
        .unwrap();
        let p = decor_transforms(&rig);
        for t in p.chain {
            assert!(t.translation.x > 0.0, "the drive side");
            assert!(t.translation.y < rig.crank.y, "below the crank axle");
        }
        // The marks are on the bike, within its length.
        for t in p.scuffs {
            assert!(t.translation.z.abs() < rig.wheelbase);
        }
    }
}

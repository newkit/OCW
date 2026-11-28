//! The rider status signs on the avatars (spec §2.6 *Rider status signs*,
//! §7.1.7 Q11): the model files of `artwork/3dmodels/` (Phase 15) worn at
//! the head – headphones ("cannot hear you", per viewer), the VR headset
//! (glasses hidden while it is on) and the X over a muted rider's mouth –
//! and the phone in the right hand while the rider is in the menu. The
//! timeline (grab / hold / store, the scale-in and out) is
//! `ocw_assets::anim::StatusAnim`, the hand and the phone's place
//! `ocw_assets::pose`; this module spawns the parts and writes their
//! transforms and visibility. Far away (beyond
//! [`crate::riders::LOD_DISTANCE_M`]) the files' `lod1` shows on the far
//! rider's head: two dark discs, a dark band; the phone a dark slab at the
//! chest (no grab animation there). Looks only.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use ocw_assets::anim::StatusAnim;
use ocw_assets::model::Model;
use ocw_assets::pose::{self, BikeRig, Fit, PoseParams, RiderRig, bike_nodes, nodes};
use ocw_assets::slots;

use crate::riders::{AvatarColours, far_range, near_range, tint_color};

/// The model files of the signs (spec §2.6; Phase 15 names: function
/// first, then shape).
pub const HEADPHONES: &str = "wear/headphones-over-ear";
pub const HEADSET: &str = "wear/headset-vr-visor";
pub const MOUTH: &str = "wear/mark-mouth-cross";
pub const PHONE: &str = "props/phone-slab";

/// The parts of one avatar's signs (on its root entity).
#[derive(Component, Debug, Clone)]
pub struct SignParts {
    /// Worn at the head (near) and on the far rider's head (far).
    pub headphones: [Entity; 2],
    pub headset: [Entity; 2],
    pub mouth: Entity,
    /// In the hand (near, under the bike's sway) and at the far chest.
    pub phone: [Entity; 2],
    /// The rider's glasses (hidden under the headset).
    pub glasses: Vec<Entity>,
}

/// One model's meshes and slot names, per node: `lod1` and below are far.
struct SignMesh {
    near: Vec<(Mesh, String, [f32; 4])>,
    far: Vec<(Mesh, String, [f32; 4])>,
}

fn mesh_of(model: &Model) -> SignMesh {
    let lod = model.node(bike_nodes::LOD1);
    let mut out = SignMesh {
        near: Vec::new(),
        far: Vec::new(),
    };
    for (i, n) in model.nodes.iter().enumerate() {
        let w = model.world(i);
        let far = lod.is_some_and(|l| model.is_below(i, l));
        for p in &n.primitives {
            let mut m = Mesh::new(
                PrimitiveTopology::TriangleList,
                RenderAssetUsages::default(),
            );
            let pos: Vec<[f32; 3]> = p
                .positions
                .iter()
                .map(|v| w.transform_point3(Vec3::from(*v)).to_array())
                .collect();
            let nor: Vec<[f32; 3]> = p
                .normals
                .iter()
                .map(|v| {
                    w.transform_vector3(Vec3::from(*v))
                        .normalize_or_zero()
                        .to_array()
                })
                .collect();
            m.insert_attribute(Mesh::ATTRIBUTE_POSITION, pos);
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, nor);
            m.insert_indices(Indices::U32(p.indices.clone()));
            let mat = &model.materials[p.material];
            let entry = (m, mat.name.clone(), mat.color);
            if far {
                out.far.push(entry);
            } else {
                out.near.push(entry);
            }
        }
    }
    out
}

/// A sign's material: the slot's colour for this rider (the headphones'
/// cups and the headset's plate in the kit's accent), the phone's screen
/// glowing.
fn material(slot: &str, color: [f32; 4], colours: &AvatarColours) -> StandardMaterial {
    let s = slots::get(slot).unwrap_or(&slots::ALL[0]);
    let base = tint_color(slot, color, colours);
    let mut m = StandardMaterial {
        base_color: base,
        metallic: s.metallic,
        perceptual_roughness: s.roughness,
        ..default()
    };
    if slot == "phone_screen" {
        m.emissive = base.to_linear() * 1.6;
    }
    m
}

/// The far rider's head in the bike's frame (the bike file's `lod1`
/// rider: the low rider seated as `ocw_assets::bikes` poses it).
pub fn far_head(bike: &BikeRig) -> (Transform, Transform) {
    let low = ocw_assets::shipped::rider_low_model();
    let Ok(rig) = RiderRig::from_model(low) else {
        return (Transform::default(), Transform::default());
    };
    let params = PoseParams {
        crank: 1.1,
        ..Default::default()
    };
    let p = pose::pose(&rig, bike, &Fit::new(&rig, bike), &params);
    let mut posed = low.clone();
    p.apply(&mut posed);
    let at = |name: &str| {
        posed
            .node(name)
            .map(|i| Transform::from_matrix(Mat4::from(posed.world(i))))
            .unwrap_or_default()
    };
    // The chest: in front of the torso, at the far rider's hands' height.
    let torso = at(nodes::TORSO);
    let chest = Transform::from_translation(
        torso.translation + torso.rotation * Vec3::new(0.05, 0.3, -0.25),
    )
    .with_rotation(torso.rotation * Quat::from_rotation_x(-0.9));
    (at(nodes::HEAD), chest)
}

#[allow(clippy::too_many_arguments)]
fn spawn_one(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    parent: Entity,
    at: Transform,
    id: &str,
    far: bool,
    colours: &AvatarColours,
) -> Entity {
    let root = commands
        .spawn((
            at.with_scale(Vec3::ZERO),
            Visibility::Hidden,
            ChildOf(parent),
            Name::new(format!("sign {id}")),
        ))
        .id();
    let model = ocw_assets::shipped::model(id);
    let m = mesh_of(&model);
    let list = if far { m.far } else { m.near };
    let range = if far { far_range() } else { near_range() };
    for (mesh, slot, color) in list {
        commands.spawn((
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(materials.add(material(&slot, color, colours))),
            range.clone(),
            ChildOf(root),
        ));
    }
    root
}

/// Spawns an avatar's sign parts (hidden): the worn ones under the head
/// entity `head`, the phone and the far parts under the bike's `sway`.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    head: Entity,
    sway: Entity,
    bike: &BikeRig,
    glasses: Vec<Entity>,
    colours: &AvatarColours,
) -> SignParts {
    let (far_head, chest) = far_head(bike);
    // The worn signs' files are modelled round the neck's joint, the head
    // node's origin before the head got its own joint under the skull
    // (bug 2026-11-28): they sit that much lower in the head's frame.
    let in_head = Transform::from_translation(-Vec3::from(ocw_assets::rider::HEAD_JOINT));
    let far_head = far_head.mul_transform(in_head);
    let mut one = |parent: Entity, at: Transform, id: &str, far: bool| {
        spawn_one(commands, meshes, materials, parent, at, id, far, colours)
    };
    let headphones = [
        one(head, in_head, HEADPHONES, false),
        one(sway, far_head, HEADPHONES, true),
    ];
    let headset = [
        one(head, in_head, HEADSET, false),
        one(sway, far_head, HEADSET, true),
    ];
    let mouth = one(head, in_head, MOUTH, false);
    let phone = [
        one(sway, Transform::IDENTITY, PHONE, false),
        one(sway, chest, PHONE, true),
    ];
    SignParts {
        headphones,
        headset,
        mouth,
        phone,
        glasses,
    }
}

/// What a sign shows at scale `s` (0 … 1, eased).
pub fn shown(s: f32) -> (f32, Visibility) {
    let e = s * s * (3.0 - 2.0 * s);
    if s <= 0.0 {
        (0.0, Visibility::Hidden)
    } else {
        (e, Visibility::Inherited)
    }
}

/// Writes the signs of one avatar: the worn parts' scale, the glasses, the
/// phone in the hand (`phone`: its place from the pose; `None` beyond the
/// near distance, or in the pocket).
pub fn update(
    parts: &SignParts,
    status: &StatusAnim,
    phone: Option<(Vec3, Quat)>,
    transforms: &mut Query<&mut Transform>,
    visibility: &mut Query<&mut Visibility>,
) {
    let mut set = |e: Entity, s: f32, at: Option<(Vec3, Quat)>| {
        let (k, vis) = shown(s);
        if let Ok(mut t) = transforms.get_mut(e) {
            if let Some((p, q)) = at {
                t.translation = p;
                t.rotation = q;
            }
            t.scale = Vec3::splat(k.max(1e-4));
        }
        if let Ok(mut v) = visibility.get_mut(e) {
            v.set_if_neq(vis);
        }
    };
    for e in parts.headphones {
        set(e, status.headphones, None);
    }
    for e in parts.headset {
        set(e, status.headset, None);
    }
    set(parts.mouth, status.mouth, None);
    // The phone: in the hand up close; held (not in the grab) far away.
    let in_hand = phone.filter(|_| status.phone_in_hand());
    set(
        parts.phone[0],
        if in_hand.is_some() { 1.0 } else { 0.0 },
        in_hand,
    );
    set(
        parts.phone[1],
        if status.phone >= 1.0 { 1.0 } else { 0.0 },
        None,
    );
    // Glasses come off under the headset (and back when it is off).
    let glasses = if status.headset > 0.0 {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    for e in &parts.glasses {
        if let Ok(mut v) = visibility.get_mut(*e) {
            v.set_if_neq(glasses);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_files_are_shipped_with_their_far_models() {
        for id in [HEADPHONES, HEADSET, MOUTH, PHONE] {
            let m = ocw_assets::shipped::model(id);
            let s = mesh_of(&m);
            assert!(!s.near.is_empty(), "{id}");
            // The X mouth is too small for the far model.
            assert_eq!(s.far.is_empty(), id == MOUTH, "{id}");
        }
    }

    #[test]
    fn signs_scale_in_and_hide_at_zero() {
        assert_eq!(shown(0.0).1, Visibility::Hidden);
        assert_eq!(shown(1.0), (1.0, Visibility::Inherited));
        let (half, v) = shown(0.5);
        assert!(half > 0.0 && half < 1.0 && v == Visibility::Inherited);
    }

    #[test]
    fn the_far_head_sits_above_the_far_bike() {
        for p in ocw_core::ProfileId::ALL {
            let bike = ocw_assets::shipped::bike_rig(p);
            let (head, chest) = far_head(&bike);
            assert!(
                head.translation.y > 0.9 && head.translation.y < 2.0,
                "{p:?}"
            );
            assert!(chest.translation.y < head.translation.y, "{p:?}");
        }
    }
}

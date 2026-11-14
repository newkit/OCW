//! Fading of see-through objects (spec §3.2, "Camera and avatar"): trees
//! never move the camera ([`crate::camera::CameraInteraction::SeeThrough`]);
//! instead the parts of them right in front of the lens or between the
//! camera and the rider are faded out.
//!
//! [`SeeThroughMaterial`] is a [`StandardMaterial`] with a small fragment
//! shader (`see_through.wesl`) that discards fragments in a 4 × 4 ordered
//! dither (screen-door transparency) by [`keep`]: no blending, so depth
//! sorting stays as for opaque meshes, and the shadow pass (which uses the
//! standard prepass shader) still draws the whole tree – a faded crown keeps
//! its shadow. [`keep`] is the CPU twin of the shader, for the tests.

use bevy::asset::{AssetPath, embedded_asset, embedded_path};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use crate::camera::CameraFocus;

/// A standard material whose fragments fade out near the camera and in
/// the line of sight to the rider. Use it for see-through objects only.
pub type SeeThroughMaterial = ExtendedMaterial<StandardMaterial, SeeThroughFade>;

/// Within this distance of the camera nothing of a see-through object is
/// drawn (m); it fades in over [`FADE_RAMP_M`] beyond.
pub const NEAR_FADE_M: f32 = 3.0;
/// Radius of the faded tube along the line of sight at the camera and at
/// the rider (m): a cone that frames the rider.
pub const SIGHT_RADIUS_EYE_M: f32 = 0.8;
pub const SIGHT_RADIUS_RIDER_M: f32 = 2.0;
/// Width of the soft edge of every faded region (m).
pub const FADE_RAMP_M: f32 = 1.2;
/// The sight tube ends this far before the rider's head (m), so trees
/// right next to the rider stay.
pub const RIDER_GAP_M: f32 = 1.0;

pub struct SeeThroughPlugin;

impl Plugin for SeeThroughPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "see_through.wesl");
        app.add_plugins(MaterialPlugin::<SeeThroughMaterial>::default())
            .add_systems(PostUpdate, update_focus.after(crate::camera::follow));
    }
}

/// The fade parameters (one uniform, binding 100 after the standard
/// material's bindings).
#[derive(Asset, AsBindGroup, Reflect, Debug, Clone, PartialEq)]
pub struct SeeThroughFade {
    /// xyz: the camera's focus (the rider's head); w: where the sight fade
    /// ends before it ([`RIDER_GAP_M`]), negative = no focus (only the
    /// near fade).
    #[uniform(100)]
    pub focus: Vec4,
    /// x: [`NEAR_FADE_M`], y/z: sight radius at the eye/at the focus,
    /// w: [`FADE_RAMP_M`].
    #[uniform(100)]
    pub shape: Vec4,
    /// The weather (spec §14 Phase 9, S4; [`crate::weather::sync_surfaces`]):
    /// x / z the wind's direction (scene axes), y how much the trees sway
    /// (0–1), w snow on the crowns (0–1).
    #[uniform(100)]
    pub wind: Vec4,
    /// How the vertex shader finds a vertex's height above the tree's foot
    /// for the sway: x 0 = from uv.x (merged package trees), 1 = the
    /// mesh's local y plus y (a part of a route tree).
    #[uniform(100)]
    pub sway: Vec4,
}

impl Default for SeeThroughFade {
    fn default() -> Self {
        Self {
            focus: focus_uniform(None),
            shape: Vec4::new(
                NEAR_FADE_M,
                SIGHT_RADIUS_EYE_M,
                SIGHT_RADIUS_RIDER_M,
                FADE_RAMP_M,
            ),
            wind: Vec4::new(1.0, 0.0, 0.0, 0.0),
            sway: Vec4::new(1.0, 0.0, 0.0, 0.0),
        }
    }
}

impl MaterialExtension for SeeThroughFade {
    fn vertex_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("see_through.wesl")).with_source("embedded"),
        )
    }

    fn fragment_shader() -> ShaderRef {
        ShaderRef::Path(
            AssetPath::from_path_buf(embedded_path!("see_through.wesl")).with_source("embedded"),
        )
    }
}

/// A see-through material on top of `base`.
pub fn see_through(base: StandardMaterial) -> SeeThroughMaterial {
    ExtendedMaterial {
        base,
        extension: SeeThroughFade::default(),
    }
}

/// A tree material that sways in the wind (spec §14 Phase 9, S4): its
/// meshes carry the height above the tree's foot in uv.x
/// (`package::tree_arrays`), or (`local_y_offset`) are a part of a tree
/// whose foot lies that far below the mesh's origin.
pub fn swaying(mut m: SeeThroughMaterial, local_y_offset: Option<f32>) -> SeeThroughMaterial {
    m.extension.sway = match local_y_offset {
        Some(y) => Vec4::new(1.0, y, 0.0, 0.0),
        None => Vec4::new(0.0, 0.0, 0.0, 0.0),
    };
    m
}

/// How far a vertex `height_m` above the foot of a tree of `tree_m`
/// sways at sway amount `amount` (0–1) and phase `phase` (rad), mirrored by
/// `see_through.wesl`: a steady lean downwind growing with the square of
/// the height, an oscillation on top, and a flutter of the twigs.
pub fn sway_offset(height_m: f32, tree_m: f32, amount: f32, phase: f32) -> f32 {
    if amount <= 0.0 || height_m <= 0.3 {
        return 0.0;
    }
    let k = (height_m / tree_m.max(1.0)).min(1.5);
    let bend = amount * k * k * (0.55 + 0.45 * phase.sin()) * 1.6;
    bend + 0.06 * amount * k * (phase * 4.3).sin()
}

fn focus_uniform(focus: Option<Vec3>) -> Vec4 {
    match focus {
        Some(f) => f.extend(RIDER_GAP_M),
        None => Vec4::new(0.0, 0.0, 0.0, -1.0),
    }
}

/// How much of a see-through fragment at `p` is drawn for a camera at
/// `eye` (0 = none, 1 = all); mirrors `see_through.wesl`.
pub fn keep(p: Vec3, eye: Vec3, fade: &SeeThroughFade) -> f32 {
    let shape = fade.shape;
    let ramp = shape.w.max(1e-3);
    let mut keep = ((p.distance(eye) - shape.x) / ramp).clamp(0.0, 1.0);
    let focus = fade.focus;
    if focus.w >= 0.0 {
        let to_focus = focus.truncate() - eye;
        let len = to_focus.length();
        if len > 1e-3 {
            let dir = to_focus / len;
            let along = (p - eye).dot(dir);
            if along > 0.0 {
                let off_axis = (p - eye - dir * along).length();
                let radius = shape.y + (shape.z - shape.y) * (along / len).min(1.0);
                let sight = ((off_axis - radius) / ramp).clamp(0.0, 1.0);
                // Ends softly before the focus.
                let end = ((len - focus.w - along) / ramp).clamp(0.0, 1.0);
                keep = keep.min(1.0 - end * (1.0 - sight));
            }
        }
    }
    keep
}

/// Hands the camera focus to every see-through material (also to ones
/// created after the last change).
fn update_focus(focus: Res<CameraFocus>, mut materials: ResMut<Assets<SeeThroughMaterial>>) {
    let f = focus_uniform(focus.0);
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.extension.focus != f)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut m) = materials.get_mut(id) {
            m.extension.focus = f;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn focused(focus: Vec3) -> SeeThroughFade {
        SeeThroughFade {
            focus: focus_uniform(Some(focus)),
            ..default()
        }
    }

    #[test]
    fn fades_right_in_front_of_the_lens() {
        let fade = SeeThroughFade::default();
        let eye = Vec3::new(5.0, 2.0, 5.0);
        assert_eq!(keep(eye + Vec3::X, eye, &fade), 0.0);
        assert_eq!(keep(eye + Vec3::X * NEAR_FADE_M, eye, &fade), 0.0);
        let half = keep(
            eye + Vec3::X * (NEAR_FADE_M + FADE_RAMP_M / 2.0),
            eye,
            &fade,
        );
        assert!((half - 0.5).abs() < 1e-4, "{half}");
        assert_eq!(keep(eye + Vec3::X * 10.0, eye, &fade), 1.0);
    }

    #[test]
    fn fades_between_the_camera_and_the_rider_only() {
        let eye = Vec3::new(0.0, 3.0, 8.0);
        let head = Vec3::new(0.0, 1.2, -4.0);
        let fade = focused(head);
        // Half way, on the line of sight: gone; 3 m beside it: drawn.
        let mid = eye.lerp(head, 0.5);
        assert_eq!(keep(mid, eye, &fade), 0.0);
        assert!(keep(mid + Vec3::X * 0.9, eye, &fade) < 0.5);
        assert_eq!(keep(mid + Vec3::X * 3.5, eye, &fade), 1.0);
        // Behind the rider and behind the camera (beyond the near fade): drawn.
        assert_eq!(keep(head + (head - eye) * 0.5, eye, &fade), 1.0);
        assert_eq!(keep(eye + (eye - head) * 0.5, eye, &fade), 1.0);
        // Right at the rider (inside the gap): drawn.
        let at_rider = head + (eye - head).normalize() * 0.2;
        assert_eq!(keep(at_rider, eye, &fade), 1.0);
        // Without a focus, only the near fade.
        assert_eq!(keep(mid, eye, &SeeThroughFade::default()), 1.0);
    }

    #[test]
    fn every_see_through_material_gets_the_focus() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<SeeThroughMaterial>()
            .init_resource::<CameraFocus>()
            .add_systems(Update, update_focus);
        let a = app
            .world_mut()
            .resource_mut::<Assets<SeeThroughMaterial>>()
            .add(see_through(StandardMaterial::default()));
        app.update();
        let materials = app.world().resource::<Assets<SeeThroughMaterial>>();
        assert_eq!(materials.get(&a).unwrap().extension.focus.w, -1.0);
        let head = Vec3::new(1.0, 2.0, 3.0);
        app.world_mut().resource_mut::<CameraFocus>().0 = Some(head);
        let b = app
            .world_mut()
            .resource_mut::<Assets<SeeThroughMaterial>>()
            .add(see_through(StandardMaterial::default()));
        app.update();
        let materials = app.world().resource::<Assets<SeeThroughMaterial>>();
        for h in [&a, &b] {
            assert_eq!(
                materials.get(h).unwrap().extension.focus,
                head.extend(RIDER_GAP_M)
            );
        }
    }
}

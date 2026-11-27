//! Road vehicles in the frame (spec 14 Phase 18, 18E T5, T7): the view
//! type the client fills ([`VehicleView`] in [`RideView::vehicles`]), the
//! vehicles drawn with their models ([`models`]: instanced, levels of
//! detail within the quality's budget ([`lod`]), lamps ([`lamps`]), the
//! colours of the region ([`look`]), turning wheels, the trailer, spray),
//! and a **debug view** – one plain box per vehicle in its class's size
//! and a colour from its look seed, with red brake lamps – instead of the
//! models behind the dev switch [`TrafficDebug`] (`OCW_TRAFFIC_BOXES=1` in
//! the environment, or the console command `traffic.boxes on`).

pub mod lamps;
pub mod lod;
pub mod look;
pub mod models;

use std::collections::BTreeMap;

use bevy::prelude::*;
use ocw_sim::traffic::VehicleClass;

use crate::console::{ConsoleAppExt, ConsoleCommand, Permission};
use crate::{RideView, to_vec3};

/// A road vehicle as the renderer sees it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleView {
    pub id: u32,
    pub class: VehicleClass,
    /// Seeds the look: body shape and colour (T7).
    pub look: u32,
    /// The ground under the centre of the vehicle (local metres, y up),
    /// in its lane.
    pub pos: [f64; 3],
    /// The direction it drives (unit, x, z).
    pub heading: [f64; 2],
    /// Metres right of the road's centreline (its lane).
    pub lateral: f64,
    pub lane: u8,
    pub speed: f64,
    /// Braking or standing: the brake lamps are lit.
    pub braking: bool,
    /// The indicator set: turning at the next junction or pulling out.
    pub indicator: Indicator,
}

/// What a vehicle did to the local rider (the server's T4 events, spec
/// 18E T7), numbered by the client.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VehicleCue {
    /// Rising: a new cue has a higher number.
    pub seq: u64,
    pub vehicle: u32,
    pub kind: VehicleCueKind,
    /// The client time it came (s).
    pub at_s: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum VehicleCueKind {
    /// A short honk (mostly polite, some realism, lunatics; Q8).
    Honk,
    /// It finished passing the rider.
    Passed,
    /// It closes in from behind, reaching the rider in `seconds`.
    Approaching { seconds: f64 },
}

/// A vehicle's indicator (spec 18E T7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Indicator {
    #[default]
    Off,
    Left,
    Right,
}

impl VehicleView {
    /// The yaw of the heading (rad, about +Y; 0 = −Z, as the riders).
    pub fn yaw(&self) -> f32 {
        (-self.heading[0]).atan2(-self.heading[1]) as f32
    }
}

/// Length, width and height of a class's debug box (m; the T6 models'
/// typical sizes).
pub fn class_size(class: VehicleClass) -> Vec3 {
    match class {
        VehicleClass::Car => Vec3::new(1.8, 1.5, 4.5),
        VehicleClass::Van => Vec3::new(2.0, 2.4, 5.4),
        VehicleClass::Lorry => Vec3::new(2.5, 3.6, 12.0),
        VehicleClass::Bus => Vec3::new(2.55, 3.1, 12.0),
        VehicleClass::Motorbike => Vec3::new(0.8, 1.4, 2.2),
        VehicleClass::Tractor => Vec3::new(2.4, 2.8, 4.5),
    }
}

/// The body colour of a look seed: mostly greys, whites and blacks, a
/// few colours (18B's palette of real-world car colours).
pub fn look_colour(look: u32) -> Color {
    const PALETTE: [[f32; 3]; 12] = [
        [0.92, 0.92, 0.92],
        [0.85, 0.86, 0.88],
        [0.55, 0.56, 0.58],
        [0.35, 0.36, 0.38],
        [0.08, 0.08, 0.09],
        [0.12, 0.13, 0.15],
        [0.65, 0.66, 0.68],
        [0.95, 0.95, 0.93],
        [0.55, 0.06, 0.06],
        [0.08, 0.18, 0.45],
        [0.12, 0.32, 0.18],
        [0.75, 0.55, 0.15],
    ];
    let c = PALETTE[(look.wrapping_mul(2_654_435_761) >> 28) as usize % PALETTE.len()];
    Color::srgb(c[0], c[1], c[2])
}

/// The dev switch of the debug boxes (off: nothing is drawn; T7 draws
/// the models).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrafficDebug {
    pub boxes: bool,
}

impl Default for TrafficDebug {
    fn default() -> Self {
        Self {
            boxes: std::env::var("OCW_TRAFFIC_BOXES").is_ok_and(|v| v != "0" && !v.is_empty()),
        }
    }
}

/// Draws [`RideView::vehicles`] as boxes while [`TrafficDebug::boxes`],
/// else with their models ([`models::VehicleModelsPlugin`]).
pub struct TrafficDebugPlugin;

impl Plugin for TrafficDebugPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<TrafficDebug>()
            .init_resource::<BoxEntities>()
            .add_systems(PostUpdate, draw_boxes)
            .console_command(ConsoleCommand::new(
                Permission::User,
                clap::Command::new("traffic.boxes")
                    .about("Road vehicles as debug boxes instead of their models (spec 18E T5, T7)")
                    .arg(
                        clap::Arg::new("on")
                            .value_parser(["on", "off"])
                            .help("on | off (without: what it is)"),
                    ),
                boxes_command,
            ));
    }
}

fn boxes_command(world: &mut World, m: &clap::ArgMatches) -> Result<String, String> {
    if let Some(v) = m.get_one::<String>("on") {
        world.resource_mut::<TrafficDebug>().boxes = v == "on";
    }
    let n = world.resource::<RideView>().vehicles.len();
    let d = world
        .get_resource::<models::TrafficDrawStats>()
        .cloned()
        .unwrap_or_default();
    Ok(format!(
        "traffic boxes: {} ({n} vehicles near the rider; models drawn: {} near, {} lod1, {} boxes, {} not drawn; {} with headlights, {} braking, {} indicating, {} spot lights, {} spray plumes)",
        if world.resource::<TrafficDebug>().boxes {
            "on"
        } else {
            "off"
        },
        d.near,
        d.lod1,
        d.boxes,
        d.culled,
        d.head,
        d.brake,
        d.blinking,
        d.spots,
        d.spray
    ))
}

#[derive(Component)]
struct VehicleBox;

#[derive(Resource, Default)]
struct BoxEntities {
    boxes: BTreeMap<u32, Entity>,
    mesh: Option<Handle<Mesh>>,
    lamp_mesh: Option<Handle<Mesh>>,
    lamp_on: Option<Handle<StandardMaterial>>,
    lamp_off: Option<Handle<StandardMaterial>>,
    colours: BTreeMap<u32, Handle<StandardMaterial>>,
}

#[derive(Component)]
struct BrakeLamp;

#[allow(clippy::too_many_arguments)] // a Bevy system: one parameter per resource
fn draw_boxes(
    mut commands: Commands,
    view: Res<RideView>,
    debug: Res<TrafficDebug>,
    mut ents: ResMut<BoxEntities>,
    meshes: Option<ResMut<Assets<Mesh>>>,
    materials: Option<ResMut<Assets<StandardMaterial>>>,
    mut boxes: Query<&mut Transform, With<VehicleBox>>,
    children: Query<&Children>,
    mut lamps: Query<&mut MeshMaterial3d<StandardMaterial>, With<BrakeLamp>>,
) {
    let (Some(mut meshes), Some(mut materials)) = (meshes, materials) else {
        return;
    };
    let ents = &mut *ents;
    let shown: &[VehicleView] = if debug.boxes { &view.vehicles } else { &[] };
    ents.boxes.retain(|id, e| {
        let keep = shown.iter().any(|v| v.id == *id);
        if !keep {
            commands.entity(*e).despawn();
        }
        keep
    });
    if shown.is_empty() {
        return;
    }
    let mesh = ents
        .mesh
        .get_or_insert_with(|| meshes.add(Cuboid::new(1.0, 1.0, 1.0)))
        .clone();
    let lamp_mesh = ents
        .lamp_mesh
        .get_or_insert_with(|| meshes.add(Cuboid::new(0.3, 0.15, 0.05)))
        .clone();
    let lamp_on = ents
        .lamp_on
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(1.0, 0.1, 0.05),
                emissive: LinearRgba::rgb(12.0, 0.6, 0.3),
                unlit: true,
                ..default()
            })
        })
        .clone();
    let lamp_off = ents
        .lamp_off
        .get_or_insert_with(|| {
            materials.add(StandardMaterial {
                base_color: Color::srgb(0.35, 0.04, 0.03),
                ..default()
            })
        })
        .clone();
    for v in shown {
        let size = class_size(v.class);
        let t = Transform {
            translation: to_vec3(v.pos) + Vec3::Y * size.y / 2.0,
            rotation: Quat::from_rotation_y(v.yaw()),
            scale: size,
        };
        let e = *ents.boxes.entry(v.id).or_insert_with(|| {
            let colour = ents
                .colours
                .entry(v.look % 64)
                .or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color: look_colour(v.look),
                        perceptual_roughness: 0.5,
                        ..default()
                    })
                })
                .clone();
            commands
                .spawn((Mesh3d(mesh.clone()), MeshMaterial3d(colour), t, VehicleBox))
                .with_children(|c| {
                    // The lamps at the back (+Z in the box's frame, unit
                    // cube: undo the scale for their own size).
                    for side in [-0.35_f32, 0.35] {
                        c.spawn((
                            Mesh3d(lamp_mesh.clone()),
                            MeshMaterial3d(lamp_off.clone()),
                            Transform {
                                translation: Vec3::new(side, 0.1, 0.51),
                                scale: Vec3::ONE / size,
                                ..default()
                            },
                            BrakeLamp,
                        ));
                    }
                })
                .id()
        });
        if let Ok(mut tf) = boxes.get_mut(e) {
            *tf = t;
        }
        if let Ok(kids) = children.get(e) {
            for k in kids.iter() {
                if let Ok(mut m) = lamps.get_mut(k) {
                    let want = if v.braking { &lamp_on } else { &lamp_off };
                    if m.0 != *want {
                        m.0 = want.clone();
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaw_follows_the_heading() {
        let v = |h: [f64; 2]| VehicleView {
            id: 1,
            class: VehicleClass::Car,
            look: 0,
            pos: [0.0; 3],
            heading: h,
            lateral: 0.0,
            lane: 0,
            speed: 0.0,
            braking: false,
            indicator: Indicator::Off,
        };
        // Facing −Z: no turn; +X (east): a quarter turn clockwise seen
        // from above (−π/2 about +Y).
        assert!(v([0.0, -1.0]).yaw().abs() < 1e-6);
        let r = Quat::from_rotation_y(v([1.0, 0.0]).yaw()) * Vec3::NEG_Z;
        assert!((r - Vec3::X).length() < 1e-5, "{r}");
    }

    #[test]
    fn looks_vary_and_sizes_fit_the_classes() {
        let colours: std::collections::BTreeSet<String> = (0..64u32)
            .map(|l| format!("{:?}", look_colour(l.wrapping_mul(7919))))
            .collect();
        assert!(colours.len() >= 6, "{}", colours.len());
        assert!(class_size(VehicleClass::Lorry).z > class_size(VehicleClass::Car).z * 2.0);
        assert!(class_size(VehicleClass::Motorbike).x < 1.0);
    }

    #[test]
    fn boxes_follow_the_view_behind_the_switch() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<StandardMaterial>()
            .insert_resource(RideView::new(ocw_world::Route::test_loop()))
            .add_plugins(TrafficDebugPlugin);
        app.insert_resource(TrafficDebug { boxes: true });
        let car = VehicleView {
            id: 7,
            class: VehicleClass::Car,
            look: 3,
            pos: [10.0, 2.0, -5.0],
            heading: [0.0, -1.0],
            lateral: 1.5,
            lane: 0,
            speed: 10.0,
            braking: true,
            indicator: Indicator::Off,
        };
        app.world_mut().resource_mut::<RideView>().vehicles = vec![car];
        app.update();
        let n = app
            .world_mut()
            .query::<&VehicleBox>()
            .iter(app.world())
            .count();
        assert_eq!(n, 1);
        let t = app
            .world_mut()
            .query_filtered::<&Transform, With<VehicleBox>>()
            .single(app.world())
            .unwrap();
        assert!((t.translation - Vec3::new(10.0, 2.75, -5.0)).length() < 1e-4);
        // Off: the boxes go.
        app.insert_resource(TrafficDebug { boxes: false });
        app.update();
        let n = app
            .world_mut()
            .query::<&VehicleBox>()
            .iter(app.world())
            .count();
        assert_eq!(n, 0);
    }
}

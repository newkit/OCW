//! Thumbnails of the bike stand's parts (spec §14 Phase 4, 4B *Tiles*):
//! small renderings of the real part models, made off screen – the bike
//! of the slot's profile with the part fitted, framed on the part from
//! the drive side, against a neutral background under a neutral light –
//! rendered **once** into an image and kept ([`Thumbnails`]: keyed by
//! profile, slot, the part's model and the paint, least recently used out
//! beyond [`CACHE_SIZE`]). The picker's tiles show the image at once; it
//! fills in when its camera has rendered ([`FRAMES_PER_THUMB`] frames, at
//! most [`AT_ONCE`] cameras at a time; then the camera and the bike are
//! despawned and only the image stays).
//!
//! The bookkeeping ([`ThumbCache`]) and the framing ([`framing`]) are pure
//! and tested without a GPU; [`ThumbnailPlugin`] spawns the cameras.

use std::collections::{HashMap, VecDeque};

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use ocw_assets::pose::bike_nodes;
use ocw_core::{ProfileId, Rgb};

use crate::gear::GearMaterial;
use crate::workshop::{BikeModels, BuiltBike};

/// The thumbnails' render layer (no other camera sees it).
pub const THUMB_LAYER: usize = 9;
/// The image size (2 : 1: two rows of tiles and the card fit the panel).
pub const THUMB_SIZE: UVec2 = UVec2::new(256, 128);
/// Frames a thumbnail camera renders before it is put away (the first
/// frames may still wait for meshes and pipelines).
pub const FRAMES_PER_THUMB: u32 = 40;
/// Thumbnail cameras at a time.
pub const AT_ONCE: usize = 6;
/// Thumbnails kept (the least recently used beyond go).
pub const CACHE_SIZE: usize = 160;
/// The neutral background.
pub const BACKGROUND: Color = Color::srgb(0.30, 0.31, 0.33);
/// The thumbnail camera's vertical field of view (rad).
pub const FOV_Y: f32 = 0.5;

/// What a thumbnail shows: a part's model in a slot of a bike of a
/// profile, in a frame paint.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ThumbKey {
    pub profile: ProfileId,
    /// One of `ocw_assets::bikes::HOTSPOT_SLOTS` (frames and paints: the
    /// whole bike).
    pub slot: String,
    /// The part's model (`ocw_assets::bikes` variant id, or "stock").
    pub model: String,
    pub paint: Rgb,
}

/// Where a thumbnail is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThumbState {
    /// Waiting for a camera.
    Queued,
    /// Its camera renders for this many more frames.
    Rendering(u32),
    /// Rendered: the image is kept, nothing more is drawn.
    Done,
}

#[derive(Debug, Clone)]
struct Entry<H> {
    handle: H,
    state: ThumbState,
    used: u64,
}

/// The thumbnails' bookkeeping (pure; `H` is the image handle).
#[derive(Debug, Clone)]
pub struct ThumbCache<H> {
    entries: HashMap<ThumbKey, Entry<H>>,
    queue: VecDeque<ThumbKey>,
    clock: u64,
    cap: usize,
    /// Renders started so far (each key once while it is kept).
    pub renders: usize,
}

impl<H: Clone> Default for ThumbCache<H> {
    fn default() -> Self {
        Self::new(CACHE_SIZE)
    }
}

impl<H: Clone> ThumbCache<H> {
    pub fn new(cap: usize) -> Self {
        Self {
            entries: HashMap::new(),
            queue: VecDeque::new(),
            clock: 0,
            cap: cap.max(1),
            renders: 0,
        }
    }

    /// The image of `key`: the kept one, or a new one from `make` queued
    /// for rendering.
    pub fn request(&mut self, key: &ThumbKey, make: impl FnOnce() -> H) -> H {
        self.clock += 1;
        if let Some(e) = self.entries.get_mut(key) {
            e.used = self.clock;
            return e.handle.clone();
        }
        let handle = make();
        self.entries.insert(
            key.clone(),
            Entry {
                handle: handle.clone(),
                state: ThumbState::Queued,
                used: self.clock,
            },
        );
        self.queue.push_back(key.clone());
        self.evict();
        handle
    }

    /// The state of `key` (`None`: not kept).
    pub fn state(&self, key: &ThumbKey) -> Option<ThumbState> {
        self.entries.get(key).map(|e| e.state)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The renders under way.
    pub fn rendering(&self) -> usize {
        self.entries
            .values()
            .filter(|e| matches!(e.state, ThumbState::Rendering(_)))
            .count()
    }

    /// Starts the queued renders while fewer than `at_once` run: their
    /// keys and images (newest requests first: the tiles on screen now).
    pub fn start(&mut self, at_once: usize) -> Vec<(ThumbKey, H)> {
        let mut out = Vec::new();
        while self.rendering() < at_once {
            let Some(key) = self.queue.pop_back() else {
                break;
            };
            let Some(e) = self.entries.get_mut(&key) else {
                continue;
            };
            if e.state != ThumbState::Queued {
                continue;
            }
            e.state = ThumbState::Rendering(FRAMES_PER_THUMB);
            self.renders += 1;
            out.push((key, e.handle.clone()));
        }
        out
    }

    /// One frame rendered: the renders that are done now (their cameras
    /// go).
    pub fn frame(&mut self) -> Vec<ThumbKey> {
        let mut done = Vec::new();
        for (k, e) in &mut self.entries {
            if let ThumbState::Rendering(n) = e.state {
                if n <= 1 {
                    e.state = ThumbState::Done;
                    done.push(k.clone());
                } else {
                    e.state = ThumbState::Rendering(n - 1);
                }
            }
        }
        done
    }

    /// Drops the least recently used finished or waiting thumbnails
    /// beyond the cap (never one being rendered).
    fn evict(&mut self) {
        while self.entries.len() > self.cap {
            let Some(old) = self
                .entries
                .iter()
                .filter(|(_, e)| !matches!(e.state, ThumbState::Rendering(_)))
                .min_by_key(|(_, e)| e.used)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.entries.remove(&old);
            self.queue.retain(|k| *k != old);
        }
    }
}

/// How far a slot's thumbnail looks around its anchor (m; frames and
/// paints show the whole bike).
pub fn slot_radius(slot: &str) -> f32 {
    match slot {
        "wheels" => 0.38,
        "tyres" => 0.17,
        "groupset" => 0.16,
        "crankset" => 0.2,
        "cassette" => 0.12,
        "chain" => 0.26,
        "cockpit" => 0.24,
        "bar_tape" => 0.15,
        "saddle" => 0.17,
        "seatpost" => 0.2,
        "pedals" => 0.13,
        "bottle_cages" => 0.17,
        "lights" => 0.17,
        "computer" => 0.11,
        "bag" => 0.17,
        _ => 0.0,
    }
}

/// Whether a slot's thumbnail is the whole bike.
pub fn whole_bike(slot: &str) -> bool {
    slot_radius(slot) == 0.0
}

/// The thumbnail camera for `slot` on `bike` (bike coordinates): (eye,
/// target). From the drive side (+X), a little ahead and above; the part
/// (its anchor and [`slot_radius`]) – or the whole bike – fills the view.
pub fn framing(bike: &BuiltBike, slot: &str) -> (Vec3, Vec3) {
    let m = &bike.model;
    let front = match (
        m.position(bike_nodes::WHEEL_FRONT),
        m.position(bike_nodes::WHEEL_REAR),
    ) {
        (Some(f), Some(r)) if (f - r).length() > 0.1 => (f - r).normalize(),
        _ => Vec3::NEG_Z,
    };
    let aspect = THUMB_SIZE.x as f32 / THUMB_SIZE.y as f32;
    let half = (FOV_Y * 0.5).tan();
    if whole_bike(slot) {
        let (lo, hi) = bike.bounds;
        let centre = (lo + hi) * 0.5;
        let size = hi - lo;
        let along = size.dot(front).abs().max(size.z.abs());
        let need = (size.y * 0.5).max(along * 0.5 / aspect) * 1.08;
        let dir = (Vec3::X + Vec3::Y * 0.12 + front * 0.06).normalize();
        return (centre + dir * (need / half), centre);
    }
    // The wheels: the front wheel whole; the other parts at their anchor.
    let at = if slot == "wheels" {
        m.position(bike_nodes::WHEEL_FRONT)
    } else {
        None
    }
    .or_else(|| {
        bike.anchors
            .iter()
            .find(|(s, _)| s == slot)
            .map(|(_, p)| *p)
    })
    .unwrap_or((bike.bounds.0 + bike.bounds.1) * 0.5);
    let r = slot_radius(slot);
    // The wheels straight from the side (the rim depth shows), the other
    // parts a little from ahead and above.
    let dir = if slot == "wheels" {
        (Vec3::X + Vec3::Y * 0.12).normalize()
    } else {
        (Vec3::X + front * 0.35 + Vec3::Y * 0.3).normalize()
    };
    (at + dir * (r / half), at)
}

/// The thumbnails of the session (requested by the picker's tiles).
#[derive(Resource, Default)]
pub struct Thumbnails {
    pub cache: ThumbCache<Handle<Image>>,
    /// The entities of each render under way (camera, bike) and its spot.
    jobs: HashMap<ThumbKey, (Vec<Entity>, usize)>,
    materials: HashMap<Vec<u32>, Handle<GearMaterial>>,
    light: Option<Entity>,
}

impl Thumbnails {
    /// The image of `key` (kept, or new and queued).
    pub fn image(&mut self, key: &ThumbKey, images: &mut Assets<Image>) -> Handle<Image> {
        self.cache.request(key, || {
            images.add(Image::new_target_texture(
                THUMB_SIZE.x,
                THUMB_SIZE.y,
                TextureFormat::Bgra8UnormSrgb,
                None,
            ))
        })
    }
}

/// Renders the thumbnails ([`Thumbnails`]); added by the workshop.
pub struct ThumbnailPlugin;

impl Plugin for ThumbnailPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Thumbnails>()
            .add_systems(PostUpdate, render_thumbs);
    }
}

/// Where render spot `k` puts its bike (far from the workshop and from
/// each other: no thumbnail sees another's bike).
fn spot(k: usize) -> Vec3 {
    Vec3::new(4000.0 + 60.0 * k as f32, -2000.0, 0.0)
}

fn layer() -> RenderLayers {
    RenderLayers::layer(THUMB_LAYER)
}

fn render_thumbs(
    mut commands: Commands,
    mut thumbs: ResMut<Thumbnails>,
    models: Res<BikeModels>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<GearMaterial>>,
) {
    let thumbs = &mut *thumbs;
    for key in thumbs.cache.frame() {
        if let Some((ents, _)) = thumbs.jobs.remove(&key) {
            for e in ents {
                if let Ok(mut c) = commands.get_entity(e) {
                    c.despawn();
                }
            }
        }
    }
    let started = thumbs.cache.start(AT_ONCE);
    if started.is_empty() {
        return;
    }
    if thumbs.light.is_none() {
        // A neutral key and fill light, on the thumbnails' layer only.
        let key = commands
            .spawn((
                DirectionalLight {
                    illuminance: 1100.0,
                    shadow_maps_enabled: false,
                    color: Color::WHITE,
                    ..default()
                },
                Transform::from_xyz(1.0, 1.4, 0.6).looking_at(Vec3::ZERO, Vec3::Y),
                layer(),
            ))
            .id();
        commands.spawn((
            DirectionalLight {
                illuminance: 380.0,
                shadow_maps_enabled: false,
                color: Color::srgb(0.92, 0.95, 1.0),
                ..default()
            },
            Transform::from_xyz(-0.6, 0.5, -1.0).looking_at(Vec3::ZERO, Vec3::Y),
            ChildOf(key),
            layer(),
        ));
        thumbs.light = Some(key);
    }
    for (key, image) in started {
        let used: Vec<usize> = thumbs.jobs.values().map(|(_, s)| *s).collect();
        let k = (0..AT_ONCE * 2).find(|s| !used.contains(s)).unwrap_or(0);
        let at = spot(k);
        let variants = if key.model == "stock" {
            Vec::new()
        } else {
            vec![key.model.clone()]
        };
        let built = models.get(key.profile, &variants);
        let (eye, target) = framing(&built, &key.slot);
        let camera = commands
            .spawn((
                Camera3d::default(),
                Camera {
                    order: -20 - k as isize,
                    clear_color: ClearColorConfig::Custom(BACKGROUND),
                    ..default()
                },
                RenderTarget::Image(image.into()),
                Projection::Perspective(PerspectiveProjection {
                    fov: FOV_Y,
                    near: 0.02,
                    ..default()
                }),
                Transform::from_translation(at + eye).looking_at(at + target, Vec3::Y),
                AmbientLight {
                    color: Color::WHITE,
                    brightness: 320.0,
                    ..default()
                },
                bevy::camera::Exposure { ev100: 7.5 },
                layer(),
            ))
            .id();
        let bike = spawn_bike(
            &mut commands,
            &built,
            key.paint,
            at,
            &mut meshes,
            &mut materials,
            &mut thumbs.materials,
        );
        thumbs.jobs.insert(key, (vec![camera, bike], k));
    }
}

/// The bike's near model at `at`, on the thumbnails' layer.
fn spawn_bike(
    commands: &mut Commands,
    built: &BuiltBike,
    paint: Rgb,
    at: Vec3,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<GearMaterial>,
    cache: &mut HashMap<Vec<u32>, Handle<GearMaterial>>,
) -> Entity {
    let model = &built.model;
    let handles = crate::workshop::upload(model, meshes, false);
    let colours = crate::workshop::bike_colours(paint);
    let root = commands
        .spawn((
            Transform::from_translation(at),
            Visibility::default(),
            layer(),
        ))
        .id();
    let far = model.node(bike_nodes::LOD1);
    let mut ents: Vec<Entity> = Vec::new();
    for (n, node) in model.nodes.iter().enumerate() {
        let parent = node.parent.map_or(root, |p| ents[p]);
        let e = commands
            .spawn((
                Transform {
                    translation: node.translation,
                    rotation: node.rotation,
                    scale: node.scale,
                },
                if far.is_some_and(|f| model.is_below(n, f)) {
                    Visibility::Hidden
                } else {
                    Visibility::Inherited
                },
                ChildOf(parent),
                layer(),
            ))
            .id();
        for (mesh, mat) in &handles[n] {
            let m = &model.materials[*mat];
            let material = crate::workshop::part_material(
                cache, materials, &m.name, &node.name, m.color, &colours,
            );
            commands.spawn((
                Mesh3d(mesh.clone()),
                MeshMaterial3d(material),
                ChildOf(e),
                layer(),
            ));
        }
        ents.push(e);
    }
    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(model: &str, paint: u32) -> ThumbKey {
        ThumbKey {
            profile: ProfileId::Road,
            slot: "wheels".into(),
            model: model.into(),
            paint: Rgb(paint),
        }
    }

    /// Render once, reuse: a second request of the same part and paint
    /// gets the same image and no new render; another paint or part is
    /// its own thumbnail.
    #[test]
    fn a_thumbnail_is_rendered_once_and_reused() {
        let mut c: ThumbCache<u32> = ThumbCache::new(16);
        let mut next = 0u32;
        let mut make = || {
            next += 1;
            next
        };
        let a = c.request(&key("wheel-carbon-50", 0xff0000), &mut make);
        assert_eq!(
            c.state(&key("wheel-carbon-50", 0xff0000)),
            Some(ThumbState::Queued)
        );
        assert_eq!(c.start(AT_ONCE).len(), 1);
        assert_eq!(c.renders, 1);
        for _ in 0..FRAMES_PER_THUMB - 1 {
            assert!(c.frame().is_empty());
        }
        assert_eq!(c.frame(), vec![key("wheel-carbon-50", 0xff0000)]);
        assert_eq!(
            c.state(&key("wheel-carbon-50", 0xff0000)),
            Some(ThumbState::Done)
        );
        // Again: the same image, nothing to render.
        assert_eq!(c.request(&key("wheel-carbon-50", 0xff0000), &mut make), a);
        assert!(c.start(AT_ONCE).is_empty());
        assert_eq!(c.renders, 1);
        // Keyed by part and paint.
        let b = c.request(&key("wheel-carbon-50", 0x00ff00), &mut make);
        let d = c.request(&key("wheel-carbon-45", 0xff0000), &mut make);
        assert!(a != b && b != d && a != d);
        assert_eq!(c.start(AT_ONCE).len(), 2);
        assert_eq!(c.renders, 3);
        assert_eq!(c.len(), 3);
    }

    /// At most `AT_ONCE` cameras; the rest wait their turn, the newest
    /// request first.
    #[test]
    fn renders_wait_their_turn() {
        let mut c: ThumbCache<usize> = ThumbCache::new(64);
        let n = 2 * AT_ONCE + 2;
        for i in 0..n {
            c.request(&key(&format!("m{i}"), 1), || i);
        }
        let first = c.start(AT_ONCE);
        assert_eq!(first.len(), AT_ONCE);
        assert_eq!(first[0].0, key(&format!("m{}", n - 1), 1));
        assert!(c.start(AT_ONCE).is_empty());
        for _ in 0..FRAMES_PER_THUMB {
            c.frame();
        }
        assert_eq!(c.rendering(), 0);
        assert_eq!(c.start(AT_ONCE).len(), AT_ONCE);
        for _ in 0..FRAMES_PER_THUMB {
            c.frame();
        }
        assert_eq!(c.start(AT_ONCE).len(), 2);
        assert_eq!(c.renders, n);
    }

    /// Beyond the cap the least recently used thumbnail goes (a used one
    /// stays), never one being rendered.
    #[test]
    fn the_least_recently_used_go_first() {
        let mut c: ThumbCache<usize> = ThumbCache::new(3);
        c.request(&key("a", 1), || 1);
        c.request(&key("b", 1), || 2);
        c.request(&key("c", 1), || 3);
        c.start(1); // "c" renders
        c.request(&key("a", 1), || 99); // "a" used again
        c.request(&key("d", 1), || 4);
        assert_eq!(c.len(), 3);
        assert_eq!(c.state(&key("b", 1)), None);
        assert!(c.state(&key("a", 1)).is_some());
        assert!(matches!(
            c.state(&key("c", 1)),
            Some(ThumbState::Rendering(_))
        ));
        // An evicted key is rendered again when it comes back.
        c.request(&key("b", 1), || 5);
        assert_eq!(c.state(&key("b", 1)), Some(ThumbState::Queued));
    }

    /// The framing: the part's anchor in the middle of the view, its
    /// radius inside it, from the drive side; frames and paints show the
    /// whole bike.
    #[test]
    fn each_slot_is_framed_on_its_part() {
        let models = BikeModels::default();
        let bike = models.get(ProfileId::Road, &[]);
        for slot in ocw_assets::bikes::HOTSPOT_SLOTS {
            let (eye, target) = framing(&bike, slot);
            let d = eye - target;
            assert!(d.x > 0.0, "{slot}: from the drive side");
            if whole_bike(slot) {
                // The whole box within the view.
                let (lo, hi) = bike.bounds;
                let half = (FOV_Y * 0.5).tan() * d.length();
                assert!((hi.y - lo.y) * 0.5 <= half, "{slot}");
            } else {
                let half = (FOV_Y * 0.5).tan() * d.length();
                assert!((half - slot_radius(slot)).abs() < 1e-4, "{slot}");
                if slot != "wheels" {
                    let anchor = bike.anchors.iter().find(|(s, _)| s == slot).unwrap().1;
                    assert!(target.distance(anchor) < 1e-5, "{slot}");
                }
            }
        }
        assert!(whole_bike("frame") && whole_bike("paint") && !whole_bike("saddle"));
    }
}

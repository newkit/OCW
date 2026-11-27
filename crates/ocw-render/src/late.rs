//! Late results of background builds (spec 3.2.1 *World switch and
//! streaming*): the ground detail, plant and tile meshes, the far terrain
//! and the neighbour loads finish on the task pool and are put into the
//! scene by commands, applied later in the frame. A ride or world switch
//! ([`crate::rebuild_scene`], an exclusive system) can despawn their chunk
//! or tile in between – a plain `insert` then panics (it did, at a world
//! switch after joining a server, 2026-11-27). These helpers drop such a
//! result instead, with a debug line: by the entity (gone) or by the
//! [`SceneGeneration`] (the scene of another world).

use bevy::ecs::error::{BevyError, ErrorContext};
use bevy::ecs::system::{EntityCommands, entity_command};
use bevy::ecs::world::EntityWorldMut;
use bevy::prelude::*;

/// Counts the scenes: [`crate::rebuild_scene`] adds one at each ride or
/// world switch. A background result queued for an older generation is
/// dropped ([`queue_if_current`], [`despawn_if_stale`]).
#[derive(Resource, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneGeneration(pub u64);

/// The current scene generation (0 before the first switch, or without the
/// resource in a bare test app).
pub fn generation(world: &World) -> u64 {
    world.get_resource::<SceneGeneration>().map_or(0, |g| g.0)
}

/// The generation in a system's `Option<Res<SceneGeneration>>`.
pub fn current(g: Option<&SceneGeneration>) -> u64 {
    g.map_or(0, |g| g.0)
}

fn dropped(what: &'static str) -> impl FnOnce(BevyError, ErrorContext) + Send + 'static {
    move |err, ctx| debug!("{what}: dropped a late result, its entity is gone ({ctx}: {err})")
}

/// Inserts, removes and edits for the results of background builds: done
/// only while the entity is still there, else dropped with a debug line
/// (never a panic).
pub trait LateCommands {
    /// `insert`, or nothing if the entity is gone.
    fn late_insert(&mut self, what: &'static str, bundle: impl Bundle) -> &mut Self;
    /// Runs `f` on the entity, or nothing if it is gone.
    fn late(
        &mut self,
        what: &'static str,
        f: impl FnOnce(EntityWorldMut) + Send + 'static,
    ) -> &mut Self;
}

impl LateCommands for EntityCommands<'_> {
    fn late_insert(&mut self, what: &'static str, bundle: impl Bundle) -> &mut Self {
        self.queue_handled(
            entity_command::insert(bundle, bevy::ecs::bundle::InsertMode::Replace),
            dropped(what),
        )
    }

    fn late(
        &mut self,
        what: &'static str,
        f: impl FnOnce(EntityWorldMut) + Send + 'static,
    ) -> &mut Self {
        self.queue_handled(f, dropped(what))
    }
}

/// Runs `f` when the commands are applied – unless the scene was rebuilt
/// (another world) after generation `then` was read.
pub fn queue_if_current(
    commands: &mut Commands,
    then: u64,
    what: &'static str,
    f: impl FnOnce(&mut World) + Send + 'static,
) {
    commands.queue(move |world: &mut World| {
        let now = generation(world);
        if now == then {
            f(world);
        } else {
            debug!("{what}: dropped a result of scene {then} (now {now})");
        }
    });
}

/// After commands that spawned `root` (and its children) for scene
/// generation `then`: despawns it again if the scene was rebuilt in between,
/// so nothing of the old world is left in the new one.
pub fn despawn_if_stale(commands: &mut Commands, then: u64, what: &'static str, root: Entity) {
    commands.queue(move |world: &mut World| {
        let now = generation(world);
        if now != then
            && let Ok(e) = world.get_entity_mut(root)
        {
            debug!("{what}: dropped {root} built for scene {then} (now {now})");
            e.despawn();
        }
    });
}

//! Collisions in the desktop app (spec §14 Phase 12): the settings
//! `ride.collisions`, `ride.chaotic_bots` (rules the server applies; the
//! host decides in shared rides) and `ride.proximity_cue` (a client-side
//! aid), their console commands, the staged contacts `dev.bump` /
//! `dev.crash` (local server only) and `ride.crashes`, the crash state.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

use crate::client::ClientCore;
use crate::connection::{ClientState, Link};

/// The rider's collision settings, kept for the next session (sent to
/// its server after the hello when they differ from the defaults).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionSettings {
    pub collisions: bool,
    pub chaotic_bots: bool,
    pub proximity_cue: bool,
}

impl Default for CollisionSettings {
    fn default() -> Self {
        // Phase 12 Q1: collisions on; Q3: chaotic bots off; 12D: no aid.
        Self {
            collisions: true,
            chaotic_bots: false,
            proximity_cue: false,
        }
    }
}

impl CollisionSettings {
    /// The rules request for a new session's server, if the rules differ
    /// from the servers' defaults.
    pub fn rules_message(&self) -> Option<Vec<u8>> {
        let d = Self::default();
        (self.collisions != d.collisions || self.chaotic_bots != d.chaotic_bots)
            .then(|| ClientCore::rules_message(Some(self.collisions), Some(self.chaotic_bots)))
    }
}

fn on_off(m: &clap::ArgMatches) -> bool {
    m.get_one::<String>("on").expect("required") == "on"
}

fn on_off_arg() -> Arg {
    Arg::new("on").required(true).value_parser(["on", "off"])
}

pub fn register_commands(app: &mut App) {
    app.init_resource::<CollisionSettings>()
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.collisions")
                .about("Rider collisions on (soft bumps and crashes; group rides soft bumps only) or off; the host decides in shared rides")
                .arg(on_off_arg()),
            |world, m| {
                let on = on_off(m);
                world.resource_mut::<CollisionSettings>().collisions = on;
                rules(world, Some(on), None)
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.chaotic_bots")
                .about("Bots that sometimes make mistakes (late reactions, wheel touches) on or off")
                .arg(on_off_arg()),
            |world, m| {
                let on = on_off(m);
                world.resource_mut::<CollisionSettings>().chaotic_bots = on;
                rules(world, None, Some(on))
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.proximity_cue")
                .about("A soft sound and a HUD arrow when someone is close beside or in front (never steers)")
                .arg(on_off_arg()),
            |world, m| {
                let on = on_off(m);
                world.resource_mut::<CollisionSettings>().proximity_cue = on;
                if let Some(mut c) = world.get_resource_mut::<ClientState>() {
                    c.0.set_proximity_cue(on);
                }
                Ok(format!("proximity cue {}", if on { "on" } else { "off" }))
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.crashes")
                .about("The collision rules in force and the riders down after a crash"),
            |world, _| {
                let now = world.resource::<Time>().elapsed_secs_f64();
                let c = &world.resource::<ClientState>().0;
                let mut s = format!(
                    "collisions: {}{}",
                    c.collision_mode().label(),
                    if c.chaotic_bots() { ", chaotic bots" } else { "" }
                );
                if let Some(n) = c.crash_notice(now) {
                    s.push_str(&format!("\n{n}"));
                }
                for line in c.crash_feed(now) {
                    s.push_str(&format!("\n{line}"));
                }
                Ok(s)
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::Dev,
            Command::new("dev.bump")
                .about("Stage a soft bump: the nearest other rider bumps into you from the side"),
            |world, _| stage(world, false),
        ))
        .console_command(ConsoleCommand::new(
            Permission::Dev,
            Command::new("dev.crash")
                .about("Stage a crash: the nearest other rider cuts in front of you (a wheel touch)"),
            |world, _| stage(world, true),
        ))
        .console_command(ConsoleCommand::new(
            Permission::Dev,
            Command::new("dev.tree").about(
                "Stage a crash into a tree (wild mode on first): you are put 8 m before the nearest tree off the road ahead",
            ),
            |world, _| {
                let link = world.get_resource::<Link>().ok_or("not riding")?;
                link.send_bytes(ClientCore::stage_tree_message());
                Ok(String::new())
            },
        ));
}

/// Sends the rules to the server (it answers; the change shows in the
/// next snapshot).
fn rules(
    world: &mut World,
    collisions: Option<bool>,
    chaotic: Option<bool>,
) -> Result<String, String> {
    let Some(link) = world.get_resource::<Link>() else {
        return Ok("applies to the next ride".into());
    };
    let c = &world.resource::<ClientState>().0;
    if c.rider_id().is_some() && !c.may_change_rules() {
        return Err("only the host may change the collision rules in this ride".into());
    }
    link.send_bytes(ClientCore::rules_message(collisions, chaotic));
    // The server confirms.
    Ok(String::new())
}

fn stage(world: &mut World, crash: bool) -> Result<String, String> {
    let link = world.get_resource::<Link>().ok_or("not riding")?;
    link.send_bytes(ClientCore::stage_message(crash));
    Ok(String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_changed_rules_are_sent_to_a_new_session() {
        assert_eq!(CollisionSettings::default().rules_message(), None);
        let off = CollisionSettings {
            collisions: false,
            ..Default::default()
        };
        assert!(off.rules_message().is_some());
        let aid = CollisionSettings {
            proximity_cue: true,
            ..Default::default()
        };
        assert_eq!(aid.rules_message(), None, "a client-side aid");
    }
}

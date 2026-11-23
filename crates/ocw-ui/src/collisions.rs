//! Collisions in both apps (spec §14 Phase 12; shared since Phase 10 B6):
//! the settings
//! `ride.collisions`, `ride.chaotic_bots` (rules the server applies; the
//! host decides in shared rides), `ride.proximity_cue` and
//! `ride.hit_warning` (client-side aids, spec §14 Phase 12D), their console
//! commands, the staged contacts `dev.bump` /
//! `dev.crash` (local server only) and `ride.crashes`, the crash state;
//! damage (spec §14 Phase 12E): `ride.damage` (off by default, Q1),
//! `ride.repair` (mend a puncture now) and `dev.damage` (staged damage,
//! local server only); the HUD's damage widget shows while damage is on
//! (no switch of its own since 2026-11-19).

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_client::client::ClientCore;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

/// The rider's collision settings, kept for the next session (sent to
/// its server after the hello when they differ from the defaults).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CollisionSettings {
    pub collisions: bool,
    pub chaotic_bots: bool,
    pub proximity_cue: bool,
    /// The "last moment" warning (12D; on, proposal v3.22).
    pub hit_warning: bool,
    /// `ride.damage` (12E; Q1: off).
    pub damage: bool,
}

impl Default for CollisionSettings {
    fn default() -> Self {
        // Phase 12 Q1: collisions on; Q3: chaotic bots off; 12D: no aid.
        Self {
            collisions: true,
            chaotic_bots: false,
            proximity_cue: false,
            hit_warning: true,
            damage: false,
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

    /// `ride.damage` for a new session's server, if on (the servers start
    /// with it off, Q1).
    pub fn damage_message(&self) -> Option<Vec<u8>> {
        self.damage.then(|| ClientCore::damage_rules_message(true))
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
            Command::new("ride.damage")
                .about("Damage on or off (only with collisions on; the host decides in shared rides); without an argument: the own condition")
                .arg(Arg::new("on").value_parser(["on", "off"])),
            |world, m| {
                let Some(on) = m.get_one::<String>("on").map(|v| v == "on") else {
                    let c = crate::client(world).ok_or("not riding")?;
                    return Ok(c.damage_state().summary(c.rider_id()));
                };
                world.resource_mut::<CollisionSettings>().damage = on;
                let Some(c) = crate::client(world) else {
                    return Ok("applies to the next ride".into());
                };
                if c.rider_id().is_some() && !c.may_change_rules() {
                    return Err("only the host may change the damage rule in this ride".into());
                }
                crate::send(world, ClientCore::damage_rules_message(on));
                Ok(String::new())
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.repair")
                .about("Stop and mend a puncture now (30 s) instead of riding on slowly"),
            |world, _| {
                send_or_err(world, ClientCore::repair_message())
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::Dev,
            Command::new("dev.damage")
                .about("Stage damage: rider PCT, a part (front_wheel, rear_wheel, front_tyre, rear_tyre, drivetrain, cockpit, frame) PCT, puncture_front, puncture_rear, chain or reset")
                .arg(Arg::new("target").required(true))
                .arg(
                    Arg::new("percent")
                        .value_parser(clap::value_parser!(f64))
                        .help("The condition in percent (0-100)"),
                ),
            |world, m| {
                let target = m.get_one::<String>("target").expect("required").clone();
                let pct = m.get_one::<f64>("percent").copied().unwrap_or(100.0);
                send_or_err(world, ClientCore::stage_damage_message(&target, pct / 100.0))
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.proximity_cue")
                .about("A soft sound and a HUD arrow when someone is close beside, in front or behind (never steers)")
                .arg(on_off_arg()),
            |world, m| {
                let on = on_off(m);
                world.resource_mut::<CollisionSettings>().proximity_cue = on;
                if let Some(mut c) = crate::client_mut(world) {
                    c.set_proximity_cue(on);
                }
                Ok(format!("proximity cue {}", if on { "on" } else { "off" }))
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.hit_warning")
                .about("The \"last moment\" warning: an amber flash about a second before a predicted hit, and when the tyres slide (never steers)")
                .arg(on_off_arg()),
            |world, m| {
                let on = on_off(m);
                world.resource_mut::<CollisionSettings>().hit_warning = on;
                if let Some(mut c) = crate::client_mut(world) {
                    c.set_hit_warning(on);
                }
                Ok(format!("last-moment warning {}", if on { "on" } else { "off" }))
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("ride.crashes")
                .about("The collision rules in force and the riders down after a crash"),
            |world, _| {
                let now = crate::now(world);
                let c = crate::client(world).ok_or("not riding")?;
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
                send_or_err(world, ClientCore::stage_tree_message())
            },
        ))
        .console_command(ConsoleCommand::new(
            Permission::Dev,
            Command::new("dev.lamp").about(
                "Stage a bump into a street lamp's pole (wild mode on first): you are put 6 m before the nearest lamp beside the road ahead, riding slowly at it",
            ),
            |world, _| {
                send_or_err(world, ClientCore::stage_lamp_message())
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
    let Some(c) = crate::client(world) else {
        return Ok("applies to the next ride".into());
    };
    if c.rider_id().is_some() && !c.may_change_rules() {
        return Err("only the host may change the collision rules in this ride".into());
    }
    crate::send(world, ClientCore::rules_message(collisions, chaotic));
    // The server confirms.
    Ok(String::new())
}

fn stage(world: &mut World, crash: bool) -> Result<String, String> {
    send_or_err(world, ClientCore::stage_message(crash))
}

/// Sends `msg`; the server answers (printed when it arrives).
fn send_or_err(world: &mut World, msg: Vec<u8>) -> Result<String, String> {
    if crate::send(world, msg) {
        Ok(String::new())
    } else {
        Err("not riding".into())
    }
}

/// `display.camera_shake` (spec §14 Phase 12C): the chase camera's short
/// shake after a crash or a hard bump; on by default, off for motion
/// comfort (the kiosk's show mode turns it off).
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CameraShake(pub bool);

impl Default for CameraShake {
    fn default() -> Self {
        Self(true)
    }
}

/// `camera.shake [on|off]` (a toggle without an argument).
pub fn register_camera_shake(app: &mut App) {
    app.init_resource::<CameraShake>()
        .console_command(ConsoleCommand::new(
            Permission::User,
            Command::new("camera.shake")
                .about("The chase camera's short shake after a crash or a hard bump (off for motion comfort)")
                .arg(Arg::new("state").value_parser(["on", "off"])),
            |world, m| {
                let mut s = world.resource_mut::<CameraShake>();
                s.0 = match m.get_one::<String>("state").map(String::as_str) {
                    Some("on") => true,
                    Some("off") => false,
                    _ => !s.0,
                };
                Ok(format!("camera shake {}", if s.0 { "on" } else { "off" }))
            },
        ));
}

/// The collision and damage commands, the riding aids and `camera.shake`.
pub struct CollisionsPlugin;

impl Plugin for CollisionsPlugin {
    fn build(&self, app: &mut App) {
        register_commands(app);
        if !app.world().contains_resource::<CameraShake>() {
            register_camera_shake(app);
        }
    }
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
        assert_eq!(
            CollisionSettings::default().damage_message(),
            None,
            "Q1: off"
        );
        let damage = CollisionSettings {
            damage: true,
            ..Default::default()
        };
        assert!(damage.damage_message().is_some());
    }
}

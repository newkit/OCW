//! The accessibility options of both apps (spec §3.2.1 M9): the console
//! commands the settings apply – `ui.scale PERCENT` (`display.ui_scale`:
//! the HUD's size, Bevy's `UiScale`; the menu is sized by the window
//! height and stays), `ui.palette standard|colour-safe`
//! (`display.palette`), `ui.contrast on|off` (`display.high_contrast`),
//! `ui.motion on|off` (`display.reduced_motion`: no camera shake, no
//! flashing warnings) – into `ocw_render::access::Accessibility`.

use bevy::prelude::*;
use clap::{Arg, Command};
use ocw_render::access::{Accessibility, Palette};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};

pub struct AccessPlugin;

impl Plugin for AccessPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Accessibility>()
            .init_resource::<UiScale>();
        register_commands(app);
    }
}

/// Whether the camera shakes: the setting, unless reduced motion is on.
pub fn shake_on(world: &World) -> bool {
    let shake = world
        .get_resource::<crate::collisions::CameraShake>()
        .is_none_or(|s| s.0);
    shake && !reduced_motion(world)
}

/// Reduced motion is on.
pub fn reduced_motion(world: &World) -> bool {
    world
        .get_resource::<Accessibility>()
        .is_some_and(|a| a.reduced_motion)
}

fn switch(m: &clap::ArgMatches, now: bool) -> Result<bool, String> {
    match m.get_one::<String>("value").map(String::as_str) {
        None | Some("toggle") => Ok(!now),
        Some("on" | "true" | "1") => Ok(true),
        Some("off" | "false" | "0") => Ok(false),
        Some(other) => Err(format!("expected on or off, not {other:?}")),
    }
}

fn on_off(b: bool) -> &'static str {
    if b { "on" } else { "off" }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ui.scale")
            .about("Size of the HUD in percent (100-200; setting display.ui_scale)")
            .arg(Arg::new("percent").required(true)),
        |world, m| {
            let p: f32 = m
                .get_one::<String>("percent")
                .expect("required")
                .parse()
                .map_err(|_| "expected a percentage".to_string())?;
            if !(50.0..=300.0).contains(&p) {
                return Err("expected 50 to 300 %".into());
            }
            // The setting; the applied `UiScale` is it times the screen's
            // fit (`ocw_render::hud_fit`: 1 on desktops, less on phones).
            let fit = world
                .get_resource::<ocw_render::hud_fit::HudLayout>()
                .map_or(1.0, |l| l.fit);
            if let Some(mut s) = world.get_resource_mut::<ocw_render::hud_fit::HudSize>() {
                s.0 = p / 100.0;
            }
            world.resource_mut::<UiScale>().0 = p / 100.0 * fit;
            Ok(format!("HUD size {p:.0} %"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ui.palette")
            .about("Status colours: standard or colour-safe (setting display.palette)")
            .arg(Arg::new("palette").required(true)),
        |world, m| {
            let name = m.get_one::<String>("palette").expect("required");
            let p = Palette::parse(name)
                .ok_or_else(|| format!("expected standard or colour-safe, not {name:?}"))?;
            world.resource_mut::<Accessibility>().palette = p;
            Ok(format!("status colours: {}", p.name()))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ui.contrast")
            .about("High-contrast HUD on / off (setting display.high_contrast)")
            .arg(Arg::new("value")),
        |world, m| {
            let mut a = world.resource_mut::<Accessibility>();
            a.high_contrast = switch(m, a.high_contrast)?;
            Ok(format!("high-contrast HUD {}", on_off(a.high_contrast)))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("ui.motion")
            .about("Reduced motion on / off: no camera shake, no flashing warnings (setting display.reduced_motion)")
            .arg(Arg::new("value")),
        |world, m| {
            let mut a = world.resource_mut::<Accessibility>();
            a.reduced_motion = switch(m, a.reduced_motion)?;
            Ok(format!("reduced motion {}", on_off(a.reduced_motion)))
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState};

    #[test]
    fn the_settings_commands_set_the_options() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .add_plugins((crate::settings::SettingsPlugin, AccessPlugin));
        app.update();
        for line in [
            "set display.ui_scale 150",
            "set display.palette colour-safe",
            "set display.high_contrast on",
            "set display.reduced_motion on",
        ] {
            app.world_mut().resource_mut::<ConsoleState>().queue(line);
            app.update();
            app.update();
        }
        assert_eq!(app.world().resource::<UiScale>().0, 1.5);
        assert_eq!(
            *app.world().resource::<Accessibility>(),
            Accessibility {
                palette: Palette::ColourSafe,
                high_contrast: true,
                reduced_motion: true,
            }
        );
        assert!(!shake_on(app.world()), "reduced motion: no shake");
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("ui.motion off");
        app.update();
        assert!(shake_on(app.world()));
    }
}

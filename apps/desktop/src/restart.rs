//! R (and the console command `reset`): start the whole app again, as at
//! program start, with the same command line: a new session, a new start
//! point, devices reconnected, camera and modes back to their defaults.
//!
//! The app exits normally first (the ride is saved, devices are shut down,
//! see `recording` and `devices`), then `main` replaces the process with a
//! fresh one of the same executable (on Unix the process id stays, so the
//! kiosk keeps watching it).

use std::sync::atomic::{AtomicBool, Ordering};

use bevy::prelude::*;
use clap::Command;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, Permission};
use ocw_render::menu::ride_input;

static REQUESTED: AtomicBool = AtomicBool::new(false);

pub struct RestartPlugin;

impl Plugin for RestartPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, reset_key.run_if(ride_input))
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("reset").about(
                    "Start everything again as at program start (key R); the ride is saved first",
                ),
                |world, _| {
                    // After an in-app ride switch the command line would
                    // start the first ride again: restart this one in the app.
                    if switched(world) {
                        return crate::ride::begin_switch(
                            world,
                            world.resource::<crate::ride::CurrentRide>().choice.clone(),
                        );
                    }
                    REQUESTED.store(true, Ordering::SeqCst);
                    world.write_message(AppExit::Success);
                    Ok("restarting".into())
                },
            ));
    }
}

/// Whether the ride was switched in the app (R then restarts it in the app).
fn switched(world: &World) -> bool {
    world
        .get_resource::<crate::ride::CurrentRide>()
        .is_some_and(|c| c.switched)
}

fn reset_key(
    keys: Res<ButtonInput<KeyCode>>,
    current: Option<Res<crate::ride::CurrentRide>>,
    console: Option<ResMut<ocw_render::console::ConsoleState>>,
    mut exit: MessageWriter<AppExit>,
) {
    if !keys.just_pressed(KeyCode::KeyR) {
        return;
    }
    if let (Some(c), Some(mut console)) = (current, console)
        && c.switched
    {
        info!("reset: restarting the ride in the app");
        console.queue("reset");
        return;
    }
    info!("reset: restarting the app");
    REQUESTED.store(true, Ordering::SeqCst);
    exit.write(AppExit::Success);
}

/// Whether the app exited to be started again.
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// The command that starts the app again: this executable with the same
/// arguments.
pub fn restart_command() -> std::io::Result<std::process::Command> {
    let mut c = std::process::Command::new(std::env::current_exe()?);
    c.args(std::env::args_os().skip(1));
    Ok(c)
}

/// Replaces this process with a fresh start of the app (Unix: `exec`, same
/// process id; elsewhere: start it and exit). Returns only on failure.
pub fn exec_restart() -> std::io::Error {
    exec_restart_with(&[])
}

/// [`exec_restart`] with extra environment variables (e.g. the graphics
/// adapter chosen by `surface`'s check).
pub fn exec_restart_with(envs: &[(&str, &str)]) -> std::io::Error {
    let mut c = match restart_command() {
        Ok(c) => c,
        Err(e) => return e,
    };
    c.envs(envs.iter().copied());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.exec()
    }
    #[cfg(not(unix))]
    {
        match c.spawn() {
            Ok(_) => std::process::exit(0),
            Err(e) => e,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restarts_the_same_executable_with_the_same_arguments() {
        let c = restart_command().unwrap();
        assert_eq!(
            c.get_program(),
            std::env::current_exe().unwrap().as_os_str()
        );
        let args: Vec<_> = c.get_args().collect();
        let expected: Vec<_> = std::env::args_os().skip(1).collect();
        assert_eq!(
            args,
            expected.iter().map(|a| a.as_os_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn r_requests_a_restart_through_the_normal_exit() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_message::<AppExit>()
            .init_resource::<ButtonInput<KeyCode>>()
            .add_systems(Update, reset_key.run_if(ride_input));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);
        app.update();
        assert!(requested());
        let exits = app.world().resource::<Messages<AppExit>>();
        assert_eq!(exits.len(), 1);
    }

    #[test]
    fn r_is_ignored_while_the_menu_or_the_console_is_open() {
        use ocw_render::menu::InputContext;
        for context in [InputContext::Menu, InputContext::Console] {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_message::<AppExit>()
                .init_resource::<ButtonInput<KeyCode>>()
                .insert_resource(context)
                .add_systems(Update, reset_key.run_if(ride_input));
            app.world_mut()
                .resource_mut::<ButtonInput<KeyCode>>()
                .press(KeyCode::KeyR);
            app.update();
            let exits = app.world().resource::<Messages<AppExit>>();
            assert!(exits.is_empty(), "{context:?}");
        }
    }

    #[test]
    fn after_a_ride_switch_r_restarts_the_ride_in_the_app() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ocw_render::console::ConsoleCorePlugin))
            .add_message::<AppExit>()
            .init_resource::<ButtonInput<KeyCode>>()
            .insert_resource(crate::ride::CurrentRide {
                choice: crate::ride::choice::RideChoice::TestRoute("loop".into()),
                world: None,
                switched: true,
            })
            .add_systems(Update, reset_key.run_if(ride_input));
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);
        app.update();
        // The console's `reset`, which then switches to the same ride.
        assert!(app.world().resource::<Messages<AppExit>>().is_empty());
        app.update();
        let console = app.world().resource::<ocw_render::console::ConsoleState>();
        assert!(
            console.log().iter().any(|l| l == "> reset"),
            "{:?}",
            console.log()
        );
    }
}

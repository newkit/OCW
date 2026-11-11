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
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};

static REQUESTED: AtomicBool = AtomicBool::new(false);

pub struct RestartPlugin;

impl Plugin for RestartPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, reset_key)
            .console_command(ConsoleCommand::new(
                Permission::User,
                Command::new("reset").about(
                    "Start everything again as at program start (key R); the ride is saved first",
                ),
                |world, _| {
                    REQUESTED.store(true, Ordering::SeqCst);
                    world.write_message(AppExit::Success);
                    Ok("restarting".into())
                },
            ));
    }
}

fn reset_key(
    keys: Res<ButtonInput<KeyCode>>,
    console: Res<ConsoleState>,
    mut exit: MessageWriter<AppExit>,
) {
    if !console.is_open() && keys.just_pressed(KeyCode::KeyR) {
        info!("reset: restarting the app");
        REQUESTED.store(true, Ordering::SeqCst);
        exit.write(AppExit::Success);
    }
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
    let mut c = match restart_command() {
        Ok(c) => c,
        Err(e) => return e,
    };
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
            .init_resource::<ConsoleState>()
            .add_systems(Update, reset_key);
        app.world_mut()
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::KeyR);
        app.update();
        assert!(requested());
        let exits = app.world().resource::<Messages<AppExit>>();
        assert_eq!(exits.len(), 1);
    }
}

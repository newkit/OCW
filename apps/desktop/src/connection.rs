//! Bevy glue between the threads (local server, trainer) and [`ClientCore`].

use std::sync::Mutex;
use std::time::Instant;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_core::RiderProfile;
use ocw_devices::{DeviceBackend, DeviceCommand, DeviceHandle, VirtualTrainer};
use ocw_protocol::v1::{self, client_message::Msg as C};
use ocw_protocol::{client_message, encode};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_server::local::{LocalConnection, LocalServer};
use ocw_world::Route;

use crate::Args;
use crate::client::{ClientCore, Notice};

pub struct ConnectionPlugin {
    pub route: Route,
}

impl Plugin for ConnectionPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        let server = LocalServer::spawn(self.route.clone(), args.bots)
            .expect("failed to start local server thread");
        let trainer = VirtualTrainer::new(args.power)
            .start()
            .expect("failed to start virtual trainer");
        let client = ClientCore::new(self.route.clone(), args.name, RiderProfile::default());
        server
            .to_server
            .send(client.hello())
            .expect("local server is running");
        app.insert_resource(Link {
            server: Mutex::new(server),
            device: Mutex::new(trainer),
            started: Instant::now(),
        })
        .insert_resource(ClientState(client))
        .add_systems(PreUpdate, (poll_device, poll_server).chain())
        .add_systems(FixedUpdate, predict)
        .add_systems(Update, update_view)
        .add_systems(Last, shutdown_on_exit);
        register_commands(app);
    }
}

/// Channels to the server and device threads (receivers are not `Sync`, hence the mutexes).
#[derive(Resource)]
pub struct Link {
    server: Mutex<LocalConnection>,
    device: Mutex<DeviceHandle>,
    started: Instant,
}

impl Link {
    fn now_s(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    fn send_bytes(&self, bytes: Vec<u8>) {
        let _ = self
            .server
            .lock()
            .expect("not poisoned")
            .to_server
            .send(bytes);
    }

    pub fn send(&self, msg: C) {
        self.send_bytes(encode(&client_message(msg)));
    }

    pub fn device_command(&self, cmd: DeviceCommand) {
        let _ = self.device.lock().expect("not poisoned").commands.send(cmd);
    }
}

#[derive(Resource)]
pub struct ClientState(pub ClientCore);

/// Reads trainer samples and forwards them to the server.
fn poll_device(link: Res<Link>, mut client: ResMut<ClientState>) {
    let events: Vec<_> = link
        .device
        .lock()
        .expect("not poisoned")
        .events
        .try_iter()
        .collect();
    for event in events {
        if let Some(sample) = client.0.on_device_event(event) {
            link.send_bytes(sample);
        }
    }
}

/// Handles server messages: welcome, snapshots, command results.
fn poll_server(
    link: Res<Link>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
) {
    let messages: Vec<Vec<u8>> = link
        .server
        .lock()
        .expect("not poisoned")
        .from_server
        .try_iter()
        .collect();
    let now = link.now_s();
    for bytes in messages {
        match client.0.on_server_message(&bytes, now) {
            Some(Notice::Joined { rider_id, route_id }) => {
                info!(rider = %rider_id, route = %route_id, "joined session");
            }
            Some(Notice::Rejected(reason)) => {
                error!(%reason, "server rejected us");
                console.print(format!("server rejected connection: {reason}"));
            }
            Some(Notice::CommandResult(text)) => console.print(text),
            None => {}
        }
    }
}

/// Client-side prediction of the local rider, at the fixed physics rate.
fn predict(mut client: ResMut<ClientState>) {
    client.0.predict_step();
}

/// Publishes what the renderer should draw.
fn update_view(link: Res<Link>, client: Res<ClientState>, mut view: ResMut<RideView>) {
    let now = link.now_s();
    view.elapsed_s = now;
    view.riders = client.0.view(now);
}

fn shutdown_on_exit(mut exit: MessageReader<AppExit>, link: Res<Link>) {
    if exit.read().next().is_some() {
        link.device_command(DeviceCommand::Shutdown);
    }
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("sim.power")
            .about("Set the virtual trainer's power in watts")
            .arg(
                Arg::new("watts")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let watts = *m.get_one::<f64>("watts").expect("required");
            if !(0.0..=2500.0).contains(&watts) {
                return Err("power must be between 0 and 2500 W".into());
            }
            world
                .resource::<Link>()
                .device_command(DeviceCommand::SetVirtualPower(watts));
            Ok(format!("virtual power {watts:.0} W"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("teleport")
            .about("Move the local rider to a distance on the route (km)")
            .arg(Arg::new("km").required(true).value_parser(value_parser!(f64))),
        |world, m| {
            let km = *m.get_one::<f64>("km").expect("required");
            world.resource::<Link>().send(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(km * 1000.0)),
            }));
            // The server confirms; the prediction snaps on the next snapshot.
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("net.stats").about("Show connection statistics"),
        |world, _| {
            let c = &world.resource::<ClientState>().0;
            let s = c.stats();
            Ok(format!(
                "server: local (in-process)\nrider: {}\nmessages in: {} ({} malformed)\nsnapshots: {} (last server tick {})\nsamples out: {}\nprediction error at last snapshot: {:.3} m",
                c.rider_id().map(|r| r.to_string()).unwrap_or("-".into()),
                s.messages_in,
                s.malformed,
                s.snapshots,
                s.last_snapshot_tick,
                s.samples_out,
                c.prediction_error()
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.state").about("Print the local rider's state"),
        |world, _| {
            let c = &world.resource::<ClientState>().0;
            let s = c.state();
            Ok(format!(
                "distance {:.1} m, speed {:.2} km/h, power {:.0} W, grade {:+.1} %",
                s.distance,
                s.speed * 3.6,
                c.power_w(),
                c.route().sample(s.distance).grade * 100.0
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use clap::Parser;
    use ocw_render::console::ConsoleCorePlugin;

    use super::*;

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(Args::parse_from([
                "ocw-desktop",
                "--bots",
                "0",
                "--power",
                "150",
            ]))
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins(ConnectionPlugin {
                route: Route::test_loop(),
            });
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().to_vec()
    }

    /// Updates the app until `done` holds or the timeout expires.
    fn update_until(app: &mut App, done: impl Fn(&ClientCore) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(3) {
            app.update();
            if done(&app.world().resource::<ClientState>().0) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    #[test]
    fn joins_local_server_and_publishes_view() {
        let mut app = app();
        assert!(update_until(&mut app, |c| c.stats().snapshots > 2));
        let view = app.world().resource::<RideView>();
        assert_eq!(view.riders.len(), 1);
        assert!(view.riders[0].is_local);
        assert!(view.local().is_some());
    }

    #[test]
    fn dev_commands_need_dev_permission() {
        let mut app = app();
        let log = run(&mut app, "sim.power 300");
        assert!(
            log.last().unwrap().contains("needs Dev permission"),
            "{log:?}"
        );
        let log = run(&mut app, "teleport 1");
        assert!(
            log.last().unwrap().contains("needs Dev permission"),
            "{log:?}"
        );
        // User commands work without Dev.
        assert!(
            run(&mut app, "net.stats")
                .iter()
                .any(|l| l.contains("snapshots"))
        );
        assert!(
            run(&mut app, "rider.state")
                .iter()
                .any(|l| l.starts_with("distance"))
        );
    }

    #[test]
    fn sim_power_and_teleport_reach_trainer_and_server() {
        let mut app = app();
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        assert!(update_until(&mut app, |c| c.rider_id().is_some()));
        assert!(
            run(&mut app, "sim.power 9000")
                .last()
                .unwrap()
                .contains("between 0 and 2500")
        );
        assert_eq!(
            run(&mut app, "sim.power 320").last().unwrap(),
            "virtual power 320 W"
        );
        assert!(update_until(&mut app, |c| (c.power_w() - 320.0).abs() < 1e-6));
        run(&mut app, "teleport 1.5");
        assert!(update_until(&mut app, |c| (c.state().distance - 1500.0)
            .abs()
            < 10.0));
        // The server's confirmation is printed to the console.
        assert!(
            app.world()
                .resource::<ConsoleState>()
                .log()
                .iter()
                .any(|l| l.contains("teleported to 1.500 km"))
        );
    }
}

//! Bevy glue between the local server thread and [`ClientCore`]. Devices
//! (trainers, sensors) are handled in `devices.rs`.

use std::sync::Mutex;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_core::RiderProfile;
use ocw_protocol::v1::{self, client_message::Msg as C};
use ocw_protocol::{client_message, encode};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_server::local::{LocalConnection, LocalServer, SteppedServer};
use ocw_world::Route;

use crate::Args;
use crate::client::{ClientCore, Notice};

pub struct ConnectionPlugin {
    pub route: Route,
}

impl Plugin for ConnectionPlugin {
    fn build(&self, app: &mut App) {
        let args = app.world().resource::<Args>().clone();
        let start_m = crate::start_point(&args, &self.route);
        let mut server = if args.capture.is_some() {
            // Frame capture: the server shares the client's fixed-step game time.
            ServerLink::Stepped(Box::new(SteppedServer::new(
                self.route.clone(),
                args.bots,
                start_m,
            )))
        } else {
            ServerLink::Threaded(
                LocalServer::spawn(self.route.clone(), args.bots, start_m)
                    .expect("failed to start local server thread"),
            )
        };
        let client = ClientCore::new(self.route.clone(), args.name, RiderProfile::default());
        server.send(client.hello());
        app.insert_resource(Link {
            server: Mutex::new(server),
        })
        .insert_resource(ClientState(client))
        .add_systems(PreUpdate, poll_server)
        .add_systems(FixedUpdate, (step_server, predict).chain())
        .add_systems(Update, (send_steer, update_view));
        register_commands(app);
    }
}

/// The local server: on its own real-time thread, or stepped by the app
/// (frame capture with a fixed time step per frame).
pub enum ServerLink {
    Threaded(LocalConnection),
    Stepped(Box<SteppedServer>),
}

impl ServerLink {
    fn send(&mut self, bytes: Vec<u8>) {
        match self {
            // A send error means the server thread ended; nothing to do then.
            Self::Threaded(c) => {
                let _ = c.to_server.send(bytes);
            }
            Self::Stepped(s) => s.send(&bytes),
        }
    }

    fn receive(&mut self) -> Vec<Vec<u8>> {
        match self {
            Self::Threaded(c) => c.from_server.try_iter().collect(),
            Self::Stepped(s) => s.drain(),
        }
    }
}

/// Link to the server (receivers are not `Sync`, hence the mutex).
#[derive(Resource)]
pub struct Link {
    server: Mutex<ServerLink>,
}

impl Link {
    pub fn send_bytes(&self, bytes: Vec<u8>) {
        self.server.lock().expect("not poisoned").send(bytes);
    }

    pub fn send(&self, msg: C) {
        self.send_bytes(encode(&client_message(msg)));
    }
}

#[derive(Resource)]
pub struct ClientState(pub ClientCore);

/// Ticks the stepped server (frame capture) at the fixed physics rate.
fn step_server(link: Res<Link>) {
    if let ServerLink::Stepped(s) = &mut *link.server.lock().expect("not poisoned") {
        s.tick();
    }
}

/// Handles server messages: welcome, snapshots, command results.
///
/// Times are game time (Bevy's virtual clock): equal to wall-clock time in
/// normal runs, a fixed step per frame during frame capture.
fn poll_server(
    link: Res<Link>,
    time: Res<Time>,
    mut client: ResMut<ClientState>,
    mut console: ResMut<ConsoleState>,
) {
    let messages = link.server.lock().expect("not poisoned").receive();
    let now = time.elapsed_secs_f64();
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
/// Sends the steering intent when the target changed.
fn send_steer(time: Res<Time>, link: Res<Link>, mut client: ResMut<ClientState>) {
    if let Some(msg) = client.0.steer_message(time.elapsed_secs_f64()) {
        link.send_bytes(msg);
    }
}

fn update_view(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    client: Res<ClientState>,
    mut view: ResMut<RideView>,
) {
    let now = time.elapsed_secs_f64();
    view.elapsed_s = now;
    // Interpolate between the last two 50 Hz prediction steps.
    view.riders = client.0.view(now, fixed.overstep_fraction_f64());
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
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
            let mode = match &*world.resource::<Link>().server.lock().expect("not poisoned") {
                ServerLink::Threaded(_) => "in-process thread",
                ServerLink::Stepped(_) => "stepped, frame capture",
            };
            let c = &world.resource::<ClientState>().0;
            let s = c.stats();
            Ok(format!(
                "server: local ({})\nrider: {}\nmessages in: {} ({} malformed)\nsnapshots: {} (last server tick {})\nsamples out: {}\nprediction error at last snapshot: {:.3} m",
                mode,
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
        Command::new("rider.steer")
            .about("Steer to a lateral position on the road in metres (positive = right; or use the arrow keys)")
            .arg(
                Arg::new("metres")
                    .required(true)
                    .allow_negative_numbers(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let target = *m.get_one::<f64>("metres").expect("required");
            let mut c = world.resource_mut::<ClientState>();
            c.0.steer_to(target);
            match c.0.lateral_target() {
                Some(t) => Ok(format!("steering to {t:+.1} m")),
                None => Err("not riding yet".into()),
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.lean")
            .about("Lean into a curve by DEGREES (positive = right, 0 = straighten up; like the arrow keys)")
            .arg(
                Arg::new("degrees")
                    .required(true)
                    .allow_negative_numbers(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let deg = *m.get_one::<f64>("degrees").expect("required");
            let mut c = world.resource_mut::<ClientState>();
            c.0.set_lean(deg.to_radians());
            match c.0.lean() {
                Some(l) => Ok(format!("leaning {:+.0}°", l.to_degrees())),
                None => Err("not riding yet".into()),
            }
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.autodrive")
            .about("Autodrive on (keeps to the middle of the road) or off (steer by leaning; key A)")
            .arg(Arg::new("on").required(true).value_parser(["on", "off"])),
        |world, m| {
            let on = m.get_one::<String>("on").expect("required") == "on";
            let mut c = world.resource_mut::<ClientState>();
            if c.0.rider_id().is_none() {
                return Err("not riding yet".into());
            }
            c.0.set_autodrive(on);
            Ok(format!("autodrive {}", if on { "on" } else { "off" }))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.wild")
            .about("Wild mode on (not bound to the road, ride anywhere) or off (key W)")
            .arg(Arg::new("on").required(true).value_parser(["on", "off"])),
        |world, m| {
            let on = m.get_one::<String>("on").expect("required") == "on";
            world.resource_mut::<ClientState>().0.set_wild(on);
            Ok(format!("wild mode {}", if on { "on" } else { "off" }))
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
    use std::time::{Duration, Instant};

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
                "--virtual",
            ]))
            .insert_resource(RideView::new(Route::test_loop()))
            .add_plugins((
                ConnectionPlugin {
                    route: Route::test_loop(),
                },
                crate::devices::DevicesPlugin,
            ));
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

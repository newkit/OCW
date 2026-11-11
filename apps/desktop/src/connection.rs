//! Connection to the (local) server, devices, prediction and interpolation.

use std::sync::Mutex;
use std::time::Instant;

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_core::{INTERPOLATION_DELAY_S, RiderId, RiderProfile};
use ocw_devices::{DeviceBackend, DeviceCommand, DeviceEvent, DeviceHandle, VirtualTrainer};
use ocw_protocol::v1::{self, client_message::Msg as C, server_message::Msg as S};
use ocw_protocol::{PROTOCOL_VERSION, client_message, decode_server, dequantize, encode};
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::{RideView, RiderView};
use ocw_server::local::{LocalConnection, LocalServer};
use ocw_sim::{Predictor, RiderState, SnapshotBuffer};
use ocw_world::Route;

use crate::Args;

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
        let hello = encode(&client_message(C::Hello(v1::Hello {
            protocol_version: PROTOCOL_VERSION,
            display_name: args.name.clone(),
            profile: Some(RiderProfile::default().into()),
        })));
        server
            .to_server
            .send(hello)
            .expect("local server is running");
        app.insert_resource(Link {
            server: Mutex::new(server),
            device: Mutex::new(trainer),
            started: Instant::now(),
        })
        .insert_resource(ClientState {
            route: self.route.clone(),
            rider_id: None,
            predictor: Predictor::new(RiderProfile::default(), RiderState::default()),
            others: SnapshotBuffer::default(),
            names: Default::default(),
            power_w: 0.0,
            cadence_rpm: None,
            seq: 0,
            stats: NetStats::default(),
        })
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

    pub fn send(&self, msg: C) {
        let bytes = encode(&client_message(msg));
        let _ = self
            .server
            .lock()
            .expect("not poisoned")
            .to_server
            .send(bytes);
    }

    pub fn device_command(&self, cmd: DeviceCommand) {
        let _ = self.device.lock().expect("not poisoned").commands.send(cmd);
    }
}

#[derive(Debug, Default, Clone)]
pub struct NetStats {
    pub messages_in: u64,
    pub snapshots: u64,
    pub samples_out: u64,
    pub last_snapshot_tick: u64,
}

#[derive(Resource)]
pub struct ClientState {
    route: Route,
    rider_id: Option<RiderId>,
    predictor: Predictor,
    others: SnapshotBuffer,
    names: std::collections::BTreeMap<RiderId, (String, f64)>,
    pub power_w: f64,
    cadence_rpm: Option<f64>,
    seq: u32,
    stats: NetStats,
}

/// Reads trainer samples and forwards them to the server.
fn poll_device(link: Res<Link>, mut client: ResMut<ClientState>) {
    let events: Vec<DeviceEvent> = link
        .device
        .lock()
        .expect("not poisoned")
        .events
        .try_iter()
        .collect();
    for event in events {
        if let DeviceEvent::Power {
            watts,
            cadence_rpm,
            timestamp,
        } = event
        {
            client.power_w = watts;
            client.cadence_rpm = cadence_rpm;
            client.seq += 1;
            client.stats.samples_out += 1;
            link.send(C::Sample(v1::SensorSample {
                seq: client.seq,
                client_time_us: timestamp.as_micros() as u64,
                power_w: Some(watts as f32),
                cadence_rpm: cadence_rpm.map(|c| c as f32),
                heart_rate_bpm: None,
            }));
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
        client.stats.messages_in += 1;
        let Ok(msg) = decode_server(&bytes) else {
            warn!("malformed server message");
            continue;
        };
        match msg {
            S::Welcome(w) => {
                info!(rider = w.rider_id, route = %w.route_id, "joined session");
                client.rider_id = Some(RiderId(w.rider_id));
            }
            S::Reject(r) => {
                error!(reason = %r.reason, "server rejected us");
                console.print(format!("server rejected connection: {}", r.reason));
            }
            S::DevResult(r) => {
                console.print(if r.ok {
                    r.message
                } else {
                    format!("error: {}", r.message)
                });
            }
            S::Snapshot(s) => {
                client.stats.snapshots += 1;
                client.stats.last_snapshot_tick = s.server_tick;
                let own = client.rider_id;
                let mut others = Vec::new();
                for u in &s.riders {
                    let (id, state) = dequantize(u);
                    client
                        .names
                        .insert(id, (u.display_name.clone(), f64::from(u.power_w)));
                    if Some(id) == own {
                        // In-process server: the snapshot is fresh (age ≈ 0).
                        let power = client.power_w;
                        let route = client.route.clone();
                        client.predictor.reconcile(&route, state, 0.0, power);
                    } else {
                        others.push((id, state));
                    }
                }
                client.others.push(now, others);
            }
        }
    }
}

/// Client-side prediction of the local rider, at the fixed physics rate.
fn predict(mut client: ResMut<ClientState>) {
    if client.rider_id.is_none() {
        return;
    }
    let power = client.power_w;
    let route = client.route.clone();
    client.predictor.step(&route, power);
}

/// Publishes what the renderer should draw.
fn update_view(link: Res<Link>, client: Res<ClientState>, mut view: ResMut<RideView>) {
    view.elapsed_s = link.now_s();
    view.riders.clear();
    let Some(own) = client.rider_id else { return };
    view.riders.push(RiderView {
        id: own,
        name: client
            .names
            .get(&own)
            .map(|n| n.0.clone())
            .unwrap_or_default(),
        state: client.predictor.state(),
        power_w: client.power_w,
        cadence_rpm: client.cadence_rpm,
        is_local: true,
    });
    for (id, state) in client.others.sample(link.now_s() - INTERPOLATION_DELAY_S) {
        let (name, power_w) = client.names.get(&id).cloned().unwrap_or_default();
        view.riders.push(RiderView {
            id,
            name,
            state,
            power_w,
            cadence_rpm: None,
            is_local: false,
        });
    }
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
            let c = world.resource::<ClientState>();
            Ok(format!(
                "server: local (in-process)\nrider: {}\nmessages in: {}\nsnapshots: {} (last server tick {})\nsamples out: {}\nprediction error at last snapshot: {:.3} m",
                c.rider_id.map(|r| r.to_string()).unwrap_or("-".into()),
                c.stats.messages_in,
                c.stats.snapshots,
                c.stats.last_snapshot_tick,
                c.stats.samples_out,
                c.predictor.last_error()
            ))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("rider.state").about("Print the local rider's state"),
        |world, _| {
            let c = world.resource::<ClientState>();
            let s = c.predictor.state();
            Ok(format!(
                "distance {:.1} m, speed {:.2} km/h, power {:.0} W, grade {:+.1} %",
                s.distance,
                s.speed * 3.6,
                c.power_w,
                c.route.sample(s.distance).grade * 100.0
            ))
        },
    ));
}

//! Bevy glue of the browser app (spec §14 Phase 10, B1): moves the frame
//! time into the [`WebSession`] (local server + virtual trainer), runs the
//! client prediction at 50 Hz in `FixedUpdate`, reads the keys and writes
//! the [`RideView`] the shared renderer (`ocw-render`) draws.

use bevy::prelude::*;
use clap::{Arg, Command, value_parser};
use ocw_client::client::Notice;
use ocw_client::input::{KeyThrottle, LINE_KEY_RATE_MPS, ThrottleKeys, lean_ramp, steer_direction};
use ocw_protocol::v1::{self, client_message::Msg as C};
use ocw_protocol::{client_message, encode};
use ocw_render::RideView;
use ocw_render::console::{ConsoleAppExt, ConsoleCommand, ConsoleState, Permission};
use ocw_render::menu::ride_input;
use ocw_sky::Zone;

use crate::session::{SessionConfig, WebSession};

/// The ride session (server, client, trainer) as a Bevy resource.
#[derive(Resource)]
pub struct Session(pub WebSession);

/// The time zone of the ride (the test routes: the default place).
#[derive(Resource)]
pub struct Place(pub Zone);

/// Seconds between the server's progress lines in the log (the headless
/// browser test reads them from the console).
pub const TICK_LOG_INTERVAL_S: f64 = 2.0;

pub struct WebSessionPlugin {
    pub config: SessionConfig,
}

impl Plugin for WebSessionPlugin {
    fn build(&self, app: &mut App) {
        let (zone, _) = ocw_client::sky::place_zone(None, None);
        app.insert_resource(Session(WebSession::new(self.config.clone())))
            .insert_resource(Place(zone))
            .add_systems(Startup, enable_dev_commands)
            .add_systems(PreUpdate, frame)
            .add_systems(FixedUpdate, predict)
            .add_systems(
                Update,
                (keys.run_if(ride_input), send_steer, update_view).chain(),
            );
        register_commands(app);
    }
}

/// The local server allows dev commands (teleport …), as on the desktop.
fn enable_dev_commands(mut console: ResMut<ConsoleState>) {
    console.permission = Permission::Dev;
}

/// The frame's time into the session; the server's notices to the console.
fn frame(
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut console: ResMut<ConsoleState>,
    mut next_log: Local<f64>,
) {
    let now = time.elapsed_secs_f64();
    for notice in session.0.frame(time.delta_secs_f64(), now) {
        match notice {
            Notice::Joined { rider_id, route_id } => {
                info!(rider = %rider_id, route = %route_id, "joined the local server");
            }
            Notice::Rejected(reason) => {
                error!(%reason, "server rejected us");
                console.print(format!("server rejected connection: {reason}"));
            }
            Notice::CommandResult(text) => console.print(text),
            Notice::God { message, .. } => console.print(message),
            Notice::Voice(_) | Notice::WorldChange(_) => {}
        }
    }
    if now >= *next_log {
        *next_log = now + TICK_LOG_INTERVAL_S;
        let s = &session.0;
        let state = s.client.state();
        info!(
            ticks = s.ticks,
            distance_m = format!("{:.1}", state.distance),
            speed_kmh = format!("{:.1}", ocw_core::mps_to_kmh(state.speed)),
            power_w = s.power_w(),
            "local server running"
        );
    }
}

/// Client-side prediction of the own rider at the server's tick rate.
fn predict(mut session: ResMut<Session>) {
    session.0.client.predict_step();
}

/// The desktop's ride keys (spec §3.2.2; the gamepad follows in B6): ↑
/// adds keyboard power, ↓ stops, Page Up / Down ±10 W, A autodrive,
/// W wild mode, ← / → the line (autodrive) or the lean.
fn keys(
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    mut session: ResMut<Session>,
    mut throttle: Local<KeyThrottle>,
    mut upright_pause: Local<f64>,
) {
    let s = &mut session.0;
    let dt = time.delta_secs_f64();
    let power_keys = ThrottleKeys {
        up_pressed: keys.just_pressed(KeyCode::ArrowUp),
        up_held: keys.pressed(KeyCode::ArrowUp),
        down_pressed: keys.just_pressed(KeyCode::ArrowDown),
        page_up: keys.just_pressed(KeyCode::PageUp),
        page_down: keys.just_pressed(KeyCode::PageDown),
    };
    s.throttle(&mut throttle, power_keys, dt, time.elapsed_secs_f64());
    if keys.just_pressed(KeyCode::KeyA) {
        let on = !s.client.autodrive();
        s.client.set_autodrive(on);
        info!("autodrive {}", if on { "on" } else { "off" });
    }
    if keys.just_pressed(KeyCode::KeyW) {
        let wild = !s.client.wild();
        s.client.set_wild(wild);
        info!("wild mode {}", if wild { "on" } else { "off" });
    }
    let dir = steer_direction(
        keys.pressed(KeyCode::ArrowLeft),
        keys.pressed(KeyCode::ArrowRight),
    );
    if s.client.autodrive() {
        if dir != 0.0 {
            s.client.steer_by(dir * LINE_KEY_RATE_MPS * dt);
        }
        return;
    }
    let lean = s.client.lean().unwrap_or(0.0);
    let next = lean_ramp(lean, dir, &mut upright_pause, dt);
    if dir != 0.0 && next != lean {
        s.client.set_lean(next);
    }
}

/// Sends the steering intent when it changed.
fn send_steer(time: Res<Time>, mut session: ResMut<Session>) {
    let s = &mut session.0;
    if let Some(msg) = s.client.steer_message(time.elapsed_secs_f64()) {
        s.send(&msg);
    }
    if let Some(msg) = s.client.turn_message() {
        s.send(&msg);
    }
}

/// The ride for the renderer: riders (interpolated between the 50 Hz
/// prediction steps), the event, the sky and the status line.
fn update_view(
    time: Res<Time>,
    fixed: Res<Time<Fixed>>,
    place: Res<Place>,
    mut session: ResMut<Session>,
    mut view: ResMut<RideView>,
) {
    let now = time.elapsed_secs_f64();
    let s = &mut session.0;
    view.elapsed_s = now;
    view.riders = s.client.view(now, fixed.overstep_fraction_f64());
    view.position = view
        .local()
        .and_then(|l| ocw_client::gps::position(s.client.track(), s.client.geo_origin(), &l.state));
    view.event = s.client.event_view(now);
    view.ghost = s.client.ghost_state(now);
    view.status = s.status_line();
    let position = view.position;
    view.sky = ocw_client::sky::sky_view(
        &mut s.client,
        now,
        &place.0,
        position,
        ocw_sky::light::DEFAULT_NIGHT_FLOOR,
    );
}

fn register_commands(app: &mut App) {
    app.console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("sim.power")
            .about("Set the virtual trainer's power (W)")
            .arg(
                Arg::new("watts")
                    .required(true)
                    .value_parser(value_parser!(f64)),
            ),
        |world, m| {
            let w = *m.get_one::<f64>("watts").expect("required");
            let set = world.resource_mut::<Session>().0.set_power(w);
            Ok(format!("virtual power {set:.0} W"))
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::Dev,
        Command::new("teleport")
            .about("Move the local rider to a distance on the route (km)")
            .arg(Arg::new("km").required(true).value_parser(value_parser!(f64))),
        |world, m| {
            let km = *m.get_one::<f64>("km").expect("required");
            let msg = encode(&client_message(C::Dev(v1::DevCommand {
                cmd: Some(v1::dev_command::Cmd::TeleportDistanceM(km * 1000.0)),
            })));
            world.resource_mut::<Session>().0.send(&msg);
            // The server confirms; the prediction snaps on the next snapshot.
            Ok(String::new())
        },
    ))
    .console_command(ConsoleCommand::new(
        Permission::User,
        Command::new("net.stats").about("Show connection statistics"),
        |world, _| {
            let s = &world.resource::<Session>().0;
            let st = s.client.stats();
            Ok(format!(
                "local (in the browser's frame loop): {} ticks, {} messages in, {} snapshots, {} samples out",
                s.ticks, st.messages_in, st.snapshots, st.samples_out
            ))
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_render::console::ConsoleCorePlugin;

    /// The glue without a window or GPU: the session runs from Bevy's
    /// frame time, the view gets the riders and the sky, and the console
    /// commands reach the session.
    #[test]
    fn the_plugin_rides_headless() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, bevy::input::InputPlugin, ConsoleCorePlugin))
            .insert_resource(Time::<Fixed>::from_hz(f64::from(ocw_core::SIM_TICK_HZ)))
            .insert_resource(RideView::new(ocw_world::Route::test_hills()))
            .add_plugins(WebSessionPlugin {
                config: SessionConfig::default(),
            });
        let start = std::time::Instant::now();
        while start.elapsed() < std::time::Duration::from_millis(800) {
            app.update();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let session = &app.world().resource::<Session>().0;
        assert!(session.ticks > 20, "{}", session.ticks);
        assert!(session.client.rider_id().is_some());
        let view = app.world().resource::<RideView>();
        assert_eq!(view.riders.len(), 1 + crate::session::DEFAULT_BOTS as usize);
        assert!(view.sky.is_some());
        assert!(view.status.starts_with("Virtual trainer 200 W"));
        app.world_mut()
            .resource_mut::<ConsoleState>()
            .queue("sim.power 260");
        app.update();
        app.update();
        assert_eq!(app.world().resource::<Session>().0.power_w(), 260.0);
    }
}

//! God mode – "Unicorn mode" (spec §14 Phase 13). Shared with the
//! browser app since Phase 13 G5 (v3.37): the glue, the input mapping, the
//! touch controls and the inspection are `ocw_ui::god`, over the desktop's
//! ride through `ocw_ui::RideAccess` (`crate::connection`); this module
//! keeps the desktop's names for them and the end-to-end test on the
//! desktop's local server.

pub use ocw_ui::god::*;

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
    use bevy::prelude::*;
    use clap::Parser;
    use ocw_render::RideView;
    use ocw_render::console::{ConsoleCorePlugin, ConsoleState, Permission};
    use ocw_world::Route;

    use super::state::Phase;
    use super::*;
    use crate::client::ClientCore;
    use crate::connection::ClientState;

    fn app() -> App {
        let argv = ["ocw-desktop", "--bots", "0", "--power", "150", "--virtual"];
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, ConsoleCorePlugin))
            .insert_resource(crate::Args::parse_from(argv))
            .insert_resource(RideView::new(Route::test_loop()))
            .insert_resource(crate::connection::Ghosts(None))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<ButtonInput<MouseButton>>()
            .init_resource::<AccumulatedMouseMotion>()
            .init_resource::<AccumulatedMouseScroll>()
            .init_resource::<ocw_render::camera::CameraObstacles>()
            .add_plugins((
                crate::connection::ConnectionPlugin {
                    route: Route::test_loop(),
                },
                crate::devices::DevicesPlugin,
                GodPlugin,
            ));
        app
    }

    fn run(app: &mut App, line: &str) -> Vec<String> {
        app.world_mut().resource_mut::<ConsoleState>().queue(line);
        app.update();
        app.world().resource::<ConsoleState>().log().to_vec()
    }

    fn update_until(app: &mut App, done: impl Fn(&ClientCore) -> bool) -> bool {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(4) {
            app.update();
            if done(&app.world().resource::<ClientState>().0) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        false
    }

    /// G1 end to end on the local server (spec §14 Phase 13F): enter, the
    /// view and the pointer, fly, land on the route, standing.
    #[test]
    fn god_toggles_flies_and_lands_on_the_local_server() {
        let mut app = app();
        // Dev commands (god.cruise): the local server's session.
        app.world_mut().resource_mut::<ConsoleState>().permission = Permission::Dev;
        let phase = |app: &App| app.world().resource::<ClientState>().0.god().phase();
        assert!(update_until(&mut app, |c| c.rider_id().is_some()));
        let out = run(&mut app, "god on");
        assert!(out.iter().all(|l| !l.starts_with("error")), "{out:?}");
        assert!(
            update_until(&mut app, |c| c.god().flying()),
            "{:?}",
            phase(&app)
        );
        app.update();
        assert!(
            app.world().resource::<FlightPointer>().captured,
            "pointer captured"
        );
        let view = app.world().resource::<RideView>();
        assert!(view.flight.is_some());
        assert_eq!(view.unicorns.len(), 1);
        assert!(view.unicorns[0].is_local);
        // The landing ring is on the route below.
        assert!(update_until(&mut app, |c| c.god().landing().is_some()));
        // Cruise forward (dev command on the local server), then land.
        let out = run(&mut app, "god.cruise 1");
        assert!(out.iter().any(|l| l.contains("cruising")), "{out:?}");
        let start = app.world().resource::<ClientState>().0.god().flight().pos;
        assert!(update_until(&mut app, |c| {
            let p = c.god().flight().pos;
            (p[0] - start[0]).hypot(p[2] - start[2]) > 20.0
        }));
        run(&mut app, "god.cruise 0");
        // Bug 2026-11-13: a German keyboard's + (the US `]` position, typing
        // "+") climbs, its − (the US `/`, typing "-") descends.
        let altitude = |app: &App| app.world().resource::<ClientState>().0.god().flight().pos[1];
        let german = |app: &mut App, code: KeyCode, typed: &str, down: bool| {
            app.world_mut().write_message(KeyboardInput {
                key_code: code,
                logical_key: bevy::input::keyboard::Key::Character(typed.into()),
                state: if down {
                    bevy::input::ButtonState::Pressed
                } else {
                    bevy::input::ButtonState::Released
                },
                text: None,
                repeat: false,
                window: Entity::PLACEHOLDER,
            });
            let mut keys = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            if down {
                keys.press(code);
            } else {
                keys.release(code);
            }
        };
        let y0 = altitude(&app);
        german(&mut app, KeyCode::BracketRight, "+", true);
        for _ in 0..80 {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        german(&mut app, KeyCode::BracketRight, "+", false);
        app.update();
        let y1 = altitude(&app);
        assert!(y1 > y0 + 2.0, "the German + climbs: {y0} → {y1}");
        german(&mut app, KeyCode::Slash, "-", true);
        for _ in 0..80 {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        german(&mut app, KeyCode::Slash, "-", false);
        app.update();
        let y2 = altitude(&app);
        assert!(y2 < y1 - 1.0, "the German - descends: {y1} → {y2}");
        // G3: the panel and the crosshair are on, a pin goes in and is
        // listed with a --start, I (god.inspect off) hides the panel.
        for _ in 0..30 {
            app.update();
            std::thread::sleep(Duration::from_millis(10));
        }
        let f = app.world().resource::<RideView>().flight.clone().unwrap();
        assert!(
            f.panel.iter().any(|l| l.starts_with("Position x")),
            "{:?}",
            f.panel
        );
        assert!(f.crosshair.is_some());
        let out = run(&mut app, "god.pin");
        assert!(out.iter().any(|l| l.contains("pin 1 put")), "{out:?}");
        let out = run(&mut app, "god.pins");
        assert!(out.iter().any(|l| l.contains("--start")), "{out:?}");
        app.update();
        assert_eq!(app.world().resource::<RideView>().pins.len(), 1);
        run(&mut app, "god.inspect off");
        app.update();
        let f = app.world().resource::<RideView>().flight.clone().unwrap();
        assert!(f.panel.is_empty() && f.crosshair.is_none());
        run(&mut app, "god.inspect on");
        run(&mut app, "god off");
        assert!(
            update_until(&mut app, |c| c.god().phase() == Phase::Riding),
            "{:?}",
            phase(&app)
        );
        app.update();
        assert!(!app.world().resource::<FlightPointer>().captured);
        assert!(app.world().resource::<RideView>().flight.is_none());
        let out = run(&mut app, "god status");
        assert!(out.iter().any(|l| l.starts_with("riding")), "{out:?}");
        let s = app.world().resource::<ClientState>().0.state();
        assert!(s.speed < 3.0, "landed standing: {s:?}");
        // The pins stay for the ride.
        assert_eq!(app.world().resource::<RideView>().pins.len(), 1);
    }
}

//! The browser app's ride session without Bevy (spec §14 Phase 10, B1):
//! the local server on the frame loop ([`FrameServer`]), the shared client
//! loop ([`ClientCore`]) and the virtual trainer ([`FrameVirtualTrainer`]).
//! Wasm has no threads, so where the desktop runs the server and the
//! trainer on their own threads, [`WebSession::frame`] advances both by
//! the frame time. The Bevy glue (`crate::plugin`) only calls it; the
//! whole loop is unit-tested natively.

use ocw_client::client::{ClientCore, Notice};
use ocw_client::input::{KeyThrottle, ThrottleKeys};
use ocw_core::RiderProfile;
use ocw_devices::{DeviceCommand, FrameVirtualTrainer};
use ocw_server::local::FrameServer;
use ocw_world::Route;

/// Power of the virtual trainer at the start (W).
pub const DEFAULT_POWER_W: f64 = 200.0;
/// The virtual power stays within this range (W).
pub const MAX_POWER_W: f64 = 1500.0;
/// Test riders on the browser's local server.
pub const DEFAULT_BOTS: u32 = 5;

/// What the session needs to start.
#[derive(Debug, Clone)]
pub struct SessionConfig {
    pub route: Route,
    pub name: String,
    pub bots: u32,
    pub start_m: f64,
    pub power_w: f64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            // The desktop's default test route (`--route hills`).
            route: Route::test_hills(),
            name: "Rider".into(),
            bots: DEFAULT_BOTS,
            start_m: 0.0,
            power_w: DEFAULT_POWER_W,
        }
    }
}

/// Server, client and trainer of one ride in the browser.
pub struct WebSession {
    pub server: FrameServer,
    pub client: ClientCore,
    pub trainer: FrameVirtualTrainer,
    /// Server ticks run so far (for the start-up log and the tests).
    pub ticks: u64,
}

impl WebSession {
    /// Starts the local server and joins it (the hello is sent at once).
    pub fn new(config: SessionConfig) -> Self {
        let SessionConfig {
            route,
            name,
            bots,
            start_m,
            power_w,
        } = config;
        let mut server = FrameServer::new(route.clone(), bots, start_m);
        let client = ClientCore::new(route, name, RiderProfile::default());
        server.send(&client.hello());
        Self {
            server,
            client,
            trainer: FrameVirtualTrainer::new(power_w),
            ticks: 0,
        }
    }

    /// One frame of `dt_s` seconds, at game time `now_s` (after the
    /// frame's time step): the trainer's sample goes to the server, the
    /// server runs its due ticks, and its messages go to the client.
    /// Returns the notices for the console.
    pub fn frame(&mut self, dt_s: f64, now_s: f64) -> Vec<Notice> {
        if let Some(event) = self.trainer.advance(dt_s)
            && let Some(sample) = self.client.on_device_event(event)
        {
            self.server.send(&sample);
        }
        self.ticks += u64::from(self.server.advance(dt_s));
        self.receive(now_s)
    }

    /// Sends a client message (steering, turns, events, dev commands).
    pub fn send(&mut self, bytes: &[u8]) {
        self.server.send(bytes);
    }

    /// Hands the server's messages to the client (also the immediate
    /// replies to [`send`](Self::send)).
    pub fn receive(&mut self, now_s: f64) -> Vec<Notice> {
        self.server
            .drain()
            .iter()
            .filter_map(|bytes| self.client.on_server_message(bytes, now_s))
            .collect()
    }

    /// The virtual trainer's set power (W).
    pub fn power_w(&self) -> f64 {
        self.trainer.set_power_w()
    }

    /// Sets the virtual trainer's power (W, clamped to 0 … [`MAX_POWER_W`]);
    /// the next sample (≤ 0.25 s) carries it.
    pub fn set_power(&mut self, watts: f64) -> f64 {
        let w = watts.clamp(0.0, MAX_POWER_W);
        self.trainer.command(DeviceCommand::SetVirtualPower(w));
        w
    }

    /// Applies the power keys of a frame ([`KeyThrottle`], the desktop's
    /// rules): ↑ adds keyboard power, ↓ stops (brake, keyboard and
    /// virtual power 0).
    pub fn throttle(&mut self, key: &mut KeyThrottle, keys: ThrottleKeys, dt_s: f64, now_s: f64) {
        let (msgs, stop) = key.update(&mut self.client, keys, dt_s, now_s);
        if stop {
            self.set_power(0.0);
        }
        for m in msgs {
            self.server.send(&m);
        }
    }

    /// The HUD's status line (device and steering).
    pub fn status_line(&self) -> String {
        let brake = if self.client.braking() {
            " | BRAKE (up: ride on)"
        } else {
            ""
        };
        format!(
            "Virtual trainer {:.0} W | {}{brake}",
            self.power_w(),
            self.client.steering_status()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs `seconds` of 60 Hz frames, with the client's prediction at
    /// 50 Hz as Bevy's FixedUpdate does it.
    fn ride(s: &mut WebSession, seconds: f64, now: &mut f64) -> Vec<Notice> {
        let dt = 1.0 / 60.0;
        let mut notices = Vec::new();
        let mut fixed = 0.0;
        let frames = (seconds / dt).round() as u32;
        for _ in 0..frames {
            *now += dt;
            fixed += dt;
            while fixed >= ocw_core::SIM_DT {
                fixed -= ocw_core::SIM_DT;
                s.client.predict_step();
            }
            notices.extend(s.frame(dt, *now));
        }
        notices
    }

    #[test]
    fn the_rider_joins_and_rides_at_the_virtual_power() {
        let mut s = WebSession::new(SessionConfig::default());
        let mut now = 0.0;
        let notices = ride(&mut s, 0.1, &mut now);
        assert!(
            notices.iter().any(|n| matches!(n, Notice::Joined { .. })),
            "{notices:?}"
        );
        assert!(s.client.rider_id().is_some());
        ride(&mut s, 10.0, &mut now);
        // 10.1 s at 50 Hz on the server.
        assert!((503..=506).contains(&s.ticks), "{}", s.ticks);
        assert_eq!(s.client.power_w(), DEFAULT_POWER_W);
        let state = s.client.state();
        assert!(state.speed > 1.5, "{state:?}");
        assert!(state.distance > 15.0, "{state:?}");
        // The local rider and the bots are in the view.
        let riders = s.client.view(now, 0.0);
        assert_eq!(riders.len(), 1 + DEFAULT_BOTS as usize);
        assert_eq!(riders.iter().filter(|r| r.is_local).count(), 1);
    }

    #[test]
    fn the_power_stays_within_bounds() {
        let mut s = WebSession::new(SessionConfig::default());
        assert_eq!(s.set_power(-50.0), 0.0);
        assert_eq!(s.set_power(5000.0), MAX_POWER_W);
        // The next sample brings the new power to the client and server.
        s.set_power(320.0);
        let mut now = 0.0;
        ride(&mut s, 0.5, &mut now);
        assert_eq!(s.client.power_w(), 320.0);
        assert!(s.status_line().starts_with("Virtual trainer 320 W"));
    }

    #[test]
    fn up_adds_keyboard_power_and_down_stops() {
        let mut s = WebSession::new(SessionConfig::default());
        let mut key = KeyThrottle::default();
        let mut now = 0.0;
        ride(&mut s, 0.5, &mut now);
        let up = ThrottleKeys {
            up_pressed: true,
            up_held: true,
            ..ThrottleKeys::default()
        };
        s.throttle(&mut key, up, 0.0, now);
        assert_eq!(s.client.power_w(), 225.0);
        let down = ThrottleKeys {
            down_pressed: true,
            ..ThrottleKeys::default()
        };
        s.throttle(&mut key, down, 0.0, now);
        ride(&mut s, 0.5, &mut now);
        assert_eq!((s.power_w(), s.client.power_w()), (0.0, 0.0));
        assert!(s.client.braking());
        assert!(s.status_line().contains("BRAKE"));
    }

    #[test]
    fn no_power_no_progress() {
        let mut s = WebSession::new(SessionConfig {
            power_w: 0.0,
            bots: 0,
            ..SessionConfig::default()
        });
        let mut now = 0.0;
        ride(&mut s, 5.0, &mut now);
        assert!(s.client.state().distance < 1.0);
        assert_eq!(s.client.view(now, 0.0).len(), 1);
    }
}

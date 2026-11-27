//! Headless bot client (`ocw-bot`): remote riders that join a game server
//! over QUIC like the desktop app does (spec §6.2) – Hello, sensor samples
//! from a virtual rider, snapshots, own-rider prediction – and report what
//! they saw, for the multiplayer test (`make test-multiplayer`, spec
//! §11.4). Bevy-free; the network is only in the binary's loop.

pub mod pacing;
pub mod report;
pub mod rider;

pub use pacing::{Effort, Leader, Pacing};
pub use report::{BotReport, Expect, RiderReport, Verdict, verdict};
pub use rider::{BotRider, RiderConfig};

/// Base power of the `i`-th rider of a process: around `power_w`, a few
/// per cent apart, so a field without a leader spreads a little.
pub fn rider_power_w(power_w: f64, i: u32) -> f64 {
    const SPREAD: [f64; 5] = [1.0, 0.92, 1.08, 0.96, 1.04];
    power_w * SPREAD[i as usize % SPREAD.len()]
}

/// The display name of rider `i` (0-based) of a process: `PREFIX-<i+1>`.
pub fn rider_name(prefix: &str, i: u32) -> String {
    format!("{prefix}-{}", i + 1)
}

/// Why `ocw-bot` has no default server (spec 11.5 *The public server as
/// the default*): every other app offers the public server first, but the
/// bots are test riders – the multiplayer test and the scripts start their
/// own local server and must never load the server on Fly.io (bursts of
/// new UDP flows get dropped there for minutes), and the public server
/// rides a world's road graph, which bots cannot ride.
pub const NO_DEFAULT_SERVER: &str = "--server HOST[:PORT] is required (or use `ocw-bot verdict`): \
ocw-bot has no default server – its riders are for tests against your own server, and the public \
server rides a world's road graph, which bots cannot ride";

/// The server to ride on: `--server`, never a default.
pub fn server_or_why(server: Option<&str>) -> Result<&str, &'static str> {
    server.ok_or(NO_DEFAULT_SERVER)
}

/// A hint for a connection that never came up.
pub fn connect_hint(reason: &str, port: u16) -> Option<String> {
    reason.contains("no answer").then(|| {
        format!(
            "no QUIC answer on UDP port {port}: is the server running (ocw-server --listen), \
             and does every firewall on the way let UDP {port} through?"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No default server: tests never reach the public server by leaving
    /// `--server` out.
    #[test]
    fn there_is_no_default_server() {
        let why = server_or_why(None).unwrap_err();
        assert!(why.contains("--server HOST[:PORT] is required"), "{why}");
        assert!(why.contains("no default server"), "{why}");
        assert_eq!(
            server_or_why(Some("127.0.0.1:47800")),
            Ok("127.0.0.1:47800")
        );
    }

    #[test]
    fn names_and_powers_per_rider() {
        assert_eq!(rider_name("mac", 0), "mac-1");
        assert_eq!(rider_name("win", 2), "win-3");
        assert_eq!(rider_power_w(200.0, 0), 200.0);
        assert!((rider_power_w(200.0, 1) - 184.0).abs() < 1e-9);
        assert_eq!(rider_power_w(200.0, 5), 200.0);
    }

    #[test]
    fn a_silent_server_gets_a_firewall_hint() {
        let h = connect_hint("no answer from the server", 47800).unwrap();
        assert!(h.contains("UDP 47800") && h.contains("firewall"));
        assert!(connect_hint("the server closed the connection", 47800).is_none());
    }
}

//! Rides that earn hearts (4O W5d, Q19 (a) and Q20 (a)).
//!
//! Two paths, each credited at most once per ride:
//! - **A signed game server** – one whose signing key the project's root
//!   key certified (the key hierarchy of spec 4.7) – sends a [`RideCredit`]
//!   signed with that key (`ocw_world::manifest::SignedDocument` under
//!   [`RIDE_CREDIT_CONTEXT`]) to `POST /rides/credit`; the backend checks
//!   the chain and credits directly.
//! - **Any other ride** (another server, the app's own local server, a
//!   ride offline) is uploaded by the app as a [`RideSummary`] to
//!   `POST /rides/summary` (bearer token); its hearts are credited once it
//!   passes the plausibility checks of spec 6.4 ([`crate::plausibility`]),
//!   flagged rides hold their hearts until reviewed.

use serde::{Deserialize, Serialize};

/// The context string of a ride credit's signature (like
/// `OCW world package manifest v1\0`).
pub const RIDE_CREDIT_CONTEXT: &[u8] = b"OCW ride credit v1\0";

/// The `kind` of a [`RideCredit`].
pub const RIDE_CREDIT_KIND: &str = "ocw-ride-credit";

/// What a signed game server says about one rider's ride it hosted.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RideCredit {
    pub kind: String,
    /// The rider's account (the `sub` of the hello's access token).
    pub account: String,
    /// The server's id of the ride (unique per server and rider).
    pub ride_id: String,
    pub started_unix_s: i64,
    pub ended_unix_s: i64,
    pub distance_m: f64,
    /// Work and climbing the server counted (virtual power, god mode,
    /// crashes and bots left out, `ocw_core::hearts::HeartsMeter`).
    pub work_j: f64,
    pub climb_m: f64,
    /// The hearts the server counted; the backend recomputes them from
    /// `work_j` and `climb_m` and refuses a mismatch.
    pub hearts: u32,
    /// The garage bike ridden (its parts count as ridden).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bike: Option<String>,
    /// When the server signed it (Unix s).
    pub issued_unix_s: i64,
}

/// A device the app rode with (spec 6.4.3 trust levels).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceKind {
    /// A smart trainer (power, often cadence).
    Trainer,
    /// An independent power meter (dual recording with a trainer).
    PowerMeter,
    Cadence,
    HeartRate,
    /// Keyboard power, the virtual trainer, `sim.power`: never earns.
    Virtual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub kind: DeviceKind,
    /// The name the device announces (shown in a review, never a person's).
    #[serde(default)]
    pub name: String,
}

/// The summary of one ride the app uploads (Q19 (a): rides not credited
/// by a signed server; Q20 (a): rides offline, queued and uploaded later).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RideSummary {
    /// The app's id of the ride (random, unique per account): the
    /// backend credits each id once.
    pub ride_id: String,
    pub started_unix_s: i64,
    pub duration_s: u32,
    pub distance_m: f64,
    pub climb_m: f64,
    /// Work counted by the app (only non-virtual power).
    pub work_j: f64,
    /// The rider's weight (kg) for the W/kg limits.
    pub weight_kg: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bike: Option<String>,
    #[serde(default)]
    pub devices: Vec<Device>,
    /// Power once a second (W).
    #[serde(default)]
    pub power_w: Vec<u16>,
    /// Cadence once a second (rpm); empty: no cadence recorded.
    #[serde(default)]
    pub cadence_rpm: Vec<u16>,
    /// Heart rate once a second (bpm); empty: none.
    #[serde(default)]
    pub heart_bpm: Vec<u16>,
}

/// What happened to an uploaded ride.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RideStatus {
    /// Passed the checks: hearts credited.
    Credited,
    /// Implausible: kept, hearts held until a review (spec 6.4.5).
    Flagged,
    /// Malformed or not eligible (virtual power, overlaps a ride already
    /// credited): no hearts.
    Refused,
}

/// The answer to `POST /rides/credit` and `/rides/summary`. A ride sent
/// again gets the first answer (`repeated`), nothing is credited twice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RideAnswer {
    pub ride_id: String,
    pub status: RideStatus,
    /// Hearts credited (0 when flagged or refused).
    pub hearts: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reasons: Vec<String>,
    pub balance: i64,
    /// This ride was answered before; this is that answer.
    #[serde(default)]
    pub repeated: bool,
}

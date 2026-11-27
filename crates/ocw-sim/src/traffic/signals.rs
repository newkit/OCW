//! Traffic lights with deterministic cycles from the tick (spec 18B
//! *Behaviour*): every signalled junction runs a two-phase cycle of 60 –
//! 90 s, its length and offset from the vertex, so server and clients
//! (and a replay) show the same lights without sending them.
//! The approaches are split into two groups by their heading against the
//! junction's axis (the first road at the vertex): along it, or across.

use ocw_world::network::splitmix;

/// Amber (ms).
pub const AMBER_MS: u64 = 3_000;
/// All red between the phases (ms).
pub const ALL_RED_MS: u64 = 2_000;

/// A light's colour for one approach.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Light {
    Green,
    Amber,
    Red,
}

/// The cycle of a signalled vertex: (length, offset) in ms.
pub fn cycle(vertex: u32) -> (u64, u64) {
    let h = splitmix(u64::from(vertex) ^ 0x5167_4E41_4C53);
    let len = 60_000 + (h % 4) * 10_000;
    (len, (h >> 8) % len)
}

/// The group of an approach heading `arrive` at a junction whose axis is
/// `axis` (unit vectors): 0 along the axis, 1 across.
pub fn group(arrive: [f64; 2], axis: [f64; 2]) -> u8 {
    let d = arrive[0] * axis[0] + arrive[1] * axis[1];
    u8::from(d * d < 0.5)
}

/// The light of `group` at `vertex` at `t_ms` (simulation time), and the
/// ms it has shown that colour.
pub fn light(vertex: u32, group: u8, t_ms: u64) -> (Light, u64) {
    let (len, offset) = cycle(vertex);
    let half = len / 2;
    let local = (t_ms + offset + len - u64::from(group) * half) % len;
    let green = half - AMBER_MS - ALL_RED_MS;
    if local < green {
        (Light::Green, local)
    } else if local < green + AMBER_MS {
        (Light::Amber, local - green)
    } else {
        (Light::Red, local - green - AMBER_MS)
    }
}

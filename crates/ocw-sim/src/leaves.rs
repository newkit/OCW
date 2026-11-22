//! Wet leaves on the road (spec 4.6.2 Q6, decided 2026-11-22: (b) and
//! (c)): from the leaf fall through the winter leaves lie on the road
//! (`ocw_world::leaves`: the same map and amount the renderer draws), and
//! while the road is wet they
//!
//! - **cost a little speed:** [`ROLLING`] × the slickness added to the
//!   factor on the rolling resistance ([`extra_rolling`], additive with
//!   the wet road's and other surface effects) – on a full cover on a
//!   soaked road +50 % (about 1 km/h less at 200 W), in the Black
//!   Forest's mixed woods (a leaf every 4 – 6 m) +20 – 30 %;
//! - **wobble the bike at speed:** a sideways sway of at most
//!   [`WOBBLE_M`] × the slickness (3 cm at full; from [`WOBBLE_FROM_MPS`],
//!   full from [`WOBBLE_FULL_MPS`]) along the road's leaves (two waves of
//!   3.7 m and 1.9 m: about 2 – 4 Hz at 30 km/h), seen on the bike's lean
//!   (up to [`WOBBLE_LEAN_RAD`], 4°); every step under 2.5 cm a tick;
//! - **with damage on, never in group rides, may make the bike slide in a
//!   corner** ([`slide_hazard`]): above [`SLIDE_MIN_MPS`] and a sideways
//!   pull of [`SLIDE_MIN_G`] × g, a chance per second of
//!   [`SLIDE_RATE_PER_S`] × the slickness × the pull's excess over it in
//!   shares of it (at most [`SLIDE_MAX_PER_S`]), drawn per tick from the
//!   rider and the tick (deterministic) – the crash of spec 14 Phase 12
//!   with its own kind ([`crate::collide::ContactKind::Leaves`], "Wet
//!   leaves – slipped"), its sound and damage. Only client riders slide
//!   (bots ride the cost and the wobble).
//!
//! The **slickness** ([`slick`], 0 – 1) is the leaves' cover (leaves per
//! metre over [`FULL_COVER_PER_M`], at most 1) × the
//! road's wetness from [`WET_FROM`] (nothing on a damp road) to
//! [`WET_FULL`]. Nothing at all on a dry road, outside the leaf season,
//! under lying snow (from half: the leaves are hidden, as drawn), in a
//! tunnel, off the road, or while the weather does not affect the physics
//! – there the ride is bit for bit the one without leaves (tested).
//!
//! The server rides it for everyone; the client's prediction rides the
//! same cost and wobble from the same map, season and weather (the slide
//! is only the server's, like every crash).

use std::sync::Arc;

use ocw_core::RiderId;
use ocw_sky::season::{SeasonDay, SeasonModel};
use ocw_world::Track;
use ocw_world::leaves::LeafMap;

use crate::collide::{ContactKind, Hit};
use crate::physics::RiderState;
use crate::weather::WeatherPhysics;

/// Leaves per metre that make a full cover: a leaf every 2.5 m on each
/// half of the road – half of a pure broadleaf wood's after the leaf fall
/// (`ocw_world::leaves::WOOD_PER_M`), so the Black Forest's mixed woods
/// (0.16 – 0.24 a metre) are 40 – 60 % slick when soaked and a beech wood
/// fully.
pub const FULL_COVER_PER_M: f64 = 0.4;
/// The road's wetness from which wet leaves count …
pub const WET_FROM: f64 = 0.1;
/// … and fully.
pub const WET_FULL: f64 = 0.5;
/// Lying snow from which the leaves are hidden (as the renderer's
/// `SNOW_HIDES_SMALL`).
pub const SNOW_HIDES: f64 = 0.5;
/// Extra rolling resistance at full slickness (factor − 1 on Crr).
pub const ROLLING: f64 = 0.5;
/// The wobble's sideways sway at full slickness (m).
pub const WOBBLE_M: f64 = 0.03;
/// No wobble below this speed (m/s; 18 km/h) …
pub const WOBBLE_FROM_MPS: f64 = 5.0;
/// … full from here (36 km/h).
pub const WOBBLE_FULL_MPS: f64 = 10.0;
/// The wobble's lean at full slickness (rad).
pub const WOBBLE_LEAN_RAD: f64 = 0.07;
/// Its two waves along the road (m) and their shares.
pub const WOBBLE_WAVES_M: [(f64, f64); 2] = [(3.7, 0.6), (1.9, 0.4)];
/// No slide below this speed (m/s; 21.6 km/h).
pub const SLIDE_MIN_MPS: f64 = 6.0;
/// No slide below this sideways pull (share of g: about 8.5° of lean).
pub const SLIDE_MIN_G: f64 = 0.15;
/// The slide's chance per second at full slickness and twice the pull.
pub const SLIDE_RATE_PER_S: f64 = 0.1;
/// Its highest chance per second.
pub const SLIDE_MAX_PER_S: f64 = 0.5;
/// The verge point the place is read at: this far beyond the road's edge
/// on the rider's side (as the renderer's road leaves).
pub const VERGE_M: f64 = 3.0;

/// Where leaves lie and the season: what the physics needs to know of
/// them. `None` on test routes and worlds without land use.
#[derive(Debug, Clone)]
pub struct LeafGround {
    pub map: Arc<LeafMap>,
    pub day: Arc<SeasonDay>,
}

impl LeafGround {
    /// Leaves per metre under the rider in `state` on `track`: the amount
    /// on the half of the road the rider rides on (0 off the road).
    pub fn per_metre(&self, track: &dyn Track, state: &RiderState) -> f64 {
        let half = track.road_width_at(state.distance) / 2.0;
        if state.lateral_offset.abs() > half {
            return 0.0;
        }
        let p = track.sample(state.distance);
        let side = if state.lateral_offset < 0.0 {
            -1.0
        } else {
            1.0
        };
        let r = p.right();
        let out = side * (half + VERGE_M);
        let (x, z) = (p.position[0] + r[0] * out, p.position[2] + r[1] * out);
        let alt = p.position[1];
        f64::from(
            self.map
                .per_metre_at(x, z, alt, self.day.leaf_fall(alt), self.day.snow(alt)),
        )
    }
}

/// How slippery the road is under the rider (0 – 1; see the module docs).
pub fn slick(
    ground: Option<&LeafGround>,
    weather: Option<&WeatherPhysics>,
    track: &dyn Track,
    state: &RiderState,
) -> f64 {
    let (Some(g), Some(w)) = (ground, weather) else {
        return 0.0;
    };
    if w.snow_cover >= SNOW_HIDES || track.sheltered(state.distance) {
        return 0.0;
    }
    let wet = ((w.wetness - WET_FROM) / (WET_FULL - WET_FROM)).clamp(0.0, 1.0);
    if wet <= 0.0 {
        return 0.0;
    }
    (g.per_metre(track, state) / FULL_COVER_PER_M).clamp(0.0, 1.0) * wet
}

/// The extra rolling resistance of wet leaves of slickness `s` (added to
/// the factor on Crr: additive, so other effects of the road surface –
/// the weather's wet road, road damage – add up with it).
pub fn extra_rolling(s: f64) -> f64 {
    ROLLING * s.max(0.0)
}

/// The weather the rider rides on wet leaves of slickness `s`: its
/// rolling factor + [`extra_rolling`]. `s` 0: `w` itself.
pub fn on_leaves(w: &WeatherPhysics, s: f64) -> WeatherPhysics {
    if s <= 0.0 {
        return *w;
    }
    WeatherPhysics {
        rolling_factor: w.rolling_factor + extra_rolling(s),
        ..*w
    }
}

fn ramp(x: f64, lo: f64, hi: f64) -> f64 {
    ((x - lo) / (hi - lo)).clamp(0.0, 1.0)
}

/// The wobble's shape along the road (−1 … 1), `phase` turning both
/// waves (a quarter wave for the lean).
fn shape(distance: f64, phase: f64) -> f64 {
    let tau = std::f64::consts::TAU;
    WOBBLE_WAVES_M
        .iter()
        .enumerate()
        .map(|(i, (wave, share))| share * (tau * distance / wave + phase + 0.7 * i as f64).sin())
        .sum()
}

/// The sideways sway (m, positive = right) at `distance` on wet leaves of
/// slickness `s` at `speed`: at most [`WOBBLE_M`] × s.
pub fn wobble_offset(s: f64, speed: f64, distance: f64) -> f64 {
    WOBBLE_M * s * ramp(speed, WOBBLE_FROM_MPS, WOBBLE_FULL_MPS) * shape(distance, 0.0)
}

/// The lean the wobble shows (rad, positive = right): a quarter wave ahead
/// of the sway, at most [`WOBBLE_LEAN_RAD`] × s.
pub fn wobble_lean(s: f64, speed: f64, distance: f64) -> f64 {
    WOBBLE_LEAN_RAD
        * s
        * ramp(speed, WOBBLE_FROM_MPS, WOBBLE_FULL_MPS)
        * shape(distance, std::f64::consts::FRAC_PI_2)
}

/// Moves the rider in `state` by the change of the wobble since the last
/// tick (`applied`: the sway already in its lateral offset, updated),
/// within `limit` of the centre line.
pub fn apply_wobble(state: &mut RiderState, s: f64, applied: &mut f64, limit: f64) {
    let w = wobble_offset(s, state.speed, state.distance);
    if w == 0.0 && *applied == 0.0 {
        return;
    }
    state.lateral_offset = (state.lateral_offset + w - *applied).clamp(-limit, limit);
    *applied = w;
}

/// The chance per second of a slide on wet leaves of slickness `s` at
/// `speed` on a path of `curvature` (1/m; see the module docs).
pub fn slide_hazard(s: f64, speed: f64, curvature: f64) -> f64 {
    if s <= 0.0 || speed < SLIDE_MIN_MPS {
        return 0.0;
    }
    let pull = speed * speed * curvature.abs() / ocw_core::GRAVITY;
    if pull <= SLIDE_MIN_G {
        return 0.0;
    }
    (SLIDE_RATE_PER_S * s * (pull - SLIDE_MIN_G) / SLIDE_MIN_G).min(SLIDE_MAX_PER_S)
}

fn splitmix(mut x: u64) -> u64 {
    x = x.wrapping_add(0x9e37_79b9_7f4a_7c15);
    x = (x ^ (x >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    x ^ (x >> 31)
}

/// Whether rider `id` slides at `tick` with `hazard` per second (one
/// deterministic draw per rider and tick).
pub fn slides(id: RiderId, tick: u64, hazard: f64, dt: f64) -> bool {
    if hazard <= 0.0 {
        return false;
    }
    let u = (splitmix((u64::from(id.0) << 40) ^ tick ^ 0x6c65_6176_6573) >> 11) as f64
        / (1u64 << 53) as f64;
    u < hazard * dt
}

/// The fall of a slide on wet leaves: a lowside out of the turn (about
/// 0.3 × the speed sideways), like the skid's.
pub fn slide_hit(speed: f64, mass: f64, curvature: f64) -> Hit {
    let sideways = 0.3 * speed;
    let outward = if curvature > 0.0 { -1.0 } else { 1.0 };
    Hit {
        kind: ContactKind::Leaves,
        other: RiderId(0),
        closing_mps: sideways,
        energy_j: 0.5 * mass * sideways * sideways,
        impulse: [-0.1 * speed, outward * sideways],
    }
}

/// The season the leaves lie in: the day of the season bucket
/// (`ocw_sky::season::season_bucket`, as the renderer's) at the world
/// instant, at the ride's place, cached. The server and the client keep
/// one each.
/// A place's time zone and season model.
type PlaceSeason = ((f64, f64), Option<ocw_sky::Zone>, Arc<SeasonModel>);

#[derive(Debug, Clone, Default)]
pub struct LeafSeason {
    place: Option<PlaceSeason>,
    day: Option<(u32, Arc<SeasonDay>)>,
}

impl LeafSeason {
    /// The season day at world instant `ms` (Unix ms) at `place` (lat,
    /// lon; its local date in its time zone).
    pub fn day_at(&mut self, ms: i64, place: (f64, f64)) -> Arc<SeasonDay> {
        if self.place.as_ref().is_none_or(|p| p.0 != place) {
            let zone = ocw_sky::Zone::get(&ocw_sky::zone::zone_for(place.0, place.1)).ok();
            let model = Arc::new(SeasonModel::for_place(place.0, place.1));
            self.place = Some((place, zone, model));
            self.day = None;
        }
        let (_, zone, model) = self.place.as_ref().expect("set above");
        let (y, m, d) = match zone {
            Some(z) => {
                let l = z.local(ms);
                (l.year, l.month, l.day)
            }
            None => ocw_sky::time::utc_date(ms),
        };
        let bucket = ocw_sky::season::season_bucket(ocw_world::look::day_of_year(y, m, d));
        if let Some((b, day)) = &self.day
            && *b == bucket
        {
            return day.clone();
        }
        let day = Arc::new(model.day(ocw_sky::season::bucket_day(bucket)));
        self.day = Some((bucket, day.clone()));
        day
    }
}

#[cfg(test)]
mod tests;

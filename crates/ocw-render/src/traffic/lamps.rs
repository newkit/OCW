//! The lamps of the road vehicles and what moves on them (spec 14 Phase
//! 18, 18A, 18E T7), pure:
//!
//! - [`lamps`]: dipped headlights and tail lamps from dusk to dawn, in
//!   rain or fog and in tunnels (daytime running lights otherwise), brake
//!   lamps from the braking flag, the indicators blinking at 1.5 Hz while
//!   the vehicle turns or pulls out, a bus's interior lights when its
//!   headlights are on;
//! - [`tag`]: the state packed into the instance's tag the vehicle shader
//!   reads (`vehicle.wesl`: the paint colour and the lamp bits), so every
//!   vehicle of a model stays in one instanced draw;
//! - [`trailer_yaw`]: the articulated lorry's trailer follows the path the
//!   kingpin drove;
//! - [`wheel_turn`]: wheels turn with the distance driven.

use super::{Indicator, VehicleView};
use ocw_sim::traffic::VehicleClass;

/// What the scene asks of the lamps.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LampScene {
    /// From dusk to dawn (`ocw_sky::Light::lights_on`).
    pub dark: bool,
    /// Rain, snow or hail falling (mm/h).
    pub precip_mm_h: f32,
    /// The visibility (m; fog).
    pub visibility_m: f32,
}

/// Headlights on in rain from this much (mm/h) …
pub const RAIN_LIGHTS_MM_H: f32 = 0.8;
/// … and in fog below this visibility (m).
pub const FOG_LIGHTS_M: f32 = 1500.0;
/// The indicators' rate (Hz; 60 – 120 a minute by law).
pub const INDICATOR_HZ: f64 = 1.5;

/// What is lit on one vehicle now.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct LampState {
    /// Dipped headlights (and the tail lamps with them).
    pub head: bool,
    /// Daytime running lights (dim, in the headlamps) while `head` is off.
    pub drl: bool,
    pub brake: bool,
    /// The left / right indicators lit in this phase of the blink.
    pub left: bool,
    pub right: bool,
    /// A bus's lights inside.
    pub interior: bool,
}

/// The lamps of `v` at time `t` (s) in `scene`; `in_tunnel`: under the
/// ground (a tunnel or a gallery).
pub fn lamps(scene: &LampScene, v: &VehicleView, in_tunnel: bool, t: f64) -> LampState {
    // A parked car is dark (spec 18B).
    if v.parked {
        return LampState::default();
    }
    let head = scene.dark
        || in_tunnel
        || scene.precip_mm_h >= RAIN_LIGHTS_MM_H
        || scene.visibility_m < FOG_LIGHTS_M;
    // Each vehicle blinks in its own phase.
    let phase = f64::from(v.id % 997) / 997.0;
    let on = (t * INDICATOR_HZ + phase).fract() < 0.5;
    let (left, right) = match v.indicator {
        Indicator::Off => (false, false),
        Indicator::Left => (on, false),
        Indicator::Right => (false, on),
    };
    LampState {
        head,
        drl: !head,
        brake: v.braking,
        left,
        right,
        interior: v.class == VehicleClass::Bus && (scene.dark || in_tunnel),
    }
}

/// Bits of the instance tag above the colour (`vehicle.wesl`).
pub const TAG_HEAD: u32 = 1 << 24;
pub const TAG_DRL: u32 = 1 << 25;
pub const TAG_BRAKE: u32 = 1 << 26;
pub const TAG_LEFT: u32 = 1 << 27;
pub const TAG_RIGHT: u32 = 1 << 28;
pub const TAG_INTERIOR: u32 = 1 << 29;

/// The instance tag of a vehicle: its paint (sRGB 0xRRGGBB) in the low 24
/// bits and its lamps above.
pub fn tag(paint: u32, l: &LampState) -> u32 {
    let mut t = paint & 0xFF_FFFF;
    for (on, bit) in [
        (l.head, TAG_HEAD),
        (l.drl, TAG_DRL),
        (l.brake, TAG_BRAKE),
        (l.left, TAG_LEFT),
        (l.right, TAG_RIGHT),
        (l.interior, TAG_INTERIOR),
    ] {
        if on {
            t |= bit;
        }
    }
    t
}

/// The shader's lamp levels for a role (the twin of `vehicle.wesl`'s
/// `lamp_level`): 0 off … 1 full.
pub fn lamp_level(role: Role, tag: u32) -> f32 {
    let has = |b: u32| tag & b != 0;
    match role {
        Role::Head => {
            if has(TAG_HEAD) {
                1.0
            } else if has(TAG_DRL) {
                0.35
            } else {
                0.0
            }
        }
        Role::Brake => {
            if has(TAG_BRAKE) {
                1.0
            } else if has(TAG_HEAD) {
                0.3
            } else {
                0.0
            }
        }
        Role::IndicatorLeft => f32::from(u8::from(has(TAG_LEFT))),
        Role::IndicatorRight => f32::from(u8::from(has(TAG_RIGHT))),
        Role::Interior => f32::from(u8::from(has(TAG_INTERIOR))),
        _ => 0.0,
    }
}

/// What a vertex of a vehicle mesh is (its uv.x; `vehicle.wesl`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Role {
    /// The model's own colour (vertex colour).
    Fixed = 0,
    /// The paint slot: the instance's colour.
    Paint = 1,
    Head = 2,
    Brake = 3,
    IndicatorLeft = 4,
    IndicatorRight = 5,
    /// A bus's windows: lit from inside at night.
    Interior = 6,
    /// Metal (rims, chrome).
    Metal = 7,
}

impl Role {
    /// The role of a vertex from its paint slot and its x in the model
    /// (the model faces −Z: −x is its left; indicators by side).
    pub fn of(slot: &str, x: f32, paint_slot: &str, bus: bool) -> Self {
        if slot == paint_slot {
            return Self::Paint;
        }
        match slot {
            "light_head" => Self::Head,
            "light_brake" => Self::Brake,
            // By position: the lod1 meshes carry all indicators in one node.
            "light_indicator" if x < 0.0 => Self::IndicatorLeft,
            "light_indicator" => Self::IndicatorRight,
            "glass" if bus => Self::Interior,
            "chrome" | "rim" | "metal" => Self::Metal,
            _ => Self::Fixed,
        }
    }
}

/// The articulated lorry's trailer yaw (rad, as [`VehicleView::yaw`]):
/// its axles follow the kingpin's track. `trail` are earlier kingpin
/// positions (x, z), the newest last; `kingpin` where it is now; `length`
/// the kingpin to the trailer's axles (m). Without enough track: `fallback`
/// (the tractor's yaw).
pub fn trailer_yaw(trail: &[[f32; 2]], kingpin: [f32; 2], length: f32, fallback: f32) -> f32 {
    // Walk back along the track until `length` from the kingpin.
    let dist = |p: [f32; 2]| ((p[0] - kingpin[0]).powi(2) + (p[1] - kingpin[1]).powi(2)).sqrt();
    let mut prev = kingpin;
    for &p in trail.iter().rev() {
        let (a, b) = (dist(prev), dist(p));
        if b >= length {
            // Between prev and p.
            let k = if b - a > 1e-6 {
                ((length - a) / (b - a)).clamp(0.0, 1.0)
            } else {
                1.0
            };
            let q = [
                prev[0] + (p[0] - prev[0]) * k,
                prev[1] + (p[1] - prev[1]) * k,
            ];
            let h = [kingpin[0] - q[0], kingpin[1] - q[1]];
            return (-h[0]).atan2(-h[1]);
        }
        prev = p;
    }
    fallback
}

/// Keeps a kingpin track: a point every `step` metres, at most `keep`.
pub fn push_trail(trail: &mut Vec<[f32; 2]>, p: [f32; 2], step: f32, keep: usize) {
    let far = trail
        .last()
        .is_none_or(|q| (q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) >= step * step);
    if far {
        trail.push(p);
        if trail.len() > keep {
            trail.drain(..trail.len() - keep);
        }
    }
}

/// A wheel's turn (rad) after driving `metres` with radius `r` (m).
pub fn wheel_turn(metres: f32, r: f32) -> f32 {
    if r <= 0.01 {
        0.0
    } else {
        (metres / r) % std::f32::consts::TAU
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn car(braking: bool, indicator: Indicator) -> VehicleView {
        VehicleView {
            id: 3,
            class: VehicleClass::Car,
            look: 1,
            pos: [0.0; 3],
            heading: [0.0, -1.0],
            lateral: 1.5,
            lane: 0,
            speed: 12.0,
            braking,
            indicator,
            line: 0,
            doors: false,
            parked: false,
        }
    }

    #[test]
    fn headlights_at_night_in_rain_fog_and_tunnels() {
        let day = LampScene {
            dark: false,
            precip_mm_h: 0.0,
            visibility_m: 30_000.0,
        };
        let v = car(false, Indicator::Off);
        let l = lamps(&day, &v, false, 0.0);
        assert!(!l.head && l.drl && !l.brake && !l.interior);
        for (scene, tunnel) in [
            (LampScene { dark: true, ..day }, false),
            (
                LampScene {
                    precip_mm_h: 2.0,
                    ..day
                },
                false,
            ),
            (
                LampScene {
                    visibility_m: 400.0,
                    ..day
                },
                false,
            ),
            (day, true),
        ] {
            let l = lamps(&scene, &v, tunnel, 0.0);
            assert!(l.head && !l.drl, "{scene:?} {tunnel}");
        }
        // Drizzle under the limit: no.
        let drizzle = LampScene {
            precip_mm_h: 0.3,
            ..day
        };
        assert!(!lamps(&drizzle, &v, false, 0.0).head);
        // A bus is lit inside at night only.
        let bus = VehicleView {
            class: VehicleClass::Bus,
            ..v
        };
        assert!(lamps(&LampScene { dark: true, ..day }, &bus, false, 0.0).interior);
        assert!(!lamps(&day, &bus, false, 0.0).interior);
        // A parked car has no lamps lit, day or night.
        let parked = VehicleView { parked: true, ..v };
        for scene in [day, LampScene { dark: true, ..day }] {
            assert_eq!(lamps(&scene, &parked, true, 0.0), LampState::default());
        }
    }

    #[test]
    fn brake_lamps_and_indicators_blink() {
        let scene = LampScene {
            dark: true,
            precip_mm_h: 0.0,
            visibility_m: 30_000.0,
        };
        assert!(lamps(&scene, &car(true, Indicator::Off), false, 0.0).brake);
        let v = car(false, Indicator::Left);
        // Over two seconds: lit half the time, never on the right.
        let mut lit = 0;
        let mut changes = 0;
        let mut last = false;
        for i in 0..2000 {
            let l = lamps(&scene, &v, false, f64::from(i) * 0.001);
            assert!(!l.right);
            if l.left {
                lit += 1;
            }
            if l.left != last {
                changes += 1;
                last = l.left;
            }
        }
        assert!((900..1100).contains(&lit), "{lit}");
        // 1.5 Hz: three blinks in two seconds (on and off: ~6 changes).
        assert!((5..=7).contains(&changes), "{changes}");
    }

    #[test]
    fn the_tag_carries_the_paint_and_the_lamps() {
        let l = LampState {
            head: true,
            brake: true,
            left: true,
            ..Default::default()
        };
        let t = tag(0x12_3456, &l);
        assert_eq!(t & 0xFF_FFFF, 0x12_3456);
        assert_eq!(lamp_level(Role::Head, t), 1.0);
        assert_eq!(lamp_level(Role::Brake, t), 1.0);
        assert_eq!(lamp_level(Role::IndicatorLeft, t), 1.0);
        assert_eq!(lamp_level(Role::IndicatorRight, t), 0.0);
        // Tail lamps dim with the headlights, off by day; DRL dim.
        let night = tag(
            0,
            &LampState {
                head: true,
                ..Default::default()
            },
        );
        assert!((lamp_level(Role::Brake, night) - 0.3).abs() < 1e-6);
        let day = tag(
            0,
            &LampState {
                drl: true,
                ..Default::default()
            },
        );
        assert_eq!(lamp_level(Role::Brake, day), 0.0);
        assert!(lamp_level(Role::Head, day) > 0.0 && lamp_level(Role::Head, day) < 0.5);
        assert_eq!(lamp_level(Role::Paint, t), 0.0);
    }

    #[test]
    fn roles_from_slots_and_nodes() {
        assert_eq!(Role::of("body", 0.5, "body", false), Role::Paint);
        assert_eq!(Role::of("livery", 0.5, "livery", true), Role::Paint);
        assert_eq!(Role::of("body", 0.5, "livery", true), Role::Fixed);
        assert_eq!(
            Role::of("light_indicator", -0.8, "body", false),
            Role::IndicatorLeft
        );
        assert_eq!(
            Role::of("light_indicator", 0.8, "body", false),
            Role::IndicatorRight
        );
        assert_eq!(Role::of("glass", 0.0, "livery", true), Role::Interior);
        assert_eq!(Role::of("glass", 0.0, "body", false), Role::Fixed);
        assert_eq!(Role::of("light_head", -0.5, "body", false), Role::Head);
        assert_eq!(Role::of("rim", 0.5, "body", false), Role::Metal);
    }

    #[test]
    fn the_trailer_follows_the_kingpin() {
        // Straight north (−z): the trailer in line.
        let mut trail = Vec::new();
        for i in 0..40 {
            push_trail(&mut trail, [0.0, 20.0 - i as f32], 0.5, 64);
        }
        let k = [0.0, -20.0];
        assert!(trailer_yaw(&trail, k, 8.0, 1.0).abs() < 1e-4);
        // Then a right turn (east, +x): the kingpin heads east, the
        // trailer still points mostly north – it lags behind the turn.
        for i in 1..=6 {
            push_trail(&mut trail, [i as f32, -20.0], 0.5, 64);
        }
        let k = [6.0, -20.0];
        let yaw = trailer_yaw(&trail, k, 8.0, 0.0);
        let tractor = (-1.0f32).atan2(0.0); // east: −π/2
        assert!(yaw < 0.0 && yaw > tractor, "{yaw}");
        // Without a track: the fallback.
        assert_eq!(trailer_yaw(&[], k, 8.0, 0.7), 0.7);
        // The trail keeps its length.
        assert!(trail.len() <= 64);
    }

    #[test]
    fn wheels_turn_with_the_distance() {
        // A 0.31 m car wheel over 1.948 m: one turn.
        let one = 2.0 * std::f32::consts::PI * 0.31;
        assert!((wheel_turn(one * 0.5, 0.31) - std::f32::consts::PI).abs() < 1e-4);
        let full = wheel_turn(one, 0.31);
        assert!(full.abs() < 1e-3 || (full - std::f32::consts::TAU).abs() < 1e-3);
        assert_eq!(wheel_turn(5.0, 0.0), 0.0);
    }
}

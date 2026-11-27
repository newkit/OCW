//! The road vehicles on the wire (spec 14 Phase 18, 18E T5; Q3 (a)):
//! `TrafficUpdate` with the plans of vehicles new to a rider
//! (`TrafficSpawn`), the changes of those it has (`TrafficChange`) and
//! despawns. The conversions are exact: the speeds, positions and the
//! driver's parameters keep their bits, so a client's
//! `ocw_sim::traffic::PlanFollower` drives each vehicle as the server's
//! follower does.

use ocw_sim::traffic::{DriverParams, Motion, Plan, PlanChange, Shift, Step, Tick, VehicleClass};

use crate::v1;

/// Ticks between the event and the update it is sent in (0 for a tick
/// after the update's: a newer event is never sent in an older update).
fn age(update_tick: Tick, tick: Tick) -> u32 {
    update_tick.saturating_sub(tick).min(u64::from(u32::MAX)) as u32
}

fn motion_to_wire(m: Motion) -> (i32, f32, f32) {
    match m {
        Motion::Free => (v1::TrafficMotion::Free as i32, 0.0, 0.0),
        Motion::Accel { a, target } => (v1::TrafficMotion::Accel as i32, a, target),
        Motion::Stopped => (v1::TrafficMotion::Stopped as i32, 0.0, 0.0),
    }
}

fn motion_from_wire(kind: i32, a: f32, target: f32) -> Motion {
    match v1::TrafficMotion::try_from(kind) {
        Ok(v1::TrafficMotion::Accel) => Motion::Accel { a, target },
        Ok(v1::TrafficMotion::Stopped) => Motion::Stopped,
        _ => Motion::Free,
    }
}

/// A route as packed arrays.
pub fn route_to_wire(route: &[Step]) -> v1::TrafficRoute {
    v1::TrafficRoute {
        edges: route
            .iter()
            .map(|s| s.edge * 2 + u32::from(!s.forward))
            .collect(),
        speeds: route.iter().map(|s| s.speed).collect(),
        turn_r: route.iter().map(|s| s.turn_r).collect(),
    }
}

/// A received route (`None` when its arrays differ in length).
pub fn route_from_wire(r: &v1::TrafficRoute) -> Option<Vec<Step>> {
    if r.speeds.len() != r.edges.len() || r.turn_r.len() != r.edges.len() {
        return None;
    }
    Some(
        r.edges
            .iter()
            .zip(&r.speeds)
            .zip(&r.turn_r)
            .map(|((&e, &speed), &turn_r)| Step {
                edge: e / 2,
                forward: e % 2 == 0,
                speed,
                turn_r,
            })
            .collect(),
    )
}

/// A plan for an update at `update_tick`.
pub fn spawn_to_wire(p: &Plan, update_tick: Tick) -> v1::TrafficSpawn {
    let (motion, accel, target) = motion_to_wire(p.motion);
    let d = p.driver;
    v1::TrafficSpawn {
        id: p.id,
        class: p.class.index() as u32,
        look: p.look,
        age: age(update_tick, p.tick),
        driver: vec![d.accel, d.decel, d.lateral, d.headway, d.gap],
        route: Some(route_to_wire(&p.route)),
        base: p.base,
        s: p.s,
        v: p.v,
        lane: u32::from(p.lane),
        motion,
        accel,
        target,
        shift_at: p.shift.at,
        shift_to: p.shift.to,
        shift_rate: p.shift.rate,
    }
}

/// A received plan (`None`: malformed or an unknown vehicle class).
pub fn spawn_from_wire(m: &v1::TrafficSpawn, update_tick: Tick) -> Option<Plan> {
    let class = VehicleClass::from_index(m.class as usize)?;
    let [accel, decel, lateral, headway, gap] = m.driver[..] else {
        return None;
    };
    let route = route_from_wire(m.route.as_ref()?)?;
    if route.is_empty() {
        return None;
    }
    Some(Plan {
        id: m.id,
        class,
        look: m.look,
        tick: update_tick.saturating_sub(u64::from(m.age)),
        driver: DriverParams {
            accel,
            decel,
            lateral,
            headway,
            gap,
        },
        route,
        base: m.base,
        s: m.s,
        v: m.v,
        lane: m.lane.min(15) as u8,
        motion: motion_from_wire(m.motion, m.accel, m.target),
        shift: Shift {
            at: m.shift_at,
            to: m.shift_to,
            rate: m.shift_rate,
        },
    })
}

/// A change for an update at `update_tick`.
pub fn change_to_wire(c: &PlanChange, update_tick: Tick) -> v1::TrafficChange {
    let (motion, accel, target) = motion_to_wire(c.motion);
    v1::TrafficChange {
        id: c.id,
        age: age(update_tick, c.tick),
        step: c.step,
        s: c.s,
        v: c.v,
        lane: u32::from(c.lane),
        motion,
        accel,
        target,
        extend: (!c.extend.is_empty()).then(|| route_to_wire(&c.extend)),
        shift_at: c.shift.at,
        shift_to: c.shift.to,
        shift_rate: c.shift.rate,
    }
}

/// A received change (`None`: a malformed route extension).
pub fn change_from_wire(m: &v1::TrafficChange, update_tick: Tick) -> Option<PlanChange> {
    let extend = match &m.extend {
        Some(r) => route_from_wire(r)?,
        None => Vec::new(),
    };
    Some(PlanChange {
        id: m.id,
        tick: update_tick.saturating_sub(u64::from(m.age)),
        step: m.step,
        s: m.s,
        v: m.v,
        lane: m.lane.min(15) as u8,
        motion: motion_from_wire(m.motion, m.accel, m.target),
        extend,
        shift: Shift {
            at: m.shift_at,
            to: m.shift_to,
            rate: m.shift_rate,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_server, encode, server_message};

    fn plan() -> Plan {
        Plan {
            id: 42,
            class: VehicleClass::Lorry,
            look: 0xDEAD_BEEF,
            tick: 990,
            driver: DriverParams {
                accel: 0.7 * 1.000_000_1,
                decel: 1.5,
                lateral: 1.8 / 3.0,
                headway: 1.234_567_890_123,
                gap: 2.0,
            },
            route: vec![
                Step {
                    edge: 7,
                    forward: true,
                    speed: 13.888_889,
                    turn_r: 10_000.0,
                },
                Step {
                    edge: 1234,
                    forward: false,
                    speed: 8.333_333,
                    turn_r: 12.5,
                },
            ],
            base: 3,
            s: 17.25,
            v: 12.1,
            lane: 1,
            motion: Motion::Accel {
                a: -1.25,
                target: 4.5,
            },
            shift: Shift {
                at: 0.4,
                to: 1.75,
                rate: 0.6,
            },
        }
    }

    #[test]
    fn plans_and_changes_round_trip_bit_for_bit() {
        let p = plan();
        let u = v1::TrafficUpdate {
            tick: 1000,
            spawns: vec![spawn_to_wire(&p, 1000)],
            changes: vec![change_to_wire(
                &PlanChange {
                    id: 42,
                    tick: 995,
                    step: 4,
                    s: 1.5,
                    v: 0.0,
                    lane: 0,
                    motion: Motion::Stopped,
                    extend: p.route.clone(),
                    shift: Shift {
                        at: -0.3,
                        to: 0.0,
                        rate: 0.5,
                    },
                },
                1000,
            )],
            despawns: vec![5, 6],
            reset: true,
            step_ms: 20,
            graph_hash: 0x1234_5678_9ABC_DEF0,
            server_tick: 77,
            cues: vec![v1::TrafficCue {
                vehicle: 42,
                kind: v1::TrafficCueKind::Honk as i32,
                seconds: 0.0,
            }],
        };
        let bytes = encode(&server_message(v1::server_message::Msg::Traffic(u)));
        let Ok(v1::server_message::Msg::Traffic(back)) = decode_server(&bytes) else {
            panic!("a traffic update");
        };
        let q = spawn_from_wire(&back.spawns[0], back.tick).unwrap();
        assert_eq!(q, p, "the plan keeps every bit");
        assert_eq!(q.driver.headway.to_bits(), p.driver.headway.to_bits());
        let c = change_from_wire(&back.changes[0], back.tick).unwrap();
        assert_eq!((c.tick, c.step, c.motion), (995, 4, Motion::Stopped));
        assert_eq!(c.extend, p.route);
        assert_eq!((c.shift.at, c.shift.to, c.shift.rate), (-0.3, 0.0, 0.5));
        assert_eq!(back.despawns, vec![5, 6]);
        assert!(back.reset);
        assert_eq!(back.graph_hash, 0x1234_5678_9ABC_DEF0);
    }

    #[test]
    fn malformed_plans_are_dropped() {
        let p = plan();
        let mut m = spawn_to_wire(&p, 1000);
        m.class = 99;
        assert!(spawn_from_wire(&m, 1000).is_none(), "unknown class");
        let mut m = spawn_to_wire(&p, 1000);
        m.driver.pop();
        assert!(spawn_from_wire(&m, 1000).is_none(), "driver incomplete");
        let mut m = spawn_to_wire(&p, 1000);
        m.route.as_mut().unwrap().speeds.pop();
        assert!(
            spawn_from_wire(&m, 1000).is_none(),
            "arrays of other lengths"
        );
        let mut m = spawn_to_wire(&p, 1000);
        m.route = None;
        assert!(spawn_from_wire(&m, 1000).is_none(), "no route");
    }

    /// The size budget: about 85 bytes for the vehicle (the driver's five
    /// doubles are 42 of them) and 11 bytes a route step.
    #[test]
    fn a_spawn_is_small() {
        let mut p = plan();
        let two = prost::Message::encoded_len(&spawn_to_wire(&p, 1000));
        assert!(two < 135, "{two} bytes");
        let step = p.route[1];
        p.route.extend(std::iter::repeat_n(step, 10));
        let twelve = prost::Message::encoded_len(&spawn_to_wire(&p, 1000));
        assert!(
            twelve - two <= 10 * 12,
            "{} bytes a step",
            (twelve - two) / 10
        );
    }
}

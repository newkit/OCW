//! The ride photographer's shot on the wire (spec §2.11, v4.38):
//! `ocw_world::photo_share::SharedShot` ↔ `v1::PhotoShot`. The numbers go
//! as doubles, so every client computes the same drone flight from them.

use ocw_world::photo::{CameraPose, Carrier, PoseKind};
use ocw_world::photo_drone::{FlightPlan, Revision};
use ocw_world::photo_share::SharedShot;

use crate::v1;

const KINDS: [PoseKind; 6] = [
    PoseKind::AheadLow,
    PoseKind::Side,
    PoseKind::Behind,
    PoseKind::Wide,
    PoseKind::BehindAbove,
    PoseKind::AerialWide,
];

fn vec3(p: [f64; 3]) -> Option<v1::Vec3d> {
    Some(v1::Vec3d {
        x: p[0],
        y: p[1],
        z: p[2],
    })
}

fn arr(v: Option<&v1::Vec3d>) -> Option<[f64; 3]> {
    v.map(|v| [v.x, v.y, v.z])
}

/// The shot as sent.
pub fn photo_to_wire(s: &SharedShot) -> v1::PhotoShot {
    let p = &s.pose;
    let (carrier, pilot) = match p.carrier {
        Carrier::Person { crouch: false } => (v1::PhotoCarrier::Standing, None),
        Carrier::Person { crouch: true } => (v1::PhotoCarrier::Crouching, None),
        Carrier::Drone { pilot } => (v1::PhotoCarrier::Drone, vec3(pilot)),
    };
    v1::PhotoShot {
        rider_id: s.rider,
        shot: s.shot,
        carrier: carrier as i32,
        kind: KINDS.iter().position(|k| *k == p.kind).unwrap_or(0) as u32,
        eye: vec3(p.eye),
        look: vec3(p.look),
        feet: vec3(p.feet),
        pilot,
        seed: s.seed,
        appear_at_s: s.appear_at,
        planned_at_s: s.planned_at,
        taken_at_s: s.taken_at,
        gone_at_s: s.gone_at,
        aborted_at_s: s.aborted_at,
        flight: s.flight.as_ref().map(|f| v1::DroneFlightPlan {
            duration_s: f.duration_s,
            launch_at_s: f.launch_at,
            path_step_m: f.path_step_m,
            path_xyz: f.path.iter().flatten().copied().collect(),
            revisions: f
                .revisions
                .iter()
                .flat_map(|r| [r.at, r.to_go_m, r.shot_at])
                .collect(),
            abort_at_s: f.abort_at,
        }),
    }
}

/// The shot as received (`None`: incomplete).
pub fn photo_from_wire(m: &v1::PhotoShot) -> Option<SharedShot> {
    let carrier = match v1::PhotoCarrier::try_from(m.carrier).ok()? {
        v1::PhotoCarrier::Standing => Carrier::Person { crouch: false },
        v1::PhotoCarrier::Crouching => Carrier::Person { crouch: true },
        v1::PhotoCarrier::Drone => Carrier::Drone {
            pilot: arr(m.pilot.as_ref())?,
        },
        v1::PhotoCarrier::Unspecified => return None,
    };
    let pose = CameraPose {
        kind: *KINDS.get(m.kind as usize)?,
        carrier,
        feet: arr(m.feet.as_ref())?,
        eye: arr(m.eye.as_ref())?,
        look: arr(m.look.as_ref())?,
    };
    let flight = match (&pose.carrier, &m.flight) {
        (Carrier::Drone { pilot }, Some(f)) => {
            if !f.path_xyz.len().is_multiple_of(3) || !f.revisions.len().is_multiple_of(3) {
                return None;
            }
            Some(FlightPlan {
                pose,
                pilot: *pilot,
                duration_s: f.duration_s,
                launch_at: f.launch_at_s,
                path_step_m: f.path_step_m,
                path: f.path_xyz.as_chunks::<3>().0.to_vec(),
                revisions: f
                    .revisions
                    .as_chunks::<3>()
                    .0
                    .iter()
                    .map(|c| Revision {
                        at: c[0],
                        to_go_m: c[1],
                        shot_at: c[2],
                    })
                    .collect(),
                abort_at: f.abort_at_s,
            })
        }
        (Carrier::Drone { .. }, None) => return None,
        _ => None,
    };
    Some(SharedShot {
        rider: m.rider_id,
        shot: m.shot,
        pose,
        seed: m.seed,
        appear_at: m.appear_at_s,
        planned_at: m.planned_at_s,
        taken_at: m.taken_at_s,
        gone_at: m.gone_at_s,
        aborted_at: m.aborted_at_s,
        flight,
    })
}

/// The client's message announcing or updating a shot.
pub fn photo_message(s: &SharedShot) -> v1::ClientMessage {
    crate::client_message(v1::client_message::Msg::Photo(photo_to_wire(s)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{decode_client, decode_server, encode, server_message};

    fn drone_shot() -> SharedShot {
        let pose = CameraPose {
            kind: PoseKind::AerialWide,
            carrier: Carrier::Drone {
                pilot: [8.0, 100.0, -20.0],
            },
            feet: [30.0, 100.0, 5.0],
            eye: [30.0, 125.123_456_789, 5.0],
            look: [-0.8, -0.6, 0.0],
        };
        let mut f = FlightPlan::new(pose, [8.0, 100.0, -20.0], 47.5);
        f.launch_at = Some(1234.56);
        f.path_step_m = 7.25;
        f.path = vec![[0.6, 100.0, 0.0], [0.6, 100.5, 7.25], [0.6, 101.0, 14.5]];
        f.revisions = vec![
            Revision {
                at: 1234.56,
                to_go_m: 380.0,
                shot_at: 1282.06,
            },
            Revision {
                at: 1240.0,
                to_go_m: 330.1,
                shot_at: 1283.2,
            },
        ];
        SharedShot {
            rider: 0,
            shot: 3,
            pose,
            seed: 0xDEAD_BEEF_1234,
            appear_at: 1231.56,
            planned_at: 1282.06,
            taken_at: Some(1283.2),
            gone_at: None,
            aborted_at: None,
            flight: Some(f),
        }
    }

    /// The shot goes to the server and back unchanged, to the bit.
    #[test]
    fn a_shot_round_trips() {
        let s = drone_shot();
        let bytes = encode(&photo_message(&s));
        let Ok(v1::client_message::Msg::Photo(m)) = decode_client(&bytes) else {
            panic!("a photo message");
        };
        assert_eq!(photo_from_wire(&m).as_ref(), Some(&s));
        // Relayed with the rider set by the server.
        let relayed = v1::PhotoShot { rider_id: 4, ..m };
        let bytes = encode(&server_message(v1::server_message::Msg::Photo(relayed)));
        let Ok(v1::server_message::Msg::Photo(m)) = decode_server(&bytes) else {
            panic!("a photo message");
        };
        let back = photo_from_wire(&m).unwrap();
        assert_eq!(back.rider, 4);
        assert_eq!(SharedShot { rider: 0, ..back }, s);
        // A person's shot (crouching), no flight.
        let mut p = s.clone();
        p.pose.carrier = Carrier::Person { crouch: true };
        p.pose.kind = PoseKind::AheadLow;
        p.flight = None;
        assert_eq!(photo_from_wire(&photo_to_wire(&p)), Some(p));
    }

    #[test]
    fn an_incomplete_shot_is_dropped() {
        let mut m = photo_to_wire(&drone_shot());
        m.flight = None;
        assert_eq!(photo_from_wire(&m), None, "a drone without its flight");
        let mut m = photo_to_wire(&drone_shot());
        m.eye = None;
        assert_eq!(photo_from_wire(&m), None);
        let mut m = photo_to_wire(&drone_shot());
        m.carrier = 0;
        assert_eq!(photo_from_wire(&m), None);
    }
}

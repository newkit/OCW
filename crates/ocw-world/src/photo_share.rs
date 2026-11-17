//! The ride photographer's shot as the other riders see it (spec §2.11,
//! §2.8.1 Q5, v4.38: "it's happening in the world and other riders are in
//! the photo as well, so they should also see it"). The photographed
//! rider's client announces the shot once the photographer shows (the
//! person appears, the drone takes off) and updates it (the drone's new
//! timings, the photo, the end); the server checks it ([`ShotLog`]) and
//! relays it to everyone; every client draws the same photographer at the
//! same game time (the server's tick time, `tick / 50`), the drone from
//! its [`FlightPlan`]. Only the photographed rider's client saves the photo.

use std::collections::BTreeMap;

use crate::photo::{CameraPose, Carrier};
use crate::photo_drone::{FlightPlan, MAX_PATH_POINTS, MAX_REVISIONS};

/// The photographer's limits (the user's: ≤ 5 photos per ride, ≥ 20
/// minutes apart; `ocw_client::photographer::Limits`).
pub const MAX_PHOTOS: usize = 5;
pub const GAP_S: f64 = 20.0 * 60.0;
/// Shots announced per ride at most, aborted ones included.
pub const MAX_ANNOUNCED: usize = 40;
/// Updates of one shot: at least this far apart (s; the photo, the end and
/// an abort pass at once) and at most this many.
pub const MIN_UPDATE_S: f64 = 0.25;
pub const MAX_UPDATES: u32 = 600;
/// The camera is at most this far from the rider when the photo is due now
/// (m, horizontal), plus [`REACH_MPS`] for every second until it is due;
/// never farther than [`MAX_REACH_M`].
pub const NEAR_RIDER_M: f64 = 400.0;
pub const REACH_MPS: f64 = 25.0;
pub const MAX_REACH_M: f64 = 8000.0;
/// The photo is due at most this far ahead (s).
pub const MAX_LEAD_S: f64 = 300.0;
/// Times may lag the server's by this much (s: the trip to the server).
pub const SLACK_S: f64 = 5.0;
/// Others draw a shot whose camera is at most this far from their rider
/// (m, horizontal).
pub const RELEVANT_M: f64 = 3000.0;
/// A shot is forgotten this long after it ended, or after it was due when
/// no end comes (s).
pub const FORGET_S: f64 = 90.0;

/// One shot of a rider's photographer, as shared.
#[derive(Debug, Clone, PartialEq)]
pub struct SharedShot {
    /// The photographed rider (set by the server).
    pub rider: u32,
    /// The shot's number from that rider's client (unique per connection).
    pub shot: u32,
    pub pose: CameraPose,
    /// The look of the photographer (clothes, hair).
    pub seed: u64,
    /// Game time the person (or the drone's pilot) shows.
    pub appear_at: f64,
    /// Game time the photo is due (the drone: its first timing's).
    pub planned_at: f64,
    /// Game time of the photo (the flash and the shutter), once taken.
    pub taken_at: Option<f64>,
    /// Game time the photographer left.
    pub gone_at: Option<f64>,
    /// Game time the shot was given up (the rider left the path).
    pub aborted_at: Option<f64>,
    /// The drone's flight (a drone's shot only).
    pub flight: Option<FlightPlan>,
}

impl SharedShot {
    /// When the shot ended (the photographer left or gave up).
    pub fn ended(&self) -> Option<f64> {
        match (self.gone_at, self.aborted_at) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Whether others may forget it at game time `t`.
    pub fn expired(&self, t: f64) -> bool {
        let last = self.ended().unwrap_or_else(|| {
            self.taken_at
                .unwrap_or(self.planned_at)
                .max(self.planned_at)
        });
        t > last + FORGET_S
    }

    /// Horizontal distance of the camera from `p` (m).
    pub fn distance_to(&self, p: [f64; 3]) -> f64 {
        let e = self.pose.eye;
        ((e[0] - p[0]).powi(2) + (e[2] - p[2]).powi(2)).sqrt()
    }

    fn numbers(&self) -> Vec<f64> {
        let mut v: Vec<f64> = self
            .pose
            .eye
            .iter()
            .chain(&self.pose.look)
            .chain(&self.pose.feet)
            .copied()
            .collect();
        if let Carrier::Drone { pilot } = self.pose.carrier {
            v.extend(pilot);
        }
        v.extend([self.appear_at, self.planned_at]);
        v.extend(self.taken_at);
        v.extend(self.gone_at);
        v.extend(self.aborted_at);
        if let Some(f) = &self.flight {
            v.extend([f.duration_s, f.path_step_m]);
            v.extend(f.launch_at);
            v.extend(f.abort_at);
            v.extend(f.path.iter().flatten());
            v.extend(
                f.revisions
                    .iter()
                    .flat_map(|r| [r.at, r.to_go_m, r.shot_at]),
            );
        }
        v
    }
}

/// The server's rules (spec §2.11): the photographer's limits; a server for
/// tests (dev commands allowed) leaves out the 20 minutes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    pub max_photos: usize,
    pub gap_s: f64,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            max_photos: MAX_PHOTOS,
            gap_s: GAP_S,
        }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    shot: SharedShot,
    last_update: f64,
    updates: u32,
}

/// One rider's shots on the server: what was announced and taken, and the
/// checks of every announcement and update.
#[derive(Debug, Clone, Default)]
pub struct ShotLog {
    taken: Vec<f64>,
    announced: usize,
    shots: BTreeMap<u32, Entry>,
}

/// The checks of one shot on its own (numbers, pose, times, distance).
pub fn check(shot: &SharedShot, now: f64, rider: [f64; 3]) -> Result<(), String> {
    if shot.numbers().iter().any(|x| !x.is_finite()) {
        return Err("not a number".into());
    }
    let p = &shot.pose;
    let l = p.look;
    if ((l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt() - 1.0).abs() > 0.01 {
        return Err("the view is not a direction".into());
    }
    let d3 = |a: [f64; 3], b: [f64; 3]| {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
    };
    if d3(p.eye, p.feet) > 200.0 {
        return Err("the camera is far from its feet".into());
    }
    match (&p.carrier, &shot.flight) {
        (Carrier::Person { .. }, None) => {}
        (Carrier::Drone { pilot }, Some(f)) => {
            if d3(*pilot, p.eye) > 400.0 {
                return Err("the pilot is far from the drone".into());
            }
            if f.pose != *p || f.pilot != *pilot {
                return Err("the flight is not the shot's".into());
            }
            if f.path.len() > MAX_PATH_POINTS + 2 || f.revisions.len() > MAX_REVISIONS {
                return Err("the flight is too long".into());
            }
            if !(1.0..=200.0).contains(&f.path_step_m)
                || !(10.0..=180.0).contains(&f.duration_s)
                || f.launch_at.is_some_and(|l| l > now + SLACK_S)
            {
                return Err("an implausible flight".into());
            }
        }
        _ => return Err("a person with a flight or a drone without".into()),
    }
    if shot.planned_at < now - MAX_LEAD_S || shot.planned_at > now + MAX_LEAD_S {
        return Err("the photo is not due now".into());
    }
    if shot.appear_at > shot.planned_at + 1.0 || shot.appear_at > now + SLACK_S {
        return Err("it shows after its photo".into());
    }
    let lead = (shot.planned_at - now).max(0.0);
    let reach = (NEAR_RIDER_M + REACH_MPS * lead).min(MAX_REACH_M);
    if shot.distance_to(rider) > reach {
        return Err(format!(
            "the camera is {:.0} m from the rider (at most {reach:.0} m)",
            shot.distance_to(rider)
        ));
    }
    Ok(())
}

impl ShotLog {
    /// Checks an announcement or update of a shot at server game time
    /// `now` with the rider at `rider`; `Ok` keeps it (to relay).
    pub fn accept(
        &mut self,
        shot: &SharedShot,
        now: f64,
        rider: [f64; 3],
        rules: &Rules,
    ) -> Result<(), String> {
        check(shot, now, rider)?;
        let newly_taken = |old: Option<&SharedShot>| {
            shot.taken_at.is_some() && old.is_none_or(|o| o.taken_at.is_none())
        };
        let taken_ok = |taken: &[f64], at: f64| {
            if !(now - SLACK_S..=now + 1.0).contains(&at) {
                return Err("the photo is not now".to_string());
            }
            if taken.len() >= rules.max_photos {
                return Err(format!("{} photos in this ride already", taken.len()));
            }
            if let Some(last) = taken.last()
                && at - last < rules.gap_s
            {
                return Err(format!(
                    "{:.0} s after the last photo (at least {:.0} s)",
                    at - last,
                    rules.gap_s
                ));
            }
            Ok(())
        };
        match self.shots.get(&shot.shot) {
            None => {
                if self.announced >= MAX_ANNOUNCED {
                    return Err("too many shots in this ride".into());
                }
                if self.taken.len() >= rules.max_photos {
                    return Err(format!("{} photos in this ride already", self.taken.len()));
                }
                if let Some(last) = self.taken.last()
                    && shot.planned_at - last < rules.gap_s - SLACK_S
                {
                    return Err("less than the photographer's gap after the last photo".into());
                }
                if newly_taken(None) {
                    taken_ok(&self.taken, shot.taken_at.expect("taken"))?;
                }
                // A new shot ends the rider's open ones (a new ride, a
                // lost end): every client does the same.
                for e in self.shots.values_mut() {
                    if e.shot.ended().is_none() {
                        e.shot.aborted_at = Some(now);
                    }
                }
                self.announced += 1;
                if let Some(t) = shot.taken_at {
                    self.taken.push(t);
                }
                self.shots.insert(
                    shot.shot,
                    Entry {
                        shot: shot.clone(),
                        last_update: now,
                        updates: 0,
                    },
                );
                Ok(())
            }
            Some(e) => {
                let old = &e.shot;
                if old.ended().is_some() {
                    return Err("the shot has ended".into());
                }
                if old.pose != shot.pose || old.seed != shot.seed || old.rider != shot.rider {
                    return Err("the shot's place changed".into());
                }
                if old.taken_at.is_some() && old.taken_at != shot.taken_at {
                    return Err("the photo's time changed".into());
                }
                if let (Some(a), Some(b)) = (&old.flight, &shot.flight) {
                    let launched = a.launch_at.is_some();
                    if (launched && (a.launch_at != b.launch_at || a.path != b.path))
                        || b.revisions.len() < a.revisions.len()
                        || b.revisions[..a.revisions.len()] != a.revisions[..]
                    {
                        return Err("the flight's past changed".into());
                    }
                }
                let news = newly_taken(Some(old))
                    || (old.gone_at.is_none() && shot.gone_at.is_some())
                    || (old.aborted_at.is_none() && shot.aborted_at.is_some());
                if !news && now - e.last_update < MIN_UPDATE_S {
                    return Err("updates too often".into());
                }
                if e.updates >= MAX_UPDATES {
                    return Err("too many updates".into());
                }
                if newly_taken(Some(old)) {
                    taken_ok(&self.taken, shot.taken_at.expect("taken"))?;
                    self.taken.push(shot.taken_at.expect("taken"));
                }
                let e = self.shots.get_mut(&shot.shot).expect("known");
                e.shot = shot.clone();
                e.last_update = now;
                e.updates += 1;
                Ok(())
            }
        }
    }

    /// The photos taken (game times).
    pub fn taken(&self) -> &[f64] {
        &self.taken
    }

    /// The shots others may still draw (a client joining gets them).
    pub fn active(&self, now: f64) -> impl Iterator<Item = &SharedShot> {
        self.shots
            .values()
            .map(|e| &e.shot)
            .filter(move |s| s.ended().is_none() && !s.expired(now))
    }

    /// Forgets the shots long over (the counts stay).
    pub fn prune(&mut self, now: f64) {
        self.shots.retain(|_, e| !e.shot.expired(now));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::photo::PoseKind;

    fn person(shot: u32, at: f64) -> SharedShot {
        SharedShot {
            rider: 1,
            shot,
            pose: CameraPose {
                kind: PoseKind::Side,
                carrier: Carrier::Person { crouch: false },
                feet: [10.0, 100.0, -200.0],
                eye: [10.0, 101.6, -200.0],
                look: [-1.0, 0.0, 0.0],
            },
            seed: 7,
            appear_at: 0.0,
            planned_at: at,
            taken_at: None,
            gone_at: None,
            aborted_at: None,
            flight: None,
        }
    }

    const RIDER: [f64; 3] = [0.0, 100.0, -100.0];

    #[test]
    fn at_most_five_photos_twenty_minutes_apart() {
        let mut log = ShotLog::default();
        let r = Rules::default();
        let mut t = 100.0;
        for n in 1..=7u32 {
            let mut s = person(n, t + 10.0);
            let announced = log.accept(&s, t, RIDER, &r);
            if n <= 5 {
                announced.unwrap();
                s.taken_at = Some(t + 10.0);
                log.accept(&s, t + 10.0, RIDER, &r).unwrap();
                s.gone_at = Some(t + 20.0);
                log.accept(&s, t + 20.0, RIDER, &r).unwrap();
            } else {
                assert!(announced.is_err(), "a sixth photo");
            }
            t += GAP_S + 30.0;
        }
        assert_eq!(log.taken().len(), 5);
        // Two photos within 20 minutes: the second is refused.
        let mut log = ShotLog::default();
        let mut a = person(1, 110.0);
        log.accept(&a, 100.0, RIDER, &r).unwrap();
        a.taken_at = Some(110.0);
        log.accept(&a, 110.0, RIDER, &r).unwrap();
        let b = person(2, 110.0 + 600.0);
        assert!(log.accept(&b, 700.0, RIDER, &r).is_err());
        // A test server (no gap): allowed, the five stay.
        let dev = Rules {
            gap_s: 0.0,
            ..Rules::default()
        };
        log.accept(&b, 700.0, RIDER, &dev).unwrap();
    }

    #[test]
    fn a_shot_far_from_the_rider_or_not_due_is_refused() {
        let mut log = ShotLog::default();
        let r = Rules::default();
        let far = [5000.0, 100.0, -100.0];
        assert!(log.accept(&person(1, 110.0), 100.0, far, &r).is_err());
        // Due in 2 min, 1.5 km away: the rider gets there.
        let mut s = person(2, 220.0);
        s.pose.eye[0] = 1500.0;
        s.pose.feet[0] = 1500.0;
        log.accept(&s, 100.0, RIDER, &r).unwrap();
        // An hour ahead.
        assert!(log.accept(&person(3, 4000.0), 100.0, RIDER, &r).is_err());
        let mut nan = person(4, 110.0);
        nan.pose.eye[1] = f64::NAN;
        assert!(log.accept(&nan, 100.0, RIDER, &r).is_err());
    }

    #[test]
    fn updates_keep_the_place_and_come_not_too_often() {
        let mut log = ShotLog::default();
        let r = Rules::default();
        let mut s = person(1, 120.0);
        log.accept(&s, 100.0, RIDER, &r).unwrap();
        // The same again at once: too often.
        assert!(log.accept(&s, 100.1, RIDER, &r).is_err());
        log.accept(&s, 100.5, RIDER, &r).unwrap();
        let mut moved = s.clone();
        moved.pose.eye[0] += 3.0;
        moved.pose.feet[0] += 3.0;
        assert!(log.accept(&moved, 101.0, RIDER, &r).is_err());
        // The photo passes at once, then its time stays.
        s.taken_at = Some(120.0);
        log.accept(&s, 120.05, RIDER, &r).unwrap();
        let mut later = s.clone();
        later.taken_at = Some(121.0);
        assert!(log.accept(&later, 121.0, RIDER, &r).is_err());
        s.gone_at = Some(130.0);
        log.accept(&s, 130.0, RIDER, &r).unwrap();
        assert!(log.accept(&s, 131.0, RIDER, &r).is_err(), "ended");
        assert_eq!(log.active(131.0).count(), 0);
    }

    #[test]
    fn a_new_shot_ends_the_open_one() {
        let mut log = ShotLog::default();
        let r = Rules::default();
        log.accept(&person(1, 120.0), 100.0, RIDER, &r).unwrap();
        log.accept(&person(2, 130.0), 110.0, RIDER, &r).unwrap();
        let active: Vec<u32> = log.active(110.0).map(|s| s.shot).collect();
        assert_eq!(active, vec![2]);
        log.prune(10_000.0);
        assert_eq!(log.active(10_000.0).count(), 0);
    }
}

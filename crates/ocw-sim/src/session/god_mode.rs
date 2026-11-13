//! God mode in the session (spec §14 Phase 13E): the server is the truth
//! for who flies and where a rider re-enters the road. A rider in god
//! mode is out of the physics (no speed along the path, no drafting, not
//! moved by the field), the road position is frozen where the flight
//! began; the unicorn's pose comes from the client and is checked here
//! ([`crate::god::check_pose`]). Leaving lands the rider on the nearest
//! road below the unicorn (free ride), the nearest point of the route
//! (fixed routes, routes on the road graph), or back where they took off.

use super::*;
use crate::god::{self, Bounds, GodPose, PoseError};

/// The area god mode may fly over: the road graph's or route's box grown
/// by this much (the world set's packages reach beyond their roads, and
/// neighbours load around them, spec §4.7).
pub const AREA_GROWTH_M: f64 = 6_000.0;

/// A rider in god mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GodRider {
    /// The road state when the flight began (restored by flying back).
    pub frozen: RiderState,
    /// The last accepted pose of the unicorn.
    pub pose: GodPose,
    /// The tick it was accepted at.
    pub pose_tick: u64,
    /// Poses refused so far (implausible).
    pub refused: u32,
}

/// How a rider leaves god mode.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GodLeave {
    /// Land on the road nearest the point (x, z) below the unicorn.
    Land { x: f64, z: f64 },
    /// Back to where the flight began (Shift + G).
    FlyBack,
}

pub(super) fn route_area(route: &Route) -> Option<Bounds> {
    Bounds::around(route.points().iter().copied()).map(|b| b.grown(AREA_GROWTH_M))
}

pub(super) fn network_area(net: &RoadNetwork) -> Option<Bounds> {
    let n = net.graph().edges.len();
    Bounds::around((0..n).flat_map(|i| net.geometry(i).points.iter().copied()))
        .map(|b| b.grown(AREA_GROWTH_M))
}

impl Session {
    /// Whether the rider is in god mode.
    pub fn is_god(&self, id: RiderId) -> bool {
        self.riders.get(&id).is_some_and(|r| r.god.is_some())
    }

    /// The rider's god-mode state, if flying.
    pub fn god(&self, id: RiderId) -> Option<GodRider> {
        self.riders.get(&id).and_then(|r| r.god)
    }

    /// The area poses are checked against.
    pub fn god_area(&self) -> Option<Bounds> {
        self.god_area
    }

    /// Why the rider may not enter god mode now, if not: in a running
    /// event (from the countdown to the rider's finish, spec §14 Phase
    /// 13A); before and after it is fine.
    pub fn god_refusal(&self, id: RiderId) -> Option<&'static str> {
        if !self.riders.contains_key(&id) {
            return Some("no such rider");
        }
        let ev = self.event.as_ref()?;
        let live = matches!(ev.phase, event::Phase::Countdown | event::Phase::Running);
        let riding = ev
            .entrant(id)
            .is_some_and(|e| e.finish_s.is_none() && !e.dnf);
        (live && riding).then_some("not during a running event (after your finish it is)")
    }

    /// Puts the rider into god mode: the unicorn starts above the rider
    /// ([`god::TAKE_OFF_RISE_M`]), looking along the rider's direction.
    /// Already flying: the current pose.
    pub fn enter_god(&mut self, id: RiderId) -> Result<GodPose, String> {
        if let Some(g) = self.god(id) {
            return Ok(g.pose);
        }
        if let Some(why) = self.god_refusal(id) {
            return Err(why.into());
        }
        let state = self.rider_state(id).ok_or("no such rider")?;
        let p = self.track(id).sample(state.distance);
        let at = p.offset_position(state.lateral_offset);
        let dir = p.direction;
        // The rider's heading (positive = right) turns the road direction.
        let (s, c) = state.heading.sin_cos();
        let heading = [dir[0] * c - dir[1] * s, dir[0] * s + dir[1] * c];
        let pose = GodPose {
            pos: [at[0], at[1] + god::TAKE_OFF_RISE_M, at[2]],
            yaw: god::yaw_of(heading),
            pitch: -0.12,
            speed: 0.0,
        };
        let tick = self.tick;
        let r = self.riders.get_mut(&id).ok_or("no such rider")?;
        r.god = Some(GodRider {
            frozen: r.state,
            pose,
            pose_tick: tick,
            refused: 0,
        });
        r.state.speed = 0.0;
        r.state.turn_curvature = 0.0;
        Ok(pose)
    }

    /// A pose of the rider's unicorn from the client: accepted if
    /// plausible ([`god::check_pose`]); `teleport` (dev command, local
    /// server only – the caller checks) skips the speed check.
    pub fn set_god_pose(
        &mut self,
        id: RiderId,
        pose: GodPose,
        teleport: bool,
    ) -> Result<(), PoseError> {
        let area = self.god_area;
        let tick = self.tick;
        let Some(g) = self.riders.get_mut(&id).and_then(|r| r.god.as_mut()) else {
            return Err(PoseError::NotFinite);
        };
        let dt = (tick.saturating_sub(g.pose_tick)) as f64 * SIM_DT;
        let prev = (!teleport).then_some(g.pose.pos);
        match god::check_pose(prev, dt, &pose, area) {
            Ok(()) => {
                g.pose = pose;
                g.pose_tick = tick;
                Ok(())
            }
            Err(e) => {
                g.refused += 1;
                Err(e)
            }
        }
    }

    /// Ends god mode: the rider stands (speed 0) on the road below the
    /// unicorn or where the flight began. Returns the road point (local
    /// metres) the rider stands on.
    pub fn leave_god(&mut self, id: RiderId, how: GodLeave) -> Result<[f64; 3], String> {
        let g = self
            .riders
            .get_mut(&id)
            .ok_or("no such rider")?
            .god
            .take()
            .ok_or("not in god mode")?;
        match how {
            GodLeave::FlyBack => {
                let r = self.riders.get_mut(&id).expect("checked");
                r.state = RiderState {
                    speed: 0.0,
                    turn_curvature: 0.0,
                    ..g.frozen
                };
            }
            GodLeave::Land { x, z } => self.land(id, x, z, g.frozen),
        }
        let st = self.rider_state(id).expect("checked");
        Ok(self
            .track(id)
            .sample(st.distance)
            .offset_position(st.lateral_offset))
    }

    /// Places the rider standing on the road nearest (x, z).
    fn land(&mut self, id: RiderId, x: f64, z: f64, frozen: RiderState) {
        let standing = |distance: f64| RiderState {
            distance,
            ..Default::default()
        };
        let graph_route = match (&self.free, &self.plan) {
            (Some((net, _)), Some(plan)) => Some(Some((net.clone(), plan.clone()))),
            (Some(_), None) => Some(None),
            (None, _) => None,
        };
        match graph_route {
            Some(Some((net, plan))) => {
                // A route on the road graph: the nearest point of the
                // route; the rider follows it from there.
                let along = god::nearest_on_plan(&net, &plan, [x, z]).map_or(0.0, |n| n.0);
                let d = god::lap_distance(frozen.distance, along, plan.length_m(), plan.is_loop);
                let r = self.riders.get_mut(&id).expect("checked");
                let next = r.path.as_ref().map_or(0, RidePath::next_index);
                let mut p = RidePath::on_route(net, plan, 0.0, next);
                p.extend(d, &mut |j| j.default);
                p.trim_behind(d);
                r.path = Some(p);
                r.path_reset = true;
                r.path_changed = None;
                r.turn = None;
                r.state = standing(d);
                r.steering = Steering::Auto(0.0);
            }
            Some(None) => {
                // Free ride: a new path from the nearest road (the distance
                // goes on from where the flight began: nothing credited).
                self.teleport_to(id, x, z);
                let r = self.riders.get_mut(&id).expect("checked");
                r.state = standing(r.state.distance);
                r.turn = None;
                r.steering = Steering::Auto(0.0);
            }
            None => {
                let (along, length, is_loop) = (
                    god::nearest_on_polyline(self.route.points(), [x, z]).map_or(0.0, |n| n.0),
                    self.route.length(),
                    self.route.is_loop(),
                );
                let d = god::lap_distance(frozen.distance, along, length, is_loop);
                let finish = self.route.finish();
                let r = self.riders.get_mut(&id).expect("checked");
                r.state = standing(finish.map_or(d, |end| d.min(end)));
                r.steering = Steering::Auto(0.0);
            }
        }
        let r = self.riders.get_mut(&id).expect("checked");
        r.wild = false;
        r.scraping = false;
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{free_session, grid_network};
    use super::*;

    fn pedal(s: &mut Session, id: RiderId, watts: f64) {
        s.apply_input(
            id,
            1,
            RiderInput {
                power_w: watts,
                ..Default::default()
            },
        );
    }

    fn ride(s: &mut Session, seconds: f64) {
        for _ in 0..(seconds / SIM_DT).round() as usize {
            s.step();
        }
    }

    #[test]
    fn a_god_rider_is_out_of_the_physics_and_the_road_position_is_frozen() {
        let mut s = Session::new(Route::test_loop());
        let me = s.add_rider("me", RiderProfile::default());
        let other = s.add_rider("other", RiderProfile::default());
        pedal(&mut s, me, 250.0);
        pedal(&mut s, other, 250.0);
        ride(&mut s, 10.0);
        let before = s.rider_state(me).unwrap();
        assert!(before.speed > 5.0);
        let pose = s.enter_god(me).unwrap();
        // Above the rider, looking along the road.
        let road = s
            .route()
            .sample(before.distance)
            .offset_position(before.lateral_offset);
        assert!((pose.pos[1] - road[1] - god::TAKE_OFF_RISE_M).abs() < 1e-9);
        assert!(s.is_god(me));
        ride(&mut s, 10.0);
        let st = s.rider_state(me).unwrap();
        assert_eq!(st.distance, before.distance, "frozen");
        assert_eq!(st.speed, 0.0);
        // The snapshot carries the pose.
        let snap = s.snapshot();
        assert_eq!(snap.iter().find(|r| r.id == me).unwrap().god, Some(pose));
        assert!(snap.iter().find(|r| r.id == other).unwrap().god.is_none());
        // Flying back restores the road position, standing.
        let at = s.leave_god(me, GodLeave::FlyBack).unwrap();
        assert!(!s.is_god(me));
        let st = s.rider_state(me).unwrap();
        assert_eq!(st.distance, before.distance);
        assert_eq!(st.speed, 0.0);
        assert!((at[0] - road[0]).abs() < 1e-9);
        assert!(s.leave_god(me, GodLeave::FlyBack).is_err(), "not flying");
    }

    #[test]
    fn nobody_drafts_a_flying_rider() {
        // The rider behind a flying one rides as if the other were far away.
        #[derive(PartialEq)]
        enum Front {
            Far,
            Flying,
            Riding,
        }
        let race = |front: Front| {
            let mut s = Session::new(Route::test_loop());
            let first = s.add_rider("front", RiderProfile::default());
            let back = s.add_rider("back", RiderProfile::default());
            s.teleport(first, if front == Front::Far { 1_500.0 } else { 102.0 });
            if front == Front::Flying {
                s.enter_god(first).unwrap();
            }
            s.teleport(back, 100.0);
            pedal(&mut s, back, 200.0);
            ride(&mut s, 5.0);
            s.rider_state(back).unwrap().speed
        };
        assert_eq!(race(Front::Flying), race(Front::Far));
        assert!(race(Front::Riding) >= race(Front::Far));
    }

    #[test]
    fn landing_on_a_route_is_on_its_nearest_point() {
        let mut s = Session::new(Route::test_loop());
        let me = s.add_rider("me", RiderProfile::default());
        s.teleport(me, 2.0 * Route::test_loop().length() + 10.0);
        s.enter_god(me).unwrap();
        // Over a point 300 m along the loop, 40 m beside it.
        let p = s.route().sample(300.0);
        let r = p.right();
        let x = p.position[0] + r[0] * 40.0;
        let z = p.position[2] + r[1] * 40.0;
        let at = s.leave_god(me, GodLeave::Land { x, z }).unwrap();
        let st = s.rider_state(me).unwrap();
        let lap = Route::test_loop().length();
        assert!(
            (st.distance - (2.0 * lap + 300.0)).abs() < 2.0,
            "{}",
            st.distance
        );
        assert_eq!((st.speed, st.lateral_offset), (0.0, 0.0));
        assert!((at[0] - p.position[0]).abs() < 3.0 && (at[2] - p.position[2]).abs() < 3.0);
    }

    #[test]
    fn landing_in_free_ride_is_on_the_nearest_road() {
        let mut s = free_session();
        let me = s.add_rider("me", RiderProfile::default());
        let d0 = s.rider_state(me).unwrap().distance;
        s.take_path_changes();
        s.enter_god(me).unwrap();
        // Over the grid's middle column (x = 600), between rows 1 and 2.
        let at = s
            .leave_god(me, GodLeave::Land { x: 610.0, z: 450.0 })
            .unwrap();
        assert!(
            (at[0] - 600.0).abs() < 1.0 && (at[2] - 450.0).abs() < 1.0,
            "{at:?}"
        );
        let st = s.rider_state(me).unwrap();
        assert_eq!(st.speed, 0.0);
        assert_eq!(st.distance, d0, "no distance credited");
        // The clients get the new path.
        assert!(
            s.take_path_changes()
                .iter()
                .any(|c| c.rider == me && c.reset)
        );
        ride(&mut s, 1.0);
        assert!(s.rider_state(me).unwrap().speed < 0.5, "standing");
    }

    #[test]
    fn landing_on_a_graph_route_follows_the_route_from_there() {
        let net = grid_network();
        let e = |a: u32, b: u32| {
            let i = (0..net.graph().edges.len())
                .find(|&i| (net.edge(i).from.0, net.edge(i).to.0) == (a, b))
                .unwrap();
            Segment::whole(&net, i, true)
        };
        let plan = RoutePlan::new(&net, "row", vec![e(0, 1), e(1, 2), e(2, 3)], false).unwrap();
        let mut s = Session::route_ride(net.clone(), Arc::new(plan));
        let me = s.add_rider("me", RiderProfile::default());
        s.enter_god(me).unwrap();
        // Over (700, 300): the route runs along z = 0; nearest is 700 m in.
        let at = s
            .leave_god(me, GodLeave::Land { x: 700.0, z: 300.0 })
            .unwrap();
        let st = s.rider_state(me).unwrap();
        assert!((st.distance - 700.0).abs() < 1.0, "{}", st.distance);
        assert!(s.path(me).unwrap().on_route_at(st.distance));
        assert!((at[0] - 700.0).abs() < 1.0 && at[2].abs() < 1.0, "{at:?}");
    }

    #[test]
    fn implausible_poses_are_refused() {
        let mut s = Session::new(Route::test_loop());
        let me = s.add_rider("me", RiderProfile::default());
        let start = s.enter_god(me).unwrap();
        ride(&mut s, 0.1);
        let mut next = start;
        next.pos[0] += 100.0;
        assert_eq!(s.set_god_pose(me, next, false), Ok(()));
        ride(&mut s, 0.1);
        let mut far = next;
        far.pos[0] += 5_000.0;
        assert_eq!(s.set_god_pose(me, far, false), Err(PoseError::TooFast));
        assert_eq!(s.god(me).unwrap().pose, next, "the last good pose stays");
        assert_eq!(s.god(me).unwrap().refused, 1);
        // A teleport (dev) skips the speed check, not the area.
        assert_eq!(s.set_god_pose(me, far, true), Ok(()));
        let mut outside = far;
        outside.pos[2] += 1e6;
        assert_eq!(
            s.set_god_pose(me, outside, true),
            Err(PoseError::OutsideWorld)
        );
    }

    #[test]
    fn refused_in_a_running_event_and_ended_by_its_start() {
        let mut s = Session::new(Route::test_loop());
        let me = s.add_rider("me", RiderProfile::default());
        s.enter_god(me).unwrap();
        // An event's start formation ends god mode …
        s.start_event(crate::EventSpec::GroupRide { target_w: None })
            .unwrap();
        assert!(!s.is_god(me));
        s.step();
        // … and from the countdown on it is not available.
        assert!(s.enter_god(me).is_err());
        s.cancel_event();
        assert!(s.enter_god(me).is_ok());
    }
}

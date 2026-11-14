//! Damage in the session (spec §14 Phase 12E, K4): `ride.damage` and the
//! organiser's rules, every rider's [`Damage`] (taken from hits in
//! [`Session::apply_response`], ticked while riding, its [`Effects`] on
//! the virtual rider in [`Session::step`]), the condition carried over from
//! earlier rides (Phase 12 Q6, from the client's hello), the roadside
//! repair asked by the rider and the staged damage of `dev.damage`.

use super::*;
use crate::damage::{self, Damage, Part, Rules as DamageRules, StopKind, TickCtx, TickOut};

/// What `dev.damage` stages (local server only; screenshots and tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DamageStage {
    /// The rider's condition (0–1).
    Rider(f64),
    /// One part's condition (0–1).
    Part(Part, f64),
    /// A puncture (front or rear).
    Puncture { rear: bool },
    /// A broken chain.
    Chain,
    /// Everything back to new.
    Reset,
}

impl Session {
    /// `ride.damage` of the session (spec §14 Phase 12A; off for a new
    /// session, Q1: off by default).
    pub fn set_damage(&mut self, on: bool) {
        self.rules.damage = on;
    }

    pub fn damage_setting(&self) -> bool {
        self.rules.damage
    }

    /// Damage in force now: only with collisions on (12A), by the
    /// organiser's rule for the current event, else the session's switch.
    pub fn damage_on(&self) -> bool {
        if !self.collision_mode().on() {
            return false;
        }
        self.event_damage_rules().on.unwrap_or(self.rules.damage)
    }

    /// The organiser's damage rules of the current event (none: defaults).
    pub fn event_damage_rules(&self) -> DamageRules {
        self.event
            .as_ref()
            .filter(|e| e.phase != event::Phase::Finished)
            .map(|e| e.damage)
            .unwrap_or_default()
    }

    /// The organiser's damage rules for the current event.
    pub fn set_event_damage(&mut self, rules: DamageRules) -> bool {
        match &mut self.event {
            Some(e) => {
                e.damage = rules;
                true
            }
            None => false,
        }
    }

    /// A rider's damage (`None`: unknown rider).
    pub fn damage(&self, id: RiderId) -> Option<&Damage> {
        self.riders.get(&id).map(|r| &r.damage)
    }

    /// The condition a client rider brings from earlier rides (Phase 12
    /// Q6, the hello): its injury and the bike's parts. A stop, crash
    /// counts and the last hit start afresh.
    pub fn carry_damage(&mut self, id: RiderId, rider: f64, parts: [f64; damage::PARTS]) -> bool {
        match self.riders.get_mut(&id) {
            Some(r) if r.control == Control::Client => {
                r.damage = Damage::carried(rider, parts);
                r.damage_changed = true;
                true
            }
            _ => false,
        }
    }

    /// The bike's condition after a repair or a swap in the workshop (spec
    /// §14 Phase 4 W6, the `BikeChange`): the parts as given; flats and a
    /// broken chain on parts now as new are gone, a spare is the own bike
    /// again. The rider's condition stays.
    pub fn set_bike_condition(&mut self, id: RiderId, parts: [f64; damage::PARTS]) -> bool {
        let Some(r) = self.riders.get_mut(&id) else {
            return false;
        };
        let d = &mut r.damage;
        for (p, c) in d.parts.iter_mut().zip(parts) {
            *p = if c.is_finite() {
                c.clamp(0.0, 1.0)
            } else {
                1.0
            };
        }
        for (i, tyre) in [Part::FrontTyre, Part::RearTyre].into_iter().enumerate() {
            if d.part(tyre) >= 1.0 {
                d.flat[i] = false;
            }
        }
        if d.part(Part::Drivetrain) >= 1.0 {
            d.chain_broken = false;
        }
        d.spare = false;
        r.damage_changed = true;
        true
    }

    /// The riders whose damage changed since the last call (sent at once,
    /// besides the 2 Hz of the snapshots).
    pub fn take_damage_changes(&mut self) -> Vec<RiderId> {
        self.riders
            .iter_mut()
            .filter_map(|(id, r)| std::mem::take(&mut r.damage_changed).then_some(*id))
            .collect()
    }

    /// The rider asks to mend a puncture now (12E: stop for a 30 s repair
    /// instead of riding on slowly).
    pub fn request_repair(&mut self, id: RiderId) -> Result<String, String> {
        if !self.damage_on() {
            return Err("damage is off (set ride.damage on)".into());
        }
        let r = self.riders.get_mut(&id).ok_or("no such rider")?;
        if r.damage.stop.is_some() {
            return Err("already stopped".into());
        }
        if !r.damage.start_repair() {
            return Err("no puncture to mend".into());
        }
        r.damage_changed = true;
        Ok(format!(
            "stopping to mend the puncture ({:.0} s)",
            damage::PUNCTURE_REPAIR_S
        ))
    }

    /// Stages damage for rider `id` (`dev.damage`, local server only).
    pub fn stage_damage(&mut self, id: RiderId, what: DamageStage) -> Result<String, String> {
        if !self.damage_on() {
            return Err("damage is off (set ride.damage on, collisions on)".into());
        }
        let tick = self.tick;
        let r = self.riders.get_mut(&id).ok_or("no such rider")?;
        let d = &mut r.damage;
        let before = (d.rider, d.bike());
        let said = match what {
            DamageStage::Rider(c) => {
                d.rider = c.clamp(0.0, 1.0);
                format!(
                    "rider condition {:.0} % ({})",
                    d.rider * 100.0,
                    d.injury().label()
                )
            }
            DamageStage::Part(p, c) => {
                d.parts[p.index()] = c.clamp(0.0, 1.0);
                format!("{} {:.0} %", p.label(), c.clamp(0.0, 1.0) * 100.0)
            }
            DamageStage::Puncture { rear } => {
                d.flat[usize::from(rear)] = true;
                format!("{} puncture", if rear { "rear" } else { "front" })
            }
            DamageStage::Chain => {
                d.chain_broken = true;
                d.parts[Part::Drivetrain.index()] = d.parts[Part::Drivetrain.index()].min(0.3);
                "chain broken".into()
            }
            DamageStage::Reset => {
                *d = Damage::fresh();
                "rider and bike as new".into()
            }
        };
        let (rider, bike) = (d.rider, d.bike());
        if rider < before.0 || bike < before.1 {
            d.last_hit = Some(damage::Loss {
                tick,
                rider: before.0 - rider,
                bike: before.1 - bike,
                parts: d.affected(),
                crash: false,
            });
        }
        r.damage_changed = true;
        Ok(said)
    }

    /// A contact's damage (12E): the crash's hit, or the contact that did
    /// not bring the rider down; a crash keeps a hurt rider down longer.
    pub(super) fn damage_hit(r: &mut Rider, resp: &crate::collide::Response, tick: u64) {
        if let Some(hit) = resp.crash {
            r.damage.apply(&hit, true, tick);
            if let Some(c) = &mut r.crash {
                c.down_s *= r.damage.down_factor();
            }
            r.damage_changed = true;
        } else if let Some(hit) = resp.touch {
            let loss = r.damage.apply(&hit, false, tick);
            if loss.rider > 0.0 || loss.bike > 0.0 {
                r.damage_changed = true;
            }
        }
    }
}

/// One damage tick of a riding rider (not down after a crash): the stops,
/// the recovery, the neutral service. Returns whether the rider abandons
/// the event.
pub(super) fn damage_tick(r: &mut Rider, in_event: bool, rules: DamageRules) -> bool {
    let ctx = TickCtx {
        dt: SIM_DT,
        speed: r.state.speed,
        power_w: r.input.power_w,
        rider_kg: r.profile.rider_mass_kg,
        bot: matches!(r.control, Control::Bot { .. }),
        in_event,
        rules,
    };
    let out = r.damage.tick(&ctx);
    if out != TickOut::Nothing {
        r.damage_changed = true;
    }
    if let TickOut::StopStarted(StopKind::SpareBike | StopKind::Medical) = out
        && let Steering::Auto(_) = r.steering
    {
        r.steering = Steering::Auto(r.state.lateral_offset);
    }
    out == TickOut::Abandon
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collide::{ContactKind, Hit, Response};

    fn straight() -> Route {
        let pts: Vec<[f64; 3]> = (0..=200).map(|i| [f64::from(i) * 5.0, 0.0, 0.0]).collect();
        Route::from_points("s", "Straight", pts, false, 8.0).unwrap()
    }

    fn session(damage: bool) -> (Session, RiderId) {
        let mut s = Session::new(straight());
        s.set_collisions(true);
        s.set_damage(damage);
        let id = s.add_rider("Me", RiderProfile::default());
        (s, id)
    }

    fn ride(s: &mut Session, id: RiderId, watts: f64, seconds: f64) {
        for i in 0..(seconds / SIM_DT) as u32 {
            s.apply_input(
                id,
                i + 1 + s.tick() as u32,
                RiderInput {
                    power_w: watts,
                    ..Default::default()
                },
            );
            s.step();
        }
    }

    fn crash(kind: ContactKind, energy_j: f64) -> Response {
        Response {
            crash: Some(Hit {
                kind,
                other: RiderId(0),
                closing_mps: 5.0,
                energy_j,
                impulse: [-4.0, 0.0],
            }),
            ..Default::default()
        }
    }

    #[test]
    fn damage_off_changes_nothing() {
        let (mut s, id) = session(false);
        assert!(!s.damage_on());
        s.apply_response(id, &crash(ContactKind::HeadOn, 1500.0));
        let d = s.damage(id).unwrap();
        assert!(d.is_fresh(), "{d:?}");
        assert_eq!(d.crashes, 0);
        assert!(s.stage_damage(id, DamageStage::Rider(0.2)).is_err());
        // Carried-over damage has no effect with damage off: the same
        // speed as a fresh rider.
        let (mut a, ia) = session(false);
        let (mut b, ib) = session(false);
        b.carry_damage(ib, 0.3, [0.2; damage::PARTS]);
        ride(&mut a, ia, 200.0, 20.0);
        ride(&mut b, ib, 200.0, 20.0);
        assert_eq!(a.rider_state(ia), b.rider_state(ib));
        assert_eq!(
            b.damage(ib).unwrap().rider,
            0.3,
            "nothing changes, nothing heals"
        );
        // Collisions off: damage is off too.
        let (mut s, id) = session(true);
        s.set_collisions(false);
        assert!(!s.damage_on());
        s.apply_response(id, &crash(ContactKind::HeadOn, 1500.0));
        assert!(s.damage(id).unwrap().is_fresh());
    }

    #[test]
    fn a_crash_takes_off_the_meters_and_keeps_a_hurt_rider_down_longer() {
        let (mut s, id) = session(true);
        ride(&mut s, id, 200.0, 5.0);
        s.apply_response(id, &crash(ContactKind::HeadOn, 1500.0));
        let d = s.damage(id).unwrap().clone();
        assert!(d.rider < 0.8 && d.bike() < 0.75, "{d:?}");
        assert_eq!(d.crashes, 1);
        assert!(s.take_damage_changes().contains(&id));
        let down = s.crash(id).unwrap().down_s;
        // The second crash of the now hurt rider lasts longer.
        ride(&mut s, id, 200.0, 15.0);
        assert!(s.crash(id).is_none());
        s.stage_damage(id, DamageStage::Rider(0.5)).unwrap();
        s.apply_response(id, &crash(ContactKind::HeadOn, 1500.0));
        let longer = s.crash(id).unwrap().down_s;
        assert!(longer > down * 1.2, "{down} → {longer}");
    }

    #[test]
    fn the_virtual_rider_is_slower_hurt_and_on_a_buckled_wheel() {
        let speed_after = |stage: Option<DamageStage>| {
            let (mut s, id) = session(true);
            if let Some(st) = stage {
                s.stage_damage(id, st).unwrap();
            }
            ride(&mut s, id, 200.0, 60.0);
            s.rider_state(id).unwrap().speed
        };
        let fresh = speed_after(None);
        let hurt = speed_after(Some(DamageStage::Rider(0.5)));
        let buckled = speed_after(Some(DamageStage::Part(Part::RearWheel, 0.3)));
        let drivetrain = speed_after(Some(DamageStage::Part(Part::Drivetrain, 0.3)));
        assert!(hurt < fresh - 0.05, "{fresh} {hurt}");
        assert!(buckled < fresh - 0.05, "{fresh} {buckled}");
        assert!(drivetrain < fresh - 0.05, "{fresh} {drivetrain}");
        // A flat: at most 15 km/h.
        let flat = speed_after(Some(DamageStage::Puncture { rear: true }));
        assert!(flat <= damage::FLAT_MAX_MPS + 1e-9, "{flat}");
    }

    #[test]
    fn a_puncture_repair_and_a_chain_stop_hold_the_rider() {
        let (mut s, id) = session(true);
        ride(&mut s, id, 250.0, 20.0);
        assert!(s.request_repair(id).is_err(), "no puncture");
        s.stage_damage(id, DamageStage::Puncture { rear: false })
            .unwrap();
        s.request_repair(id).unwrap();
        // Braking to a stop, then 30 s, then riding again without a flat.
        ride(&mut s, id, 250.0, 6.0);
        assert!(s.rider_state(id).unwrap().speed < 0.1);
        assert!(s.damage(id).unwrap().stop.is_some());
        ride(&mut s, id, 250.0, 30.0);
        let d = s.damage(id).unwrap();
        assert!(d.stop.is_none() && !d.flat[0], "{d:?}");
        ride(&mut s, id, 250.0, 5.0);
        assert!(s.rider_state(id).unwrap().speed > 2.0);
        // A broken chain: no power, a 45 s stop.
        s.stage_damage(id, DamageStage::Chain).unwrap();
        ride(&mut s, id, 300.0, 10.0);
        assert!(s.rider_state(id).unwrap().speed < 0.1);
        ride(&mut s, id, 300.0, 40.0);
        assert!(!s.damage(id).unwrap().chain_broken);
    }

    #[test]
    fn a_badly_injured_entrant_is_dnf_with_the_organisers_rule() {
        let mut s = Session::new(straight());
        s.set_collisions(true);
        s.set_damage(true);
        let me = s.add_rider("Me", RiderProfile::default());
        s.add_bot("Bot", 150.0);
        s.start_event(crate::EventSpec::Race {
            laps: None,
            length_m: Some(800.0),
        })
        .unwrap();
        s.set_event_damage(DamageRules {
            on: Some(true),
            dnf_badly_injured: true,
            neutral_service: false,
        });
        ride(&mut s, me, 200.0, crate::event::COUNTDOWN_S + 3.0);
        s.stage_damage(me, DamageStage::Rider(0.1)).unwrap();
        ride(&mut s, me, 200.0, 1.0);
        let e = s.event().unwrap();
        assert!(e.entrant(me).unwrap().dnf);
        assert!(s.damage(me).unwrap().abandoned);
        assert!(
            s.take_event_changes()
                .contains(&crate::event::EventChange::Dnf(me))
        );
    }

    #[test]
    fn bots_take_damage_and_mend_punctures_at_once() {
        let mut s = Session::new(straight());
        s.set_collisions(true);
        s.set_damage(true);
        let bot = s.add_bot("Bot", 200.0);
        ride(&mut s, bot, 0.0, 5.0);
        s.stage_damage(bot, DamageStage::Puncture { rear: true })
            .unwrap();
        s.step();
        assert_eq!(
            s.damage(bot).unwrap().stop.map(|s| s.kind),
            Some(StopKind::Puncture)
        );
        // The carried condition is only for clients.
        assert!(!s.carry_damage(bot, 0.5, [0.5; damage::PARTS]));
    }

    #[test]
    fn a_workshop_repair_reaches_the_ride() {
        let (mut s, id) = session(true);
        let mut parts = [1.0; damage::PARTS];
        parts[Part::RearTyre.index()] = 0.5;
        s.carry_damage(id, 0.6, parts);
        s.stage_damage(id, DamageStage::Puncture { rear: true })
            .unwrap();
        s.take_damage_changes();
        assert!(s.set_bike_condition(id, [1.0; damage::PARTS]));
        let d = s.damage(id).unwrap();
        assert_eq!(d.bike(), 1.0);
        assert!(!d.flat[1], "a new tyre");
        assert_eq!(d.rider, 0.6, "the rider is not repaired");
        assert_eq!(s.take_damage_changes(), vec![id]);
    }
}

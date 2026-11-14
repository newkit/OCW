//! The client's side of damage (spec §14 Phase 12E / 12F, K4 / K5): the
//! switch in force (Welcome, Snapshot), every rider's damage from the
//! snapshots (server-authoritative, 2 Hz and on every change), the own
//! rider's effects for the prediction, the HUD widget with its hit
//! animation ("−12 % bike" for 2 s, the bar dropping, the icon flashing),
//! the damage dots of others, the visible damage, and the condition carried
//! over between rides (Phase 12 Q6: [`Carried`] with the healing over real
//! time; anonymous riders never keep it).

use std::collections::BTreeMap;

use ocw_core::RiderId;
use ocw_protocol::v1;
use ocw_render::damage_hud::{DamageWidgetView, MeterView};
use ocw_sim::damage::{
    self, BikeLevel, Damage, Effects, Injury, PARTS, Part, Status, Visible, part_status,
};

/// The loss of a hit shows this long (s, 12F).
pub const LOSS_S: f64 = 2.0;
/// The bar drops over this long (s).
pub const DROP_S: f64 = 0.6;
/// The icon flashes this long (s).
pub const FLASH_S: f64 = 0.9;

/// The last hit of the own rider as the client saw it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct SeenHit {
    tick: u64,
    at_s: f64,
    rider: f64,
    bike: f64,
}

/// Damage state of the client.
#[derive(Debug, Clone, Default)]
pub struct DamageClient {
    on: bool,
    riders: BTreeMap<RiderId, Damage>,
    hit: Option<SeenHit>,
}

impl DamageClient {
    /// Damage in force (the server says; only with collisions on).
    pub fn on(&self) -> bool {
        self.on
    }

    pub fn on_welcome(&mut self, w: &v1::Welcome) {
        self.on = w.damage;
        self.riders.clear();
        self.hit = None;
    }

    /// A snapshot: the switch and the damage of every rider in it (absent:
    /// as before); riders who left are forgotten.
    pub fn on_snapshot(&mut self, s: &v1::Snapshot, now_s: f64, own: Option<RiderId>) {
        self.on = s.damage;
        for u in &s.riders {
            let id = RiderId(u.rider_id);
            if let Some(d) = ocw_protocol::rider_damage(u) {
                if Some(id) == own
                    && let Some(h) = d.last_hit
                    && self.hit.is_none_or(|seen| seen.tick != h.tick)
                {
                    // A new hit: the first one seen after joining only if
                    // it is fresh (not the last ride's).
                    let fresh = self.riders.contains_key(&id) || h.tick + 50 >= s.server_tick;
                    if fresh {
                        self.hit = Some(SeenHit {
                            tick: h.tick,
                            at_s: now_s,
                            rider: h.rider,
                            bike: h.bike,
                        });
                    } else {
                        self.hit = Some(SeenHit {
                            tick: h.tick,
                            at_s: f64::NEG_INFINITY,
                            rider: 0.0,
                            bike: 0.0,
                        });
                    }
                }
                self.riders.insert(id, d);
            }
        }
        self.riders
            .retain(|id, _| s.riders.iter().any(|u| u.rider_id == id.0));
    }

    /// A rider's damage as last received.
    pub fn damage(&self, id: RiderId) -> Option<&Damage> {
        self.riders.get(&id)
    }

    /// The effects the server applies to rider `id` (the prediction rides
    /// the same); none with damage off.
    pub fn effects(&self, id: Option<RiderId>) -> Effects {
        match id.and_then(|i| self.riders.get(&i)) {
            Some(d) if self.on => d.effects(),
            _ => Effects::NONE,
        }
    }

    /// Stopped by the road (a repair, a medical stop, a spare bike): the
    /// prediction takes the server's state.
    pub fn held(&self, id: Option<RiderId>) -> bool {
        self.effects(id).held
    }

    /// What shows on rider `id`'s bike and body (`None`: nothing, or
    /// damage off).
    pub fn visible(&self, id: RiderId) -> Option<Visible> {
        let v = self.riders.get(&id).filter(|_| self.on)?.visible();
        v.any().then_some(v)
    }

    /// The damage dot of rider `id` in the standings (12F): amber when
    /// hurt or the bike damaged, red when badly injured or the bike
    /// broken; `None` when fine (or damage off).
    pub fn dot(&self, id: RiderId) -> Option<Status> {
        let d = self.riders.get(&id).filter(|_| self.on)?;
        dot(d)
    }

    /// The HUD widget of the own rider at client time `now_s` (12F);
    /// `None` with damage off.
    pub fn widget(&self, own: Option<RiderId>, now_s: f64) -> Option<DamageWidgetView> {
        if !self.on {
            return None;
        }
        let d = self.riders.get(&own?)?;
        Some(widget(
            d,
            self.hit.map(|h| (now_s - h.at_s, h.rider, h.bike)),
        ))
    }

    /// The HUD's notice line while stopped ("Medical stop: 42 s").
    pub fn notice(&self, own: Option<RiderId>) -> Option<String> {
        if !self.on {
            return None;
        }
        let d = self.riders.get(&own?)?;
        if d.abandoned && d.stop.is_none() {
            return Some("Out of the event (DNF)".into());
        }
        if let Some(line) = d.stop_line() {
            return Some(line);
        }
        if d.flat.iter().any(|f| *f) {
            return Some("Puncture: stop (down) to mend it, or ride on slowly".into());
        }
        None
    }

    /// The text of `ride.damage` (console): the meters and the labels.
    pub fn summary(&self, own: Option<RiderId>) -> String {
        if !self.on {
            return "damage: off".into();
        }
        let Some(d) = own.and_then(|id| self.riders.get(&id)) else {
            return "damage: on".into();
        };
        let mut s = format!(
            "damage: on\nrider {:.0} % ({}), bike {:.0} % ({}), {} crash{}",
            d.rider * 100.0,
            d.injury().label(),
            d.bike() * 100.0,
            d.bike_level().label(),
            d.crashes,
            if d.crashes == 1 { "" } else { "es" }
        );
        for p in Part::ALL {
            if d.part(p) < 1.0 {
                s.push_str(&format!("\n  {} {:.0} %", p.label(), d.part(p) * 100.0));
            }
        }
        for l in d.labels() {
            s.push_str(&format!("\n  {l}"));
        }
        s
    }
}

/// The damage dot of a rider (12F).
pub fn dot(d: &Damage) -> Option<Status> {
    let worst = d.injury().status().max(d.bike_level().status());
    match worst {
        Status::Good => None,
        Status::Warning | Status::Serious => Some(Status::Warning),
        Status::Critical => Some(Status::Critical),
    }
}

/// "−12 % bike" / "−8 % rider, −12 % bike".
pub fn loss_text(rider: f64, bike: f64) -> Option<String> {
    let pct = |v: f64| (v * 100.0).round() as i64;
    match (pct(rider), pct(bike)) {
        (0, 0) => None,
        (0, b) => Some(format!("−{b} % bike")),
        (r, 0) => Some(format!("−{r} % rider")),
        (r, b) => Some(format!("−{r} % rider, −{b} % bike")),
    }
}

fn ease_out(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t) * (1.0 - t)
}

/// The widget of damage `d`; `hit`: seconds since the last hit was seen and
/// what it took off (rider, bike).
pub fn widget(d: &Damage, hit: Option<(f64, f64, f64)>) -> DamageWidgetView {
    let recent = hit.filter(|(age, _, _)| (0.0..LOSS_S).contains(age));
    let meter = |value: f64, word: &str, status: Status, lost: f64| {
        let (shown, flash) = match recent {
            Some((age, _, _)) if lost > 0.0 => (
                value + lost * (1.0 - ease_out(age / DROP_S)),
                if age < FLASH_S {
                    // Two flashes.
                    (1.0 - age / FLASH_S) * (0.5 + 0.5 * (age * 14.0).cos())
                } else {
                    0.0
                },
            ),
            _ => (value, 0.0),
        };
        MeterView {
            shown: shown.clamp(0.0, 1.0) as f32,
            pct: (value * 100.0).round() as u32,
            word: word.into(),
            status,
            flash: flash.clamp(0.0, 1.0) as f32,
        }
    };
    let (rider_lost, bike_lost) = recent.map_or((0.0, 0.0), |(_, r, b)| (r, b));
    let injury: Injury = d.injury();
    let level: BikeLevel = d.bike_level();
    let mut parts = [None; PARTS];
    let affected = d.affected();
    for p in Part::ALL {
        if affected & p.bit() != 0 {
            let mut s = part_status(d.part(p)).max(Status::Warning);
            let flat = match p {
                Part::FrontTyre => d.flat[0],
                Part::RearTyre => d.flat[1],
                _ => false,
            };
            if flat || (p == Part::Drivetrain && d.chain_broken) || (p == Part::Frame && d.broken())
            {
                s = Status::Critical;
            }
            parts[p.index()] = Some(s);
        }
    }
    DamageWidgetView {
        rider: meter(d.rider, injury.label(), injury.status(), rider_lost),
        bike: meter(d.bike(), level.label(), level.status(), bike_lost),
        parts,
        labels: d.labels(),
        loss: recent.and_then(|(_, r, b)| loss_text(r, b)),
        stop: d.stop_line(),
    }
}

/// The condition carried over between rides (spec §14 Phase 12 Q6): the
/// rider's injury and the bike's parts, with the real time it was saved
/// at (the injury heals over real time). The desktop keeps it in the
/// local wallet; anonymous browser riders never keep it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Carried {
    pub rider: f64,
    pub parts: [f64; PARTS],
    /// UTC unix seconds when the rider's condition was saved.
    pub at_unix: i64,
}

impl Carried {
    /// What the next ride starts with at `now_unix`: the injury healed by
    /// the real time since it was saved; the bike as it is (repairs are
    /// paid in the workshop).
    pub fn at(&self, now_unix: i64) -> (f64, [f64; PARTS]) {
        let hours = (now_unix - self.at_unix) as f64 / 3600.0;
        (damage::heal(self.rider, hours), self.parts)
    }

    /// The hello's damage at `now_unix` (`None`: fresh, nothing to say),
    /// and never for an anonymous rider.
    pub fn hello(
        carried: Option<&Carried>,
        anonymous: bool,
        now_unix: i64,
    ) -> Option<v1::DamageState> {
        if anonymous {
            return None;
        }
        let (rider, parts) = carried?.at(now_unix);
        (rider < 1.0 || parts.iter().any(|c| *c < 1.0))
            .then(|| ocw_protocol::carried_damage(rider, &parts))
    }

    /// What to keep of the server's damage `d` at the end of (or during)
    /// a ride at `now_unix`; `None` for anonymous riders. A spare bike's
    /// damage is not the own bike's: the bike keeps `before`'s parts.
    pub fn keep(
        d: &Damage,
        before: Option<&Carried>,
        anonymous: bool,
        now_unix: i64,
    ) -> Option<Carried> {
        if anonymous {
            return None;
        }
        let parts = match (d.spare, before) {
            (true, Some(b)) => b.parts,
            (true, None) => [1.0; PARTS],
            (false, _) => d.parts,
        };
        Some(Carried {
            rider: d.rider,
            parts,
            at_unix: now_unix,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ocw_sim::damage::Loss;

    fn snapshot(tick: u64, damage: bool, riders: Vec<(u32, Option<&Damage>)>) -> v1::Snapshot {
        v1::Snapshot {
            server_tick: tick,
            damage,
            riders: riders
                .into_iter()
                .map(|(id, d)| v1::RiderUpdate {
                    rider_id: id,
                    damage: d.map(ocw_protocol::damage_to_wire),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn hurt() -> Damage {
        let mut d = Damage::fresh();
        d.rider = 0.62;
        d.parts[Part::RearWheel.index()] = 0.3;
        d.last_hit = Some(Loss {
            tick: 1000,
            rider: 0.08,
            bike: 0.12,
            parts: Part::RearWheel.bit(),
            crash: true,
        });
        d
    }

    #[test]
    fn hud_widget_states() {
        let me = RiderId(1);
        let mut c = DamageClient::default();
        c.on_snapshot(
            &snapshot(990, true, vec![(1, Some(&Damage::fresh()))]),
            10.0,
            Some(me),
        );
        let w = c.widget(Some(me), 10.0).unwrap();
        assert_eq!((w.rider.pct, w.rider.word.as_str()), (100, "fine"));
        assert_eq!(w.rider.status, Status::Good);
        assert!(w.labels.is_empty() && w.loss.is_none());
        assert!(w.parts.iter().all(|p| p.is_none()));
        // A crash: the loss shows for 2 s, the bar drops, the icon flashes.
        c.on_snapshot(
            &snapshot(1005, true, vec![(1, Some(&hurt()))]),
            11.0,
            Some(me),
        );
        let w = c.widget(Some(me), 11.05).unwrap();
        assert_eq!(w.loss.as_deref(), Some("−8 % rider, −12 % bike"));
        assert_eq!(w.rider.word, "hurt");
        assert_eq!(w.rider.status, Status::Warning);
        assert!(
            w.rider.shown > 0.66,
            "the bar starts where it was: {}",
            w.rider.shown
        );
        assert!(w.bike.flash > 0.5);
        assert_eq!(w.parts[Part::RearWheel.index()], Some(Status::Serious));
        assert!(w.labels.contains(&"rear wheel buckled".to_string()));
        let later = c.widget(Some(me), 11.0 + DROP_S + 0.1).unwrap();
        assert!((later.rider.shown - 0.62).abs() < 1e-6, "dropped");
        let calm = c.widget(Some(me), 11.0 + FLASH_S + 0.05).unwrap();
        assert_eq!(calm.rider.flash, 0.0);
        assert!(calm.loss.is_some(), "the loss stays 2 s");
        let gone = c.widget(Some(me), 11.0 + LOSS_S + 0.1).unwrap();
        assert!(gone.loss.is_none());
        // The same hit again (2 Hz) does not restart the animation.
        c.on_snapshot(
            &snapshot(1030, true, vec![(1, Some(&hurt()))]),
            14.0,
            Some(me),
        );
        assert!(c.widget(Some(me), 14.0).unwrap().loss.is_none());
        // A snapshot without damage keeps the last.
        c.on_snapshot(&snapshot(1035, true, vec![(1, None)]), 14.1, Some(me));
        assert_eq!(c.widget(Some(me), 14.1).unwrap().rider.pct, 62);
        // Damage off: hidden, no effects.
        c.on_snapshot(&snapshot(1040, false, vec![(1, None)]), 14.2, Some(me));
        assert!(c.widget(Some(me), 14.2).is_none());
        assert!(c.effects(Some(me)).is_none());
        assert_eq!(c.dot(me), None);
    }

    #[test]
    fn a_hit_from_an_earlier_ride_does_not_animate() {
        let me = RiderId(1);
        let mut c = DamageClient::default();
        let mut d = hurt();
        d.last_hit.as_mut().unwrap().tick = 10;
        c.on_snapshot(&snapshot(5000, true, vec![(1, Some(&d))]), 3.0, Some(me));
        assert!(c.widget(Some(me), 3.0).unwrap().loss.is_none());
    }

    #[test]
    fn dots_effects_and_notices() {
        let mut c = DamageClient::default();
        let mut bad = Damage::fresh();
        bad.rider = 0.1;
        let mut flat = Damage::fresh();
        flat.flat[1] = true;
        c.on_snapshot(
            &snapshot(
                50,
                true,
                vec![
                    (1, Some(&Damage::fresh())),
                    (2, Some(&hurt())),
                    (3, Some(&bad)),
                    (4, Some(&flat)),
                ],
            ),
            1.0,
            Some(RiderId(4)),
        );
        assert_eq!(c.dot(RiderId(1)), None);
        assert_eq!(c.dot(RiderId(2)), Some(Status::Warning));
        assert_eq!(c.dot(RiderId(3)), Some(Status::Critical));
        assert!(c.effects(Some(RiderId(4))).speed_cap.is_some());
        assert!(c.notice(Some(RiderId(4))).unwrap().starts_with("Puncture"));
        assert!(c.visible(RiderId(2)).unwrap().bent[1] > 0.0);
        assert_eq!(c.visible(RiderId(1)), None);
        // Riders who left are forgotten.
        c.on_snapshot(&snapshot(55, true, vec![(1, None)]), 1.1, None);
        assert!(c.damage(RiderId(2)).is_none());
    }

    #[test]
    fn carried_condition_heals_over_real_time_and_anonymous_riders_keep_nothing() {
        let mut parts = [1.0; PARTS];
        parts[Part::Frame.index()] = 0.6;
        let c = Carried {
            rider: 0.4,
            parts,
            at_unix: 1_000_000,
        };
        let (r, p) = c.at(1_000_000 + 3 * 3600);
        assert!((r - 0.7).abs() < 1e-9, "{r}");
        assert_eq!(
            p, parts,
            "the bike is repaired in the workshop, not by time"
        );
        assert!(Carried::hello(Some(&c), false, 1_000_000).is_some());
        assert!(
            Carried::hello(Some(&c), true, 1_000_000).is_none(),
            "anonymous"
        );
        assert!(Carried::hello(None, false, 1_000_000).is_none());
        let fresh = Carried {
            rider: 0.9,
            parts: [1.0; PARTS],
            at_unix: 0,
        };
        assert!(
            Carried::hello(Some(&fresh), false, 7200).is_none(),
            "healed: nothing to say"
        );
        // Keeping the server's damage; a spare bike keeps the own one's.
        let mut d = hurt();
        let kept = Carried::keep(&d, Some(&c), false, 5).unwrap();
        assert_eq!(kept.parts, d.parts);
        assert_eq!(kept.at_unix, 5);
        d.spare = true;
        assert_eq!(Carried::keep(&d, Some(&c), false, 5).unwrap().parts, parts);
        assert!(Carried::keep(&d, Some(&c), true, 5).is_none());
    }

    #[test]
    fn loss_texts() {
        assert_eq!(loss_text(0.0, 0.12), Some("−12 % bike".into()));
        assert_eq!(loss_text(0.081, 0.0), Some("−8 % rider".into()));
        assert_eq!(loss_text(0.001, 0.002), None);
    }
}

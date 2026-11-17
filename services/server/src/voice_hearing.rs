//! Who receives whose voice (spec §7.1.7 Q6, Q8, Q10, §7.1.3): the game
//! server knows every rider's place, so it decides – per listener – whose
//! voice may reach them, and tells each rider ([`v1::VoiceHearing`]) whose
//! voice it may receive and who may receive its own. The clients subscribe
//! only to the first and allow their microphone only to the second
//! (LiveKit's track subscription permissions, enforced by the voice
//! server), so the voice server sends a rider's voice only to riders who
//! can hear it.
//!
//! A listener L receives speaker S when all hold:
//!
//! - **range** (Q6 / Q10 / Q20): the two are at most [`GRANT_M`] apart
//!   (the voices fade from 5 m to zero at 10 m; the margin lets a voice be
//!   ready before it is audible), and once granted the grant holds up to
//!   [`KEEP_M`] – nobody pops in and out at the edge. **No range** when L
//!   listens to *Voice room: Selected riders* and S is among them (Q20:
//!   selected riders are always audible); with the Echo rider on either
//!   side the Echo rider's wider range ([`ECHO_GRANT_M`], [`ECHO_KEEP_M`]);
//! - **Who hears me** of S (Q8): *All nearby*, or *Selected riders* with L
//!   among S's selection (*No one*, the default, lets nobody hear S);
//! - **Voice room** of L (§7.1.3): *Everybody*, or *Selected riders* with S
//!   among L's selection;
//! - **no block** between them in either direction (Q9) – a block always
//!   wins, also over a selection.
//!
//! The Echo rider (spec §7 *Echo bot*) has no connection: it listens to
//! everybody and is heard by all nearby, like a rider with the defaults
//! *Everybody* / *All nearby*; the speakers' own *Who hears me* still
//! decides whether it hears them.
//!
//! Distances: on a route the gap along it (the shorter way round on a
//! loop); in free ride the straight-line distance of the riders' places –
//! as the clients place the voices (`ocw_client::voice::peers`).
//!
//! Pure: [`hearing`] takes the riders, their places and audiences, the
//! sets apart and the previous answer (for the margin).

use std::collections::{BTreeMap, BTreeSet};

use ocw_core::RiderId;
use ocw_protocol::v1;

use crate::voice_block::{self, Who};

/// A voice is granted within this distance (m): the 10 m of the hearing
/// range (spec §7.1.7 Q20) and a margin of half a metre.
pub const GRANT_M: f64 = 10.5;
/// A granted voice is kept up to this distance (m): a rider riding away
/// keeps the (silent) voice for 2 m more, so two riders riding side by
/// side at the edge are not granted and revoked with every snapshot.
pub const KEEP_M: f64 = 12.0;
/// The Echo rider's range (spec §7 *Echo bot*: its playback fades to zero
/// at 300 m, `ocw_voice::spatial::ECHO_RANGE_M`), granted within …
pub const ECHO_GRANT_M: f64 = 310.0;
/// … and kept up to this distance (m).
pub const ECHO_KEEP_M: f64 = 340.0;

/// *Who hears me* (spec §7.1.7 Q8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeardBy {
    #[default]
    NoOne,
    Nearby,
    Selected,
}

/// *Voice room* (spec §7.1.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Listen {
    #[default]
    Everybody,
    Selected,
}

/// A rider's audience as the server keeps it: the two settings and the
/// selected riders (matched like a block list: by account, by name).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Audience {
    pub heard_by: HeardBy,
    pub listen: Listen,
    pub selected: voice_block::Blocks,
}

impl Audience {
    /// From the wire (the lists cleaned like a block list).
    pub fn from_wire(a: &v1::VoiceAudience) -> Self {
        Self {
            heard_by: match a.heard_by() {
                v1::VoiceHeardBy::NoOne => HeardBy::NoOne,
                v1::VoiceHeardBy::Nearby => HeardBy::Nearby,
                v1::VoiceHeardBy::Selected => HeardBy::Selected,
            },
            listen: match a.listen() {
                v1::VoiceListen::Everybody => Listen::Everybody,
                v1::VoiceListen::Selected => Listen::Selected,
            },
            selected: voice_block::Blocks::from_wire(&v1::VoiceBlocks {
                account_ids: a.selected_account_ids.clone(),
                names: a.selected_names.clone(),
            }),
        }
    }

    /// The Echo rider's: hears everybody, heard by all nearby.
    pub fn echo() -> Self {
        Self {
            heard_by: HeardBy::Nearby,
            listen: Listen::Everybody,
            selected: Default::default(),
        }
    }

    /// Lets `listener` hear the rider with this audience.
    fn lets_hear(&self, listener: &Who) -> bool {
        match self.heard_by {
            HeardBy::NoOne => false,
            HeardBy::Nearby => true,
            HeardBy::Selected => self.selected.blocks(listener),
        }
    }

    /// Listens to `speaker`.
    fn listens_to(&self, speaker: &Who) -> bool {
        match self.listen {
            Listen::Everybody => true,
            Listen::Selected => self.selected.blocks(speaker),
        }
    }

    /// Hears `speaker` at any distance (spec §7.1.7 Q20): listens to
    /// *Selected riders* and `speaker` is among them.
    fn always_hears(&self, speaker: &Who) -> bool {
        self.listen == Listen::Selected && self.selected.blocks(speaker)
    }
}

/// Where a rider is, for the distance between two.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Place {
    /// Distance along the route (m).
    Route(f64),
    /// The place in free ride (x, z in m).
    Free([f64; 2]),
}

/// The distance between two places (m); `loop_m`: the route's length if
/// it is a loop. Unknown (different kinds, not finite): infinite.
pub fn distance(a: Place, b: Place, loop_m: Option<f64>) -> f64 {
    let d = match (a, b) {
        (Place::Route(x), Place::Route(y)) => {
            let mut gap = (x - y).abs();
            if let Some(len) = loop_m.filter(|l| *l > 0.0) {
                gap = gap.rem_euclid(len);
                gap = gap.min(len - gap);
            }
            gap
        }
        (Place::Free(p), Place::Free(q)) => (p[0] - q[0]).hypot(p[1] - q[1]),
        _ => f64::INFINITY,
    };
    if d.is_finite() { d } else { f64::INFINITY }
}

/// A rider in voice: its key (`None`: the Echo rider, no connection), who
/// it is, where and its audience.
#[derive(Debug, Clone)]
pub struct Member<K> {
    pub key: Option<K>,
    pub who: Who,
    pub place: Place,
    pub audience: Audience,
}

/// The answer for one rider: whose voice it may receive and who may
/// receive its own.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Hearing {
    pub may_hear: BTreeSet<RiderId>,
    pub heard_by: BTreeSet<RiderId>,
}

impl Hearing {
    pub fn to_wire(&self) -> v1::VoiceHearing {
        v1::VoiceHearing {
            may_hear: self.may_hear.iter().map(|r| r.0).collect(),
            heard_by: self.heard_by.iter().map(|r| r.0).collect(),
        }
    }
}

/// The hearing of every member with a key (see the module docs). `apart`:
/// the riders each is kept apart from by a block (either direction);
/// `previous`: the last answer (the margin: a grant holds up to
/// [`KEEP_M`]).
pub fn hearing<K: Ord + Copy>(
    members: &[Member<K>],
    apart: &BTreeMap<K, BTreeSet<RiderId>>,
    previous: &BTreeMap<K, Hearing>,
    loop_m: Option<f64>,
) -> BTreeMap<K, Hearing> {
    let apart_of = |m: &Member<K>| m.key.and_then(|k| apart.get(&k));
    // Whether `l` heard `s` in the last answer (asked of either side's).
    let granted_before = |s: &Member<K>, l: &Member<K>| {
        l.key
            .and_then(|k| previous.get(&k))
            .map(|h| h.may_hear.contains(&s.who.rider))
            .or_else(|| {
                s.key
                    .and_then(|k| previous.get(&k))
                    .map(|h| h.heard_by.contains(&l.who.rider))
            })
            .unwrap_or(false)
    };
    let mut out: BTreeMap<K, Hearing> = members
        .iter()
        .filter_map(|m| m.key.map(|k| (k, Hearing::default())))
        .collect();
    for s in members {
        for l in members {
            if s.who.rider == l.who.rider {
                continue;
            }
            let kept_apart = apart_of(s).is_some_and(|a| a.contains(&l.who.rider))
                || apart_of(l).is_some_and(|a| a.contains(&s.who.rider));
            if kept_apart || !s.audience.lets_hear(&l.who) || !l.audience.listens_to(&s.who) {
                continue;
            }
            if !l.audience.always_hears(&s.who) {
                let echo = s.key.is_none() || l.key.is_none();
                let (grant, keep) = if echo {
                    (ECHO_GRANT_M, ECHO_KEEP_M)
                } else {
                    (GRANT_M, KEEP_M)
                };
                let d = distance(s.place, l.place, loop_m);
                let limit = if granted_before(s, l) { keep } else { grant };
                if d > limit {
                    continue;
                }
            }
            if let Some(h) = l.key.and_then(|k| out.get_mut(&k)) {
                h.may_hear.insert(s.who.rider);
            }
            if let Some(h) = s.key.and_then(|k| out.get_mut(&k)) {
                h.heard_by.insert(l.who.rider);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn who(id: u32, name: &str) -> Who {
        Who {
            rider: RiderId(id),
            name: name.into(),
            account_id: None,
        }
    }

    fn nearby() -> Audience {
        Audience {
            heard_by: HeardBy::Nearby,
            ..Default::default()
        }
    }

    fn member(key: u8, name: &str, at: f64, audience: Audience) -> Member<u8> {
        Member {
            key: Some(key),
            who: who(u32::from(key), name),
            place: Place::Route(at),
            audience,
        }
    }

    fn ids(set: &BTreeSet<RiderId>) -> Vec<u32> {
        set.iter().map(|r| r.0).collect()
    }

    fn selecting(names: &[&str]) -> voice_block::Blocks {
        voice_block::Blocks {
            names: names.iter().map(|n| (*n).to_owned()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn distances_along_a_loop_and_in_free_ride() {
        assert_eq!(
            distance(Place::Route(10.0), Place::Route(110.0), None),
            100.0
        );
        // The shorter way round a 1 km loop.
        assert_eq!(
            distance(Place::Route(950.0), Place::Route(50.0), Some(1000.0)),
            100.0
        );
        assert_eq!(
            distance(Place::Free([0.0, 0.0]), Place::Free([3.0, 4.0]), None),
            5.0
        );
        assert_eq!(
            distance(Place::Route(0.0), Place::Free([0.0, 0.0]), None),
            f64::INFINITY
        );
        assert_eq!(
            distance(Place::Route(f64::NAN), Place::Route(0.0), None),
            f64::INFINITY
        );
    }

    /// Range (Q20, changing Q10): near riders hear each other, a far one
    /// nobody; moving in grants at 10.5 m, moving out keeps the grant to
    /// 12 m.
    #[test]
    fn range_with_a_margin_in_and_out() {
        assert_eq!((GRANT_M, KEEP_M), (10.5, 12.0));
        let mut members = vec![
            member(1, "Ada Rider", 0.0, nearby()),
            member(2, "Bea Climber", 3.0, nearby()),
            member(3, "Cleo Sprinter", 1000.0, nearby()),
        ];
        let none = BTreeMap::new();
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert_eq!(ids(&h[&1].may_hear), [2]);
        assert_eq!(ids(&h[&1].heard_by), [2]);
        assert_eq!(ids(&h[&2].may_hear), [1]);
        assert!(h[&3].may_hear.is_empty() && h[&3].heard_by.is_empty());
        // A rider about 20 m away is not sent either.
        members[2].place = Place::Route(20.0);
        let h = hearing(&members, &none, &h, None);
        assert!(h[&3].may_hear.is_empty() && h[&3].heard_by.is_empty());
        // Cleo comes nearer: 14 m from Ada, 11 m from Bea - not yet.
        members[2].place = Place::Route(14.0);
        let h = hearing(&members, &none, &h, None);
        assert!(h[&3].may_hear.is_empty(), "outside 10.5 m: not yet");
        // 13 m from Ada, 10 m from Bea: Bea only.
        members[2].place = Place::Route(13.0);
        let h = hearing(&members, &none, &h, None);
        assert_eq!(ids(&h[&3].may_hear), [2]);
        members[2].place = Place::Route(10.4);
        let h = hearing(&members, &none, &h, None);
        assert_eq!(ids(&h[&3].may_hear), [1, 2], "both within 10.5 m");
        // Moving away: kept up to 12 m, then gone.
        members[2].place = Place::Route(11.9);
        let h = hearing(&members, &none, &h, None);
        assert_eq!(ids(&h[&3].may_hear), [1, 2]);
        assert_eq!(ids(&h[&1].heard_by), [2, 3]);
        members[2].place = Place::Route(15.5);
        let h = hearing(&members, &none, &h, None);
        assert!(h[&3].may_hear.is_empty(), "past 12 m from both");
        assert_eq!(ids(&h[&1].heard_by), [2]);
        // And back in only within 10.5 m again: 8 m from Bea, 11 m from
        // Ada.
        members[2].place = Place::Route(11.0);
        let h = hearing(&members, &none, &h, None);
        assert_eq!(ids(&h[&3].may_hear), [2]);
    }

    /// Selected riders are always audible (Q20): with *Voice room:
    /// Selected riders* the listener receives them at any distance; the
    /// speaker's *Who hears me* still decides, and a block still wins.
    #[test]
    fn selected_riders_are_sent_at_any_distance() {
        let none = BTreeMap::new();
        let ada = Audience {
            heard_by: HeardBy::Nearby,
            listen: Listen::Selected,
            selected: selecting(&["Bea Climber"]),
        };
        let mut members = vec![
            member(1, "Ada Rider", 0.0, ada),
            member(2, "Bea Climber", 1000.0, nearby()),
            member(3, "Cleo Sprinter", 1000.0, nearby()),
        ];
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert_eq!(ids(&h[&1].may_hear), [2], "the selected Bea 1 km away");
        assert_eq!(ids(&h[&2].heard_by), [1, 3]);
        assert!(
            h[&1].heard_by.is_empty(),
            "Bea listens to everybody: Ada 1 km away is out of her range"
        );
        assert!(!h[&3].heard_by.contains(&RiderId(1)), "Cleo not selected");
        // Bea's *Who hears me: No one*: not even her selector hears her.
        members[1].audience.heard_by = HeardBy::NoOne;
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert!(h[&1].may_hear.is_empty());
        // A block wins over the selection, however far.
        members[1].audience.heard_by = HeardBy::Nearby;
        let apart: BTreeMap<u8, BTreeSet<RiderId>> = [(2, [RiderId(1)].into())].into();
        let h = hearing(&members, &apart, &BTreeMap::new(), None);
        assert!(h[&1].may_hear.is_empty() && h[&2].heard_by.is_empty());
        // *Everybody* again: the range holds for Bea too.
        members[0].audience.listen = Listen::Everybody;
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert!(h[&1].may_hear.is_empty());
    }

    /// *Who hears me* (Q8): No one (the default) – nobody; All nearby;
    /// Selected riders – only them, and only in range.
    #[test]
    fn who_hears_me_no_one_nearby_selected() {
        let none = BTreeMap::new();
        let members = vec![
            member(1, "Ada Rider", 0.0, Audience::default()),
            member(2, "Bea Climber", 5.0, nearby()),
            member(3, "Cleo Sprinter", 8.0, nearby()),
        ];
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert!(h[&1].heard_by.is_empty(), "the default: no one hears Ada");
        assert_eq!(ids(&h[&1].may_hear), [2, 3], "Ada hears the others");
        assert_eq!(ids(&h[&2].may_hear), [3]);
        let mut members = members;
        members[0].audience = Audience {
            heard_by: HeardBy::Selected,
            selected: selecting(&["Cleo Sprinter", "Dan Descender"]),
            ..Default::default()
        };
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert_eq!(ids(&h[&1].heard_by), [3], "only the selected Cleo");
        assert_eq!(ids(&h[&3].may_hear), [1, 2]);
        assert_eq!(ids(&h[&2].may_hear), [3], "Bea does not hear Ada");
        // The selected rider far away: not heard either (*Who hears me* is
        // not the *Voice room*: Cleo listens to everybody, by range).
        members[2].place = Place::Route(900.0);
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert!(h[&1].heard_by.is_empty());
        // By account: a selection by account id matches the signed-in
        // rider whatever the name.
        members[2].place = Place::Route(8.0);
        members[2].who.account_id = Some("acc-cleo".into());
        members[2].who.name = "Cleo Renamed".into();
        members[0].audience.selected = voice_block::Blocks {
            account_ids: ["acc-cleo".to_owned()].into(),
            ..Default::default()
        };
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert_eq!(ids(&h[&1].heard_by), [3]);
    }

    /// *Voice room: Selected riders* (§7.1.3): the listener receives only
    /// the selected riders.
    #[test]
    fn voice_room_selected_receives_only_the_selection() {
        let none = BTreeMap::new();
        let members = vec![
            member(
                1,
                "Ada Rider",
                0.0,
                Audience {
                    heard_by: HeardBy::Nearby,
                    listen: Listen::Selected,
                    selected: selecting(&["Bea Climber"]),
                },
            ),
            member(2, "Bea Climber", 5.0, nearby()),
            member(3, "Cleo Sprinter", 8.0, nearby()),
        ];
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert_eq!(ids(&h[&1].may_hear), [2]);
        assert_eq!(ids(&h[&3].heard_by), [2], "Cleo's voice is not sent to Ada");
        assert_eq!(ids(&h[&1].heard_by), [2, 3], "Ada is still heard by both");
    }

    /// A block always wins (Q9), over a selection too, in both directions.
    #[test]
    fn a_block_always_wins() {
        let members = vec![
            member(
                1,
                "Ada Rider",
                0.0,
                Audience {
                    heard_by: HeardBy::Selected,
                    listen: Listen::Selected,
                    selected: selecting(&["Bea Climber", "Cleo Sprinter"]),
                },
            ),
            member(2, "Bea Climber", 5.0, nearby()),
            member(3, "Cleo Sprinter", 8.0, nearby()),
        ];
        let apart: BTreeMap<u8, BTreeSet<RiderId>> =
            [(1, [RiderId(2)].into()), (2, [RiderId(1)].into())].into();
        let h = hearing(&members, &apart, &BTreeMap::new(), None);
        assert_eq!(ids(&h[&1].may_hear), [3]);
        assert_eq!(ids(&h[&1].heard_by), [3]);
        assert_eq!(ids(&h[&2].may_hear), [3]);
        assert_eq!(ids(&h[&2].heard_by), [3]);
        // Only one side's set says it: enough.
        let one: BTreeMap<u8, BTreeSet<RiderId>> = [(2, [RiderId(1)].into())].into();
        assert_eq!(hearing(&members, &one, &BTreeMap::new(), None), h);
    }

    /// The Echo rider listens to everybody and is heard by all nearby; the
    /// speaker's *Who hears me* still decides.
    #[test]
    fn the_echo_rider_hears_whom_the_speakers_allow() {
        let none = BTreeMap::new();
        let echo = Member {
            key: None,
            who: who(9, "Echo"),
            place: Place::Route(20.0),
            audience: Audience::echo(),
        };
        let members = vec![
            member(1, "Ada Rider", 0.0, nearby()),
            member(2, "Bea Climber", 5.0, Audience::default()),
            echo,
        ];
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert_eq!(
            ids(&h[&1].heard_by),
            [2, 9],
            "Bea and the Echo rider 20 m away (its own, wider range)"
        );
        assert_eq!(ids(&h[&1].may_hear), [9], "Bea is heard by no one");
        assert_eq!(ids(&h[&2].may_hear), [1, 9]);
        assert!(h[&2].heard_by.is_empty(), "not even by the Echo rider");
        assert_eq!(h.len(), 2, "the Echo rider gets no answer");
        // Its range ends at 310 m (kept to 340 m).
        let mut members = members;
        members[2].place = Place::Route(311.0);
        let h = hearing(&members, &none, &BTreeMap::new(), None);
        assert_eq!(ids(&h[&1].heard_by), [2]);
    }

    #[test]
    fn audiences_from_the_wire() {
        let a = Audience::from_wire(&v1::VoiceAudience {
            heard_by: v1::VoiceHeardBy::Selected as i32,
            listen: v1::VoiceListen::Selected as i32,
            selected_account_ids: vec!["acc-1".into()],
            selected_names: vec![" Bea Climber ".into(), String::new()],
        });
        assert_eq!(a.heard_by, HeardBy::Selected);
        assert_eq!(a.listen, Listen::Selected);
        assert_eq!(a.selected.names.len(), 1);
        assert_eq!(
            Audience::from_wire(&v1::VoiceAudience::default()),
            Audience::default()
        );
        // An unknown value from a newer client: the default.
        let odd = Audience::from_wire(&v1::VoiceAudience {
            heard_by: 77,
            ..Default::default()
        });
        assert_eq!(odd.heard_by, HeardBy::NoOne);
    }
}

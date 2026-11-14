//! The rider list (spec §14 Phase 13E: "a host can see who is in god
//! mode in the rider list"; Phase 12F: the damage dot next to riders in
//! the rider list): one row per rider of the session – the name, where the
//! rider is (the gap to the own rider along the route, or the distance in
//! free ride, where everyone rides an own path), a unicorn marker for
//! riders flying in unicorn mode and the damage dot (amber / red, with
//! its icon: never colour alone). The app's menu page draws the rows.

use ocw_core::RiderId;
use ocw_sim::damage::Status;

/// One rider as the list needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub id: RiderId,
    pub name: String,
    pub is_local: bool,
    /// Distance along the route (m; routes, where it is comparable).
    pub along_m: f64,
    /// Where the rider is (local metres, x, y up, z).
    pub pos: [f64; 3],
    /// Flying in unicorn mode (spec §14 Phase 13).
    pub flying: bool,
    /// Down after a crash (spec §14 Phase 12C).
    pub down: bool,
    /// The damage dot (12F): amber or red; `None` when fine or damage off.
    pub dot: Option<Status>,
}

/// One row of the list.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: RiderId,
    /// The name, "(you)" after the own one.
    pub label: String,
    /// "+120 m", "1.4 km behind", "350 m away", "flying"; "you" for the
    /// own row with its distance.
    pub detail: String,
    pub flying: bool,
    pub dot: Option<Status>,
}

fn metres(m: f64) -> String {
    let a = m.abs();
    if a < 1000.0 {
        format!("{a:.0} m")
    } else {
        format!("{:.1} km", a / 1000.0)
    }
}

/// The rows: the own rider first, then the others nearest first; `free`
/// rides compare straight-line distances (everyone on an own path).
pub fn rows(entries: &[Entry], free: bool) -> Vec<Row> {
    let me = entries.iter().find(|e| e.is_local);
    let gap = |e: &Entry| -> f64 {
        match me {
            Some(m) if free => {
                let d = [e.pos[0] - m.pos[0], e.pos[2] - m.pos[2]];
                d[0].hypot(d[1])
            }
            Some(m) => e.along_m - m.along_m,
            None => e.along_m,
        }
    };
    let mut others: Vec<&Entry> = entries.iter().filter(|e| !e.is_local).collect();
    others.sort_by(|a, b| {
        gap(a)
            .abs()
            .total_cmp(&gap(b).abs())
            .then(a.id.0.cmp(&b.id.0))
    });
    let mut out = Vec::new();
    let mut push = |e: &Entry, place: String| {
        let mut detail = place;
        if e.flying {
            detail = format!("flying, {detail}");
        } else if e.down {
            detail = format!("down, {detail}");
        }
        out.push(Row {
            id: e.id,
            label: if e.is_local {
                format!("{} (you)", e.name)
            } else {
                e.name.clone()
            },
            detail,
            flying: e.flying,
            dot: e.dot,
        });
    };
    if let Some(m) = me {
        push(m, format!("km {:.1}", m.along_m / 1000.0));
    }
    for e in others {
        let g = gap(e);
        let place = match (me, free) {
            (None, _) => format!("km {:.1}", e.along_m / 1000.0),
            (Some(_), true) => format!("{} away", metres(g)),
            (Some(_), false) if g.abs() < 1.0 => "beside you".to_owned(),
            (Some(_), false) if g > 0.0 => format!("{} ahead", metres(g)),
            (Some(_), false) => format!("{} behind", metres(g)),
        };
        push(e, place);
    }
    out
}

/// The entries of the riders in `view` (the own one marked), who flies
/// from its unicorns, the damage dots from `dot`.
pub fn entries(view: &ocw_render::RideView, dot: impl Fn(RiderId) -> Option<Status>) -> Vec<Entry> {
    view.riders
        .iter()
        .map(|r| {
            let unicorn = view.unicorns.iter().find(|u| u.id == r.id);
            let pos = match unicorn {
                Some(u) => u.pos,
                None => {
                    let p = view.track(r.id).sample(r.state.distance);
                    p.offset_position(r.state.lateral_offset)
                }
            };
            Entry {
                id: r.id,
                name: r.name.clone(),
                is_local: r.is_local,
                along_m: r.state.distance,
                pos,
                flying: unicorn.is_some(),
                down: r.crash.is_some(),
                dot: dot(r.id),
            }
        })
        .collect()
}

/// The list's heading: "5 riders, 1 flying, 1 hurt".
pub fn summary(rows: &[Row]) -> String {
    let flying = rows.iter().filter(|r| r.flying).count();
    let hurt = rows.iter().filter(|r| r.dot.is_some()).count();
    let mut s = format!(
        "{} rider{}",
        rows.len(),
        if rows.len() == 1 { "" } else { "s" }
    );
    if flying > 0 {
        s.push_str(&format!(", {flying} flying"));
    }
    if hurt > 0 {
        s.push_str(&format!(", {hurt} damaged"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u32, name: &str, along_m: f64) -> Entry {
        Entry {
            id: RiderId(id),
            name: name.into(),
            is_local: false,
            along_m,
            pos: [along_m, 0.0, 0.0],
            flying: false,
            down: false,
            dot: None,
        }
    }

    #[test]
    fn rows_name_place_flying_and_damage() {
        let me = Entry {
            is_local: true,
            ..entry(1, "Nils", 12_400.0)
        };
        let ahead = entry(2, "Anna", 12_520.0);
        let behind = Entry {
            dot: Some(Status::Critical),
            down: true,
            ..entry(3, "Bot 3", 10_900.0)
        };
        let flying = Entry {
            flying: true,
            ..entry(4, "Kim", 12_400.4)
        };
        let hurt = Entry {
            dot: Some(Status::Warning),
            ..entry(5, "Bot 5", 12_380.0)
        };
        let r = rows(&[ahead.clone(), behind, me, flying, hurt], false);
        let labels: Vec<&str> = r.iter().map(|r| r.label.as_str()).collect();
        // The own rider first, then by gap.
        assert_eq!(labels, ["Nils (you)", "Kim", "Bot 5", "Anna", "Bot 3"]);
        let details: Vec<&str> = r.iter().map(|r| r.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "km 12.4",
                "flying, beside you",
                "20 m behind",
                "120 m ahead",
                "down, 1.5 km behind"
            ]
        );
        assert!(r[1].flying && !r[2].flying);
        assert_eq!(r[2].dot, Some(Status::Warning));
        assert_eq!(r[4].dot, Some(Status::Critical));
        assert_eq!(summary(&r), "5 riders, 1 flying, 2 damaged");
    }

    #[test]
    fn free_ride_compares_straight_line_distances() {
        let me = Entry {
            is_local: true,
            pos: [0.0, 0.0, 0.0],
            ..entry(1, "Me", 5_000.0)
        };
        let near = Entry {
            pos: [30.0, 5.0, 40.0],
            ..entry(2, "Near", 100.0)
        };
        let far = Entry {
            pos: [3_000.0, 0.0, 0.0],
            ..entry(3, "Far", 5_010.0)
        };
        let r = rows(&[far, near, me], true);
        assert_eq!(r[1].label, "Near");
        assert_eq!(r[1].detail, "50 m away");
        assert_eq!(r[2].detail, "3.0 km away");
        // Without an own rider (spectating): by distance along.
        let r = rows(&[entry(7, "A", 2_000.0)], false);
        assert_eq!(
            (r[0].detail.as_str(), summary(&r).as_str()),
            ("km 2.0", "1 rider")
        );
    }
}

//! Free-ride decisions (spec §2.2.1, §2.2.2, §6.5), shared by the server
//! (authority) and the client's prediction: which exit is taken at the
//! lock point, the bots' choices, and drafting between riders whose paths
//! differ.

use ocw_world::graph::VertexId;
use ocw_world::network::splitmix;
use ocw_world::{Junction, RidePath};

/// Least time between two manual U-turns (spec §2.2.1: rate-limited).
pub const UTURN_INTERVAL_S: f64 = 10.0;

/// A rider's exit choice for a junction (spec §6.5 `TurnIntent`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnChoice {
    pub junction: VertexId,
    /// Exit edge index and its riding direction.
    pub edge: usize,
    pub forward: bool,
}

/// The exit taken at `j`: the choice if it is for this junction and names
/// one of its exits, otherwise the default ("follow the road").
pub fn exit_index(j: &Junction, choice: Option<&TurnChoice>) -> usize {
    choice
        .filter(|c| c.junction == j.vertex)
        .and_then(|c| {
            j.exits
                .iter()
                .position(|e| e.edge == c.edge && e.forward == c.forward)
        })
        .unwrap_or(j.default)
}

/// Extends `path` for a rider at `distance` riding at `speed` (spec §5.2), locking the next
/// junction with `choice` at its lock point. Returns the first changed
/// step index and whether a junction was locked with the choice applying
/// to it (the choice is then used up).
pub fn advance(
    path: &mut RidePath,
    distance: f64,
    speed: f64,
    choice: Option<&TurnChoice>,
) -> (Option<u64>, bool) {
    let mut used = false;
    let changed = path.extend_at_speed(distance, speed, &mut |j| {
        if choice.is_some_and(|c| c.junction == j.vertex) {
            used = true;
        }
        exit_index(j, choice)
    });
    (changed, used)
}

/// A bot's exit (deterministic for a `seed`): on a route always the
/// route's; in free ride mostly the default, else another exit that is not
/// a dead end.
pub fn bot_exit(j: &Junction, seed: u64) -> usize {
    if let Some(i) = j.route {
        return i;
    }
    let r = splitmix(seed);
    if r % 5 < 3 {
        return j.default;
    }
    let live: Vec<usize> = (0..j.exits.len())
        .filter(|&i| !j.exits[i].dead_end)
        .collect();
    if live.is_empty() {
        return j.default;
    }
    live[((r >> 8) % live.len() as u64) as usize]
}

/// Other riders as (distance along `me`'s path, lateral offset), for those
/// on a stretch of road `me` rides too (in the same direction), so the
/// drafting model (spec §5.2) works between different paths.
pub fn others_on_path<'a>(
    me: &RidePath,
    me_m: f64,
    others: impl IntoIterator<Item = (&'a RidePath, f64, f64)>,
) -> Vec<(f64, f64)> {
    others
        .into_iter()
        .filter_map(|(path, d, lateral)| {
            let (edge, forward, offset) = path.locate(d)?;
            let mine = me.distance_of(edge, forward, offset, me_m)?;
            Some((mine, lateral))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use ocw_world::graph::{Edge, EdgeId, Oneway, RoadClass, RoadGraph};
    use ocw_world::{RoadNetwork, Track};

    /// A crossroads at the origin with 300 m arms: west (edge 0), east (1),
    /// north (2), south (3); edges run from the centre outwards.
    pub(crate) fn crossroads() -> RoadGraph {
        let v = [
            [-300.0, 0.0],
            [300.0, 0.0],
            [0.0, -300.0],
            [0.0, 300.0],
            [0.0, 0.0],
        ];
        RoadGraph {
            vertices: v.to_vec(),
            edges: (0..4u32)
                .map(|i| Edge {
                    id: EdgeId(100 + u64::from(i)),
                    from: VertexId(4),
                    to: VertexId(i),
                    points: vec![v[4], v[i as usize]],
                    class: RoadClass::Residential,
                    name: None,
                    width_m: 6.0,
                    bridge: false,
                    tunnel: false,
                    dead_end_m: None,
                    boundary: false,
                    oneway: Oneway::No,
                    osm_way: i64::from(i),
                    profile: Vec::new(),
                })
                .collect(),
            boundary_vertices: Default::default(),
        }
    }

    fn start() -> RidePath {
        let net = Arc::new(RoadNetwork::new(crossroads()));
        // 200 m west of the centre, riding east.
        RidePath::start(net, 0, 200.0, false, 0.0, 0)
    }

    #[test]
    fn the_choice_applies_at_the_lock_point_of_its_junction() {
        let mut p = start();
        let (j, at) = p.next_junction().unwrap();
        assert_eq!(at, 200.0);
        let left = j.exits[0].clone();
        let choice = TurnChoice {
            junction: j.vertex,
            edge: left.edge,
            forward: left.forward,
        };
        // Before the lock point: nothing is decided.
        assert_eq!(advance(&mut p, 150.0, 0.0, Some(&choice)), (None, false));
        let (changed, used) = advance(&mut p, 181.0, 0.0, Some(&choice));
        assert_eq!(changed, Some(1));
        assert!(used);
        // Riding north now.
        assert!(p.sample(260.0).direction[1] < -0.99);
    }

    #[test]
    fn a_choice_for_another_junction_falls_back_to_the_default() {
        let mut p = start();
        let (j, _) = p.next_junction().unwrap();
        let other = TurnChoice {
            junction: VertexId(3),
            edge: 2,
            forward: true,
        };
        assert_eq!(exit_index(&j, Some(&other)), j.default);
        // Not an exit of this junction (back where we came from).
        let back = TurnChoice {
            junction: j.vertex,
            edge: 0,
            forward: true,
        };
        assert_eq!(exit_index(&j, Some(&back)), j.default);
        let (_, used) = advance(&mut p, 181.0, 0.0, Some(&other));
        assert!(!used);
        // Straight on: east.
        assert!(p.sample(260.0).direction[0] > 0.99);
    }

    #[test]
    fn bots_pick_varied_but_deterministic_exits() {
        let p = start();
        let (j, _) = p.next_junction().unwrap();
        let picks: Vec<usize> = (0..60).map(|s| bot_exit(&j, s)).collect();
        assert_eq!(picks, (0..60).map(|s| bot_exit(&j, s)).collect::<Vec<_>>());
        let defaults = picks.iter().filter(|&&i| i == j.default).count();
        assert!(defaults > 20 && defaults < 60, "{defaults}");
        let distinct: std::collections::BTreeSet<_> = picks.iter().collect();
        assert_eq!(distinct.len(), 3);
    }

    #[test]
    fn drafting_positions_map_between_paths() {
        let me = start();
        // Another rider started 30 m further east on the same road, its
        // path distance counting from 5000 m.
        let net = me.network().clone();
        let other = RidePath::start(net.clone(), 0, 170.0, false, 5000.0, 0);
        let mapped = others_on_path(&me, 10.0, [(&other, 5000.0, 1.2)]);
        assert_eq!(mapped.len(), 1);
        assert!((mapped[0].0 - 30.0).abs() < 1e-9, "{mapped:?}");
        assert_eq!(mapped[0].1, 1.2);
        // Riding the other way on that road: no drafting.
        let oncoming = RidePath::start(net, 0, 170.0, true, 0.0, 0);
        assert!(others_on_path(&me, 10.0, [(&oncoming, 0.0, 0.0)]).is_empty());
    }
}

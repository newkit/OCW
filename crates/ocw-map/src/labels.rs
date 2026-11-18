//! Place names on the map – a hook (spec §3.2 *Navigation map*): the
//! names of towns, villages and places come with the world packages'
//! place data (being added); until then a map has no labels. The HUD's
//! navigation map draws whatever [`visible_labels`] returns, the studio
//! map can do the same.

use crate::view::NaviView;

/// A place name on the map.
#[derive(Debug, Clone, PartialEq)]
pub struct PlaceLabel {
    /// Where (home metres, x z).
    pub pos: [f64; 2],
    pub name: String,
    /// 0 = most important (city) … higher = smaller places; the map
    /// shows the important ones first when room is short.
    pub rank: u8,
}

/// The labels inside the map (`inset` points from its edge) with their
/// map points, most important first, at most `max`.
pub fn visible_labels<'a>(
    labels: &'a [PlaceLabel],
    view: &NaviView,
    inset: f32,
    max: usize,
) -> Vec<([f32; 2], &'a PlaceLabel)> {
    let mut v: Vec<_> = labels
        .iter()
        .map(|l| (view.to_screen(l.pos), l))
        .filter(|(s, _)| {
            s[0] >= inset
                && s[1] >= inset
                && s[0] <= view.size[0] - inset
                && s[1] <= view.size[1] - inset
        })
        .collect();
    v.sort_by(|a, b| a.1.rank.cmp(&b.1.rank).then(a.1.name.cmp(&b.1.name)));
    v.truncate(max);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_inside_the_map_most_important_first() {
        let view = NaviView {
            rider: [0.0, 0.0],
            heading: 0.0,
            m_per_px: 1.0,
            size: [200.0, 200.0],
        };
        let l = |x: f64, z: f64, name: &str, rank| PlaceLabel {
            pos: [x, z],
            name: name.into(),
            rank,
        };
        let labels = [
            l(0.0, -50.0, "Hamlet", 3),
            l(10.0, -100.0, "Town", 1),
            l(5000.0, 0.0, "Far away", 0),
        ];
        let v = visible_labels(&labels, &view, 8.0, 5);
        let names: Vec<&str> = v.iter().map(|(_, l)| l.name.as_str()).collect();
        assert_eq!(names, ["Town", "Hamlet"]);
        assert_eq!(v[1].0, [100.0, 90.0]);
        assert_eq!(visible_labels(&labels, &view, 8.0, 1).len(), 1);
    }
}

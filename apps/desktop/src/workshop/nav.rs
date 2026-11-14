//! Hotspot navigation in screen space and pointer hit tests (spec §14
//! Phase 4, 4B *Hotspots*), pure: directional keys and the D-pad move to
//! the nearest hotspot in the pressed direction by the projected
//! positions (recomputed whenever the camera moves); mouse and touch hit
//! the nearest marker within the hit circle, which is never smaller than
//! the touch minimum of §3.2.2 (48 logical px), whatever the drawn size.

use bevy::math::Vec2;

/// The smallest hit circle's diameter (logical px; spec §3.2.2: 9 mm and
/// at least 48 px).
pub const TOUCH_MIN_PX: f32 = 48.0;

/// A direction in screen space (y grows downwards, as on the screen).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Left,
    Right,
    Up,
    Down,
}

impl Dir {
    pub fn vec(self) -> Vec2 {
        match self {
            Dir::Left => Vec2::NEG_X,
            Dir::Right => Vec2::X,
            Dir::Up => Vec2::NEG_Y,
            Dir::Down => Vec2::Y,
        }
    }
}

/// The hotspot to move to from `from` in `dir`: of the points within 60°
/// of the direction, the one with the smallest distance weighted by how
/// far off the direction it is. `points` are screen positions (`None`:
/// behind the camera or hidden).
pub fn next_in_direction(points: &[Option<Vec2>], from: usize, dir: Dir) -> Option<usize> {
    let at = (*points.get(from)?)?;
    let d = dir.vec();
    points
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != from)
        .filter_map(|(i, p)| {
            let v = (*p)? - at;
            let len = v.length();
            if len < 1.0 {
                return None;
            }
            let cos = v.dot(d) / len;
            // Within 60° of the direction.
            (cos > 0.5).then_some((i, len * (1.0 + 2.0 * (1.0 - cos))))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The point hit at `at`: the nearest within `radius` (at least half of
/// [`TOUCH_MIN_PX`]).
pub fn hit(points: &[Option<Vec2>], at: Vec2, radius: f32) -> Option<usize> {
    let r = radius.max(TOUCH_MIN_PX * 0.5);
    points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| {
            let d = (*p)?.distance(at);
            (d <= r).then_some((i, d))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// A screen rectangle (min, max), e.g. a bike's projected box.
pub type Rect = (Vec2, Vec2);

/// The rectangle containing `at`, the smallest if several do (a bike in
/// front of a larger one).
pub fn hit_rect(rects: &[Option<Rect>], at: Vec2) -> Option<usize> {
    rects
        .iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let (a, b) = (*r)?;
            let inside = at.x >= a.x && at.x <= b.x && at.y >= a.y && at.y <= b.y;
            inside.then(|| (i, (b - a).x * (b - a).y))
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The box around projected points (`None` if none is in front).
pub fn bounds(points: impl IntoIterator<Item = Option<Vec2>>) -> Option<Rect> {
    points.into_iter().flatten().fold(None, |acc, p| match acc {
        None => Some((p, p)),
        Some((a, b)) => Some((a.min(p), b.max(p))),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cross of hotspots: centre, left, right, up, down, and a far
    /// diagonal one.
    fn cross() -> Vec<Option<Vec2>> {
        vec![
            Some(Vec2::new(500.0, 500.0)),
            Some(Vec2::new(300.0, 520.0)),
            Some(Vec2::new(700.0, 480.0)),
            Some(Vec2::new(510.0, 300.0)),
            Some(Vec2::new(490.0, 700.0)),
            Some(Vec2::new(900.0, 100.0)),
            None,
        ]
    }

    #[test]
    fn directions_go_to_the_nearest_hotspot_that_way() {
        let p = cross();
        assert_eq!(next_in_direction(&p, 0, Dir::Left), Some(1));
        assert_eq!(next_in_direction(&p, 0, Dir::Right), Some(2));
        assert_eq!(next_in_direction(&p, 0, Dir::Up), Some(3));
        assert_eq!(next_in_direction(&p, 0, Dir::Down), Some(4));
        // From the left one, right goes back to the centre (not the far one).
        assert_eq!(next_in_direction(&p, 1, Dir::Right), Some(0));
        // Nothing further left: stay.
        assert_eq!(next_in_direction(&p, 1, Dir::Left), None);
        // Up from the right one: the nearer one up-left beats the far
        // one up-right; down from the far one comes back.
        assert_eq!(next_in_direction(&p, 2, Dir::Up), Some(3));
        assert_eq!(next_in_direction(&p, 5, Dir::Down), Some(2));
        // A hidden hotspot is never chosen, and from it nothing moves.
        assert_eq!(next_in_direction(&p, 6, Dir::Left), None);
    }

    #[test]
    fn the_projection_decides_so_an_orbit_changes_the_neighbours() {
        // The same three anchors seen from the other side: left and right swap.
        let front = vec![Some(Vec2::new(400.0, 500.0)), Some(Vec2::new(600.0, 500.0))];
        let back = vec![Some(Vec2::new(600.0, 500.0)), Some(Vec2::new(400.0, 500.0))];
        assert_eq!(next_in_direction(&front, 0, Dir::Right), Some(1));
        assert_eq!(next_in_direction(&back, 0, Dir::Right), None);
        assert_eq!(next_in_direction(&back, 0, Dir::Left), Some(1));
    }

    #[test]
    fn hits_use_at_least_the_touch_minimum() {
        let p = cross();
        // Drawn as 8 px dots: a tap 20 px off still hits (24 px radius).
        assert_eq!(hit(&p, Vec2::new(520.0, 500.0), 4.0), Some(0));
        assert_eq!(hit(&p, Vec2::new(530.0, 500.0), 4.0), None);
        // A bigger radius on request.
        assert_eq!(hit(&p, Vec2::new(530.0, 500.0), 40.0), Some(0));
        // The nearest of two.
        assert_eq!(hit(&p, Vec2::new(495.0, 650.0), 60.0), Some(4));
    }

    #[test]
    fn rectangles_prefer_the_smallest() {
        let big = (Vec2::new(0.0, 0.0), Vec2::new(100.0, 100.0));
        let small = (Vec2::new(40.0, 40.0), Vec2::new(60.0, 60.0));
        let r = vec![Some(big), Some(small), None];
        assert_eq!(hit_rect(&r, Vec2::new(50.0, 50.0)), Some(1));
        assert_eq!(hit_rect(&r, Vec2::new(10.0, 10.0)), Some(0));
        assert_eq!(hit_rect(&r, Vec2::new(150.0, 10.0)), None);
        let b = bounds([Some(Vec2::new(5.0, 9.0)), None, Some(Vec2::new(1.0, 20.0))]);
        assert_eq!(b, Some((Vec2::new(1.0, 9.0), Vec2::new(5.0, 20.0))));
        assert_eq!(bounds([None]), None);
    }
}

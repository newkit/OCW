use std::f64::consts::TAU;

/// A sampled point on a route.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoutePoint {
    /// Position of the road centerline in local metres (`x`, `y` = up, `z`).
    pub position: [f64; 3],
    /// Horizontal unit vector pointing in the direction of travel (`x`, `z`).
    pub direction: [f64; 2],
    /// Gradient as rise over run (0.05 = 5 %).
    pub grade: f64,
}

impl RoutePoint {
    /// Horizontal unit vector pointing to the rider's right (`x`, `z`).
    pub fn right(&self) -> [f64; 2] {
        // With y up, rotating the forward vector by -90° around y gives "right".
        [-self.direction[1], self.direction[0]]
    }

    /// Centerline position shifted sideways by `lateral_offset` metres (positive = right).
    pub fn offset_position(&self, lateral_offset: f64) -> [f64; 3] {
        let r = self.right();
        [
            self.position[0] + r[0] * lateral_offset,
            self.position[1],
            self.position[2] + r[1] * lateral_offset,
        ]
    }
}

/// A rideable path: a polyline with cumulative distance, sampled densely.
///
/// Riders move along a route in 1D (distance from the start, spec §5.2); this
/// type turns that distance back into a 3D position, direction and gradient.
#[derive(Debug, Clone)]
pub struct Route {
    id: String,
    name: String,
    points: Vec<[f64; 3]>,
    /// Cumulative horizontal distance at each point; `cumulative[0] == 0`.
    cumulative: Vec<f64>,
    is_loop: bool,
    road_width_m: f64,
}

/// Errors when constructing a route.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteError {
    /// A route needs at least two distinct points.
    TooFewPoints,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::TooFewPoints => write!(f, "a route needs at least two distinct points"),
        }
    }
}

impl std::error::Error for RouteError {}

impl Route {
    /// Builds a route from centerline points. For a loop, the last point is
    /// connected back to the first one.
    pub fn from_points(
        id: impl Into<String>,
        name: impl Into<String>,
        mut points: Vec<[f64; 3]>,
        is_loop: bool,
        road_width_m: f64,
    ) -> Result<Self, RouteError> {
        if is_loop && points.len() > 1 {
            points.push(points[0]);
        }
        let mut cumulative = Vec::with_capacity(points.len());
        let mut total = 0.0;
        for (i, p) in points.iter().enumerate() {
            if i > 0 {
                total += horizontal_distance(&points[i - 1], p);
            }
            cumulative.push(total);
        }
        if points.len() < 2 || total <= 0.0 {
            return Err(RouteError::TooFewPoints);
        }
        Ok(Self {
            id: id.into(),
            name: name.into(),
            points,
            cumulative,
            is_loop,
            road_width_m,
        })
    }

    /// The Phase 1 test route: a rolling ~4 km loop with one climb of about
    /// 30 m, sampled every ~5 m like real roads will be (spec §4.3).
    pub fn test_loop() -> Self {
        const SAMPLES: usize = 900;
        let points = (0..SAMPLES)
            .map(|i| {
                let t = i as f64 / SAMPLES as f64 * TAU;
                let x = 650.0 * t.cos() + 120.0 * (3.0 * t).cos();
                let z = 520.0 * t.sin() + 90.0 * (2.0 * t).sin();
                let y = 15.0 * (1.0 - t.cos()) + 4.0 * (3.0 * t).sin();
                [x, y, z]
            })
            .collect();
        Self::from_points("test-loop", "Test Loop", points, true, 7.0).expect("test loop is valid")
    }

    /// Stable identifier of the route.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Display name of the route.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Total horizontal length in metres.
    pub fn length(&self) -> f64 {
        *self.cumulative.last().expect("route has points")
    }

    /// Whether the route is a closed loop.
    pub fn is_loop(&self) -> bool {
        self.is_loop
    }

    /// Road width in metres.
    pub fn road_width_m(&self) -> f64 {
        self.road_width_m
    }

    /// Centerline points (for a loop, the first point is repeated at the end).
    pub fn points(&self) -> &[[f64; 3]] {
        &self.points
    }

    /// Maps any distance to a distance on the route: wraps for loops, clamps otherwise.
    pub fn normalize_distance(&self, distance: f64) -> f64 {
        let len = self.length();
        if self.is_loop {
            distance.rem_euclid(len)
        } else {
            distance.clamp(0.0, len)
        }
    }

    /// Samples the route at `distance` metres from the start.
    pub fn sample(&self, distance: f64) -> RoutePoint {
        let d = self.normalize_distance(distance);
        // Index of the segment containing d.
        let i = match self
            .cumulative
            .binary_search_by(|c| c.partial_cmp(&d).expect("distances are finite"))
        {
            Ok(i) => i.min(self.points.len() - 2),
            Err(i) => i.saturating_sub(1).min(self.points.len() - 2),
        };
        let (a, b) = (self.points[i], self.points[i + 1]);
        let seg_len = self.cumulative[i + 1] - self.cumulative[i];
        let t = if seg_len > 0.0 {
            (d - self.cumulative[i]) / seg_len
        } else {
            0.0
        };
        let position = [
            a[0] + (b[0] - a[0]) * t,
            a[1] + (b[1] - a[1]) * t,
            a[2] + (b[2] - a[2]) * t,
        ];
        let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
        let horiz = (dx * dx + dz * dz).sqrt();
        let direction = if horiz > 0.0 {
            [dx / horiz, dz / horiz]
        } else {
            [1.0, 0.0]
        };
        let grade = if horiz > 0.0 {
            (b[1] - a[1]) / horiz
        } else {
            0.0
        };
        RoutePoint {
            position,
            direction,
            grade,
        }
    }

    /// Total climbing in metres (sum of positive elevation changes).
    pub fn elevation_gain(&self) -> f64 {
        self.points
            .windows(2)
            .map(|w| (w[1][1] - w[0][1]).max(0.0))
            .sum()
    }
}

fn horizontal_distance(a: &[f64; 3], b: &[f64; 3]) -> f64 {
    let (dx, dz) = (b[0] - a[0], b[2] - a[2]);
    (dx * dx + dz * dz).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight() -> Route {
        Route::from_points(
            "s",
            "Straight",
            vec![[0.0, 0.0, 0.0], [100.0, 5.0, 0.0]],
            false,
            6.0,
        )
        .unwrap()
    }

    #[test]
    fn straight_route_sampling() {
        let r = straight();
        assert_eq!(r.length(), 100.0);
        let p = r.sample(50.0);
        assert!((p.position[0] - 50.0).abs() < 1e-9);
        assert!((p.position[1] - 2.5).abs() < 1e-9);
        assert!((p.grade - 0.05).abs() < 1e-9);
        assert_eq!(p.direction, [1.0, 0.0]);
    }

    #[test]
    fn open_route_clamps() {
        let r = straight();
        assert_eq!(r.sample(-10.0).position, [0.0, 0.0, 0.0]);
        assert_eq!(r.sample(500.0).position, [100.0, 5.0, 0.0]);
    }

    #[test]
    fn right_vector_points_right() {
        // Travelling +x (east) with y up and z south, the right-hand side is +z (south).
        let p = straight().sample(10.0);
        assert_eq!(p.right(), [-0.0, 1.0]);
        assert_eq!(p.offset_position(2.0)[2], 2.0);
    }

    #[test]
    fn test_loop_properties() {
        let r = Route::test_loop();
        assert!(r.is_loop());
        assert!(
            (3500.0..4500.0).contains(&r.length()),
            "length {}",
            r.length()
        );
        // Wraps around.
        let a = r.sample(10.0).position;
        let b = r.sample(10.0 + r.length()).position;
        for k in 0..3 {
            assert!((a[k] - b[k]).abs() < 1e-6);
        }
        // Rolling, but rideable.
        let max_grade = (0..1000)
            .map(|i| r.sample(i as f64 * r.length() / 1000.0).grade.abs())
            .fold(0.0, f64::max);
        assert!(max_grade < 0.08, "max grade {max_grade}");
        assert!(r.elevation_gain() > 25.0);
    }

    #[test]
    fn too_few_points() {
        assert_eq!(
            Route::from_points("x", "x", vec![[0.0; 3]], false, 5.0).unwrap_err(),
            RouteError::TooFewPoints
        );
    }
}

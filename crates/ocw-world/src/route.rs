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
    /// Direction at each point: the mean of the adjacent segments' directions.
    /// `sample` interpolates between these, so the heading turns smoothly
    /// instead of jumping at every point.
    vertex_direction: Vec<[f64; 2]>,
    /// Gradient at each point, the mean of the adjacent segments' gradients.
    vertex_grade: Vec<f64>,
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
        let (vertex_direction, vertex_grade) = vertex_attributes(&points, is_loop);
        Ok(Self {
            id: id.into(),
            name: name.into(),
            points,
            cumulative,
            vertex_direction,
            vertex_grade,
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
        let (da, db) = (self.vertex_direction[i], self.vertex_direction[i + 1]);
        let direction =
            normalize([da[0] + (db[0] - da[0]) * t, da[1] + (db[1] - da[1]) * t]).unwrap_or(da);
        let grade = self.vertex_grade[i] + (self.vertex_grade[i + 1] - self.vertex_grade[i]) * t;
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

fn normalize(v: [f64; 2]) -> Option<[f64; 2]> {
    let len = (v[0] * v[0] + v[1] * v[1]).sqrt();
    (len > 1e-12).then(|| [v[0] / len, v[1] / len])
}

/// Per-point direction and gradient, averaged over the adjacent segments
/// (wrapping around for loops, whose last point repeats the first).
fn vertex_attributes(points: &[[f64; 3]], is_loop: bool) -> (Vec<[f64; 2]>, Vec<f64>) {
    let segments: Vec<([f64; 2], f64)> = points
        .windows(2)
        .map(|w| {
            let (dx, dz) = (w[1][0] - w[0][0], w[1][2] - w[0][2]);
            let horiz = (dx * dx + dz * dz).sqrt();
            match normalize([dx, dz]) {
                Some(dir) => (dir, (w[1][1] - w[0][1]) / horiz),
                None => ([0.0, 0.0], 0.0), // duplicate point: ignored below
            }
        })
        .collect();
    let n = segments.len();
    let mut directions = Vec::with_capacity(n + 1);
    let mut grades = Vec::with_capacity(n + 1);
    for v in 0..=n {
        let before = if v > 0 {
            Some(v - 1)
        } else if is_loop {
            Some(n - 1)
        } else {
            None
        };
        let after = if v < n {
            Some(v)
        } else if is_loop {
            Some(0)
        } else {
            None
        };
        let adjacent: Vec<&([f64; 2], f64)> = [before, after]
            .into_iter()
            .flatten()
            .map(|s| &segments[s])
            .filter(|(d, _)| *d != [0.0, 0.0])
            .collect();
        let sum = adjacent
            .iter()
            .fold([0.0, 0.0], |acc, (d, _)| [acc[0] + d[0], acc[1] + d[1]]);
        // A U-turn cancels out: fall back to the incoming direction.
        let dir = normalize(sum).unwrap_or(adjacent.first().map_or([1.0, 0.0], |s| s.0));
        directions.push(dir);
        grades.push(if adjacent.is_empty() {
            0.0
        } else {
            adjacent.iter().map(|s| s.1).sum::<f64>() / adjacent.len() as f64
        });
    }
    (directions, grades)
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

    /// The heading and gradient change continuously along the route, so the
    /// rider and camera do not jerk at every route point.
    #[test]
    fn direction_and_grade_are_continuous() {
        let r = Route::test_loop();
        let step = 0.05;
        let (mut max_turn, mut max_grade_step) = (0.0f64, 0.0f64);
        let mut prev = r.sample(0.0);
        let mut d = step;
        // Include the loop seam.
        while d <= r.length() + 50.0 {
            let p = r.sample(d);
            let cross = prev.direction[0] * p.direction[1] - prev.direction[1] * p.direction[0];
            let dot = prev.direction[0] * p.direction[0] + prev.direction[1] * p.direction[1];
            max_turn = max_turn.max(cross.atan2(dot).abs().to_degrees());
            max_grade_step = max_grade_step.max((p.grade - prev.grade).abs());
            prev = p;
            d += step;
        }
        // Was up to 2 degrees in one step with per-segment directions.
        assert!(max_turn < 0.05, "turn of {max_turn:.3} deg in {step} m");
        assert!(max_grade_step < 0.001, "grade step {max_grade_step}");
    }

    #[test]
    fn vertex_direction_is_the_mean_of_both_segments() {
        // A 90-degree corner: east, then south.
        let r = Route::from_points(
            "c",
            "Corner",
            vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [10.0, 1.0, 10.0]],
            false,
            6.0,
        )
        .unwrap();
        let s = std::f64::consts::FRAC_1_SQRT_2;
        let at_corner = r.sample(10.0).direction;
        assert!((at_corner[0] - s).abs() < 1e-9 && (at_corner[1] - s).abs() < 1e-9);
        // Ends keep their segment's direction and gradient.
        assert_eq!(r.sample(0.0).direction, [1.0, 0.0]);
        assert_eq!(r.sample(20.0).direction, [0.0, 1.0]);
        assert!((r.sample(20.0).grade - 0.1).abs() < 1e-9);
        assert!((r.sample(10.0).grade - 0.05).abs() < 1e-9);
    }

    #[test]
    fn u_turn_does_not_produce_nan() {
        let r = Route::from_points(
            "u",
            "U-turn",
            vec![[0.0, 0.0, 0.0], [10.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
            false,
            6.0,
        )
        .unwrap();
        let p = r.sample(10.0);
        assert!(p.direction[0].is_finite() && p.direction[1].is_finite());
        assert!((p.direction[0].hypot(p.direction[1]) - 1.0).abs() < 1e-9);
    }
}

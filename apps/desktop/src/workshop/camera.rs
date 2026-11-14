//! The workshop scene's camera (spec §14 Phase 4, 4A *Camera states*):
//! a pure state machine. Every change of state is an eased move of
//! [`MOVE_S`] from wherever the camera is (no cuts); the stand view
//! orbits and zooms within the limits of §3.2.2 (0.8–4 m, pitch −10° …
//! 70°) with a short inertia after a flick.

use bevy::math::{Mat4, Vec2, Vec3};
use ocw_assets::workshop::Layout;

/// Length of a camera move between two states (s).
pub const MOVE_S: f32 = 0.6;
/// Vertical field of view (rad); the renderer uses the same.
pub const FOV_Y: f32 = 0.75;
/// Orbit distance limits around the stand bike (m).
pub const MIN_DIST: f32 = 0.8;
pub const MAX_DIST: f32 = 4.0;
/// Orbit pitch limits (rad): −10° … 70°.
pub const MIN_PITCH: f32 = -10.0 * std::f32::consts::PI / 180.0;
pub const MAX_PITCH: f32 = 70.0 * std::f32::consts::PI / 180.0;
/// A flick keeps turning for this long (s).
pub const INERTIA_S: f32 = 0.3;
/// The part close-up: the camera this far from the part (m).
pub const PART_DIST: f32 = 1.1;

/// Where the camera is and what it looks at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub eye: Vec3,
    pub target: Vec3,
}

impl Pose {
    pub fn lerp(self, o: Pose, t: f32) -> Pose {
        Pose {
            eye: self.eye.lerp(o.eye, t),
            target: self.target.lerp(o.target, t),
        }
    }

    /// The view-projection matrix for a window of `aspect` (width / height).
    // glam's older names: the same matrices as the new free functions.
    #[allow(deprecated)]
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(FOV_Y, aspect.max(0.1), 0.05, 100.0)
            * Mat4::look_at_rh(self.eye, self.target, Vec3::Y)
    }

    /// `p` in normalised device coordinates (x right, y up, −1 … 1), or
    /// `None` behind the camera.
    pub fn project(&self, aspect: f32, p: Vec3) -> Option<Vec2> {
        let c = self.view_proj(aspect) * p.extend(1.0);
        (c.w > 1e-4).then(|| Vec2::new(c.x / c.w, c.y / c.w))
    }

    /// `p` in window pixels (origin top left) for a window of `size`.
    pub fn to_screen(self, size: Vec2, p: Vec3) -> Option<Vec2> {
        let n = self.project(size.x / size.y.max(1.0), p)?;
        Some(Vec2::new(
            (n.x + 1.0) * 0.5 * size.x,
            (1.0 - n.y) * 0.5 * size.y,
        ))
    }
}

/// Smooth start and stop (smoothstep).
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The orbit around the stand bike: yaw (0 = from the front of the room,
/// +Z), pitch (up) and distance, with inertia.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Orbit {
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    /// Turn rate after a flick (rad/s: yaw, pitch) and its time left.
    spin: Vec2,
    spin_left: f32,
}

impl Default for Orbit {
    fn default() -> Self {
        Self {
            yaw: 0.13,
            pitch: 0.18,
            dist: 1.9,
            spin: Vec2::ZERO,
            spin_left: 0.0,
        }
    }
}

impl Orbit {
    /// Turns by `delta` (rad: yaw, pitch); `dt` > 0 makes it a flick
    /// that keeps turning for [`INERTIA_S`].
    pub fn turn(&mut self, delta: Vec2, dt: f32) {
        self.yaw += delta.x;
        self.pitch = (self.pitch + delta.y).clamp(MIN_PITCH, MAX_PITCH);
        if dt > 0.0 && delta != Vec2::ZERO {
            self.spin = delta / dt;
            self.spin_left = INERTIA_S;
        }
    }

    /// Zooms by `steps` (positive = closer), within the limits.
    pub fn zoom(&mut self, steps: f32) {
        self.dist = (self.dist * 0.88f32.powf(steps)).clamp(MIN_DIST, MAX_DIST);
    }

    /// The inertia's share of this frame.
    pub fn update(&mut self, dt: f32) {
        if self.spin_left <= 0.0 {
            return;
        }
        let k = self.spin_left / INERTIA_S;
        let step = self.spin * dt.min(self.spin_left) * k;
        self.yaw += step.x;
        self.pitch = (self.pitch + step.y).clamp(MIN_PITCH, MAX_PITCH);
        self.spin_left -= dt;
    }

    #[cfg(test)]
    pub fn spinning(&self) -> bool {
        self.spin_left > 0.0
    }

    /// The eye around `centre`.
    pub fn eye(&self, centre: Vec3) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        centre + Vec3::new(sy * cp, sp, cy * cp) * self.dist
    }
}

/// The camera states (spec §14 Phase 4, 4A).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum View {
    /// On entering: stand and wall in view.
    Overview,
    /// Panned along the wall to hook `i`.
    Wall(usize),
    /// Close to the bike on the stand, orbiting.
    Stand,
    /// Flown to a part at `anchor` (world), framed at the left third.
    Part(Vec3),
    /// The workbench with the parts shelf (4D).
    Bench,
    /// The changing corner (4C).
    Mirror,
}

/// The fixed poses of the room's views.
pub struct Poses<'a> {
    pub layout: &'a Layout,
}

impl Poses<'_> {
    /// The middle of the bike on the stand (the orbit's centre).
    pub fn stand_centre(&self) -> Vec3 {
        self.layout.stand_bike.0 + Vec3::Y * 0.55
    }

    pub fn pose(&self, view: View, orbit: &Orbit) -> Pose {
        let l = self.layout;
        let stand = self.stand_centre();
        let from = |v: ocw_assets::workshop::View| Pose {
            eye: v.eye,
            target: v.target,
        };
        match view {
            View::Overview => from(l.overview_view()),
            View::Wall(i) => {
                // Further back than the layout's view: the whole hung bike and
                // its card (above the hint bar) in the picture.
                let v = l.wall_view(i.min(l.hooks.len().saturating_sub(1)));
                Pose {
                    eye: Vec3::new(v.eye.x, 1.45, v.eye.z + 1.6),
                    target: Vec3::new(v.target.x, 1.35, v.target.z),
                }
            }
            View::Stand => Pose {
                eye: orbit.eye(stand),
                target: stand,
            },
            View::Part(anchor) => {
                // From the orbit's direction, close, the part at the left
                // third (the picker takes the right 40 %).
                let dir = (orbit.eye(stand) - stand).normalize_or(Vec3::Z);
                let eye = anchor + dir * PART_DIST;
                let right = (anchor - eye).cross(Vec3::Y).normalize_or(Vec3::X);
                let shift = PART_DIST * (FOV_Y * 0.5).tan() * (16.0 / 9.0) * 0.35;
                Pose {
                    eye: eye + right * shift,
                    target: anchor + right * shift,
                }
            }
            View::Bench => from(l.bench_view()),
            View::Mirror => from(l.mirror_view()),
        }
    }
}

/// The camera machine: the current view and the eased move to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CameraMachine {
    pub view: View,
    from: Pose,
    t: f32,
}

impl CameraMachine {
    /// Starts at `view` without a move.
    pub fn new(view: View, at: Pose) -> Self {
        Self {
            view,
            from: at,
            t: MOVE_S,
        }
    }

    /// Moves to `view`, starting from `current` (the pose shown now).
    pub fn go(&mut self, view: View, current: Pose) {
        if view == self.view {
            return;
        }
        self.view = view;
        self.from = current;
        self.t = 0.0;
    }

    pub fn update(&mut self, dt: f32) {
        self.t = (self.t + dt.max(0.0)).min(MOVE_S);
    }

    /// Still moving between two states.
    pub fn moving(&self) -> bool {
        self.t < MOVE_S
    }

    /// The pose to show, with `goal` the current view's pose (the stand's
    /// moves with the orbit).
    pub fn pose(&self, goal: Pose) -> Pose {
        self.from.lerp(goal, ease(self.t / MOVE_S))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        ocw_assets::workshop::layout(6)
    }

    #[test]
    fn moves_are_eased_and_never_cut() {
        let l = layout();
        let p = Poses { layout: &l };
        let o = Orbit::default();
        let mut m = CameraMachine::new(View::Overview, p.pose(View::Overview, &o));
        let start = m.pose(p.pose(View::Overview, &o));
        m.go(View::Stand, start);
        // The first frame after the change still shows where it was.
        assert_eq!(m.pose(p.pose(View::Stand, &o)), start);
        let mut last = start;
        let mut frames = 0;
        while m.moving() {
            m.update(1.0 / 60.0);
            let now = m.pose(p.pose(View::Stand, &o));
            // No jumps: at most a few centimetres per frame.
            assert!(now.eye.distance(last.eye) < 0.2, "{frames}");
            last = now;
            frames += 1;
        }
        assert!((35..=37).contains(&frames), "0.6 s at 60 fps: {frames}");
        assert_eq!(last, p.pose(View::Stand, &o));
        // Going to the same view again does not restart the move.
        m.go(View::Stand, last);
        assert!(!m.moving());
    }

    #[test]
    fn the_orbit_keeps_its_limits_and_coasts() {
        let mut o = Orbit::default();
        o.zoom(100.0);
        assert_eq!(o.dist, MIN_DIST);
        o.zoom(-100.0);
        assert_eq!(o.dist, MAX_DIST);
        o.turn(Vec2::new(0.0, 10.0), 0.0);
        assert_eq!(o.pitch, MAX_PITCH);
        o.turn(Vec2::new(0.0, -10.0), 0.0);
        assert_eq!(o.pitch, MIN_PITCH);
        // A flick coasts for 0.3 s, then stops.
        let mut o = Orbit::default();
        o.turn(Vec2::new(0.1, 0.0), 1.0 / 60.0);
        let after_flick = o.yaw;
        for _ in 0..10 {
            o.update(1.0 / 60.0);
        }
        assert!(o.yaw > after_flick, "inertia");
        for _ in 0..30 {
            o.update(1.0 / 60.0);
        }
        assert!(!o.spinning());
        let still = o.yaw;
        o.update(1.0 / 60.0);
        assert_eq!(o.yaw, still);
        // The eye is at the orbit's distance from the centre.
        let c = Vec3::new(1.0, 1.0, 1.0);
        assert!((o.eye(c).distance(c) - o.dist).abs() < 1e-4);
    }

    #[test]
    fn every_view_looks_at_its_place() {
        let l = layout();
        let p = Poses { layout: &l };
        let o = Orbit::default();
        let aspect = 16.0 / 9.0;
        // The stand bike is in the middle of the overview and the stand view.
        for v in [View::Overview, View::Stand] {
            let n = p.pose(v, &o).project(aspect, p.stand_centre()).unwrap();
            assert!(n.x.abs() < 0.3 && n.y.abs() < 0.6, "{v:?}: {n}");
        }
        // Each hook's bike is in the middle of its wall view.
        for (i, h) in l.hooks.iter().enumerate() {
            let n = p.pose(View::Wall(i), &o).project(aspect, h.bike.0).unwrap();
            assert!(n.length() < 0.3, "hook {i}: {n}");
        }
        // A part close-up puts the part at the left third.
        let anchor = p.stand_centre() + Vec3::new(0.3, 0.1, 0.0);
        let n = p
            .pose(View::Part(anchor), &o)
            .project(aspect, anchor)
            .unwrap();
        assert!((-0.6..-0.15).contains(&n.x) && n.y.abs() < 0.1, "{n}");
        for v in [View::Bench, View::Mirror] {
            let pose = p.pose(v, &o);
            assert!(pose.eye.distance(pose.target) > 1.0, "{v:?}");
        }
    }

    #[test]
    fn projection_matches_the_window() {
        let pose = Pose {
            eye: Vec3::new(0.0, 0.0, 5.0),
            target: Vec3::ZERO,
        };
        let size = Vec2::new(1920.0, 1080.0);
        let c = pose.to_screen(size, Vec3::ZERO).unwrap();
        assert!(c.distance(size / 2.0) < 1e-3);
        // Right is right, up is up (screen y grows downwards).
        let r = pose.to_screen(size, Vec3::X).unwrap();
        let u = pose.to_screen(size, Vec3::Y).unwrap();
        assert!(r.x > c.x && u.y < c.y);
        assert!(pose.project(1.0, Vec3::new(0.0, 0.0, 10.0)).is_none());
    }
}

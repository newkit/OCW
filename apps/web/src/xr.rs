//! WebXR's pure side (spec §14 Phase 14, 14I W1): the immersive session's
//! state, the frames the page hands over and the eyes' cameras – no Bevy,
//! no browser, unit-tested natively.
//!
//! The page (`web/ocw-xr.js`) runs the session: the *Enter VR* button
//! (a user gesture), `navigator.xr.requestSession('immersive-vr')`, the
//! `XRWebGLLayer` on the app's WebGL 2 context. In each frame of the
//! session's `requestAnimationFrame` it hands the viewer's and the eyes'
//! poses and projections over ([`parse_frame`]: one `Float32Array`), steps
//! the app (Bevy's loop waits on the window's `requestAnimationFrame`,
//! which the page runs from the XR frame while the session lasts), and
//! copies the canvas into the layer's framebuffer. The Bevy side
//! (`webxr.rs`, feature `webxr`) puts two eye cameras where
//! [`rig_eye`] says, with [`eye_clip_from_view`]'s projection, into the
//! canvas parts [`View::canvas_rect`] gives.
//!
//! Conventions: WebXR's matrices are column-major `Float32Array`s, OpenGL
//! clip space (depth −1 … 1, finite far plane); its poses are metres in the
//! reference space (`local-floor`: y up from the floor, −z ahead at the
//! start), quaternions `[x, y, z, w]`. Bevy draws right-handed, −Z ahead,
//! with an **infinite reverse-Z** projection (depth 1 at the near plane, 0
//! at infinity) – the same as the desktop VR mode's eyes (Phase 11,
//! `ocw-xr::projection` on branch `agent/vr-x1`, which W2 shares).

/// The layout version of the page's frame data (`ocw-xr.js`, `frameData`).
pub const FRAME_LAYOUT: f32 = 1.0;
/// Floats before the first view: layout, view count, framebuffer width
/// and height (px), the frame's time and the page's time for the previous
/// frame (ms), the viewer's position (3) and orientation (4).
pub const HEADER_LEN: usize = 13;
/// Floats per view: the eye (0 none, 1 left, 2 right), its viewport in the
/// framebuffer (x, y, width, height in px, OpenGL's origin bottom left),
/// the projection (16, column-major), the eye's position (3) and
/// orientation (4).
pub const VIEW_LEN: usize = 28;
/// More views than this are refused (a headset has two, a CAVE more; the
/// spike draws two).
pub const MAX_VIEWS: usize = 4;
/// The eyes' near plane (m): a hand on the bars is 0.3 m away.
pub const EYE_NEAR_M: f32 = 0.05;

/// The immersive session's state (spec 14I W1: offered → entering →
/// running → visible-blurred → ended).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum XrState {
    /// Not known yet: the page has not asked the browser.
    #[default]
    Unknown,
    /// The browser has no immersive VR (no `navigator.xr`, no headset, not
    /// a secure context): the flat view only, the page says why.
    Unsupported,
    /// The *Enter VR* button is shown.
    Offered,
    /// Pressed: the session is being requested and set up.
    Entering,
    /// In the headset, frames from the session.
    Running,
    /// Still drawing, but the system's UI has the input
    /// (`visibilityState` `visible-blurred`).
    Blurred,
    /// Not shown (`hidden`): the session sends no frames.
    Hidden,
    /// Ended (by the user, the page or the browser): back to the flat
    /// view; the button is offered again.
    Ended,
}

/// What happens to the session (from the page, `window.ocw.xrEvent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XrEvent {
    /// `isSessionSupported('immersive-vr')` answered.
    Supported(bool),
    /// The *Enter VR* button was pressed.
    Enter,
    /// The session runs (the layer is set, the first frame was asked for).
    Started,
    /// Entering failed (refused, no layer): the reason.
    Failed(String),
    /// The session's `visibilityState` changed.
    Visibility(Visibility),
    /// The session ended.
    Ended,
}

/// WebXR's `XRVisibilityState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    VisibleBlurred,
    Hidden,
}

impl XrEvent {
    /// The page's words (`window.ocw.xrEvent(name, detail)`).
    pub fn parse(name: &str, detail: &str) -> Option<Self> {
        Some(match name {
            "supported" => Self::Supported(detail == "true"),
            "enter" => Self::Enter,
            "started" => Self::Started,
            "failed" => Self::Failed(detail.to_owned()),
            "visibility" => Self::Visibility(match detail {
                "visible" => Visibility::Visible,
                "visible-blurred" => Visibility::VisibleBlurred,
                "hidden" => Visibility::Hidden,
                _ => return None,
            }),
            "ended" => Self::Ended,
            _ => return None,
        })
    }
}

impl XrState {
    /// The state after `event`; an event that does not fit the state
    /// leaves it as it is (`Err` says why, for the log).
    pub fn next(self, event: &XrEvent) -> Result<Self, String> {
        use XrState::*;
        let to = match (self, event) {
            (_, XrEvent::Supported(false)) => Unsupported,
            (Unknown | Unsupported | Ended, XrEvent::Supported(true)) => Offered,
            (Offered | Ended, XrEvent::Enter) => Entering,
            (Entering, XrEvent::Started) => Running,
            (Entering, XrEvent::Failed(_)) => Offered,
            (Running | Blurred | Hidden, XrEvent::Visibility(v)) => match v {
                Visibility::Visible => Running,
                Visibility::VisibleBlurred => Blurred,
                Visibility::Hidden => Hidden,
            },
            (Entering | Running | Blurred | Hidden, XrEvent::Ended) => Ended,
            (s, e) => return Err(format!("xr: {e:?} in state {s:?} ignored")),
        };
        Ok(to)
    }

    /// Whether the eyes are drawn (frames come from the session).
    pub fn in_headset(self) -> bool {
        matches!(self, Self::Running | Self::Blurred)
    }

    /// Whether the session exists (the flat view is not shown).
    pub fn in_session(self) -> bool {
        matches!(self, Self::Running | Self::Blurred | Self::Hidden)
    }

    /// The state's name (the page's button, `window.ocw.xr()`).
    pub fn name(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Unsupported => "unsupported",
            Self::Offered => "offered",
            Self::Entering => "entering",
            Self::Running => "running",
            Self::Blurred => "visible-blurred",
            Self::Hidden => "hidden",
            Self::Ended => "ended",
        }
    }
}

/// Which eye a view is (WebXR's `XREye`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eye {
    None,
    Left,
    Right,
}

/// A position and an orientation (`[x, y, z, w]`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: [f32; 3],
    pub orientation: [f32; 4],
}

impl Pose {
    pub const IDENTITY: Self = Self {
        position: [0.0; 3],
        orientation: [0.0, 0.0, 0.0, 1.0],
    };
}

/// One view of a frame (an eye).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub eye: Eye,
    /// In the layer's framebuffer: x, y, width, height (px, OpenGL's
    /// origin at the bottom left).
    pub viewport: [f32; 4],
    /// WebXR's projection (column-major, OpenGL clip space).
    pub projection: [f32; 16],
    /// The eye in the reference space.
    pub pose: Pose,
}

/// One XR frame as the page hands it over.
#[derive(Debug, Clone, PartialEq)]
pub struct XrFrame {
    /// The layer's framebuffer (px).
    pub framebuffer: [f32; 2],
    /// The frame's time (ms, the page's clock).
    pub time_ms: f64,
    /// How long the page's previous XR frame took (ms: the app's step,
    /// rendering and the copy), 0 for the first.
    pub previous_frame_ms: f32,
    /// The head (WebXR's viewer) in the reference space.
    pub viewer: Pose,
    pub views: Vec<View>,
}

fn pose_at(d: &[f32]) -> Pose {
    Pose {
        position: [d[0], d[1], d[2]],
        orientation: [d[3], d[4], d[5], d[6]],
    }
}

/// Reads the page's frame data (`ocw-xr.js`: [`HEADER_LEN`] floats, then
/// [`VIEW_LEN`] per view).
pub fn parse_frame(d: &[f32]) -> Result<XrFrame, String> {
    if d.len() < HEADER_LEN {
        return Err(format!(
            "xr frame: {} floats, the header has {HEADER_LEN}",
            d.len()
        ));
    }
    if d[0] != FRAME_LAYOUT {
        return Err(format!(
            "xr frame: layout {} (this build reads {FRAME_LAYOUT})",
            d[0]
        ));
    }
    let n = d[1] as usize;
    if d[1] < 0.0 || d[1].fract() != 0.0 || n > MAX_VIEWS {
        return Err(format!("xr frame: {} views", d[1]));
    }
    if d.len() != HEADER_LEN + n * VIEW_LEN {
        return Err(format!(
            "xr frame: {} floats for {n} views (want {})",
            d.len(),
            HEADER_LEN + n * VIEW_LEN
        ));
    }
    if d.iter().any(|v| !v.is_finite()) {
        return Err("xr frame: a value is not finite".into());
    }
    let views = (0..n)
        .map(|i| {
            let v = &d[HEADER_LEN + i * VIEW_LEN..HEADER_LEN + (i + 1) * VIEW_LEN];
            let eye = match v[0] as i32 {
                1 => Eye::Left,
                2 => Eye::Right,
                _ => Eye::None,
            };
            let mut projection = [0.0; 16];
            projection.copy_from_slice(&v[5..21]);
            View {
                eye,
                viewport: [v[1], v[2], v[3], v[4]],
                projection,
                pose: pose_at(&v[21..28]),
            }
        })
        .collect();
    Ok(XrFrame {
        framebuffer: [d[2], d[3]],
        time_ms: f64::from(d[4]),
        previous_frame_ms: d[5],
        viewer: pose_at(&d[6..13]),
        views,
    })
}

impl View {
    /// Where the eye goes on the canvas, as fractions of its size with the
    /// origin at the **top** left (Bevy's viewports): the page copies the
    /// whole canvas onto the whole framebuffer, so each eye lands in its
    /// viewport whatever the canvas' size. `None` for an empty viewport
    /// (the emulator's mono mode gives the right eye no width).
    pub fn canvas_rect(&self, framebuffer: [f32; 2]) -> Option<[f32; 4]> {
        let [x, y, w, h] = self.viewport;
        let [fw, fh] = framebuffer;
        if w < 1.0 || h < 1.0 || fw < 1.0 || fh < 1.0 {
            return None;
        }
        let x0 = (x / fw).clamp(0.0, 1.0);
        let x1 = ((x + w) / fw).clamp(0.0, 1.0);
        // OpenGL's y runs up from the bottom; Bevy's down from the top.
        let top = (1.0 - (y + h) / fh).clamp(0.0, 1.0);
        let bottom = (1.0 - y / fh).clamp(0.0, 1.0);
        (x1 > x0 && bottom > top).then_some([x0, top, x1 - x0, bottom - top])
    }
}

/// Pixels of a render target of `size` for a [`View::canvas_rect`]:
/// position and size (at least 1 × 1, inside the target).
pub fn rect_pixels(rect: [f32; 4], size: [u32; 2]) -> ([u32; 2], [u32; 2]) {
    let [w, h] = [size[0].max(1) as f32, size[1].max(1) as f32];
    let x = (rect[0] * w).round().clamp(0.0, w - 1.0);
    let y = (rect[1] * h).round().clamp(0.0, h - 1.0);
    let x1 = ((rect[0] + rect[2]) * w).round().clamp(x + 1.0, w);
    let y1 = ((rect[1] + rect[3]) * h).round().clamp(y + 1.0, h);
    ([x as u32, y as u32], [(x1 - x) as u32, (y1 - y) as u32])
}

/// The tangents of an eye's frustum edges (left, right, down, up; left
/// and down negative for a view through its centre) from an OpenGL
/// perspective matrix (`glFrustum`: column-major, `m[0]` = 2n/(r−l),
/// `m[8]` = (r+l)/(r−l), `m[5]` = 2n/(t−b), `m[9]` = (t+b)/(t−b)).
pub fn frustum_tangents(m: &[f32; 16]) -> Result<[f32; 4], String> {
    if m[0] <= 0.0 || m[5] <= 0.0 || m[11] != -1.0 || m[15] != 0.0 {
        return Err(format!("xr: not a perspective projection: {m:?}"));
    }
    let left = (m[8] - 1.0) / m[0];
    let right = (m[8] + 1.0) / m[0];
    let down = (m[9] - 1.0) / m[5];
    let up = (m[9] + 1.0) / m[5];
    Ok([left, right, down, up])
}

/// Bevy's projection for the frustum of `tangents` ([`frustum_tangents`]):
/// the asymmetric form of glam's right-handed
/// `perspective_infinite_reverse` (column-major) – depth 1 at `near`, 0 at
/// infinity. For a symmetric frustum it is glam's matrix (tested).
pub fn clip_from_view(tangents: [f32; 4], near: f32) -> [f32; 16] {
    let [l, r, d, u] = tangents;
    let (w, h) = (r - l, u - d);
    [
        2.0 / w,
        0.0,
        0.0,
        0.0,
        //
        0.0,
        2.0 / h,
        0.0,
        0.0,
        //
        (r + l) / w,
        (u + d) / h,
        0.0,
        -1.0,
        //
        0.0,
        0.0,
        near,
        0.0,
    ]
}

/// The eye's Bevy projection from WebXR's (the frustum kept, our near
/// plane and an infinite far one).
pub fn eye_clip_from_view(webxr: &[f32; 16]) -> Result<[f32; 16], String> {
    Ok(clip_from_view(frustum_tangents(webxr)?, EYE_NEAR_M))
}

/// The frustum's eight corners at the distances `near` and `far` (view
/// space, −Z ahead): bottom right, top right, top left, bottom left at
/// `near`, then the same at `far` (Bevy's `get_frustum_corners` order:
/// the shadow cascades are fitted to them).
pub fn frustum_corners(tangents: [f32; 4], near: f32, far: f32) -> [[f32; 3]; 8] {
    let [l, r, d, u] = tangents;
    let at = |z: f32| {
        [
            [r * z, d * z, -z],
            [r * z, u * z, -z],
            [l * z, u * z, -z],
            [l * z, d * z, -z],
        ]
    };
    let (a, b) = (at(near), at(far));
    [a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3]]
}

fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    let [ax, ay, az, aw] = a;
    let [bx, by, bz, bw] = b;
    [
        aw * bx + ax * bw + ay * bz - az * by,
        aw * by - ax * bz + ay * bw + az * bx,
        aw * bz + ax * by - ay * bx + az * bw,
        aw * bw - ax * bx - ay * by - az * bz,
    ]
}

fn quat_normalized(q: [f32; 4]) -> [f32; 4] {
    let n = q.iter().map(|v| v * v).sum::<f32>().sqrt();
    if n > 1e-6 {
        q.map(|v| v / n)
    } else {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// A turn about +Y (up) by `yaw` rad (counter-clockwise seen from above).
pub fn yaw_quat(yaw: f32) -> [f32; 4] {
    let (s, c) = (0.5 * yaw).sin_cos();
    [0.0, s, 0.0, c]
}

/// The turn about +Y of an orientation (its heading; the pitch and roll
/// dropped): the angle of its forward (−Z) direction in the ground plane.
pub fn yaw_of(q: [f32; 4]) -> f32 {
    let [x, y, z, w] = quat_normalized(q);
    // The rotated −Z axis: −(third column of the rotation matrix).
    let fx = -(2.0 * (x * z + w * y));
    let fz = -(1.0 - 2.0 * (x * x + y * y));
    (-fx).atan2(-fz)
}

/// Where an eye is in the world: the reference space's origin (the floor
/// under the saddle for `local-floor`) stands at `origin` turned by
/// `origin_yaw` (the bike's heading: the head is never pitched or rolled
/// by the road – comfort, Phase 11), the eye at `eye` in that space.
pub fn rig_eye(origin: [f32; 3], origin_yaw: f32, eye: Pose) -> Pose {
    let turn = yaw_quat(origin_yaw);
    let p = eye.position;
    let (s, c) = origin_yaw.sin_cos();
    // Rotating (x, z) counter-clockwise about +Y seen from above.
    let rotated = [c * p[0] + s * p[2], p[1], -s * p[0] + c * p[2]];
    Pose {
        position: [
            origin[0] + rotated[0],
            origin[1] + rotated[1],
            origin[2] + rotated[2],
        ],
        orientation: quat_normalized(quat_mul(turn, eye.orientation)),
    }
}

/// How far the eyes are ahead of the head's centre (m).
pub const EYES_AHEAD_OF_HEAD_M: f32 = 0.1;

/// The point on the floor of the reference space (`local-floor`): under
/// the rider's eyes – the head's centre `head` moved
/// [`EYES_AHEAD_OF_HEAD_M`] along the bike's heading `yaw` – at the bike's
/// height `ground_y` (where the wheels touch the road). The tracked head
/// then puts the eyes where the rider's are (a rider's head is about 1.4 –
/// 1.6 m above the floor on a trainer, as on the road).
pub fn floor_under_eyes(head: [f32; 3], ground_y: f32, yaw: f32) -> [f32; 3] {
    let (s, c) = yaw.sin_cos();
    // −Z turned by `yaw` about +Y.
    let ahead = [-s, -c];
    [
        head[0] + ahead[0] * EYES_AHEAD_OF_HEAD_M,
        ground_y,
        head[2] + ahead[1] * EYES_AHEAD_OF_HEAD_M,
    ]
}

/// The XR frames' timing (spec 14I W3 / W4: the frame-time readout; here
/// the emulator's sanity numbers): the gaps between frames and the page's
/// time per frame, the last [`FrameStats::KEEP`] of each.
#[derive(Debug, Clone, Default)]
pub struct FrameStats {
    last_time_ms: Option<f64>,
    gaps_ms: Vec<f32>,
    work_ms: Vec<f32>,
    /// Frames seen.
    pub frames: u64,
    /// The longest gap (ms).
    pub longest_gap_ms: f32,
}

impl FrameStats {
    pub const KEEP: usize = 600;

    /// A frame at `time_ms` whose predecessor took `previous_ms` of the
    /// page's time.
    pub fn push(&mut self, time_ms: f64, previous_ms: f32) {
        self.frames += 1;
        if let Some(t) = self.last_time_ms {
            let gap = (time_ms - t) as f32;
            if gap >= 0.0 {
                keep(&mut self.gaps_ms, gap);
                self.longest_gap_ms = self.longest_gap_ms.max(gap);
            }
        }
        if previous_ms > 0.0 {
            keep(&mut self.work_ms, previous_ms);
        }
        self.last_time_ms = Some(time_ms);
    }

    /// A new session starts counting afresh.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// The gap between frames at quantile `q` (0 … 1), ms.
    pub fn gap_ms(&self, q: f32) -> Option<f32> {
        quantile(&self.gaps_ms, q)
    }

    /// The page's time per frame at quantile `q`, ms.
    pub fn work_ms(&self, q: f32) -> Option<f32> {
        quantile(&self.work_ms, q)
    }

    /// Frames per second from the median gap.
    pub fn fps(&self) -> Option<f32> {
        self.gap_ms(0.5).filter(|g| *g > 0.0).map(|g| 1000.0 / g)
    }

    /// Frames that came later than 1.5 × the median gap (dropped ones).
    pub fn late(&self) -> usize {
        let Some(m) = self.gap_ms(0.5) else { return 0 };
        self.gaps_ms.iter().filter(|g| **g > 1.5 * m).count()
    }
}

fn keep(v: &mut Vec<f32>, x: f32) {
    if v.len() == FrameStats::KEEP {
        v.remove(0);
    }
    v.push(x);
}

fn quantile(v: &[f32], q: f32) -> Option<f32> {
    if v.is_empty() {
        return None;
    }
    let mut s = v.to_vec();
    s.sort_by(f32::total_cmp);
    let i = ((s.len() - 1) as f32 * q.clamp(0.0, 1.0)).round() as usize;
    Some(s[i])
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Mat4, Quat, Vec3, Vec4};

    /// An OpenGL frustum matrix (`glFrustum`), column-major.
    fn gl_frustum(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> [f32; 16] {
        [
            2.0 * n / (r - l),
            0.0,
            0.0,
            0.0,
            0.0,
            2.0 * n / (t - b),
            0.0,
            0.0,
            (r + l) / (r - l),
            (t + b) / (t - b),
            -(f + n) / (f - n),
            -1.0,
            0.0,
            0.0,
            -2.0 * f * n / (f - n),
            0.0,
        ]
    }

    /// A view for [`frame_data`]: eye, viewport, projection, pose.
    type TestView = (f32, [f32; 4], [f32; 16], [f32; 7]);

    fn frame_data(views: &[TestView]) -> Vec<f32> {
        let mut d = vec![
            FRAME_LAYOUT,
            views.len() as f32,
            2880.0,
            1600.0,
            1000.0,
            9.5,
            0.0,
            1.6,
            0.0,
            0.0,
            0.0,
            0.0,
            1.0,
        ];
        for (eye, vp, proj, pose) in views {
            d.push(*eye);
            d.extend_from_slice(vp);
            d.extend_from_slice(proj);
            d.extend_from_slice(pose);
        }
        d
    }

    #[test]
    fn the_session_goes_offered_entering_running_blurred_ended() {
        let mut s = XrState::default();
        for (e, want) in [
            (XrEvent::Supported(true), XrState::Offered),
            (XrEvent::Enter, XrState::Entering),
            (XrEvent::Started, XrState::Running),
            (
                XrEvent::Visibility(Visibility::VisibleBlurred),
                XrState::Blurred,
            ),
            (XrEvent::Visibility(Visibility::Visible), XrState::Running),
            (XrEvent::Visibility(Visibility::Hidden), XrState::Hidden),
            (
                XrEvent::Visibility(Visibility::VisibleBlurred),
                XrState::Blurred,
            ),
            (XrEvent::Ended, XrState::Ended),
            // The button is offered again: a second session.
            (XrEvent::Enter, XrState::Entering),
            (XrEvent::Started, XrState::Running),
            (XrEvent::Ended, XrState::Ended),
        ] {
            s = s.next(&e).unwrap_or_else(|why| panic!("{why}"));
            assert_eq!(s, want, "after {e:?}");
        }
    }

    #[test]
    fn eyes_are_drawn_only_while_the_session_shows_them() {
        assert!(XrState::Running.in_headset());
        assert!(XrState::Blurred.in_headset());
        assert!(!XrState::Hidden.in_headset());
        assert!(XrState::Hidden.in_session());
        for s in [
            XrState::Unknown,
            XrState::Unsupported,
            XrState::Offered,
            XrState::Entering,
            XrState::Ended,
        ] {
            assert!(!s.in_headset() && !s.in_session(), "{s:?}");
        }
    }

    #[test]
    fn a_refused_session_offers_the_button_again_and_odd_events_are_ignored() {
        let s = XrState::Entering
            .next(&XrEvent::Failed("NotAllowedError".into()))
            .unwrap();
        assert_eq!(s, XrState::Offered);
        // Frames or visibility without a session, a second Enter: ignored.
        assert!(XrState::Offered.next(&XrEvent::Started).is_err());
        assert!(
            XrState::Offered
                .next(&XrEvent::Visibility(Visibility::Hidden))
                .is_err()
        );
        assert!(XrState::Running.next(&XrEvent::Enter).is_err());
        assert!(XrState::Unsupported.next(&XrEvent::Enter).is_err());
        // No headset any more: unsupported from anywhere.
        assert_eq!(
            XrState::Offered.next(&XrEvent::Supported(false)).unwrap(),
            XrState::Unsupported
        );
    }

    #[test]
    fn the_pages_event_words_parse() {
        assert_eq!(
            XrEvent::parse("supported", "true"),
            Some(XrEvent::Supported(true))
        );
        assert_eq!(
            XrEvent::parse("supported", "false"),
            Some(XrEvent::Supported(false))
        );
        assert_eq!(
            XrEvent::parse("visibility", "visible-blurred"),
            Some(XrEvent::Visibility(Visibility::VisibleBlurred))
        );
        assert_eq!(
            XrEvent::parse("failed", "NotAllowedError: no gesture"),
            Some(XrEvent::Failed("NotAllowedError: no gesture".into()))
        );
        assert_eq!(XrEvent::parse("visibility", "dim"), None);
        assert_eq!(XrEvent::parse("jump", ""), None);
    }

    #[test]
    fn a_frame_parses_with_its_views() {
        let p = gl_frustum(-1.0, 0.8, -1.1, 0.9, 0.1, 1000.0);
        let d = frame_data(&[
            (
                1.0,
                [0.0, 0.0, 1440.0, 1600.0],
                p,
                [-0.032, 1.6, 0.0, 0.0, 0.0, 0.0, 1.0],
            ),
            (
                2.0,
                [1440.0, 0.0, 1440.0, 1600.0],
                p,
                [0.032, 1.6, 0.0, 0.0, 0.0, 0.0, 1.0],
            ),
        ]);
        let f = parse_frame(&d).unwrap();
        assert_eq!(f.framebuffer, [2880.0, 1600.0]);
        assert_eq!(f.time_ms, 1000.0);
        assert_eq!(f.previous_frame_ms, 9.5);
        assert_eq!(f.viewer.position, [0.0, 1.6, 0.0]);
        assert_eq!(f.views.len(), 2);
        assert_eq!(f.views[0].eye, Eye::Left);
        assert_eq!(f.views[1].eye, Eye::Right);
        assert_eq!(f.views[1].viewport, [1440.0, 0.0, 1440.0, 1600.0]);
        assert_eq!(f.views[1].projection, p);
        assert_eq!(f.views[1].pose.position, [0.032, 1.6, 0.0]);
    }

    #[test]
    fn broken_frames_are_refused() {
        let p = gl_frustum(-1.0, 1.0, -1.0, 1.0, 0.1, 100.0);
        let good = frame_data(&[(
            1.0,
            [0.0, 0.0, 10.0, 10.0],
            p,
            [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0],
        )]);
        assert!(parse_frame(&good).is_ok());
        assert!(parse_frame(&good[..5]).is_err(), "short header");
        assert!(parse_frame(&good[..good.len() - 1]).is_err(), "short view");
        let mut layout = good.clone();
        layout[0] = 2.0;
        assert!(parse_frame(&layout).is_err(), "other layout");
        let mut count = good.clone();
        count[1] = 9.0;
        assert!(parse_frame(&count).is_err(), "too many views");
        let mut nan = good.clone();
        nan[20] = f32::NAN;
        assert!(parse_frame(&nan).is_err(), "NaN");
        // No views (no pose this frame) is a frame without eyes.
        let empty = frame_data(&[]);
        assert!(parse_frame(&empty).unwrap().views.is_empty());
    }

    #[test]
    fn viewports_become_canvas_parts_from_the_top_left() {
        let fb = [2880.0, 1600.0];
        let view = |vp| View {
            eye: Eye::Left,
            viewport: vp,
            projection: [0.0; 16],
            pose: Pose::IDENTITY,
        };
        assert_eq!(
            view([0.0, 0.0, 1440.0, 1600.0]).canvas_rect(fb),
            Some([0.0, 0.0, 0.5, 1.0])
        );
        assert_eq!(
            view([1440.0, 0.0, 1440.0, 1600.0]).canvas_rect(fb),
            Some([0.5, 0.0, 0.5, 1.0])
        );
        // OpenGL's bottom-left origin: the lower quarter is Bevy's bottom.
        assert_eq!(
            view([0.0, 0.0, 1440.0, 400.0]).canvas_rect(fb),
            Some([0.0, 0.75, 0.5, 0.25])
        );
        // The emulator's mono mode: the right eye has no width.
        assert_eq!(view([1440.0, 0.0, 0.0, 1600.0]).canvas_rect(fb), None);
        assert_eq!(view([0.0, 0.0, 10.0, 10.0]).canvas_rect([0.0, 0.0]), None);
    }

    #[test]
    fn canvas_parts_become_pixels_inside_the_target() {
        assert_eq!(
            rect_pixels([0.5, 0.0, 0.5, 1.0], [1281, 720]),
            ([641, 0], [640, 720])
        );
        assert_eq!(
            rect_pixels([0.0, 0.0, 0.5, 1.0], [1281, 720]),
            ([0, 0], [641, 720])
        );
        // Never empty, never outside.
        assert_eq!(
            rect_pixels([0.99, 0.99, 0.0, 0.0], [100, 100]),
            ([99, 99], [1, 1])
        );
        assert_eq!(rect_pixels([0.0, 0.0, 1.0, 1.0], [0, 0]), ([0, 0], [1, 1]));
    }

    #[test]
    fn the_frustum_is_read_from_webxrs_matrix() {
        let near = 0.1;
        let (l, r, b, t) = (-near, 0.8 * near, -1.1 * near, 0.9 * near);
        let tan = frustum_tangents(&gl_frustum(l, r, b, t, near, 1000.0)).unwrap();
        for (got, want) in tan.iter().zip([-1.0, 0.8, -1.1, 0.9]) {
            assert!((got - want).abs() < 1e-5, "{tan:?}");
        }
        // An orthographic or broken matrix is refused.
        let mut ortho = [0.0; 16];
        ortho[0] = 1.0;
        ortho[5] = 1.0;
        ortho[10] = -1.0;
        ortho[15] = 1.0;
        assert!(frustum_tangents(&ortho).is_err());
    }

    #[test]
    fn a_symmetric_eye_is_glams_infinite_reverse_z_projection() {
        let (fov, aspect, near) = (1.6f32, 0.9f32, EYE_NEAR_M);
        let t = (0.5 * fov).tan();
        let gl = gl_frustum(
            -t * aspect * near,
            t * aspect * near,
            -t * near,
            t * near,
            near,
            500.0,
        );
        let ours = Mat4::from_cols_array(&eye_clip_from_view(&gl).unwrap());
        // glam's right-handed `perspective_infinite_reverse` (Bevy's own).
        let f = 1.0 / t;
        let glam = Mat4::from_cols(
            Vec4::new(f / aspect, 0.0, 0.0, 0.0),
            Vec4::new(0.0, f, 0.0, 0.0),
            Vec4::new(0.0, 0.0, 0.0, -1.0),
            Vec4::new(0.0, 0.0, near, 0.0),
        );
        assert!(ours.abs_diff_eq(glam, 1e-5), "{ours:?} vs {glam:?}");
        let bevy = bevy::camera::PerspectiveProjection {
            fov,
            aspect_ratio: aspect,
            near,
            ..Default::default()
        };
        let bevy = bevy::camera::CameraProjection::get_clip_from_view(&bevy);
        assert!(ours.abs_diff_eq(bevy, 1e-5), "{ours:?} vs Bevy's {bevy:?}");
    }

    #[test]
    fn an_asymmetric_eye_maps_its_frustum_edges_to_the_clip_edges() {
        let tan = [-1.2f32, 0.9, -1.3, 0.8];
        let m = Mat4::from_cols_array(&clip_from_view(tan, EYE_NEAR_M));
        let ndc = |p: Vec3| {
            let c = m * Vec4::new(p.x, p.y, p.z, 1.0);
            c.truncate() / c.w
        };
        for z in [0.3f32, 7.0, 900.0] {
            let at = |tx: f32, ty: f32| ndc(Vec3::new(tx * z, ty * z, -z));
            assert!((at(tan[0], 0.0).x + 1.0).abs() < 1e-5);
            assert!((at(tan[1], 0.0).x - 1.0).abs() < 1e-5);
            assert!((at(0.0, tan[2]).y + 1.0).abs() < 1e-5);
            assert!((at(0.0, tan[3]).y - 1.0).abs() < 1e-5);
        }
        // Reverse Z: 1 at the near plane, towards 0 far away.
        assert!((ndc(Vec3::new(0.0, 0.0, -EYE_NEAR_M)).z - 1.0).abs() < 1e-5);
        assert!(ndc(Vec3::new(0.0, 0.0, -1.0e6)).z < 1e-6);
    }

    #[test]
    fn the_corners_lie_on_the_frustum_edges() {
        let tan = [-1.2f32, 0.9, -1.3, 0.8];
        let c = frustum_corners(tan, 0.5, 100.0);
        assert_eq!(c[0], [0.9 * 0.5, -1.3 * 0.5, -0.5], "near bottom right");
        assert_eq!(c[2], [-1.2 * 0.5, 0.8 * 0.5, -0.5], "near top left");
        assert_eq!(c[5], [0.9 * 100.0, 0.8 * 100.0, -100.0], "far top right");
        assert_eq!(
            c[7],
            [-1.2 * 100.0, -1.3 * 100.0, -100.0],
            "far bottom left"
        );
    }

    #[test]
    fn the_rig_turns_the_space_with_the_bike_but_never_pitches_it() {
        let to_vec = |p: [f32; 3]| Vec3::from_array(p);
        let to_quat = |q: [f32; 4]| Quat::from_array(q);
        let origin = [100.0, 20.0, -50.0];
        // An eye 1.5 m up, 0.03 m to the right, looking 10° down.
        let eye = Pose {
            position: [0.03, 1.5, 0.0],
            orientation: Quat::from_rotation_x(-10f32.to_radians()).to_array(),
        };
        for yaw in [0.0f32, 0.7, -2.5, 3.1] {
            let got = rig_eye(origin, yaw, eye);
            let turn = Quat::from_rotation_y(yaw);
            let want_pos = to_vec(origin) + turn * to_vec(eye.position);
            assert!(to_vec(got.position).abs_diff_eq(want_pos, 1e-4), "{yaw}");
            let want_rot = turn * to_quat(eye.orientation);
            assert!(
                to_quat(got.orientation).abs_diff_eq(want_rot, 1e-5),
                "{yaw}"
            );
            // The head's height above the origin stays the head's own.
            assert!((got.position[1] - 21.5).abs() < 1e-5);
        }
    }

    #[test]
    fn the_floor_is_under_the_eyes_at_the_road() {
        for yaw in [0.0f32, 1.0, -2.2] {
            let got = floor_under_eyes([10.0, 21.5, -4.0], 20.0, yaw);
            let ahead = Quat::from_rotation_y(yaw) * Vec3::NEG_Z * EYES_AHEAD_OF_HEAD_M;
            let want = Vec3::new(10.0, 20.0, -4.0) + ahead;
            assert!(
                Vec3::from_array(got).abs_diff_eq(want, 1e-5),
                "{yaw}: {got:?}"
            );
        }
    }

    #[test]
    fn the_heading_of_an_orientation_ignores_pitch_and_roll() {
        for yaw in [0.0f32, 0.4, -1.9, 3.0] {
            let pitched = Quat::from_euler(bevy::math::EulerRot::YXZ, yaw, 0.15, -0.1);
            let got = yaw_of(pitched.to_array());
            assert!((got - yaw).abs() < 1e-4, "{yaw} → {got}");
            let back = Quat::from_array(yaw_quat(got));
            assert!(back.abs_diff_eq(Quat::from_rotation_y(yaw), 1e-4));
        }
    }

    #[test]
    fn frame_stats_measure_gaps_work_and_late_frames() {
        let mut s = FrameStats::default();
        assert_eq!(s.fps(), None);
        let mut t = 0.0f64;
        for i in 0..100 {
            // 72 Hz with one dropped frame every 25.
            t += if i % 25 == 24 { 27.8 } else { 13.9 };
            s.push(t, 8.0 + (i % 5) as f32);
        }
        assert_eq!(s.frames, 100);
        let fps = s.fps().unwrap();
        assert!((fps - 71.94).abs() < 0.1, "{fps}");
        assert_eq!(s.late(), 4);
        assert!((s.longest_gap_ms - 27.8).abs() < 1e-3);
        assert_eq!(s.work_ms(0.5), Some(10.0));
        assert_eq!(s.work_ms(1.0), Some(12.0));
        s.reset();
        assert_eq!(s.frames, 0);
        assert_eq!(s.gap_ms(0.5), None);
    }

    #[test]
    fn frame_stats_keep_a_window() {
        let mut s = FrameStats::default();
        for i in 0..(FrameStats::KEEP + 50) {
            s.push(i as f64 * 10.0, 1.0);
        }
        assert_eq!(s.gaps_ms.len(), FrameStats::KEEP);
        assert_eq!(s.work_ms.len(), FrameStats::KEEP);
    }
}

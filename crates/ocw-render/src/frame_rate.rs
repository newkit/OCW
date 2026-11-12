//! The real render rate (spec §3.2, §11.4): frames per second measured on
//! the wall clock, shown by the FPS counter (console `fps`, setting
//! `display.fps`) at the top right and, with [`LogRenderRate`], written to
//! the log every few seconds.
//!
//! Bevy's own frame-time diagnostic follows `Time<Real>`, which frame
//! capture (`--capture`) steps by a fixed amount per frame: it would show
//! the capture rate (25), not how fast the app really draws. This meter
//! reads [`Instant::now`] once per frame instead.
//!
//! [`FrameLimit`] (`--max-fps`) caps the render rate by sleeping at the end
//! of a frame: with `--no-vsync` the app then draws at that rate even where
//! the display gives no refresh to wait for (a locked kiosk screen: the X
//! server paces a window on a screen that is off at one frame a second).

use std::collections::VecDeque;

use bevy::platform::time::Instant;
use bevy::prelude::*;

use crate::hud::ShowFps;

pub struct FrameRatePlugin;

impl Plugin for FrameRatePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderRate>()
            .init_resource::<LogRenderRate>()
            .init_resource::<ShowFps>()
            .add_systems(First, measure)
            .init_resource::<FrameLimit>()
            .add_systems(Update, update_badge)
            .add_systems(Last, limit_frame_rate);
    }
}

/// Wall-clock frame times of the last [`FrameRateMeter::WINDOW_S`].
#[derive(Debug, Clone, Default)]
pub struct FrameRateMeter {
    frames: VecDeque<f64>,
    sum: f64,
}

impl FrameRateMeter {
    /// The window the rate is averaged over, in seconds.
    pub const WINDOW_S: f64 = 1.0;

    /// One frame that took `dt_s` seconds.
    pub fn record(&mut self, dt_s: f64) {
        let dt = dt_s.max(0.0);
        self.frames.push_back(dt);
        self.sum += dt;
        // Keep at least the newest frame (a frame longer than the window).
        while self.frames.len() > 1 && self.sum - self.frames[0] >= Self::WINDOW_S {
            self.sum -= self.frames.pop_front().unwrap_or(0.0);
        }
    }

    /// Frames per second over the window (none before the first frame).
    pub fn fps(&self) -> Option<f64> {
        (self.sum > 0.0).then(|| self.frames.len() as f64 / self.sum)
    }

    /// The longest frame of the window, in milliseconds.
    pub fn worst_ms(&self) -> Option<f64> {
        self.frames
            .iter()
            .copied()
            .reduce(f64::max)
            .map(|s| s * 1000.0)
    }
}

/// Totals for the periodic log line: frames and time since the last line.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RateTotals {
    pub frames: u32,
    pub seconds: f64,
    pub worst_s: f64,
}

impl RateTotals {
    pub fn record(&mut self, dt_s: f64) {
        self.frames += 1;
        self.seconds += dt_s;
        self.worst_s = self.worst_s.max(dt_s);
    }

    pub fn fps(&self) -> f64 {
        if self.seconds > 0.0 {
            f64::from(self.frames) / self.seconds
        } else {
            0.0
        }
    }
}

/// The measured render rate of this app.
#[derive(Resource, Debug, Default)]
pub struct RenderRate {
    last: Option<Instant>,
    pub meter: FrameRateMeter,
    totals: RateTotals,
    /// Wall-clock seconds since the badge text was last renewed.
    since_label: f64,
}

/// Log the render rate every this many seconds (`--log-fps`; none: never).
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq)]
pub struct LogRenderRate(pub Option<f64>);

/// At most this many frames per second (`--max-fps`; none: no cap).
#[derive(Resource, Debug, Default, Clone)]
pub struct FrameLimit {
    pub max_fps: Option<f64>,
    /// When the next frame may end.
    next: Option<Instant>,
}

impl FrameLimit {
    pub fn new(max_fps: Option<f64>) -> Self {
        Self {
            max_fps: max_fps.filter(|f| f.is_finite() && *f > 0.0),
            next: None,
        }
    }
}

/// The deadline after a frame that was due at `due` and ended at `now`:
/// one `period` later, but a frame that ended more than a period late
/// starts a new schedule (no burst of short frames to catch up).
pub fn next_deadline(due: Instant, now: Instant, period: std::time::Duration) -> Instant {
    if now > due + period {
        now + period
    } else {
        due + period
    }
}

fn limit_frame_rate(mut limit: ResMut<FrameLimit>) {
    let Some(fps) = limit.max_fps else {
        return;
    };
    let period = std::time::Duration::from_secs_f64(1.0 / fps);
    let now = Instant::now();
    let due = limit.next.unwrap_or(now);
    if due > now {
        std::thread::sleep(due - now);
    }
    limit.next = Some(next_deadline(due, Instant::now(), period));
}

/// The FPS counter's text, e.g. `58 fps  max 21 ms`.
pub fn badge_label(meter: &FrameRateMeter) -> String {
    match (meter.fps(), meter.worst_ms()) {
        (Some(fps), Some(worst)) => format!("{fps:.0} fps  max {worst:.0} ms"),
        _ => "-- fps".into(),
    }
}

fn measure(mut rate: ResMut<RenderRate>, log: Res<LogRenderRate>) {
    let now = Instant::now();
    let Some(last) = rate.last.replace(now) else {
        return;
    };
    let dt = now.duration_since(last).as_secs_f64();
    rate.meter.record(dt);
    rate.since_label += dt;
    rate.totals.record(dt);
    match log.0 {
        Some(period) if rate.totals.seconds >= period => {
            let t = std::mem::take(&mut rate.totals);
            info!(
                fps = format!("{:.1}", t.fps()),
                worst_ms = format!("{:.0}", t.worst_s * 1000.0),
                frames = t.frames,
                seconds = format!("{:.1}", t.seconds),
                "render rate"
            );
        }
        Some(_) => {}
        // Not logged: nothing to keep.
        None => rate.totals = RateTotals::default(),
    }
}

/// Marks the FPS counter (top right, below the REC indicator).
#[derive(Component)]
pub(crate) struct FpsBadge;

/// The FPS counter's node, a child of the top-right column.
pub(crate) fn badge_bundle() -> impl Bundle {
    (
        Node {
            display: Display::None,
            padding: UiRect::axes(px(8), px(4)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.75)),
        Text::new(""),
        TextFont {
            font_size: 18.0.into(),
            ..default()
        },
        TextColor(Color::srgb(0.55, 1.0, 0.45)),
        FpsBadge,
    )
}

fn update_badge(
    show: Res<ShowFps>,
    mut rate: ResMut<RenderRate>,
    mut badge: Query<(&mut Node, &mut Text), With<FpsBadge>>,
) {
    let Ok((mut node, mut text)) = badge.single_mut() else {
        return;
    };
    let display = if show.0 { Display::Flex } else { Display::None };
    if node.display != display {
        node.display = display;
    }
    // Renewed twice a second: readable, and no text layout every frame.
    if show.0 && (rate.since_label >= 0.5 || text.0.is_empty()) {
        rate.since_label = 0.0;
        let label = badge_label(&rate.meter);
        if text.0 != label {
            text.0 = label;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_over_the_last_second() {
        let mut m = FrameRateMeter::default();
        assert_eq!(m.fps(), None);
        assert_eq!(badge_label(&m), "-- fps");
        for _ in 0..120 {
            m.record(1.0 / 60.0);
        }
        assert!((m.fps().unwrap() - 60.0).abs() < 1e-6);
        // Slow frames push the fast ones out of the window.
        for _ in 0..10 {
            m.record(0.2);
        }
        assert!((m.fps().unwrap() - 5.0).abs() < 1e-6, "{:?}", m.fps());
        assert!((m.worst_ms().unwrap() - 200.0).abs() < 1e-6);
        assert_eq!(badge_label(&m), "5 fps  max 200 ms");
    }

    #[test]
    fn a_frame_longer_than_the_window_still_counts() {
        let mut m = FrameRateMeter::default();
        m.record(0.01);
        m.record(2.0);
        assert!((m.fps().unwrap() - 0.5).abs() < 1e-9);
        assert!((m.worst_ms().unwrap() - 2000.0).abs() < 1e-6);
    }

    #[test]
    fn frame_limit_deadlines() {
        let t0 = Instant::now();
        let p = std::time::Duration::from_millis(40);
        // On time or early: the next frame is due one period later.
        assert_eq!(next_deadline(t0, t0, p), t0 + p);
        assert_eq!(next_deadline(t0, t0 + p / 2, p), t0 + p);
        // Very late (a slow frame): a new schedule from now.
        let late = t0 + p * 3;
        assert_eq!(next_deadline(t0, late, p), late + p);
        assert_eq!(FrameLimit::new(Some(0.0)).max_fps, None);
        assert_eq!(FrameLimit::new(Some(f64::NAN)).max_fps, None);
        assert_eq!(FrameLimit::new(Some(60.0)).max_fps, Some(60.0));
    }

    /// `--max-fps` holds the app at the cap (here 50 fps: 10 frames take
    /// at least 9 periods).
    #[test]
    fn frame_limit_caps_the_rate() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(FrameRatePlugin)
            .insert_resource(FrameLimit::new(Some(50.0)));
        app.update();
        let t0 = std::time::Instant::now();
        for _ in 0..10 {
            app.update();
        }
        assert!(
            t0.elapsed() >= std::time::Duration::from_millis(180),
            "{:?}",
            t0.elapsed()
        );
    }

    #[test]
    fn totals_for_the_log() {
        let mut t = RateTotals::default();
        assert_eq!(t.fps(), 0.0);
        for dt in [0.02, 0.02, 0.06] {
            t.record(dt);
        }
        assert_eq!(t.frames, 3);
        assert!((t.fps() - 30.0).abs() < 1e-9);
        assert!((t.worst_s - 0.06).abs() < 1e-12);
    }

    /// The badge follows `fps on|off` (it shows the wall-clock rate, so its
    /// text is only checked for being set).
    #[test]
    fn badge_shows_and_hides_with_show_fps() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(FrameRatePlugin)
            .add_systems(Startup, |mut c: Commands| {
                c.spawn(badge_bundle());
            });
        app.update();
        let shown = |app: &mut App| {
            let w = app.world_mut();
            let mut q = w.query_filtered::<(&Node, &Text), With<FpsBadge>>();
            let (n, t) = q.single(w).unwrap();
            (n.display, t.0.clone())
        };
        assert_eq!(shown(&mut app).0, Display::None);
        app.world_mut().resource_mut::<ShowFps>().0 = true;
        app.update();
        app.update();
        let (display, text) = shown(&mut app);
        assert_eq!(display, Display::Flex);
        assert!(text.ends_with("fps") || text.contains("fps  max"), "{text}");
        app.world_mut().resource_mut::<ShowFps>().0 = false;
        app.update();
        assert_eq!(shown(&mut app).0, Display::None);
    }
}

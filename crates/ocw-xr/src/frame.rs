//! The frame loop: the order of the OpenXR frame calls, and what they
//! cost (frame time, latency, missed frames: the X1 measurements).
//!
//! Per frame (OpenXR's rules):
//!
//! 1. `xrWaitFrame` – blocks until it is time to start the next frame;
//!    gives the predicted display time and whether to render
//!    ([`FrameSource::wait_frame`], main thread, start of the frame);
//! 2. `xrLocateViews` for the time the frame will be on the display
//!    ([`pose_time`]: one frame later with Bevy's pipelined renderer);
//! 3. the world is simulated and drawn into the eye textures;
//! 4. on the render thread ([`submit`]): `xrBeginFrame`, and if rendering:
//!    acquire / wait / copy the eye textures in / release the swapchain
//!    image; `xrEndFrame` with the projection layer (or none).
//!
//! The calls are behind [`FrameSource`] / [`FrameSink`], so the order is
//! tested with the mock runtime.

use crate::session::XrError;
use crate::view::StereoViews;

/// From `xrWaitFrame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FrameTiming {
    /// When this frame will be shown (runtime clock, ns).
    pub display_time_ns: i64,
    /// The display's frame period (ns).
    pub period_ns: i64,
    pub should_render: bool,
}

impl FrameTiming {
    /// The display's refresh rate (Hz).
    pub fn refresh_hz(&self) -> f32 {
        if self.period_ns > 0 {
            1e9 / self.period_ns as f32
        } else {
            0.0
        }
    }
}

/// The time to locate the views for: the display time, one period later
/// when the renderer draws a frame behind the simulation (Bevy's
/// pipelined rendering).
pub fn pose_time(t: &FrameTiming, pipelined: bool) -> i64 {
    if pipelined {
        t.display_time_ns + t.period_ns
    } else {
        t.display_time_ns
    }
}

/// The main thread's calls.
pub trait FrameSource {
    fn wait_frame(&mut self) -> Result<FrameTiming, XrError>;
    /// The eyes at `time_ns` (tracking space); `None` while tracking is
    /// lost (keep the last views).
    fn locate_views(&mut self, time_ns: i64) -> Result<Option<StereoViews>, XrError>;
    /// The runtime's clock now (ns), for the latency.
    fn now_ns(&mut self) -> Option<i64>;
}

/// The render thread's calls.
pub trait FrameSink {
    fn begin_frame(&mut self) -> Result<(), XrError>;
    /// The swapchain image to draw into.
    fn acquire_image(&mut self) -> Result<u32, XrError>;
    fn wait_image(&mut self) -> Result<(), XrError>;
    fn release_image(&mut self) -> Result<(), XrError>;
    /// `layer`: the views the images were drawn with; `None`: nothing to
    /// show this frame.
    fn end_frame(
        &mut self,
        display_time_ns: i64,
        layer: Option<&StereoViews>,
    ) -> Result<(), XrError>;
}

/// What [`submit`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submitted {
    /// The eye images went to the headset.
    Shown,
    /// An empty frame (the runtime said not to render, or no views yet).
    Empty,
}

/// What became of the submitted frames (`vr status`): an empty or failed
/// frame shows nothing new in the headset – the runtime repeats or
/// reprojects the last one, seen as a flicker.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SubmitCounts {
    pub shown: u64,
    pub empty: u64,
    pub failed: u64,
}

impl SubmitCounts {
    pub fn count(&mut self, r: &Result<Submitted, XrError>) {
        match r {
            Ok(Submitted::Shown) => self.shown += 1,
            Ok(Submitted::Empty) => self.empty += 1,
            Err(_) => self.failed += 1,
        }
    }

    pub fn line(&self) -> String {
        format!(
            "submitted: {} shown, {} empty, {} failed",
            self.shown, self.empty, self.failed
        )
    }
}

/// One frame's submission. `copy(image)` puts the eye textures into
/// swapchain image `image` (records and submits the GPU copy). A failed
/// copy still releases the image and ends the frame (empty), so the
/// runtime's frame loop never stalls.
pub fn submit(
    sink: &mut dyn FrameSink,
    timing: &FrameTiming,
    views: Option<&StereoViews>,
    copy: &mut dyn FnMut(u32) -> Result<(), XrError>,
) -> Result<Submitted, XrError> {
    sink.begin_frame()?;
    let Some(views) = views.filter(|_| timing.should_render) else {
        sink.end_frame(timing.display_time_ns, None)?;
        return Ok(Submitted::Empty);
    };
    let image = sink.acquire_image()?;
    sink.wait_image()?;
    let copied = copy(image);
    sink.release_image()?;
    match copied {
        Ok(()) => {
            sink.end_frame(timing.display_time_ns, Some(views))?;
            Ok(Submitted::Shown)
        }
        Err(e) => {
            sink.end_frame(timing.display_time_ns, None)?;
            Err(e)
        }
    }
}

/// Frame-time and latency statistics over the last frames.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameStats {
    cpu_ms: Vec<f32>,
    latency_ms: Vec<f32>,
    next: usize,
    /// Frames the display showed twice (our frame came late).
    pub missed: u64,
    pub frames: u64,
    last_display_ns: Option<i64>,
    /// The display's period (ms), the last one seen.
    pub period_ms: f32,
}

/// Frames kept for the statistics (about 2 s at 72 Hz).
pub const STATS_FRAMES: usize = 144;

impl Default for FrameStats {
    fn default() -> Self {
        Self {
            cpu_ms: Vec::with_capacity(STATS_FRAMES),
            latency_ms: Vec::with_capacity(STATS_FRAMES),
            next: 0,
            missed: 0,
            frames: 0,
            last_display_ns: None,
            period_ms: 0.0,
        }
    }
}

/// The statistics in numbers.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatsSummary {
    pub frames: u64,
    pub mean_ms: f32,
    pub p95_ms: f32,
    pub max_ms: f32,
    pub latency_ms: f32,
    pub missed: u64,
    pub period_ms: f32,
}

impl FrameStats {
    /// One frame: its time on the CPU (from one wait to the next, ms), the
    /// timing it waited for, and the motion-to-photon estimate (the views'
    /// sample time to the display time, ms; `None` if unknown).
    pub fn record(&mut self, cpu_ms: f32, timing: &FrameTiming, latency_ms: Option<f32>) {
        self.frames += 1;
        if timing.period_ns > 0 {
            self.period_ms = timing.period_ns as f32 / 1e6;
            if let Some(last) = self.last_display_ns {
                let gap = timing.display_time_ns - last;
                // More than one and a half periods between two displays:
                // the frames between were missed.
                if gap * 2 > timing.period_ns * 3 {
                    self.missed +=
                        ((gap as f64 / timing.period_ns as f64).round() as u64).max(2) - 1;
                }
            }
            self.last_display_ns = Some(timing.display_time_ns);
        }
        let push = |v: &mut Vec<f32>, x: f32, at: usize| {
            if v.len() < STATS_FRAMES {
                v.push(x);
            } else {
                v[at] = x;
            }
        };
        let at = self.next;
        push(&mut self.cpu_ms, cpu_ms, at);
        if let Some(l) = latency_ms {
            push(&mut self.latency_ms, l, at % STATS_FRAMES);
        }
        self.next = (self.next + 1) % STATS_FRAMES;
    }

    pub fn summary(&self) -> StatsSummary {
        let mut v = self.cpu_ms.clone();
        v.sort_by(f32::total_cmp);
        let mean = |v: &[f32]| {
            if v.is_empty() {
                0.0
            } else {
                v.iter().sum::<f32>() / v.len() as f32
            }
        };
        let p95 = if v.is_empty() {
            0.0
        } else {
            v[((v.len() as f32 * 0.95).ceil() as usize).clamp(1, v.len()) - 1]
        };
        StatsSummary {
            frames: self.frames,
            mean_ms: mean(&v),
            p95_ms: p95,
            max_ms: v.last().copied().unwrap_or(0.0),
            latency_ms: mean(&self.latency_ms),
            missed: self.missed,
            period_ms: self.period_ms,
        }
    }
}

impl StatsSummary {
    /// "frame 9.8 ms (p95 11.2, max 14.0) of 13.9 ms, latency 24.1 ms,
    /// missed 3 of 2000".
    pub fn line(&self) -> String {
        format!(
            "frame {:.1} ms (p95 {:.1}, max {:.1}) of {:.1} ms, latency {:.1} ms, missed {} of {}",
            self.mean_ms,
            self.p95_ms,
            self.max_ms,
            self.period_ms,
            self.latency_ms,
            self.missed,
            self.frames
        )
    }

    /// Within budget: the p95 frame fits the display's period.
    pub fn in_budget(&self) -> bool {
        self.period_ms > 0.0 && self.p95_ms <= self.period_ms
    }
}

#[cfg(test)]
mod counts_tests {
    use super::*;

    #[test]
    fn submissions_are_counted() {
        let mut c = SubmitCounts::default();
        c.count(&Ok(Submitted::Shown));
        c.count(&Ok(Submitted::Shown));
        c.count(&Ok(Submitted::Empty));
        c.count(&Err(XrError::Other("the eye images are not ready".into())));
        assert_eq!(
            c,
            SubmitCounts {
                shown: 2,
                empty: 1,
                failed: 1
            }
        );
        assert_eq!(c.line(), "submitted: 2 shown, 1 empty, 1 failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Pose;
    use crate::mock::{Call, MockRuntime};
    use crate::view::{QUEST1_LEFT_FOV, sim_views};

    fn views() -> StereoViews {
        sim_views(&Pose::IDENTITY, 0.063, &QUEST1_LEFT_FOV)
    }

    #[test]
    fn a_rendered_frame_calls_in_openxr_order() {
        let mut rt = MockRuntime::default();
        let t = FrameTiming {
            display_time_ns: 1_000,
            period_ns: 13_888_889,
            should_render: true,
        };
        let mut copied = Vec::new();
        let r = submit(&mut rt, &t, Some(&views()), &mut |i| {
            copied.push(i);
            Ok(())
        });
        assert_eq!(r, Ok(Submitted::Shown));
        assert_eq!(
            rt.calls,
            [
                Call::BeginFrame,
                Call::AcquireImage,
                Call::WaitImage,
                Call::ReleaseImage,
                Call::EndFrame { layer: true }
            ]
        );
        assert_eq!(copied, [0]);
        assert_eq!(rt.ended_at, Some(1_000));
        // The images rotate through the swapchain.
        let _ = submit(&mut rt, &t, Some(&views()), &mut |i| {
            copied.push(i);
            Ok(())
        });
        assert_eq!(copied, [0, 1]);
    }

    #[test]
    fn frames_not_to_render_end_empty() {
        let mut rt = MockRuntime::default();
        let t = FrameTiming {
            should_render: false,
            ..Default::default()
        };
        let r = submit(&mut rt, &t, Some(&views()), &mut |_| panic!("no copy"));
        assert_eq!(r, Ok(Submitted::Empty));
        assert_eq!(
            rt.calls,
            [Call::BeginFrame, Call::EndFrame { layer: false }]
        );
        // No views yet (tracking not found): empty too.
        let mut rt = MockRuntime::default();
        let t = FrameTiming {
            should_render: true,
            ..t
        };
        assert_eq!(
            submit(&mut rt, &t, None, &mut |_| Ok(())),
            Ok(Submitted::Empty)
        );
    }

    #[test]
    fn a_failed_copy_still_releases_and_ends() {
        let mut rt = MockRuntime::default();
        let t = FrameTiming {
            should_render: true,
            ..Default::default()
        };
        let r = submit(&mut rt, &t, Some(&views()), &mut |_| {
            Err(XrError::Other("copy".into()))
        });
        assert!(r.is_err());
        assert_eq!(
            rt.calls,
            [
                Call::BeginFrame,
                Call::AcquireImage,
                Call::WaitImage,
                Call::ReleaseImage,
                Call::EndFrame { layer: false }
            ]
        );
        // A lost session on begin stops the frame there.
        let mut rt = MockRuntime {
            frame_error: Some(XrError::SessionLost),
            ..Default::default()
        };
        assert_eq!(
            submit(&mut rt, &t, Some(&views()), &mut |_| Ok(())),
            Err(XrError::SessionLost)
        );
        assert!(rt.calls.is_empty());
    }

    #[test]
    fn poses_are_located_for_when_the_frame_is_shown() {
        let t = FrameTiming {
            display_time_ns: 5_000,
            period_ns: 1_000,
            should_render: true,
        };
        assert_eq!(pose_time(&t, false), 5_000);
        assert_eq!(pose_time(&t, true), 6_000);
        assert!(
            (FrameTiming {
                period_ns: 11_111_111,
                ..t
            }
            .refresh_hz()
                - 90.0)
                .abs()
                < 0.01
        );
        assert_eq!(FrameTiming::default().refresh_hz(), 0.0);
    }

    #[test]
    fn stats_count_frame_times_latency_and_missed_frames() {
        let mut s = FrameStats::default();
        let period = 13_888_889i64;
        let mut t = FrameTiming {
            display_time_ns: 0,
            period_ns: period,
            should_render: true,
        };
        for i in 0..200 {
            // Every 50th frame shows a frame late (two periods).
            t.display_time_ns += if i % 50 == 49 { 2 * period } else { period };
            let cpu = if i % 10 == 0 { 15.0 } else { 9.0 };
            s.record(cpu, &t, Some(24.0));
        }
        let sum = s.summary();
        assert_eq!(sum.frames, 200);
        assert_eq!(sum.missed, 4);
        assert!((sum.period_ms - 13.889).abs() < 1e-3);
        // The last 144 frames: one in ten at 15 ms.
        assert!((sum.mean_ms - 9.6).abs() < 0.1, "{}", sum.mean_ms);
        assert_eq!(sum.p95_ms, 15.0);
        assert_eq!(sum.max_ms, 15.0);
        assert_eq!(sum.latency_ms, 24.0);
        assert!(!sum.in_budget(), "p95 15 ms > 13.9 ms");
        assert!(sum.line().contains("missed 4 of 200"), "{}", sum.line());
        let empty = FrameStats::default().summary();
        assert_eq!((empty.mean_ms, empty.p95_ms, empty.frames), (0.0, 0.0, 0));
        assert!(!empty.in_budget());
    }
}

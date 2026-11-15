//! An in-memory runtime (no headset, no OpenXR): scripted events, a log of
//! the calls made, and a head that looks around ([`crate::view::sim_head`]).
//! For the tests here and in the renderer, and for `--vr-sim`'s frame loop.

use std::collections::VecDeque;

use crate::frame::{FrameSink, FrameSource, FrameTiming};
use crate::projection::Fov;
use crate::session::{RuntimeEvent, XrError, XrRuntime};
use crate::view::{DEFAULT_IPD_M, QUEST1_LEFT_FOV, StereoViews, sim_head, sim_views};

/// A call the mock received.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Call {
    BeginSession,
    EndSession,
    RequestExit,
    RecreateSession,
    WaitFrame,
    LocateViews,
    BeginFrame,
    AcquireImage,
    WaitImage,
    ReleaseImage,
    EndFrame { layer: bool },
}

#[derive(Debug, Clone)]
pub struct MockRuntime {
    /// Events handed out by `poll_event`, in order.
    pub events: VecDeque<RuntimeEvent>,
    /// Every call, in order (polls and clock reads are not logged).
    pub calls: Vec<Call>,
    /// `poll_event` fails with this while set.
    pub poll_error: Option<XrError>,
    /// `begin_session` fails with this once.
    pub begin_error: Option<XrError>,
    /// `recreate_session`'s results in turn (then `Ok`).
    pub recreate_results: VecDeque<Result<(), XrError>>,
    /// The next frame call (wait or begin) fails with this once.
    pub frame_error: Option<XrError>,
    /// The swapchain's images.
    pub images: u32,
    /// The next image `acquire_image` hands out.
    pub next_image: u32,
    /// The display period (ns): 72 Hz like the Quest 1.
    pub period_ns: i64,
    /// The runtime clock (ns): advanced a period per wait.
    pub clock_ns: i64,
    pub should_render: bool,
    pub ipd_m: f32,
    pub fov: Fov,
    /// The display time `end_frame` was last called with.
    pub ended_at: Option<i64>,
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self {
            events: VecDeque::new(),
            calls: Vec::new(),
            poll_error: None,
            begin_error: None,
            recreate_results: VecDeque::new(),
            frame_error: None,
            images: 3,
            next_image: 0,
            period_ns: 13_888_889,
            clock_ns: 1_000_000_000,
            should_render: true,
            ipd_m: DEFAULT_IPD_M,
            fov: QUEST1_LEFT_FOV,
            ended_at: None,
        }
    }
}

impl MockRuntime {
    fn frame_call(&mut self, c: Call) -> Result<(), XrError> {
        if let Some(e) = self.frame_error.take() {
            return Err(e);
        }
        self.calls.push(c);
        Ok(())
    }
}

impl XrRuntime for MockRuntime {
    fn poll_event(&mut self) -> Result<Option<RuntimeEvent>, XrError> {
        if let Some(e) = &self.poll_error {
            return Err(e.clone());
        }
        Ok(self.events.pop_front())
    }

    fn begin_session(&mut self) -> Result<(), XrError> {
        if let Some(e) = self.begin_error.take() {
            return Err(e);
        }
        self.calls.push(Call::BeginSession);
        Ok(())
    }

    fn end_session(&mut self) -> Result<(), XrError> {
        self.calls.push(Call::EndSession);
        Ok(())
    }

    fn request_exit(&mut self) -> Result<(), XrError> {
        self.calls.push(Call::RequestExit);
        Ok(())
    }

    fn recreate_session(&mut self) -> Result<(), XrError> {
        self.calls.push(Call::RecreateSession);
        self.recreate_results.pop_front().unwrap_or(Ok(()))
    }
}

impl FrameSource for MockRuntime {
    fn wait_frame(&mut self) -> Result<FrameTiming, XrError> {
        self.frame_call(Call::WaitFrame)?;
        self.clock_ns += self.period_ns;
        Ok(FrameTiming {
            // Shown two periods from now (the compositor's pipeline).
            display_time_ns: self.clock_ns + 2 * self.period_ns,
            period_ns: self.period_ns,
            should_render: self.should_render,
        })
    }

    fn locate_views(&mut self, time_ns: i64) -> Result<Option<StereoViews>, XrError> {
        self.calls.push(Call::LocateViews);
        let t = (time_ns as f64 / 1e9) as f32;
        Ok(Some(sim_views(&sim_head(t), self.ipd_m, &self.fov)))
    }

    fn now_ns(&mut self) -> Option<i64> {
        Some(self.clock_ns)
    }
}

impl FrameSink for MockRuntime {
    fn begin_frame(&mut self) -> Result<(), XrError> {
        self.frame_call(Call::BeginFrame)
    }

    fn acquire_image(&mut self) -> Result<u32, XrError> {
        self.calls.push(Call::AcquireImage);
        let i = self.next_image;
        self.next_image = (self.next_image + 1) % self.images.max(1);
        Ok(i)
    }

    fn wait_image(&mut self) -> Result<(), XrError> {
        self.calls.push(Call::WaitImage);
        Ok(())
    }

    fn release_image(&mut self) -> Result<(), XrError> {
        self.calls.push(Call::ReleaseImage);
        Ok(())
    }

    fn end_frame(
        &mut self,
        display_time_ns: i64,
        layer: Option<&StereoViews>,
    ) -> Result<(), XrError> {
        self.calls.push(Call::EndFrame {
            layer: layer.is_some(),
        });
        self.ended_at = Some(display_time_ns);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::{FrameStats, pose_time, submit};
    use crate::session::{Phase, RuntimeState, SessionMachine};
    use crate::view::ipd;

    /// A whole session against the mock: start, 100 frames, exit.
    #[test]
    fn a_session_runs_frames_and_exits() {
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::new();
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Ready));
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Focused));
        let mut stats = FrameStats::default();
        let mut shown = 0;
        for _ in 0..100 {
            m.update(&mut rt, 1.0 / 72.0);
            assert!(m.frame_loop());
            let t = rt.wait_frame().unwrap();
            let views = rt.locate_views(pose_time(&t, true)).unwrap();
            assert!((ipd(views.as_ref().unwrap()) - DEFAULT_IPD_M).abs() < 1e-6);
            let latency = rt
                .now_ns()
                .map(|now| (t.display_time_ns - now) as f32 / 1e6);
            stats.record(9.0, &t, latency);
            if submit(&mut rt, &t, views.as_ref(), &mut |_| Ok(())).unwrap()
                == crate::frame::Submitted::Shown
            {
                shown += 1;
            }
        }
        assert_eq!(*m.phase(), Phase::Focused);
        assert_eq!(shown, 100);
        let s = stats.summary();
        assert_eq!(s.missed, 0);
        assert!(
            (s.latency_ms - 27.78).abs() < 0.01,
            "two periods: {}",
            s.latency_ms
        );
        m.request_exit(&mut rt);
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Stopping));
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Exiting));
        m.update(&mut rt, 0.0);
        assert_eq!(*m.phase(), Phase::Exited);
        assert!(!m.frame_loop());
    }
}

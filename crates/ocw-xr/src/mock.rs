//! An in-memory runtime (no headset, no OpenXR): scripted events, a log of
//! the calls made, and a head that looks around ([`crate::view::sim_head`]).
//! For the tests here and in the renderer, and for `--vr-sim`'s frame loop.

use std::collections::VecDeque;

use crate::controllers::{ControllerSource, ControllerState};
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
    SyncActions,
    /// The session's teardown (`XrRuntime::shutdown`).
    Shutdown,
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
    /// What `sync_controllers` reports (`None`: no controllers).
    pub controllers: Option<ControllerState>,
    /// After `shutdown` every call fails (`SessionLost`).
    pub shut_down: bool,
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
            controllers: None,
            shut_down: false,
        }
    }
}

impl MockRuntime {
    fn frame_call(&mut self, c: Call) -> Result<(), XrError> {
        if self.shut_down {
            return Err(XrError::SessionLost);
        }
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

    fn shutdown(&mut self) {
        self.calls.push(Call::Shutdown);
        self.shut_down = true;
    }
}

impl ControllerSource for MockRuntime {
    fn sync_controllers(&mut self) -> Result<Option<ControllerState>, XrError> {
        if self.shut_down {
            return Err(XrError::SessionLost);
        }
        self.calls.push(Call::SyncActions);
        Ok(self.controllers)
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

    /// Regression (2026-11-17, Weela: 0xC0000005 after "rider left"): the
    /// app's exit tears the session down itself, in its last frame, and
    /// nothing touches the session afterwards.
    #[test]
    fn the_exit_tears_the_session_down_once() {
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::new();
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Ready));
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Focused));
        m.update(&mut rt, 0.0);
        assert!(m.frame_loop());
        m.shutdown(&mut rt);
        assert_eq!(*m.phase(), Phase::Exited);
        assert!(!m.frame_loop(), "no frames after the teardown");
        let tail: Vec<Call> = rt.calls.iter().rev().take(2).rev().copied().collect();
        assert_eq!(tail, [Call::RequestExit, Call::Shutdown]);
        assert_eq!(rt.wait_frame(), Err(XrError::SessionLost));
        assert_eq!(rt.sync_controllers(), Err(XrError::SessionLost));
        // A second exit does nothing more; reconnects stop.
        m.shutdown(&mut rt);
        m.update(&mut rt, 10.0);
        assert!(!rt.calls.contains(&Call::RecreateSession));
        // Without VR there is nothing to tear down.
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::unavailable(XrError::NoHeadset);
        m.shutdown(&mut rt);
        assert!(rt.calls.is_empty());
    }

    #[test]
    fn the_mock_reports_its_controllers() {
        let mut rt = MockRuntime::default();
        assert_eq!(rt.sync_controllers(), Ok(None));
        let s = ControllerState::at_rest(true);
        rt.controllers = Some(s);
        assert_eq!(rt.sync_controllers(), Ok(Some(s)));
        assert_eq!(rt.calls.last(), Some(&Call::SyncActions));
    }
}

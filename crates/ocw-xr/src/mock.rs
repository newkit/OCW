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
    EndFrame {
        layer: bool,
    },
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
    /// The next `acquire_image` fails with this once.
    pub image_error: Option<XrError>,
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
    /// Breaks of OpenXR's frame rules seen (§10.4: wait → begin → end; an
    /// image acquired → waited → released before `xrEndFrame`), in order.
    pub violations: Vec<&'static str>,
    /// Like the Oculus runtime (v4.64): a broken frame rule loses the
    /// session (the call returns `SessionLost`).
    pub lose_on_violation: bool,
    /// A waited frame not begun yet.
    pub waited_open: bool,
    /// A begun frame not ended yet.
    pub begun_open: bool,
    /// The acquired image: 0 none, 1 acquired, 2 waited.
    pub image_state: u8,
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
            image_error: None,
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
            violations: Vec::new(),
            lose_on_violation: false,
            waited_open: false,
            begun_open: false,
            image_state: 0,
        }
    }
}

impl MockRuntime {
    /// A frame rule was broken: noted; with [`Self::lose_on_violation`] the
    /// session is lost.
    fn rule(&mut self, broken: bool, what: &'static str) -> Result<(), XrError> {
        if !broken {
            return Ok(());
        }
        self.violations.push(what);
        if self.lose_on_violation {
            Err(XrError::SessionLost)
        } else {
            Ok(())
        }
    }

    /// Images acquired and not released (must be 0 at every `xrEndFrame`).
    pub fn images_held(&self) -> u8 {
        u8::from(self.image_state != 0)
    }

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
        // A new session starts with a clean frame loop.
        self.waited_open = false;
        self.begun_open = false;
        self.image_state = 0;
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
        let open = self.waited_open;
        self.rule(
            open,
            "xrWaitFrame before the last waited frame's xrBeginFrame",
        )?;
        self.waited_open = true;
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
        self.frame_call(Call::BeginFrame)?;
        let (waited, begun) = (self.waited_open, self.begun_open);
        self.rule(!waited, "xrBeginFrame without xrWaitFrame")?;
        self.rule(begun, "xrBeginFrame discarded a frame never ended")?;
        self.waited_open = false;
        self.begun_open = true;
        Ok(())
    }

    fn acquire_image(&mut self) -> Result<u32, XrError> {
        self.calls.push(Call::AcquireImage);
        if let Some(e) = self.image_error.take() {
            return Err(e);
        }
        let held = self.image_state != 0;
        self.rule(held, "xrAcquireSwapchainImage with an image not released")?;
        self.image_state = 1;
        let i = self.next_image;
        self.next_image = (self.next_image + 1) % self.images.max(1);
        Ok(i)
    }

    fn wait_image(&mut self) -> Result<(), XrError> {
        self.calls.push(Call::WaitImage);
        let acquired = self.image_state == 1;
        self.rule(!acquired, "xrWaitSwapchainImage without an acquired image")?;
        self.image_state = 2;
        Ok(())
    }

    fn release_image(&mut self) -> Result<(), XrError> {
        self.calls.push(Call::ReleaseImage);
        let waited = self.image_state == 2;
        self.rule(!waited, "xrReleaseSwapchainImage without a waited image")?;
        self.image_state = 0;
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
        let (begun, held) = (self.begun_open, self.image_state != 0);
        self.rule(!begun, "xrEndFrame without xrBeginFrame")?;
        self.rule(held, "xrEndFrame with a swapchain image not released")?;
        self.begun_open = false;
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

    use crate::frame::{FrameCall, FramePacer, FrameTiming, begin, finish};
    use std::collections::VecDeque;

    /// Bevy's two worlds against the mock: the main world waits, the
    /// render world begins and ends the frames it was handed, a frame
    /// behind (pipelined) – and every `hitch_every` frames the render
    /// thread is held for three frames (a slow frame: shaders, uploads).
    /// Returns the runtime, frames waited and frames shown.
    fn pipelined(pacer_on: bool, hitch_every: u32, steps: u32) -> (MockRuntime, u32, u32) {
        let mut rt = MockRuntime::default();
        let mut pacer = FramePacer::default();
        let mut queue: VecDeque<(u64, FrameTiming)> = VecDeque::new();
        let (mut waits, mut shown, mut hitch) = (0, 0, 0);
        let mut clock = || 0.0;
        let mut report = |_: FrameCall, _: f32| {};
        let views = Some(crate::view::sim_views(
            &crate::Pose::IDENTITY,
            DEFAULT_IPD_M,
            &QUEST1_LEFT_FOV,
        ));
        for step in 0..steps {
            if !pacer_on || pacer.may_wait() {
                let t = rt.wait_frame().unwrap();
                waits += 1;
                queue.push_back((pacer.waited(), t));
            }
            if step % hitch_every == hitch_every - 1 {
                hitch = 3;
            }
            if hitch > 0 {
                hitch -= 1;
                continue;
            }
            if let Some((id, t)) = queue.pop_front() {
                let begun = begin(&mut rt, &mut clock, &mut report);
                pacer.handled(id);
                if begun.is_ok()
                    && finish(
                        &mut rt,
                        &t,
                        views.as_ref(),
                        &mut |_| Ok(()),
                        &mut clock,
                        &mut report,
                    ) == Ok(crate::frame::Submitted::Shown)
                {
                    shown += 1;
                }
            }
        }
        (rt, waits, shown)
    }

    /// Regression (2026-11-17, the VR kiosk on Weela, Oculus runtime
    /// 1.208 over Link: "ovr_WaitToBeginFrame missing ovr_Begin/EndFrame
    /// calls", then the session lost, every ~30 s): without the handshake a
    /// slow render frame lets the main world wait for the next frame before
    /// the last one was begun.
    #[test]
    fn a_slow_render_frame_without_the_pacer_breaks_the_frame_rules() {
        let (rt, _, _) = pipelined(false, 50, 200);
        assert!(
            rt.violations
                .contains(&"xrWaitFrame before the last waited frame's xrBeginFrame"),
            "{:?}",
            rt.violations
        );
    }

    /// With the pacer every waited frame is begun before the next wait, a
    /// slow frame only delays the loop, every frame is submitted (shown),
    /// and nothing breaks OpenXR's order (an image is never left acquired).
    #[test]
    fn a_slow_frame_never_stops_submission_with_the_pacer() {
        let (rt, waits, shown) = pipelined(true, 50, 2000);
        assert!(rt.violations.is_empty(), "{:?}", rt.violations);
        assert!(waits > 1800, "{waits} frames waited of 2000 steps");
        // All but the last one (still queued at the end) reached the runtime.
        assert!(waits - shown <= 1, "{waits} waited, {shown} shown");
        assert_eq!(rt.images_held(), 0);
        // Even with the render thread held three frames of every five.
        let (rt, waits, shown) = pipelined(true, 5, 400);
        assert!(rt.violations.is_empty(), "{:?}", rt.violations);
        assert!(waits - shown <= 1 && shown > 50, "{waits} / {shown}");
    }

    /// A failed acquire / wait / release still ends the begun frame: the
    /// runtime never sees a frame left open (the next begin would discard
    /// it, and Link drops a session whose frames go missing).
    #[test]
    fn a_failed_swapchain_call_still_ends_the_frame() {
        let mut rt = MockRuntime {
            image_error: Some(XrError::Other("acquire".into())),
            ..Default::default()
        };
        let mut clock = || 0.0;
        let mut report = |_: FrameCall, _: f32| {};
        let t = rt.wait_frame().unwrap();
        begin(&mut rt, &mut clock, &mut report).unwrap();
        let views = rt.locate_views(t.display_time_ns).unwrap();
        let r = finish(
            &mut rt,
            &t,
            views.as_ref(),
            &mut |_| Ok(()),
            &mut clock,
            &mut report,
        );
        assert_eq!(r.unwrap_err().call, FrameCall::AcquireImage);
        assert_eq!(rt.calls.last(), Some(&Call::EndFrame { layer: false }));
        // The next frame is a normal one.
        let t = rt.wait_frame().unwrap();
        begin(&mut rt, &mut clock, &mut report).unwrap();
        let r = finish(
            &mut rt,
            &t,
            views.as_ref(),
            &mut |_| Ok(()),
            &mut clock,
            &mut report,
        );
        assert_eq!(r, Ok(crate::frame::Submitted::Shown));
        assert!(rt.violations.is_empty(), "{:?}", rt.violations);
        assert_eq!(rt.images_held(), 0);
    }

    /// A session lost in the middle of the loop: the machine reconnects, the
    /// new session gets a clean frame loop (nothing of the old one
    /// outstanding) and frames reach it – with a runtime as strict as
    /// Oculus's, which loses the session on any broken rule.
    #[test]
    fn a_reconnect_gets_a_clean_session_that_receives_frames() {
        let mut rt = MockRuntime {
            lose_on_violation: true,
            ..Default::default()
        };
        let mut m = SessionMachine::new();
        let mut pacer = FramePacer::default();
        let mut clock = || 0.0;
        let mut report = |_: FrameCall, _: f32| {};
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Ready));
        rt.events
            .push_back(RuntimeEvent::State(RuntimeState::Focused));
        let mut shown_after = 0;
        for frame in 0..400 {
            m.update(&mut rt, 1.0 / 72.0);
            if frame == 100 {
                // Lost after the wait, before the frame was begun.
                m.frame_error(XrError::SessionLost);
                rt.events.push_back(RuntimeEvent::State(RuntimeState::Idle));
                rt.events
                    .push_back(RuntimeEvent::State(RuntimeState::Ready));
                rt.events
                    .push_back(RuntimeEvent::State(RuntimeState::Focused));
                continue;
            }
            if !m.frame_loop() {
                pacer.new_session();
                continue;
            }
            assert!(pacer.may_wait(), "frame {frame}");
            let t = rt.wait_frame().unwrap();
            let id = pacer.waited();
            let views = rt.locate_views(t.display_time_ns).unwrap();
            begin(&mut rt, &mut clock, &mut report).unwrap();
            pacer.handled(id);
            let r = finish(
                &mut rt,
                &t,
                views.as_ref(),
                &mut |_| Ok(()),
                &mut clock,
                &mut report,
            );
            if frame > 100 && r == Ok(crate::frame::Submitted::Shown) {
                shown_after += 1;
            }
        }
        assert_eq!(m.reconnects, 1);
        assert_eq!(*m.phase(), Phase::Focused);
        assert!(rt.violations.is_empty(), "{:?}", rt.violations);
        assert!(shown_after > 100, "{shown_after} frames in the new session");
        assert!(rt.calls.contains(&Call::RecreateSession));
    }

    #[test]
    fn the_pacer_lets_a_wait_follow_only_a_begun_frame() {
        let mut p = FramePacer::default();
        assert!(p.may_wait());
        let a = p.waited();
        assert!(!p.may_wait());
        assert_eq!(p.outstanding(), 1);
        p.handled(a);
        assert!(p.may_wait());
        let _ = p.waited();
        // A new session forgets the old frame.
        p.new_session();
        assert!(p.may_wait());
        // Old ids do not count twice.
        let c = p.waited();
        p.handled(a);
        assert!(!p.may_wait());
        p.handled(c);
        assert!(p.may_wait());
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

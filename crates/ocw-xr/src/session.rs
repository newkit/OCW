//! The VR session's state machine (spec §14 Phase 11, X1).
//!
//! OpenXR drives a session through `XrSessionState`s; the app answers some
//! of them (begin the session when READY, end it when STOPPING) and keeps
//! the frame loop running in between. Our phases, as `vr status` shows
//! them:
//!
//! ```text
//!  Unavailable (no loader / runtime / headset: the app runs as before)
//!  Idle ──READY──▶ Ready ──VISIBLE──▶ Visible ──FOCUSED──▶ Focused
//!    ▲               (frame loop runs from Ready until Stopping)  │
//!    │                                                            │
//!    └──IDLE── Stopping ◀─────────────STOPPING────────────────────┘
//!  LOSS_PENDING / EXITING / errors ──▶ Lost ──retry every 2 s──▶ Idle
//!  INSTANCE_LOSS_PENDING ──▶ Lost (needs a restart of the app)
//! ```
//!
//! A lost session (the headset unplugged, Link restarted, the runtime's
//! dashboard closed the app) is **reconnected**: every
//! [`SessionMachine::retry_s`] seconds a new session is tried on the same
//! OpenXR instance and GPU device, so the ride goes on on the monitor
//! meanwhile. A lost *instance* (the runtime itself went away) cannot be
//! reconnected without a new GPU device: the app says so and `vr
//! reconnect` restarts it.
//!
//! The runtime is behind [`XrRuntime`], so the machine is tested with
//! [`crate::mock::MockRuntime`]; the OpenXR binding is `ocw-xr-openxr`.

use std::fmt;

/// `XrSessionState`, as the runtime reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeState {
    Idle,
    Ready,
    Synchronized,
    Visible,
    Focused,
    Stopping,
    LossPending,
    Exiting,
}

/// What the runtime tells between frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeEvent {
    State(RuntimeState),
    /// The runtime goes away (`XrEventDataInstanceLossPending`).
    InstanceLossPending,
    /// The user recentred in the runtime's own menu (`LOCAL` space moved):
    /// our recentre is redone.
    ReferenceSpaceChanged,
}

/// Why VR does not run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XrError {
    /// The build has no VR (`--features vr`).
    NotBuilt,
    /// No OpenXR loader library (libopenxr_loader.so.1 / openxr_loader.dll).
    NoLoader(String),
    /// A loader but no active runtime (none installed, or not set active).
    NoRuntime(String),
    /// A runtime but no headset (not connected, Link off).
    NoHeadset,
    /// The runtime or GPU lacks what we need (Vulkan, a format).
    Unsupported(String),
    /// The session is gone; a new one may work.
    SessionLost,
    /// The instance is gone; only a restart helps.
    InstanceLost,
    Other(String),
}

impl fmt::Display for XrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotBuilt => {
                f.write_str("this build has no VR (build ocw-desktop with --features vr)")
            }
            Self::NoLoader(e) => write!(
                f,
                "no OpenXR loader found ({e}); install a runtime (Meta Quest Link app, SteamVR, Monado) or put openxr_loader beside the app"
            ),
            Self::NoRuntime(e) => write!(
                f,
                "no active OpenXR runtime ({e}); set one active (Meta Quest Link app: Settings → General → OpenXR Runtime)"
            ),
            Self::NoHeadset => f.write_str(
                "no headset found: connect it (Oculus Link) and start again, or `vr reconnect`",
            ),
            Self::Unsupported(e) => write!(f, "the OpenXR runtime cannot run OCW: {e}"),
            Self::SessionLost => f.write_str("the VR session was lost"),
            Self::InstanceLost => {
                f.write_str("the OpenXR runtime went away (restart the app to reconnect)")
            }
            Self::Other(e) => f.write_str(e),
        }
    }
}

impl std::error::Error for XrError {}

/// The calls the machine makes; implemented by the OpenXR binding and by
/// the mock.
pub trait XrRuntime {
    /// The next event, if any (never blocks).
    fn poll_event(&mut self) -> Result<Option<RuntimeEvent>, XrError>;
    /// `xrBeginSession` (primary stereo).
    fn begin_session(&mut self) -> Result<(), XrError>;
    /// `xrEndSession`.
    fn end_session(&mut self) -> Result<(), XrError>;
    /// `xrRequestExitSession` (the app quits or VR is switched off).
    fn request_exit(&mut self) -> Result<(), XrError>;
    /// After a loss: destroy the old session and create a new one on the
    /// same instance and device (`NoHeadset` while none is there).
    fn recreate_session(&mut self) -> Result<(), XrError>;
    /// The app quits: wait for the GPU, then destroy the swapchain, the
    /// frame stream and the session – before the GPU device and the
    /// instance go (v4.38: the exit crash on the Quest 1 over Link came
    /// from the other order). Nothing works on the session after it.
    fn shutdown(&mut self) {}
}

/// Our phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// No VR; why.
    Unavailable(XrError),
    /// A session, waiting for the runtime (headset not on, or not ready).
    Idle,
    /// Running, frames rendered, not yet shown (READY / SYNCHRONIZED).
    Ready,
    /// Shown in the headset, without input focus (the runtime's menu is
    /// over it).
    Visible,
    /// Shown and focused: the normal ride.
    Focused,
    /// The runtime asked to stop; the session was ended.
    Stopping,
    /// Lost; reconnecting (`restart` if only a restart helps).
    Lost { restart: bool },
    /// The session ended because the app asked.
    Exited,
}

impl Phase {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Unavailable(_) => "unavailable",
            Self::Idle => "idle",
            Self::Ready => "ready",
            Self::Visible => "visible",
            Self::Focused => "focused",
            Self::Stopping => "stopping",
            Self::Lost { .. } => "lost",
            Self::Exited => "exited",
        }
    }
}

/// The session's state machine.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionMachine {
    phase: Phase,
    /// `xrBeginSession` succeeded and `xrEndSession` not yet called.
    running: bool,
    exit_requested: bool,
    /// Seconds between reconnect attempts.
    pub retry_s: f32,
    since_retry_s: f32,
    /// Reconnects so far (for `vr status`).
    pub reconnects: u32,
    /// The last error (for `vr status`).
    pub last_error: Option<XrError>,
    /// Set when the runtime moved its reference space: redo the recentre.
    pub recentre_wanted: bool,
}

impl SessionMachine {
    /// A session was created: wait for the runtime.
    pub fn new() -> Self {
        Self::with_phase(Phase::Idle)
    }

    /// No session could be created.
    pub fn unavailable(why: XrError) -> Self {
        let mut m = Self::with_phase(Phase::Unavailable(why.clone()));
        m.last_error = Some(why);
        m
    }

    fn with_phase(phase: Phase) -> Self {
        Self {
            phase,
            running: false,
            exit_requested: false,
            retry_s: 2.0,
            since_retry_s: 0.0,
            reconnects: 0,
            last_error: None,
            recentre_wanted: false,
        }
    }

    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    /// The frame loop (wait, begin, end frame) must run.
    pub fn frame_loop(&self) -> bool {
        self.running && matches!(self.phase, Phase::Ready | Phase::Visible | Phase::Focused)
    }

    /// The headset shows our frames.
    pub fn visible(&self) -> bool {
        matches!(self.phase, Phase::Visible | Phase::Focused)
    }

    pub fn is_available(&self) -> bool {
        !matches!(self.phase, Phase::Unavailable(_))
    }

    /// Handles the runtime's events and reconnects; `dt`: seconds since
    /// the last call.
    pub fn update(&mut self, rt: &mut dyn XrRuntime, dt: f32) {
        if matches!(self.phase, Phase::Unavailable(_) | Phase::Exited) {
            return;
        }
        if let Phase::Lost { restart } = self.phase {
            if restart || self.exit_requested {
                return;
            }
            self.since_retry_s += dt.max(0.0);
            if self.since_retry_s < self.retry_s {
                return;
            }
            self.since_retry_s = 0.0;
            match rt.recreate_session() {
                Ok(()) => {
                    self.reconnects += 1;
                    self.phase = Phase::Idle;
                }
                Err(XrError::InstanceLost) => self.lose(true, XrError::InstanceLost),
                Err(e) => self.last_error = Some(e),
            }
            return;
        }
        // A bounded number per frame: a runtime that floods cannot hang us.
        for _ in 0..64 {
            match rt.poll_event() {
                Ok(Some(e)) => self.on_event(rt, e),
                Ok(None) => break,
                Err(e) => {
                    let restart = e == XrError::InstanceLost;
                    self.lose(restart, e);
                    break;
                }
            }
        }
    }

    fn on_event(&mut self, rt: &mut dyn XrRuntime, e: RuntimeEvent) {
        use RuntimeState as S;
        match e {
            RuntimeEvent::InstanceLossPending => self.lose(true, XrError::InstanceLost),
            RuntimeEvent::ReferenceSpaceChanged => self.recentre_wanted = true,
            RuntimeEvent::State(s) => match s {
                S::Idle => {
                    if !matches!(self.phase, Phase::Lost { .. }) {
                        self.phase = Phase::Idle;
                    }
                }
                S::Ready => match rt.begin_session() {
                    Ok(()) => {
                        self.running = true;
                        self.phase = Phase::Ready;
                        // A fresh session: the head is wherever it is now.
                        self.recentre_wanted = true;
                    }
                    Err(e) => self.lose(false, e),
                },
                S::Synchronized => {
                    if self.running {
                        self.phase = Phase::Ready;
                    }
                }
                S::Visible => {
                    if self.running {
                        self.phase = Phase::Visible;
                    }
                }
                S::Focused => {
                    if self.running {
                        self.phase = Phase::Focused;
                    }
                }
                S::Stopping => {
                    if self.running {
                        if let Err(e) = rt.end_session() {
                            self.last_error = Some(e);
                        }
                        self.running = false;
                    }
                    self.phase = Phase::Stopping;
                }
                S::Exiting if self.exit_requested => {
                    self.running = false;
                    self.phase = Phase::Exited;
                }
                // The runtime closed us (its dashboard, Link turned off):
                // reconnect when it lets us.
                S::Exiting | S::LossPending => self.lose(false, XrError::SessionLost),
            },
        }
    }

    fn lose(&mut self, restart: bool, why: XrError) {
        self.running = false;
        self.since_retry_s = 0.0;
        self.last_error = Some(why);
        self.phase = Phase::Lost { restart };
    }

    /// The app quits now (the last frame): asks the runtime to stop and
    /// tears the session down at once ([`XrRuntime::shutdown`]); the
    /// phase is `Exited` after it.
    pub fn shutdown(&mut self, rt: &mut dyn XrRuntime) {
        if matches!(self.phase, Phase::Unavailable(_)) {
            return;
        }
        if self.running
            && let Err(e) = rt.request_exit()
        {
            self.last_error = Some(e);
        }
        self.exit_requested = true;
        self.running = false;
        rt.shutdown();
        self.phase = Phase::Exited;
    }

    /// The app quits or VR is switched off: ask the runtime to stop.
    pub fn request_exit(&mut self, rt: &mut dyn XrRuntime) {
        self.exit_requested = true;
        if self.running {
            if let Err(e) = rt.request_exit() {
                self.last_error = Some(e);
            }
        } else if !matches!(self.phase, Phase::Unavailable(_)) {
            self.phase = Phase::Exited;
        }
    }

    /// A frame call failed: a lost session is reconnected, others noted.
    pub fn frame_error(&mut self, e: XrError) {
        match e {
            XrError::SessionLost => self.lose(false, e),
            XrError::InstanceLost => self.lose(true, e),
            other => self.last_error = Some(other),
        }
    }

    /// One line for `vr status`.
    pub fn summary(&self) -> String {
        let mut s = match &self.phase {
            Phase::Unavailable(why) => format!("unavailable: {why}"),
            Phase::Lost { restart: true } => "lost (restart the app to reconnect)".into(),
            Phase::Lost { restart: false } => {
                format!("lost, reconnecting every {:.0} s", self.retry_s)
            }
            p => p.name().to_string(),
        };
        if self.reconnects > 0 {
            s.push_str(&format!(", {} reconnects", self.reconnects));
        }
        if let (Some(e), false) = (
            &self.last_error,
            matches!(self.phase, Phase::Unavailable(_)),
        ) {
            s.push_str(&format!(" (last error: {e})"));
        }
        s
    }
}

impl Default for SessionMachine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mock::{Call, MockRuntime};
    use RuntimeState as S;

    fn run(m: &mut SessionMachine, rt: &mut MockRuntime, events: &[RuntimeEvent]) {
        rt.events.extend(events.iter().copied());
        m.update(rt, 0.016);
    }

    #[test]
    fn idle_ready_focused_stopping_idle() {
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::new();
        assert_eq!(*m.phase(), Phase::Idle);
        assert!(!m.frame_loop());
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Ready)]);
        assert_eq!(*m.phase(), Phase::Ready);
        assert!(m.frame_loop() && !m.visible());
        assert!(m.recentre_wanted, "a new session recentres");
        assert_eq!(rt.calls, [Call::BeginSession]);
        run(
            &mut m,
            &mut rt,
            &[
                RuntimeEvent::State(S::Synchronized),
                RuntimeEvent::State(S::Visible),
                RuntimeEvent::State(S::Focused),
            ],
        );
        assert_eq!(*m.phase(), Phase::Focused);
        assert!(m.frame_loop() && m.visible());
        // The runtime's menu over us: visible, not focused.
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Visible)]);
        assert_eq!(*m.phase(), Phase::Visible);
        assert!(m.frame_loop());
        // Headset taken off: stopping → the session is ended, then idle.
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Stopping)]);
        assert_eq!(*m.phase(), Phase::Stopping);
        assert!(!m.frame_loop());
        assert_eq!(rt.calls, [Call::BeginSession, Call::EndSession]);
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Idle)]);
        assert_eq!(*m.phase(), Phase::Idle);
        // Put on again: a second begin.
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Ready)]);
        assert_eq!(
            rt.calls
                .iter()
                .filter(|c| **c == Call::BeginSession)
                .count(),
            2
        );
        assert_eq!(m.summary(), "ready");
    }

    #[test]
    fn a_lost_session_reconnects() {
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::new();
        run(
            &mut m,
            &mut rt,
            &[
                RuntimeEvent::State(S::Ready),
                RuntimeEvent::State(S::Focused),
            ],
        );
        // Link unplugged.
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::LossPending)]);
        assert_eq!(*m.phase(), Phase::Lost { restart: false });
        assert!(!m.frame_loop());
        assert!(
            m.summary().starts_with("lost, reconnecting"),
            "{}",
            m.summary()
        );
        // No headset yet: tried every 2 s, nothing more.
        rt.recreate_results.push_back(Err(XrError::NoHeadset));
        m.update(&mut rt, 1.0);
        assert!(!rt.calls.contains(&Call::RecreateSession), "not before 2 s");
        m.update(&mut rt, 1.1);
        assert_eq!(
            rt.calls
                .iter()
                .filter(|c| **c == Call::RecreateSession)
                .count(),
            1
        );
        assert_eq!(m.last_error, Some(XrError::NoHeadset));
        assert_eq!(*m.phase(), Phase::Lost { restart: false });
        // Plugged in again: a new session, the runtime says ready.
        m.update(&mut rt, 2.0);
        assert_eq!(*m.phase(), Phase::Idle);
        assert_eq!(m.reconnects, 1);
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Ready)]);
        assert!(m.frame_loop());
        // A frame call that loses the session reconnects too.
        m.frame_error(XrError::SessionLost);
        assert_eq!(*m.phase(), Phase::Lost { restart: false });
        // The runtime's own exit (its dashboard) is a loss, not our exit.
        let mut m = SessionMachine::new();
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Exiting)]);
        assert_eq!(*m.phase(), Phase::Lost { restart: false });
    }

    #[test]
    fn a_lost_instance_needs_a_restart() {
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::new();
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Ready)]);
        run(&mut m, &mut rt, &[RuntimeEvent::InstanceLossPending]);
        assert_eq!(*m.phase(), Phase::Lost { restart: true });
        for _ in 0..10 {
            m.update(&mut rt, 5.0);
        }
        assert!(!rt.calls.contains(&Call::RecreateSession));
        assert!(m.summary().contains("restart"), "{}", m.summary());
        // An instance lost while polling, and while reconnecting.
        let mut m = SessionMachine::new();
        rt.poll_error = Some(XrError::InstanceLost);
        m.update(&mut rt, 0.0);
        assert_eq!(*m.phase(), Phase::Lost { restart: true });
        rt.poll_error = None;
        let mut m = SessionMachine::new();
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::LossPending)]);
        rt.recreate_results.push_back(Err(XrError::InstanceLost));
        m.update(&mut rt, 3.0);
        assert_eq!(*m.phase(), Phase::Lost { restart: true });
    }

    #[test]
    fn exit_asks_the_runtime_and_ends_the_session() {
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::new();
        run(
            &mut m,
            &mut rt,
            &[
                RuntimeEvent::State(S::Ready),
                RuntimeEvent::State(S::Focused),
            ],
        );
        m.request_exit(&mut rt);
        assert_eq!(rt.calls.last(), Some(&Call::RequestExit));
        run(
            &mut m,
            &mut rt,
            &[
                RuntimeEvent::State(S::Stopping),
                RuntimeEvent::State(S::Exiting),
            ],
        );
        assert_eq!(*m.phase(), Phase::Exited);
        assert!(rt.calls.contains(&Call::EndSession));
        // Nothing more happens after the exit.
        let n = rt.calls.len();
        m.update(&mut rt, 10.0);
        assert_eq!(rt.calls.len(), n);
        // Exit before the session ever ran: no call.
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::new();
        m.request_exit(&mut rt);
        assert_eq!(*m.phase(), Phase::Exited);
        assert!(rt.calls.is_empty());
    }

    #[test]
    fn without_a_runtime_nothing_is_called() {
        let mut rt = MockRuntime::default();
        let mut m = SessionMachine::unavailable(XrError::NoLoader("libopenxr_loader.so.1".into()));
        rt.events.push_back(RuntimeEvent::State(S::Ready));
        m.update(&mut rt, 1.0);
        m.request_exit(&mut rt);
        assert!(rt.calls.is_empty());
        assert!(!m.is_available() && !m.frame_loop());
        let s = m.summary();
        assert!(s.starts_with("unavailable: no OpenXR loader found"), "{s}");
        assert!(XrError::NotBuilt.to_string().contains("--features vr"));
        assert!(
            XrError::NoRuntime("x".into())
                .to_string()
                .contains("OpenXR Runtime")
        );
    }

    #[test]
    fn a_failed_begin_is_a_loss_and_the_reference_space_change_recentres() {
        let mut rt = MockRuntime {
            begin_error: Some(XrError::Other("begin failed".into())),
            ..Default::default()
        };
        let mut m = SessionMachine::new();
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Ready)]);
        assert_eq!(*m.phase(), Phase::Lost { restart: false });
        let mut m = SessionMachine::new();
        rt.begin_error = None;
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Ready)]);
        m.recentre_wanted = false;
        run(&mut m, &mut rt, &[RuntimeEvent::ReferenceSpaceChanged]);
        assert!(m.recentre_wanted);
        // Visible / focused without a running session are ignored.
        let mut m = SessionMachine::new();
        run(&mut m, &mut rt, &[RuntimeEvent::State(S::Focused)]);
        assert_eq!(*m.phase(), Phase::Idle);
    }
}

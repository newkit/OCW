//! Our own thin OpenXR layer (spec §14 Phase 11, X1, Q3 variant B).
//!
//! [`boot`] runs before the renderer starts: it loads the OpenXR loader,
//! creates the instance, finds the headset, makes the Vulkan device the
//! headset needs ([`vulkan`]) and creates the session, the `LOCAL`
//! reference space (seated: the origin where the head was) and a
//! swapchain of two array layers (one per eye). The renderer takes the
//! wgpu device of [`Booted::gpu`] (Bevy's `RenderCreation::Manual`) and
//! the two handles:
//!
//! - [`MainHandle`] (main thread): the session calls
//!   ([`ocw_xr::session::XrRuntime`]) and `xrWaitFrame` / `xrLocateViews`
//!   ([`ocw_xr::frame::FrameSource`]);
//! - [`RenderHandle`] (render thread): `xrBeginFrame`, the swapchain and
//!   `xrEndFrame` ([`ocw_xr::frame::FrameSink`]) – the eye textures are
//!   copied into the swapchain images, which [`RenderHandle::images`]
//!   gives.
//!
//! Every failure is an [`XrError`]; the app then runs without VR. A lost
//! session is recreated on the same instance and device
//! ([`XrRuntime::recreate_session`]); the swapchain's generation changes,
//! so the renderer re-reads the images.
//!
//! **Controllers** ([`controls`], v4.52): one action set with an action per
//! input of `ocw_xr::controllers` and its suggested bindings (Oculus
//! Touch, the Khronos simple controller), attached to every session;
//! [`MainHandle`] syncs it each frame ([`ControllerSource`]).
//!
//! **Frame pacing** (v4.64, the VR kiosk on Weela lost its session every
//! ~30 s): `xrWaitFrame` runs only after the last waited frame's
//! `xrBeginFrame` ([`ocw_xr::frame::FramePacer`], [`Shared::pacer`]) –
//! the Oculus runtime does not block a second wait as OpenXR says, it
//! reports "ovr_WaitToBeginFrame missing ovr_Begin/EndFrame calls" and
//! loses the session. **Reconnects run in the background**
//! ([`MainHandle::recreate_session`]): creating a session and its
//! swapchain took the Oculus runtime ~25 s, which froze the app.
//!
//! **Teardown** (v4.52, the exit crash on the Quest 1 over Link): the app's
//! last frame calls [`XrRuntime::shutdown`]: wait for the GPU, then drop the
//! swapchain (the wrapped textures never own its images), the frame
//! stream and the session – while the Vulkan device still lives. The
//! render thread submits under [`Shared::frame`], so it never copies into
//! a swapchain being destroyed.

pub mod controls;
pub mod vulkan;

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ocw_xr::controllers::{ControllerSource, ControllerState};
use ocw_xr::frame::{FramePacer, FrameSink, FrameSource, FrameTiming};
use ocw_xr::headset::{self, HeadsetProfile};
use ocw_xr::projection::Fov;
use ocw_xr::session::{RuntimeEvent, RuntimeState, XrError, XrRuntime};
use ocw_xr::swapchain::{ColorFormat, FormatSpace, choose_format, eye_extent};
use ocw_xr::view::{EyeView, StereoViews};
use ocw_xr::{Pose, Quat, Vec3};
use openxr as xr;

const VIEW_TYPE: xr::ViewConfigurationType = xr::ViewConfigurationType::PRIMARY_STEREO;

/// OpenXR's result as our error.
pub fn xr_error(e: xr::sys::Result) -> XrError {
    use xr::sys::Result as R;
    match e {
        R::ERROR_SESSION_LOST => XrError::SessionLost,
        R::ERROR_INSTANCE_LOST => XrError::InstanceLost,
        R::ERROR_FORM_FACTOR_UNAVAILABLE => XrError::NoHeadset,
        R::ERROR_RUNTIME_UNAVAILABLE | R::ERROR_RUNTIME_FAILURE => {
            XrError::NoRuntime(format!("{e}"))
        }
        R::ERROR_EXTENSION_NOT_PRESENT | R::ERROR_FORM_FACTOR_UNSUPPORTED => {
            XrError::Unsupported(format!("{e}"))
        }
        other => XrError::Other(format!("OpenXR: {other}")),
    }
}

/// What to boot.
#[derive(Debug, Clone)]
pub struct BootOptions {
    pub app_name: String,
    /// Scale of the runtime's recommended eye size (the quality preset).
    pub render_scale: f32,
}

impl Default for BootOptions {
    fn default() -> Self {
        Self {
            app_name: "Open Cycling World".into(),
            render_scale: 1.0,
        }
    }
}

/// What the runtime and headset are.
#[derive(Debug, Clone, PartialEq)]
pub struct XrInfo {
    pub runtime: String,
    pub system: String,
    pub profile: &'static HeadsetProfile,
    /// Each eye's image (px).
    pub eye_px: (u32, u32),
    pub recommended_px: (u32, u32),
    pub format: ColorFormat,
    /// The refresh rate asked for (Hz), when the runtime lets us choose.
    pub refresh_hz: Option<f32>,
    /// The GPU the runtime chose (its Vulkan name).
    pub gpu: String,
}

/// The wgpu objects for the renderer.
pub struct Gpu {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

/// A successful boot.
pub struct Booted {
    pub gpu: Gpu,
    pub info: XrInfo,
    pub main: MainHandle,
    pub render: RenderHandle,
}

/// The swapchain and the frame stream (render thread).
struct Stream {
    // Field order is drop order: the wrapped textures before the
    // swapchain that owns their images.
    textures: Vec<wgpu::Texture>,
    swapchain: xr::Swapchain<xr::Vulkan>,
    stream: xr::FrameStream<xr::Vulkan>,
}

/// The session and its space (both threads, briefly locked).
struct Live {
    space: xr::Space,
    session: xr::Session<xr::Vulkan>,
}

struct Shared {
    system: xr::SystemId,
    session_info: xr::vulkan::SessionCreateInfo,
    format: ColorFormat,
    wgpu_format: wgpu::TextureFormat,
    extent: (u32, u32),
    /// Held by the render thread for a whole frame submission (begin,
    /// copy into the swapchain, end) and by the teardown.
    frame: Mutex<()>,
    // Drop order (fields drop top to bottom): the stream and the waiter
    // before the session, the session before the action set, all of them
    // before the instance and the GPU device (the runtime's session uses
    // the device until it is destroyed).
    stream: Mutex<Option<Stream>>,
    waiter: Mutex<Option<xr::FrameWaiter>>,
    live: Mutex<Option<Live>>,
    controls: Option<controls::Controls>,
    /// Bumped by every new session (new swapchain images).
    generation: AtomicU64,
    /// Which waited frame was begun ([`FramePacer`]); `begun` wakes the
    /// main thread waiting for it.
    pacer: Mutex<FramePacer>,
    begun: Condvar,
    /// The app quits: a session made in the background is dropped at once.
    closing: AtomicBool,
    /// `OCW_VR_PACE=end`: the next wait only after the last frame's
    /// `xrEndFrame`, not its `xrBeginFrame` (stricter than OpenXR asks, no
    /// overlap of simulation and rendering: a fallback for runtimes that
    /// want it).
    pace_after_end: bool,
    /// Between `xrBeginSession` and `xrEndSession` / the session's end:
    /// the render thread submits frames only then (a frame extracted
    /// before STOPPING reached `xrBeginFrame` after `xrEndSession`).
    running: AtomicBool,
    instance: xr::Instance,
    device: wgpu::Device,
}

// SAFETY-free note: the raw Vulkan handles in `session_info` are plain
// numbers to OpenXR; the objects they name live in `device` (wgpu-hal),
// which outlives every session (sessions are dropped first, see above).
#[allow(unsafe_code)]
// SAFETY: `SessionCreateInfo` only holds handles (no thread affinity);
// all other fields are `Send + Sync` or behind mutexes.
unsafe impl Send for Shared {}
#[allow(unsafe_code)]
// SAFETY: as above.
unsafe impl Sync for Shared {}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// The loader's file names, tried in order. `Entry::load` alone opens only
/// the unversioned `libopenxr_loader.so`, which on Linux comes with the
/// development package; the runtime package (Debian's `libopenxr-loader1`,
/// Fedora's `openxr-libs`) installs `libopenxr_loader.so.1`.
fn loader_names() -> &'static [&'static str] {
    if cfg!(target_os = "windows") {
        &["openxr_loader.dll"]
    } else if cfg!(target_os = "macos") {
        &["libopenxr_loader.dylib"]
    } else {
        &["libopenxr_loader.so.1", "libopenxr_loader.so"]
    }
}

/// Opens the first loader of [`loader_names`] that loads.
fn load_entry() -> Result<xr::Entry, XrError> {
    let mut errors = Vec::new();
    for name in loader_names() {
        #[allow(unsafe_code)]
        // SAFETY: loading a library that follows the OpenXR loader ABI.
        match unsafe { xr::Entry::load_from(std::path::Path::new(name)) } {
            Ok(entry) => return Ok(entry),
            Err(e) => errors.push(format!("{name}: {e}")),
        }
    }
    Err(XrError::NoLoader(errors.join("; ")))
}

/// Boots OpenXR (see the crate docs).
pub fn boot(opts: &BootOptions) -> Result<Booted, XrError> {
    let entry = load_entry()?;
    let available = entry
        .enumerate_extensions()
        .map_err(|e| XrError::NoRuntime(e.to_string()))?;
    if !available.khr_vulkan_enable2 {
        return Err(XrError::Unsupported(
            "the runtime has no Vulkan support (XR_KHR_vulkan_enable2)".into(),
        ));
    }
    let mut exts = xr::ExtensionSet::default();
    exts.khr_vulkan_enable2 = true;
    exts.fb_display_refresh_rate = available.fb_display_refresh_rate;
    let instance = entry
        .create_instance(
            &xr::ApplicationInfo {
                application_name: &opts.app_name,
                application_version: 1,
                engine_name: "Bevy",
                engine_version: 20,
                api_version: xr::Version::new(1, 0, 0),
            },
            &exts,
            &[],
        )
        .map_err(xr_error)?;
    let props = instance.properties().map_err(xr_error)?;
    let runtime = format!("{} {}", props.runtime_name, props.runtime_version);
    let system = instance
        .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
        .map_err(xr_error)?;
    let system_name = instance
        .system_properties(system)
        .map(|p| p.system_name)
        .unwrap_or_default();
    let profile = headset::detect(&system_name);
    let views = instance
        .enumerate_view_configuration_views(system, VIEW_TYPE)
        .map_err(xr_error)?;
    let Some(v) = views.first() else {
        return Err(XrError::Unsupported("no stereo view configuration".into()));
    };
    let recommended = (
        v.recommended_image_rect_width,
        v.recommended_image_rect_height,
    );
    let max = (v.max_image_rect_width, v.max_image_rect_height);
    // Not more pixels than the panel (the Quest 1 over Link: 1808 × 2000
    // recommended, 1440 × 1600 drawn), times the option.
    let extent = eye_extent(
        recommended,
        max,
        opts.render_scale * headset::pc_render_scale(profile, recommended),
    );
    // The action set and its bindings come before the session (they are
    // the instance's; each session attaches them). Without them the
    // controllers do nothing – the first headset run (2026-11-17) had none.
    let controls = match controls::Controls::new(&instance) {
        Ok(c) => Some(c),
        Err(e) => {
            tracing::warn!("VR: no controller actions: {e}");
            None
        }
    };
    let dev = vulkan::create_device(&instance, system, &opts.app_name)?;
    let (session, waiter, stream) = vulkan::create_session(&instance, system, &dev.session_info)?;
    let gpu = dev.gpu_name.clone();
    let formats = session.enumerate_swapchain_formats().map_err(xr_error)?;
    let offered: Vec<i64> = formats.iter().map(|f| i64::from(*f)).collect();
    let format = choose_format(&offered, FormatSpace::Vulkan).ok_or_else(|| {
        XrError::Unsupported("the runtime offers no sRGB or float colour format".into())
    })?;
    let wgpu_format = vulkan::wgpu_format(format);
    let shared = Arc::new(Shared {
        system,
        session_info: dev.session_info,
        format,
        wgpu_format,
        extent,
        frame: Mutex::new(()),
        stream: Mutex::new(None),
        waiter: Mutex::new(None),
        live: Mutex::new(None),
        controls,
        generation: AtomicU64::new(0),
        pacer: Mutex::new(FramePacer::default()),
        begun: Condvar::new(),
        closing: AtomicBool::new(false),
        pace_after_end: std::env::var("OCW_VR_PACE").is_ok_and(|v| v == "end"),
        running: AtomicBool::new(false),
        instance,
        device: dev.device.clone(),
    });
    let refresh_hz = shared.install(session, waiter, stream)?;
    Ok(Booted {
        gpu: Gpu {
            instance: dev.instance,
            adapter: dev.adapter,
            device: dev.device,
            queue: dev.queue,
        },
        info: XrInfo {
            runtime,
            system: system_name,
            profile,
            eye_px: extent,
            recommended_px: recommended,
            format,
            refresh_hz,
            gpu,
        },
        main: MainHandle {
            shared: shared.clone(),
            reconnect: None,
            frame_id: 0,
        },
        render: RenderHandle { shared },
    })
}

impl Shared {
    /// Takes a new session: its space, swapchain and images; asks for the
    /// refresh rate.
    fn install(
        &self,
        session: xr::Session<xr::Vulkan>,
        waiter: xr::FrameWaiter,
        stream: xr::FrameStream<xr::Vulkan>,
    ) -> Result<Option<f32>, XrError> {
        let space = session
            .create_reference_space(xr::ReferenceSpaceType::LOCAL, xr::Posef::IDENTITY)
            .map_err(xr_error)?;
        let swapchain = session
            .create_swapchain(&xr::SwapchainCreateInfo {
                create_flags: xr::SwapchainCreateFlags::EMPTY,
                usage_flags: xr::SwapchainUsageFlags::COLOR_ATTACHMENT
                    | xr::SwapchainUsageFlags::TRANSFER_DST,
                format: self.format.vk() as u32,
                sample_count: 1,
                width: self.extent.0,
                height: self.extent.1,
                face_count: 1,
                array_size: 2,
                mip_count: 1,
            })
            .map_err(xr_error)?;
        if let Some(c) = &self.controls
            && let Err(e) = c.attach(&session)
        {
            tracing::warn!("VR: the controllers' actions cannot be attached: {e}");
        }
        let images = swapchain.enumerate_images().map_err(xr_error)?;
        let textures = vulkan::wrap_images(&self.device, &images, self.wgpu_format, self.extent)?;
        let refresh = if self.instance.exts().fb_display_refresh_rate.is_some() {
            session
                .enumerate_display_refresh_rates()
                .ok()
                .and_then(|rates| headset::target_rate(&rates))
                .inspect(|hz| {
                    if let Err(e) = session.request_display_refresh_rate(*hz) {
                        tracing::warn!("VR: refresh rate {hz} Hz refused: {e}");
                    }
                })
        } else {
            None
        };
        if self.closing.load(Ordering::SeqCst) {
            // Made in the background while the app quit: never installed
            // (it must not outlive the device).
            return Ok(refresh);
        }
        lock(&self.pacer).new_session();
        *lock(&self.stream) = Some(Stream {
            textures,
            swapchain,
            stream,
        });
        *lock(&self.waiter) = Some(waiter);
        *lock(&self.live) = Some(Live { space, session });
        self.generation.fetch_add(1, Ordering::SeqCst);
        Ok(refresh)
    }

    /// Destroys the session and what hangs on it, in order: no frame is in
    /// flight on the render thread (`frame`), the GPU has finished with the
    /// swapchain images, then the swapchain and the frame stream, the
    /// frame waiter, the space and the session. The device and the
    /// instance stay (a new session may follow).
    fn drop_session(&self) {
        let _frame = lock(&self.frame);
        self.running.store(false, Ordering::SeqCst);
        if lock(&self.stream).is_some()
            && let Err(e) = self.device.poll(wgpu::PollType::wait_indefinitely())
        {
            tracing::warn!("VR: waiting for the GPU before the teardown: {e}");
        }
        *lock(&self.stream) = None;
        *lock(&self.waiter) = None;
        *lock(&self.live) = None;
        // Nothing of the old session is outstanding: wake a waiting main
        // thread.
        lock(&self.pacer).new_session();
        self.begun.notify_all();
    }

    /// A new session on the same instance and device (see
    /// [`MainHandle::recreate_session`]).
    fn create_new(&self) -> Result<(), XrError> {
        let system = self
            .instance
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(xr_error)?;
        if system != self.system {
            // Another system would need another device: a restart.
            return Err(XrError::InstanceLost);
        }
        let (session, waiter, stream) =
            vulkan::create_session(&self.instance, self.system, &self.session_info)?;
        self.install(session, waiter, stream).map(|_| ())
    }
}

/// How long the main thread waits for the render thread to begin the last
/// frame before it skips this frame's wait ([`MainHandle::ready_to_wait`]).
/// Short on purpose (v4.64, measured on Weela): the render world sometimes
/// needs the main thread itself (Bevy's main-thread executor, ticked only
/// between the main world's systems), so a main thread blocked here for
/// long holds the render thread too – with 1 s the loop stalled for 1 s
/// every few seconds (32 fps). The normal wait is one render frame.
pub const BEGIN_TIMEOUT: Duration = Duration::from_millis(40);

impl Drop for Shared {
    fn drop(&mut self) {
        // Whatever drops the last handle: the session goes first.
        self.drop_session();
    }
}

/// The main thread's handle.
pub struct MainHandle {
    shared: Arc<Shared>,
    /// A session being created in the background, and since when.
    reconnect: Option<(JoinHandle<Result<(), XrError>>, Instant)>,
    /// The last waited frame's id ([`FramePacer`]), for the render world.
    frame_id: u64,
}

impl MainHandle {
    fn with_session<R>(&self, f: impl FnOnce(&Live) -> Result<R, XrError>) -> Result<R, XrError> {
        match lock(&self.shared.live).as_ref() {
            Some(l) => f(l),
            None => Err(XrError::SessionLost),
        }
    }

    /// The id of the frame the last [`FrameSource::wait_frame`] returned:
    /// the render world hands it back with [`RenderHandle::begin_frame_for`].
    pub fn frame_id(&self) -> u64 {
        self.frame_id
    }

    /// The session generation (changes with every new session).
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    /// Whether `xrWaitFrame` may run now: the last waited frame was begun
    /// (waits at most `timeout` for it). `false`: skip this frame's wait –
    /// the frame stays outstanding until the render world begins it, so
    /// the next wait still never comes before that begin.
    pub fn ready_to_wait(&self, timeout: Duration) -> bool {
        self.wait_for_begin(timeout)
    }

    /// Waits (at most `timeout`) until the last waited frame was begun.
    fn wait_for_begin(&self, timeout: Duration) -> bool {
        let pacer = lock(&self.shared.pacer);
        let (pacer, _) = self
            .shared
            .begun
            .wait_timeout_while(pacer, timeout, |p| !p.may_wait())
            .unwrap_or_else(|p| p.into_inner());
        pacer.may_wait()
    }
}

impl XrRuntime for MainHandle {
    fn poll_event(&mut self) -> Result<Option<RuntimeEvent>, XrError> {
        // A fresh buffer per poll: the buffer is not `Send`, the handle is.
        let mut events = xr::EventDataBuffer::new();
        loop {
            let Some(e) = self
                .shared
                .instance
                .poll_event(&mut events)
                .map_err(xr_error)?
            else {
                return Ok(None);
            };
            use xr::Event as E;
            use xr::SessionState as S;
            let ours = match e {
                // An event of a session already destroyed (queued before a
                // reconnect) must not drive the new one.
                E::SessionStateChanged(s)
                    if lock(&self.shared.live)
                        .as_ref()
                        .is_none_or(|l| l.session.as_raw() != s.session()) =>
                {
                    tracing::info!(
                        "VR: runtime state {:?} of an old session ignored",
                        s.state()
                    );
                    None
                }
                E::SessionStateChanged(s) => Some(RuntimeEvent::State(match s.state() {
                    S::IDLE => RuntimeState::Idle,
                    S::READY => RuntimeState::Ready,
                    S::SYNCHRONIZED => RuntimeState::Synchronized,
                    S::VISIBLE => RuntimeState::Visible,
                    S::FOCUSED => RuntimeState::Focused,
                    S::STOPPING => RuntimeState::Stopping,
                    S::LOSS_PENDING => RuntimeState::LossPending,
                    S::EXITING => RuntimeState::Exiting,
                    _ => continue,
                })),
                E::InstanceLossPending(_) => Some(RuntimeEvent::InstanceLossPending),
                E::ReferenceSpaceChangePending(_) => Some(RuntimeEvent::ReferenceSpaceChanged),
                _ => None,
            };
            if ours.is_some() {
                return Ok(ours);
            }
        }
    }

    fn begin_session(&mut self) -> Result<(), XrError> {
        self.with_session(|l| l.session.begin(VIEW_TYPE).map(|_| ()).map_err(xr_error))?;
        self.shared.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// Under the frame lock: never while the render thread is inside
    /// `xrBeginFrame` … `xrEndFrame` (the session is externally
    /// synchronised); frames after it are skipped (`running`).
    fn end_session(&mut self) -> Result<(), XrError> {
        let _frame = lock(&self.shared.frame);
        self.shared.running.store(false, Ordering::SeqCst);
        self.with_session(|l| l.session.end().map(|_| ()).map_err(xr_error))
    }

    fn request_exit(&mut self) -> Result<(), XrError> {
        self.with_session(|l| l.session.request_exit().map_err(xr_error))
    }

    fn shutdown(&mut self) {
        self.shared.closing.store(true, Ordering::SeqCst);
        if let Some((job, since)) = self.reconnect.take() {
            // A session made in the background must not outlive the device.
            tracing::info!(
                "VR: waiting for the session being created ({:.1} s so far)",
                since.elapsed().as_secs_f32()
            );
            let _ = job.join();
        }
        self.shared.drop_session();
        tracing::info!(
            "VR: session torn down (swapchain, frame stream, session) before the GPU device"
        );
    }

    /// In the background (v4.64): the old session is destroyed here, the
    /// new one is created on a thread – the Oculus runtime took ~25 s for
    /// `xrCreateSession` … `xrCreateSwapchain` on Weela, which froze the
    /// app on the monitor and in the headset. Until it is there this
    /// answers [`XrError::Reconnecting`].
    fn recreate_session(&mut self) -> Result<(), XrError> {
        if let Some((job, since)) = self.reconnect.take() {
            if !job.is_finished() {
                self.reconnect = Some((job, since));
                return Err(XrError::Reconnecting);
            }
            let r = job
                .join()
                .unwrap_or_else(|_| Err(XrError::Other("the reconnect thread panicked".into())));
            tracing::info!(
                "VR: new session {} after {:.1} s",
                if r.is_ok() { "created" } else { "failed" },
                since.elapsed().as_secs_f32()
            );
            return r;
        }
        self.shared.drop_session();
        let shared = self.shared.clone();
        let job = std::thread::Builder::new()
            .name("ocw-xr-reconnect".into())
            .spawn(move || shared.create_new())
            .map_err(|e| XrError::Other(format!("cannot start the reconnect thread: {e}")))?;
        tracing::info!("VR: old session destroyed, creating a new one in the background");
        self.reconnect = Some((job, Instant::now()));
        Err(XrError::Reconnecting)
    }
}

fn to_pose(p: &xr::Posef) -> Pose {
    Pose::new(
        Quat::from_xyzw(
            p.orientation.x,
            p.orientation.y,
            p.orientation.z,
            p.orientation.w,
        ),
        Vec3::new(p.position.x, p.position.y, p.position.z),
    )
}

fn from_pose(p: &Pose) -> xr::Posef {
    xr::Posef {
        orientation: xr::Quaternionf {
            x: p.orientation.x,
            y: p.orientation.y,
            z: p.orientation.z,
            w: p.orientation.w,
        },
        position: xr::Vector3f {
            x: p.position.x,
            y: p.position.y,
            z: p.position.z,
        },
    }
}

fn to_fov(f: &xr::Fovf) -> Fov {
    Fov {
        left: f.angle_left,
        right: f.angle_right,
        up: f.angle_up,
        down: f.angle_down,
    }
}

fn from_fov(f: &Fov) -> xr::Fovf {
    xr::Fovf {
        angle_left: f.left,
        angle_right: f.right,
        angle_up: f.up,
        angle_down: f.down,
    }
}

impl FrameSource for MainHandle {
    fn wait_frame(&mut self) -> Result<FrameTiming, XrError> {
        // Never wait again before the last waited frame was begun (the
        // Oculus runtime loses the session for that, see the crate docs).
        // The caller asked [`Self::ready_to_wait`] first; this guard only
        // keeps the rule.
        if !lock(&self.shared.pacer).may_wait() {
            return Err(XrError::Other(format!(
                "frame {} not begun yet: xrWaitFrame not called",
                self.frame_id
            )));
        }
        let mut w = lock(&self.shared.waiter);
        let w = w.as_mut().ok_or(XrError::SessionLost)?;
        let s = w.wait().map_err(xr_error)?;
        self.frame_id = lock(&self.shared.pacer).waited();
        Ok(FrameTiming {
            display_time_ns: s.predicted_display_time.as_nanos(),
            period_ns: s.predicted_display_period.as_nanos(),
            should_render: s.should_render,
        })
    }

    fn locate_views(&mut self, time_ns: i64) -> Result<Option<StereoViews>, XrError> {
        self.with_session(|l| {
            let (flags, views) = l
                .session
                .locate_views(VIEW_TYPE, xr::Time::from_nanos(time_ns), &l.space)
                .map_err(xr_error)?;
            let tracked = flags.contains(xr::ViewStateFlags::ORIENTATION_VALID)
                && flags.contains(xr::ViewStateFlags::POSITION_VALID);
            if !tracked || views.len() < 2 {
                return Ok(None);
            }
            let eye = |v: &xr::View| EyeView {
                pose: to_pose(&v.pose),
                fov: to_fov(&v.fov),
            };
            let out = [eye(&views[0]), eye(&views[1])];
            Ok(out
                .iter()
                .all(|e| e.pose.is_valid() && e.fov.is_valid())
                .then_some(out))
        })
    }

    fn now_ns(&mut self) -> Option<i64> {
        // `xrConvertTimespecTimeToTimeKHR` would need another extension;
        // the latency is measured on the render thread instead.
        None
    }
}

impl ControllerSource for MainHandle {
    fn sync_controllers(&mut self) -> Result<Option<ControllerState>, XrError> {
        let Some(c) = &self.shared.controls else {
            return Ok(None);
        };
        self.with_session(|l| c.sync(&l.session).map(Some))
    }
}

impl MainHandle {
    /// The interaction profile the runtime chose for each hand (`vr
    /// status`; empty strings before the first focus).
    pub fn controller_profiles(&self) -> [String; 2] {
        let Some(c) = &self.shared.controls else {
            return Default::default();
        };
        lock(&self.shared.live)
            .as_ref()
            .map(|l| c.profiles(&self.shared.instance, &l.session))
            .unwrap_or_default()
    }
}

/// The render thread's handle.
#[derive(Clone)]
pub struct RenderHandle {
    shared: Arc<Shared>,
}

impl RenderHandle {
    /// The swapchain images (wgpu textures, 2 layers each) and their
    /// generation (changes with a new session).
    pub fn images(&self) -> (u64, Vec<wgpu::Texture>) {
        let g = self.shared.generation.load(Ordering::SeqCst);
        let images = lock(&self.shared.stream)
            .as_ref()
            .map(|s| s.textures.clone())
            .unwrap_or_default();
        (g, images)
    }

    /// `xrBeginFrame` for the waited frame `id` of session `generation`,
    /// under the frame lock; tells the main thread it may wait for the
    /// next frame. `Ok(false)`: not begun (the session ended or is another
    /// one now) – nothing to finish.
    pub fn begin_frame_for(&mut self, id: u64, generation: u64) -> Result<bool, XrError> {
        let shared = self.shared.clone();
        let _frame = lock(&shared.frame);
        let current = self.running() && shared.generation.load(Ordering::SeqCst) == generation;
        let r = if current {
            self.begin_frame().map(|()| true)
        } else {
            Ok(false)
        };
        if !(shared.pace_after_end && matches!(r, Ok(true))) {
            lock(&shared.pacer).handled(id);
            shared.begun.notify_all();
        }
        r
    }

    /// The begun frame `id` is finished (ended, or given up): with
    /// `OCW_VR_PACE=end` the main thread may wait for the next one now.
    pub fn frame_ended(&self, id: u64) {
        lock(&self.shared.pacer).handled(id);
        self.shared.begun.notify_all();
    }

    /// Whether the main thread waits for the last frame's end (not only
    /// its begin) – `OCW_VR_PACE=end`.
    pub fn paces_after_end(&self) -> bool {
        self.shared.pace_after_end
    }

    /// The session generation (changes with every new session).
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::SeqCst)
    }

    /// Held for a whole frame submission: the teardown waits for it.
    pub fn frame_guard(&self) -> MutexGuard<'_, ()> {
        lock(&self.shared.frame)
    }

    /// The session runs (begun, not ended): frames may be submitted. Read
    /// it under [`Self::frame_guard`].
    pub fn running(&self) -> bool {
        self.shared.running.load(Ordering::SeqCst)
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.shared.wgpu_format
    }

    pub fn extent(&self) -> (u32, u32) {
        self.shared.extent
    }

    fn with_stream<R>(
        &self,
        f: impl FnOnce(&mut Stream) -> Result<R, XrError>,
    ) -> Result<R, XrError> {
        match lock(&self.shared.stream).as_mut() {
            Some(s) => f(s),
            // Ours, not the runtime's: the session was torn down meanwhile.
            None => Err(XrError::Other(
                "no swapchain (the session was torn down)".into(),
            )),
        }
    }
}

impl FrameSink for RenderHandle {
    fn begin_frame(&mut self) -> Result<(), XrError> {
        self.with_stream(|s| s.stream.begin().map_err(xr_error))
    }

    fn acquire_image(&mut self) -> Result<u32, XrError> {
        self.with_stream(|s| s.swapchain.acquire_image().map_err(xr_error))
    }

    fn wait_image(&mut self) -> Result<(), XrError> {
        self.with_stream(|s| {
            s.swapchain
                .wait_image(xr::Duration::INFINITE)
                .map_err(xr_error)
        })
    }

    fn release_image(&mut self) -> Result<(), XrError> {
        self.with_stream(|s| s.swapchain.release_image().map_err(xr_error))
    }

    fn end_frame(
        &mut self,
        display_time_ns: i64,
        layer: Option<&StereoViews>,
    ) -> Result<(), XrError> {
        let extent = self.shared.extent;
        let live = lock(&self.shared.live);
        let live = live
            .as_ref()
            .ok_or_else(|| XrError::Other("no session (it was torn down)".into()))?;
        self.with_stream(|s| {
            let time = xr::Time::from_nanos(display_time_ns);
            let blend = xr::EnvironmentBlendMode::OPAQUE;
            let Some(views) = layer else {
                return s.stream.end(time, blend, &[]).map_err(xr_error);
            };
            let rect = xr::Rect2Di {
                offset: xr::Offset2Di { x: 0, y: 0 },
                extent: xr::Extent2Di {
                    width: extent.0 as i32,
                    height: extent.1 as i32,
                },
            };
            let eye = |i: usize| {
                xr::CompositionLayerProjectionView::new()
                    .pose(from_pose(&views[i].pose))
                    .fov(from_fov(&views[i].fov))
                    .sub_image(
                        xr::SwapchainSubImage::new()
                            .swapchain(&s.swapchain)
                            .image_array_index(i as u32)
                            .image_rect(rect),
                    )
            };
            let eyes = [eye(0), eye(1)];
            let projection = xr::CompositionLayerProjection::new()
                .space(&live.space)
                .views(&eyes);
            s.stream.end(time, blend, &[&projection]).map_err(xr_error)
        })
    }
}

#[cfg(test)]
mod tests {
    /// Regression (2026-11-16, marge): with only the runtime package
    /// installed there is no unversioned `libopenxr_loader.so`, so the
    /// versioned name must come first on Linux.
    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn the_versioned_loader_comes_first() {
        assert_eq!(
            loader_names(),
            &["libopenxr_loader.so.1", "libopenxr_loader.so"]
        );
    }

    use super::*;

    #[test]
    fn openxr_results_map_to_our_errors() {
        use xr::sys::Result as R;
        assert_eq!(xr_error(R::ERROR_SESSION_LOST), XrError::SessionLost);
        assert_eq!(xr_error(R::ERROR_INSTANCE_LOST), XrError::InstanceLost);
        assert_eq!(
            xr_error(R::ERROR_FORM_FACTOR_UNAVAILABLE),
            XrError::NoHeadset
        );
        assert!(matches!(
            xr_error(R::ERROR_RUNTIME_UNAVAILABLE),
            XrError::NoRuntime(_)
        ));
        assert!(matches!(
            xr_error(R::ERROR_VALIDATION_FAILURE),
            XrError::Other(_)
        ));
    }

    #[test]
    fn poses_and_fovs_convert_both_ways() {
        let p = Pose::new(Quat::from_rotation_y(0.3), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(to_pose(&from_pose(&p)), p);
        let f = ocw_xr::view::QUEST1_LEFT_FOV;
        assert_eq!(to_fov(&from_fov(&f)), f);
    }

    /// Without a loader (the build machines have no OpenXR runtime) boot
    /// fails with a message and nothing else happens; with one (CI with
    /// Monado) it gets further – either is fine here, a panic is not.
    #[test]
    fn boot_without_a_runtime_is_an_error_not_a_crash() {
        match boot(&BootOptions::default()) {
            Ok(b) => assert!(b.info.eye_px.0 >= 16),
            Err(e) => assert!(!e.to_string().is_empty()),
        }
    }
}

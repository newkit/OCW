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

pub mod vulkan;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ocw_xr::frame::{FrameSink, FrameSource, FrameTiming};
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
    instance: xr::Instance,
    system: xr::SystemId,
    session_info: xr::vulkan::SessionCreateInfo,
    device: wgpu::Device,
    format: ColorFormat,
    wgpu_format: wgpu::TextureFormat,
    extent: (u32, u32),
    // Drop order (fields drop top to bottom): the stream and the waiter
    // before the session.
    stream: Mutex<Option<Stream>>,
    waiter: Mutex<Option<xr::FrameWaiter>>,
    live: Mutex<Option<Live>>,
    /// Bumped by every new session (new swapchain images).
    generation: AtomicU64,
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

/// Boots OpenXR (see the crate docs).
pub fn boot(opts: &BootOptions) -> Result<Booted, XrError> {
    // SAFETY-free: `Entry::load` opens the loader library by its standard
    // name (libopenxr_loader.so.1, openxr_loader.dll).
    #[allow(unsafe_code)]
    // SAFETY: loading a library that follows the OpenXR loader ABI.
    let entry = unsafe { xr::Entry::load() }.map_err(|e| XrError::NoLoader(e.to_string()))?;
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
    let extent = eye_extent(recommended, max, opts.render_scale);
    let dev = vulkan::create_device(&instance, system, &opts.app_name)?;
    let (session, waiter, stream) = vulkan::create_session(&instance, system, &dev.session_info)?;
    let formats = session.enumerate_swapchain_formats().map_err(xr_error)?;
    let offered: Vec<i64> = formats.iter().map(|f| i64::from(*f)).collect();
    let format = choose_format(&offered, FormatSpace::Vulkan).ok_or_else(|| {
        XrError::Unsupported("the runtime offers no sRGB or float colour format".into())
    })?;
    let wgpu_format = vulkan::wgpu_format(format);
    let shared = Arc::new(Shared {
        instance,
        system,
        session_info: dev.session_info,
        device: dev.device.clone(),
        format,
        wgpu_format,
        extent,
        stream: Mutex::new(None),
        waiter: Mutex::new(None),
        live: Mutex::new(None),
        generation: AtomicU64::new(0),
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
        },
        main: MainHandle {
            shared: shared.clone(),
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

    fn drop_session(&self) {
        *lock(&self.stream) = None;
        *lock(&self.waiter) = None;
        *lock(&self.live) = None;
    }
}

/// The main thread's handle.
pub struct MainHandle {
    shared: Arc<Shared>,
}

impl MainHandle {
    fn with_session<R>(&self, f: impl FnOnce(&Live) -> Result<R, XrError>) -> Result<R, XrError> {
        match lock(&self.shared.live).as_ref() {
            Some(l) => f(l),
            None => Err(XrError::SessionLost),
        }
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
        self.with_session(|l| l.session.begin(VIEW_TYPE).map(|_| ()).map_err(xr_error))
    }

    fn end_session(&mut self) -> Result<(), XrError> {
        self.with_session(|l| l.session.end().map(|_| ()).map_err(xr_error))
    }

    fn request_exit(&mut self) -> Result<(), XrError> {
        self.with_session(|l| l.session.request_exit().map_err(xr_error))
    }

    fn recreate_session(&mut self) -> Result<(), XrError> {
        let s = &self.shared;
        s.drop_session();
        // Is the headset back?
        let system = s
            .instance
            .system(xr::FormFactor::HEAD_MOUNTED_DISPLAY)
            .map_err(xr_error)?;
        if system != s.system {
            // Another system would need another device: a restart.
            return Err(XrError::InstanceLost);
        }
        let (session, waiter, stream) =
            vulkan::create_session(&s.instance, s.system, &s.session_info)?;
        s.install(session, waiter, stream).map(|_| ())
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
        let mut w = lock(&self.shared.waiter);
        let w = w.as_mut().ok_or(XrError::SessionLost)?;
        let s = w.wait().map_err(xr_error)?;
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
            None => Err(XrError::SessionLost),
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
        let live = live.as_ref().ok_or(XrError::SessionLost)?;
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

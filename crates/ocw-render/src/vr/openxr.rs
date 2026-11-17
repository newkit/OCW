//! The headset backend over our own thin OpenXR layer (`ocw-xr-openxr`,
//! spec §14 Phase 11, X1 variant B).
//!
//! - [`boot`] (before the app is built): OpenXR makes the Vulkan device;
//!   Bevy renders with it (`RenderCreation::Manual`).
//! - Main world, [`First`]: the session's events and state machine,
//!   `xrWaitFrame` (it paces the app to the headset: 72 Hz on the Quest 1),
//!   `xrLocateViews` for when the frame will be shown → [`VrState::views`];
//!   [`super::place_vr`] puts the eye cameras there.
//! - Render world, after the render graph: `xrBeginFrame`, the eye images
//!   copied into the swapchain's two layers, `xrEndFrame` with the
//!   projection layer (`frame::finish`). (Begun before the graph the Link
//!   runtime's compositor timed out on the GPU, v4.71: see
//!   [`begin_waited_frame`]; `OCW_VR_BEGIN=early` for other runtimes.)
//!   The main world waits for a frame only after the last one was begun
//!   (`FramePacer`, v4.71: the Oculus runtime lost the session otherwise).
//! - The loop's health (`ocw_xr::watch`): every call timed, failures with
//!   the call's name at once, a report line every 10 s (`vr status`).
//! - The controllers: `xrSyncActions` after the wait, every frame of the
//!   loop → [`VrState::controllers`] (the gamepad of `super::controllers`).
//! - The app's exit ([`AppExit`], in [`Last`]): the session is torn down
//!   before Bevy drops the GPU device (`XrRuntime::shutdown`, v4.68).

use std::sync::{Arc, Mutex};

use bevy::platform::time::Instant;
use bevy::prelude::*;
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::renderer::{
    RenderAdapter, RenderAdapterInfo, RenderDevice, RenderInstance, RenderQueue,
};
use bevy::render::settings::RenderCreation;
use bevy::render::texture::GpuImage;
use bevy::render::{Render, RenderApp, RenderSystems};
use ocw_xr::controllers::ControllerSource;
use ocw_xr::frame::{FrameCall, FrameFailure, FrameSource, FrameTiming, finish, pose_time};
use ocw_xr::session::{SessionMachine, XrError};
use ocw_xr::view::StereoViews;
use ocw_xr::watch::LoopWatch;
pub use ocw_xr_openxr::{BootOptions, XrInfo};
use ocw_xr_openxr::{MainHandle, RenderHandle};

use super::{EyeImages, VrState};

/// A booted headset: the renderer's device and the plugin.
pub struct Boot {
    pub creation: RenderCreation,
    pub plugin: OpenXrPlugin,
    pub info: XrInfo,
}

/// Boots OpenXR (no window, no Bevy yet): see `ocw_xr_openxr::boot`.
pub fn boot(opts: &BootOptions) -> Result<Boot, XrError> {
    let b = ocw_xr_openxr::boot(opts)?;
    let info_line = b.gpu.adapter.get_info();
    let creation = RenderCreation::manual(
        RenderDevice::from(b.gpu.device),
        RenderQueue::new(b.gpu.queue),
        RenderAdapterInfo::new(info_line),
        RenderAdapter::new(b.gpu.adapter),
        RenderInstance::new(b.gpu.instance),
    );
    Ok(Boot {
        creation,
        plugin: OpenXrPlugin {
            main: Mutex::new(Some(b.main)),
            render: b.render,
            info: b.info.clone(),
        },
        info: b.info,
    })
}

/// The runtime one line (`vr status`).
pub fn describe(info: &XrInfo) -> String {
    format!(
        "{} on {} ({}), {}{}, eyes {}×{} px, GPU {}",
        info.runtime,
        info.system,
        info.profile.name,
        info.format.name(),
        info.refresh_hz
            .map_or(String::new(), |hz| format!(", {hz:.0} Hz")),
        info.eye_px.0,
        info.eye_px.1,
        if info.gpu.is_empty() { "?" } else { &info.gpu }
    )
}

pub struct OpenXrPlugin {
    main: Mutex<Option<MainHandle>>,
    render: RenderHandle,
    info: XrInfo,
}

/// The main world's handle.
#[derive(Resource)]
struct XrMain(Mutex<MainHandle>);

/// Errors the render world met (the session machine hears of them).
#[derive(Resource, Clone, Default)]
struct XrErrors(Arc<Mutex<Vec<FrameFailure>>>);

/// The loop's health, fed by both worlds, and the clock it is timed by.
#[derive(Resource, Clone)]
struct XrWatch {
    watch: Arc<Mutex<LoopWatch>>,
    origin: Instant,
}

impl XrWatch {
    fn new() -> Self {
        Self {
            watch: Arc::new(Mutex::new(LoopWatch::default())),
            origin: Instant::now(),
        }
    }

    /// Milliseconds since the start.
    fn ms(&self) -> f64 {
        self.origin.elapsed().as_secs_f64() * 1000.0
    }

    fn secs(&self) -> f64 {
        self.origin.elapsed().as_secs_f64()
    }
}

/// Times a main-thread call into the watch.
fn timed<T>(
    w: &XrWatch,
    call: FrameCall,
    f: impl FnOnce() -> Result<T, XrError>,
) -> Result<T, FrameFailure> {
    let t0 = w.ms();
    let r = f();
    let ms = (w.ms() - t0) as f32;
    lock(&w.watch).call(call, ms);
    r.map_err(|error| FrameFailure { call, error, ms })
}

/// The render world's submission counts, read by the main world.
#[derive(Resource, Clone, Default)]
struct XrCounts(Arc<Mutex<ocw_xr::frame::SubmitCounts>>);

/// What the render world submits for this frame.
#[derive(Resource, Clone, Default)]
struct XrFrame {
    timing: Option<FrameTiming>,
    /// The waited frame's id and its session's generation (the render
    /// world begins exactly this frame, on exactly that session).
    id: u64,
    generation: u64,
    views: Option<StereoViews>,
    eyes: Option<[AssetId<Image>; 2]>,
}

impl ExtractResource<RenderApp> for XrFrame {
    type Source = XrFrame;

    fn extract_resource(source: &Self) -> Self {
        source.clone()
    }
}

/// The render world's handle and the swapchain images it copies into.
#[derive(Resource)]
struct XrRender {
    handle: RenderHandle,
    generation: u64,
    images: Vec<wgpu::Texture>,
    errors: XrErrors,
    counts: XrCounts,
    watch: XrWatch,
    /// The frame begun this render frame: its session, when, its id.
    begun: Option<(u64, f64, u64)>,
    /// `OCW_VR_BEGIN=early`: `xrBeginFrame` before the render graph.
    begin_early: bool,
    /// When this render frame started (ms on the watch's clock).
    render_start_ms: f64,
}

impl Plugin for OpenXrPlugin {
    fn build(&self, app: &mut App) {
        let Some(main) = self.main.lock().ok().and_then(|mut m| m.take()) else {
            return;
        };
        let errors = XrErrors::default();
        let counts = XrCounts::default();
        let watch = XrWatch::new();
        app.insert_resource(XrMain(Mutex::new(main)))
            .insert_resource(watch.clone())
            .insert_resource(errors.clone())
            .insert_resource(counts.clone())
            .insert_resource(VrState {
                session: Some(SessionMachine::new()),
                device: describe(&self.info),
                ..default()
            })
            .init_resource::<XrFrame>()
            .add_plugins(ExtractResourcePlugin::<XrFrame>::default())
            .add_systems(First, frame_start)
            .add_systems(Last, shutdown_on_exit);
        let handle = self.render.clone();
        if handle.paces_after_end() {
            info!("VR: OCW_VR_PACE=end – each frame is waited for only after the last one ended");
        }
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(XrRender {
                    handle,
                    generation: u64::MAX,
                    images: Vec::new(),
                    errors,
                    counts,
                    watch,
                    begun: None,
                    begin_early: std::env::var("OCW_VR_BEGIN").is_ok_and(|v| v == "early"),
                    render_start_ms: 0.0,
                })
                .add_systems(
                    Render,
                    begin_xr_frame
                        .after(RenderSystems::ExtractCommands)
                        .before(RenderSystems::PrepareViews)
                        .before(RenderSystems::PrepareAssets),
                )
                .add_systems(
                    Render,
                    submit_frame
                        .after(RenderSystems::Render)
                        .before(RenderSystems::Cleanup),
                );
        }
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Events, the session, `xrWaitFrame`, `xrLocateViews`.
#[allow(clippy::too_many_arguments)] // a Bevy system
fn frame_start(
    main: Res<XrMain>,
    errors: Res<XrErrors>,
    counts: Res<XrCounts>,
    watch: Res<XrWatch>,
    eyes: Option<Res<EyeImages>>,
    mut state: ResMut<VrState>,
    mut frame: ResMut<XrFrame>,
    mut loop_state: Local<(Option<Instant>, u64)>,
    mut exit: MessageWriter<AppExit>,
    mut limit: Option<ResMut<crate::frame_rate::FrameLimit>>,
    mut idle_cap: Local<Option<Option<f64>>>,
) {
    let (last, polls) = &mut *loop_state;
    let mut rt = lock(&main.0);
    let now = Instant::now();
    let dt = last.map_or(0.0, |l| now.duration_since(l).as_secs_f32());
    state.submitted = *lock(&counts.0);
    let mut machine = state.session.take().unwrap_or_default();
    // Before the render world's errors: a loss they cause is logged too.
    let before = machine.phase().clone();
    for f in lock(&errors.0).drain(..) {
        machine.frame_error(f.error);
    }
    let t0 = watch.ms();
    machine.update(&mut *rt, dt);
    lock(&watch.watch).call(FrameCall::PollEvent, (watch.ms() - t0) as f32);
    for line in machine.take_log() {
        info!("VR {line}");
    }
    if *machine.phase() != before {
        info!("VR session: {}", machine.summary());
    }
    if std::mem::take(&mut machine.quit_wanted) {
        // EXITING not asked for: the runtime wants the app closed (Quit in
        // the headset's menu). Staying made Link end the process.
        info!("VR: the runtime closed the app (Quit in the headset's menu): quitting");
        exit.write(AppExit::Success);
    }
    if std::mem::take(&mut machine.recentre_wanted) {
        state.recentre_now = true;
    }
    let mut next = XrFrame {
        eyes: eyes.map(|e| [e.eyes[0].id(), e.eyes[1].id()]),
        ..default()
    };
    let main_failed = |f: FrameFailure, machine: &mut ocw_xr::session::SessionMachine| {
        lock(&watch.watch).failed(&f);
        machine.frame_error(f.error);
    };
    // The last waited frame not begun yet (the render world is late, or
    // waits for this thread): no wait this frame, never one too early.
    let ready = !machine.frame_loop() || rt.ready_to_wait(ocw_xr_openxr::BEGIN_TIMEOUT);
    if !ready {
        lock(&watch.watch).skipped();
        state.should_render = false;
    } else if machine.frame_loop() {
        // The app's own work since the last wait (the blocking wait is
        // not the app's time).
        let work_ms = last.map(|l| now.duration_since(l).as_secs_f32() * 1000.0);
        match timed(&watch, FrameCall::WaitFrame, || rt.wait_frame()) {
            Ok(t) => {
                // For this frame's own display time: the render world
                // submits this frame with this wait's display time
                // (`XrFrame`), so the eyes are drawn where the head will
                // be then. Until v4.71 one period later (+13.9 ms): the
                // compositor had to reproject every frame from a pose the
                // head had not reached yet.
                let at = pose_time(&t, false);
                let sampled = rt.now_ns();
                match timed(&watch, FrameCall::LocateViews, || rt.locate_views(at)) {
                    Ok(Some(v)) => state.views = Some(v),
                    Ok(None) => {}
                    Err(f) => main_failed(f, &mut machine),
                }
                let latency = sampled.map(|s| (at - s) as f32 / 1e6);
                if let Some(w) = work_ms {
                    state.stats.record(w, &t, latency);
                }
                lock(&watch.watch).waited(&t, work_ms);
                state.should_render = t.should_render;
                next.timing = Some(t);
                next.id = rt.frame_id();
                next.generation = rt.generation();
                next.views = state.views;
                match timed(&watch, FrameCall::SyncActions, || rt.sync_controllers()) {
                    Ok(c) => state.controllers = c,
                    Err(f) => main_failed(f, &mut machine),
                }
                *polls += 1;
                if *polls % 72 == 1 {
                    let [l, r] = rt.controller_profiles();
                    state.controller_profiles = [l, r];
                }
            }
            Err(f) => {
                main_failed(f, &mut machine);
                state.should_render = false;
            }
        }
        *last = Some(Instant::now());
    } else {
        state.should_render = false;
        // No frame loop (headset off, runtime menu): the pad rests.
        state.controllers = None;
        *last = Some(now);
    }
    {
        let mut w = lock(&watch.watch);
        let secs = watch.secs();
        w.check(secs, machine.frame_loop(), machine.visible());
        for line in w.take_lines(secs) {
            info!("{line}");
        }
        state.loop_line = w.last_report().to_string();
    }
    // The frame cap (the app sets it to the headset's rate): off while
    // `xrWaitFrame` paces the loop (a second clock could make a frame miss
    // its slot), on otherwise – headset off, a skipped wait – so the app
    // never draws more frames than the headset shows (v4.71: uncapped it
    // ran the laptop's GPU flat out).
    if let Some(limit) = limit.as_mut() {
        let cap = *idle_cap.get_or_insert(limit.max_fps);
        let want = if machine.frame_loop() && ready {
            None
        } else {
            cap
        };
        if limit.max_fps != want {
            limit.max_fps = want;
        }
    }
    state.session = Some(machine);
    // Written every frame: the render world submits exactly this frame.
    *frame = next;
}

/// The app quits: the session goes now, while the GPU device lives (Bevy
/// drops the render world, and with it the device, after the last frame;
/// destroyed after the device, the session crashed the runtime: 0xC0000005
/// on Windows, 2026-11-17).
fn shutdown_on_exit(
    mut exits: MessageReader<AppExit>,
    main: Res<XrMain>,
    mut state: ResMut<VrState>,
) {
    if exits.read().last().is_none() {
        return;
    }
    let mut rt = lock(&main.0);
    if let Some(machine) = state.session.as_mut() {
        machine.shutdown(&mut *rt);
        info!("VR: session ended for the exit");
    }
}

/// Tells the pacer a begun frame is finished when dropped.
struct FrameDone(RenderHandle, u64);

impl Drop for FrameDone {
    fn drop(&mut self) {
        self.0.frame_ended(self.1);
    }
}

/// The start of the render world's frame: noted (the render graph's time),
/// and with `OCW_VR_BEGIN=early` the frame is begun here, before the graph.
fn begin_xr_frame(mut xr: ResMut<XrRender>, frame: Res<XrFrame>) {
    xr.render_start_ms = xr.watch.ms();
    xr.begun = None;
    if xr.begin_early {
        begin_waited_frame(&mut xr, &frame);
    }
}

/// `xrBeginFrame` for the frame the main world waited for, on the session
/// it waited on; lets the main world wait for the next frame.
///
/// **Where** (v4.71, measured on Weela, Quest 1 over Link, AMD): by
/// default right before the copy, after the render graph. Begun before the
/// graph (`OCW_VR_BEGIN=early`, as `bevy_mod_openxr` does) the Link
/// runtime's compositor timed out waiting on the GPU (`LiquidVRComputeContext::
/// CPUWaitOnCompute: LiquidVR wait timed out`, 136 times in 2 minutes)
/// and `xrWaitFrame` blocked ~1.9 s: 0.5 – 2 frames per second.
fn begin_waited_frame(xr: &mut XrRender, frame: &XrFrame) {
    if frame.timing.is_none() {
        return;
    }
    let watch = xr.watch.clone();
    let t0 = watch.ms();
    let r = xr.handle.begin_frame_for(frame.id, frame.generation);
    let ms = (watch.ms() - t0) as f32;
    lock(&watch.watch).call(FrameCall::BeginFrame, ms);
    match r {
        Ok(true) => xr.begun = Some((frame.generation, watch.ms(), frame.id)),
        Ok(false) => {}
        Err(error) => {
            let f = FrameFailure {
                call: FrameCall::BeginFrame,
                error,
                ms,
            };
            lock(&xr.counts.0).count::<()>(&Err(()));
            lock(&watch.watch).submitted(&Err(f.clone()), watch.secs());
            lock(&xr.errors.0).push(f);
        }
    }
}

/// The begun frame's end: the copy into the swapchain, `xrEndFrame`.
fn submit_frame(
    mut xr: ResMut<XrRender>,
    frame: Res<XrFrame>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    if !xr.begin_early {
        begin_waited_frame(&mut xr, &frame);
    }
    let Some((generation, _begun_ms, id)) = xr.begun.take() else {
        return;
    };
    // However this ends: the frame is done (`OCW_VR_PACE=end` lets the
    // main world wait for the next one only now).
    let _done = FrameDone(xr.handle.clone(), id);
    let Some(timing) = frame.timing else {
        return;
    };
    let watch = xr.watch.clone();
    lock(&watch.watch).rendered((watch.ms() - xr.render_start_ms) as f32);
    // The whole submission under the frame lock: the teardown waits.
    let held = xr.handle.clone();
    let _frame = held.frame_guard();
    // The session ended or was replaced since the begin: that frame is
    // gone with it.
    if !held.running() || held.generation() != generation {
        return;
    }
    let (generation, images) = xr.handle.images();
    if generation != xr.generation {
        xr.generation = generation;
        xr.images = images;
    }
    let src = frame
        .eyes
        .map(|ids| ids.map(|id| gpu_images.get(id).map(|g| g.texture.clone())));
    let images = xr.images.clone();
    let (w, h) = xr.handle.extent();
    let mut handle = xr.handle.clone();
    let gpu_watch = watch.clone();
    let mut copy = |i: u32| -> Result<(), XrError> {
        let dst = images
            .get(i as usize)
            .ok_or_else(|| XrError::Other(format!("no swapchain image {i}")))?;
        let Some([Some(l), Some(r)]) = &src else {
            return Err(XrError::Other("the eye images are not ready".into()));
        };
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("vr swapchain copy"),
        });
        for (layer, s) in [l, r].into_iter().enumerate() {
            if s.width() != w || s.height() != h {
                return Err(XrError::Other(
                    "eye image and swapchain sizes differ".into(),
                ));
            }
            enc.copy_texture_to_texture(
                s.as_image_copy(),
                wgpu::TexelCopyTextureInfo {
                    texture: dst,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: 0,
                        z: layer as u32,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
            );
        }
        // OpenXR wants the images back as colour attachments: an empty
        // pass over each layer leaves them in that layout.
        for layer in 0..2 {
            let view = dst.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2),
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            });
            let _pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("vr swapchain layout"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }
        queue.submit([enc.finish()]);
        // When the GPU finished this frame's work (all of it: the queue is
        // in order) – at most this late, wgpu reports it at its next poll.
        let w = gpu_watch.clone();
        let submitted = w.ms();
        queue.on_submitted_work_done(move || {
            let ms = (w.ms() - submitted) as f32;
            lock(&w.watch).gpu_done(ms);
        });
        Ok(())
    };
    let mut clock = || watch.ms();
    let mut report = |c: FrameCall, ms: f32| lock(&watch.watch).call(c, ms);
    let result = finish(
        &mut handle,
        &timing,
        frame.views.as_ref(),
        &mut copy,
        &mut clock,
        &mut report,
    );
    lock(&xr.counts.0).count(&result);
    lock(&watch.watch).submitted(&result, watch.secs());
    if let Err(f) = result {
        lock(&xr.errors.0).push(f);
    }
}

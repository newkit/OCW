//! The headset backend over our own thin OpenXR layer (`ocw-xr-openxr`,
//! spec §14 Phase 11, X1 variant B).
//!
//! - [`boot`] (before the app is built): OpenXR makes the Vulkan device;
//!   Bevy renders with it (`RenderCreation::Manual`).
//! - Main world, [`First`]: the session's events and state machine,
//!   `xrWaitFrame` (it paces the app to the headset: 72 Hz on the Quest 1),
//!   `xrLocateViews` for when the frame will be shown → [`VrState::views`];
//!   [`super::place_vr`] puts the eye cameras there.
//! - Render world, after the render graph ran: `xrBeginFrame`, the eye
//!   images copied into the swapchain's two layers, `xrEndFrame` with the
//!   projection layer (`ocw_xr::frame::submit`).
//! - The controllers: `xrSyncActions` after the wait, every frame of the
//!   loop → [`VrState::controllers`] (the gamepad of `super::controllers`).
//! - The app's exit ([`AppExit`], in [`Last`]): the session is torn down
//!   before Bevy drops the GPU device (`XrRuntime::shutdown`, v4.38).

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
use ocw_xr::frame::{FrameSource, FrameTiming, pose_time, submit};
use ocw_xr::session::{SessionMachine, XrError};
use ocw_xr::view::StereoViews;
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
        "{} on {} ({}), {}{}",
        info.runtime,
        info.system,
        info.profile.name,
        info.format.name(),
        info.refresh_hz
            .map_or(String::new(), |hz| format!(", {hz:.0} Hz"))
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
struct XrErrors(Arc<Mutex<Vec<XrError>>>);

/// What the render world submits for this frame.
#[derive(Resource, Clone, Default)]
struct XrFrame {
    timing: Option<FrameTiming>,
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
}

impl Plugin for OpenXrPlugin {
    fn build(&self, app: &mut App) {
        let Some(main) = self.main.lock().ok().and_then(|mut m| m.take()) else {
            return;
        };
        let errors = XrErrors::default();
        app.insert_resource(XrMain(Mutex::new(main)))
            .insert_resource(errors.clone())
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
        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .insert_resource(XrRender {
                    handle,
                    generation: u64::MAX,
                    images: Vec::new(),
                    errors,
                })
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
fn frame_start(
    main: Res<XrMain>,
    errors: Res<XrErrors>,
    eyes: Option<Res<EyeImages>>,
    mut state: ResMut<VrState>,
    mut frame: ResMut<XrFrame>,
    mut last: Local<Option<Instant>>,
    mut polls: Local<u64>,
) {
    let mut rt = lock(&main.0);
    let now = Instant::now();
    let dt = last.map_or(0.0, |l| now.duration_since(l).as_secs_f32());
    let mut machine = state.session.take().unwrap_or_default();
    for e in lock(&errors.0).drain(..) {
        machine.frame_error(e);
    }
    let before = machine.phase().clone();
    machine.update(&mut *rt, dt);
    if *machine.phase() != before {
        info!("VR session: {}", machine.summary());
    }
    if std::mem::take(&mut machine.recentre_wanted) {
        state.recentre_now = true;
    }
    let mut next = XrFrame {
        eyes: eyes.map(|e| [e.eyes[0].id(), e.eyes[1].id()]),
        ..default()
    };
    if machine.frame_loop() {
        // The app's own work since the last wait (the blocking wait is
        // not the app's time).
        let work_ms = last.map(|l| now.duration_since(l).as_secs_f32() * 1000.0);
        match rt.wait_frame() {
            Ok(t) => {
                let at = pose_time(&t, true);
                let sampled = rt.now_ns();
                match rt.locate_views(at) {
                    Ok(Some(v)) => state.views = Some(v),
                    Ok(None) => {}
                    Err(e) => machine.frame_error(e),
                }
                let latency = sampled.map(|s| (at - s) as f32 / 1e6);
                if let Some(w) = work_ms {
                    state.stats.record(w, &t, latency);
                }
                state.should_render = t.should_render;
                next.timing = Some(t);
                next.views = state.views;
                match rt.sync_controllers() {
                    Ok(c) => state.controllers = c,
                    Err(e) => machine.frame_error(e),
                }
                *polls += 1;
                if *polls % 72 == 1 {
                    let [l, r] = rt.controller_profiles();
                    state.controller_profiles = [l, r];
                }
            }
            Err(e) => {
                machine.frame_error(e);
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

/// `xrBeginFrame`, the copy into the swapchain, `xrEndFrame`.
fn submit_frame(
    mut xr: ResMut<XrRender>,
    frame: Res<XrFrame>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
) {
    let Some(timing) = frame.timing else {
        return;
    };
    // The whole submission under the frame lock: the teardown waits.
    let held = xr.handle.clone();
    let _frame = held.frame_guard();
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
        Ok(())
    };
    if let Err(e) = submit(&mut handle, &timing, frame.views.as_ref(), &mut copy) {
        lock(&xr.errors.0).push(e);
    }
}

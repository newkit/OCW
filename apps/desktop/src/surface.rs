//! A graphics adapter that cannot present to the window: restart on one that
//! can, instead of Bevy's panic.
//!
//! Bevy picks the adapter before the window exists (no compatible surface),
//! so wgpu takes the "best" one, e.g. a discrete GPU. On an X server without
//! DRI3 (Xvfb, some VNC/remote X servers) a hardware Vulkan driver (RADV)
//! cannot present at all ("vulkan: No DRI3 support detected"): its surface
//! reports no formats and no present modes, and Bevy's present-mode fallback
//! panics with "Fallback system failed to choose present mode … Options: []"
//! (2026-11-13 on marge, once the build user could open the GPU's render
//! node).
//!
//! [`SurfaceGuardPlugin`] probes each new window in the main world, before
//! the renderer extracts it (with its own wgpu instance: the probe needs no
//! `unsafe`). If the chosen adapter cannot present to it, the window is not
//! handed to the renderer, the app logs an error, exits normally and `main`
//! starts it again with `WGPU_ADAPTER_NAME` set to an adapter that can
//! (Mesa's lavapipe under Xvfb). If there is none, or this run already is the
//! restart, it exits with an error instead of panicking. The probe runs on
//! Linux only (X11/Wayland; other platforms present on every adapter).

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::sync::Mutex;

use bevy::prelude::*;

/// Set in the environment of the restarted app: no second restart.
pub const RETRY_ENV: &str = "OCW_SURFACE_FALLBACK";

/// One adapter and whether it can present to the window's surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub name: String,
    /// A software rasteriser (lavapipe/llvmpipe, WARP, SwiftShader).
    pub cpu: bool,
    pub presents: bool,
}

/// What to do about a window surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// The adapter can present to it: Bevy goes on as usual.
    Supported,
    /// Start again on this adapter (`WGPU_ADAPTER_NAME`).
    Restart { adapter: String },
    /// No adapter can present (or the restart did not help): exit.
    Fail { reason: String },
}

/// Whether a surface with these capabilities can be configured at all
/// (Bevy panics on an empty present-mode or format list).
pub fn can_present(present_modes: usize, formats: usize) -> bool {
    present_modes > 0 && formats > 0
}

/// The decision for the window: `current` is the renderer's adapter,
/// `candidates` all adapters (the current one included, if found),
/// `retried` whether this run already is the restart.
pub fn verdict(current: &str, candidates: &[Candidate], retried: bool) -> Verdict {
    let current_presents = candidates.iter().any(|c| c.name == current && c.presents);
    // Not found: the probe cannot judge, Bevy goes on as before.
    let found = candidates.iter().any(|c| c.name == current);
    if current_presents || !found {
        return Verdict::Supported;
    }
    if retried {
        return Verdict::Fail {
            reason: format!(
                "graphics adapter '{current}' cannot present to the window, also after the restart"
            ),
        };
    }
    // A GPU that can present first, then a software rasteriser.
    let pick = candidates
        .iter()
        .filter(|c| c.presents && c.name != current)
        .min_by_key(|c| c.cpu);
    match pick {
        Some(c) => Verdict::Restart {
            adapter: c.name.clone(),
        },
        None => Verdict::Fail {
            reason: format!(
                "graphics adapter '{current}' cannot present to the window and no other adapter \
                 can (X11 without DRI3, e.g. Xvfb: install Mesa's lavapipe, mesa-vulkan-drivers)"
            ),
        },
    }
}

/// The exit for a verdict (`None`: keep running).
pub fn exit_for(v: &Verdict) -> Option<AppExit> {
    match v {
        Verdict::Supported => None,
        Verdict::Restart { .. } => Some(AppExit::Success),
        Verdict::Fail { .. } => Some(AppExit::error()),
    }
}

/// The adapter to restart on, once the app has exited for it.
static RESTART_ADAPTER: Mutex<Option<String>> = Mutex::new(None);

/// The adapter the app exited to be started again on (see `main`).
pub fn restart_adapter() -> Option<String> {
    RESTART_ADAPTER.lock().ok().and_then(|a| a.clone())
}

pub struct SurfaceGuardPlugin;

impl Plugin for SurfaceGuardPlugin {
    fn build(&self, _app: &mut App) {
        // `First`: before this frame's extraction hands the window to the
        // renderer (bevy_winit creates windows between frames).
        #[cfg(target_os = "linux")]
        _app.add_systems(First, check_new_windows);
    }
}

/// Probes each new window; one the renderer's adapter cannot present to
/// loses its `RawHandleWrapper` (so the renderer never creates its surface)
/// and the app exits (see the module docs).
#[cfg(target_os = "linux")]
fn check_new_windows(
    mut commands: Commands,
    windows: Query<Entity, (With<Window>, With<bevy::window::RawHandleWrapper>)>,
    info: Option<Res<bevy::render::renderer::RenderAdapterInfo>>,
    mut exit: MessageWriter<AppExit>,
    _main_thread: bevy::ecs::system::NonSendMarker,
    mut checked: Local<bevy::platform::collections::HashSet<Entity>>,
) {
    let Some(info) = info else {
        return;
    };
    for entity in &windows {
        if checked.contains(&entity) {
            continue;
        }
        let candidates = bevy::winit::WINIT_WINDOWS.with_borrow(|w| {
            w.get_window(entity)
                .map(|window| probe(window, info.backend))
        });
        let Some(candidates) = candidates else {
            continue; // not created yet
        };
        checked.insert(entity);
        let current = info.name.clone();
        let v = verdict(&current, &candidates, std::env::var_os(RETRY_ENV).is_some());
        let Some(code) = exit_for(&v) else {
            continue;
        };
        error!(
            "graphics adapter '{current}' cannot present to the window (no present modes); \
             adapters: {candidates:?}"
        );
        match &v {
            Verdict::Restart { adapter } => {
                error!("restarting on graphics adapter '{adapter}' (WGPU_ADAPTER_NAME)");
                if let Ok(mut a) = RESTART_ADAPTER.lock() {
                    *a = Some(adapter.clone());
                }
            }
            Verdict::Fail { reason } => error!("cannot show the window: {reason}"),
            Verdict::Supported => {}
        }
        commands
            .entity(entity)
            .remove::<bevy::window::RawHandleWrapper>();
        exit.write(code);
    }
}

/// All adapters of `backend` and whether each can present to `window`.
#[cfg(target_os = "linux")]
fn probe(window: &winit::window::Window, backend: wgpu::Backend) -> Vec<Candidate> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = backend.into();
    desc.flags = wgpu::InstanceFlags::empty();
    let instance = wgpu::Instance::new(desc);
    let surface = match instance.create_surface(window) {
        Ok(s) => s,
        Err(e) => {
            warn!("surface probe: {e}");
            return Vec::new();
        }
    };
    bevy::tasks::block_on(instance.enumerate_adapters(backend.into()))
        .iter()
        .map(|a| {
            let info = a.get_info();
            let caps = surface.get_capabilities(a);
            Candidate {
                name: info.name,
                cpu: info.device_type == wgpu::DeviceType::Cpu,
                presents: can_present(caps.present_modes.len(), caps.formats.len()),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(name: &str, cpu: bool, presents: bool) -> Candidate {
        Candidate {
            name: name.into(),
            cpu,
            presents,
        }
    }

    const RADV: &str = "AMD Radeon RX Vega (RADV VEGA10)";
    const LAVAPIPE: &str = "llvmpipe (LLVM 19.1.7, 256 bits)";

    /// The marge case (2026-11-13): RADV picked under Xvfb, only lavapipe
    /// presents.
    fn marge() -> Vec<Candidate> {
        vec![c(RADV, false, false), c(LAVAPIPE, true, true)]
    }

    #[test]
    fn empty_present_modes_or_formats_cannot_present() {
        // Bevy's present-mode fallback panics on `Options: []`.
        assert!(!can_present(0, 2));
        assert!(!can_present(4, 0));
        assert!(!can_present(0, 0));
        assert!(can_present(1, 1));
    }

    #[test]
    fn a_presenting_adapter_is_kept() {
        let list = vec![c(RADV, false, true), c(LAVAPIPE, true, true)];
        assert_eq!(verdict(RADV, &list, false), Verdict::Supported);
        assert_eq!(verdict(LAVAPIPE, &marge(), false), Verdict::Supported);
        assert_eq!(exit_for(&Verdict::Supported), None);
    }

    #[test]
    fn a_gpu_without_presentation_restarts_on_lavapipe() {
        let v = verdict(RADV, &marge(), false);
        assert_eq!(
            v,
            Verdict::Restart {
                adapter: LAVAPIPE.into()
            }
        );
        // A normal exit (ride saved, devices stopped), then `main` restarts.
        assert_eq!(exit_for(&v), Some(AppExit::Success));
    }

    #[test]
    fn a_presenting_gpu_is_preferred_over_software() {
        let mut list = marge();
        list.push(c("Intel(R) UHD Graphics", false, true));
        assert_eq!(
            verdict(RADV, &list, false),
            Verdict::Restart {
                adapter: "Intel(R) UHD Graphics".into()
            }
        );
    }

    #[test]
    fn no_presenting_adapter_fails_instead_of_panicking() {
        let v = verdict(RADV, &[c(RADV, false, false)], false);
        assert!(matches!(v, Verdict::Fail { .. }), "{v:?}");
        assert_eq!(exit_for(&v), Some(AppExit::error()));
    }

    #[test]
    fn an_adapter_the_probe_does_not_see_is_left_to_bevy() {
        assert_eq!(verdict(RADV, &[], false), Verdict::Supported);
        assert_eq!(
            verdict(RADV, &[c(LAVAPIPE, true, true)], false),
            Verdict::Supported
        );
    }

    #[test]
    fn no_second_restart() {
        assert!(matches!(
            verdict(RADV, &marge(), true),
            Verdict::Fail { .. }
        ));
    }
}

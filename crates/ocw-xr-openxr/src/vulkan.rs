//! The Vulkan device, made through OpenXR (`XR_KHR_vulkan_enable2`) and
//! handed to wgpu: OpenXR picks the GPU the headset is on and creates the
//! VkInstance / VkDevice with the extensions it needs; wgpu-hal wraps the
//! raw handles, and wgpu (so Bevy) renders with them. The swapchain's
//! VkImages are wrapped as wgpu textures the same way.
//!
//! All `unsafe` of OCW lives here: each block names the rule it keeps.
#![allow(unsafe_code)]

use std::ffi::{CString, c_void};

use ash::vk::{self, Handle as _};
use ocw_xr::session::XrError;
use openxr as xr;
use wgpu_hal::api::Vulkan as HalVulkan;

/// The wgpu objects of the device OpenXR chose, and what the session
/// needs to bind to it.
pub struct VulkanDevice {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub session_info: xr::vulkan::SessionCreateInfo,
}

/// The Vulkan version we ask for: 1.2 where the runtime allows (wgpu's
/// Vulkan backend wants 1.1+; Android's Quest runtime 1.1).
fn api_version(reqs: &xr::vulkan::Requirements) -> Result<xr::Version, XrError> {
    let want = if cfg!(target_os = "android") {
        xr::Version::new(1, 1, 0)
    } else {
        xr::Version::new(1, 2, 0)
    };
    if want < reqs.min_api_version_supported {
        return Err(XrError::Unsupported(format!(
            "the runtime needs Vulkan {} or newer",
            reqs.min_api_version_supported
        )));
    }
    if want.major() > reqs.max_api_version_supported.major() {
        return Err(XrError::Unsupported(format!(
            "the runtime supports Vulkan up to {}",
            reqs.max_api_version_supported
        )));
    }
    Ok(want)
}

fn vk_version(v: xr::Version) -> u32 {
    vk::make_api_version(0, u32::from(v.major()), u32::from(v.minor()), v.patch())
}

fn other(what: &str, e: impl std::fmt::Display) -> XrError {
    XrError::Other(format!("{what}: {e}"))
}

/// Creates the Vulkan device for `system` and wraps it for wgpu.
pub fn create_device(
    instance: &xr::Instance,
    system: xr::SystemId,
    app_name: &str,
) -> Result<VulkanDevice, XrError> {
    let reqs = instance
        .graphics_requirements::<xr::Vulkan>(system)
        .map_err(crate::xr_error)?;
    let version = api_version(&reqs)?;
    let vk_target = vk_version(version);
    // SAFETY: loading the system's Vulkan loader library; nothing else
    // runs in it yet.
    let entry = unsafe { ash::Entry::load() }
        .map_err(|e| XrError::Unsupported(format!("no Vulkan loader: {e}")))?;
    let flags = wgpu::InstanceFlags::default().with_env();
    let instance_exts =
        <HalVulkan as wgpu_hal::Api>::Instance::desired_extensions(&entry, vk_target, flags)
            .map_err(|e| other("Vulkan instance extensions", e))?;
    let name = CString::new(app_name).unwrap_or_default();
    let app_info = vk::ApplicationInfo::default()
        .application_name(&name)
        .application_version(1)
        .engine_name(c"Bevy")
        .engine_version(20)
        .api_version(vk_target);
    let ext_ptrs: Vec<*const i8> = instance_exts.iter().map(|e| e.as_ptr()).collect();
    let create_info = vk::InstanceCreateInfo::default()
        .application_info(&app_info)
        .enabled_extension_names(&ext_ptrs);
    // SAFETY: `create_info` and what it points to live until the call
    // returns; the proc-address function is the loaded entry's (the type
    // is the same C function pointer, spelled by two crates).
    let raw_instance = unsafe {
        instance.create_vulkan_instance(
            system,
            std::mem::transmute::<
                vk::PFN_vkGetInstanceProcAddr,
                xr::sys::platform::VkGetInstanceProcAddr,
            >(entry.static_fn().get_instance_proc_addr),
            &create_info as *const vk::InstanceCreateInfo as *const _,
        )
    }
    .map_err(crate::xr_error)?
    .map_err(|r| other("vkCreateInstance (through OpenXR)", vk::Result::from_raw(r)))?;
    // SAFETY: the handle was just created from this entry.
    let vk_instance = unsafe {
        ash::Instance::load(entry.static_fn(), vk::Instance::from_raw(raw_instance as _))
    };
    // SAFETY: the instance was made by `create_vulkan_instance` for this system.
    let raw_physical =
        unsafe { instance.vulkan_graphics_device(system, vk_instance.handle().as_raw() as _) }
            .map_err(crate::xr_error)?;
    let physical = vk::PhysicalDevice::from_raw(raw_physical as _);
    // SAFETY: a valid instance and one of its physical devices.
    let props = unsafe { vk_instance.get_physical_device_properties(physical) };
    if props.api_version < vk_target {
        return Err(XrError::Unsupported(format!(
            "the headset's GPU has Vulkan {}.{}, OCW needs {version}",
            vk::api_version_major(props.api_version),
            vk::api_version_minor(props.api_version)
        )));
    }
    // SAFETY: enumerating layers of a loaded entry.
    let has_nv_optimus = unsafe { entry.enumerate_instance_layer_properties() }
        .unwrap_or_default()
        .iter()
        .any(|l| {
            l.layer_name_as_c_str()
                .is_ok_and(|n| n == c"VK_LAYER_NV_optimus")
        });
    // SAFETY: `vk_instance` was created from `entry` with `instance_exts`
    // (wgpu's desired extensions) and `flags`; wgpu-hal takes it over (no
    // drop callback): it destroys it last.
    let hal_instance = unsafe {
        <HalVulkan as wgpu_hal::Api>::Instance::from_raw(
            entry.clone(),
            vk_instance.clone(),
            vk_target,
            0,
            None,
            instance_exts,
            flags,
            wgpu::MemoryBudgetThresholds::default(),
            has_nv_optimus,
            None,
        )
    }
    .map_err(|e| other("wgpu Vulkan instance", e))?;
    let exposed = hal_instance
        .expose_adapter(physical)
        .ok_or_else(|| XrError::Unsupported("wgpu cannot use the headset's GPU".into()))?;
    // What Bevy asks for by default (all the adapter has, minus mappable
    // primary buffers – slow on discrete GPUs).
    let features = exposed.features - wgpu::Features::MAPPABLE_PRIMARY_BUFFERS;
    let limits = exposed.capabilities.limits.clone();
    let mut extensions = exposed.adapter.required_device_extensions(features);
    extensions.dedup();
    let mut phd_features = exposed
        .adapter
        .physical_device_features(&extensions, features);
    // The first queue family has graphics on every desktop and Quest GPU
    // wgpu supports (wgpu itself picks family 0's first queue).
    let family_index = 0;
    let priorities = [1.0f32];
    let queues = [vk::DeviceQueueCreateInfo::default()
        .queue_family_index(family_index)
        .queue_priorities(&priorities)];
    let ext_ptrs: Vec<*const i8> = extensions.iter().map(|e| e.as_ptr()).collect();
    let device_info = phd_features
        .add_to_device_create(vk::DeviceCreateInfo::default().queue_create_infos(&queues))
        .enabled_extension_names(&ext_ptrs);
    // SAFETY: as for the instance; `device_info` and its chain live until
    // the call returns.
    let raw_device = unsafe {
        instance.create_vulkan_device(
            system,
            std::mem::transmute::<
                vk::PFN_vkGetInstanceProcAddr,
                xr::sys::platform::VkGetInstanceProcAddr,
            >(entry.static_fn().get_instance_proc_addr),
            physical.as_raw() as _,
            &device_info as *const vk::DeviceCreateInfo as *const _,
        )
    }
    .map_err(crate::xr_error)?
    .map_err(|r| other("vkCreateDevice (through OpenXR)", vk::Result::from_raw(r)))?;
    // SAFETY: the device was just created on `physical` of `vk_instance`.
    let vk_device =
        unsafe { ash::Device::load(vk_instance.fp_v1_0(), vk::Device::from_raw(raw_device as _)) };
    // SAFETY: made from this adapter with `extensions` (a superset of the
    // required ones), `phd_features`, family 0, queue 0; wgpu-hal owns it.
    let open = unsafe {
        exposed.adapter.device_from_raw(
            vk_device,
            None,
            &extensions,
            features,
            &limits,
            &wgpu::MemoryHints::Performance,
            family_index,
            0,
        )
    }
    .map_err(|e| other("wgpu Vulkan device", e))?;
    // SAFETY: the hal objects come from this instance, in this order.
    let wgpu_instance = unsafe { wgpu::Instance::from_hal::<HalVulkan>(hal_instance) };
    let adapter = unsafe { wgpu_instance.create_adapter_from_hal(exposed) };
    let (device, queue) = unsafe {
        adapter.create_device_from_hal(
            open,
            &wgpu::DeviceDescriptor {
                label: Some("OCW VR device"),
                required_features: features,
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
                ..Default::default()
            },
        )
    }
    .map_err(|e| other("wgpu device", e))?;
    let session_info = xr::vulkan::SessionCreateInfo {
        instance: vk_instance.handle().as_raw() as *const c_void,
        physical_device: physical.as_raw() as *const c_void,
        device: raw_device,
        queue_family_index: family_index,
        queue_index: 0,
    };
    Ok(VulkanDevice {
        instance: wgpu_instance,
        adapter,
        device,
        queue,
        session_info,
    })
}

/// Creates a session on the device.
pub fn create_session(
    instance: &xr::Instance,
    system: xr::SystemId,
    info: &xr::vulkan::SessionCreateInfo,
) -> Result<
    (
        xr::Session<xr::Vulkan>,
        xr::FrameWaiter,
        xr::FrameStream<xr::Vulkan>,
    ),
    XrError,
> {
    // SAFETY: the handles are the live instance / device made for this
    // system by `create_device`; they outlive the session (the session is
    // dropped before the wgpu device, see `Shared`).
    unsafe { instance.create_session::<xr::Vulkan>(system, info) }.map_err(crate::xr_error)
}

/// The wgpu format of ours.
pub fn wgpu_format(f: ocw_xr::swapchain::ColorFormat) -> wgpu::TextureFormat {
    use ocw_xr::swapchain::ColorFormat as C;
    match f {
        C::Rgba8UnormSrgb => wgpu::TextureFormat::Rgba8UnormSrgb,
        C::Bgra8UnormSrgb => wgpu::TextureFormat::Bgra8UnormSrgb,
        C::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        C::Rgba8Unorm => wgpu::TextureFormat::Rgba8Unorm,
        C::Bgra8Unorm => wgpu::TextureFormat::Bgra8Unorm,
    }
}

/// Wraps the swapchain's images (two array layers, one per eye) as wgpu
/// textures we copy the eye images into.
pub fn wrap_images(
    device: &wgpu::Device,
    images: &[u64],
    format: wgpu::TextureFormat,
    extent: (u32, u32),
) -> Result<Vec<wgpu::Texture>, XrError> {
    let size = wgpu::Extent3d {
        width: extent.0,
        height: extent.1,
        depth_or_array_layers: 2,
    };
    // SAFETY: the backend is Vulkan (the device was made by
    // `create_device`).
    let hal = unsafe { device.as_hal::<HalVulkan>() }
        .ok_or_else(|| XrError::Other("the wgpu device is not a Vulkan device".into()))?;
    let out = images
        .iter()
        .map(|&image| {
            // SAFETY: a VkImage of the runtime's swapchain, created with
            // this size, format, 2 layers and colour / transfer usage; it
            // stays valid until the swapchain is destroyed, which happens
            // only after these textures are dropped (`Stream`). The memory
            // is the runtime's (`External`, no drop callback).
            let raw = unsafe {
                hal.texture_from_raw(
                    vk::Image::from_raw(image),
                    &wgpu_hal::TextureDescriptor {
                        label: Some("VR swapchain"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUses::COLOR_TARGET | wgpu::TextureUses::COPY_DST,
                        memory_flags: wgpu_hal::MemoryFlags::empty(),
                        view_formats: vec![],
                    },
                    None,
                    wgpu_hal::vulkan::TextureMemory::External,
                )
            };
            // SAFETY: `raw` was made from this device's hal just now.
            unsafe {
                device.create_texture_from_hal::<HalVulkan>(
                    raw,
                    &wgpu::TextureDescriptor {
                        label: Some("VR swapchain"),
                        size,
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format,
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                            | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    },
                    wgpu::TextureUses::UNINITIALIZED,
                )
            }
        })
        .collect();
    Ok(out)
}

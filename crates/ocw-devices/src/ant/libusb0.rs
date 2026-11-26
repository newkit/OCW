//! ANT USB sticks through Garmin's own libusb-win32 driver (spec 3.4.1
//! *The driver route on Windows*, 14 Phase 17 Q7 (d)).
//!
//! On Windows the ANT USB-m (`0fcf:1009`), the ANT USB2 (`0fcf:1008`) and
//! the clones that copy their ids get Garmin's signed driver from Windows
//! Update (on Windows 11 under *Optional updates*) – the driver Zwift,
//! Garmin Express and TrainerRoad use. It is a libusb-win32 driver
//! (`libusb0.sys`), and its package puts `libusb0.dll` into the system
//! folder. This link drives it as GoldenCheetah does: it loads the
//! system's `libusb0.dll` at run time (we ship no copy and link nothing of
//! it – libusb-win32 is LGPL), finds the stick by its ids, sets
//! configuration 1, claims interface 0 and moves the same ANT serial
//! messages over the bulk endpoints as the `nusb` link. Garmin's programs
//! keep working with the stick (one program at a time). A stick bound to
//! WinUSB (Zadig) is opened by the `nusb` link instead; [`super::Fallback`]
//! tries that one first.
//!
//! The libusb-0.1 calls sit behind the [`Usb0`] seam: on Windows with the
//! feature `ant-usb` the DLL (`Libusb0Transport::system`), in tests a
//! fake. Everything above the seam (finding the stick, the endpoints from
//! the configuration descriptor, the error codes, reads and writes) is
//! plain Rust and tested on every system.

use std::sync::Arc;
use std::time::Duration;

use super::service::{AntLink, AntTransport};
use super::{USB_PRODUCT_USB2, USB_PRODUCT_USBM, USB_VENDOR, stick_name};
use crate::DeviceError;

/// The `errno` values libusb-win32 returns, negated.
pub mod errno {
    pub const EPERM: i32 = 1;
    pub const ENOENT: i32 = 2;
    pub const EIO: i32 = 5;
    pub const ENXIO: i32 = 6;
    pub const EACCES: i32 = 13;
    pub const EBUSY: i32 = 16;
    pub const ENODEV: i32 = 19;
    pub const EINVAL: i32 = 22;
    /// libusb-0.1 on Linux.
    pub const ETIMEDOUT: i32 = 110;
    /// libusb-win32's own `ETRANSFER_TIMEDOUT`.
    pub const ETRANSFER_TIMEDOUT: i32 = 116;
    /// MSVC's `ETIMEDOUT`.
    pub const ETIMEDOUT_MSVC: i32 = 138;
}

/// A device on one of the buses, as `usb_get_busses` lists it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usb0Device {
    pub vendor: u16,
    pub product: u16,
    /// Which one, for [`Usb0::open`] (the DLL: the `usb_device` pointer,
    /// valid until the next [`Usb0::devices`]).
    pub key: usize,
}

/// An open device (`usb_dev_handle *`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Usb0Handle(pub usize);

/// The libusb-0.1 calls the link makes (`usb_*` of `lusb0_usb.h`); return
/// codes as there: `>= 0` success (bytes for transfers), `< 0` a negated
/// [`errno`].
pub trait Usb0: Send + Sync + 'static {
    /// `usb_find_busses` + `usb_find_devices`, then every device of
    /// `usb_get_busses`.
    fn devices(&self) -> Vec<Usb0Device>;
    /// `usb_open` (`None`: it failed).
    fn open(&self, device: &Usb0Device) -> Option<Usb0Handle>;
    fn set_configuration(&self, h: Usb0Handle, configuration: i32) -> i32;
    fn claim_interface(&self, h: Usb0Handle, interface: i32) -> i32;
    fn release_interface(&self, h: Usb0Handle, interface: i32) -> i32;
    /// `usb_control_msg` reading into `buf`.
    #[allow(clippy::too_many_arguments)]
    fn control_msg(
        &self,
        h: Usb0Handle,
        request_type: i32,
        request: i32,
        value: i32,
        index: i32,
        buf: &mut [u8],
        timeout_ms: i32,
    ) -> i32;
    fn bulk_write(&self, h: Usb0Handle, endpoint: i32, bytes: &[u8], timeout_ms: i32) -> i32;
    fn bulk_read(&self, h: Usb0Handle, endpoint: i32, buf: &mut [u8], timeout_ms: i32) -> i32;
    fn close(&self, h: Usb0Handle);
    /// `usb_strerror`: the last error's text.
    fn last_error(&self) -> String;
}

/// Loads the libusb-0.1 library (`Err`: why not, e.g. no `libusb0.dll`).
pub type Loader = Box<dyn FnMut() -> Result<Arc<dyn Usb0>, String> + Send>;

/// The sticks through libusb-0.1.
pub struct Libusb0Transport {
    load: Loader,
    api: Option<Arc<dyn Usb0>>,
}

impl Libusb0Transport {
    /// Over this library (loaded on first use, and again on each use
    /// while it is missing: the driver may be installed meanwhile).
    pub fn new(load: impl FnMut() -> Result<Arc<dyn Usb0>, String> + Send + 'static) -> Self {
        Self {
            load: Box::new(load),
            api: None,
        }
    }

    /// The system's `libusb0.dll` (Windows, feature `ant-usb`).
    #[cfg(all(windows, feature = "ant-usb"))]
    pub fn system() -> Self {
        Self::new(dll::load)
    }

    fn api(&mut self) -> Result<Arc<dyn Usb0>, String> {
        if let Some(a) = &self.api {
            return Ok(a.clone());
        }
        let a = (self.load)()?;
        self.api = Some(a.clone());
        Ok(a)
    }

    fn first_stick(api: &dyn Usb0) -> Option<Usb0Device> {
        api.devices().into_iter().find(|d| {
            d.vendor == USB_VENDOR && matches!(d.product, USB_PRODUCT_USB2 | USB_PRODUCT_USBM)
        })
    }
}

/// A libusb-0.1 return code as our error.
pub fn code_error(rc: i32, what: &str, text: &str) -> DeviceError {
    match -rc {
        errno::EBUSY => DeviceError::StickBusy,
        errno::EACCES | errno::EPERM => DeviceError::StickPermission,
        errno::ENODEV | errno::ENXIO | errno::ENOENT => DeviceError::NoStick,
        _ if text.is_empty() => DeviceError::Ant(format!("libusb0 {what}: error {rc}")),
        _ => DeviceError::Ant(format!("libusb0 {what}: error {rc} ({text})")),
    }
}

fn is_timeout(rc: i32) -> bool {
    matches!(
        -rc,
        errno::ETIMEDOUT | errno::ETRANSFER_TIMEDOUT | errno::ETIMEDOUT_MSVC
    )
}

/// The ANT sticks' endpoints when the configuration descriptor cannot be
/// read (both sticks use these).
pub const DEFAULT_ENDPOINTS: (u8, u8) = (0x81, 0x01);

/// The first bulk IN and OUT endpoints of interface 0 (alternate setting
/// 0) in a configuration descriptor with its interface and endpoint
/// descriptors.
pub fn bulk_endpoints(config: &[u8]) -> Option<(u8, u8)> {
    const INTERFACE: u8 = 4;
    const ENDPOINT: u8 = 5;
    let (mut ep_in, mut ep_out) = (None, None);
    let mut in_interface_0 = false;
    let mut i = 0;
    while i + 2 <= config.len() {
        let len = config[i] as usize;
        if len < 2 || i + len > config.len() {
            break;
        }
        let d = &config[i..i + len];
        match d[1] {
            INTERFACE if len >= 4 => in_interface_0 = d[2] == 0 && d[3] == 0,
            ENDPOINT if len >= 4 && in_interface_0 && d[3] & 0x03 == 0x02 => {
                if d[2] & 0x80 != 0 {
                    ep_in = ep_in.or(Some(d[2]));
                } else {
                    ep_out = ep_out.or(Some(d[2]));
                }
            }
            _ => {}
        }
        i += len;
    }
    Some((ep_in?, ep_out?))
}

const TIMEOUT_MS: i32 = 1000;

impl AntTransport for Libusb0Transport {
    fn find(&mut self) -> Option<String> {
        let api = self.api().ok()?;
        Self::first_stick(api.as_ref()).map(|d| stick_name(d.product).to_owned())
    }

    fn open(&mut self) -> Result<Box<dyn AntLink>, DeviceError> {
        let api = self.api().map_err(DeviceError::StickDriver)?;
        // Only sticks bound to libusb-win32 are listed (Garmin's driver).
        let dev = Self::first_stick(api.as_ref()).ok_or(DeviceError::NoStick)?;
        let name = stick_name(dev.product).to_owned();
        let h = api.open(&dev).ok_or_else(|| {
            let text = api.last_error();
            DeviceError::Ant(format!("libusb0 open: {text}"))
        })?;
        let fail = |rc: i32, what: &str, claimed: bool| {
            let e = code_error(rc, what, &api.last_error());
            if claimed {
                api.release_interface(h, 0);
            }
            api.close(h);
            e
        };
        let rc = api.set_configuration(h, 1);
        if rc < 0 {
            return Err(fail(rc, "set configuration", false));
        }
        let rc = api.claim_interface(h, 0);
        if rc < 0 {
            return Err(fail(rc, "claim interface", false));
        }
        // GET_DESCRIPTOR (configuration 0): the endpoints.
        let mut buf = [0u8; 255];
        let rc = api.control_msg(h, 0x80, 0x06, 0x0200, 0, &mut buf, TIMEOUT_MS);
        let found = (rc > 0)
            .then(|| bulk_endpoints(&buf[..(rc as usize).min(buf.len())]))
            .flatten();
        let (ep_in, ep_out) = found.unwrap_or_else(|| {
            tracing::debug!(rc, "ANT stick: no endpoints read, using 0x81 / 0x01");
            DEFAULT_ENDPOINTS
        });
        tracing::info!(stick = %name, "ANT stick opened through libusb-win32 (Garmin's driver)");
        Ok(Box::new(Libusb0Link {
            api,
            h,
            name,
            ep_in,
            ep_out,
        }))
    }
}

struct Libusb0Link {
    api: Arc<dyn Usb0>,
    h: Usb0Handle,
    name: String,
    ep_in: u8,
    ep_out: u8,
}

impl Drop for Libusb0Link {
    fn drop(&mut self) {
        self.api.release_interface(self.h, 0);
        self.api.close(self.h);
    }
}

impl AntLink for Libusb0Link {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        let rc = self
            .api
            .bulk_write(self.h, i32::from(self.ep_out), bytes, TIMEOUT_MS);
        if rc < 0 {
            return Err(code_error(rc, "write", &self.api.last_error()));
        }
        if (rc as usize) < bytes.len() {
            return Err(DeviceError::Ant(format!(
                "libusb0 write: {rc} of {} bytes",
                bytes.len()
            )));
        }
        Ok(())
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        let mut buf = [0u8; 64];
        let ms = i32::try_from(timeout.as_millis())
            .unwrap_or(i32::MAX)
            .max(1);
        let rc = self
            .api
            .bulk_read(self.h, i32::from(self.ep_in), &mut buf, ms);
        match rc {
            n if n >= 0 => Ok(buf[..(n as usize).min(buf.len())].to_vec()),
            n if is_timeout(n) => Ok(Vec::new()),
            n => Err(code_error(n, "read", &self.api.last_error())),
        }
    }
}

/// The system's `libusb0.dll`, loaded at run time from the system folder
/// only (where Garmin's driver package puts it; never from the app's
/// folder or the working directory).
#[cfg(all(windows, feature = "ant-usb"))]
#[allow(unsafe_code)]
mod dll {
    use std::ffi::{CStr, c_char, c_int, c_ulong, c_void};
    use std::sync::{Arc, Mutex};

    use libloading::os::windows::{LOAD_LIBRARY_SEARCH_SYSTEM32, Library};

    use super::{Usb0, Usb0Device, Usb0Handle};

    /// `LIBUSB_PATH_MAX` of libusb-win32's `lusb0_usb.h`.
    const PATH_MAX: usize = 512;

    /// `struct usb_device_descriptor` (18 bytes; the same packed or not).
    #[repr(C)]
    #[allow(dead_code)] // the C layout
    struct DeviceDescriptor {
        b_length: u8,
        b_descriptor_type: u8,
        bcd_usb: u16,
        b_device_class: u8,
        b_device_sub_class: u8,
        b_device_protocol: u8,
        b_max_packet_size0: u8,
        id_vendor: u16,
        id_product: u16,
        bcd_device: u16,
        i_manufacturer: u8,
        i_product: u8,
        i_serial_number: u8,
        b_num_configurations: u8,
    }

    /// The head of `struct usb_device`: only the fields up to the
    /// descriptor are read (their offsets do not depend on packing).
    #[repr(C)]
    #[allow(dead_code)] // the C layout
    struct UsbDevice {
        next: *mut UsbDevice,
        prev: *mut UsbDevice,
        filename: [c_char; PATH_MAX],
        bus: *mut UsbBus,
        descriptor: DeviceDescriptor,
    }

    /// The head of `struct usb_bus`.
    #[repr(C)]
    #[allow(dead_code)] // the C layout
    struct UsbBus {
        next: *mut UsbBus,
        prev: *mut UsbBus,
        dirname: [c_char; PATH_MAX],
        devices: *mut UsbDevice,
        location: c_ulong,
    }

    type Handle = *mut c_void;

    struct Api {
        _lib: Library,
        /// libusb-0.1 keeps its bus list in globals.
        lock: Mutex<()>,
        find_busses: unsafe extern "C" fn() -> c_int,
        find_devices: unsafe extern "C" fn() -> c_int,
        get_busses: unsafe extern "C" fn() -> *mut UsbBus,
        open: unsafe extern "C" fn(*mut UsbDevice) -> Handle,
        close: unsafe extern "C" fn(Handle) -> c_int,
        set_configuration: unsafe extern "C" fn(Handle, c_int) -> c_int,
        claim_interface: unsafe extern "C" fn(Handle, c_int) -> c_int,
        release_interface: unsafe extern "C" fn(Handle, c_int) -> c_int,
        control_msg: unsafe extern "C" fn(
            Handle,
            c_int,
            c_int,
            c_int,
            c_int,
            *mut c_char,
            c_int,
            c_int,
        ) -> c_int,
        bulk_write: unsafe extern "C" fn(Handle, c_int, *mut c_char, c_int, c_int) -> c_int,
        bulk_read: unsafe extern "C" fn(Handle, c_int, *mut c_char, c_int, c_int) -> c_int,
        strerror: unsafe extern "C" fn() -> *mut c_char,
    }

    // SAFETY: the function pointers stay valid while `_lib` is loaded; the
    // bus list (libusb-0.1's globals) is only walked under `lock`.
    unsafe impl Send for Api {}
    unsafe impl Sync for Api {}

    /// A libusb-0.1 function (`T`: its signature as in lusb0_usb.h).
    fn sym<T: Copy>(lib: &Library, name: &str) -> Result<T, String> {
        let mut n = name.as_bytes().to_vec();
        n.push(0);
        // SAFETY: the caller names the symbol's C signature as `T`.
        unsafe { lib.get::<T>(&n) }
            .map(|s| *s)
            .map_err(|e| format!("libusb0.dll has no {name}: {e}"))
    }

    pub fn load() -> Result<Arc<dyn Usb0>, String> {
        // SAFETY: loading the system's libusb0.dll runs its DllMain, which
        // only sets up the library.
        let lib = unsafe { Library::load_with_flags("libusb0.dll", LOAD_LIBRARY_SEARCH_SYSTEM32) }
            .map_err(|e| {
                format!(
                    "libusb0.dll not found ({e}): Garmin's ANT driver (libusb-win32) is not installed"
                )
            })?;
        let init: unsafe extern "C" fn() = sym(&lib, "usb_init")?;
        let api = Api {
            lock: Mutex::new(()),
            find_busses: sym(&lib, "usb_find_busses")?,
            find_devices: sym(&lib, "usb_find_devices")?,
            get_busses: sym(&lib, "usb_get_busses")?,
            open: sym(&lib, "usb_open")?,
            close: sym(&lib, "usb_close")?,
            set_configuration: sym(&lib, "usb_set_configuration")?,
            claim_interface: sym(&lib, "usb_claim_interface")?,
            release_interface: sym(&lib, "usb_release_interface")?,
            control_msg: sym(&lib, "usb_control_msg")?,
            bulk_write: sym(&lib, "usb_bulk_write")?,
            bulk_read: sym(&lib, "usb_bulk_read")?,
            strerror: sym(&lib, "usb_strerror")?,
            _lib: lib,
        };
        // SAFETY: usb_init takes no arguments and may be called again.
        unsafe { init() };
        tracing::info!("libusb0.dll loaded (Garmin's ANT driver)");
        Ok(Arc::new(api))
    }

    fn h(x: Usb0Handle) -> Handle {
        x.0 as Handle
    }

    fn len(n: usize) -> c_int {
        c_int::try_from(n).unwrap_or(c_int::MAX)
    }

    impl Usb0 for Api {
        fn devices(&self) -> Vec<Usb0Device> {
            let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            let mut out = Vec::new();
            // SAFETY: libusb-0.1's bus and device lists, rebuilt by
            // find_busses / find_devices and read before the next rebuild.
            unsafe {
                (self.find_busses)();
                (self.find_devices)();
                let mut bus = (self.get_busses)();
                while !bus.is_null() {
                    let mut dev = (*bus).devices;
                    while !dev.is_null() {
                        let d = &(*dev).descriptor;
                        out.push(Usb0Device {
                            vendor: d.id_vendor,
                            product: d.id_product,
                            key: dev as usize,
                        });
                        dev = (*dev).next;
                    }
                    bus = (*bus).next;
                }
            }
            out
        }

        fn open(&self, device: &Usb0Device) -> Option<Usb0Handle> {
            let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
            // SAFETY: the key is a usb_device pointer of the last list.
            let p = unsafe { (self.open)(device.key as *mut UsbDevice) };
            (!p.is_null()).then_some(Usb0Handle(p as usize))
        }

        fn set_configuration(&self, x: Usb0Handle, configuration: i32) -> i32 {
            // SAFETY: an open handle.
            unsafe { (self.set_configuration)(h(x), configuration) }
        }

        fn claim_interface(&self, x: Usb0Handle, interface: i32) -> i32 {
            // SAFETY: an open handle.
            unsafe { (self.claim_interface)(h(x), interface) }
        }

        fn release_interface(&self, x: Usb0Handle, interface: i32) -> i32 {
            // SAFETY: an open handle.
            unsafe { (self.release_interface)(h(x), interface) }
        }

        fn control_msg(
            &self,
            x: Usb0Handle,
            request_type: i32,
            request: i32,
            value: i32,
            index: i32,
            buf: &mut [u8],
            timeout_ms: i32,
        ) -> i32 {
            // SAFETY: an open handle; the buffer's length is passed.
            unsafe {
                (self.control_msg)(
                    h(x),
                    request_type,
                    request,
                    value,
                    index,
                    buf.as_mut_ptr().cast(),
                    len(buf.len()),
                    timeout_ms,
                )
            }
        }

        fn bulk_write(&self, x: Usb0Handle, endpoint: i32, bytes: &[u8], timeout_ms: i32) -> i32 {
            // libusb-0.1 takes a `char *` but does not write to it.
            let mut copy = bytes.to_vec();
            // SAFETY: an open handle; the buffer's length is passed.
            unsafe {
                (self.bulk_write)(
                    h(x),
                    endpoint,
                    copy.as_mut_ptr().cast(),
                    len(copy.len()),
                    timeout_ms,
                )
            }
        }

        fn bulk_read(&self, x: Usb0Handle, endpoint: i32, buf: &mut [u8], timeout_ms: i32) -> i32 {
            // SAFETY: an open handle; the buffer's length is passed.
            unsafe {
                (self.bulk_read)(
                    h(x),
                    endpoint,
                    buf.as_mut_ptr().cast(),
                    len(buf.len()),
                    timeout_ms,
                )
            }
        }

        fn close(&self, x: Usb0Handle) {
            // SAFETY: an open handle, closed once (the link's drop).
            unsafe { (self.close)(h(x)) };
        }

        fn last_error(&self) -> String {
            // SAFETY: usb_strerror returns a static NUL-terminated string.
            unsafe {
                let p = (self.strerror)();
                if p.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(p).to_string_lossy().into_owned()
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ant::codec::Message;
    use crate::ant::{Fallback, SimTransport, fallback_error};

    /// A configuration descriptor as the ANT USB-m sends it: one
    /// interface, bulk IN 0x81 and bulk OUT 0x01 (64 bytes each).
    const CONFIG: [u8; 32] = [
        9, 2, 32, 0, 1, 1, 0, 0x80, 50, // configuration
        9, 4, 0, 0, 2, 0xff, 0, 0, 0, // interface 0
        7, 5, 0x81, 2, 64, 0, 1, // bulk IN
        7, 5, 0x01, 2, 64, 0, 1, // bulk OUT
    ];

    #[derive(Default)]
    struct State {
        devices: Vec<Usb0Device>,
        /// What the next open / claim / transfer answers.
        open_fails: bool,
        claim_rc: i32,
        config: Option<Vec<u8>>,
        read_queue: Vec<i32>,
        reads: Vec<Vec<u8>>,
        writes: Vec<(i32, Vec<u8>)>,
        write_rc: Option<i32>,
        read_eps: Vec<i32>,
        open: usize,
        claimed: usize,
        log: Vec<String>,
    }

    /// The fake `libusb0.dll`.
    #[derive(Default)]
    struct Fake(Mutex<State>);

    impl Fake {
        fn with<R>(&self, f: impl FnOnce(&mut State) -> R) -> R {
            f(&mut self.0.lock().unwrap())
        }
    }

    impl Usb0 for Fake {
        fn devices(&self) -> Vec<Usb0Device> {
            self.with(|s| s.devices.clone())
        }
        fn open(&self, d: &Usb0Device) -> Option<Usb0Handle> {
            self.with(|s| {
                s.log.push(format!("open {:04x}", d.product));
                if s.open_fails {
                    return None;
                }
                s.open += 1;
                Some(Usb0Handle(0x1000 + d.key))
            })
        }
        fn set_configuration(&self, _: Usb0Handle, c: i32) -> i32 {
            self.with(|s| s.log.push(format!("configuration {c}")));
            0
        }
        fn claim_interface(&self, _: Usb0Handle, i: i32) -> i32 {
            self.with(|s| {
                s.log.push(format!("claim {i}"));
                if s.claim_rc >= 0 {
                    s.claimed += 1;
                }
                s.claim_rc
            })
        }
        fn release_interface(&self, _: Usb0Handle, _: i32) -> i32 {
            self.with(|s| s.claimed = s.claimed.saturating_sub(1));
            0
        }
        fn control_msg(
            &self,
            _: Usb0Handle,
            rt: i32,
            r: i32,
            v: i32,
            _: i32,
            buf: &mut [u8],
            _: i32,
        ) -> i32 {
            assert_eq!(
                (rt, r, v),
                (0x80, 6, 0x0200),
                "GET_DESCRIPTOR configuration"
            );
            self.with(|s| match &s.config {
                Some(c) => {
                    let n = c.len().min(buf.len());
                    buf[..n].copy_from_slice(&c[..n]);
                    n as i32
                }
                None => -errno::EIO,
            })
        }
        fn bulk_write(&self, _: Usb0Handle, ep: i32, bytes: &[u8], _: i32) -> i32 {
            self.with(|s| {
                s.writes.push((ep, bytes.to_vec()));
                s.write_rc.unwrap_or(bytes.len() as i32)
            })
        }
        fn bulk_read(&self, _: Usb0Handle, ep: i32, buf: &mut [u8], _: i32) -> i32 {
            self.with(|s| {
                s.read_eps.push(ep);
                if let Some(rc) = s.read_queue.pop() {
                    return rc;
                }
                if s.reads.is_empty() {
                    return -errno::ETRANSFER_TIMEDOUT;
                }
                let r = s.reads.remove(0);
                buf[..r.len()].copy_from_slice(&r);
                r.len() as i32
            })
        }
        fn close(&self, _: Usb0Handle) {
            self.with(|s| s.open = s.open.saturating_sub(1));
        }
        fn last_error(&self) -> String {
            "fake error".into()
        }
    }

    fn usbm() -> Usb0Device {
        Usb0Device {
            vendor: USB_VENDOR,
            product: USB_PRODUCT_USBM,
            key: 7,
        }
    }

    fn transport(fake: &Arc<Fake>) -> Libusb0Transport {
        let fake = fake.clone();
        Libusb0Transport::new(move || Ok(fake.clone() as Arc<dyn Usb0>))
    }

    #[test]
    fn opens_the_stick_by_its_ids_and_moves_the_message_stream() {
        let fake = Arc::new(Fake::default());
        fake.with(|s| {
            s.devices = vec![
                Usb0Device {
                    vendor: 0x046d,
                    product: 0xc52b,
                    key: 1,
                },
                usbm(),
            ];
            s.config = Some(CONFIG.to_vec());
        });
        let mut t = transport(&fake);
        assert_eq!(t.find().as_deref(), Some("ANT USB-m stick"));
        let mut link = t.open().expect("opens");
        assert_eq!(link.name(), "ANT USB-m stick");
        assert_eq!(
            fake.with(|s| s.log.clone()),
            ["open 1009", "configuration 1", "claim 0"]
        );
        // The same ANT serial messages as over nusb, on the bulk endpoints.
        let reset = Message::reset().encode();
        link.write(&reset).unwrap();
        assert_eq!(fake.with(|s| s.writes.clone()), [(0x01, reset.clone())]);
        fake.with(|s| s.reads.push(vec![0xa4, 1, 0x6f, 0x20, 0xea]));
        assert_eq!(
            link.read(Duration::from_millis(20)).unwrap(),
            [0xa4, 1, 0x6f, 0x20, 0xea]
        );
        // A timeout is no error, only nothing.
        assert!(link.read(Duration::from_millis(20)).unwrap().is_empty());
        assert!(fake.with(|s| s.read_eps.iter().all(|&e| e == 0x81)));
        // Closed with the link.
        drop(link);
        assert_eq!(fake.with(|s| (s.open, s.claimed)), (0, 0));
    }

    #[test]
    fn the_usb2_and_its_endpoints_from_the_descriptor() {
        let fake = Arc::new(Fake::default());
        let mut config = CONFIG.to_vec();
        config[20] = 0x82; // IN 0x82
        config[27] = 0x03; // OUT 0x03
        fake.with(|s| {
            s.devices = vec![Usb0Device {
                vendor: USB_VENDOR,
                product: USB_PRODUCT_USB2,
                key: 3,
            }];
            s.config = Some(config);
        });
        let mut link = transport(&fake).open().unwrap();
        assert_eq!(link.name(), "ANT USB2 stick");
        link.write(&[1, 2, 3]).unwrap();
        let _ = link.read(Duration::from_millis(5));
        assert_eq!(fake.with(|s| s.writes[0].0), 0x03);
        assert_eq!(fake.with(|s| s.read_eps[0]), 0x82);
        // No descriptor: the sticks' usual endpoints.
        let fake = Arc::new(Fake::default());
        fake.with(|s| s.devices = vec![usbm()]);
        let mut link = transport(&fake).open().unwrap();
        link.write(&[1]).unwrap();
        assert_eq!(fake.with(|s| s.writes[0].0), 0x01);
    }

    #[test]
    fn not_found_missing_dll_busy_and_permission() {
        // No ANT stick on libusb-win32 (another device only).
        let fake = Arc::new(Fake::default());
        fake.with(|s| {
            s.devices = vec![Usb0Device {
                vendor: USB_VENDOR,
                product: 0x1004, // the USB1 is a serial stick
                key: 1,
            }]
        });
        let mut t = transport(&fake);
        assert_eq!(t.find(), None);
        assert!(matches!(t.open(), Err(DeviceError::NoStick)));
        // No libusb0.dll: said so, and tried again next time.
        let mut tries = 0;
        let mut t = Libusb0Transport::new(move || {
            tries += 1;
            Err(format!("libusb0.dll not found (try {tries})"))
        });
        assert_eq!(t.find(), None);
        match t.open() {
            Err(DeviceError::StickDriver(why)) => {
                assert!(why.contains("libusb0.dll not found (try 2)"), "{why}")
            }
            other => panic!("{:?}", other.err()),
        }
        // Busy: another program claimed it (Zwift, Garmin Express).
        let fake = Arc::new(Fake::default());
        fake.with(|s| {
            s.devices = vec![usbm()];
            s.claim_rc = -errno::EBUSY;
        });
        assert!(matches!(
            transport(&fake).open(),
            Err(DeviceError::StickBusy)
        ));
        assert_eq!(fake.with(|s| s.open), 0, "closed again");
        fake.with(|s| s.claim_rc = -errno::EACCES);
        assert!(matches!(
            transport(&fake).open(),
            Err(DeviceError::StickPermission)
        ));
        fake.with(|s| s.open_fails = true);
        assert!(matches!(transport(&fake).open(), Err(DeviceError::Ant(_))));
    }

    #[test]
    fn read_and_write_errors() {
        let fake = Arc::new(Fake::default());
        fake.with(|s| s.devices = vec![usbm()]);
        let mut link = transport(&fake).open().unwrap();
        // Unplugged.
        fake.with(|s| s.read_queue.push(-errno::ENODEV));
        assert!(matches!(
            link.read(Duration::from_millis(5)),
            Err(DeviceError::NoStick)
        ));
        fake.with(|s| s.read_queue.push(-errno::EIO));
        assert!(matches!(
            link.read(Duration::from_millis(5)),
            Err(DeviceError::Ant(e)) if e.contains("fake error")
        ));
        fake.with(|s| s.write_rc = Some(2));
        assert!(matches!(link.write(&[1, 2, 3]), Err(DeviceError::Ant(_))));
        fake.with(|s| s.write_rc = Some(-errno::ENODEV));
        assert!(matches!(link.write(&[1]), Err(DeviceError::NoStick)));
        // The other timeout codes are no errors either.
        for rc in [errno::ETIMEDOUT, errno::ETIMEDOUT_MSVC] {
            fake.with(|s| s.read_queue.push(-rc));
            assert!(link.read(Duration::ZERO).unwrap().is_empty());
        }
    }

    #[test]
    fn endpoints_only_of_interface_0_and_bulk() {
        assert_eq!(bulk_endpoints(&CONFIG), Some((0x81, 0x01)));
        // An interrupt endpoint first, then bulk ones.
        let mut c = CONFIG[..18].to_vec();
        c.extend([7, 5, 0x83, 3, 8, 0, 1]);
        c.extend(&CONFIG[18..]);
        assert_eq!(bulk_endpoints(&c), Some((0x81, 0x01)));
        // Truncated or empty: none.
        assert_eq!(bulk_endpoints(&CONFIG[..20]), None);
        assert_eq!(bulk_endpoints(&[]), None);
        assert_eq!(bulk_endpoints(&[0, 0, 0]), None);
    }

    #[test]
    fn fallback_tries_winusb_first_then_garmins_driver() {
        // WinUSB (here: the simulated stick) opens: the second is not asked.
        let fake = Arc::new(Fake::default());
        fake.with(|s| s.devices = vec![usbm()]);
        let sim = SimTransport::new(None, Vec::new());
        let mut both = Fallback::new(sim.clone(), transport(&fake));
        let link = both.open().unwrap();
        assert_eq!(link.name(), "Simulated ANT stick");
        assert!(fake.with(|s| s.log.is_empty()));
        drop(link);
        // The first route cannot open it (Garmin's driver is bound, not
        // WinUSB): libusb-win32 opens it.
        sim.set_denied(true);
        let link = both.open().unwrap();
        assert_eq!(link.name(), "ANT USB-m stick");
        drop(link);
        // Neither: the stick needs a driver.
        fake.with(|s| s.devices.clear());
        assert!(matches!(both.open(), Err(DeviceError::StickDriver(_))));
        // Busy on either route: busy.
        sim.set_denied(false);
        sim.set_busy(true);
        assert!(matches!(both.open(), Err(DeviceError::StickBusy)));
    }

    #[test]
    fn fallback_errors_say_the_most_useful_thing() {
        use DeviceError as E;
        let drv = || E::StickDriver("libusb0.dll not found".into());
        assert!(matches!(fallback_error(E::NoStick, E::NoStick), E::NoStick));
        assert!(matches!(fallback_error(E::NoStick, drv()), E::NoStick));
        assert!(matches!(
            fallback_error(E::StickPermission, E::NoStick),
            E::StickDriver(_)
        ));
        assert!(matches!(
            fallback_error(E::Ant("incompatible driver".into()), drv()),
            E::StickDriver(w) if w.contains("incompatible driver") && w.contains("libusb0.dll")
        ));
        assert!(matches!(
            fallback_error(E::StickBusy, E::NoStick),
            E::StickBusy
        ));
        assert!(matches!(
            fallback_error(E::NoStick, E::StickBusy),
            E::StickBusy
        ));
        assert!(matches!(
            fallback_error(E::Ant("x".into()), E::StickPermission),
            E::StickPermission
        ));
    }
}

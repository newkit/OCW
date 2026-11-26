//! ANT USB sticks over raw USB with `nusb` (pure Rust, MIT / Apache-2.0;
//! spec 3.4.1 *Links*): the ANT USB-m (`0fcf:1009`) and ANT USB2
//! (`0fcf:1008`) and the clones that copy their ids – interface 0, one
//! bulk OUT and one bulk IN endpoint carrying the ANT serial messages.
//!
//! - **Linux:** the kernel's `usb_serial_simple` driver claims the stick
//!   as `/dev/ttyUSBn`; opening detaches it (and the kernel takes it back
//!   when the stick is closed). An ordinary user may open the device node
//!   only when a udev rule allows it – the packaging question of spec 14
//!   Phase 17 Q8; without it the Devices page says *cannot open the stick
//!   (permission)*.
//! - **Windows:** `nusb` opens only a stick with WinUSB bound to it
//!   (Zadig); a stick on Garmin's own libusb-win32 driver is opened by
//!   `libusb0::Libusb0Transport`, tried after this one (Q7 (d)).
//! - **macOS:** no kernel driver claims it.

use std::time::Duration;

use nusb::MaybeFuture;
use nusb::descriptors::TransferType;
use nusb::transfer::{Buffer, Bulk, Direction, In, Out, TransferError};

use super::service::{AntLink, AntTransport};
use super::{USB_PRODUCT_USB2, USB_PRODUCT_USBM, USB_VENDOR, stick_name};
use crate::DeviceError;

/// The real sticks.
#[derive(Debug, Default)]
pub struct UsbTransport;

impl UsbTransport {
    pub fn new() -> Self {
        Self
    }
}

fn is_ant(d: &nusb::DeviceInfo) -> bool {
    d.vendor_id() == USB_VENDOR && matches!(d.product_id(), USB_PRODUCT_USB2 | USB_PRODUCT_USBM)
}

fn first_stick() -> Option<nusb::DeviceInfo> {
    match nusb::list_devices().wait() {
        Ok(mut list) => list.find(is_ant),
        Err(e) => {
            tracing::debug!("cannot list USB devices: {e}");
            None
        }
    }
}

fn error(e: &nusb::Error) -> DeviceError {
    match e.kind() {
        nusb::ErrorKind::PermissionDenied => DeviceError::StickPermission,
        nusb::ErrorKind::Busy => DeviceError::StickBusy,
        nusb::ErrorKind::Disconnected | nusb::ErrorKind::NotFound => DeviceError::NoStick,
        _ => DeviceError::Ant(e.to_string()),
    }
}

impl AntTransport for UsbTransport {
    fn find(&mut self) -> Option<String> {
        first_stick().map(|d| stick_name(d.product_id()).to_owned())
    }

    fn open(&mut self) -> Result<Box<dyn AntLink>, DeviceError> {
        let info = first_stick().ok_or(DeviceError::NoStick)?;
        let name = stick_name(info.product_id()).to_owned();
        let device = info.open().wait().map_err(|e| error(&e))?;
        let interface = device
            .detach_and_claim_interface(0)
            .wait()
            .map_err(|e| error(&e))?;
        let (mut ep_in, mut ep_out) = (None, None);
        if let Some(desc) = interface.descriptor() {
            for ep in desc.endpoints() {
                if ep.transfer_type() != TransferType::Bulk {
                    continue;
                }
                match ep.direction() {
                    Direction::In => ep_in = ep_in.or(Some(ep.address())),
                    Direction::Out => ep_out = ep_out.or(Some(ep.address())),
                }
            }
        }
        let (Some(a_in), Some(a_out)) = (ep_in, ep_out) else {
            return Err(DeviceError::Ant(format!("{name}: no bulk endpoints")));
        };
        let reader = interface
            .endpoint::<Bulk, In>(a_in)
            .map_err(|e| error(&e))?;
        let writer = interface
            .endpoint::<Bulk, Out>(a_out)
            .map_err(|e| error(&e))?;
        tracing::info!(stick = %name, "ANT stick opened over USB");
        Ok(Box::new(UsbLink {
            name,
            reader,
            writer,
            _interface: interface,
        }))
    }
}

struct UsbLink {
    name: String,
    reader: nusb::Endpoint<Bulk, In>,
    writer: nusb::Endpoint<Bulk, Out>,
    _interface: nusb::Interface,
}

fn transfer_error(e: TransferError) -> DeviceError {
    match e {
        TransferError::Disconnected => DeviceError::NoStick,
        other => DeviceError::Ant(other.to_string()),
    }
}

impl AntLink for UsbLink {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn write(&mut self, bytes: &[u8]) -> Result<(), DeviceError> {
        let c = self
            .writer
            .transfer_blocking(Buffer::from(bytes.to_vec()), Duration::from_secs(1));
        c.status.map_err(transfer_error)
    }

    fn read(&mut self, timeout: Duration) -> Result<Vec<u8>, DeviceError> {
        // One IN transfer stays pending between reads.
        if self.reader.pending() == 0 {
            let size = self.reader.max_packet_size().max(64);
            self.reader.submit(Buffer::new(size));
        }
        let Some(c) = self.reader.wait_next_complete(timeout) else {
            return Ok(Vec::new());
        };
        match c.status {
            Ok(()) => {
                let n = c.actual_len.min(c.buffer.len());
                Ok(c.buffer[..n].to_vec())
            }
            Err(TransferError::Cancelled) => Ok(Vec::new()),
            Err(e) => Err(transfer_error(e)),
        }
    }
}

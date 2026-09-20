//! The TC002 on a USB cable. Its USB-C port is an adb gadget, the same adbd as on port 5555, so over the cable the
//! bridge can do all it does over WiFi, with no network at all.
//!
//! Only one program can hold the interface. Where an adb server runs (anyone who uses the adb tool has one), it
//! holds every adb device it sees, and we go through it instead (server.rs).

use std::io::{self, Write};
use std::time::Duration;

use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, Direction, In, Out};
use nusb::MaybeFuture;

use crate::adb::{Adb, Sender, TICK};

/// What the TC002's firmware sets in init.rc: Google's vendor id, a product id of ZKSWE's.
const ID: (u16, u16) = (0x18d1, 0xd002);
/// adb's interface: vendor-specific class, subclass 0x42, protocol 1.
const ADB: (u8, u8, u8) = (0xff, 0x42, 0x01);

fn bad(what: impl Into<String>) -> io::Error {
    io::Error::other(what.into())
}

fn panels() -> Vec<nusb::DeviceInfo> {
    let is_panel = |d: &nusb::DeviceInfo| (d.vendor_id(), d.product_id()) == ID && d.interfaces().any(|i| (i.class(), i.subclass(), i.protocol()) == ADB);
    nusb::list_devices().wait().map(|all| all.filter(is_panel).collect()).unwrap_or_default()
}

/// Whether a TC002 is on the cable right now. Cheap (it reads sysfs), so it can be asked several times a second;
/// that matters after a power-up, when the port is only there for a few seconds (see `plant`).
pub fn present() -> bool {
    !panels().is_empty()
}

struct Usb(nusb::io::EndpointWrite<Bulk>);

impl Sender for Usb {
    fn send(&mut self, header: &[u8; 24], payload: &[u8]) -> io::Result<()> {
        // Each ends its transfer, with a zero-length packet where the length is a multiple of the packet size.
        self.0.write_all(header)?;
        self.0.flush_end()?;
        if !payload.is_empty() {
            self.0.write_all(payload)?;
            self.0.flush_end()?;
        }
        Ok(())
    }
}

/// Opens the one TC002 on the cable and says hello to its adbd.
pub fn connect() -> io::Result<Adb> {
    let info = match panels().as_slice() {
        [one] => one.clone(),
        [] => return Err(bad("no TC002 on a USB cable (after a power-up its port only answers while the pixbar program runs; see README)")),
        _ => return Err(bad("more than one TC002 on USB; unplug all but one")),
    };
    let number = info.interfaces().find(|i| (i.class(), i.subclass(), i.protocol()) == ADB).map(|i| i.interface_number()).unwrap();
    let device = info.open().wait().map_err(|e| match e.kind() {
        nusb::ErrorKind::PermissionDenied => bad(format!(
            "no permission to open the TC002 on USB (bus {} device {}). A udev rule grants it:\n  echo 'SUBSYSTEM==\"usb\", ATTR{{idVendor}}==\"18d1\", ATTR{{idProduct}}==\"d002\", MODE=\"0660\", TAG+=\"uaccess\"' | sudo tee /etc/udev/rules.d/70-pixbar.rules && sudo udevadm control --reload && sudo udevadm trigger\n(uaccess covers whoever is logged in at the machine; for a service without a login add GROUP=\"<your group>\")",
            info.bus_id(),
            info.device_address()
        )),
        _ => bad(format!("opening the TC002 on USB: {e}")),
    })?;
    let interface = device.claim_interface(number).wait().map_err(|e| match e.kind() {
        nusb::ErrorKind::Busy => io::Error::new(io::ErrorKind::ResourceBusy, "another program holds the TC002's USB interface (an adb server?)"),
        _ => bad(format!("claiming the TC002's adb interface: {e}")),
    })?;
    let config = device.active_configuration().map_err(|e| bad(format!("reading the TC002's USB descriptors: {e}")))?;
    let bulk = |dir: Direction| {
        config
            .interface_alt_settings()
            .filter(|alt| alt.interface_number() == number)
            .flat_map(|alt| alt.endpoints().collect::<Vec<_>>())
            .find(|e| e.transfer_type() == TransferType::Bulk && e.direction() == dir)
            .map(|e| e.address())
            .ok_or_else(|| bad("the TC002's adb interface lacks a bulk endpoint"))
    };
    let mut from_device = interface.endpoint::<Bulk, In>(bulk(Direction::In)?).map_err(|e| bad(e.to_string()))?;
    let mut to_device = interface.endpoint::<Bulk, Out>(bulk(Direction::Out)?).map_err(|e| bad(e.to_string()))?;
    // A client that went away mid-transfer leaves an endpoint halted, and every transfer after it would stall.
    let _ = from_device.clear_halt().wait();
    let _ = to_device.clear_halt().wait();
    // 16 KiB: a multiple of the packet size, as reads have to be.
    let reader = from_device.reader(16 * 1024).with_read_timeout(TICK);
    let writer = to_device.writer(16 * 1024).with_write_timeout(Duration::from_secs(10));
    Adb::over(Box::new(reader), Box::new(Usb(writer)))
}

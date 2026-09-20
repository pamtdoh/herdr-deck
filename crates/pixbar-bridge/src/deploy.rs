//! Starting the pixbar program on a TC002 and handing the panel back, over the device's own adbd.

use std::io;
use std::path::PathBuf;

use crate::adb::Adb;
use crate::link::Link;
use crate::{server, usb};

/// The ARM build of pixbar-device that build.rs made for this bridge.
static DEVICE_PROGRAM: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/pixbar-device"));

const STOP: &str = "if [ -f /tmp/pixbar-device.pid ]; then kill $(cat /tmp/pixbar-device.pid) 2>/dev/null; rm -f /tmp/pixbar-device.pid; fi";

/// FNV-1a, to tell builds apart; the device reports the same over its own executable.
pub fn build_id(bytes: &[u8]) -> String {
    format!("{:016x}", bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)))
}

pub fn embedded_build() -> String {
    build_id(DEVICE_PROGRAM)
}

/// Which way to a TC002's adbd: its address on the network, or the USB cable.
#[derive(Clone, Debug, PartialEq)]
pub enum Route {
    Ip(String),
    Usb,
}

impl Route {
    /// `usb`, or an address.
    pub fn of(arg: &str) -> Route {
        if arg == "usb" { Route::Usb } else { Route::Ip(arg.to_string()) }
    }
}

impl std::fmt::Display for Route {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Route::Ip(ip) => write!(f, "{ip}"),
            Route::Usb => write!(f, "usb"),
        }
    }
}

/// A TC002's adbd, reached by our own adb client, or (the cable, where an adb server holds it) through that server.
pub enum Panel {
    Ours(Adb),
    Server,
}

impl Panel {
    pub fn open(route: &Route) -> io::Result<Panel> {
        match route {
            Route::Ip(ip) => Adb::tcp(ip).map(Panel::Ours),
            Route::Usb => match usb::connect() {
                Err(e) if e.kind() == io::ErrorKind::ResourceBusy && server::has_panel() => Ok(Panel::Server),
                // Only one program can hold the cable, and a bridge that is connected over it does.
                Err(e) if e.kind() == io::ErrorKind::ResourceBusy && crate::a_run_is_active() => Err(io::Error::new(
                    io::ErrorKind::ResourceBusy,
                    format!(
                        "a running pixbar-bridge (the service?) holds the cable. Stop it first ({}), or reach the panel by its network address",
                        crate::install::stop_service_hint()
                    ),
                )),
                other => other.map(Panel::Ours),
            },
        }
    }

    pub fn shell(&mut self, command: &str) -> io::Result<String> {
        match self {
            Panel::Ours(adb) => adb.shell(command),
            Panel::Server => server::shell(command),
        }
    }

    /// Starts `command` without waiting for it: it is meant to outlive this connection.
    pub fn shell_detached(&mut self, command: &str) -> io::Result<()> {
        match self {
            Panel::Ours(adb) => adb.shell_detached(command),
            Panel::Server => server::shell_detached(command),
        }
    }

    fn push(&mut self, bytes: &[u8], path: &str, mode: u32) -> io::Result<()> {
        match self {
            Panel::Ours(adb) => adb.push(bytes, path, mode),
            Panel::Server => server::push(bytes, path, mode),
        }
    }

    /// The pixbar program's port on the device, as a socket here: what `adb forward` would set up.
    pub fn into_link(self, port: u16) -> io::Result<Link> {
        match self {
            Panel::Ours(adb) => adb.into_stream(&format!("tcp:{port}")).map(Link::Unix),
            Panel::Server => server::stream(&format!("tcp:{port}")).map(Link::Tcp),
        }
    }

    /// It is Ulanzi's app that joins the WiFi, some 20 s into a power-up, and the network stays up once that app
    /// is stopped. Over the cable we can be there sooner than that: stopping the app then would leave the panel
    /// without a network for this power-up, and other hosts without a way to it. So a panel that has a network
    /// set up and is still young is given the time.
    fn wait_for_wifi(&mut self) -> io::Result<()> {
        const LOOK: &str = "cat /proc/uptime; ifconfig wlan0; cat /data/misc/wifi/wpa_supplicant.conf";
        let mut said = false;
        loop {
            let seen = self.shell(LOOK)?;
            let uptime = seen.split_whitespace().next().and_then(|s| s.parse::<f32>().ok()).unwrap_or(f32::MAX);
            if seen.contains("inet addr") || !seen.contains("ssid=") || uptime > 45.0 {
                return Ok(());
            }
            if !std::mem::replace(&mut said, true) {
                eprintln!("the panel has just been switched on; giving Ulanzi's app the time to join the WiFi");
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    /// Changes with every power-up of the panel.
    pub fn boot_id(&mut self) -> io::Result<String> {
        Ok(self.shell("cat /proc/sys/kernel/random/boot_id")?.trim().to_string())
    }

    /// `false`: its user handed this panel back to Ulanzi's app, and it has not been switched off and on since.
    pub fn ours_to_start(&mut self, mac: &str) -> io::Result<bool> {
        match handed_back(mac) {
            Some(then) if then == self.boot_id()? => Ok(false),
            Some(_) => {
                set_handed_back(mac, None);
                Ok(true)
            }
            None => Ok(true),
        }
    }

    pub fn mac(&mut self) -> io::Result<String> {
        Ok(normal_mac(&self.shell("cat /sys/class/net/wlan0/address")?))
    }

    /// Whether the pixbar program is running on it.
    pub fn runs_pixbar(&mut self) -> io::Result<bool> {
        Ok(self.shell("if [ -f /tmp/pixbar-device.pid ] && [ -d /proc/$(cat /tmp/pixbar-device.pid) ]; then echo up; fi")?.contains("up"))
    }

    /// Pushes the device program and starts it in place of Ulanzi's app (or of an older copy of itself).
    /// Ulanzi's app is only stopped for a program that arrived whole, and comes back if ours does not stay up.
    pub fn start(&mut self) -> io::Result<()> {
        self.wait_for_wifi()?;
        self.shell(STOP)?;
        self.push(DEVICE_PROGRAM, "/tmp/pixbar-device.new", 0o100755)?;
        // --daemon detaches, so the shell comes back.
        let said = self.shell("mv /tmp/pixbar-device.new /tmp/pixbar-device; chmod 755 /tmp/pixbar-device; setprop ctl.stop zkswe; /tmp/pixbar-device --daemon")?;
        if !said.trim().is_empty() {
            eprintln!("device: {}", said.trim());
        }
        // It opens the panel, the knob and its port in its first moments, and dies there if it cannot.
        std::thread::sleep(std::time::Duration::from_millis(1200));
        if self.runs_pixbar()? {
            return Ok(());
        }
        let log = self.shell("cat /tmp/pixbar-device.log; setprop ctl.start zkswe").unwrap_or_default();
        Err(io::Error::other(format!("the pixbar program did not stay up on this device (Ulanzi's app is starting again). It said:\n{}", log.trim())))
    }
}

/// Pushes the device program and starts it. Returns the device's MAC, by which it is recognised later.
pub fn start(route: &Route) -> io::Result<String> {
    let once = || {
        let mut panel = Panel::open(route)?;
        let mac = panel.mac()?;
        panel.start()?;
        Ok(mac)
    };
    // The cable changes hands: an adb server takes it the moment another program lets go, and for a second or
    // two its connection to the panel is not there yet. Starting is safe to do again.
    let mut tried = once();
    for _ in 0..2 {
        if *route != Route::Usb || tried.is_ok() || tried.as_ref().is_err_and(|e: &io::Error| e.kind() == io::ErrorKind::ResourceBusy) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(1500));
        tried = once();
    }
    tried
}

/// Stops the pixbar program and starts Ulanzi's app again, to stay until `deploy` or the next power-up.
pub fn stock(route: &Route) -> io::Result<()> {
    let mut panel = Panel::open(route)?;
    let (mac, boot) = (panel.mac()?, panel.boot_id()?);
    panel.shell(&format!("{STOP}; setprop ctl.start zkswe"))?;
    remember(&mac);
    set_handed_back(&mac, Some(&boot));
    Ok(())
}

const OTG: &str = "/sys/bus/platform/devices/soc:usbotg";

/// After a power-up the TC002's USB port answers for a few seconds (measured: from 3.4 s to 6.0 s after the
/// reset), then the kernel turns it into a host port and a PC sees nothing until something on the device turns
/// it back. Caught inside those seconds, a job is left behind that waits for the turn and undoes it; the port
/// is back for good some five seconds later. Returns whether that was needed: `false` means the port is already
/// the device's to keep. (The device has no `sleep`; a ping to itself takes its place.)
pub fn keep_usb_port(panel: &mut Panel) -> io::Result<bool> {
    if panel.shell(&format!("cat {OTG}/otg_role"))?.trim() == "usb_device" {
        return Ok(false);
    }
    // Once per power-up: /tmp is the panel's RAM.
    panel.shell_detached(&format!(
        "if [ ! -f /tmp/pixbar-usb ]; then touch /tmp/pixbar-usb; trap '' HUP; (until [ \"$(cat {OTG}/otg_role)\" = usb_host ]; do ping -c 2 127.0.0.1; done; ping -c 3 127.0.0.1; cat {OTG}/usb_device) >/dev/null 2>&1 & fi"
    ))?;
    Ok(true)
}

/// What the device program has written since it started (it logs to a file in the panel's RAM).
pub fn log(route: &Route) -> io::Result<String> {
    Panel::open(route)?.shell("cat /tmp/pixbar-device.log")
}

pub fn normal_mac(text: &str) -> String {
    text.chars().filter(char::is_ascii_hexdigit).collect::<String>().to_lowercase()
}

/// The stock firmware's beacon is `Ulanzi TC002 <tail>:<mac, 12 hex digits>:<serial>:<flag>`.
pub fn mac_in_stock_beacon(text: &str) -> Option<String> {
    let mac = normal_mac(text.strip_prefix("Ulanzi")?.split(':').nth(1)?);
    (mac.len() == 12).then_some(mac)
}

/// Devices this machine has started the program on before. Only those are taken over unasked when they
/// show up running the stock firmware: a bridge must not grab every TC002 on the network.
fn known_file() -> Option<PathBuf> {
    let base = std::env::var("XDG_CONFIG_HOME").ok().filter(|s| !s.is_empty()).map(PathBuf::from);
    let base = base.or_else(|| Some(PathBuf::from(std::env::var("HOME").ok()?).join(".config")))?;
    Some(base.join("pixbar/devices"))
}

/// One device per line: its MAC, then `stock <boot id>` while its user wants Ulanzi's app on it.
fn known() -> Vec<(String, Option<String>)> {
    let text = known_file().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
    text.lines()
        .filter_map(|l| {
            let mut words = l.split_whitespace();
            let mac = words.next()?.to_string();
            Some((mac, (words.next() == Some("stock")).then(|| words.next().unwrap_or_default().to_string())))
        })
        .collect()
}

pub fn is_known(mac: &str) -> bool {
    known().iter().any(|(m, _)| m == mac)
}

/// Handed back to Ulanzi's app on purpose (`pixbar-bridge stock`, or STOCK FW on the panel): the boot id the
/// panel had then. `run` leaves such a panel alone for as long as it has not been switched off and on.
pub fn handed_back(mac: &str) -> Option<String> {
    known().into_iter().find(|(m, _)| m == mac)?.1
}

/// `Some(boot id)`: handed back during that power-up. `None`: ours again.
pub fn set_handed_back(mac: &str, boot: Option<&str>) {
    let Some(file) = known_file() else { return };
    let mut all = known();
    match all.iter_mut().find(|(m, _)| m == mac) {
        Some(entry) if entry.1.as_deref() != boot => entry.1 = boot.map(str::to_string),
        _ => return,
    }
    let text: String = all.iter().map(|(m, stock)| format!("{m}{}\n", stock.as_ref().map_or(String::new(), |b| format!(" stock {b}")))).collect();
    if let Err(e) = std::fs::write(&file, text) {
        eprintln!("could not update {}: {e}", file.display());
    }
}

pub fn remember(mac: &str) {
    if mac.len() != 12 || is_known(mac) {
        return;
    }
    let Some(file) = known_file() else { return };
    let _ = std::fs::create_dir_all(file.parent().unwrap());
    let mut all = std::fs::read_to_string(&file).unwrap_or_default();
    all.push_str(&format!("{mac}\n"));
    if let Err(e) = std::fs::write(&file, all) {
        eprintln!("could not remember this device in {}: {e}", file.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_stock_beacon_gives_the_mac_the_device_reports_itself() {
        assert_eq!(mac_in_stock_beacon("Ulanzi TC002 a1b2:0011223344FF:SN0123456789ABCDE:true").as_deref(), Some("0011223344ff"));
        assert_eq!(normal_mac("00:11:22:33:44:ff\n"), "0011223344ff");
        assert_eq!(mac_in_stock_beacon("PIXBAR 17002"), None);
        assert_eq!(mac_in_stock_beacon("Ulanzi TC002 junk"), None);
    }

    #[test]
    fn the_bridge_carries_an_arm_program() {
        // ELF, 32-bit, little-endian, machine 40 (ARM): what the TC002 runs.
        assert_eq!(&DEVICE_PROGRAM[..6], b"\x7fELF\x01\x01");
        assert_eq!(u16::from_le_bytes([DEVICE_PROGRAM[18], DEVICE_PROGRAM[19]]), 40);
    }

    #[test]
    fn a_device_handed_back_to_stock_stays_known() {
        let dir = std::env::temp_dir().join(format!("pixbar-known-test-{}", std::process::id()));
        std::env::set_var("XDG_CONFIG_HOME", &dir);
        remember("0011223344ff");
        remember("0011223344aa");
        set_handed_back("0011223344ff", Some("boot-1"));
        assert!(is_known("0011223344ff") && handed_back("0011223344ff").as_deref() == Some("boot-1"));
        assert!(is_known("0011223344aa") && handed_back("0011223344aa").is_none());
        set_handed_back("0011223344ff", None);
        assert!(is_known("0011223344ff") && handed_back("0011223344ff").is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

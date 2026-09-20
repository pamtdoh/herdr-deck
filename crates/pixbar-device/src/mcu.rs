//! The panel MCU's serial link: battery level, whether USB power is present, and power-off.
//!
//!   FF 55 <cmd> <len> <payload[len]> <sum_hi> <sum_lo>
//!
//! The checksum is the 16-bit sum of every byte before it. Replies use the same framing and command byte.
//! `/dev/ttyS1`, 1.5 Mbaud, 8N1. Ulanzi's app owns this port while it runs; with `zkswe` stopped nobody else
//! asks the MCU anything, including whether the battery is nearly flat.

use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

use pixbar_render::Power;

const PORT: &str = "/dev/ttyS1";
const BAUD: u32 = 1_500_000;
const QUERY_USB: u8 = 0x02;
const QUERY_BATTERY: u8 = 0x03;
const POWER_OFF: u8 = 0x10;
const MAX_PAYLOAD: usize = 32;
const POLL: Duration = Duration::from_secs(10);
const REPLY_TIMEOUT: Duration = Duration::from_millis(500);

/// A reading only counts while the MCU keeps answering: silence must never look like a flat or a full battery.
const STALE: Duration = Duration::from_secs(60);
/// The voltage jitters by a few mV from one report to the next; less than this is not news.
const MV_STEP: u16 = 20;

pub fn encode(cmd: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![0xff, 0x55, cmd, payload.len() as u8];
    frame.extend_from_slice(payload);
    let sum = frame.iter().fold(0u16, |s, &b| s.wrapping_add(b as u16));
    frame.extend_from_slice(&sum.to_be_bytes());
    frame
}

/// Takes the next valid frame off the front of `buf` as (command, payload), skipping line noise.
/// `None` when no complete frame is there yet.
pub fn next_frame(buf: &mut Vec<u8>) -> Option<(u8, Vec<u8>)> {
    loop {
        let start = buf.windows(2).position(|w| w == [0xff, 0x55]);
        match start {
            Some(i) => drop(buf.drain(..i)),
            None => {
                // Keep a trailing FF: its 55 may still be on the way.
                let keep = buf.last().is_some_and(|&b| b == 0xff) as usize;
                buf.drain(..buf.len() - keep);
                return None;
            }
        }
        if buf.len() < 4 {
            return None;
        }
        let len = buf[3] as usize;
        if len <= MAX_PAYLOAD && buf.len() < 6 + len {
            return None;
        }
        let good = len <= MAX_PAYLOAD && encode(buf[2], &buf[4..4 + len])[4 + len..] == buf[4 + len..6 + len];
        if good {
            let frame: Vec<u8> = buf.drain(..6 + len).collect();
            return Some((frame[2], frame[4..4 + len].to_vec()));
        }
        // Not a frame after all: step past this header and look again.
        buf.drain(..2);
    }
}

/// Battery reply: charge in percent as the MCU reckons it, then the cell voltage in the vendor's units.
pub fn parse_battery(payload: &[u8]) -> Option<(Option<u8>, u16)> {
    let raw = u16::from_be_bytes([*payload.get(1)?, *payload.get(2)?]);
    let mv = (raw as f32 * 1.323_529_4).min(65_535.0) as u16;
    Some(((payload[0] <= 100).then_some(payload[0]), mv))
}

pub struct Mcu {
    port: File,
    buf: Vec<u8>,
    reading: Power,
    /// What `poll` last handed out.
    reported: Power,
    last_heard: Instant,
    next_poll: Instant,
    /// Command we are waiting on, and until when.
    awaiting: Option<(u8, Instant)>,
}

#[repr(C)]
struct Termios2 {
    iflag: u32,
    oflag: u32,
    cflag: u32,
    lflag: u32,
    line: u8,
    cc: [u8; 19],
    ispeed: u32,
    ospeed: u32,
}

impl Mcu {
    pub fn open() -> io::Result<Mcu> {
        let port = OpenOptions::new().read(true).write(true).custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY).open(PORT)?;
        // termios2 with BOTHER: 1.5 Mbaud is not one of the classic speed constants.
        const TCSETS2: libc::c_ulong = 0x402c_542b;
        const BOTHER: u32 = 0o010000;
        const CS8_CREAD_CLOCAL: u32 = 0o60 | 0o200 | 0o4000;
        let raw =
            Termios2 { iflag: 0, oflag: 0, cflag: BOTHER | CS8_CREAD_CLOCAL, lflag: 0, line: 0, cc: [0; 19], ispeed: BAUD, ospeed: BAUD };
        // SAFETY: TCSETS2 reads one termios2 from the pointer; `raw` is one and outlives the call.
        if unsafe { libc::ioctl(port.as_raw_fd(), TCSETS2 as _, &raw) } != 0 {
            return Err(io::Error::last_os_error());
        }
        let now = Instant::now();
        Ok(Mcu { port, buf: Vec::new(), reading: Power::default(), reported: Power::default(), last_heard: now, next_poll: now, awaiting: None })
    }

    fn send(&mut self, cmd: u8) {
        if self.port.write_all(&encode(cmd, &[])).is_ok() {
            self.awaiting = Some((cmd, Instant::now() + REPLY_TIMEOUT));
        }
    }

    /// Call every frame. Asks for the battery and then the USB state every few seconds, one question at a
    /// time (the MCU also reports the battery unasked, about once a second); returns the reading when it
    /// changed enough to matter.
    pub fn poll(&mut self) -> Option<Power> {
        let mut chunk = [0u8; 64];
        while let Ok(n @ 1..) = self.port.read(&mut chunk) {
            self.buf.extend_from_slice(&chunk[..n]);
        }
        while let Some((cmd, payload)) = next_frame(&mut self.buf) {
            self.last_heard = Instant::now();
            match cmd {
                QUERY_BATTERY => {
                    if let Some((percent, mv)) = parse_battery(&payload) {
                        (self.reading.percent, self.reading.millivolts) = (percent, Some(mv));
                    }
                }
                QUERY_USB => self.reading.on_usb = payload.first().map(|&b| b != 0),
                _ => {}
            }
            if self.awaiting.is_some_and(|(c, _)| c == cmd) {
                self.awaiting = None;
                if cmd == QUERY_BATTERY {
                    self.send(QUERY_USB);
                }
            }
        }
        match self.awaiting {
            Some((_, deadline)) if Instant::now() >= deadline => self.awaiting = None,
            None if Instant::now() >= self.next_poll => {
                self.next_poll = Instant::now() + POLL;
                self.send(QUERY_BATTERY);
            }
            _ => {}
        }
        if self.last_heard.elapsed() > STALE {
            self.reading = Power::default();
        }
        let (new, old) = (self.reading, self.reported);
        let moved = match (new.millivolts, old.millivolts) {
            (Some(a), Some(b)) => a.abs_diff(b) >= MV_STEP,
            (a, b) => a != b,
        };
        (moved || new.percent != old.percent || new.on_usb != old.on_usb).then(|| {
            self.reported = new;
            new
        })
    }

    /// The MCU cuts the power itself; nothing runs after this.
    pub fn power_off(&mut self) {
        let _ = self.port.write_all(&encode(POWER_OFF, &[]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_carry_the_vendors_checksums() {
        assert_eq!(encode(QUERY_BATTERY, &[]), [0xff, 0x55, 0x03, 0x00, 0x01, 0x57]);
        assert_eq!(encode(QUERY_USB, &[]), [0xff, 0x55, 0x02, 0x00, 0x01, 0x56]);
        assert_eq!(encode(0x04, &[0]), [0xff, 0x55, 0x04, 0x01, 0x00, 0x01, 0x59]);
    }

    #[test]
    fn frames_are_found_in_a_noisy_split_stream() {
        let reply = encode(QUERY_BATTERY, &[87, 0x0b, 0xb8]);
        let mut buf = vec![0x00, 0xff, 0x55, 0x03]; // noise, then a header whose frame turns out corrupt
        buf.extend_from_slice(&[0x01, 0x09, 0x00, 0x00]);
        buf.extend_from_slice(&reply[..4]);
        assert_eq!(next_frame(&mut buf), None, "second frame incomplete so far");
        buf.extend_from_slice(&reply[4..]);
        let (cmd, payload) = next_frame(&mut buf).unwrap();
        assert_eq!((cmd, parse_battery(&payload)), (QUERY_BATTERY, Some((Some(87), 3970))));
        assert_eq!(next_frame(&mut buf), None);
        assert!(buf.is_empty());
    }
}

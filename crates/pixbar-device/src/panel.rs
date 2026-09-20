//! The LED panel: a separate MCU fed whole frames over SPI, latched by GPIO 35.
//!
//! Protocol as reverse-engineered by the community (LED-SPI.md in tc002-customisation): pull the latch low,
//! wait 1 ms, write 3072 bytes (16 rows of 52 RGB pixels + 36 pad bytes), wait 1 ms, release the latch.
//! The MCU double-buffers, so a frame only becomes visible when the next one arrives — we simply keep sending.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::FileExt;
use std::thread::sleep;
use std::time::Duration;

use pixbar_render::{Frame, H, W};

const SPIDEV: &str = "/dev/spidev0.0";
const LATCH_GPIO: u32 = 35;
const ROW_BYTES: usize = 192;
pub const FRAME_BYTES: usize = ROW_BYTES * H;

// _IOW('k', nr, type) from linux/spi/spidev.h
const SPI_IOC_WR_MODE: u32 = 0x4001_6b01;
const SPI_IOC_WR_BITS_PER_WORD: u32 = 0x4001_6b03;
const SPI_IOC_WR_MAX_SPEED_HZ: u32 = 0x4004_6b04;

pub struct Panel {
    spi: File,
    latch: File,
}

impl Panel {
    pub fn open() -> io::Result<Panel> {
        let spi = OpenOptions::new().write(true).open(SPIDEV)?;
        let (mode, bits, speed): (u8, u8, u32) = (0, 8, 10_000_000);
        // SAFETY: plain spidev configuration ioctls on a descriptor we own, each reading one value.
        unsafe {
            for (req, ptr) in [
                (SPI_IOC_WR_MODE, &mode as *const u8 as *const libc::c_void),
                (SPI_IOC_WR_BITS_PER_WORD, &bits as *const u8 as *const libc::c_void),
                (SPI_IOC_WR_MAX_SPEED_HZ, &speed as *const u32 as *const libc::c_void),
            ] {
                if libc::ioctl(spi.as_raw_fd(), req as _, ptr) < 0 {
                    return Err(io::Error::last_os_error());
                }
            }
        }

        let dir = format!("/sys/class/gpio/gpio{LATCH_GPIO}");
        if !std::path::Path::new(&dir).exists() {
            std::fs::write("/sys/class/gpio/export", LATCH_GPIO.to_string())?;
        }
        std::fs::write(format!("{dir}/direction"), "out")?;
        let latch = OpenOptions::new().write(true).open(format!("{dir}/value"))?;
        latch.write_at(b"1", 0)?;
        Ok(Panel { spi, latch })
    }

    pub fn show(&mut self, frame: &Frame, brightness: u32) -> io::Result<()> {
        let bytes = pack(frame, brightness);
        self.latch.write_at(b"0", 0)?;
        sleep(Duration::from_millis(1));
        self.spi.write_all(&bytes)?;
        sleep(Duration::from_millis(1));
        self.latch.write_at(b"1", 0).map(drop)
    }

    /// Leaves the panel dark; sent twice because of the MCU's one-frame lag.
    pub fn blank(&mut self) -> io::Result<()> {
        let black = Frame::new();
        self.show(&black, 0)?;
        self.show(&black, 0)
    }
}

/// Brightness (percent) first, then the driver's level curve: 0 stays 0, anything else lands in 50..=255,
/// because the LED driver shows nothing below 50.
pub fn pack(frame: &Frame, brightness: u32) -> [u8; FRAME_BYTES] {
    let level = |v: u8| -> u8 {
        let v = v as u32 * brightness.min(100) / 100;
        if v == 0 {
            0
        } else {
            (50 + (v - 1) * 205 / 254) as u8
        }
    };
    let mut out = [0u8; FRAME_BYTES];
    for (i, px) in frame.pixels().iter().enumerate() {
        let at = (i / W) * ROW_BYTES + (i % W) * 3;
        out[at..at + 3].copy_from_slice(&[level(px.0), level(px.1), level(px.2)]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pixbar_render::Rgb;

    #[test]
    fn rows_are_padded_and_levels_follow_the_curve() {
        let mut f = Frame::new();
        f.set(0, 0, Rgb(255, 1, 0));
        f.set(51, 15, Rgb(0, 0, 255));
        let b = pack(&f, 100);
        assert_eq!(&b[..3], &[255, 50, 0]);
        assert_eq!(&b[15 * 192 + 51 * 3..15 * 192 + 52 * 3], &[0, 0, 255]);
        assert!(b[52 * 3..192].iter().all(|&v| v == 0), "pad bytes stay zero");
        assert_eq!(pack(&f, 50)[0] as u32, 50 + (127 - 1) * 205 / 254);
    }
}

//! The line to the pixbar program, however it is reached: its TCP port over WiFi, or the same port at the far end
//! of an adb stream over the cable (a socket pair of ours, or the adb server's socket).

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

pub enum Link {
    Tcp(TcpStream),
    Unix(UnixStream),
}

impl Link {
    pub fn try_clone(&self) -> io::Result<Link> {
        Ok(match self {
            Link::Tcp(s) => Link::Tcp(s.try_clone()?),
            Link::Unix(s) => Link::Unix(s.try_clone()?),
        })
    }

    /// A link that went away without a word (WiFi dropped, panel out of range) otherwise blocks a write for as
    /// long as the kernel keeps retrying, about a quarter of an hour.
    pub fn set_write_timeout(&self, timeout: Duration) {
        let _ = match self {
            Link::Tcp(s) => s.set_nodelay(true).and_then(|_| s.set_write_timeout(Some(timeout))),
            Link::Unix(s) => s.set_write_timeout(Some(timeout)),
        };
    }
}

impl Link {
    /// Ends the line for every holder of it. Over the cable this is what closes the adb stream; an adbd whose
    /// host just vanishes resets its USB port, and the panel is off the bus for a second or two.
    pub fn shutdown(&self) {
        let _ = match self {
            Link::Tcp(s) => s.shutdown(std::net::Shutdown::Both),
            Link::Unix(s) => s.shutdown(std::net::Shutdown::Both),
        };
    }
}

impl Read for Link {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Link::Tcp(s) => s.read(buf),
            Link::Unix(s) => s.read(buf),
        }
    }
}

impl Write for Link {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Link::Tcp(s) => s.write(buf),
            Link::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

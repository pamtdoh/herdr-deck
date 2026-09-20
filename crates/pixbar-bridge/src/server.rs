//! The TC002 on a USB cable, through an adb server that is already running on this machine.
//!
//! An adb server claims every adb device on the bus and keeps it, so where one runs the cable is not ours to
//! open. Its own protocol (TCP port 5037: a length-prefixed request, `OKAY` or `FAIL`, and after a transport
//! request the socket *is* the device) gives the same three things: a shell, the sync service, and a stream to a
//! TCP port on the device. Nothing here starts a server or needs the adb tool; without a server, usb.rs opens
//! the cable itself.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::adb::{sync_send, sync_verdict};

fn bad(what: impl Into<String>) -> io::Error {
    io::Error::other(what.into())
}

fn connect() -> io::Result<TcpStream> {
    let port = std::env::var("ANDROID_ADB_SERVER_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(5037);
    let stream = TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_millis(500))?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    stream.set_nodelay(true)?;
    Ok(stream)
}

/// Whether an adb server runs here and has the TC002 on the cable.
pub fn has_panel() -> bool {
    panel().is_ok()
}

fn ask(stream: &mut TcpStream, request: &str) -> io::Result<()> {
    stream.write_all(format!("{:04x}{request}", request.len()).as_bytes())?;
    let mut verdict = [0u8; 4];
    stream.read_exact(&mut verdict)?;
    if &verdict == b"OKAY" {
        return Ok(());
    }
    Err(bad(format!("adb server: {}", sized(stream).unwrap_or_else(|_| "refused".into()))))
}

fn sized(stream: &mut TcpStream) -> io::Result<String> {
    let mut len = [0u8; 4];
    stream.read_exact(&mut len)?;
    let len = usize::from_str_radix(&String::from_utf8_lossy(&len), 16).map_err(|_| bad("adb server: bad length"))?;
    let mut text = vec![0u8; len];
    stream.read_exact(&mut text)?;
    Ok(String::from_utf8_lossy(&text).into_owned())
}

/// A connection that the server has switched through to the TC002 on the cable. Its USB serial number is the
/// same on every unit, so it is picked by what it says it is, and addressed by the server's own number for it.
fn panel() -> io::Result<TcpStream> {
    let mut list = connect()?;
    ask(&mut list, "host:devices-l")?;
    let devices = sized(&mut list)?;
    let ids: Vec<&str> = devices
        .lines()
        .filter(|l| l.contains(" usb:") && l.contains("Zkswe") && l.split_whitespace().nth(1) == Some("device"))
        .filter_map(|l| l.split_whitespace().find_map(|w| w.strip_prefix("transport_id:")))
        .collect();
    let id = match ids.as_slice() {
        [one] => one,
        [] => return Err(bad("the adb server on this machine sees no TC002 on USB")),
        _ => return Err(bad("more than one TC002 on USB; unplug all but one")),
    };
    let mut stream = connect()?;
    ask(&mut stream, &format!("host:transport-id:{id}"))?;
    Ok(stream)
}

pub fn shell(command: &str) -> io::Result<String> {
    let mut stream = panel()?;
    ask(&mut stream, &format!("shell:{command}"))?;
    let mut said = Vec::new();
    stream.read_to_end(&mut said)?;
    Ok(String::from_utf8_lossy(&said).into_owned())
}

/// Starts `command` and does not wait for it to end.
pub fn shell_detached(command: &str) -> io::Result<()> {
    let mut stream = panel()?;
    ask(&mut stream, &format!("shell:{command}"))?;
    // Long enough for the shell to have started what it was asked to; it usually says so by closing.
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    let _ = stream.read(&mut [0u8; 64]);
    Ok(())
}

pub fn push(bytes: &[u8], path: &str, mode: u32) -> io::Result<()> {
    let mut stream = panel()?;
    ask(&mut stream, "sync:")?;
    stream.write_all(&sync_send(bytes, path, mode))?;
    let mut answer = [0u8; 8];
    stream.read_exact(&mut answer)?;
    let mut why = Vec::new();
    if &answer[..4] != b"OKAY" {
        let _ = stream.take(u32::from_le_bytes(answer[4..].try_into().unwrap()) as u64).read_to_end(&mut why);
    }
    sync_verdict(&[answer.as_slice(), &why].concat(), path)
}

/// A socket to a TCP port on the device, which is what `adb forward` sets up.
pub fn stream(service: &str) -> io::Result<TcpStream> {
    let mut stream = panel()?;
    ask(&mut stream, service)?;
    stream.set_read_timeout(None)?;
    Ok(stream)
}

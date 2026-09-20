//! Just enough of the adb protocol to start the pixbar program on a TC002 and to talk to it: run a shell command,
//! push a file, and hold a stream to a TCP port on the device open (what `adb forward` does). No adb install
//! needed on the host.
//!
//! The device's adbd asks for no key. Messages are a 24-byte little-endian header (command, arg0, arg1, payload
//! length, payload byte sum, command ^ 0xffffffff) plus payload. Streams: OPEN a service, then WRTE each way, every
//! WRTE answered by OKAY, until CLSE. The same messages go over TCP (port 5555) and over the USB cable (usb.rs);
//! the two differ only in how a message is put on the wire.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const PORT: u16 = 5555;
const CNXN: u32 = u32::from_le_bytes(*b"CNXN");
const AUTH: u32 = u32::from_le_bytes(*b"AUTH");
const OPEN: u32 = u32::from_le_bytes(*b"OPEN");
const OKAY: u32 = u32::from_le_bytes(*b"OKAY");
const WRTE: u32 = u32::from_le_bytes(*b"WRTE");
const CLSE: u32 = u32::from_le_bytes(*b"CLSE");
/// The protocol version that still carries payload checksums, which is what this adbd speaks.
const VERSION: u32 = 0x0100_0000;
/// How long a command waits for the device's next message.
const PATIENCE: Duration = Duration::from_secs(10);
/// Readers give up this often, so that whoever waits on them can look at the clock or at a stop flag.
pub const TICK: Duration = Duration::from_secs(1);

pub fn header(command: u32, arg0: u32, arg1: u32, payload: &[u8]) -> [u8; 24] {
    let sum = payload.iter().fold(0u32, |s, &b| s.wrapping_add(b as u32));
    let mut out = [0u8; 24];
    for (i, word) in [command, arg0, arg1, payload.len() as u32, sum, command ^ 0xffff_ffff].into_iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
    }
    out
}

/// The sending half of a connection to an adbd.
pub trait Sender: Send {
    /// One message. adbd reads the header with a read of exactly its size, so over USB header and payload are
    /// separate transfers; over TCP they may as well be one write.
    fn send(&mut self, header: &[u8; 24], payload: &[u8]) -> io::Result<()>;
}

impl Sender for TcpStream {
    fn send(&mut self, header: &[u8; 24], payload: &[u8]) -> io::Result<()> {
        self.write_all(&[header.as_slice(), payload].concat())
    }
}

pub struct Adb {
    /// Gives up every `TICK` (see `read_full`).
    reader: Box<dyn Read + Send>,
    sender: Box<dyn Sender>,
    /// Largest payload the device takes in one message.
    max: usize,
    next_local: u32,
}

/// One open service on the device.
struct Channel {
    local: u32,
    remote: u32,
    /// Bytes the device sent that nobody has consumed yet.
    inbox: Vec<u8>,
    closed: bool,
}

fn bad(what: impl Into<String>) -> io::Error {
    io::Error::other(what.into())
}

/// Fills `buf` from a reader that gives up every `TICK`: until `deadline`, or for as long as `stop` stays unset.
fn read_full(reader: &mut dyn Read, buf: &mut [u8], deadline: Option<Instant>, stop: Option<&AtomicBool>) -> io::Result<()> {
    let mut filled = 0;
    while filled < buf.len() {
        match reader.read(&mut buf[filled..]) {
            Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
            Ok(n) => filled += n,
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted) => {
                if deadline.is_some_and(|d| Instant::now() > d) {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "the device stopped answering"));
                }
                if stop.is_some_and(|s| s.load(Ordering::SeqCst)) {
                    return Err(bad("stopped"));
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn read_message(reader: &mut dyn Read, deadline: Option<Instant>, stop: Option<&AtomicBool>) -> io::Result<(u32, u32, u32, Vec<u8>)> {
    let mut head = [0u8; 24];
    read_full(reader, &mut head, deadline, stop)?;
    let word = |i: usize| u32::from_le_bytes(head[i * 4..i * 4 + 4].try_into().unwrap());
    if word(0) ^ 0xffff_ffff != word(5) || word(3) > 1 << 20 {
        return Err(bad("not an adb message (the connection is out of step)"));
    }
    let mut payload = vec![0u8; word(3) as usize];
    // Once a header has come, its payload follows at once: no waiting on a stop flag here.
    read_full(reader, &mut payload, Some(Instant::now() + PATIENCE), None)?;
    Ok((word(0), word(1), word(2), payload))
}

impl Adb {
    /// The adbd that the device's firmware serves on its network address.
    pub fn tcp(ip: &str) -> io::Result<Adb> {
        let addr = (ip, PORT).to_socket_addrs()?.next().ok_or_else(|| bad("no address"))?;
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(3))?;
        stream.set_read_timeout(Some(TICK))?;
        stream.set_write_timeout(Some(PATIENCE))?;
        stream.set_nodelay(true)?;
        Adb::over(Box::new(stream.try_clone()?), Box::new(stream))
    }

    /// Says hello over a connection that is already there.
    pub fn over(reader: Box<dyn Read + Send>, sender: Box<dyn Sender>) -> io::Result<Adb> {
        let mut adb = Adb { reader, sender, max: 4096, next_local: 1 };
        adb.send(CNXN, VERSION, 256 * 1024, b"host::pixbar-bridge\0")?;
        match adb.read()? {
            (CNXN, _, max, _) => adb.max = (max as usize).clamp(256, 256 * 1024),
            (AUTH, ..) => return Err(bad("this adbd wants a key (a firmware update has switched authentication on); this bridge has none to offer")),
            (other, ..) => return Err(bad(format!("adbd answered {:?} instead of CNXN", String::from_utf8_lossy(&other.to_le_bytes())))),
        }
        Ok(adb)
    }

    fn send(&mut self, command: u32, arg0: u32, arg1: u32, payload: &[u8]) -> io::Result<()> {
        self.sender.send(&header(command, arg0, arg1, payload), payload)
    }

    fn read(&mut self) -> io::Result<(u32, u32, u32, Vec<u8>)> {
        read_message(&mut *self.reader, Some(Instant::now() + PATIENCE), None)
    }

    /// Reads one message for `ch`: data goes to its inbox (and is acknowledged); returns the command seen.
    fn pump(&mut self, ch: &mut Channel) -> io::Result<u32> {
        let (command, remote, local, payload) = self.read()?;
        if local != ch.local {
            return Ok(0); // a stream we no longer care about; its CLSE in particular must not close this one
        }
        match command {
            WRTE => {
                ch.inbox.extend_from_slice(&payload);
                self.send(OKAY, ch.local, remote, &[])?;
            }
            CLSE => ch.closed = true,
            _ => {}
        }
        Ok(command)
    }

    fn open(&mut self, service: &str) -> io::Result<Channel> {
        let local = self.next_local;
        self.next_local += 1;
        self.send(OPEN, local, 0, format!("{service}\0").as_bytes())?;
        loop {
            match self.read()? {
                (OKAY, remote, l, _) if l == local => return Ok(Channel { local, remote, inbox: Vec::new(), closed: false }),
                (CLSE, _, l, _) if l == local => return Err(bad(format!("device refused {service}"))),
                _ => {}
            }
        }
    }

    fn write(&mut self, ch: &mut Channel, bytes: &[u8]) -> io::Result<()> {
        for chunk in bytes.chunks(self.max) {
            self.send(WRTE, ch.local, ch.remote, chunk)?;
            while self.pump(ch)? != OKAY {
                if ch.closed {
                    return Err(bad("device closed the stream mid-write"));
                }
            }
        }
        Ok(())
    }

    /// Runs `command` in the device's shell and returns what it printed.
    pub fn shell(&mut self, command: &str) -> io::Result<String> {
        let mut ch = self.open(&format!("shell:{command}"))?;
        while !ch.closed {
            self.pump(&mut ch)?;
        }
        self.send(CLSE, ch.local, ch.remote, &[])?;
        Ok(String::from_utf8_lossy(&ch.inbox).into_owned())
    }

    /// Starts `command` in the device's shell and does not wait for it to end (it may outlive the connection).
    pub fn shell_detached(&mut self, command: &str) -> io::Result<()> {
        let mut ch = self.open(&format!("shell:{command}"))?;
        // Long enough for the shell to have started what it was asked to; it usually says so by closing.
        let until = Instant::now() + Duration::from_secs(1);
        while !ch.closed && Instant::now() < until {
            match read_message(&mut *self.reader, Some(until), None) {
                Ok((CLSE, _, local, _)) if local == ch.local => ch.closed = true,
                Ok(_) => {}
                Err(_) => break,
            }
        }
        let _ = self.send(CLSE, ch.local, ch.remote, &[]);
        Ok(())
    }

    /// Writes `bytes` to `path` on the device (adb's sync service: SEND, DATA..., DONE).
    pub fn push(&mut self, bytes: &[u8], path: &str, mode: u32) -> io::Result<()> {
        let mut ch = self.open("sync:")?;
        self.write(&mut ch, &sync_send(bytes, path, mode))?;
        while ch.inbox.len() < 8 && !ch.closed {
            self.pump(&mut ch)?;
        }
        let verdict = sync_verdict(&ch.inbox, path);
        let _ = self.write(&mut ch, &[b"QUIT".as_slice(), &0u32.to_le_bytes()].concat());
        self.send(CLSE, ch.local, ch.remote, &[])?;
        verdict
    }

    /// Opens `service` (`tcp:17002`: a TCP port on the device) and hands back a socket whose other end is that
    /// stream, for as long as the connection lasts. Two threads carry it: what the device sends, and what we send
    /// (each message of ours waits for the device's OKAY, which the first thread sees).
    pub fn into_stream(mut self, service: &str) -> io::Result<UnixStream> {
        let ch = self.open(service)?;
        let (ours, theirs) = UnixStream::pair()?;
        let Adb { mut reader, sender, max, .. } = self;
        let sender = Arc::new(Mutex::new(sender));
        let stop = Arc::new(AtomicBool::new(false));
        let (acked, ack) = mpsc::channel::<()>();
        let (local, remote) = (ch.local, ch.remote);
        let say = move |sender: &Mutex<Box<dyn Sender>>, command: u32, payload: &[u8]| {
            sender.lock().unwrap().send(&header(command, local, remote, payload), payload)
        };

        let (mut to_host, from_device, halt) = (theirs.try_clone()?, sender.clone(), stop.clone());
        std::thread::spawn(move || {
            loop {
                match read_message(&mut *reader, None, Some(&halt)) {
                    Ok((WRTE, _, to, payload)) if to == local => {
                        if to_host.write_all(&payload).is_err() || say(&from_device, OKAY, &[]).is_err() {
                            break;
                        }
                    }
                    Ok((OKAY, _, to, _)) if to == local => {
                        let _ = acked.send(());
                    }
                    Ok((CLSE, _, to, _)) if to == local => break,
                    Ok(_) => {}
                    Err(_) => break,
                }
            }
            halt.store(true, Ordering::SeqCst);
            let _ = to_host.shutdown(Shutdown::Both);
        });

        let mut from_host = theirs;
        std::thread::spawn(move || {
            let mut buf = vec![0u8; max.min(64 * 1024)];
            while !stop.load(Ordering::SeqCst) {
                match from_host.read(&mut buf) {
                    Ok(n) if n > 0 => {
                        if say(&sender, WRTE, &buf[..n]).is_err() || ack.recv_timeout(PATIENCE).is_err() {
                            break;
                        }
                    }
                    _ => break,
                }
            }
            stop.store(true, Ordering::SeqCst);
            let _ = say(&sender, CLSE, &[]);
            let _ = from_host.shutdown(Shutdown::Both);
        });
        Ok(ours)
    }
}

/// A whole file as the sync service wants it.
pub fn sync_send(bytes: &[u8], path: &str, mode: u32) -> Vec<u8> {
    let spec = format!("{path},{mode}");
    let mut request = [b"SEND".as_slice(), &(spec.len() as u32).to_le_bytes(), spec.as_bytes()].concat();
    for chunk in bytes.chunks(64 * 1024) {
        request.extend_from_slice(b"DATA");
        request.extend_from_slice(&(chunk.len() as u32).to_le_bytes());
        request.extend_from_slice(chunk);
    }
    request.extend_from_slice(b"DONE");
    request.extend_from_slice(&0u32.to_le_bytes());
    request
}

/// What the sync service answered to a `sync_send`.
pub fn sync_verdict(answer: &[u8], path: &str) -> io::Result<()> {
    match answer.get(..4) {
        Some(b"OKAY") => Ok(()),
        _ => Err(bad(format!("push to {path} failed: {}", String::from_utf8_lossy(answer.get(8..).unwrap_or_default())))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[test]
    fn messages_carry_length_byte_sum_and_magic() {
        let h = header(OPEN, 1, 0, b"shell:ls\0");
        assert_eq!(&h[..4], b"OPEN");
        let word = |i: usize| u32::from_le_bytes(h[i * 4..i * 4 + 4].try_into().unwrap());
        assert_eq!((word(1), word(2), word(3)), (1, 0, 9));
        assert_eq!(word(4), b"shell:ls\0".iter().map(|&b| b as u32).sum::<u32>());
        assert_eq!(word(5), OPEN ^ 0xffff_ffff);
    }

    /// An adbd of a few lines: a shell that echoes its command, a sync service that keeps what it is sent, and a
    /// `tcp:` service that answers every line in capitals. Before each answer it sends a CLSE for a stream that
    /// is long gone, which used to close whichever stream was being read.
    fn fake_adbd() -> (String, mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (pushed, files) = mpsc::channel();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let (mut s, pushed) = (stream.unwrap(), pushed.clone());
                std::thread::spawn(move || {
                    let mut say = {
                        let mut s = s.try_clone().unwrap();
                        move |command, arg0, arg1, payload: &[u8]| s.send(&header(command, arg0, arg1, payload), payload).unwrap()
                    };
                    let (mut service, mut file) = (String::new(), Vec::new());
                    while let Ok((command, host, _, payload)) = read_message(&mut s, None, None) {
                        match command {
                            CNXN => say(CNXN, VERSION, 4096, b"device::\0"),
                            OPEN => {
                                service = String::from_utf8_lossy(&payload).trim_end_matches('\0').to_string();
                                say(OKAY, 7, host, &[]);
                                say(CLSE, 99, host + 1000, &[]);
                                if let Some(command) = service.strip_prefix("shell:") {
                                    say(WRTE, 7, host, format!("ran {command}\n").as_bytes());
                                    say(CLSE, 7, host, &[]);
                                }
                            }
                            WRTE => {
                                say(OKAY, 7, host, &[]);
                                if service == "sync:" {
                                    file.extend_from_slice(&payload);
                                    if payload.ends_with(b"DONE\0\0\0\0") {
                                        pushed.send(std::mem::take(&mut file)).unwrap();
                                        say(WRTE, 7, host, b"OKAY\0\0\0\0");
                                    }
                                } else {
                                    say(WRTE, 7, host, String::from_utf8_lossy(&payload).to_uppercase().as_bytes());
                                }
                            }
                            _ => {}
                        }
                    }
                });
            }
        });
        (format!("127.0.0.1:{port}"), files)
    }

    fn connect(addr: &str) -> Adb {
        let stream = TcpStream::connect(addr).unwrap();
        stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        Adb::over(Box::new(stream.try_clone().unwrap()), Box::new(stream)).unwrap()
    }

    #[test]
    fn shell_push_and_a_held_stream_against_a_fake_adbd() {
        let (addr, files) = fake_adbd();
        let mut adb = connect(&addr);
        assert_eq!(adb.shell("ls /tmp").unwrap(), "ran ls /tmp\n");
        let program = vec![0xabu8; 10_000]; // more than the 4096 this adbd takes at once
        adb.push(&program, "/tmp/x", 0o100755).unwrap();
        assert_eq!(files.recv().unwrap(), sync_send(&program, "/tmp/x", 0o100755));
        assert_eq!(adb.shell("again").unwrap(), "ran again\n", "a second stream on the same connection");

        let mut link = connect(&addr).into_stream("tcp:17002").unwrap();
        link.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        for line in ["{\"type\":\"hello\"}\n", "{\"type\":\"state\"}\n"] {
            link.write_all(line.as_bytes()).unwrap();
            let mut answer = vec![0u8; line.len()];
            link.read_exact(&mut answer).unwrap();
            assert_eq!(answer, line.to_uppercase().as_bytes());
        }
    }
}

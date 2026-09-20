//! Knob and buttons, read straight from evdev.
//!
//! Buttons are a polled gpio-keys device; the knob is a second device that reports quadrature *state codes*
//! on ABS_X rather than deltas: a detent is the pair 8 then 1 (clockwise) or 13 then 11 (counter-clockwise).

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

use pixbar_render::Input;

const EV_KEY: u16 = 1;
const EV_ABS: u16 = 3;
const KEY_KNOB: u16 = 103;
const KEY_MIDDLE: u16 = 105;
const KEY_RIGHT: u16 = 106;
const KEY_LEFT: u16 = 108;
const CW: (i32, i32) = (8, 1);
const CCW: (i32, i32) = (13, 11);
/// Holding the knob in this long is a long-press (settings) rather than a push.
const LONG_PRESS: Duration = Duration::from_millis(600);
/// Left and right repeat while held, like a key: after a pause, then at the pace the effort knob glides at.
const REPEAT_AFTER: Duration = Duration::from_millis(400);
const REPEAT_EVERY: Duration = Duration::from_millis(150);

pub struct Inputs {
    buttons: File,
    knob: File,
    knob_latch: Option<i32>,
    /// When the knob went down, and whether that hold has already been reported as a long-press.
    knob_down: Option<(Instant, bool)>,
    /// The left or right button that is down, what to send while it stays down, and when next.
    held: Option<(u16, Input, Instant)>,
    pub log: bool,
}

/// Finds `/dev/input/eventN` by device name; the numbers are not stable across firmware builds.
fn node_for(name: &str) -> io::Result<String> {
    let listing = std::fs::read_to_string("/proc/bus/input/devices")?;
    for block in listing.split("\n\n") {
        if !block.contains(&format!("Name=\"{name}\"")) {
            continue;
        }
        let handlers = block.lines().find_map(|l| l.strip_prefix("H: Handlers="));
        if let Some(ev) = handlers.and_then(|h| h.split_whitespace().find(|w| w.starts_with("event"))) {
            return Ok(format!("/dev/input/{ev}"));
        }
    }
    Err(io::Error::new(io::ErrorKind::NotFound, format!("no input device named {name}")))
}

fn open(name: &str) -> io::Result<File> {
    let path = node_for(name)?;
    OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK).open(&path)
}

impl Inputs {
    pub fn open() -> io::Result<Inputs> {
        Ok(Inputs { buttons: open("soc:gpio_keys_1")?, knob: open("knob_key")?, knob_latch: None, knob_down: None, held: None, log: false })
    }

    /// Drains whatever the kernel has queued since the last frame.
    pub fn poll(&mut self) -> Vec<Input> {
        let mut out = Vec::new();
        for ev in drain(&mut self.buttons) {
            if self.log {
                eprintln!("button event type={} code={} value={}", ev.0, ev.1, ev.2);
            }
            match (ev.0, ev.1, ev.2) {
                (EV_KEY, KEY_LEFT, 1) => {
                    out.push(Input::Left);
                    self.held = Some((KEY_LEFT, Input::LeftHeld, Instant::now() + REPEAT_AFTER));
                }
                (EV_KEY, KEY_RIGHT, 1) => {
                    out.push(Input::Right);
                    self.held = Some((KEY_RIGHT, Input::RightHeld, Instant::now() + REPEAT_AFTER));
                }
                (EV_KEY, key, 0) if self.held.is_some_and(|h| h.0 == key) => self.held = None,
                (EV_KEY, KEY_MIDDLE, 1) => out.push(Input::Middle),
                // The knob's push is only known to be short once it is released.
                (EV_KEY, KEY_KNOB, 1) => self.knob_down = Some((Instant::now(), false)),
                (EV_KEY, KEY_KNOB, 0) => {
                    if let Some((_, false)) = self.knob_down.take() {
                        out.push(Input::KnobPush);
                    }
                }
                _ => {}
            }
        }
        if let Some((key, repeat, due)) = self.held {
            if Instant::now() >= due {
                out.push(repeat);
                self.held = Some((key, repeat, due + REPEAT_EVERY));
            }
        }
        if let Some((since, false)) = self.knob_down {
            if since.elapsed() >= LONG_PRESS {
                self.knob_down = Some((since, true));
                out.push(Input::KnobLong);
            }
        }
        for ev in drain(&mut self.knob) {
            if self.log {
                eprintln!("knob event type={} code={} value={}", ev.0, ev.1, ev.2);
            }
            if ev.0 != EV_ABS {
                continue;
            }
            match (self.knob_latch, ev.2) {
                (_, v) if v == CW.0 || v == CCW.0 => self.knob_latch = Some(v),
                (Some(start), v) if (start, v) == CW => {
                    out.push(Input::KnobCw);
                    self.knob_latch = None;
                }
                (Some(start), v) if (start, v) == CCW => {
                    out.push(Input::KnobCcw);
                    self.knob_latch = None;
                }
                _ => {}
            }
        }
        out
    }
}

/// (type, code, value) triples. `struct input_event` on 32-bit ARM is 16 bytes: two 32-bit time fields first.
fn drain(f: &mut File) -> Vec<(u16, u16, i32)> {
    let mut events = Vec::new();
    let mut buf = [0u8; 16 * 32];
    while let Ok(n) = f.read(&mut buf) {
        if n < 16 {
            break;
        }
        for ev in buf[..n].chunks_exact(16) {
            events.push((
                u16::from_ne_bytes([ev[8], ev[9]]),
                u16::from_ne_bytes([ev[10], ev[11]]),
                i32::from_ne_bytes([ev[12], ev[13], ev[14], ev[15]]),
            ));
        }
    }
    events
}

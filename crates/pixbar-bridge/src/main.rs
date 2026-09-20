//! Host side of the pixbar: watches herdr, tells the device what to show, and carries out what it asks for.
//!
//!   pixbar-bridge install [--no-service] [--no-statusline]    put the bridge in ~/.local/bin, run it as a service,
//!                                                             make it Claude Code's status line command
//!   pixbar-bridge uninstall                                   take all of that away again
//!   pixbar-bridge doctor                                      check every link from Claude Code to the panel
//!   pixbar-bridge run [--device IP[:PORT]|usb] [--socket PATH]  normal operation (nothing named: a panel on a USB
//!                                                             cable, else one that announces itself on the network)
//!   pixbar-bridge deploy [IP|usb]                             push the pixbar program (carried inside this binary) and
//!                                                             start it; from then on `run` does that by itself for this device
//!   pixbar-bridge trust [IP|usb]                              take on a panel that another machine started, as it runs
//!   pixbar-bridge stock [IP|usb]                              stop it and give the panel back to Ulanzi's firmware
//!   pixbar-bridge log [IP|usb]                                what the device program has logged
//!   pixbar-bridge shell IP|usb COMMAND                        the panel's root shell
//!   pixbar-bridge find                                        list the TC002s heard on this network (ours or stock firmware)
//!   pixbar-bridge state [--socket PATH]                       print the agent list once, as the device would get it
//!   pixbar-bridge set-effort PANE LEVEL [--socket PATH]       drive one session's picker directly (for testing)
//!   pixbar-bridge set-model PANE MODEL [--socket PATH]          MODEL as the picker calls it: opus, sonnet, ...

mod adb;
mod claude;
mod deploy;
mod doctor;
mod herdr;
mod install;
mod link;
mod picker;
mod server;
mod usb;

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, UdpSocket};

use deploy::{Panel, Route};
use link::Link;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use claude::Sessions;
use herdr::Herdr;
use picker::{Change, Picker};
use pixbar_proto::{decode, encode, Action, AgentState, FromDevice, ToDevice, BEACON_PORT, BEACON_PREFIX, DEFAULT_PORT, PROTO};
use pixbar_render::{Agent, Effort, Model, Status};
use serde_json::Value;

/// Set by SIGTERM / SIGINT. A picker sequence under way backs out with Esc before the process goes.
pub static STOPPING: AtomicBool = AtomicBool::new(false);
/// An intent from the panel is being carried out (keys are going into a Claude Code session).
static EXECUTING: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOPPING.store(true, Ordering::SeqCst);
}

/// Leaves as soon as no key sequence is under way; a stopped bridge must not leave `/model` open in a pane.
fn exit_if_stopping() {
    if STOPPING.load(Ordering::SeqCst) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while EXECUTING.load(Ordering::SeqCst) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        eprintln!("stopping");
        std::process::exit(0);
    }
}

#[derive(Default)]
struct Shared {
    /// pane -> (ultracode tag seen on screen, when we looked)
    ultra: HashMap<String, (bool, Instant)>,
    /// Panes whose effort ring ended before ultracode when we went there: not offered again.
    no_ultra: HashSet<String>,
    last_focused: Option<String>,
}

struct Bridge {
    herdr: Herdr,
    sessions: Sessions,
    shared: Arc<Mutex<Shared>>,
    /// Panes whose session has no status line data, which has been said.
    silent: HashSet<String>,
}

fn num(v: &Value, key: &str) -> u64 {
    v[key].as_u64().unwrap_or(u64::MAX)
}

/// Ultracode is recorded everywhere as plain xhigh; the only live trace is the violet `ultracode` tag that
/// Claude Code draws into the top border of its prompt box.
fn screen_shows_ultracode(screen: &str) -> bool {
    screen.lines().any(|l| l.contains('─') && l.contains(" ultracode "))
}

impl Bridge {
    fn state(&mut self) -> std::io::Result<ToDevice> {
        let snap = self.herdr.snapshot()?;
        let by_id = |list: &str, key: &str| -> HashMap<String, Value> {
            snap[list].as_array().into_iter().flatten().map(|v| (v[key].as_str().unwrap_or("").to_string(), v.clone())).collect()
        };
        let (spaces, tabs) = (by_id("workspaces", "workspace_id"), by_id("tabs", "tab_id"));

        let mut claudes: Vec<&Value> =
            snap["agents"].as_array().into_iter().flatten().filter(|a| a["agent"] == "claude").collect();
        // herdr's sidebar order: workspace, then tab, then pane.
        claudes.sort_by_key(|a| {
            let (w, t) = (&spaces.get(a["workspace_id"].as_str().unwrap_or("")), &tabs.get(a["tab_id"].as_str().unwrap_or("")));
            let pane = a["pane_id"].as_str().and_then(|p| p.rsplit('p').next()?.parse::<u64>().ok()).unwrap_or(0);
            (w.map_or(u64::MAX, |w| num(w, "number")), t.map_or(u64::MAX, |t| num(t, "number")), pane)
        });

        let focused_pane = snap["focused_pane_id"].as_str().unwrap_or("");
        let mut shared = self.shared.lock().unwrap();
        let mut agents = Vec::new();
        for a in claudes {
            let id = a["pane_id"].as_str().unwrap_or("").to_string();
            let label = |v: Option<&Value>| v.and_then(|v| v["label"].as_str()).unwrap_or("?").to_string();
            let status = match a["agent_status"].as_str() {
                Some("idle") => Status::Idle,
                Some("working") => Status::Working,
                Some("blocked") => Status::Blocked,
                Some("done") => Status::Done,
                _ => Status::Unknown,
            };
            let info = a["agent_session"]["value"].as_str().and_then(|s| self.sessions.info(s));
            let (model, mut effort, ctx_used, ctx_window) = match &info {
                Some(i) => (i.model, i.effort, i.ctx_used, i.ctx_window),
                None => (Model::new(""), Effort::High, 0, 200_000),
            };
            if info.is_some() {
                self.silent.remove(&id);
            } else if self.silent.insert(id.clone()) {
                eprintln!("{id}: nothing from this session's status line yet; it has to call `pixbar-bridge statusline` (README)");
            }

            // Look at the screen for the ultracode tag: often for the focused agent, rarely for the rest.
            if effort == Effort::XHigh || effort == Effort::Ultra {
                let max_age = Duration::from_secs(if id == focused_pane { 2 } else { 15 });
                let seen = match shared.ultra.get(&id) {
                    Some((seen, at)) if at.elapsed() < max_age => *seen,
                    _ => {
                        let seen = self.herdr.screen(&id).map(|s| screen_shows_ultracode(&s)).unwrap_or(false);
                        shared.ultra.insert(id.clone(), (seen, Instant::now()));
                        seen
                    }
                };
                effort = if seen { Effort::Ultra } else { Effort::XHigh };
            }

            let next_model = info.as_ref().and_then(|i| self.sessions.next_model(i.model));
            let ultra_ok = !shared.no_ultra.contains(&id);
            agents.push(AgentState {
                id,
                agent: Agent {
                    reported: info.is_some(),
                    has_effort: info.as_ref().is_some_and(|i| i.has_effort),
                    next_model,
                    space: label(spaces.get(a["workspace_id"].as_str().unwrap_or(""))),
                    tab: label(tabs.get(a["tab_id"].as_str().unwrap_or(""))),
                    status,
                    model,
                    effort,
                    ctx_used,
                    ctx_window,
                    ultra_ok,
                    dir: a["cwd"].as_str().and_then(|p| p.trim_end_matches('/').rsplit('/').next()).unwrap_or("").to_string(),
                    title: a["terminal_title_stripped"].as_str().unwrap_or("").to_string(),
                    session: info.as_ref().map(|i| i.session.clone()).unwrap_or_default(),
                    cost_cents: info.as_ref().map_or(0, |i| i.cost_cents),
                    // The account's, not the session's: filled in below, once every session has been heard.
                    limit_5h: None,
                    limit_7d: None,
                    fresh: info.as_ref().is_some_and(|i| i.fresh),
                },
            });
        }

        let [limit_5h, limit_7d] = self.sessions.limits(SystemTime::now());
        for a in &mut agents {
            (a.agent.limit_5h, a.agent.limit_7d) = (limit_5h, limit_7d);
        }

        // Focus on a plain shell pane keeps the panel on the agent you were last looking at.
        if agents.iter().any(|a| a.id == focused_pane) {
            shared.last_focused = Some(focused_pane.to_string());
        }
        let focused = shared.last_focused.clone().filter(|f| agents.iter().any(|a| &a.id == f));
        Ok(ToDevice::State { agents, focused })
    }
}

fn execute(herdr: &Herdr, shared: &Mutex<Shared>, pane: &str, action: Action) {
    EXECUTING.store(true, Ordering::SeqCst);
    let result = match action {
        Action::Focus => herdr.focus(pane).map_err(|e| e.to_string()),
        Action::SetEffort { effort } => Picker { herdr, pane }.apply(Change::Effort(effort)),
        Action::SetModel { model } => Picker { herdr, pane }.apply(Change::Model(model)),
    };
    match result {
        Ok(()) => {
            eprintln!("{pane}: {action:?} done");
            // Claude Code reruns its status line for the change, so the new value arrives the usual way; until
            // then the panel holds what it asked for. Only the ultracode tag needs a fresh look at the screen.
            shared.lock().unwrap().ultra.remove(pane);
        }
        // The device shows its change optimistically and snaps back by itself when we never confirm it.
        Err(e) => {
            eprintln!("{pane}: {action:?} failed: {e}");
            if matches!(action, Action::SetEffort { effort: Effort::Ultra }) && e.starts_with(picker::SHORT_RING) {
                eprintln!("{pane}: no ultracode stop in this session; the panel will not offer it there again");
                shared.lock().unwrap().no_ultra.insert(pane.to_string());
            }
        }
    }
    EXECUTING.store(false, Ordering::SeqCst);
}

/// Logs `what` unless it was the last thing said: started at login, the bridge may wait hours for herdr, retrying
/// every second.
fn say_once(said: &mut Option<String>, what: String) {
    if said.as_ref() != Some(&what) {
        eprintln!("{what}");
        *said = Some(what);
    }
}

/// Ulanzi's firmware announces itself on this port the way ours does on `BEACON_PORT`.
const STOCK_BEACON_PORT: u16 = 55555;

/// A TC002 heard on the network.
#[derive(Clone, Debug, PartialEq)]
enum Heard {
    /// Running the pixbar program, at this `ip:port`, and the MAC it says it has (programs before protocol 2 said none).
    Pixbar(String, Option<String>),
    /// Running Ulanzi's firmware.
    Stock { ip: String, mac: String },
    /// Not heard but seen: a TC002 has come up on a USB cable while we were listening.
    Usb,
}

/// A UDP socket on `port` that does not keep the port to itself: the service listens nearly all the time, and
/// `find` or `deploy` next to it must hear the same announcements. (std has no way to set these options
/// before the bind.)
fn beacon_socket(port: u16) -> std::io::Result<UdpSocket> {
    use std::os::fd::FromRawFd;
    // SAFETY: plain socket calls on a descriptor this function owns; it is closed on every error path.
    unsafe {
        let fd = libc::socket(libc::AF_INET, libc::SOCK_DGRAM, 0);
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        let on: libc::c_int = 1;
        let size = std::mem::size_of_val(&on) as libc::socklen_t;
        let mut addr: libc::sockaddr_in = std::mem::zeroed();
        addr.sin_family = libc::AF_INET as libc::sa_family_t;
        #[cfg(target_os = "macos")]
        {
            addr.sin_len = std::mem::size_of_val(&addr) as u8;
        }
        addr.sin_port = port.to_be();
        let failed = [libc::SO_REUSEADDR, libc::SO_REUSEPORT]
            .iter()
            .any(|&opt| libc::setsockopt(fd, libc::SOL_SOCKET, opt, &on as *const _ as *const libc::c_void, size) != 0)
            || libc::bind(fd, &addr as *const _ as *const libc::sockaddr, std::mem::size_of_val(&addr) as libc::socklen_t) != 0;
        if failed {
            let e = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(e);
        }
        Ok(UdpSocket::from_raw_fd(fd))
    }
}

/// Listens for both beacons for up to `wait`. `first`: return as soon as one device is heard.
fn listen(wait: Duration, first: bool) -> Vec<Heard> {
    let socks: Vec<UdpSocket> = [BEACON_PORT, STOCK_BEACON_PORT]
        .into_iter()
        .filter_map(|port| {
            let s = beacon_socket(port).map_err(|e| eprintln!("udp/{port}: {e}")).ok()?;
            s.set_read_timeout(Some(Duration::from_millis(200))).ok()?;
            Some(s)
        })
        .collect();
    let (mut heard, mut buf, until) = (Vec::new(), [0u8; 256], Instant::now() + wait);
    if socks.is_empty() {
        thread::sleep(Duration::from_secs(2)); // callers retry; do not spin
    }
    let cable_then = usb::present();
    while Instant::now() < until && !socks.is_empty() && (!first || heard.is_empty()) && !STOPPING.load(Ordering::SeqCst) {
        // Looked for between the announcements: after a power-up the cable answers for some two seconds.
        if first && !cable_then && usb::present() {
            return vec![Heard::Usb];
        }
        for sock in &socks {
            let Ok((n, from)) = sock.recv_from(&mut buf) else { continue };
            let text = String::from_utf8_lossy(&buf[..n]);
            let mut ours = text.strip_prefix(BEACON_PREFIX).unwrap_or_default().split_whitespace();
            let found = match ours.next().and_then(|p| p.parse::<u16>().ok()) {
                Some(port) => Some(Heard::Pixbar(format!("{}:{port}", from.ip()), ours.next().map(deploy::normal_mac).filter(|m| m.len() == 12))),
                None => deploy::mac_in_stock_beacon(&text).map(|mac| Heard::Stock { ip: from.ip().to_string(), mac }),
            };
            heard.extend(found.filter(|f| !heard.contains(f)));
        }
    }
    heard
}

/// Lists every TC002 heard within a few seconds: `IP pixbar|stock`. A network that blocks broadcasts between
/// clients shows nothing.
fn find() {
    let heard = listen(Duration::from_secs(4), false);
    for h in &heard {
        match h {
            Heard::Pixbar(addr, mac) => {
                let known = mac.as_deref().is_some_and(deploy::is_known);
                println!("{} pixbar{}", addr.split(':').next().unwrap_or(addr), if known { "" } else { " (not one this machine has started: `pixbar-bridge trust` takes it on)" });
            }
            Heard::Stock { ip, mac } => println!("{ip} stock{}", if deploy::is_known(mac) { " (known: `run` starts it by itself)" } else { "" }),
            Heard::Usb => {}
        }
    }
    if heard.is_empty() {
        eprintln!("no TC002 heard in 4 s (other subnet, or the network blocks broadcasts between clients)");
        std::process::exit(1);
    }
}

/// `deploy [IP]` / `stock [IP]`: without an address, the one device heard on the network.
fn one_device(named: Option<&str>) -> Route {
    if let Some(named) = named {
        return Route::of(named);
    }
    // A cable is the more deliberate of the two.
    if usb::present() {
        return Route::Usb;
    }
    let mut ips: Vec<String> = listen(Duration::from_secs(4), false)
        .into_iter()
        .map(|h| match h {
            Heard::Pixbar(addr, _) => addr.split(':').next().unwrap_or_default().to_string(),
            Heard::Stock { ip, .. } => ip,
            Heard::Usb => String::new(),
        })
        .collect();
    ips.dedup();
    match ips.as_slice() {
        [ip] => Route::Ip(ip.clone()),
        [] => fail("no TC002 on a USB cable and none heard on this network; pass its address".into()),
        many => fail(format!("several TC002s heard ({}); pass the address of the one you mean", many.join(", "))),
    }
}

/// What `run` does by itself for a panel on Ulanzi's firmware: starts the program, unless its user handed the
/// panel back on purpose during this power-up. `told`: said once, not every time the panel is heard.
fn start_device(route: &Route, told: &mut Option<String>) -> bool {
    let started = Panel::open(route).and_then(|mut panel| {
        let mac = panel.mac()?;
        if !panel.ours_to_start(&mac)? {
            say_once(told, format!("the TC002 at {route} was handed back to its stock firmware; `pixbar-bridge deploy {route}` or switching it off and on takes it over again"));
            return Ok(false);
        }
        eprintln!("starting the pixbar program on {route}");
        panel.start()?;
        deploy::remember(&mac);
        Ok(true)
    });
    started.unwrap_or_else(|e| {
        eprintln!("could not start it on {route}: {e}");
        false
    })
}

/// The pixbar program at the far end of the cable, started first if it is not running. `None`: the port was caught
/// in the seconds it answers after a power-up; it goes away now and is back for good a few seconds later.
fn usb_link(told: &mut Option<String>) -> std::io::Result<Option<Link>> {
    let mut panel = Panel::open(&Route::Usb)?;
    if deploy::keep_usb_port(&mut panel)? {
        say_once(told, "the TC002 on the cable has just been switched on; holding on to its USB port".into());
        return Ok(None);
    }
    if !panel.runs_pixbar()? {
        let mac = panel.mac()?;
        if !panel.ours_to_start(&mac)? {
            say_once(told, "the TC002 on the cable was handed back to its stock firmware; `pixbar-bridge deploy usb` or switching it off and on takes it over again".into());
            return Ok(None);
        }
        eprintln!("starting the pixbar program over the cable");
        panel.start()?;
        deploy::remember(&mac);
    }
    panel.into_link(DEFAULT_PORT).map(Some)
}

/// `0.1.0 (device program 71e8…)`: which bridge this is, and which device program it carries.
fn version() -> String {
    format!("{} (device program {})", env!("CARGO_PKG_VERSION"), deploy::embedded_build())
}

const USAGE: &str = "usage: pixbar-bridge install [--no-service] [--no-statusline] | uninstall | doctor
       pixbar-bridge run|deploy [IP|usb]|trust [IP|usb]|stock [IP|usb]|log [IP|usb]|shell IP|usb COMMAND|find|state|statusline|set-effort PANE LEVEL|set-model PANE MODEL  [--device IP[:PORT]|usb] [--socket PATH]
       pixbar-bridge --version";

/// One `run` per device argument: a second one would connect to the same panel, which lists every agent twice.
/// The lock goes with the process, however it ends.
fn only_one_run(device: Option<&str>) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let dir = var("XDG_RUNTIME_DIR").unwrap_or_else(std::env::temp_dir);
    let which: String = device.unwrap_or("auto").chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    let path = dir.join(format!("pixbar-bridge-{which}.lock"));
    let file = match std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(&path) {
        Ok(file) => file,
        Err(e) => fail(format!("{}: {e}", path.display())),
    };
    // SAFETY: flock on a descriptor that `file` keeps open.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let pid = std::fs::read_to_string(&path).unwrap_or_default();
        fail(format!("another `pixbar-bridge run` is already serving this panel (pid {}); stop it, or the service, first", pid.trim()));
    }
    let _ = file.set_len(0);
    let _ = (&file).write_all(std::process::id().to_string().as_bytes());
    file
}

/// Whether some `pixbar-bridge run` is going on this machine (it holds its lock for as long as it lives).
fn a_run_is_active() -> bool {
    use std::os::fd::AsRawFd;
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()).map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    std::fs::read_dir(dir).into_iter().flatten().flatten().any(|entry| {
        let name = entry.file_name();
        let ours = name.to_string_lossy().starts_with("pixbar-bridge-") && name.to_string_lossy().ends_with(".lock");
        // SAFETY: flock on a descriptor that `file` keeps open; a lock we do get is given up when `file` closes.
        ours && std::fs::File::open(entry.path()).is_ok_and(|file| unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0)
    })
}

fn run(herdr: Herdr, device: Option<String>) {
    let _lock = only_one_run(device.as_deref());
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
    }
    eprintln!("pixbar-bridge {}", version());
    let shared = Arc::new(Mutex::new(Shared::default()));
    let dirty = Arc::new(AtomicBool::new(true));
    // Set from the panel's settings screen; herdr events are pushed at once regardless.
    let refresh_ms = Arc::new(AtomicU64::new(1000));
    let mut bridge = Bridge { herdr: herdr.clone(), sessions: Sessions::default(), shared: shared.clone(), silent: HashSet::new() };
    claude::forget_old();

    {
        let (herdr, dirty) = (herdr.clone(), dirty.clone());
        thread::spawn(move || {
            let mut said = None;
            loop {
                match herdr.subscribe(|| dirty.store(true, Ordering::SeqCst)) {
                    Err(e) => say_once(&mut said, format!("herdr events at {}: {e}", herdr.socket().display())),
                    // It was connected until now, so the next failure is news again.
                    Ok(()) => said = None,
                }
                thread::sleep(Duration::from_secs(2));
            }
        });
    }

    // A program that dies at once must not be pushed again every two seconds.
    const START_EVERY: Duration = Duration::from_secs(30);
    let mut told_handed_back = None;
    // A cable that cannot be used (no permission, held by another program) is said once and tried again now and then.
    let (mut told_usb, mut usb_failed): (Option<String>, Option<Instant>) = (None, None);
    // Kept across connections: the panel drops a host that sends no state for 10 s, so while herdr is down the
    // bridge reconnects over and over.
    let (mut last_start, mut told_unknown, mut herdr_down) = (None, None, None);
    // macOS has no /etc/hostname.
    let host = std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::process::Command::new("hostname").arg("-s").output().ok().map(|o| String::from_utf8_lossy(&o.stdout).into_owned()))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "host".into());
    let (mut searching_since, mut told_silence) = (Instant::now(), None);
    loop {
        exit_if_stopping();
        let by_cable = device.as_deref() == Some("usb");
        // The panel we mean to reach, where it was found by its announcement.
        let mut expected: Option<String> = None;
        // The cable first: it is the deliberate one, and after a power-up its port answers for seconds only.
        let mut cable = None;
        if (by_cable || device.is_none()) && usb::present() && usb_failed.is_none_or(|t| t.elapsed() > Duration::from_secs(3)) {
            match usb_link(&mut told_handed_back) {
                Ok(link) => {
                    (cable, usb_failed, told_usb) = (link, None, None);
                    if cable.is_none() {
                        thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                }
                Err(e) => {
                    say_once(&mut told_usb, format!("the TC002 on the cable: {e}"));
                    usb_failed = Some(Instant::now());
                }
            }
        }
        let (mut stream, addr) = if let Some(link) = cable {
            (link, "the far end of the cable".to_string())
        } else if by_cable {
            say_once(&mut told_silence, "waiting for a TC002 on a USB cable".into());
            thread::sleep(Duration::from_millis(200));
            continue;
        } else {
            let addr = match device.clone() {
                Some(d) if d.contains(':') => d,
                Some(d) => format!("{d}:{DEFAULT_PORT}"),
                // Nobody named a device: take a running pixbar, or start one this machine has started before.
                None => match listen(Duration::from_secs(5), true).into_iter().next() {
                    // Anything on the network can claim to be a pixbar, and would be sent every session's name, cost
                    // and usage, and could change focus, effort and model. Only panels this machine has started
                    // (or was told to trust) are taken on by themselves; the panel says again who it is once connected.
                    Some(Heard::Pixbar(addr, Some(mac))) if deploy::is_known(&mac) => {
                        expected = Some(mac);
                        addr
                    }
                    Some(Heard::Pixbar(addr, _)) => {
                        let ip = addr.split(':').next().unwrap_or(&addr).to_string();
                        say_once(&mut told_unknown, format!("a pixbar at {ip} that this machine has not started: `pixbar-bridge trust {ip}` takes it on, `run --device {ip}` uses it this once (a panel running an older pixbar program needs `pixbar-bridge deploy {ip}`)"));
                        continue;
                    }
                    Some(Heard::Usb) => continue,
                    Some(Heard::Stock { ip, mac }) if deploy::is_known(&mac) => {
                        // Every 10 s: a panel that was handed back is asked whether it has been switched off and on since.
                        if last_start.is_none_or(|t: Instant| t.elapsed() > Duration::from_secs(10)) {
                            last_start = Some(Instant::now());
                            // One that does not stay up must not be pushed again every few seconds.
                            if !start_device(&Route::Ip(ip), &mut told_handed_back) {
                                last_start = Some(Instant::now() + START_EVERY);
                            }
                        }
                        continue;
                    }
                    Some(Heard::Stock { ip, .. }) => {
                        say_once(&mut told_unknown, format!("a TC002 on stock firmware at {ip} is not one of ours; `pixbar-bridge deploy {ip}` adopts it"));
                        continue;
                    }
                    None => {
                        if searching_since.elapsed() > Duration::from_secs(15) {
                            say_once(&mut told_silence, "no TC002 heard yet. It announces itself by UDP broadcast (ports 17003 and 55555): is it switched on and on this network, does a firewall here drop incoming UDP, does the network keep its clients apart (guest WiFi)? `--device IP` needs no broadcast, and a USB cable needs no network".to_string());
                        }
                        continue;
                    }
                },
            };
            match TcpStream::connect(&addr) {
                Ok(s) => (Link::Tcp(s), addr),
                Err(e) => {
                    eprintln!("connect {addr}: {e}");
                    // A device that was named is ours to start: after a power-up it runs the stock firmware.
                    let named = device.as_deref().map(|d| d.split(':').next().unwrap_or(d).to_string());
                    if let Some(ip) = named.filter(|_| last_start.is_none_or(|t| t.elapsed() > START_EVERY)) {
                        last_start = Some(Instant::now());
                        start_device(&Route::Ip(ip), &mut told_handed_back);
                    }
                    thread::sleep(Duration::from_secs(2));
                    continue;
                }
            }
        };
        stream.set_write_timeout(Duration::from_secs(5));
        told_silence = None;
        eprintln!("connected to pixbar at {addr}");
        let _ = stream.write_all(encode(&ToDevice::Hello { host: host.clone(), proto: PROTO }).as_bytes());

        // Reader: one intent at a time, in order; a picker sequence takes a few hundred ms.
        let alive = Arc::new(AtomicBool::new(true));
        // Whether the panel has said who it is. Only asked of one that was found by its announcement.
        let named = Arc::new(AtomicBool::new(expected.is_none()));
        {
            let (reader, herdr, shared, dirty, alive, refresh_ms, expected, named) = (
                stream.try_clone().expect("clone socket"),
                herdr.clone(),
                shared.clone(),
                dirty.clone(),
                alive.clone(),
                refresh_ms.clone(),
                expected.clone(),
                named.clone(),
            );
            thread::spawn(move || {
                let (mut panel, mut booted) = (String::new(), String::new());
                for line in BufReader::new(reader).lines().map_while(Result::ok) {
                    match decode::<FromDevice>(&line) {
                        // Nothing is done for whoever answered an announcement until it has said it is that panel.
                        Ok(FromDevice::Intent { .. }) if !named.load(Ordering::SeqCst) => break,
                        Ok(FromDevice::Intent { id, action }) => {
                            execute(&herdr, &shared, &id, action);
                            dirty.store(true, Ordering::SeqCst);
                        }
                        Ok(FromDevice::Stock) => {
                            eprintln!("STOCK FW was picked on the panel; leaving it to Ulanzi's app until it is deployed to or power-cycled");
                            deploy::set_handed_back(&panel, Some(&booted));
                        }
                        Ok(FromDevice::Hello { version, build, mac, boot, proto }) => {
                            if expected.as_ref().is_some_and(|e| *e != mac) {
                                eprintln!("the pixbar that answered is not the one that was announced ({mac:?}); leaving it");
                                break;
                            }
                            named.store(true, Ordering::SeqCst);
                            (panel, booted) = (mac, boot);
                            eprintln!("pixbar program {version} (build {build})");
                            if proto != PROTO {
                                eprintln!("it speaks protocol {proto}, this bridge {PROTO}: what one does not know of the other is left out; `pixbar-bridge deploy` brings them together");
                            }
                            // Not updated unasked: two hosts carrying different builds would push over each other forever.
                            if !build.is_empty() && deploy::embedded_build() != build {
                                eprintln!("this bridge carries a different build; `pixbar-bridge deploy` puts it on the device");
                            }
                        }
                        Ok(FromDevice::Config { refresh_ms: ms }) => {
                            eprintln!("refresh every {ms} ms");
                            // Whatever the panel says, stay well inside its 10 s host timeout.
                            refresh_ms.store((ms as u64).clamp(100, 5000), Ordering::SeqCst);
                        }
                        Err(e) => eprintln!("bad line from device: {e}"),
                    }
                }
                alive.store(false, Ordering::SeqCst);
            });
        }
        // Where the panel was found by its announcement, nothing about the sessions is sent before it has said who
        // it is; it says so at once on connect.
        let asked = Instant::now();
        while !named.load(Ordering::SeqCst) && alive.load(Ordering::SeqCst) && asked.elapsed() < Duration::from_secs(3) {
            thread::sleep(Duration::from_millis(20));
        }
        if !named.load(Ordering::SeqCst) {
            eprintln!("{addr} did not say which panel it is; leaving it");
            stream.shutdown();
            thread::sleep(Duration::from_secs(5));
            continue;
        }

        let (mut last_said, mut last_at) = (None, Instant::now());
        while alive.load(Ordering::SeqCst) {
            if STOPPING.load(Ordering::SeqCst) {
                stream.shutdown();
                thread::sleep(Duration::from_millis(300));
            }
            exit_if_stopping();
            // Events make this immediate; the slow poll catches what has no event (transcripts, the screen).
            let changed = dirty.swap(false, Ordering::SeqCst);
            if changed || last_at.elapsed() >= Duration::from_millis(refresh_ms.load(Ordering::SeqCst)) {
                match bridge.state() {
                    Ok(state) => {
                        if herdr_down.take().is_some() {
                            eprintln!("herdr is back");
                        }
                        // Resent even when unchanged: it doubles as the keepalive.
                        if stream.write_all(encode(&state).as_bytes()).is_err() {
                            break;
                        }
                        // The state itself changes with every reply and every minute of a usage window.
                        if let ToDevice::State { agents, focused } = state {
                            say_once(&mut last_said, format!("state: {} agents, focused {focused:?}", agents.len()));
                        }
                        last_at = Instant::now();
                    }
                    Err(e) => {
                        say_once(&mut herdr_down, format!("herdr at {}: {e}", bridge.herdr.socket().display()));
                        // Still here, with nothing to show: the panel drops a host it has not heard from for 10 s.
                        if stream.write_all(encode(&ToDevice::State { agents: Vec::new(), focused: None }).as_bytes()).is_err() {
                            break;
                        }
                        last_at = Instant::now();
                        thread::sleep(Duration::from_secs(1));
                    }
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
        eprintln!("pixbar link lost, reconnecting");
        searching_since = Instant::now();
        thread::sleep(Duration::from_secs(1));
    }
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut take = |flag: &str| -> Option<String> {
        let i = args.iter().position(|a| a == flag)?;
        args.remove(i);
        (i < args.len()).then(|| args.remove(i))
    };
    let (socket, device) = (take("--socket"), take("--device"));
    let herdr = Herdr::new(socket);
    let shared = Mutex::new(Shared::default());

    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["run"] => run(herdr, device),
        // Called by Claude Code's status line command with its input on stdin. Prints nothing: used as the whole
        // command it leaves the line empty, and inside a script it stays out of that script's output.
        ["statusline"] => {
            let mut input = String::new();
            if let Err(e) = std::io::stdin().read_to_string(&mut input).and_then(|_| claude::keep(&input)) {
                eprintln!("statusline: {e}");
            }
            // After `install`, this is Claude Code's whole status line command: the one that was there before
            // still draws the line. Whatever went wrong above must not cost the user their status line.
            install::pass_on(&input);
        }
        ["install", flags @ ..] if flags.iter().all(|f| ["--no-service", "--no-statusline"].contains(f)) => {
            if let Err(e) = install::install(&Herdr::default_socket(), !flags.contains(&"--no-service"), !flags.contains(&"--no-statusline")) {
                fail(format!("install: {e}"));
            }
        }
        // A panel that another machine started: taken on as one of ours without starting the program again.
        ["trust", of @ ..] if of.len() <= 1 => {
            let route = one_device(of.first().copied());
            match Panel::open(&route).and_then(|mut panel| panel.mac()) {
                Ok(mac) if mac.len() == 12 => {
                    deploy::remember(&mac);
                    eprintln!("{route} ({mac}) is one of this machine's panels now: `run` connects to it, and starts the program on it after a power-up");
                }
                Ok(_) => fail(format!("{route}: it did not say its MAC")),
                Err(e) => fail(format!("{route}: {e}")),
            }
        }
        ["log", ip @ ..] if ip.len() <= 1 => match deploy::log(&one_device(ip.first().copied())) {
            Ok(text) => print!("{text}"),
            Err(e) => fail(format!("log: {e}")),
        },
        // The device's root shell, for looking around without the adb tool. It has no sleep, grep, head or tail.
        ["shell", to, command] => match Panel::open(&Route::of(to)).and_then(|mut panel| panel.shell(command)) {
            Ok(said) => print!("{said}"),
            Err(e) => fail(format!("shell: {e}")),
        },
        ["doctor"] => {
            if !doctor::run(&herdr) {
                std::process::exit(1);
            }
        }
        ["uninstall"] => {
            if let Err(e) = install::uninstall() {
                fail(format!("uninstall: {e}"));
            }
        }
        ["find"] => find(),
        ["deploy", to @ ..] if to.len() <= 1 => match deploy::start(&one_device(to.first().copied())) {
            Ok(mac) => {
                deploy::remember(&mac);
                deploy::set_handed_back(&mac, None);
            }
            Err(e) => fail(format!("could not start it: {e}")),
        },
        ["stock", of @ ..] if of.len() <= 1 => {
            let route = one_device(of.first().copied());
            // Kept with the panel's boot id: otherwise a running `run` (the service) would see the stock firmware
            // and start the program again.
            match deploy::stock(&route) {
                Ok(()) => eprintln!("stock firmware starting on {route}; it stays until `pixbar-bridge deploy` or until the panel is switched off and on"),
                Err(e) => fail(format!("{route}: {e}")),
            }
        }
        ["state"] => {
            let mut b = Bridge { herdr, sessions: Sessions::default(), shared: Arc::new(shared), silent: HashSet::new() };
            match b.state() {
                Ok(state) => print!("{}", encode(&state)),
                Err(e) => fail(format!("herdr at {}: {e}", b.herdr.socket().display())),
            }
        }
        ["set-effort", pane, level] => {
            let Ok(effort) = serde_json::from_value::<Effort>(Value::String(level.to_string())) else { fail("LEVEL is one of low|med|high|x_high|max|ultra".into()) };
            execute(&herdr, &shared, pane, Action::SetEffort { effort });
        }
        ["set-model", pane, model] => {
            execute(&herdr, &shared, pane, Action::SetModel { model: claude::model_named(model) });
        }
        ["--version" | "-V" | "version"] => println!("pixbar-bridge {}", version()),
        ["--help" | "-h" | "help"] => println!("{USAGE}"),
        _ => {
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    }
}

fn fail(why: String) -> ! {
    eprintln!("{why}");
    std::process::exit(1)
}

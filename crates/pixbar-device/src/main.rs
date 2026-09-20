//! The program that runs on the TC002 in place of Ulanzi's app: reads the knob and buttons, draws the UI
//! at 50 fps, and talks to the host bridges. Nothing is flashed; it runs from /tmp and a reboot restores stock.
//!
//!   pixbar-device [--port 17002] [--brightness 70] [--daemon] [--seconds N] [--demo] [--log-input] [--dry-run]

mod input;
mod mcu;
mod net;
mod panel;

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use pixbar_proto::{Action, DEFAULT_PORT};
use pixbar_render::{render_notice, Agent, DeviceAction, Effort, Frame, Info, Intent, Limit, Model, Settings, Status, Ui, World};

/// 20 ms per frame: inside every reading of the vendor's "no faster than 15 ms" rule.
const FRAME: Duration = Duration::from_millis(20);
/// /data is the one partition that survives a reboot; the stock app keeps its own settings there too.
const CONFIG_FILE: &str = "/data/pixbar.conf";
const LOG_FILE: &std::ffi::CStr = c"/tmp/pixbar-device.log";

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

struct Args {
    port: u16,
    brightness: Option<u8>,
    daemon: bool,
    seconds: Option<u64>,
    demo: bool,
    log_input: bool,
    dry_run: bool,
}

fn parse_args() -> Args {
    let mut a =
        Args { port: DEFAULT_PORT, brightness: None, daemon: false, seconds: None, demo: false, log_input: false, dry_run: false };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut num = |name: &str| -> u64 {
            it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| panic!("{name} needs a number"))
        };
        match flag.as_str() {
            "--port" => a.port = num("--port") as u16,
            "--brightness" => a.brightness = Some(num("--brightness").clamp(10, 100) as u8),
            "--seconds" => a.seconds = Some(num("--seconds")),
            "--daemon" => a.daemon = true,
            "--demo" => a.demo = true,
            "--log-input" => a.log_input = true,
            "--dry-run" => a.dry_run = true,
            other => panic!("unknown option {other}"),
        }
    }
    a
}

/// Detaches from the adb shell that started us, so the panel keeps running after `adb shell` returns.
fn daemonize() {
    // SAFETY: called before any thread exists; classic fork + setsid + stdio redirection.
    unsafe {
        if libc::fork() != 0 {
            // adbd kills the shell's children the moment the shell returns; give the child time to setsid.
            libc::usleep(300_000);
            libc::_exit(0);
        }
        libc::setsid();
        let log = libc::open(LOG_FILE.as_ptr(), libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC, 0o644);
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
        libc::dup2(null, 0);
        libc::dup2(log, 1);
        libc::dup2(log, 2);
    }
}

/// Settings changed on the panel (knob long-press).
fn load_settings() -> Settings {
    Settings::from_config(&std::fs::read_to_string(CONFIG_FILE).unwrap_or_default())
}

fn save_settings(s: &Settings) {
    if let Err(e) = std::fs::write(CONFIG_FILE, s.to_config()) {
        eprintln!("could not save settings: {e}");
    }
}

/// The USB-C port is dual-role and can sit in host mode, where a PC on the cable sees nothing; adb and with it
/// the wired link to a host (a stream through adbd) need device mode. Reading the driver's `usb_device` file is what
/// switches it (writing a role name to `otg_role` does nothing on this unit).
fn usb_device_mode() {
    const OTG: &str = "/sys/bus/platform/devices/soc:usbotg";
    let role = |what: &str| std::fs::read_to_string(format!("{OTG}/{what}")).unwrap_or_default();
    let before = role("otg_role");
    if before.trim() != "usb_device" {
        role("usb_device");
        eprintln!("usb port: {} -> {}", before.trim(), role("otg_role").trim());
    }
}

fn demo_world() -> World {
    let agent = |space: &str, status, model: &str, effort, used| Agent {
        reported: true,
        has_effort: true,
        next_model: Some(Model::new(if model == "opus" { "fable" } else { "opus" })),
        space: space.into(),
        tab: "1".into(),
        status,
        model: Model::new(model),
        effort,
        ctx_used: used,
        ctx_window: 1_000_000,
        ultra_ok: true,
        dir: format!("{space}-dir"),
        title: format!("{space} session"),
        fresh: false,
        session: format!("{space} named"),
        cost_cents: used / 70,
        limit_5h: Some(Limit { used_pct: 12, resets_in_min: 200 }),
        limit_7d: Some(Limit { used_pct: 93, resets_in_min: 40 }),
    };
    World {
        agents: vec![
            agent("demo", Status::Working, "opus", Effort::XHigh, 104_000),
            agent("blocked", Status::Blocked, "fable", Effort::High, 742_000),
            agent("idle", Status::Idle, "fable", Effort::Low, 38_000),
            agent("done", Status::Done, "opus", Effort::Med, 186_000),
        ],
        focused: 0,
    }
}

/// The panel's `ip:port` for hosts on the network; empty while there is no network (a host can still come in by cable).
fn local_addr(port: u16) -> String {
    // Connecting a UDP socket sends nothing; it just makes the kernel pick our outgoing address.
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:9").and_then(|_| s.local_addr()))
        .map(|a| format!("{}:{port}", a.ip()))
        .unwrap_or_default()
}

/// The network Ulanzi's setup saved for wpa_supplicant (it keeps one).
fn wifi_ssid() -> String {
    let conf = std::fs::read_to_string("/data/misc/wifi/wpa_supplicant.conf").unwrap_or_default();
    conf.lines().find_map(|l| l.trim().strip_prefix("ssid=")).map(|s| s.trim_matches('"').to_string()).unwrap_or_default()
}

fn main() {
    let args = parse_args();
    if args.daemon {
        daemonize();
    }
    // SAFETY: the handler only stores to an atomic.
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as usize);
        libc::signal(libc::SIGINT, on_signal as *const () as usize);
    }

    usb_device_mode();
    let mut panel = (!args.dry_run).then(|| panel::Panel::open().expect("open LED panel (is zkswe stopped?)"));
    let mut mcu = (!args.dry_run).then(|| mcu::Mcu::open().map_err(|e| eprintln!("mcu link: {e}")).ok()).flatten();
    let mut inputs = input::Inputs::open().expect("open knob and buttons");
    inputs.log = args.log_input;
    let mut hosts = net::Hosts::listen(args.port).expect("listen for hosts");
    if args.daemon {
        // The device shell has no pidof or pkill; the stop script reads this. Written only now: a second copy
        // dies on the line above, and must not pass for the one that runs.
        let _ = std::fs::write("/tmp/pixbar-device.pid", std::process::id().to_string());
    }
    let (mut ui, mut frame) = (Ui::new(), Frame::new());
    ui.settings = load_settings();
    if let Some(b) = args.brightness {
        ui.settings.brightness = b;
    }
    hosts.set_refresh(ui.settings.refresh_ms as u32);
    // The address can change under us (DHCP, WiFi coming up late); looking it up costs a socket, so not per frame.
    let mut addr_checked = Instant::now();
    ui.info = Info { addr: local_addr(args.port), ssid: wifi_ssid(), hosts: Vec::new() };
    eprintln!("pixbar-device up: {}, brightness {}%", ui.info.addr, ui.settings.brightness);
    // Whose agents are on the strip: recomputed when a host comes or goes, and when the HOSTS page moves.
    let mut picked = ui.settings.host;
    let (mut world, mut owners) = if args.demo { (demo_world(), Vec::new()) } else { hosts.world(picked) };
    let start = Instant::now();
    let mut next = start;
    let (mut frames, mut late) = (0u64, 0u64);
    let mut back_to_stock = false;

    while !STOP.load(Ordering::SeqCst) && args.seconds.is_none_or(|s| start.elapsed() < Duration::from_secs(s)) {
        let now = start.elapsed().as_millis() as u64;

        // poll() first, always: it is what accepts, reads and expires the connections.
        if hosts.poll() | (picked != ui.settings.host) {
            picked = ui.settings.host;
            ui.info.hosts = hosts.links();
            if !args.demo {
                (world, owners) = hosts.world(picked);
            }
        }
        if addr_checked.elapsed() >= Duration::from_secs(5) {
            addr_checked = Instant::now();
            ui.info.addr = local_addr(args.port);
        }
        if let Some(power) = mcu.as_mut().and_then(|m| m.poll()) {
            eprintln!("power: {power:?}");
            ui.set_power(power, now);
        }
        let mut intents = Vec::new();
        for input in inputs.poll() {
            intents.extend(ui.input(&world, input, now));
        }
        intents.extend(ui.tick(&world, now));
        if let Some(changed) = ui.take_settings_change() {
            save_settings(&changed);
            hosts.set_refresh(changed.refresh_ms as u32);
        }
        match ui.take_device_action() {
            Some(DeviceAction::PowerOff) => {
                eprintln!("powering off");
                if let Some(panel) = panel.as_mut() {
                    let _ = panel.blank();
                }
                if let Some(mcu) = mcu.as_mut() {
                    mcu.power_off();
                }
                // The MCU cuts the power; if we are still here in a moment, it did not (it may refuse on USB power).
                std::thread::sleep(Duration::from_secs(3));
                eprintln!("still running after the power-off command");
                next = Instant::now();
            }
            Some(DeviceAction::Stock) => {
                hosts.announce_stock();
                back_to_stock = true;
                break;
            }
            None => {}
        }
        for intent in intents {
            let (index, action) = match intent {
                Intent::Focus(i) => (i, Action::Focus),
                Intent::SetEffort { agent, effort } => (agent, Action::SetEffort { effort }),
                Intent::SetModel { agent, model } => (agent, Action::SetModel { model }),
            };
            eprintln!("intent: {action:?} on agent {index}");
            match owners.get(index) {
                Some(owner) => hosts.send_intent(owner, action),
                // Demo mode has no host: apply the change ourselves so the screen still responds.
                None => match (world.agents.get_mut(index), action) {
                    (Some(_), Action::Focus) => world.focused = index,
                    (Some(a), Action::SetEffort { effort }) => a.effort = effort,
                    (Some(a), Action::SetModel { model }) => a.model = model,
                    (None, _) => {}
                },
            }
        }

        // A picked host that is not connected leaves an empty strip; say so, rather than show nothing and
        // let it look broken. The settings screen still wins, so HOSTS is always there to pick again.
        let away = !picked.is_all() && !ui.info.hosts.iter().any(|h| h.name == picked.as_str());
        if !args.demo && !ui.wants_screen(now) && (hosts.connected() == 0 || away) {
            let addr = if ui.info.addr.is_empty() { "NO WIFI - USE USB" } else { &ui.info.addr };
            let line2 = if away { format!("{} NOT HERE", picked.as_str()) } else { addr.to_string() };
            render_notice(&mut frame, "NO HOST", &line2, now);
        } else {
            ui.render(&world, now, &mut frame);
        }
        if let Some(panel) = panel.as_mut() {
            if let Err(e) = panel.show(&frame, ui.settings.brightness as u32) {
                eprintln!("panel write failed: {e}");
            }
        }

        frames += 1;
        next += FRAME;
        match next.checked_duration_since(Instant::now()) {
            Some(wait) => std::thread::sleep(wait),
            None => {
                late += 1;
                next = Instant::now();
            }
        }
    }

    if let Some(panel) = panel.as_mut() {
        let _ = panel.blank();
    }
    eprintln!("stopped after {frames} frames ({late} late)");
    if back_to_stock {
        // Asked for on the settings screen. The panel and the MCU port are released by now; Ulanzi's app takes them.
        drop((panel, mcu));
        let _ = std::fs::remove_file("/tmp/pixbar-device.pid");
        let started = std::process::Command::new("/bin/setprop").args(["ctl.start", "zkswe"]).status();
        eprintln!("back to stock firmware: {started:?}");
    }
}

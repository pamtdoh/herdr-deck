//! `pixbar-bridge doctor`: every link between a Claude Code session and the panel, checked one by one.
//!
//! Most of these links fail without a sound: a status line that is not ours, or not run, leaves the panel
//! showing a plausible default; a service that points at a binary that is gone restarts quietly for ever. Nothing
//! is changed here, only looked at; each finding says what would mend it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::herdr::Herdr;
use crate::deploy::{Panel, Route};
use crate::{claude, deploy, install, usb, Heard};

/// herdr's socket protocol this bridge was written against.
const HERDR_PROTOCOL: u64 = 20;

#[derive(Default)]
struct Report {
    problems: usize,
}

impl Report {
    fn part(&self, name: &str) {
        println!("\n{name}");
    }

    fn ok(&self, what: impl AsRef<str>) {
        println!("  ok   {}", what.as_ref());
    }

    fn note(&self, what: impl AsRef<str>) {
        println!("  --   {}", what.as_ref());
    }

    fn bad(&mut self, what: impl AsRef<str>, mend: impl AsRef<str>) {
        self.problems += 1;
        println!("  !!   {}\n       -> {}", what.as_ref(), mend.as_ref());
    }
}

fn json(file: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(file).ok()?).ok()
}

/// The program a shell command starts, if it names one by path.
fn program_of(command: &str) -> Option<PathBuf> {
    let first = command.trim_start();
    let path = match first.strip_prefix('\'') {
        Some(quoted) => quoted.split('\'').next()?,
        None => first.split_whitespace().next()?,
    };
    path.starts_with('/').then(|| PathBuf::from(path))
}

fn managed_settings() -> Vec<PathBuf> {
    let dir = if cfg!(target_os = "macos") { "/Library/Application Support/ClaudeCode" } else { "/etc/claude-code" };
    let mut files = vec![Path::new(dir).join("managed-settings.json")];
    files.extend(std::fs::read_dir(Path::new(dir).join("managed-settings.d")).into_iter().flatten().flatten().map(|e| e.path()));
    files.retain(|f| f.exists());
    files
}

fn bridge(r: &mut Report) {
    r.part("bridge");
    r.ok(format!("pixbar-bridge {}", crate::version()));
    match install::installed_bin() {
        Ok(bin) if bin.exists() => match (std::fs::read(&bin), std::fs::read("/proc/self/exe").or_else(|_| std::fs::read(std::env::current_exe().unwrap_or_default()))) {
            (Ok(there), Ok(me)) if there == me => r.ok(format!("installed at {}", bin.display())),
            _ => r.bad(format!("{} is another build than the one you are running", bin.display()), "run this one's `install` to replace it"),
        },
        Ok(bin) => r.bad(format!("not installed ({} is missing)", bin.display()), "pixbar-bridge install"),
        Err(e) => r.bad(e.to_string(), "set HOME"),
    }
}

fn service(r: &mut Report) {
    r.part("service");
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else { return };
    let said = |program: &str, args: &[&str]| std::process::Command::new(program).args(args).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    if cfg!(target_os = "macos") {
        let file = home.join("Library/LaunchAgents/dev.pixbar.bridge.plist");
        if !file.exists() {
            return r.bad("no LaunchAgent: the bridge only runs while you run it", "pixbar-bridge install");
        }
        // SAFETY: getuid has no failure mode.
        let target = format!("gui/{}/dev.pixbar.bridge", unsafe { libc::getuid() });
        match std::process::Command::new("launchctl").args(["print", &target]).output() {
            Ok(o) if o.status.success() => r.ok(format!("{} is loaded", file.display())),
            _ => r.bad(format!("{} is not loaded", file.display()), "pixbar-bridge install"),
        }
        r.note("macOS asks once whether pixbar-bridge may find devices on the local network; refused, it hears no panel (System Settings > Privacy & Security > Local Network)");
        return;
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map_or(home.join(".config"), PathBuf::from);
    let file = config.join("systemd/user/pixbar-bridge.service");
    let Ok(unit) = std::fs::read_to_string(&file) else {
        return r.bad("no systemd user unit: the bridge only runs while you run it", "pixbar-bridge install");
    };
    let exec = unit.lines().find_map(|l| l.strip_prefix("ExecStart=")).unwrap_or_default().trim().trim_start_matches('"');
    let program = exec.split(['"', ' ']).next().unwrap_or_default();
    if !Path::new(program).exists() {
        r.bad(format!("{} starts {program}, which is not there", file.display()), "pixbar-bridge install");
    } else if program.contains("/target/") {
        r.bad(format!("the service runs {program}, inside a build directory (a clean, a move or a rebuild breaks it)"), "pixbar-bridge install");
    } else {
        r.ok(format!("{} -> {program}", file.display()));
    }
    match said("systemctl", &["--user", "is-active", "pixbar-bridge"]).as_deref() {
        Some("active") => r.ok("running"),
        Some(state) => r.bad(format!("the service is {state}"), "journalctl --user -u pixbar-bridge -n 20, then `systemctl --user restart pixbar-bridge`"),
        None => r.bad("systemctl did not answer", "is this a systemd machine with a user session?"),
    }
    let user = std::env::var("USER").unwrap_or_default();
    if said("loginctl", &["show-user", &user, "-p", "Linger"]).as_deref() == Some("Linger=no") {
        r.note("it runs from your login to your logout. `loginctl enable-linger` makes it start with the machine (a box nobody logs in to)");
    }
}

/// Whose status line command runs, and whether it is ours.
fn status_line(r: &mut Report) {
    r.part("Claude Code status line (where model, effort, context, cost and usage come from)");
    let Some(dir) = install::claude_dir() else { return r.bad("HOME is not set", "set HOME") };
    let file = dir.join("settings.json");
    let settings = json(&file).unwrap_or(Value::Null);
    let ours = |c: &str| c.contains("pixbar-bridge") && c.trim_end().ends_with("statusline");
    match settings["statusLine"]["command"].as_str() {
        Some(c) if ours(c) => match program_of(c) {
            Some(p) if !p.exists() => r.bad(format!("{} runs {}, which is not there", file.display(), p.display()), "pixbar-bridge install"),
            _ => {
                r.ok(format!("{}: {c}", file.display()));
                match install::previous_command() {
                    Some(p) => r.ok(format!("your own status line still draws the line: {p}")),
                    None => r.note("no status line of your own behind it: the line stays empty"),
                }
            }
        },
        Some(c) => {
            // The hookup made by hand: their script pipes its input to a pixbar-bridge somewhere.
            let script = c.split_whitespace().last().and_then(|f| std::fs::read_to_string(f).ok()).unwrap_or_default();
            let called = script.lines().find(|l| l.contains("pixbar-bridge") && l.contains("statusline") && !l.trim_start().starts_with('#'));
            let program = called.and_then(|l| l.split_whitespace().find(|w| w.contains("pixbar-bridge"))).map(|w| w.trim_matches(['"', '\'']));
            match program {
                Some(p) if Path::new(p).exists() && !p.contains("/target/") => r.ok(format!("{c} calls {p}")),
                Some(p) if Path::new(p).exists() => r.bad(format!("{c} calls {p}, inside a build directory; when that goes, the panel silently stops following"), "pixbar-bridge install (your script keeps drawing the line)"),
                Some(p) => r.bad(format!("{c} calls {p}, which is not there: nothing reaches the panel"), "pixbar-bridge install"),
                None => r.bad(format!("the status line is `{c}`, which does not call pixbar-bridge"), "pixbar-bridge install (it keeps your command and runs it)"),
            }
        }
        None => r.bad(format!("{} has no statusLine", file.display()), "pixbar-bridge install"),
    }
    if settings["disableAllHooks"] == true {
        r.bad(format!("{} sets disableAllHooks, which also switches the status line off", file.display()), "remove it, or set it to false");
    }
    for managed in managed_settings() {
        let m = json(&managed).unwrap_or(Value::Null);
        if !m["statusLine"].is_null() || m["disableAllHooks"] == true {
            r.bad(format!("{} sets the status line (or disableAllHooks) for this machine, over your own settings", managed.display()), "that file is your administrator's");
        }
    }
    if settings["statusLine"]["refreshInterval"].is_null() {
        r.note("Claude Code reruns the status line when model or effort change (not documented, but so far it does). If a change made on the panel snaps back after 5 s, add \"refreshInterval\": 2 to statusLine");
    }
}

fn sessions(r: &mut Report, herdr: &Herdr) {
    r.part(&format!("herdr ({})", herdr.socket().display()));
    let snapshot = match herdr.snapshot() {
        Ok(s) => s,
        Err(e) => return r.bad(format!("no answer: {e}"), "is herdr running? A named herdr session has its own socket: --socket PATH, or run `install` from inside one of its panes"),
    };
    let (version, protocol) = (snapshot["version"].as_str().unwrap_or("?"), snapshot["protocol"].as_u64());
    if protocol == Some(HERDR_PROTOCOL) {
        r.ok(format!("herdr {version}, socket protocol {HERDR_PROTOCOL}"));
    } else {
        r.bad(format!("herdr {version} speaks socket protocol {protocol:?}; this bridge was written against {HERDR_PROTOCOL}"), "it may well work; if agents, names or button presses are off, this is where to look");
    }
    let agents: Vec<&Value> = snapshot["agents"].as_array().into_iter().flatten().filter(|a| a["agent"] == "claude").collect();
    if agents.is_empty() {
        return r.note("no Claude Code session in any pane right now: nothing more to check here");
    }
    let named = agents.iter().filter(|a| a["agent_session"]["value"].is_string()).count();
    if named == 0 {
        r.bad(format!("herdr sees {} Claude Code pane(s) but knows the session of none, so status line data cannot be matched to a pane", agents.len()), "herdr integration install claude");
    }

    r.part("sessions");
    for a in agents {
        let pane = a["pane_id"].as_str().unwrap_or("?");
        let Some(session) = a["agent_session"]["value"].as_str() else {
            r.note(format!("{pane}: herdr does not know its session yet (it learns it from the session's first hook)"));
            continue;
        };
        let cwd = Path::new(a["cwd"].as_str().unwrap_or_default());
        let local = [".claude/settings.local.json", ".claude/settings.json"].iter().map(|f| cwd.join(f)).find(|f| json(f).is_some_and(|v| !v["statusLine"].is_null()));
        let heard = claude::file_of(session).and_then(|f| std::fs::metadata(f).ok()?.modified().ok());
        match (heard, local) {
            (_, Some(file)) => r.bad(format!("{pane}: {} has a statusLine of its own, which replaces yours in that project", file.display()), "have that one call `pixbar-bridge statusline` too, or remove it"),
            (Some(at), None) => {
                let age = SystemTime::now().duration_since(at).unwrap_or_default().as_secs();
                r.ok(format!("{pane}: status line heard {} ago", if age < 120 { format!("{age} s") } else { format!("{} min", age / 60) }));
            }
            (None, None) => r.bad(
                format!("{pane}: nothing from this session's status line ({})", cwd.display()),
                "it reports after the session's next reply. If it never does: the folder's trust prompt was not accepted (Claude Code then runs no status line), or the session was started with --settings",
            ),
        }
    }
}

/// The cable. Returns whether a panel answers on it.
fn cable(r: &mut Report) -> bool {
    if !usb::present() {
        return false;
    }
    let asked = Panel::open(&Route::Usb).and_then(|mut panel| {
        let through = if matches!(panel, Panel::Server) { " (through the adb server that runs on this machine and holds it)" } else { "" };
        panel.runs_pixbar().map(|up| (up, through))
    });
    match asked {
        Ok((true, through)) => r.ok(format!("a TC002 on the USB cable, running the pixbar program{through}")),
        Ok((false, through)) => r.note(format!("a TC002 on the USB cable, on Ulanzi's firmware{through}; `run` starts the program on it")),
        // Only one program can hold the cable, and a bridge that is connected over it does.
        Err(_) if crate::a_run_is_active() => r.ok("a TC002 on the USB cable (a running pixbar-bridge holds it, so it cannot be asked more from here)"),
        Err(e) if e.kind() == std::io::ErrorKind::ResourceBusy => {
            r.bad("a TC002 is on the USB cable, but another program holds it, and it is not an adb server we could go through", "which program has the device open?");
            return false;
        }
        Err(e) => {
            r.bad(format!("a TC002 is on the USB cable: {e}"), "unplug it and plug it in again; after a power-up with no bridge listening, switch it off and on");
            return false;
        }
    }
    true
}

fn panel(r: &mut Report) {
    r.part("panel");
    let cabled = cable(r);
    let heard = crate::listen(Duration::from_secs(3), false);
    for h in &heard {
        match h {
            Heard::Pixbar(addr, Some(mac)) if deploy::is_known(mac) => r.ok(format!("the pixbar program is running at {addr}")),
            Heard::Pixbar(addr, _) => {
                let ip = addr.split(':').next().unwrap_or(addr);
                r.bad(format!("a pixbar at {addr} that this machine has not started (or one running an older program): `run` does not connect to it by itself"), format!("pixbar-bridge trust {ip}, or pixbar-bridge deploy {ip}"))
            }
            Heard::Stock { ip, mac } if deploy::handed_back(mac).is_some() => r.note(format!("{ip} is on Ulanzi's firmware because you handed it back; `pixbar-bridge deploy {ip}` or switching it off and on takes it over again")),
            Heard::Stock { ip, mac } if deploy::is_known(mac) => r.note(format!("{ip} is on Ulanzi's firmware; a running `pixbar-bridge run` (the service) starts the program on it")),
            Heard::Stock { ip, .. } => r.bad(format!("a TC002 at {ip} is on Ulanzi's firmware and has never been started from this machine"), format!("pixbar-bridge deploy {ip}")),
            Heard::Usb => {}
        }
    }
    if heard.is_empty() && !cabled {
        r.bad(
            "no TC002 on a USB cable, and none heard on the network in 3 s",
            "is it switched on, and joined to this network with Ulanzi's app? Its announcements are UDP broadcasts (ports 17003 and 55555): a firewall here, or a network that keeps its clients apart, hides them; `run --device IP` needs none, and neither does a USB cable (plugged in while the panel is switched on, or before)",
        );
    }
}

pub fn run(herdr: &Herdr) -> bool {
    let mut r = Report::default();
    bridge(&mut r);
    service(&mut r);
    status_line(&mut r);
    sessions(&mut r, herdr);
    panel(&mut r);
    println!("\n{}", if r.problems == 0 { "all links are in place".to_string() } else { format!("{} problem(s), marked !!", r.problems) });
    r.problems == 0
}

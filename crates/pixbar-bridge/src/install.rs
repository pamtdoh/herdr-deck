//! `pixbar-bridge install` / `uninstall`: everything the bridge needs around it on this machine, put there and
//! taken away by the bridge itself.
//!
//! - a copy of this binary in `~/.local/bin`, so that nothing points into a build directory;
//! - a service that runs `pixbar-bridge run` from login on (a systemd user unit, or a LaunchAgent on macOS);
//! - Claude Code's status line, which is where model, effort and context come from: a marked block in the user's
//!   own status line script that hands Claude Code's input on to `pixbar-bridge statusline`, the way an installer
//!   adds a PATH line to a shell profile. The script stays the user's, and stays the status line command.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const UNIT: &str = "pixbar-bridge.service";
const AGENT: &str = "dev.pixbar.bridge";
/// Around what `install` adds to a status line script, so that it can be found again, brought up to date and removed.
const BEGIN: &str = "# >>> pixbar-bridge >>>";
const END: &str = "# <<< pixbar-bridge <<<";

fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

fn home() -> io::Result<PathBuf> {
    var("HOME").ok_or_else(|| io::Error::other("HOME is not set"))
}

/// `~/.config/pixbar`: the devices this machine starts, and the models its button goes through.
pub fn config_dir() -> Option<PathBuf> {
    Some(var("XDG_CONFIG_HOME").or_else(|| Some(var("HOME")?.join(".config")))?.join("pixbar"))
}

pub fn claude_dir() -> Option<PathBuf> {
    var("CLAUDE_CONFIG_DIR").or_else(|| Some(var("HOME")?.join(".claude")))
}

pub fn installed_bin() -> io::Result<PathBuf> {
    Ok(home()?.join(".local/bin/pixbar-bridge"))
}

/// Replaces `file` in one step, so that nothing ever reads half of it.
fn write_whole(file: &Path, bytes: &[u8], mode: Option<u32>) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
    let tmp = file.with_file_name(format!(".{}.{}", file.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    std::fs::write(&tmp, bytes)?;
    if let Some(mode) = mode {
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
    }
    std::fs::rename(&tmp, file)
}

// ---------------------------------------------------------------- the binary

/// This very program, even if the file it was started from has been replaced or removed since.
fn own_bytes() -> io::Result<Vec<u8>> {
    std::fs::read("/proc/self/exe").or_else(|_| std::fs::read(std::env::current_exe()?))
}

fn install_binary() -> io::Result<PathBuf> {
    let to = installed_bin()?;
    let bytes = own_bytes()?;
    if std::fs::read(&to).is_ok_and(|there| there == bytes) {
        println!("binary       {} (already this build)", to.display());
    } else {
        write_whole(&to, &bytes, Some(0o755))?;
        println!("binary       {}", to.display());
    }
    let on_path = std::env::var_os("PATH").is_some_and(|p| std::env::split_paths(&p).any(|d| Some(d.as_path()) == to.parent()));
    if !on_path {
        println!("             ({} is not on your PATH; the service and the status line use the full path)", to.parent().unwrap().display());
    }
    Ok(to)
}

// ---------------------------------------------------------------- the service

fn systemd_unit_file() -> io::Result<PathBuf> {
    Ok(var("XDG_CONFIG_HOME").map_or(home()?.join(".config"), |c| c).join("systemd/user").join(UNIT))
}

fn launch_agent_file() -> io::Result<PathBuf> {
    Ok(home()?.join("Library/LaunchAgents").join(format!("{AGENT}.plist")))
}

/// A herdr socket worth pinning in the service: the one this shell is in, when it is not the one the bridge
/// would find by itself (a named herdr session).
fn socket_to_pin(default: &Path) -> Option<PathBuf> {
    var("HERDR_SOCKET_PATH").filter(|s| s != default)
}

fn ran(program: &str, args: &[&str]) -> io::Result<()> {
    let out = Command::new(program).args(args).output().map_err(|e| io::Error::other(format!("{program}: {e}")))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("{program} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())))
    }
}

fn install_service(bin: &Path, default_socket: &Path) -> io::Result<()> {
    let socket = socket_to_pin(default_socket);
    if cfg!(target_os = "macos") {
        let xml = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let log = home()?.join("Library/Logs/pixbar-bridge.log");
        let env = socket.as_ref().map_or(String::new(), |s| {
            format!("    <key>EnvironmentVariables</key>\n    <dict><key>HERDR_SOCKET_PATH</key><string>{}</string></dict>\n", xml(&s.to_string_lossy()))
        });
        let plist = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n<dict>\n    <key>Label</key><string>{AGENT}</string>\n    <key>ProgramArguments</key>\n    <array><string>{}</string><string>run</string></array>\n{env}    <key>RunAtLoad</key><true/>\n    <key>KeepAlive</key><true/>\n    <key>ProcessType</key><string>Background</string>\n    <key>StandardErrorPath</key><string>{}</string>\n</dict>\n</plist>\n",
            xml(&bin.to_string_lossy()),
            xml(&log.to_string_lossy()),
        );
        let file = launch_agent_file()?;
        write_whole(&file, plist.as_bytes(), None)?;
        // SAFETY: getuid has no failure mode.
        let domain = format!("gui/{}", unsafe { libc::getuid() });
        let _ = ran("launchctl", &["bootout", &format!("{domain}/{AGENT}")]);
        ran("launchctl", &["bootstrap", &domain, &file.to_string_lossy()])?;
        println!("service      {} (log: {})", file.display(), log.display());
    } else {
        let quoted = |p: &Path| format!("\"{}\"", p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""));
        let env = socket.as_ref().map_or(String::new(), |s| format!("Environment=HERDR_SOCKET_PATH={}\n", quoted(s)));
        // Exit status 2 is a wrong command line, which no restart mends.
        let unit = format!(
            "[Unit]\nDescription=Pixbar bridge: herdr's agents on the Ulanzi TC002\n\n[Service]\nExecStart={} run\n{env}Restart=always\nRestartSec=5\nRestartPreventExitStatus=2\n\n[Install]\nWantedBy=default.target\n",
            quoted(bin)
        );
        let file = systemd_unit_file()?;
        write_whole(&file, unit.as_bytes(), None)?;
        ran("systemctl", &["--user", "daemon-reload"])?;
        ran("systemctl", &["--user", "enable", UNIT])?;
        ran("systemctl", &["--user", "restart", UNIT])?;
        println!("service      {} (log: journalctl --user -u pixbar-bridge)", file.display());
    }
    if let Some(s) = socket {
        println!("             pinned to this herdr session: {}", s.display());
    }
    Ok(())
}

fn uninstall_service() -> io::Result<()> {
    if cfg!(target_os = "macos") {
        let file = launch_agent_file()?;
        if file.exists() {
            // SAFETY: getuid has no failure mode.
            let _ = ran("launchctl", &["bootout", &format!("gui/{}/{AGENT}", unsafe { libc::getuid() })]);
            std::fs::remove_file(&file)?;
            println!("removed      {}", file.display());
        }
    } else {
        let file = systemd_unit_file()?;
        if file.exists() {
            let _ = ran("systemctl", &["--user", "disable", "--now", UNIT]);
            std::fs::remove_file(&file)?;
            let _ = ran("systemctl", &["--user", "daemon-reload"]);
            println!("removed      {}", file.display());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------- the status line

/// Where the value at `path` sits in a JSON text, as a byte range. The settings file is someone else's: only
/// the bytes of the one value are replaced, and everything around them stays exactly as it was written.
pub fn span_of(text: &str, path: &[&str]) -> Option<(usize, usize)> {
    fn ws(b: &[u8], mut i: usize) -> usize {
        while b.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        i
    }
    fn string_end(b: &[u8], mut i: usize) -> Option<usize> {
        i += 1; // the opening quote
        while *b.get(i)? != b'"' {
            i += if b[i] == b'\\' { 2 } else { 1 };
        }
        Some(i + 1)
    }
    /// The end of the value that starts at `i`; and, with a non-empty `path`, the range of the value it leads to.
    fn value(b: &[u8], i: usize, path: &[&str], found: &mut Option<(usize, usize)>) -> Option<usize> {
        let i = ws(b, i);
        let end = match *b.get(i)? {
            b'"' => string_end(b, i)?,
            open @ (b'{' | b'[') => {
                let close = if open == b'{' { b'}' } else { b']' };
                let mut k = ws(b, i + 1);
                while *b.get(k)? != close {
                    let mut inner: &[&str] = &[];
                    if open == b'{' {
                        let key_end = string_end(b, k)?;
                        let key: String = serde_json::from_slice(&b[k..key_end]).ok()?;
                        if path.first() == Some(&key.as_str()) && found.is_none() {
                            inner = &path[1..];
                        }
                        k = ws(b, key_end);
                        if *b.get(k)? != b':' {
                            return None;
                        }
                        k = ws(b, k + 1);
                        let leads_here = path.first() == Some(&key.as_str()) && found.is_none();
                        let end = value(b, k, inner, found)?;
                        if leads_here && path.len() == 1 {
                            *found = Some((k, end));
                        }
                        k = end;
                    } else {
                        k = value(b, k, &[], found)?;
                    }
                    k = ws(b, k);
                    if b.get(k) == Some(&b',') {
                        k = ws(b, k + 1);
                    }
                }
                k + 1
            }
            _ => {
                let mut k = i;
                while b.get(k).is_some_and(|c| !c.is_ascii_whitespace() && !b",]}".contains(c)) {
                    k += 1;
                }
                k
            }
        };
        Some(end)
    }
    let mut found = None;
    value(text.as_bytes(), 0, path, &mut found)?;
    found
}

fn is_ours(command: &str) -> bool {
    command.contains("pixbar-bridge") && command.trim_end().ends_with("statusline")
}

/// The binary as a word of a shell command.
fn quoted(bin: &Path) -> String {
    let bin = bin.to_string_lossy();
    if bin.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        bin.into_owned()
    } else {
        format!("'{}'", bin.replace('\'', "'\\''"))
    }
}

fn our_command(bin: &Path) -> String {
    format!("{} statusline", quoted(bin))
}

/// The words of a shell command, as far as a status line command goes: quotes, and a home directory.
fn words(command: &str) -> Vec<String> {
    let (mut words, mut word, mut quote, mut any) = (Vec::new(), String::new(), None, false);
    for c in command.chars() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '\'' | '"') => (quote, any) = (Some(c), true),
            (None, c) if c.is_whitespace() => {
                if any || !word.is_empty() {
                    words.push(std::mem::take(&mut word));
                }
                any = false;
            }
            (_, c) => word.push(c),
        }
    }
    if any || !word.is_empty() {
        words.push(word);
    }
    let home = var("HOME").map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let at_home = |w: String| match w.strip_prefix("~/").or(w.strip_prefix("$HOME/")).or(w.strip_prefix("${HOME}/")) {
        Some(rest) => format!("{home}/{rest}"),
        None => w,
    };
    words.into_iter().map(at_home).collect()
}

/// The script a status line command runs, where it runs one: the first of its words that names a text file
/// (`bash ~/.claude/statusline.sh`, `/bin/sh /path/line.sh`, or the script by itself).
pub fn script_of(command: &str) -> Option<PathBuf> {
    words(command).into_iter().filter(|w| w.contains('/')).map(PathBuf::from).find(|file| std::fs::read(file).is_ok_and(|bytes| !bytes.starts_with(b"\x7fELF") && std::str::from_utf8(&bytes).is_ok()))
}

/// The variable a shell script reads Claude Code's input into, if `line` is where it does: `input=$(cat)`, the
/// form in Claude Code's own examples and the one `/statusline` writes.
fn input_variable(line: &str) -> Option<&str> {
    let (name, read) = line.trim().split_once('=')?;
    let reads_stdin = ["$(cat)", "\"$(cat)\"", "`cat`", "\"`cat`\""].contains(&read.trim().trim_end_matches(';'));
    (reads_stdin && !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).then_some(name)
}

fn calls_us(line: &str) -> bool {
    !line.trim_start().starts_with('#') && line.contains("pixbar-bridge") && line.contains("statusline")
}

/// A shell script with our block taken out again.
fn without_call(script: &str) -> String {
    let (mut out, mut inside) = (String::new(), false);
    for line in script.split_inclusive('\n') {
        match line.trim() {
            BEGIN => inside = true,
            END if inside => inside = false,
            _ if !inside => out.push_str(line),
            _ => {}
        }
    }
    out
}

/// A shell script with our block in it. Where it goes: where it already is; else in place of a call to the bridge
/// that was put there by hand; else right after the line that reads Claude Code's input, which our call is handed
/// on from. `None`: not a script we know how to add to (it does not read its input into a variable, or it is not
/// a shell script at all).
fn with_call(script: &str, bin: &Path) -> Option<String> {
    let shell = script.lines().next().is_none_or(|first| !first.starts_with("#!") || first.contains("sh"));
    let lines: Vec<&str> = script.split_inclusive('\n').collect();
    let read_at = lines.iter().position(|l| input_variable(l).is_some()).filter(|_| shell)?;
    let variable = input_variable(lines[read_at])?;
    // Which lines are ours already: a block of ours from its first line to its last, or a call by hand.
    let mut inside = false;
    let ours: Vec<bool> = lines
        .iter()
        .map(|l| {
            let begins = l.trim() == BEGIN;
            let part = inside || begins || calls_us(l);
            inside = (inside || begins) && l.trim() != END;
            part
        })
        .collect();
    let stands_at = ours.iter().position(|&o| o).filter(|&at| at > read_at);
    let indent: String = lines[stands_at.unwrap_or(read_at)].chars().take_while(|c| c.is_whitespace() && *c != '\n').collect();
    let block = format!(
        "{indent}{BEGIN}\n{indent}# Added by `pixbar-bridge install`, removed by `pixbar-bridge uninstall`: the panel's model, effort and context.\n{indent}printf '%s' \"${variable}\" | {} statusline 2>/dev/null || true\n{indent}{END}\n",
        quoted(bin)
    );
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        if Some(i) == stands_at {
            out.push_str(&block);
        }
        if ours[i] {
            continue;
        }
        out.push_str(line);
        if i == read_at && stands_at.is_none() {
            if !line.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(&block);
        }
    }
    Some(out)
}

/// The settings text with `statusLine.command` set to `command` (`None`: the whole `statusLine` taken out),
/// and what the file must then mean, to check the edit against.
fn with_command(text: &str, command: Option<&str>) -> io::Result<String> {
    let bad = |what: &str| io::Error::other(format!("settings.json: {what}; not touching it"));
    let before: Value = serde_json::from_str(text).map_err(|e| bad(&format!("not JSON ({e})")))?;
    let mut expect = before.clone();
    let top = expect.as_object_mut().ok_or_else(|| bad("not a JSON object"))?;
    let after = match (command, span_of(text, &["statusLine", "command"]), span_of(text, &["statusLine"])) {
        (Some(command), Some((from, to)), _) => {
            top["statusLine"]["command"] = Value::String(command.into());
            format!("{}{}{}", &text[..from], Value::String(command.into()), &text[to..])
        }
        (Some(_), None, Some(_)) => return Err(bad("there is a statusLine without a command")),
        (Some(command), None, None) => {
            let entry = serde_json::json!({ "type": "command", "command": command });
            top.insert("statusLine".into(), entry.clone());
            let open = text.find('{').ok_or_else(|| bad("not a JSON object"))?;
            let empty = before.as_object().is_some_and(|o| o.is_empty());
            // Written out by hand: serde_json sorts an object's keys.
            let pretty = format!("{{\n    \"type\": \"command\",\n    \"command\": {}\n  }}", Value::String(command.into()));
            format!("{}\n  \"statusLine\": {pretty}{}{}", &text[..=open], if empty { "\n" } else { "," }, &text[open + 1..])
        }
        (None, _, Some((from, to))) => {
            top.remove("statusLine");
            // With its key, and the comma that goes with it.
            let bytes = text.as_bytes();
            let key = text[..from].rfind("\"statusLine\"").ok_or_else(|| bad("statusLine has no key"))?;
            let mut end = to;
            while bytes.get(end).is_some_and(u8::is_ascii_whitespace) {
                end += 1;
            }
            if bytes.get(end) == Some(&b',') {
                format!("{}{}", &text[..key], text[end + 1..].trim_start())
            } else {
                let kept = text[..key].trim_end();
                format!("{}{}", kept.strip_suffix(',').unwrap_or(kept), &text[to..])
            }
        }
        (None, _, None) => text.to_string(),
    };
    match serde_json::from_str::<Value>(&after) {
        Ok(v) if v == expect => Ok(after),
        _ => Err(bad("the edit would have changed more than the status line")),
    }
}

/// Writes a script back where it was: through a symlink (a dotfiles checkout), and with its permissions.
fn rewrite_script(file: &Path, text: &str) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let real = std::fs::canonicalize(file)?;
    let mode = std::fs::metadata(&real)?.permissions().mode();
    write_whole(&real, text.as_bytes(), Some(mode))
}

fn install_status_line(bin: &Path) -> io::Result<()> {
    let file = claude_dir().ok_or_else(|| io::Error::other("HOME is not set"))?.join("settings.json");
    let text = std::fs::read_to_string(&file).unwrap_or_else(|_| "{}\n".into());
    let now: Value = serde_json::from_str(&text).map_err(|e| io::Error::other(format!("{}: not JSON ({e}); not touching it", file.display())))?;
    let line = format!("printf '%s' \"$input\" | {} statusline 2>/dev/null || true", quoted(bin));
    match now["statusLine"]["command"].as_str() {
        // No status line of the user's to add to: ours is the whole command, and the line stays empty.
        None => {
            if !now["statusLine"].is_null() {
                return Err(io::Error::other(format!("{}: there is a statusLine without a command; not touching it", file.display())));
            }
            write_whole(&file, with_command(&text, Some(&our_command(bin)))?.as_bytes(), None)?;
            println!("status line  {}: statusLine.command = {} (you had none; it draws nothing)", file.display(), our_command(bin));
        }
        Some(c) if is_ours(c) => {
            if c != our_command(bin) {
                write_whole(&file, with_command(&text, Some(&our_command(bin)))?.as_bytes(), None)?;
            }
            println!("status line  {}: statusLine.command = {}", file.display(), our_command(bin));
        }
        Some(theirs) => {
            let script = script_of(theirs);
            let added = script.as_ref().and_then(|f| Some((f, std::fs::read_to_string(f).ok()?))).and_then(|(f, was)| Some((f, with_call(&was, bin)?, was)));
            match added {
                Some((script, text, was)) if text == was => println!("status line  {} (already calls this bridge)", script.display()),
                Some((script, text, was)) => {
                    let adopted = was.lines().any(|l| calls_us(l)) && !was.contains(BEGIN);
                    rewrite_script(script, &text)?;
                    println!("status line  {}: {}", script.display(), if adopted { "the call you had put in by hand now goes to the installed bridge, in a marked block" } else { "a marked block hands Claude Code's input on to the bridge" });
                }
                None => {
                    println!("status line  not changed: `{theirs}` is not a shell script that reads its input with `input=$(cat)`, so there is no safe place to add to.");
                    println!("             Wherever it has Claude Code's input (the JSON on its stdin), hand a copy to:  {}", our_command(bin));
                    println!("             In a shell script:  input=$(cat);  {line}");
                }
            }
        }
    }
    Ok(())
}

fn uninstall_status_line() -> io::Result<()> {
    let Some(file) = claude_dir().map(|d| d.join("settings.json")) else { return Ok(()) };
    let Ok(text) = std::fs::read_to_string(&file) else { return Ok(()) };
    let now: Value = serde_json::from_str(&text).map_err(|e| io::Error::other(format!("{}: not JSON ({e}); not touching it", file.display())))?;
    match now["statusLine"]["command"].as_str() {
        Some(c) if is_ours(c) => {
            write_whole(&file, with_command(&text, None)?.as_bytes(), None)?;
            println!("status line  {}: statusLine removed (it was only ours)", file.display());
        }
        Some(theirs) => {
            if let Some(script) = script_of(theirs) {
                let was = std::fs::read_to_string(&script)?;
                if was.contains(BEGIN) {
                    rewrite_script(&script, &without_call(&was))?;
                    println!("status line  {}: our block is out again", script.display());
                }
            }
        }
        None => {}
    }
    Ok(())
}

// ---------------------------------------------------------------- the two commands

pub fn install(default_socket: &Path, service: bool, status_line: bool) -> io::Result<()> {
    let bin = install_binary()?;
    if status_line {
        install_status_line(&bin)?;
    }
    if service {
        install_service(&bin, default_socket)?;
    }
    println!("\n`pixbar-bridge doctor` checks every link from Claude Code to the panel.");
    Ok(())
}

pub fn uninstall() -> io::Result<()> {
    uninstall_service()?;
    uninstall_status_line()?;
    let cache = var("XDG_CACHE_HOME").or_else(|| Some(var("HOME")?.join(".cache"))).map(|c| c.join("pixbar"));
    for dir in [cache, config_dir()].into_iter().flatten() {
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
            println!("removed      {}", dir.display());
        }
    }
    let bin = installed_bin()?;
    if bin.exists() {
        std::fs::remove_file(&bin)?;
        println!("removed      {}", bin.display());
    }
    println!("\nA panel that still runs the pixbar program goes back to Ulanzi's firmware when it is switched off and on.\nIts settings stay on it in /data/pixbar.conf (128 bytes).");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = "{\n  \"model\": \"fable\",\n  \"statusLine\": {\n    \"type\": \"command\",\n    \"command\": \"bash ~/.claude/line.sh \\\"quoted\\\"\",\n    \"padding\": 0\n  },\n  \"hooks\": {\"Stop\": [{\"statusLine\": {\"command\": \"decoy\"}}]},\n  \"big\": 12345678901234567890123\n}\n";

    #[test]
    fn only_the_command_changes_in_a_settings_file() {
        let (from, to) = span_of(SETTINGS, &["statusLine", "command"]).unwrap();
        assert_eq!(&SETTINGS[from..to], "\"bash ~/.claude/line.sh \\\"quoted\\\"\"");
        let after = with_command(SETTINGS, Some("/home/me/.local/bin/pixbar-bridge statusline")).unwrap();
        assert_eq!(after, SETTINGS.replace("\"bash ~/.claude/line.sh \\\"quoted\\\"\"", "\"/home/me/.local/bin/pixbar-bridge statusline\""));
        // And back.
        assert_eq!(with_command(&after, Some("bash ~/.claude/line.sh \"quoted\"")).unwrap(), SETTINGS);
    }

    #[test]
    fn a_status_line_is_added_where_there_was_none_and_taken_out_again() {
        let without = "{\n  \"model\": \"fable\",\n  \"env\": {}\n}\n";
        let with = with_command(without, Some("x statusline")).unwrap();
        assert_eq!(with, "{\n  \"statusLine\": {\n    \"type\": \"command\",\n    \"command\": \"x statusline\"\n  },\n  \"model\": \"fable\",\n  \"env\": {}\n}\n");
        assert_eq!(with_command(&with, None).unwrap(), without);
        assert_eq!(with_command(&with_command("{}\n", Some("x statusline")).unwrap(), None).unwrap().split_whitespace().collect::<String>(), "{}");
        // Last in its object: the comma before it goes too.
        let last = "{\n  \"model\": \"fable\",\n  \"statusLine\": {\"type\": \"command\", \"command\": \"y\"}\n}\n";
        assert_eq!(with_command(last, None).unwrap(), "{\n  \"model\": \"fable\"\n}\n");
    }

    #[test]
    fn a_settings_file_that_is_not_what_we_expect_is_left_alone() {
        assert!(with_command("{ // comment\n}", Some("x")).is_err());
        assert!(with_command("[]", Some("x")).is_err());
        assert!(with_command("{\"statusLine\": {\"type\": \"command\"}}", Some("x")).is_err());
    }

    #[test]
    fn our_command_is_recognised_wherever_the_binary_lives() {
        assert!(is_ours("/home/me/.local/bin/pixbar-bridge statusline"));
        assert!(is_ours("'/home/my name/bin/pixbar-bridge' statusline"));
        assert!(!is_ours("bash /home/me/.claude/statusline-command.sh"));
        assert_eq!(our_command(Path::new("/home/my name/bin/pixbar-bridge")), "'/home/my name/bin/pixbar-bridge' statusline");
    }

    const SCRIPT: &str = "#!/usr/bin/env bash\n# my line\n\ninput=$(cat)\n\ncwd=$(echo \"$input\" | jq -r .cwd)\nprintf '%s' \"$cwd\"\n";

    #[test]
    fn a_marked_block_goes_in_after_the_input_is_read_and_comes_out_again() {
        let bin = Path::new("/home/me/.local/bin/pixbar-bridge");
        let with = with_call(SCRIPT, bin).unwrap();
        let expected = "input=$(cat)\n# >>> pixbar-bridge >>>\n# Added by `pixbar-bridge install`, removed by `pixbar-bridge uninstall`: the panel's model, effort and context.\nprintf '%s' \"$input\" | /home/me/.local/bin/pixbar-bridge statusline 2>/dev/null || true\n# <<< pixbar-bridge <<<\n\ncwd=";
        assert!(with.contains(expected), "{with}");
        assert_eq!(with_call(&with, bin).unwrap(), with, "again changes nothing");
        assert_eq!(without_call(&with), SCRIPT, "and out again leaves the script as it was");
        // The binary has moved: the block follows, and there is still one.
        let moved = with_call(&with, Path::new("/opt/pixbar/pixbar-bridge")).unwrap();
        assert!(moved.contains("| /opt/pixbar/pixbar-bridge statusline") && moved.matches("pixbar-bridge statusline").count() == 1, "{moved}");
    }

    #[test]
    fn a_call_put_in_by_hand_is_taken_over_where_it_stands() {
        let by_hand = SCRIPT.replace("\ncwd=", "\n  # the panel reads along\n  printf '%s' \"$input\" | /src/target/release/pixbar-bridge statusline 2>/dev/null || true\ncwd=");
        let with = with_call(&by_hand, Path::new("/home/me/.local/bin/pixbar-bridge")).unwrap();
        assert!(!with.contains("/src/target") && with.matches("pixbar-bridge statusline").count() == 1, "{with}");
        assert!(with.contains("  # the panel reads along\n  # >>> pixbar-bridge >>>\n"), "their comment stays, the block takes the call's place and indent:\n{with}");
        assert_eq!(with_call(&with, Path::new("/home/me/.local/bin/pixbar-bridge")).unwrap(), with, "and it stays where it stands");
    }

    #[test]
    fn a_script_we_cannot_add_to_is_left_alone() {
        let bin = Path::new("/b/pixbar-bridge");
        assert_eq!(with_call("#!/usr/bin/env python3\nimport sys\ninput=$(cat)\n", bin), None, "not a shell script");
        assert_eq!(with_call("#!/bin/sh\njq -r .model.display_name\n", bin), None, "reads its input straight into jq");
        for line in ["data=\"$(cat)\"", "  IN=`cat`;", "input=$(cat)"] {
            assert!(input_variable(line).is_some(), "{line}");
        }
        assert_eq!(input_variable("input=$(cat file)"), None);
        assert_eq!(words("bash '/home/my name/line.sh' --x"), ["bash", "/home/my name/line.sh", "--x"]);
    }
}

//! herdr's socket API: newline-delimited JSON over a unix socket.
//!
//! The server answers exactly one request per connection and then closes it; only `events.subscribe` keeps a
//! connection open, streaming events. So: a fresh connection per request, and one long-lived subscription.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Value};

#[derive(Clone)]
pub struct Herdr {
    socket: PathBuf,
}

/// The toggle in the header of herdr's agent panel: "grouped" by space, or by "priority" (who wants attention).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PanelSort {
    Spaces,
    Priority,
}

impl PanelSort {
    fn named(name: &str) -> Option<PanelSort> {
        match name {
            "spaces" | "workspaces" => Some(PanelSort::Spaces),
            "priority" => Some(PanelSort::Priority),
            _ => None,
        }
    }

    /// `[ui] agent_panel_sort` in herdr's config.toml.
    fn in_config(toml: &str) -> Option<PanelSort> {
        let mut in_ui = false;
        for line in toml.lines().map(|l| l.split('#').next().unwrap_or("").trim()) {
            if line.starts_with('[') {
                in_ui = line == "[ui]";
            } else if let Some((key, value)) = line.split_once('=').filter(|_| in_ui) {
                if key.trim() == "agent_panel_sort" {
                    return PanelSort::named(value.trim().trim_matches(['"', '\'']));
                }
            }
        }
        None
    }

    /// `agent_panel_sort` in the state file of a herdr 0.9 client; absent until the toggle has been clicked.
    fn in_client_state(json: &str) -> Option<PanelSort> {
        PanelSort::named(serde_json::from_str::<Value>(json).ok()?["agent_panel_sort"].as_str()?)
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

/// An XDG base directory, or its usual place under the home directory.
fn xdg(name: &str, under_home: &str) -> PathBuf {
    var(name).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(var("HOME").unwrap_or_default()).join(under_home))
}

/// herdr names a client's state file after the FNV-1a hash of the client socket's path.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |hash, b| (hash ^ u64::from(*b)).wrapping_mul(0x100000001b3))
}

impl Herdr {
    /// `--socket`, else what herdr exported into its panes, else where herdr itself puts it: under its config
    /// directory, and for a named session (`herdr --session work`) under `sessions/<name>`.
    pub fn new(socket: Option<String>) -> Herdr {
        let pane = std::env::var("HERDR_SOCKET_PATH").ok().filter(|v| !v.is_empty());
        Herdr { socket: socket.or(pane).map(PathBuf::from).unwrap_or_else(Herdr::default_socket) }
    }

    /// Where herdr puts its socket when nothing says otherwise.
    pub fn default_socket() -> PathBuf {
        let config = xdg("XDG_CONFIG_HOME", ".config");
        match var("HERDR_SESSION") {
            Some(name) => config.join("herdr/sessions").join(name).join("herdr.sock"),
            None => config.join("herdr/herdr.sock"),
        }
    }

    pub fn socket(&self) -> &std::path::Path {
        &self.socket
    }

    /// How herdr's agent panel is ordered right now. The socket API does not say; herdr keeps the toggle on disk.
    /// Up to 0.8 a click rewrites `agent_panel_sort` in config.toml. From 0.9 the click goes to the client's own
    /// state file, which then overrules config.toml.
    pub fn panel_sort(&self) -> PanelSort {
        let read = |path: PathBuf| std::fs::read_to_string(path).ok();
        let config = var("HERDR_CONFIG_PATH").map(PathBuf::from).unwrap_or_else(|| xdg("XDG_CONFIG_HOME", ".config").join("herdr/config.toml"));
        read(self.client_state())
            .and_then(|json| PanelSort::in_client_state(&json))
            .or_else(|| read(config).and_then(|toml| PanelSort::in_config(&toml)))
            .unwrap_or(PanelSort::Spaces)
    }

    /// Where a herdr 0.9 client keeps what was clicked in its chrome: named after its own socket, which sits next
    /// to the API socket as `<stem>-client.sock`.
    fn client_state(&self) -> PathBuf {
        let stem = self.socket.file_stem().and_then(|s| s.to_str()).unwrap_or("herdr");
        let client = self.socket.with_file_name(format!("{stem}-client.sock"));
        let hash = fnv1a(client.to_string_lossy().as_bytes());
        xdg("XDG_STATE_HOME", ".local/state").join(format!("herdr/client-shell/local-{hash:016x}.json"))
    }

    pub fn request(&self, method: &str, params: Value) -> io::Result<Value> {
        let mut stream = UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(Duration::from_secs(5)))?;
        let req = json!({ "id": "pixbar", "method": method, "params": params });
        stream.write_all(format!("{req}\n").as_bytes())?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line)?;
        let mut resp: Value = serde_json::from_str(&line).map_err(io::Error::other)?;
        match resp.get("error") {
            Some(e) => Err(io::Error::other(format!("{method}: {e}"))),
            None => Ok(resp["result"].take()),
        }
    }

    pub fn snapshot(&self) -> io::Result<Value> {
        Ok(self.request("session.snapshot", json!({}))?["snapshot"].take())
    }

    pub fn agent_status(&self, pane: &str) -> io::Result<String> {
        let r = self.request("agent.get", json!({ "target": pane }))?;
        Ok(r["agent"]["agent_status"].as_str().unwrap_or("unknown").to_string())
    }

    pub fn focus(&self, pane: &str) -> io::Result<()> {
        self.request("agent.focus", json!({ "target": pane })).map(drop)
    }

    pub fn send_keys(&self, pane: &str, keys: &[&str]) -> io::Result<()> {
        self.request("pane.send_keys", json!({ "pane_id": pane, "keys": keys })).map(drop)
    }

    /// Literal text, as if pasted: no key names are read into it.
    pub fn send_text(&self, pane: &str, text: &str) -> io::Result<()> {
        self.request("pane.send_text", json!({ "pane_id": pane, "text": text })).map(drop)
    }

    /// A new pane to the `right` of this one or `down` under it, in the same directory. The focus goes with it:
    /// whoever splits wants to type there.
    pub fn split(&self, pane: &str, direction: &str) -> io::Result<()> {
        self.request("pane.split", json!({ "target_pane_id": pane, "direction": direction, "focus": true })).map(drop)
    }

    pub fn tab_of(&self, pane: &str) -> io::Result<String> {
        let info = self.request("pane.get", json!({ "pane_id": pane }))?;
        info["pane"]["tab_id"].as_str().map(str::to_string).ok_or_else(|| io::Error::other("pane.get names no tab"))
    }

    /// Any pane, agent or not.
    pub fn focus_pane(&self, pane: &str) -> io::Result<()> {
        self.request("pane.focus", json!({ "pane_id": pane })).map(drop)
    }

    pub fn rename_tab(&self, tab: &str, label: &str) -> io::Result<()> {
        self.request("tab.rename", json!({ "tab_id": tab, "label": label })).map(drop)
    }

    /// Asks the keyboard for a new name for this pane's tab. herdr's own rename prompt cannot be opened from
    /// outside (every rename call wants the name, and keys sent to a pane reach its program, not herdr), so the
    /// question is put in a few lines split off under the pane: `pixbar-bridge name-tab` asks there, renames,
    /// hands the focus back to the pane it came from, and the shell's `exit` takes the asking pane away again.
    pub fn ask_tab_name(&self, pane: &str) -> io::Result<()> {
        let tab = self.tab_of(pane)?;
        let split = json!({ "target_pane_id": pane, "direction": "down", "focus": true, "ratio": 0.85 });
        let asking = self.request("pane.split", split)?["pane"]["pane_id"].as_str().unwrap_or_default().to_string();
        // Once the shell has drawn its prompt. (Typed earlier it would still arrive, but some prompts eat it.)
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while self.screen(&asking).is_ok_and(|s| s.trim().is_empty()) && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let me = std::env::current_exe()?;
        // The leading space keeps it out of the history of shells that are set up that way.
        self.send_text(&asking, &format!(" {} name-tab {tab} {pane}; exit", me.display()))?;
        self.send_keys(&asking, &["enter"])
    }

    /// herdr asks nothing: the pane and what runs in it are gone. The last pane takes its tab with it, the last
    /// tab its space.
    pub fn close_pane(&self, pane: &str) -> io::Result<()> {
        self.request("pane.close", json!({ "pane_id": pane })).map(drop)
    }

    /// The tab this pane is in, with every pane in it.
    pub fn close_tab_of(&self, pane: &str) -> io::Result<()> {
        self.request("tab.close", json!({ "tab_id": self.tab_of(pane)? })).map(drop)
    }

    pub fn screen(&self, pane: &str) -> io::Result<String> {
        let r = self.request("pane.read", json!({ "pane_id": pane, "source": "visible" }))?;
        Ok(r["read"]["text"].as_str().or(r["text"].as_str()).unwrap_or_default().to_string())
    }

    /// Blocks, calling `on_event` for every herdr event, until the connection drops.
    pub fn subscribe(&self, mut on_event: impl FnMut()) -> io::Result<()> {
        let kinds = [
            "pane.updated", "pane.created", "pane.closed", "pane.focused", "pane.agent_detected", "pane.exited",
            "pane.moved", "tab.focused", "tab.renamed", "tab.closed", "workspace.focused", "workspace.renamed",
            "workspace.closed", "workspace.reordered",
        ];
        let subs: Vec<Value> = kinds.iter().map(|k| json!({ "type": k })).collect();
        let mut stream = UnixStream::connect(&self.socket)?;
        let req = json!({ "id": "pixbar-events", "method": "events.subscribe", "params": { "subscriptions": subs } });
        stream.write_all(format!("{req}\n").as_bytes())?;
        for line in BufReader::new(stream).lines() {
            if line?.contains("\"event\"") {
                on_event();
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_sort_is_read_from_the_ui_section_of_config_toml() {
        assert_eq!(PanelSort::in_config("onboarding = false\n[ui]\nagent_panel_sort = \"priority\"\n"), Some(PanelSort::Priority));
        assert_eq!(PanelSort::in_config("[ui]\n  agent_panel_sort = 'workspaces'  # the old name\n"), Some(PanelSort::Spaces));
        assert_eq!(PanelSort::in_config("[ui]\n# agent_panel_sort = \"priority\"\n"), None, "commented out, as herdr's sample config has it");
        assert_eq!(PanelSort::in_config("[ui.toast]\nagent_panel_sort = \"priority\"\n"), None, "another section's key");
        assert_eq!(PanelSort::in_config("[ui]\nagent_panel_sort = \"sideways\"\n"), None);
    }

    #[test]
    fn a_click_in_herdr_0_9_is_read_from_the_client_state_file() {
        assert_eq!(PanelSort::in_client_state(r#"{"sidebar_width": 30, "agent_panel_sort": "priority"}"#), Some(PanelSort::Priority));
        assert_eq!(PanelSort::in_client_state(r#"{"sidebar_width": 30}"#), None, "never clicked: config.toml decides");
    }

    #[test]
    fn the_client_state_file_is_named_as_herdr_names_it() {
        assert_eq!(fnv1a(b"a"), 0xaf63dc4c8601ec8c, "FNV-1a, 64 bit");
        let herdr = Herdr { socket: PathBuf::from("/home/me/.config/herdr/herdr.sock") };
        let name = format!("local-{:016x}.json", fnv1a(b"/home/me/.config/herdr/herdr-client.sock"));
        assert!(herdr.client_state().ends_with(format!("herdr/client-shell/{name}")));
    }
}

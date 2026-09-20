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

impl Herdr {
    /// `--socket`, else what herdr exported into its panes, else where herdr itself puts it: under its config
    /// directory, and for a named session (`herdr --session work`) under `sessions/<name>`.
    pub fn new(socket: Option<String>) -> Herdr {
        let pane = std::env::var("HERDR_SOCKET_PATH").ok().filter(|v| !v.is_empty());
        Herdr { socket: socket.or(pane).map(PathBuf::from).unwrap_or_else(Herdr::default_socket) }
    }

    /// Where herdr puts its socket when nothing says otherwise.
    pub fn default_socket() -> PathBuf {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.is_empty());
        let config = var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(var("HOME").unwrap_or_default()).join(".config"));
        match var("HERDR_SESSION") {
            Some(name) => config.join("herdr/sessions").join(name).join("herdr.sock"),
            None => config.join("herdr/herdr.sock"),
        }
    }

    pub fn socket(&self) -> &std::path::Path {
        &self.socket
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

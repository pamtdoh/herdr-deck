//! Changing a live Claude Code session by driving its `/model` picker with keystrokes.
//!
//! There is no control API for a terminal session, so this is the one write path. Tested by hand first:
//! `alt+p` opens the picker as an overlay and leaves a half-typed prompt alone; left/right moves the effort
//! row; up/down moves the model cursor; `s` applies "this session only" and never writes settings.json.
//! Two keys must never be sent: a digit (selects a model AND saves it as the global default at once) and
//! Enter (saves as default). The effort row wraps (right of Ultracode is Low), so every key is followed by a
//! read of the screen, and the sequence is abandoned with Esc the moment the screen is not what we expect.

use std::thread::sleep;
use std::time::{Duration, Instant};

use pixbar_render::{Effort, Model};

use crate::herdr::Herdr;

#[derive(Debug, PartialEq, Clone)]
pub struct PickerView {
    /// Model rows top to bottom, by their label ("Default (recommended)", "Opus (1M context)", "Fable", ...).
    pub rows: Vec<String>,
    pub cursor: usize,
    pub effort: Option<Effort>,
}

/// `None` unless the picker is on screen.
pub fn parse(screen: &str) -> Option<PickerView> {
    // Only the title is a safe marker: at Max the warning text pushes the footer off a short pane.
    if !screen.contains("Select model") {
        return None;
    }
    let (mut rows, mut cursor, mut effort) = (Vec::new(), None, None);
    for line in screen.lines() {
        let t = line.trim_start();
        let (has_cursor, t) = match t.strip_prefix('❯') {
            Some(rest) => (true, rest.trim_start()),
            None => (false, t),
        };
        let digits = t.chars().take_while(char::is_ascii_digit).count();
        if digits > 0 && t[digits..].starts_with(". ") {
            let label = t[digits + 2..].split("  ").next().unwrap_or("").trim_end_matches('✔').trim();
            if has_cursor {
                cursor = Some(rows.len());
            }
            rows.push(label.to_string());
        } else if t.contains(" effort") && t.contains("to adjust") {
            effort = t.split_whitespace().nth(1).and_then(|w| match w.to_ascii_lowercase().as_str() {
                "low" => Some(Effort::Low),
                "medium" => Some(Effort::Med),
                "high" => Some(Effort::High),
                "xhigh" => Some(Effort::XHigh),
                "max" => Some(Effort::Max),
                "ultracode" => Some(Effort::Ultra),
                _ => None,
            });
        }
    }
    Some(PickerView { rows, cursor: cursor?, effort })
}

/// The row whose label goes by the same name as `model`. Never the `Default` row, which is a model too, but
/// whichever the account's default happens to be.
fn row_for(view: &PickerView, model: Model) -> Option<usize> {
    view.rows.iter().position(|row| !row.starts_with("Default") && crate::claude::model_named(row) == model)
}

/// How an error starts when a session's effort ring turned out to lack the stop we were heading for.
pub const SHORT_RING: &str = "this session's effort ring is shorter than expected";

pub enum Change {
    Effort(Effort),
    Model(Model),
}

pub struct Picker<'a> {
    pub herdr: &'a Herdr,
    pub pane: &'a str,
}

/// The keys that drive the picker. They are Claude Code's defaults unless `~/.claude/keybindings.json` binds the
/// actions to others (`s`, for "this session only", is not among the actions that file can move).
#[derive(Debug, PartialEq)]
struct Keys {
    open: Vec<String>,
    less: Vec<String>,
    more: Vec<String>,
    up: Vec<String>,
    down: Vec<String>,
    cancel: Vec<String>,
}

/// The key (or chord) for one of Claude Code's actions. `bindings`: the parsed keybindings file, or `Null`
/// where there is none.
fn binding(bindings: &serde_json::Value, context: &str, action: &str, default: &str) -> Result<Vec<String>, String> {
    let Some(contexts) = bindings["bindings"].as_array() else { return Ok(vec![in_herdrs_words(default)]) };
    let bound = contexts.iter().filter(|c| c["context"] == context).filter_map(|c| c["bindings"].as_object()).flatten();
    let keys: Vec<&str> = bound.filter(|(_, a)| a.as_str() == Some(action)).map(|(k, _)| k.as_str()).collect();
    // A file that does not mention the context at all leaves it as it was.
    let mentioned = contexts.iter().any(|c| c["context"] == context);
    match keys.iter().find(|k| **k == default).or(keys.first()) {
        Some(k) => Ok(k.split_whitespace().map(in_herdrs_words).collect()),
        None if !mentioned => Ok(vec![in_herdrs_words(default)]),
        None => Err(format!("no key is bound to {action} in ~/.claude/keybindings.json, and the panel works Claude Code through its keys")),
    }
}

fn bindings_file() -> serde_json::Value {
    let file = crate::install::claude_dir().map(|d| d.join("keybindings.json"));
    let text = file.and_then(|f| std::fs::read_to_string(f).ok());
    text.and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(serde_json::Value::Null)
}

/// As the user has it bound right now.
pub fn key_for(context: &str, action: &str, default: &str) -> Result<Vec<String>, String> {
    binding(&bindings_file(), context, action, default)
}

impl Keys {
    fn from(bindings: &serde_json::Value) -> Result<Keys, String> {
        let key = |context: &str, action: &str, default: &str| binding(bindings, context, action, default);
        Ok(Keys {
            open: key("Chat", "chat:modelPicker", "meta+p")?,
            less: key("ModelPicker", "modelPicker:decreaseEffort", "left")?,
            more: key("ModelPicker", "modelPicker:increaseEffort", "right")?,
            up: key("Select", "select:previous", "up")?,
            down: key("Select", "select:next", "down")?,
            cancel: key("Select", "select:cancel", "escape")?,
        })
    }

    fn load() -> Result<Keys, String> {
        Keys::from(&bindings_file())
    }
}

/// Claude Code writes `meta+p` and `escape` where herdr's `pane.send_keys` takes `alt+p` and `esc`.
fn in_herdrs_words(key: &str) -> String {
    key.split('+').map(|part| match part {
        "meta" | "option" | "opt" => "alt",
        "escape" => "esc",
        "control" => "ctrl",
        other => other,
    }).collect::<Vec<_>>().join("+")
}

impl Picker<'_> {
    fn view(&self) -> Option<PickerView> {
        self.herdr.screen(self.pane).ok().as_deref().and_then(parse)
    }

    /// Polls the screen until `ok` holds; the TUI needs a few tens of ms to repaint after a key.
    fn wait(&self, what: &str, ok: impl Fn(Option<&PickerView>) -> bool) -> Result<Option<PickerView>, String> {
        let deadline = Instant::now() + Duration::from_millis(1500);
        loop {
            let v = self.view();
            if ok(v.as_ref()) {
                return Ok(v);
            }
            if crate::STOPPING.load(std::sync::atomic::Ordering::SeqCst) {
                return Err("the bridge is stopping".into());
            }
            if Instant::now() > deadline {
                return Err(format!("timed out waiting for {what}"));
            }
            sleep(Duration::from_millis(40));
        }
    }

    fn key(&self, k: &str) -> Result<(), String> {
        self.herdr.send_keys(self.pane, &[k]).map_err(|e| e.to_string())
    }

    /// One binding, which may be a chord of several keys.
    fn press(&self, keys: &[String]) -> Result<(), String> {
        keys.iter().try_for_each(|k| self.key(k))
    }

    pub fn apply(&self, change: Change) -> Result<(), String> {
        // An injected key could answer a permission prompt; never type into a blocked agent.
        match self.herdr.agent_status(self.pane).map_err(|e| e.to_string())?.as_str() {
            "blocked" => return Err("agent is blocked on a prompt".into()),
            _ if self.view().is_some() => return Err("picker already open".into()),
            _ => {}
        }
        let keys = Keys::load()?;
        self.press(&keys.open)?;
        let opened = self.wait("the picker to open (is it still on the key Claude Code gives it, or the one in keybindings.json?)", |v| v.is_some());
        let result = opened.and_then(|v| self.drive(v.unwrap(), change, &keys));
        // Never leave the picker sitting open over the user's prompt, even if we could not make sense of it.
        if result.is_err() && self.herdr.screen(self.pane).is_ok_and(|s| s.contains("Select model")) {
            let _ = self.press(&keys.cancel);
        }
        result
    }

    fn drive(&self, mut view: PickerView, change: Change, keys: &Keys) -> Result<(), String> {
        match change {
            Change::Model(model) => {
                let target = row_for(&view, model).ok_or_else(|| format!("no row for {} in this account's picker ({})", model.word(), view.rows.join(", ")))?;
                while view.cursor != target {
                    let (k, next) = if target > view.cursor { (&keys.down, view.cursor + 1) } else { (&keys.up, view.cursor - 1) };
                    self.press(k)?;
                    view = self.wait("the model cursor to move", |v| v.is_some_and(|v| v.cursor == next))?.unwrap();
                }
            }
            Change::Effort(target) => {
                let mut at = view.effort.ok_or("this model has no effort row")?;
                while at != target {
                    let dir = if target > at { 1 } else { -1 };
                    let next = at.step(dir, true).unwrap();
                    self.press(if dir > 0 { &keys.more } else { &keys.less })?;
                    // If the row lands anywhere else the ring is shorter than we think (no ultracode here)
                    // and it wrapped: bail out, Esc discards the whole thing.
                    view = self.wait("the effort row to change", |v| v.is_some_and(|v| v.effort != Some(at)))?.unwrap();
                    if view.effort != Some(next) {
                        return Err(format!("{SHORT_RING}: the effort row went to {:?} instead of {next:?}", view.effort));
                    }
                    at = next;
                }
            }
        }
        self.key("s")?;
        self.wait("the picker to close", |v| v.is_none()).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_are_claude_codes_own_unless_the_user_moved_them() {
        let defaults = Keys::from(&serde_json::Value::Null).unwrap();
        assert_eq!((defaults.open.as_slice(), defaults.cancel.as_slice()), (["alt+p".to_string()].as_slice(), ["esc".to_string()].as_slice()));
        let moved = serde_json::json!({ "bindings": [
            { "context": "Chat", "bindings": { "ctrl+x m": "chat:modelPicker", "enter": "chat:submit" } },
            { "context": "Select", "bindings": { "j": "select:next", "down": "select:next", "k": "select:previous", "escape": "select:cancel" } },
        ]});
        let keys = Keys::from(&moved).unwrap();
        assert_eq!(keys.open, ["ctrl+x", "m"], "a chord is sent key by key");
        assert_eq!((keys.down.as_slice(), keys.up.as_slice()), (["down".to_string()].as_slice(), ["k".to_string()].as_slice()), "the default where it still holds");
        assert_eq!(keys.more, ["right"], "a context the file does not mention is as it was");
        let unbound = serde_json::json!({ "bindings": [{ "context": "Chat", "bindings": { "enter": "chat:submit" } }] });
        assert!(Keys::from(&unbound).unwrap_err().contains("chat:modelPicker"));
    }

    const SCREEN: &str = "\
  Select model
  Switch between Claude models. Your pick becomes the default for new sessions.
    1. Default (recommended)  Opus 5 with 1M context · Best for everyday, complex tasks
    2. Opus (1M context)      Opus 5 with 1M context · Best for everyday, complex tasks
  ❯ 3. Fable ✔                Fable 5.1 · Most capable for your hardest and longest-running
                              tasks
    4. Sonnet                 Sonnet 5 · Efficient for routine tasks
    5. Haiku                  Haiku 4.5 · Fastest for quick answers
  ◉ xHigh effort ←/→ to adjust
  Enter to set as default · s to use this session only · Esc to cancel";

    #[test]
    // One account's picker in September 2026 (Claude Code 2.1.27x). Yours has other rows; only its shape matters here:
    // a title, numbered rows, a cursor, an effort line.
    fn reads_rows_cursor_and_effort_from_the_real_picker() {
        let v = parse(SCREEN).unwrap();
        assert_eq!(v.rows, ["Default (recommended)", "Opus (1M context)", "Fable", "Sonnet", "Haiku"]);
        assert_eq!((v.cursor, v.effort), (2, Some(Effort::XHigh)));
        assert_eq!((row_for(&v, Model::new("opus")), row_for(&v, Model::new("fable"))), (Some(1), Some(2)));
        assert_eq!((row_for(&v, Model::new("sonnet")), row_for(&v, Model::new("default")), row_for(&v, Model::new("gpt"))), (Some(3), None, None));
        let ultra = SCREEN.replace("◉ xHigh effort", "✦ Ultracode effort");
        assert_eq!(parse(&ultra).unwrap().effort, Some(Effort::Ultra));
        assert_eq!(parse("❯ my half typed draft"), None);
        let no_footer = SCREEN.rsplit_once('\n').unwrap().0;
        assert_eq!(parse(no_footer).unwrap().cursor, 2, "footer scrolled off a short pane");
    }
}

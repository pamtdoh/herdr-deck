//! Typing a slash command into a live Claude Code session: the menu's `/compact` and `/clear`.
//!
//! Tried by hand first, in a disposable session (Claude Code 2.1.278):
//! - The prompt box is the `❯` line between two rules. Empty, it is that character alone; a draft, a queued
//!   message's hint (`Press up to edit queued messages`) and the grey `[name]` after `/rename ` all read as text.
//! - Text typed into a box that holds a draft is added to the draft, and Enter then sends the lot as a prompt.
//!   So nothing is typed unless the box is empty, and Enter is only pressed once the box holds the command alone.
//! - Enter on `/clear` runs it at once, the completion list above the box notwithstanding. In the middle of a
//!   turn it is queued instead and runs when the turn ends, on a reply nobody has read yet: refused.
//! - An injected Enter answers an open permission dialog: refused while the agent is blocked.

use std::thread::sleep;
use std::time::{Duration, Instant};

use crate::herdr::Herdr;

/// What the prompt box holds: `Some("")` when it is empty, `None` when there is no box on the screen (a dialog
/// is up, or this is not Claude Code) or the box runs over several lines.
pub fn prompt_box(screen: &str) -> Option<String> {
    let lines: Vec<&str> = screen.lines().collect();
    let rule = |l: &&str| l.starts_with('─');
    let at = (1..lines.len().saturating_sub(1)).rev().find(|&i| lines[i].starts_with('❯') && rule(&lines[i - 1]) && rule(&lines[i + 1]))?;
    // Claude Code puts a no-break space after the prompt character.
    let held = lines[at].trim_start_matches('❯').replace('\u{a0}', " ");
    Some(held.strip_prefix(' ').unwrap_or(&held).trim_end().to_string())
}

fn wait(herdr: &Herdr, pane: &str, what: &str, ok: impl Fn(&str) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_millis(1500);
    loop {
        let held = herdr.screen(pane).ok().as_deref().and_then(prompt_box);
        if held.as_deref().is_some_and(&ok) {
            return Ok(());
        }
        if crate::STOPPING.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("the bridge is stopping".into());
        }
        if Instant::now() > deadline {
            return Err(format!("timed out waiting for {what} (the prompt box holds {held:?})"));
        }
        sleep(Duration::from_millis(40));
    }
}

/// Types `command` into the session's empty prompt box and sends it.
pub fn slash(herdr: &Herdr, pane: &str, command: &str) -> Result<(), String> {
    match herdr.agent_status(pane).map_err(|e| e.to_string())?.as_str() {
        "blocked" => return Err("agent is blocked on a prompt".into()),
        "working" => return Err("mid-turn it would be queued and run when the turn ends".into()),
        _ => {}
    }
    let screen = herdr.screen(pane).map_err(|e| e.to_string())?;
    match prompt_box(&screen) {
        Some(held) if held.is_empty() => {}
        Some(held) => return Err(format!("the prompt box is not empty ({held:?}); typing there would add to it")),
        None => return Err("no prompt box on the screen".into()),
    }
    let submit = crate::picker::key_for("Chat", "chat:submit", "enter")?;
    herdr.send_text(pane, command).map_err(|e| e.to_string())?;
    if let Err(e) = wait(herdr, pane, "the command to show in the prompt box", |held| held == command) {
        // Whatever of ours is sitting there must not wait for somebody's Enter. Only if it is ours alone.
        let ours = herdr.screen(pane).ok().as_deref().and_then(prompt_box).is_some_and(|held| !held.is_empty() && command.starts_with(&held));
        if ours {
            let _ = herdr.send_keys(pane, &["ctrl+u"]);
        }
        return Err(e);
    }
    let keys: Vec<&str> = submit.iter().map(String::as_str).collect();
    herdr.send_keys(pane, &keys).map_err(|e| e.to_string())?;
    wait(herdr, pane, "the command to leave the prompt box", str::is_empty)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RULE: &str = "─────────────────────────────────────────";

    fn screen(above: &str, held: &str) -> String {
        format!("{above}\n{RULE}\n{held}\n{RULE}\n  boop@ ~ · Fable 5.1\n  ⏵⏵ auto mode on (shift+tab to cycle)\n")
    }

    #[test]
    fn the_prompt_box_is_the_line_between_the_rules() {
        assert_eq!(prompt_box(&screen("● done", "❯")).as_deref(), Some(""));
        assert_eq!(prompt_box(&screen("● done", "❯\u{a0}hello draft")).as_deref(), Some("hello draft"));
        assert_eq!(prompt_box(&screen("● done", "❯\u{a0}/rename  [name]")).as_deref(), Some("/rename  [name]"));
        assert_eq!(prompt_box(&screen("● done", "❯ Press up to edit queued messages")).as_deref(), Some("Press up to edit queued messages"));
        // What was sent earlier is echoed above with the same character: only the boxed line counts.
        assert_eq!(prompt_box(&screen("❯ /clear\n  ⎿  done", "❯")).as_deref(), Some(""));
        assert_eq!(prompt_box("❯ /clear\n● a reply\n  Do you want to proceed?\n❯ 1. Yes\n  2. No\n"), None, "a dialog, no box");
        assert_eq!(prompt_box(&format!("{RULE}\n❯\u{a0}two\n  lines\n{RULE}\n")), None, "a draft of several lines");
    }
}

//! What Claude Code tells us about a session: the data it hands its status line.
//!
//! Claude Code runs the status line command whenever the model, the effort or the token usage changes, with the
//! live values on stdin. `pixbar-bridge statusline`, called from that command, keeps the newest input of every
//! session in `~/.cache/pixbar/sessions/<session id>.json`; herdr gives the session id per pane. Ultracode is
//! invisible there (the effort says xhigh), which is why the bridge also looks at the pane's screen.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use pixbar_render::{Effort, Limit, Model};
use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub struct SessionInfo {
    pub model: Model,
    /// `false`: Claude Code reports no effort for this model, which means it has no levels.
    pub has_effort: bool,
    pub effort: Effort,
    pub ctx_used: u32,
    pub ctx_window: u32,
    /// No reply yet since the session started, was cleared or was compacted: nothing is cached for a model
    /// switch to throw away.
    pub fresh: bool,
    /// The name given with `/rename`; empty if it never was.
    pub session: String,
    pub cost_cents: u32,
    /// The account's 5-hour and 7-day usage windows as this session last heard of them.
    windows: [Option<Window>; 2],
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Window {
    used_pct: u8,
    /// Unix seconds.
    resets_at: u64,
}

fn dir() -> Option<PathBuf> {
    let base = std::env::var("XDG_CACHE_HOME").ok().filter(|s| !s.is_empty()).map(PathBuf::from);
    let base = base.or_else(|| Some(PathBuf::from(std::env::var("HOME").ok()?).join(".cache")))?;
    Some(base.join("pixbar/sessions"))
}

/// The session id becomes a file name, and it comes from outside.
pub fn file_of(session: &str) -> Option<PathBuf> {
    let plain = !session.is_empty() && session.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    plain.then(|| dir().map(|d| d.join(format!("{session}.json")))).flatten()
}

/// The `statusline` subcommand: keeps one status line input. Renamed into place, so the bridge never reads half of it.
pub fn keep(input: &str) -> io::Result<()> {
    let v: Value = serde_json::from_str(input)?;
    let file = v["session_id"].as_str().and_then(file_of).ok_or_else(|| io::Error::other("no session_id"))?;
    std::fs::create_dir_all(file.parent().unwrap_or(Path::new(".")))?;
    let tmp = file.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&tmp, input)?;
    std::fs::rename(tmp, file)
}

/// Sessions end without saying so; their files are small, but there is one for every session ever started.
pub fn forget_old() {
    const KEEP: Duration = Duration::from_secs(30 * 24 * 3600);
    for entry in dir().and_then(|d| std::fs::read_dir(d).ok()).into_iter().flatten().flatten() {
        if entry.metadata().and_then(|m| m.modified()).is_ok_and(|m| m.elapsed().is_ok_and(|age| age > KEEP)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[derive(Default)]
pub struct Sessions {
    /// session -> (its transcript's mtime and length, the context its last compaction left)
    compacted: HashMap<String, (SystemTime, u64, Option<u32>)>,
    /// The models the button steps through, and when that was last read from the config file.
    models: Option<(SystemTime, Vec<Model>)>,
    /// The usage windows belong to the account, and an idle session's idea of them goes stale: the newest
    /// report of any session counts for all. Each window on its own: a report that carries only one of them
    /// says nothing about the other.
    windows: [Option<(SystemTime, Window)>; 2],
}

impl Sessions {
    /// `None`: this session's status line has not been heard from.
    pub fn info(&mut self, session: &str) -> Option<SessionInfo> {
        let file = file_of(session)?;
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).ok()?).ok()?;
        let mut info = parse(&v)?;
        // A session that has not had a reply yet reports no windows; that is no news about the account.
        if let Ok(at) = std::fs::metadata(&file).and_then(|m| m.modified()) {
            self.heard(at, info.windows);
        }
        if info.fresh {
            // Claude Code reports no usage between a compaction and the next reply; the transcript knows what was kept.
            info.ctx_used = v["transcript_path"].as_str().and_then(|t| self.compacted_to(session, Path::new(t))).unwrap_or(0);
        }
        Some(info)
    }

    /// What the panel's model button leads to from `now`: the next of the models it steps through, round and
    /// round. Which models those are is `~/.config/pixbar/models` if there is one (a name per line, as Claude
    /// Code's picker calls them: `Opus`, `Sonnet`), else `DEFAULT_MODELS`.
    pub fn next_model(&mut self, now: Model) -> Option<Model> {
        const FRESH: Duration = Duration::from_secs(10);
        if self.models.as_ref().is_none_or(|(at, _)| at.elapsed().is_ok_and(|age| age > FRESH)) {
            self.models = Some((SystemTime::now(), chosen_models().unwrap_or_else(default_models)));
        }
        next_in(&self.models.as_ref()?.1, now)
    }

    fn heard(&mut self, at: SystemTime, windows: [Option<Window>; 2]) {
        for (known, new) in self.windows.iter_mut().zip(windows) {
            if let Some(new) = new.filter(|_| known.is_none_or(|(newest, _)| at >= newest)) {
                *known = Some((at, new));
            }
        }
    }

    /// The 5-hour and the 7-day window. A window whose end has passed is unknown until the next reply reports
    /// the one that follows it.
    pub fn limits(&self, now: SystemTime) -> [Option<Limit>; 2] {
        let now = now.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        self.windows.map(|w| {
            let (_, w) = w?;
            let left = w.resets_at.checked_sub(now).filter(|&s| s > 0)?;
            Some(Limit { used_pct: w.used_pct, resets_in_min: left.div_ceil(60) as u32 })
        })
    }

    fn compacted_to(&mut self, session: &str, transcript: &Path) -> Option<u32> {
        let meta = std::fs::metadata(transcript).ok()?;
        let stamp = (meta.modified().ok()?, meta.len());
        if let Some((m, len, post)) = self.compacted.get(session) {
            if (*m, *len) == stamp {
                return *post;
            }
        }
        let post = last_compaction(transcript, 1 << 20);
        self.compacted.insert(session.to_string(), (stamp.0, stamp.1, post));
        post
    }
}

/// Where `~/.config/pixbar/models` says nothing, these are what the middle button steps between. Deliberately
/// a fixed pair rather than a look at what this machine has been on: that told a freshly set up machine it had
/// only ever used one model, which left the button with nowhere to go and no way to tell why.
const DEFAULT_MODELS: [&str; 2] = ["Opus", "Fable"];

fn chosen_models() -> Option<Vec<Model>> {
    let text = std::fs::read_to_string(crate::install::config_dir()?.join("models")).ok()?;
    let models: Vec<Model> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).map(model_named).collect();
    (!models.is_empty()).then_some(models)
}

fn default_models() -> Vec<Model> {
    DEFAULT_MODELS.iter().copied().map(Model::new).collect()
}

/// The next model after `now`, round and round; `None` where that would not move, so the button knocks
/// instead of sending a change to the model the session is already on.
fn next_in(models: &[Model], now: Model) -> Option<Model> {
    let next = match models.iter().position(|&m| m == now) {
        Some(i) => models[(i + 1) % models.len()],
        // On something else entirely (Sonnet, a model of the day): the first of ours is where the button goes.
        None => *models.first()?,
    };
    (next != now).then_some(next)
}

pub fn parse(v: &Value) -> Option<SessionInfo> {
    let model = model_of(v)?;
    // Absent for a model that has no effort levels. A level this build has no word for is shown as the nearest
    // thing to nothing said, and logged, so that a new one is noticed.
    let level = v["effort"]["level"].as_str();
    let effort = match level {
        Some("low") => Effort::Low,
        Some("medium") => Effort::Med,
        Some("high") | None => Effort::High,
        Some("xhigh") => Effort::XHigh,
        Some("max") => Effort::Max,
        Some(other) => {
            static SAID: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
            let mut said = SAID.lock().unwrap();
            if !said.iter().any(|s| s == other) {
                eprintln!("Claude Code reports an effort level this bridge has no word for: {other:?} (shown as HIGH)");
                said.push(other.to_string());
            }
            Effort::High
        }
    };
    let context = &v["context_window"];
    // What was sent plus what came back, like Claude Code's own count: the reply is part of the context from now on.
    let usage = &context["current_usage"];
    let used: u64 = ["input_tokens", "output_tokens", "cache_creation_input_tokens", "cache_read_input_tokens"]
        .iter()
        .map(|k| usage[*k].as_u64().unwrap_or(0))
        .sum();
    let window = |name: &str| {
        let w = &v["rate_limits"][name];
        Some(Window { used_pct: w["used_percentage"].as_f64()?.round().clamp(0.0, 100.0) as u8, resets_at: w["resets_at"].as_u64()? })
    };
    Some(SessionInfo {
        model,
        has_effort: level.is_some(),
        effort,
        ctx_used: used as u32,
        ctx_window: context["context_window_size"].as_u64().unwrap_or(200_000) as u32,
        fresh: usage.is_null(),
        session: v["session_name"].as_str().unwrap_or_default().to_string(),
        cost_cents: (v["cost"]["total_cost_usd"].as_f64().unwrap_or(0.0) * 100.0).round() as u32,
        windows: [window("five_hour"), window("seven_day")],
    })
}

/// The newest compaction marker in the last `bytes` of a transcript.
fn last_compaction(transcript: &Path, bytes: u64) -> Option<u32> {
    let mut f = File::open(transcript).ok()?;
    let len = f.metadata().ok()?.len();
    f.seek(SeekFrom::Start(len.saturating_sub(bytes))).ok()?;
    let mut tail = Vec::new();
    f.read_to_end(&mut tail).ok()?;
    // The first line of the window is usually cut in half; it simply fails to parse.
    String::from_utf8_lossy(&tail).lines().rev().find_map(compacted_to)
}

/// `/compact` leaves a marker that records how large the new context is.
pub fn compacted_to(line: &str) -> Option<u32> {
    if !line.contains("\"compact_boundary\"") {
        return None;
    }
    let v: Value = serde_json::from_str(line).ok()?;
    (v["subtype"] == "compact_boundary").then(|| v["compactMetadata"]["postTokens"].as_u64().unwrap_or(0) as u32)
}

/// The panel only knows the two models its button toggles between; anything else is shown as Opus.
/// The panel's name for a model, from what Claude Code calls it: `Opus 5 (1M context)` is `OPUS`, `Claude
/// Sonnet 4.5` is `SONNET`, the id `us.anthropic.claude-haiku-4-5-v1:0` is `HAIKU`. The first word that is a name.
pub fn model_named(name: &str) -> Model {
    let word = name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .find(|w| w.chars().next().is_some_and(|c| c.is_ascii_alphabetic()) && !["claude", "anthropic", "us", "eu", "apac", "global"].contains(&w.to_ascii_lowercase().as_str()));
    Model::new(word.unwrap_or(name))
}

fn model_of(v: &Value) -> Option<Model> {
    let said = v["model"]["display_name"].as_str().filter(|n| !n.is_empty()).or(v["model"]["id"].as_str())?;
    Some(model_named(said))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_status_line_input_gives_model_effort_and_context() {
        let v: Value = serde_json::from_str(
            r#"{"session_id":"s","session_name":"parser","model":{"id":"claude-fable-5-1[1m]","display_name":"Fable 5.1"},"effort":{"level":"xhigh"},
               "cost":{"total_cost_usd":12.34192},"rate_limits":{"five_hour":{"used_percentage":1,"resets_at":1700000600},"seven_day":{"used_percentage":28.6,"resets_at":1700295200}},
               "context_window":{"context_window_size":1000000,"used_percentage":10,
               "current_usage":{"input_tokens":2,"output_tokens":1389,"cache_creation_input_tokens":1460,"cache_read_input_tokens":102862}}}"#,
        )
        .unwrap();
        let info = parse(&v).unwrap();
        assert_eq!((info.model.word(), info.has_effort, info.effort, info.ctx_used, info.ctx_window, info.fresh), ("FABLE", true, Effort::XHigh, 105_713, 1_000_000, false));
        assert_eq!((info.session.as_str(), info.cost_cents), ("parser", 1_234));

        // The windows count down to their end, and past it nothing is known.
        let at = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        let mut sessions = Sessions::default();
        sessions.heard(at(2), info.windows);
        let seven_days = Some(Limit { used_pct: 29, resets_in_min: (1_700_295_200 - 1_700_000_000) / 60 });
        assert_eq!(sessions.limits(at(1_700_000_000)), [Some(Limit { used_pct: 1, resets_in_min: 10 }), seven_days]);
        assert_eq!(sessions.limits(at(1_700_000_600))[0], None);
        assert_eq!(Sessions::default().limits(at(0)), [None, None]);

        // A later report that carries one window leaves the other as it was; an older report changes nothing.
        let week = Window { used_pct: 40, resets_at: 1_700_295_200 };
        sessions.heard(at(5), [None, Some(week)]);
        sessions.heard(at(1), [Some(Window { used_pct: 99, resets_at: 1_700_000_600 }), None]);
        let now = sessions.limits(at(1_700_000_000));
        assert_eq!((now[0].map(|l| l.used_pct), now[1].map(|l| l.used_pct)), (Some(1), Some(40)));

        // Before the first reply, and between a compaction and the next one. No effort: a model without levels.
        let v: Value = serde_json::from_str(r#"{"model":{"id":"claude-opus-5"},"context_window":{"context_window_size":200000,"current_usage":null}}"#).unwrap();
        let info = parse(&v).unwrap();
        assert_eq!((info.model.word(), info.has_effort, info.effort, info.ctx_used, info.ctx_window, info.fresh), ("OPUS", false, Effort::High, 0, 200_000, true));
        assert_eq!((info.session.as_str(), info.cost_cents, info.windows), ("", 0, [None, None]));
        assert_eq!(parse(&Value::Null), None);
    }

    #[test]
    fn the_model_button_steps_between_opus_and_fable_by_default() {
        let models = default_models();
        let (opus, fable) = (Model::new("opus"), Model::new("fable"));
        assert_eq!(models, [opus, fable], "a fixed pair, not whatever this machine happens to have run");
        assert_eq!(next_in(&models, opus), Some(fable));
        assert_eq!(next_in(&models, fable), Some(opus), "and round again");
        // On a model that is not in the list at all, the button leads to the first of them.
        assert_eq!(next_in(&models, Model::new("sonnet")), Some(opus));
        // A list of one has nowhere to go: the panel knocks rather than send a change that would not move.
        assert_eq!(next_in(&[opus], opus), None);
        assert_eq!(next_in(&[], opus), None);
    }

    #[test]
    fn models_go_by_the_first_word_that_is_a_name() {
        for (said, word) in [
            ("Opus 5 (1M context)", "OPUS"),
            ("Fable 5.1", "FABLE"),
            ("Claude Sonnet 4.5", "SONNET"),
            ("claude-haiku-4-5-20251001", "HAIKU"),
            ("us.anthropic.claude-opus-5-v1:0", "OPUS"),
            ("my-own-gateway-model", "MY"),
            ("4o", "4O"),
        ] {
            assert_eq!(model_named(said).word(), word, "{said}");
        }
    }

    #[test]
    fn the_newest_compaction_marker_says_what_was_kept() {
        let older = r#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"auto","preTokens":900000,"postTokens":40000}}"#;
        let marker = r#"{"type":"system","subtype":"compact_boundary","compactMetadata":{"trigger":"manual","preTokens":512712,"postTokens":17577}}"#;
        let quoted = r#"{"type":"user","message":{"content":"grep said \"compact_boundary\" appears twice"}}"#;
        assert_eq!(compacted_to(marker), Some(17_577));
        assert_eq!(compacted_to(quoted), None, "only the marker itself counts, not text that mentions it");

        let path = std::env::temp_dir().join(format!("pixbar-compact-test-{}.jsonl", std::process::id()));
        std::fs::write(&path, format!("{older}\n{marker}\n{quoted}\n")).unwrap();
        let post = last_compaction(&path, 1 << 20);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(post, Some(17_577));
    }

    #[test]
    fn a_session_id_cannot_leave_the_directory() {
        // Wherever the cache directory is on the machine that runs this test.
        std::env::set_var("XDG_CACHE_HOME", std::env::temp_dir());
        assert!(file_of("0a1b2c3d-0000-4000-8000-00000000abcd").is_some_and(|p| p.ends_with("pixbar/sessions/0a1b2c3d-0000-4000-8000-00000000abcd.json")));
        assert_eq!(file_of("../../etc/passwd"), None);
        assert_eq!(file_of(""), None);
    }
}

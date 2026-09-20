//! What the host knows about herdr and Claude Code, as pushed to the device.

use serde::{Deserialize, Serialize};

use crate::settings::NameOf;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Idle,
    Working,
    Blocked,
    Done,
    /// Also what a status this build has not heard of reads as: a newer host must not cost the panel its agents.
    #[serde(other)]
    Unknown,
}

/// A model as the panel knows it: the short name its host made of Claude Code's own (`OPUS`, `SONNET`). Which
/// models there are is the account's business and changes over the months, so the panel has no list of them: it
/// draws the name it is given, and asks for the one it is told the middle button leads to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Model([u8; Model::LETTERS]);

impl Model {
    /// As many as fit beside an effort word on a resting row, and in the big face on the overlay.
    pub const LETTERS: usize = 6;

    /// The letters and digits of `name`, in capitals.
    pub fn new(name: &str) -> Model {
        let mut letters = [0u8; Model::LETTERS];
        for (slot, c) in letters.iter_mut().zip(name.chars().filter(char::is_ascii_alphanumeric)) {
            *slot = c.to_ascii_uppercase() as u8;
        }
        Model(letters)
    }

    pub fn word(&self) -> &str {
        let n = self.0.iter().position(|&b| b == 0).unwrap_or(Model::LETTERS);
        std::str::from_utf8(&self.0[..n]).unwrap_or_default()
    }
}

impl Serialize for Model {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.word().to_ascii_lowercase())
    }
}

impl<'de> Deserialize<'de> for Model {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Model, D::Error> {
        Ok(Model::new(&String::deserialize(d)?))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effort {
    Low,
    Med,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
    /// Claude Code's ultracode: the stop past max on its effort slider. Sends xhigh to the model and
    /// has Claude orchestrate multi-agent workflows; the statusline still reports it as xhigh.
    Ultra,
}

impl Effort {
    pub const ALL: [Effort; 6] =
        [Effort::Low, Effort::Med, Effort::High, Effort::XHigh, Effort::Max, Effort::Ultra];

    /// The stops a session offers: ultracode exists only where workflows and xhigh are available.
    pub fn stops(ultra_ok: bool) -> &'static [Effort] {
        &Effort::ALL[..if ultra_ok { 6 } else { 5 }]
    }

    pub fn index(self) -> usize {
        self as usize
    }

    /// One step up or down; `None` at the ends. Claude Code's own picker wraps here (right of ultracode
    /// is low); the rail clamps instead, and the bridge steps the picker by the exact difference.
    pub fn step(self, dir: i32, ultra_ok: bool) -> Option<Effort> {
        let i = self.index() as i32 + dir;
        (0..Effort::stops(ultra_ok).len() as i32).contains(&i).then(|| Effort::ALL[i as usize])
    }

    pub fn word(self) -> &'static str {
        ["LOW", "MED", "HIGH", "XHIGH", "MAX", "ULTRA"][self.index()]
    }
}

/// One of the account's usage windows, as Claude Code reports it to its status line.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limit {
    pub used_pct: u8,
    /// Minutes until the window starts over. Counted by the host: the panel's clock is nobody's business.
    pub resets_in_min: u32,
}

impl Limit {
    /// `45M`, `3H`, `6D`: the largest unit that has started.
    pub fn resets_in(self) -> String {
        match self.resets_in_min {
            m if m < 60 => format!("{m}M"),
            m if m < 48 * 60 => format!("{}H", m / 60),
            m => format!("{}D", m / (24 * 60)),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Agent {
    /// herdr workspace label and tab label.
    pub space: String,
    pub tab: String,
    /// Last component of the working directory, and the session title Claude Code sets on its terminal.
    #[serde(default)]
    pub dir: String,
    #[serde(default)]
    pub title: String,
    /// The name the session was given with `/rename`.
    #[serde(default)]
    pub session: String,
    /// What the session has cost so far, in cents.
    #[serde(default)]
    pub cost_cents: u32,
    /// The account's 5-hour and 7-day usage windows; `None` where Claude Code reports none (API billing).
    #[serde(default)]
    pub limit_5h: Option<Limit>,
    #[serde(default)]
    pub limit_7d: Option<Limit>,
    /// Not replied yet since it started, was cleared or was compacted: a model switch costs it nothing.
    /// (A token threshold cannot tell that: compacting leaves ~17K, a session's first reply ~30K.)
    #[serde(default)]
    pub fresh: bool,
    /// `false`: Claude Code has told the host nothing about this session (its status line does not reach the
    /// bridge): model, effort and context are not known, and nothing is sent into it.
    #[serde(default = "yes")]
    pub reported: bool,
    /// `false`: a model without effort levels.
    #[serde(default = "yes")]
    pub has_effort: bool,
    /// What the middle button switches this session to; `None`: the host knows of nothing to switch to.
    #[serde(default)]
    pub next_model: Option<Model>,
    pub status: Status,
    pub model: Model,
    pub effort: Effort,
    pub ctx_used: u32,
    pub ctx_window: u32,
    /// Whether this session's effort ring has the ultracode stop.
    pub ultra_ok: bool,
}

fn yes() -> bool {
    true
}

impl Agent {
    /// What the panel calls this agent. A host that does not send the dir or title, or a session that was never
    /// named, falls back to the space.
    pub fn label(&self, name: NameOf) -> String {
        let or_space = |s: &String| if s.is_empty() { self.space.clone() } else { s.clone() };
        match name {
            NameOf::Space => self.space.clone(),
            NameOf::Tab => self.tab.clone(),
            NameOf::Both => format!("{}·{}", self.space, self.tab),
            NameOf::Dir => or_space(&self.dir),
            NameOf::Title => or_space(&self.title),
            NameOf::Session => or_space(&self.session),
        }
    }

    /// Rounded to the nearest percent, as Claude Code does: 37.5K of 1M is 4 %, not 3.
    pub fn ctx_pct(&self) -> u32 {
        let (used, window) = (self.ctx_used as u64, self.ctx_window as u64);
        (used * 100 + window / 2).checked_div(window).map_or(0, |pct| pct.min(100) as u32)
    }
}

/// What the panel's MCU says about its own power; `None` until it has answered (or once it has gone quiet).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Power {
    /// Charge as the MCU reckons it.
    pub percent: Option<u8>,
    pub millivolts: Option<u16>,
    pub on_usb: Option<bool>,
}

/// Agents in herdr sidebar order, plus which one herdr has focused.
#[derive(Clone, Debug, Default)]
pub struct World {
    pub agents: Vec<Agent>,
    pub focused: usize,
}

/// `$0.42`, `$12.34`, `$1234`.
pub fn dollars_short(cents: u32) -> String {
    if cents >= 100_000 {
        format!("${}", cents / 100)
    } else {
        format!("${}.{:02}", cents / 100, cents % 100)
    }
}

/// `104K`, `1.2M` — token counts as the statusline prints them.
pub fn tokens_short(n: u32) -> String {
    if n >= 1_000_000 {
        let tenths = (n as u64 + 50_000) / 100_000;
        if tenths.is_multiple_of(10) {
            format!("{}M", tenths / 10)
        } else {
            format!("{}.{}M", tenths / 10, tenths % 10)
        }
    } else {
        format!("{}K", (n + 500) / 1000)
    }
}

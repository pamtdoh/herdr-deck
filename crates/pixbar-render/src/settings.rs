//! What the user can change on the panel itself (knob long-press), and the text form the device keeps it in.

/// The agent blocks in the left strip: smaller or touching blocks, more agents.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Blocks {
    /// Edge in pixels.
    pub size: u8,
    /// 1 px between blocks. Without it neighbours touch and every other block is drawn a shade darker.
    pub gap: bool,
}

impl Blocks {
    /// In the order the settings screen steps through them: big to small, spaced before touching.
    pub const CHOICES: [Blocks; 6] = [
        Blocks { size: 4, gap: true },
        Blocks { size: 4, gap: false },
        Blocks { size: 3, gap: true },
        Blocks { size: 3, gap: false },
        Blocks { size: 2, gap: true },
        Blocks { size: 2, gap: false },
    ];

    /// Pitch, then the columns and rows that fit the 9x16 strip.
    pub(crate) fn grid(self) -> (i32, i32, i32) {
        let gap = self.gap as i32;
        let pitch = self.size as i32 + gap;
        (pitch, (9 + gap) / pitch, (16 + gap) / pitch)
    }

    /// How many agents the strip shows at once; past that it pages along with the focus.
    pub fn capacity(self) -> usize {
        let (_, cols, rows) = self.grid();
        (cols * rows) as usize
    }
}

/// What one of the two resting rows shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Show {
    /// `FABLE XHIGH`
    Model,
    /// `104K 10%`
    Context,
    Name,
    /// `5H 12% 3H`: the account's 5-hour usage window and how long until it starts over.
    Limit5h,
    /// `7D 29% 4D`
    Limit7d,
    /// `$12.34`: what the session has cost.
    Cost,
}

impl Show {
    const ALL: [(Show, &'static str, &'static str); 6] = [
        (Show::Model, "model", "MODEL"),
        (Show::Context, "context", "CTX"),
        (Show::Name, "name", "NAME"),
        (Show::Limit5h, "limit5h", "5H"),
        (Show::Limit7d, "limit7d", "7D"),
        (Show::Cost, "cost", "COST"),
    ];

    pub const CHOICES: [Show; 6] = [Show::Model, Show::Context, Show::Name, Show::Limit5h, Show::Limit7d, Show::Cost];

    pub fn word(self) -> &'static str {
        Show::ALL.iter().find(|s| s.0 == self).map_or("", |s| s.2)
    }
}

/// How a resting row is drawn, so that the two rows need not look alike.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Style {
    /// Lit text on black.
    Plain,
    /// The same at reduced brightness, so the row steps back behind the other one.
    Dim,
    /// Black text on a rounded, lit card.
    Card,
    /// Lit text on a rounded, faintly lit card: the card without giving up the 3x5 font's legibility.
    Tint,
}

impl Style {
    pub const ALL: [(Style, &'static str, &'static str); 4] = [
        (Style::Plain, "plain", "PLAIN"),
        (Style::Dim, "dim", "DIM"),
        (Style::Card, "card", "CARD"),
        (Style::Tint, "tint", "TINT"),
    ];

    /// Card and tint rows are 7 px tall and sit 1 px further out than text-only rows.
    pub(crate) fn carded(self) -> bool {
        matches!(self, Style::Card | Style::Tint)
    }

    pub fn word(self) -> &'static str {
        Style::ALL.iter().find(|s| s.0 == self).map_or("", |s| s.2)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Row {
    pub show: Show,
    pub style: Style,
}

/// What an agent is called on the panel. herdr's sidebar shows space and tab; the rest comes with its snapshot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NameOf {
    Space,
    Tab,
    Both,
    /// Last component of the agent's working directory.
    Dir,
    /// The session title Claude Code sets on its terminal.
    Title,
    /// The name the session was given with `/rename`.
    Session,
}

impl NameOf {
    pub const ALL: [(NameOf, &'static str, &'static str); 6] = [
        (NameOf::Space, "space", "SPACE"),
        (NameOf::Tab, "tab", "TAB"),
        (NameOf::Both, "both", "BOTH"),
        (NameOf::Dir, "dir", "DIR"),
        (NameOf::Title, "title", "TITLE"),
        (NameOf::Session, "session", "SESS"),
    ];

    pub fn word(self) -> &'static str {
        NameOf::ALL.iter().find(|n| n.0 == self).map_or("", |n| n.2)
    }
}

pub const BRIGHTNESS_MIN: u8 = 10;
pub const BRIGHTNESS_STEP: u8 = 10;
pub const LINGER_CHOICES_S: [u8; 5] = [2, 3, 5, 10, 20];
/// The slowest still sits well inside the device's 10 s host timeout: the refresh doubles as the keepalive.
pub const REFRESH_CHOICES_MS: [u16; 5] = [250, 500, 1000, 2000, 5000];

/// Which host's agents the panel shows while several are connected: empty means all of them, which is the
/// default and what the panel did before there was anything to pick. The name is kept as bytes rather than a
/// `String` so that `Settings` stays `Copy` like every other setting; a longer name is remembered, and
/// matched, by as much of it as fits.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HostPick([u8; HostPick::MAX]);

impl HostPick {
    /// Longer than the names the HOSTS row can show anyway.
    pub const MAX: usize = 31;

    pub fn new(name: &str) -> HostPick {
        let mut bytes = [0u8; HostPick::MAX];
        // A cut has to land on a character boundary, so that what is kept is still a str.
        let end = name.char_indices().map(|(i, c)| i + c.len_utf8()).take_while(|&e| e <= HostPick::MAX).last().unwrap_or(0);
        bytes[..end].copy_from_slice(&name.as_bytes()[..end]);
        HostPick(bytes)
    }

    /// Nothing picked: every connected host's agents are shown, one after another.
    pub fn is_all(&self) -> bool {
        self.0[0] == 0
    }

    pub fn as_str(&self) -> &str {
        let end = self.0.iter().position(|&b| b == 0).unwrap_or(HostPick::MAX);
        std::str::from_utf8(&self.0[..end]).unwrap_or("")
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Settings {
    /// Percent, 10..=100.
    pub brightness: u8,
    pub blocks: Blocks,
    /// Top and bottom resting row.
    pub rows: [Row; 2],
    pub name: NameOf,
    /// How long the agent's name stays up after the knob was turned.
    pub linger_s: u8,
    /// How often hosts re-read what herdr has no event for (model, effort, context). The device only passes it on.
    pub refresh_ms: u16,
    /// Whose agents to show, when more than one machine is connected.
    pub host: HostPick,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            brightness: 70,
            blocks: Blocks { size: 3, gap: true },
            rows: [Row { show: Show::Model, style: Style::Plain }, Row { show: Show::Context, style: Style::Plain }],
            name: NameOf::Both,
            linger_s: 10,
            refresh_ms: 1000,
            host: HostPick::default(),
        }
    }
}

/// The neighbour of `current` in `choices`; at the ends it either wraps or stays.
pub(crate) fn step<T: Copy + PartialEq>(choices: &[T], current: T, dir: i32, wrap: bool) -> T {
    let (i, n) = (choices.iter().position(|&c| c == current).unwrap_or(0) as i32, choices.len() as i32);
    let next = if wrap { (i + dir).rem_euclid(n) } else { (i + dir).clamp(0, n - 1) };
    choices[next as usize]
}

impl Settings {
    /// `key=value` lines, as kept in the device's config file.
    pub fn to_config(&self) -> String {
        let show = |s: Show| Show::ALL.iter().find(|x| x.0 == s).map_or("", |x| x.1);
        let style = |s: Style| Style::ALL.iter().find(|x| x.0 == s).map_or("", |x| x.1);
        let name = NameOf::ALL.iter().find(|n| n.0 == self.name).map_or("", |n| n.1);
        format!(
            "brightness={}\nblocks={}\nblocks_gap={}\nrow1={}\nrow1_style={}\nrow2={}\nrow2_style={}\nname={name}\nlinger_s={}\nrefresh_ms={}\nhost={}\n",
            self.brightness,
            self.blocks.size,
            self.blocks.gap as u8,
            show(self.rows[0].show),
            style(self.rows[0].style),
            show(self.rows[1].show),
            style(self.rows[1].style),
            self.linger_s,
            self.refresh_ms,
            self.host.as_str(),
        )
    }

    /// Anything missing, unknown or outside what the settings screen can reach keeps its default.
    pub fn from_config(text: &str) -> Settings {
        let mut s = Settings::default();
        for (key, v) in text.lines().filter_map(|l| l.split_once('=')) {
            let flag = || v == "1";
            let show = |old: Show| Show::ALL.iter().find(|x| x.1 == v).map_or(old, |x| x.0);
            let style = |old: Style| Style::ALL.iter().find(|x| x.1 == v).map_or(old, |x| x.0);
            match key {
                "brightness" => s.brightness = v.parse().map_or(s.brightness, |b: u8| b.clamp(BRIGHTNESS_MIN, 100)),
                "blocks" => s.blocks.size = v.parse().ok().filter(|n| (2..=4).contains(n)).unwrap_or(s.blocks.size),
                "blocks_gap" => s.blocks.gap = flag(),
                "row1" => s.rows[0].show = show(s.rows[0].show),
                "row1_style" => s.rows[0].style = style(s.rows[0].style),
                "row2" => s.rows[1].show = show(s.rows[1].show),
                "row2_style" => s.rows[1].style = style(s.rows[1].style),
                "name" => s.name = NameOf::ALL.iter().find(|n| n.1 == v).map_or(s.name, |n| n.0),
                "linger_s" => s.linger_s = v.parse().ok().filter(|v| LINGER_CHOICES_S.contains(v)).unwrap_or(s.linger_s),
                "refresh_ms" => {
                    s.refresh_ms = v.parse().ok().filter(|v| REFRESH_CHOICES_MS.contains(v)).unwrap_or(s.refresh_ms)
                }
                "host" => s.host = HostPick::new(v),
                _ => {}
            }
        }
        s
    }
}

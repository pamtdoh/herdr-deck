//! The menu's entries and their icons. Like the fonts, the pictures are spelled out here: `#` is the panel's
//! white, `o` the entry's own colour.

use crate::frame::{Frame, Rgb};
use crate::state::Command;
use crate::ui::palette::*;

pub const ICON: i32 = 9;
pub type Icon = [&'static str; ICON as usize];

/// One thing an entry can do. An entry with more than one is stepped through with left / right.
pub struct Choice {
    pub command: Command,
    pub label: &'static str,
    pub icon: &'static Icon,
    /// What the icon's `o` pixels are lit with.
    pub accent: Rgb,
}

pub struct Entry {
    pub choices: &'static [Choice],
    /// What cannot be taken back, or costs something, takes a second press.
    pub confirm: bool,
}

/// In carousel order; the menu always opens on the first, so a count of clicks always lands on the same entry.
pub const ENTRIES: [Entry; 4] = [
    // The conversation: both go in through the prompt, and both are refused at the same times.
    Entry {
        choices: &[
            Choice { command: Command::Compact, label: "COMPACT", icon: &COMPACT, accent: FABLE },
            Choice { command: Command::Clear, label: "CLEAR", icon: &CLEAR, accent: AMBER },
        ],
        confirm: true,
    },
    Entry { choices: &[Choice { command: Command::Rename, label: "RENAME TAB", icon: &RENAME, accent: AMBER }], confirm: false },
    Entry {
        choices: &[
            Choice { command: Command::SplitRight, label: "SPLIT RIGHT", icon: &SPLIT_RIGHT, accent: DONE },
            Choice { command: Command::SplitDown, label: "SPLIT DOWN", icon: &SPLIT_DOWN, accent: DONE },
        ],
        confirm: false,
    },
    Entry {
        choices: &[
            Choice { command: Command::CloseTab, label: "CLOSE TAB", icon: &CLOSE_TAB, accent: RED },
            Choice { command: Command::ClosePane, label: "CLOSE PANE", icon: &CLOSE_PANE, accent: RED },
        ],
        confirm: true,
    },
];

/// Two arrows pressing on a line.
const COMPACT: Icon = [
    "....#....",
    "..#####..",
    "...###...",
    "....#....",
    "ooooooooo",
    "....#....",
    "...###...",
    "..#####..",
    "....#....",
];

const CLEAR: Icon = [
    "...ooo...",
    ".ooooooo.",
    ".........",
    ".#######.",
    ".#.#.#.#.",
    ".#.#.#.#.",
    ".#.#.#.#.",
    ".#.#.#.#.",
    "..#####..",
];

/// A pencil, point down.
const RENAME: Icon = [
    ".......#.",
    "......###",
    ".....###.",
    "....###..",
    "...###...",
    "..###....",
    ".o##.....",
    "oo#......",
    "oo.......",
];

/// The pane to come is the filled half.
const SPLIT_RIGHT: Icon = [
    ".#######.",
    "#...#ooo#",
    "#...#ooo#",
    "#...#ooo#",
    "#...#ooo#",
    "#...#ooo#",
    "#...#ooo#",
    "#...#ooo#",
    ".#######.",
];

const SPLIT_DOWN: Icon = [
    ".#######.",
    "#.......#",
    "#.......#",
    "#.......#",
    "#########",
    "#ooooooo#",
    "#ooooooo#",
    "#ooooooo#",
    ".#######.",
];

const CLOSE_TAB: Icon = [
    "oo.....oo",
    "ooo...ooo",
    ".ooo.ooo.",
    "..ooooo..",
    "...ooo...",
    "..ooooo..",
    ".ooo.ooo.",
    "ooo...ooo",
    "oo.....oo",
];

/// The cross inside one pane's outline.
const CLOSE_PANE: Icon = [
    ".#######.",
    "#.......#",
    "#.o...o.#",
    "#..o.o..#",
    "#...o...#",
    "#..o.o..#",
    "#.o...o.#",
    "#.......#",
    ".#######.",
];

/// Top-left at (x, y); `ink` says what each of the two kinds of pixel is lit with.
pub fn draw_icon(f: &mut Frame, icon: &Icon, x: i32, y: i32, ink: impl Fn(bool) -> Rgb) {
    for (r, row) in icon.iter().enumerate() {
        for (k, b) in row.bytes().enumerate() {
            if b != b'.' {
                f.set(x + k as i32, y + r as i32, ink(b == b'o'));
            }
        }
    }
}

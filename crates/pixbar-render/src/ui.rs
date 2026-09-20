//! Input handling, overlays and drawing.
//!
//! Pure by construction: no clock, no I/O. The caller feeds inputs and `now_ms`, applies the returned
//! intents to herdr, and pushes world updates back. The same code runs in the simulator and on the device.

use crate::font::{BIG, SMALL};
use crate::frame::{Frame, Rect, Rgb, H, W};
use crate::settings::{
    step, Blocks, NameOf, Row, Settings, Show, Style, BRIGHTNESS_MIN, BRIGHTNESS_STEP, LINGER_CHOICES_S, REFRESH_CHOICES_MS,
};
use crate::state::{dollars_short, tokens_short, Agent, Effort, Model, Power, Status, World};

pub mod palette {
    use crate::frame::Rgb;
    pub const WHITE: Rgb = Rgb(229, 229, 217);
    pub const DIM: Rgb = Rgb(63, 63, 63);
    pub const OPUS: Rgb = Rgb(255, 106, 61);
    pub const FABLE: Rgb = Rgb(0, 200, 180);
    pub const SONNET: Rgb = Rgb(90, 140, 255);
    pub const HAIKU: Rgb = Rgb(120, 220, 90);
    /// A model whose name says nothing to us.
    pub const MODEL: Rgb = Rgb(200, 170, 110);
    pub const WORKING: Rgb = Rgb(255, 150, 0);
    pub const BLOCKED: Rgb = Rgb(255, 40, 40);
    pub const DONE: Rgb = Rgb(0, 230, 90);
    pub const IDLE: Rgb = Rgb(0, 100, 255);
    pub const UNKNOWN: Rgb = Rgb(150, 150, 150);
    pub const AMBER: Rgb = Rgb(255, 160, 0);
    pub const RED: Rgb = Rgb(255, 40, 40);
    /// Claude Code's ultracode colours: the violet tag and its shimmer highlight,
    /// and the eight background bands of the slider's "violet-ripple".
    pub const ULTRA: Rgb = Rgb(175, 135, 255);
    pub const ULTRA_SHIMMER: Rgb = Rgb(208, 180, 255);
    pub const RIPPLE: [Rgb; 8] = [
        Rgb(62, 22, 118),
        Rgb(73, 30, 135),
        Rgb(84, 39, 153),
        Rgb(95, 47, 170),
        Rgb(107, 55, 188),
        Rgb(118, 63, 205),
        Rgb(129, 72, 223),
        Rgb(140, 80, 240),
    ];
}
use palette::*;

/// The agent strip: the left 9 columns.
const STRIP_W: i32 = 9;
/// Everything right of the agent strip.
const MAIN: Rect = Rect { x0: 11, y0: 0, x1: 50, y1: 15 };
const MAIN_W: i32 = 40;
const LINE1_Y: i32 = 2;
const LINE2_Y: i32 = 9;
/// A card is 7 rows tall (the text plus 1 px above and below), so a carded row sits 1 px further out than a
/// plain one and two cards fill the height: 0..=6 and 9..=15. Its left edge is the main area's, where plain
/// text starts too. `FABLE XHIGH` is as wide as the whole main area, so beside a model chip the effort runs
/// into the panel's last column.
const CARD_CLIP: Rect = Rect { x0: MAIN.x0, y0: 0, x1: MAIN.x1 + 1, y1: 15 };
/// Padding left and right of a card's text. The model chip gets 1: that is all `FABLE XHIGH` leaves.
const CARD_PAD: i32 = 2;
/// The dim style's text.
const DIM_STYLE_LEVEL: f32 = 0.4;
/// Touching blocks: every other one this much darker, or neighbours with the same status read as one bar.
const TIGHT_SHADE: f32 = 0.7;

/// After a knob push jumped to an agent: at least this long, and long enough to read a scrolling name once.
/// After a turn the name stays for `Settings::linger_s`, or until the knob is pushed.
const JUMP_LINGER_MS: u64 = 2000;
/// A focus change this soon after a knob click is taken to be the knob's own, still on its way back from the host.
const FOCUS_GRACE_MS: u64 = 1000;
/// The host never followed the knob: stop showing an agent that is not the focused one.
const FOCUS_GIVE_UP_MS: u64 = 2500;
/// Button changes are sent once the presses stop for this long; pressing back within it sends nothing.
const COMMIT_SETTLE_MS: u64 = 700;
/// A lone knob click focuses at once; during a spin only the agent you stop on is focused.
const KNOB_SETTLE_MS: u64 = 150;
const OVERLAY_LINGER_MS: u64 = 1200;
/// A model switch makes the session re-read its whole context uncached, so the button only arms the switch and
/// a second press within this window sends it. Only a session with nothing to lose yet (`Agent::fresh`) switches
/// on one press.
const CONFIRM_WINDOW_MS: u64 = 4000;
/// Long enough for the ultracode ripple to flood the panel once.
const ULTRA_LINGER_MS: u64 = 2600;
const NAME_FLASH_MS: u64 = 2000;
const OPTIMISTIC_TIMEOUT_MS: u64 = 5000;
const KNOB_GLIDE_MS: u64 = 140;
const FLIP_MS: u64 = 160;

/// Rail geometry: round knob 7 px across. Five stops sit 8 px apart. With ultracode the five close up to
/// 6 px and ultracode stands apart past a divider, the way Claude Code's own slider detaches it past `┆`.
const RAIL_X0: i32 = MAIN.x0 + 1;
const RAIL_Y: i32 = 12;
const ULTRA_DIVIDER_X: i32 = 43;

/// Tells one agent from another across list changes: where it lives, not what it is doing (FNV-1a).
fn who(a: &Agent) -> u64 {
    [a.space.as_bytes(), &[0], a.tab.as_bytes(), &[0], a.dir.as_bytes()]
        .concat()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
}

fn stop_x(e: Effort, ultra_ok: bool) -> i32 {
    match (ultra_ok, e) {
        (true, Effort::Ultra) => 47,
        (true, _) => RAIL_X0 + 3 + e.index() as i32 * 6,
        (false, _) => RAIL_X0 + 3 + e.index().min(4) as i32 * 8,
    }
}

/// Ripple tuned to the 40 px field so it keeps Claude Code's rhythm (a ring passes a point every ~650 ms)
/// rather than its cell counts, which were sized for a 62-column dialog.
const RIPPLE_SPEED_PX_PER_MS: f32 = 0.020;
const RIPPLE_WAVELENGTH_PX: f32 = 13.0;
/// Claude Code's keyword shimmer: a 3-wide bright window advancing one step per 50 ms, starting 10 steps
/// before the word and resting for the remainder of a (width + 20)-step cycle.
const SHIMMER_STEP_MS: u64 = 50;
const MARQUEE_PAUSE_MS: u64 = 1000;
const MARQUEE_STEP_MS: u64 = 45;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Input {
    KnobCw,
    KnobCcw,
    KnobPush,
    /// Knob held down for a moment: opens the settings.
    KnobLong,
    Left,
    Right,
    Middle,
    /// Left or right kept down: repeated the way a keyboard repeats a key. It steps like a press, but it
    /// confirms nothing and does not knock on an end stop.
    LeftHeld,
    RightHeld,
}

/// What the host should do to herdr / Claude Code.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intent {
    Focus(usize),
    SetEffort { agent: usize, effort: Effort },
    SetModel { agent: usize, model: Model },
}

/// Facts about the device for the settings' info page; the renderer cannot find them out itself.
#[derive(Clone, Default, Debug)]
pub struct Info {
    /// The panel's own `ip:port` on WiFi, for hosts to connect to; empty without a network.
    pub addr: String,
    /// The network the panel is set up to join; empty when none is.
    pub ssid: String,
    /// Connected hosts, each with how it got here: `DESKTOP USB`, `LAPTOP WIFI`.
    pub hosts: Vec<String>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Item {
    Brightness,
    Blocks,
    /// What the top (0) or bottom (1) resting row shows, and how it is drawn.
    Row(usize),
    Style(usize),
    Name,
    Linger,
    Refresh,
    /// Read-only: which hosts are connected and how.
    Hosts,
    /// Read-only facts about the panel itself; left / right step through them.
    Device,
    /// Right arms it, right again does it.
    Action(DeviceAction),
}

/// What the device page steps through.
const DEVICE_FACTS: [&str; 4] = ["BATT", "WIFI", "IP", "VER"];

/// Battery, as Ulanzi's firmware treats it: warn at 3600 mV, and at 3550 mV count down and power off, so the
/// cell is not run down to its protection cut-off. Never while USB power is present.
const BATTERY_LOW_MV: u16 = 3600;
const BATTERY_OFF_MV: u16 = 3550;
/// The voltage sags under load; it has to stay down this long before the countdown starts.
const BATTERY_OFF_CONFIRM_MS: u64 = 10_000;
const BATTERY_OFF_COUNTDOWN_MS: u64 = 30_000;
/// A battery notice comes up on its own when the cable goes in or out and when the charge falls through one
/// of these; while the battery is low it comes back every minute.
const BATTERY_NOTICE_AT: [u8; 3] = [20, 10, 5];
const BATTERY_NOTICE_MS: u64 = 3500;
const BATTERY_NAG_MS: u64 = 60_000;

/// Something only the device itself can carry out.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceAction {
    PowerOff,
    /// Hand the panel back to Ulanzi's firmware.
    Stock,
}

const ITEMS: [Item; 13] = [
    Item::Brightness,
    Item::Blocks,
    Item::Row(0),
    Item::Style(0),
    Item::Row(1),
    Item::Style(1),
    Item::Name,
    Item::Linger,
    Item::Refresh,
    Item::Hosts,
    Item::Device,
    Item::Action(DeviceAction::PowerOff),
    Item::Action(DeviceAction::Stock),
];
const SETTINGS_LINGER_MS: u64 = 10_000;
/// The read-only pages are for reading, and may have a long line to scroll.
const INFO_LINGER_MS: u64 = 30_000;

#[derive(Clone, Copy, Debug)]
enum Overlay {
    None,
    /// `jumped`: opened by a knob push (next agent that needs you) rather than by turning.
    Picker { hover: usize, last_input: u64, dir: i32, sent: Option<usize>, jumped: bool },
    Effort {
        agent: usize,
        from: Effort,
        target: Effort,
        moved_at: u64,
        last_input: u64,
        dir: i32,
        bump_at: Option<u64>,
        committed_at: Option<u64>,
    },
    /// `committed_at: None` is the armed state: shown, not sent, waiting for the second press.
    Model { agent: usize, target: Model, changed_at: u64, committed_at: Option<u64> },
    /// `paged`: the last input turned the page (knob) rather than changed the value (buttons).
    Settings { item: usize, last_input: u64, dir: i32, paged: bool, bump_at: Option<u64> },
}

/// A change we have sent but the host has not confirmed yet; shown as if it had already happened.
#[derive(Clone, Copy, Debug)]
struct Optimistic {
    agent: usize,
    effort: Option<Effort>,
    model: Option<Model>,
    since: u64,
}

pub struct Ui {
    pub settings: Settings,
    pub info: Info,
    device_fact: usize,
    /// When an action page was armed by its first press.
    armed_at: Option<u64>,
    device_action: Option<DeviceAction>,
    power: Power,
    /// When the battery notice came up.
    power_notice: Option<u64>,
    /// Since when the cell has been under the power-off voltage, and since when the countdown runs.
    critical_since: Option<u64>,
    countdown_since: Option<u64>,
    settings_changed: bool,
    overlay: Overlay,
    /// Which agent an effort or model overlay was opened on. The overlay holds a list position, and the list
    /// can shift under it: a change meant for one session must not reach the one that slid into its place.
    overlay_who: u64,
    optimistic: Option<Optimistic>,
    last_focused: usize,
    /// (since, until): the label takes line 2 after focus moved without the knob.
    name_flash: Option<(u64, u64)>,
    reject_at: Option<u64>,
    confirm_at: Option<u64>,
}

impl Default for Ui {
    fn default() -> Self {
        Self::new()
    }
}

impl Ui {
    pub fn new() -> Self {
        Ui {
            settings: Settings::default(),
            info: Info::default(),
            device_fact: 0,
            armed_at: None,
            device_action: None,
            power: Power::default(),
            power_notice: None,
            critical_since: None,
            countdown_since: None,
            settings_changed: false,
            overlay: Overlay::None,
            overlay_who: 0,
            optimistic: None,
            last_focused: 0,
            name_flash: None,
            reject_at: None,
            confirm_at: None,
        }
    }

    fn effort_of(&self, world: &World, idx: usize) -> Effort {
        match self.optimistic {
            Some(Optimistic { agent, effort: Some(e), .. }) if agent == idx => e,
            _ => world.agents[idx].effort,
        }
    }

    fn model_of(&self, world: &World, idx: usize) -> Model {
        match self.optimistic {
            Some(Optimistic { agent, model: Some(m), .. }) if agent == idx => m,
            _ => world.agents[idx].model,
        }
    }

    fn set_optimistic(&mut self, agent: usize, effort: Option<Effort>, model: Option<Model>, now: u64) {
        let prev = self.optimistic.filter(|o| o.agent == agent);
        self.optimistic = Some(Optimistic {
            agent,
            effort: effort.or(prev.and_then(|o| o.effort)),
            model: model.or(prev.and_then(|o| o.model)),
            since: now,
        });
    }

    /// Sends a pending effort change now instead of waiting out the settle time.
    /// An armed model switch is not pending in that sense: without its second press it is dropped.
    fn flush(&mut self, world: &World, now: u64) -> Option<Intent> {
        if let Overlay::Effort { agent, from, target, moved_at, last_input, dir, bump_at, committed_at: None } = self.overlay {
            self.overlay =
                Overlay::Effort { agent, from, target, moved_at, last_input, dir, bump_at, committed_at: Some(now) };
            if agent < world.agents.len() && target != world.agents[agent].effort {
                self.set_optimistic(agent, Some(target), None, now);
                return Some(Intent::SetEffort { agent, effort: target });
            }
        }
        None
    }

    /// Settings that were changed on the panel, handed over once when the settings screen closes,
    /// so the device writes its config file once rather than on every knob click.
    pub fn take_settings_change(&mut self) -> Option<Settings> {
        let closed = !matches!(self.overlay, Overlay::Settings { .. });
        (closed && std::mem::take(&mut self.settings_changed)).then_some(self.settings)
    }

    /// While the settings screen is up the knob turns the pages and left / right change the value, the way
    /// they move the effort rail; nothing reaches herdr.
    fn settings_input(&mut self, item: usize, input: Input, held: bool, now: u64) {
        let (dir, paged) = match input {
            Input::KnobPush | Input::KnobLong | Input::Middle => {
                self.overlay = Overlay::None;
                return;
            }
            Input::KnobCw => (1, true),
            Input::KnobCcw => (-1, true),
            Input::Right | Input::RightHeld => (1, false),
            Input::Left | Input::LeftHeld => (-1, false),
        };
        if paged {
            (self.device_fact, self.armed_at) = (0, None);
            let item = (item as i32 + dir).rem_euclid(ITEMS.len() as i32) as usize;
            self.overlay = Overlay::Settings { item, last_input: now, dir, paged, bump_at: None };
            return;
        }
        let (before, s) = (self.settings, &mut self.settings);
        // Scales stop at their ends; lists with no order to them go round.
        match ITEMS[item] {
            Item::Brightness => {
                let b = s.brightness as i32 + dir * BRIGHTNESS_STEP as i32;
                s.brightness = b.clamp(BRIGHTNESS_MIN as i32, 100) as u8
            }
            Item::Blocks => s.blocks = step(&Blocks::CHOICES, s.blocks, dir, false),
            Item::Row(i) => s.rows[i].show = step(&Show::CHOICES, s.rows[i].show, dir, true),
            Item::Style(i) => s.rows[i].style = step(&Style::ALL.map(|s| s.0), s.rows[i].style, dir, true),
            Item::Name => s.name = step(&NameOf::ALL.map(|n| n.0), s.name, dir, true),
            Item::Linger => s.linger_s = step(&LINGER_CHOICES_S, s.linger_s, dir, false),
            Item::Refresh => s.refresh_ms = step(&REFRESH_CHOICES_MS, s.refresh_ms, dir, false),
            Item::Hosts => {}
            // Turning the device off takes two deliberate presses; a finger resting on the button is one.
            Item::Action(_) if held => return,
            Item::Action(action) => {
                let armed = self.armed_at.is_some_and(|t| now.saturating_sub(t) < CONFIRM_WINDOW_MS);
                if dir > 0 && armed {
                    (self.device_action, self.armed_at, self.overlay) = (Some(action), None, Overlay::None);
                } else {
                    self.armed_at = (dir > 0).then_some(now);
                    self.overlay = Overlay::Settings { item, last_input: now, dir, paged, bump_at: None };
                }
                return;
            }
            Item::Device => {
                self.device_fact = (self.device_fact as i32 + dir).rem_euclid(DEVICE_FACTS.len() as i32) as usize;
                self.overlay = Overlay::Settings { item, last_input: now, dir, paged, bump_at: None };
                return;
            }
        }
        let changed = self.settings != before;
        if held && !changed {
            return;
        }
        self.settings_changed |= changed;
        self.overlay = Overlay::Settings { item, last_input: now, dir, paged, bump_at: (!changed).then_some(now) };
    }

    pub fn input(&mut self, world: &World, input: Input, now: u64) -> Option<Intent> {
        self.drop_stale_overlay(world);
        let intent = self.input_checked(world, input, now);
        if let Overlay::Effort { agent, .. } | Overlay::Model { agent, .. } = self.overlay {
            self.overlay_who = world.agents.get(agent).map_or(0, who);
        }
        intent
    }

    /// An effort or model overlay whose agent left the list, or is no longer the one at that position, is
    /// closed without sending anything.
    fn drop_stale_overlay(&mut self, world: &World) {
        if let Overlay::Effort { agent, .. } | Overlay::Model { agent, .. } = self.overlay {
            if world.agents.get(agent).map(who) != Some(self.overlay_who) {
                self.overlay = Overlay::None;
                self.optimistic = self.optimistic.filter(|o| o.agent != agent);
            }
        }
    }

    fn input_checked(&mut self, world: &World, input: Input, now: u64) -> Option<Intent> {
        let (input, held) = match input {
            Input::LeftHeld => (Input::Left, true),
            Input::RightHeld => (Input::Right, true),
            pressed => (pressed, false),
        };
        if let Overlay::Settings { item, .. } = self.overlay {
            self.settings_input(item, input, held, now);
            return None;
        }
        if input == Input::KnobLong {
            let flushed = if world.agents.is_empty() { None } else { self.flush(world, now) };
            self.overlay = Overlay::Settings { item: 0, last_input: now, dir: 0, paged: true, bump_at: None };
            return flushed;
        }
        if world.agents.is_empty() {
            return None;
        }
        let focused = world.focused.min(world.agents.len() - 1);
        match input {
            Input::KnobLong | Input::LeftHeld | Input::RightHeld => None,
            Input::KnobCw | Input::KnobCcw => {
                let dir = if input == Input::KnobCw { 1 } else { -1 };
                let flushed = self.flush(world, now);
                let (from, spinning, sent) = match self.overlay {
                    Overlay::Picker { hover, last_input, sent, .. } if hover < world.agents.len() => {
                        (hover, now.saturating_sub(last_input) < KNOB_SETTLE_MS, sent)
                    }
                    _ => (focused, false, None),
                };
                let hover = (from as i32 + dir).rem_euclid(world.agents.len() as i32) as usize;
                // Mid-spin (or when the flush already used this call's intent) `tick` focuses once the knob rests.
                let focus_now = !spinning && flushed.is_none();
                let sent = if focus_now { Some(hover) } else { sent };
                self.overlay = Overlay::Picker { hover, last_input: now, dir, sent, jumped: false };
                flushed.or(focus_now.then_some(Intent::Focus(hover)))
            }
            // Push while the name lingers after a turn: "got it", back to the resting screen.
            // Otherwise: jump to the next agent that needs you (blocked first, then done).
            Input::KnobPush => {
                if let Overlay::Picker { hover, sent, jumped: false, .. } = self.overlay {
                    self.overlay = Overlay::None;
                    return (sent != Some(hover) && hover < world.agents.len()).then_some(Intent::Focus(hover));
                }
                let n = world.agents.len();
                let after = |want: Status| (1..=n).map(|k| (focused + k) % n).find(|&i| world.agents[i].status == want);
                match after(Status::Blocked).or_else(|| after(Status::Done)) {
                    Some(hover) => {
                        let flushed = self.flush(world, now);
                        self.confirm_at = Some(now);
                        let sent = flushed.is_none().then_some(hover);
                        self.overlay = Overlay::Picker { hover, last_input: now, dir: 1, sent, jumped: true };
                        flushed.or(Some(Intent::Focus(hover)))
                    }
                    None => {
                        self.reject_at = Some(now);
                        None
                    }
                }
            }
            Input::Left | Input::Right => {
                // Injecting keys into a pane that shows a permission prompt could answer it.
                // Nor is there anything to move where the session's effort is not known, or its model has none.
                let a = &world.agents[focused];
                if a.status == Status::Blocked || !a.reported || !a.has_effort {
                    self.reject_at = (!held).then_some(now).or(self.reject_at);
                    return None;
                }
                let dir = if input == Input::Right { 1 } else { -1 };
                let (current, moved_at, committed_at) = match self.overlay {
                    Overlay::Effort { agent, target, moved_at, committed_at, .. } if agent == focused => {
                        (target, moved_at, committed_at)
                    }
                    _ => (self.effort_of(world, focused), 0, Some(now)),
                };
                let next = current.step(dir, world.agents[focused].ultra_ok);
                // Held against the end of the rail: neither a knock nor a reason to put off sending.
                if held && next.is_none() {
                    return None;
                }
                self.overlay = match next {
                    Some(next) => Overlay::Effort {
                        agent: focused,
                        from: current,
                        target: next,
                        moved_at: now,
                        last_input: now,
                        dir,
                        bump_at: None,
                        committed_at: None,
                    },
                    None => Overlay::Effort {
                        agent: focused,
                        from: current,
                        target: current,
                        moved_at,
                        last_input: now,
                        dir,
                        bump_at: Some(now),
                        committed_at,
                    },
                };
                None
            }
            Input::Middle => {
                // Which model the button leads to is the host's to say: it knows the account's models, the panel
                // does not. While a switch of ours is still unconfirmed, that is the way back.
                let a = &world.agents[focused];
                let shown = self.model_of(world, focused);
                let target = a.next_model.map(|next| if next == shown { a.model } else { next }).filter(|&t| t != shown);
                let Some(target) = target.filter(|_| a.status != Status::Blocked && a.reported) else {
                    self.reject_at = Some(now);
                    return None;
                };
                // The first press arms the switch and shows what it costs, the second sends it; left alone it
                // lapses and nothing is sent. A fresh session has nothing to lose: one press.
                if let Overlay::Model { agent, target, changed_at, committed_at: None } = self.overlay {
                    if agent == focused {
                        self.overlay = Overlay::Model { agent, target, changed_at, committed_at: Some(now) };
                        self.set_optimistic(agent, None, Some(target), now);
                        return Some(Intent::SetModel { agent, model: target });
                    }
                }
                let flushed = self.flush(world, now);
                // (One input returns one intent; if this press just flushed an effort change, it only arms.)
                if world.agents[focused].fresh && flushed.is_none() {
                    self.overlay = Overlay::Model { agent: focused, target, changed_at: now, committed_at: Some(now) };
                    self.set_optimistic(focused, None, Some(target), now);
                    return Some(Intent::SetModel { agent: focused, model: target });
                }
                self.overlay = Overlay::Model { agent: focused, target, changed_at: now, committed_at: None };
                flushed
            }
        }
    }

    fn tick_power(&mut self, now: u64) {
        if self.armed_at.is_some_and(|t| now.saturating_sub(t) >= CONFIRM_WINDOW_MS) {
            self.armed_at = None;
        }
        let critical = self.on_cell() && self.power.millivolts.is_some_and(|mv| mv <= BATTERY_OFF_MV);
        if !critical {
            (self.critical_since, self.countdown_since) = (None, None);
        } else if now.saturating_sub(*self.critical_since.get_or_insert(now)) >= BATTERY_OFF_CONFIRM_MS {
            let started = *self.countdown_since.get_or_insert(now);
            if now.saturating_sub(started) >= BATTERY_OFF_COUNTDOWN_MS {
                (self.device_action, self.countdown_since, self.critical_since) = (Some(DeviceAction::PowerOff), None, None);
            }
        }
        // While low, the notice keeps coming back, but never on top of something you are doing.
        let due = self.power_notice.is_none_or(|t| now.saturating_sub(t) >= BATTERY_NAG_MS);
        if self.battery_low() && due && matches!(self.overlay, Overlay::None) {
            self.power_notice = Some(now);
        }
    }

    /// Call every frame before `render`. Runs timers; may emit the settled effort change.
    pub fn tick(&mut self, world: &World, now: u64) -> Option<Intent> {
        self.tick_power(now);
        self.drop_stale_overlay(world);
        if world.focused != self.last_focused {
            self.last_focused = world.focused;
            // The knob's own focus change needs no announcement: the picker already shows the label.
            let ours = matches!(self.overlay, Overlay::Picker { hover, last_input, .. }
                if hover == world.focused || now.saturating_sub(last_input) < FOCUS_GRACE_MS);
            if !ours {
                // Focus moved from the keyboard: follow it, even out of a lingering picker.
                if matches!(self.overlay, Overlay::Picker { .. }) {
                    self.overlay = Overlay::None;
                }
                let pass = world.agents.get(world.focused).map_or(0, |a| marquee_pass_ms(&a.label(self.settings.name), MAIN_W));
                self.name_flash = Some((now, now + NAME_FLASH_MS.max(pass)));
            }
        }
        if let Some(o) = self.optimistic {
            let confirmed = world.agents.get(o.agent).is_some_and(|a| {
                o.effort.is_none_or(|e| e == a.effort) && o.model.is_none_or(|m| m == a.model)
            });
            if confirmed || now.saturating_sub(o.since) >= OPTIMISTIC_TIMEOUT_MS {
                self.optimistic = None;
            }
        }
        match self.overlay {
            Overlay::None => None,
            Overlay::Settings { item, last_input, .. } => {
                let reading = matches!(ITEMS[item], Item::Hosts | Item::Device);
                let linger = if reading { INFO_LINGER_MS } else { SETTINGS_LINGER_MS };
                if now.saturating_sub(last_input) >= linger {
                    self.overlay = Overlay::None;
                }
                None
            }
            Overlay::Picker { hover, last_input, dir, sent, jumped } => {
                let idle = now.saturating_sub(last_input);
                let linger = match world.agents.get(hover) {
                    Some(a) if jumped => JUMP_LINGER_MS.max(marquee_pass_ms(&a.label(self.settings.name), MAIN_W)),
                    Some(_) => self.settings.linger_s as u64 * 1000,
                    None => 0,
                };
                let not_followed = sent == Some(hover) && world.focused != hover && idle >= FOCUS_GIVE_UP_MS;
                if idle >= linger || not_followed {
                    self.overlay = Overlay::None;
                    None
                } else if sent != Some(hover) && idle >= KNOB_SETTLE_MS {
                    self.overlay = Overlay::Picker { hover, last_input, dir, sent: Some(hover), jumped };
                    Some(Intent::Focus(hover))
                } else {
                    None
                }
            }
            Overlay::Effort { agent, target, last_input, committed_at, .. } => {
                let idle = now.saturating_sub(last_input);
                let linger = if target == Effort::Ultra { ULTRA_LINGER_MS } else { OVERLAY_LINGER_MS };
                if agent >= world.agents.len() || idle >= linger {
                    let flushed = self.flush(world, now);
                    self.overlay = Overlay::None;
                    flushed
                } else if committed_at.is_none() && idle >= COMMIT_SETTLE_MS {
                    self.flush(world, now)
                } else {
                    None
                }
            }
            Overlay::Model { agent, changed_at, committed_at, .. } => {
                let expired = match committed_at {
                    Some(t) => now.saturating_sub(t) >= OVERLAY_LINGER_MS,
                    None => now.saturating_sub(changed_at) >= CONFIRM_WINDOW_MS,
                };
                if agent >= world.agents.len() || expired {
                    self.overlay = Overlay::None;
                }
                None
            }
        }
    }

    /// The settings screen and the battery notices need no host and no agents; the device shows them even
    /// over its "no host" notice.
    pub fn wants_screen(&self, now: u64) -> bool {
        matches!(self.overlay, Overlay::Settings { .. }) || self.countdown_since.is_some() || self.notice_up(now)
    }

    /// Power off or back to stock, asked for on the settings screen or by an empty battery. Handed over once.
    pub fn take_device_action(&mut self) -> Option<DeviceAction> {
        self.device_action.take()
    }

    fn notice_up(&self, now: u64) -> bool {
        self.power_notice.is_some_and(|t| now.saturating_sub(t) < BATTERY_NOTICE_MS)
    }

    fn on_cell(&self) -> bool {
        self.power.on_usb == Some(false)
    }

    fn battery_low(&self) -> bool {
        self.on_cell() && (self.power.millivolts.is_some_and(|mv| mv <= BATTERY_LOW_MV) || self.power.percent.is_some_and(|p| p <= 5))
    }

    /// A new reading from the MCU.
    pub fn set_power(&mut self, power: Power, now: u64) {
        let was = std::mem::replace(&mut self.power, power);
        let cable_moved = was.on_usb.is_some() && power.on_usb.is_some() && was.on_usb != power.on_usb;
        let fell_through = self.on_cell()
            && BATTERY_NOTICE_AT.iter().any(|&t| was.percent.is_some_and(|w| w > t) && power.percent.is_some_and(|p| p <= t));
        if cable_moved || fell_through {
            self.power_notice = Some(now);
        }
    }

    pub fn render(&self, world: &World, now: u64, f: &mut Frame) {
        f.clear();
        f.reset_clip();
        let caret = match self.overlay {
            Overlay::Picker { hover, .. } if hover < world.agents.len() => hover,
            _ => world.focused,
        };
        self.draw_strip(world, caret, now, f);

        f.set_clip(MAIN);
        if let Some(since) = self.countdown_since {
            draw_power_off_countdown(f, BATTERY_OFF_COUNTDOWN_MS.saturating_sub(now.saturating_sub(since)), now);
        } else if let (Overlay::None, Some(since), true) = (self.overlay, self.power_notice, self.notice_up(now)) {
            draw_battery(f, self.power, self.battery_low(), now.saturating_sub(since), now);
        } else if let Overlay::Settings { item, last_input, dir, paged, bump_at } = self.overlay {
            self.draw_settings(f, world, ITEMS[item], last_input, dir, paged, bump_at, now);
        } else if world.agents.is_empty() {
            SMALL.draw(f, "NO AGENTS", MAIN.x0, 5, DIM);
        } else {
            let focused = world.focused.min(world.agents.len() - 1);
            match self.overlay {
                Overlay::None | Overlay::Settings { .. } => self.draw_rest(world, focused, now, f),
                Overlay::Picker { hover, last_input, dir, .. } => {
                    self.draw_picker(world, hover.min(world.agents.len() - 1), last_input, dir, now, f)
                }
                // `tick` closes an overlay whose agent is gone; a caller that renders first gets the rest screen.
                Overlay::Effort { agent, .. } | Overlay::Model { agent, .. } if agent >= world.agents.len() => {
                    self.draw_rest(world, focused, now, f)
                }
                Overlay::Effort { target, from, moved_at, dir, bump_at, committed_at, agent, .. } => {
                    let hue = model_hue(self.model_of(world, agent));
                    let ultra_ok = world.agents[agent].ultra_ok || target == Effort::Ultra;
                    draw_effort(f, hue, ultra_ok, from, target, moved_at, dir, bump_at, committed_at, now)
                }
                Overlay::Model { agent, target, changed_at, committed_at } => {
                    draw_model(f, target, world.agents[agent].ctx_used, changed_at, committed_at, now)
                }
            }
        }
        f.reset_clip();
    }

    /// Left 9 px: one block per agent in sidebar order, filled column by column. Focus is shown by brightness
    /// alone, so no pixels go to a cursor. More agents than blocks: the strip shows the page the focus is on.
    fn draw_strip(&self, world: &World, caret: usize, now: u64, f: &mut Frame) {
        let Blocks { size, gap } = self.settings.blocks;
        let (size, gap) = (size as i32, gap as i32);
        let (pitch, cols, rows) = self.settings.blocks.grid();
        let (ox, oy) = ((STRIP_W - cols * pitch + gap) / 2, (H as i32 - rows * pitch + gap) / 2);
        let cap = self.settings.blocks.capacity();
        let first = caret / cap * cap;
        for (slot, (i, a)) in world.agents.iter().enumerate().skip(first).take(cap).enumerate() {
            let (col, row) = (slot as i32 / rows, slot as i32 % rows);
            let (x0, y0) = (ox + col * pitch, oy + row * pitch);
            let hue = match a.status {
                Status::Idle => IDLE,
                Status::Working => WORKING,
                Status::Blocked => BLOCKED,
                Status::Done => DONE,
                Status::Unknown => UNKNOWN,
            };
            let shade = if gap == 0 && i != caret && (col + row) % 2 == 1 { TIGHT_SHADE } else { 1.0 };
            let mut c = hue.scale(block_level(a.status, i == caret, now) * shade);
            if i == caret && self.confirm_at.is_some_and(|t| now.saturating_sub(t) < 50) {
                c = WHITE;
            }
            f.fill_rect(Rect { x0, y0, x1: x0 + size - 1, y1: y0 + size - 1 }, c);
        }
    }

    fn draw_rest(&self, world: &World, idx: usize, now: u64, f: &mut Frame) {
        // Refused input (agent is blocked): 1 px shake instead of an overlay.
        let dx = self.reject_at.map_or(0, |t| bump_offset(now.saturating_sub(t), 1));
        let rows = self.settings.rows;
        self.draw_row(f, world, idx, rows[0], 0, dx, 0, now);
        // Focus moved without the knob: if no row carries the name, the bottom one gives way to it for a moment.
        let named = rows.iter().any(|r| r.show == Show::Name);
        match self.name_flash.filter(|&(_, until)| !named && now < until) {
            Some((since, _)) => {
                let label = world.agents[idx].label(self.settings.name);
                draw_marquee(f, &label, MAIN.x0 + dx, LINE2_Y, MAIN_W, WHITE, now.saturating_sub(since));
            }
            None => self.draw_row(f, world, idx, rows[1], 1, dx, 0, now),
        }
    }

    /// One resting row; `slot` 0 is the top one.
    #[allow(clippy::too_many_arguments)]
    fn draw_row(&self, f: &mut Frame, world: &World, idx: usize, row: Row, slot: usize, dx: i32, dy: i32, now: u64) {
        let a = &world.agents[idx];
        let x = MAIN.x0 + dx;
        let plain = !row.style.carded();
        let y = dy + match (slot, plain) {
            (0, true) => LINE1_Y,
            (0, false) => LINE1_Y - 1,
            (_, true) => LINE2_Y,
            (_, false) => LINE2_Y + 1,
        };
        // Context and usage windows alike: the colour says how little is left.
        let level = |pct: u32| match pct {
            90.. => RED,
            70.. => AMBER,
            _ => WHITE,
        };
        let pct = a.ctx_pct();
        let ctx_color = level(pct);
        // Text-only rows are the same code with no padding and no field.
        let pad = if plain { 0 } else { CARD_PAD };
        let inner = MAIN_W - 2 * pad;
        let field = |f: &mut Frame, w: i32, c: Rgb| match row.style.carded() {
            true => draw_card(f, x, y, w, pad, field_color(row.style, c)),
            false => x,
        };
        let ink = |c: Rgb| match row.style {
            Style::Card => Rgb::OFF,
            Style::Dim => c.scale(DIM_STYLE_LEVEL),
            _ => c,
        };
        match row.show {
            // Not known is not the same as a default: a session whose status line never reached the host.
            Show::Model | Show::Context if !a.reported => {
                let tx = field(f, SMALL.width("--"), DIM);
                SMALL.draw(f, "--", tx, y, ink(DIM));
            }
            Show::Model => {
                let effort = a.has_effort.then(|| self.effort_of(world, idx));
                draw_model_effort(f, self.model_of(world, idx), effort, x, y, row.style, now)
            }
            Show::Context => {
                let s = format!("{} {}%", tokens_short(a.ctx_used), pct);
                let tx = field(f, SMALL.width(&s), ctx_color);
                SMALL.draw(f, &s, tx, y, ink(ctx_color));
            }
            Show::Name => {
                let label = a.label(self.settings.name);
                let tx = field(f, SMALL.width(&label).min(inner), WHITE);
                f.set_clip(Rect { x0: tx, x1: tx + inner - 1, ..MAIN });
                draw_marquee(f, &label, tx, y, inner, ink(WHITE), now);
                f.set_clip(MAIN);
            }
            Show::Limit5h | Show::Limit7d => {
                let (window, limit) = if row.show == Show::Limit5h { ("5H", a.limit_5h) } else { ("7D", a.limit_7d) };
                // How much of the window is used, then, stepping back, how long until it starts over.
                let (used, rest) = match limit {
                    Some(l) => (format!("{window} {}%", l.used_pct), format!(" {}", l.resets_in())),
                    None => (format!("{window} --"), String::new()),
                };
                let color = level(limit.map_or(0, |l| l.used_pct as u32));
                let tx = field(f, SMALL.width(&used) + SMALL.width(&rest), color);
                let rx = SMALL.draw(f, &used, tx, y, ink(color));
                SMALL.draw(f, &rest, rx, y, if row.style == Style::Card { Rgb::OFF } else { DIM });
            }
            Show::Cost => {
                let s = dollars_short(a.cost_cents);
                let tx = field(f, SMALL.width(&s), WHITE);
                SMALL.draw(f, &s, tx, y, ink(WHITE));
            }
        }
    }

    /// Settings screen: name of the setting and page dots on the left; the value is a big number, a word, or
    /// the very thing it changes, drawn live. Turning a page slides it in vertically, changing a value sideways.
    #[allow(clippy::too_many_arguments)]
    fn draw_settings(
        &self,
        f: &mut Frame,
        world: &World,
        item: Item,
        last_input: u64,
        dir: i32,
        paged: bool,
        bump_at: Option<u64>,
        now: u64,
    ) {
        let s = &self.settings;
        let since = now.saturating_sub(last_input);
        let bump = bump_at.map_or(0, |t| bump_offset(now.saturating_sub(t), dir));
        let dy = if paged { slide_offset(since, &[6, 4, 2, -1, 0]) * dir } else { 0 };
        let slide = if paged { 0 } else { slide_offset(since, &[3, 2, 1, 0]) * dir };
        let focused = (!world.agents.is_empty()).then(|| world.focused.min(world.agents.len() - 1));

        let label = match item {
            Item::Brightness => "BRIGHT",
            Item::Blocks => "BLOCKS",
            Item::Row(0) => "ROW 1",
            Item::Row(_) => "ROW 2",
            Item::Style(0) => "STYLE 1",
            Item::Style(_) => "STYLE 2",
            Item::Name => "NAME",
            Item::Linger => "LINGER",
            Item::Refresh => "REFRESH",
            Item::Hosts => "HOSTS",
            Item::Device => "DEVICE",
            Item::Action(DeviceAction::PowerOff) => "TURN OFF",
            Item::Action(DeviceAction::Stock) => "STOCK FW",
        };
        SMALL.draw(f, label, MAIN.x0, 1 + dy, DIM);
        let big_right = |f: &mut Frame, text: &str| BIG.draw(f, text, MAIN.x1 + 1 - BIG.width(text) + bump, dy, WHITE);
        let word_right = |f: &mut Frame, text: &str| SMALL.draw(f, text, MAIN.x1 + 1 - SMALL.width(text) + bump, 1 + dy, WHITE);
        let line2 = |f: &mut Frame, text: &str| SMALL.draw(f, text, MAIN.x0 + slide + bump, LINE2_Y + dy, WHITE);
        match item {
            Item::Brightness => {
                big_right(f, &s.brightness.to_string());
                let (min, step) = (BRIGHTNESS_MIN as i32, BRIGHTNESS_STEP as i32);
                let kx = RAIL_X0 + 3 + (s.brightness as i32 - min) / step * 32 / ((100 - min) / step) - slide + bump;
                let ry = RAIL_Y + dy;
                for x in RAIL_X0..kx {
                    for y in ry - 1..=ry + 1 {
                        if x != RAIL_X0 || y == ry {
                            f.set(x, y, AMBER);
                        }
                    }
                }
                for x in (kx + 5..=MAIN.x1 - 1).step_by(4) {
                    f.set(x, ry, DIM);
                }
                f.disc(kx, ry, 3, WHITE);
                f.disc(kx, ry, 2, AMBER);
            }
            // The strip next to it is the preview; the number is how many agents fit.
            Item::Blocks => {
                big_right(f, &s.blocks.capacity().to_string());
                line2(f, &format!("{0}X{0} {1}", s.blocks.size, if s.blocks.gap { "GAP" } else { "TIGHT" }));
            }
            // The row itself is the preview, drawn with the focused agent's data; its name goes top right
            // where there is room for it.
            Item::Row(i) | Item::Style(i) => {
                if let Item::Row(_) = item {
                    word_right(f, s.rows[i].show.word());
                }
                match focused {
                    Some(idx) => self.draw_row(f, world, idx, s.rows[i], 1, slide + bump, dy, now),
                    None => drop(line2(f, s.rows[i].style.word())),
                }
            }
            Item::Name => {
                word_right(f, s.name.word());
                if let Some(idx) = focused {
                    let label = world.agents[idx].label(s.name);
                    draw_marquee(f, &label, MAIN.x0 + slide + bump, LINE2_Y + dy, MAIN_W, WHITE, since);
                }
            }
            Item::Linger => drop(line2(f, &format!("{} SEC", s.linger_s))),
            Item::Refresh => drop(line2(f, &format!("{} SEC", s.refresh_ms as f32 / 1000.0))),
            // Like the model button: the first press asks, the second within the window does it.
            Item::Action(_) => match self.armed_at {
                Some(t) => {
                    if now.saturating_sub(t) % 600 < 400 {
                        line2(f, "ONCE MORE");
                    }
                    let left = 1.0 - (now.saturating_sub(t) as f32 / CONFIRM_WINDOW_MS as f32).min(1.0);
                    let w = (MAIN_W as f32 * left).round() as i32;
                    if w > 0 {
                        f.fill_rect(Rect { x0: MAIN.x0, y0: 15, x1: MAIN.x0 + w - 1, y1: 15 }, RED);
                    }
                }
                None => drop(line2(f, "PUSH RIGHT")),
            },
            // The link: who is connected, and whether by cable or network.
            Item::Hosts => {
                big_right(f, &self.info.hosts.len().to_string());
                let line = if self.info.hosts.is_empty() { "NONE".to_string() } else { self.info.hosts.join(" · ") };
                draw_marquee(f, &line, MAIN.x0, LINE2_Y + dy, MAIN_W, WHITE, since);
            }
            // The panel itself. Its address is what `pixbar-bridge --device` takes, whoever is connected right now.
            Item::Device => {
                word_right(f, DEVICE_FACTS[self.device_fact]);
                let or = |s: &str, none: &str| if s.is_empty() { none.to_string() } else { s.to_string() };
                let line = match self.device_fact {
                    0 => match (self.power.percent, self.power.on_usb, self.power.millivolts) {
                        (Some(p), Some(true), _) => format!("{p}% ON USB"),
                        (Some(p), _, Some(mv)) => format!("{p}% {}.{:02}V", mv / 1000, mv % 1000 / 10),
                        _ => "NO READING".to_string(),
                    },
                    1 => or(&self.info.ssid, "NOT SET UP"),
                    2 => or(&self.info.addr, "NO NETWORK"),
                    _ => env!("CARGO_PKG_VERSION").to_string(),
                };
                draw_marquee(f, &line, MAIN.x0 + slide, LINE2_Y + dy, MAIN_W, WHITE, since);
            }
        }
        // Page dots under the name: which setting you are on.
        for (i, it) in ITEMS.iter().enumerate() {
            f.set(MAIN.x0 + 3 * i as i32, 7, if *it == item { WHITE } else { DIM });
        }
    }

    fn draw_picker(&self, world: &World, hover: usize, last_input: u64, dir: i32, now: u64, f: &mut Frame) {
        let a = &world.agents[hover];
        // New rows slide in from the side the knob is heading, with a 1 px overshoot.
        let dy = slide_offset(now.saturating_sub(last_input), &[6, 4, 2, -1, 0]) * dir;
        draw_marquee(f, &a.label(self.settings.name), MAIN.x0, LINE1_Y + dy, MAIN_W, WHITE, now.saturating_sub(last_input));
        // Underneath, how full that agent is, greyed: the name is the point here.
        self.draw_row(f, world, hover, Row { show: Show::Context, style: Style::Dim }, 1, 0, dy, now);
    }
}

/// Battery notice: charge as a big number over a gauge in the rail's shape, green, amber under 20 %, red under 10 %.
fn draw_battery(f: &mut Frame, power: Power, low: bool, shown_ms: u64, now: u64) {
    let pct = power.percent.unwrap_or(0).min(100) as i32;
    let hue = match pct {
        0..=10 => RED,
        11..=20 => AMBER,
        _ => DONE,
    };
    SMALL.draw(f, if power.on_usb == Some(true) { "ON USB" } else { "BATT" }, MAIN.x0, 1, DIM);
    // A low battery blinks its number: the one other thing on the panel that asks you to act.
    if !low || now % 800 < 500 {
        let text = power.percent.map_or("--".to_string(), |p| p.to_string());
        let px = MAIN.x1 + 1 - 4 - BIG.width(&text);
        BIG.draw(f, &text, px, 0, if low { RED } else { WHITE });
        SMALL.draw(f, "%", MAIN.x1 - 2, 2, DIM);
    }
    // The gauge fills up to the charge as the notice arrives.
    let span = MAIN.x1 - 1 - RAIL_X0;
    let grown = (shown_ms as f32 / 400.0).min(1.0);
    // (Never shorter than the smallest capsule, so a nearly flat cell still shows a red nub.)
    let end = RAIL_X0 + ((span as f32 * pct as f32 / 100.0 * grown).round() as i32).max(2);
    for x in RAIL_X0..=end {
        for y in RAIL_Y - 1..=RAIL_Y + 1 {
            if (x != RAIL_X0 && x != end) || y == RAIL_Y {
                f.set(x, y, hue);
            }
        }
    }
    for x in (end + 3..=MAIN.x1 - 1).step_by(4) {
        f.set(x, RAIL_Y, DIM);
    }
}

fn draw_power_off_countdown(f: &mut Frame, left_ms: u64, now: u64) {
    if now % 800 < 500 {
        SMALL.draw(f, "LOW BATT", MAIN.x0, 1, RED);
    }
    SMALL.draw(f, "OFF IN", MAIN.x0, LINE2_Y + 1, WHITE);
    let secs = left_ms.div_ceil(1000).to_string();
    BIG.draw(f, &secs, MAIN.x1 + 1 - BIG.width(&secs), 9, WHITE);
}

/// Two plain lines across the whole panel, for when there is nothing to show yet (no host, no agents).
pub fn render_notice(f: &mut Frame, line1: &str, line2: &str, now: u64) {
    f.clear();
    f.reset_clip();
    SMALL.draw(f, line1, 1, LINE1_Y, WHITE);
    draw_marquee(f, line2, 1, LINE2_Y, W as i32 - 2, DIM, now);
}

/// The colour a model goes by. The names are Claude Code's and come and go; one that is not here yet still gets
/// drawn, in a colour of its own.
fn model_hue(m: Model) -> Rgb {
    match m.word() {
        "OPUS" => OPUS,
        "FABLE" => FABLE,
        "SONNET" => SONNET,
        "HAIKU" => HAIKU,
        _ => MODEL,
    }
}

/// `MODEL EFFORT`. Carded, the model name becomes a chip in its hue and the effort stays lit text beside it:
/// the effort is what the buttons change, and a field under both words would leave neither any padding.
fn draw_model_effort(f: &mut Frame, model: Model, effort: Option<Effort>, x: i32, y: i32, style: Style, now: u64) {
    let word = effort.map_or("", Effort::word);
    // What the two words may take between them: a chip spends a pixel either side of the name, and may use the
    // panel's last column. A long model name gives way to the effort word, which is what the buttons change:
    // `SONN XHIGH`.
    let room = if style.carded() { CARD_CLIP.x1 - x - 2 } else { MAIN.x1 - x - 1 };
    let mut name = model.word();
    while !word.is_empty() && name.len() > 1 && SMALL.width(name) + SMALL.width(word) > room {
        name = &name[..name.len() - 1];
    }
    let mw = SMALL.width(name);
    let lit = |c: Rgb| if style == Style::Dim { c.scale(DIM_STYLE_LEVEL) } else { c };
    let hue = model_hue(model);
    let wx = if style.carded() {
        let tx = draw_card(f, x, y, mw, 1, field_color(style, hue));
        SMALL.draw(f, name, tx, y, if style == Style::Card { Rgb::OFF } else { hue });
        f.set_clip(CARD_CLIP);
        tx + mw + 2
    } else {
        SMALL.draw(f, name, x, y, lit(hue)) + 2
    };
    if effort != Some(Effort::Ultra) {
        SMALL.draw(f, word, wx, y, lit(WHITE));
    } else {
        // Ultracode keeps a violet tag on screen the way Claude Code keeps one in the prompt border,
        // with the keyword shimmer sweeping across it.
        let w = SMALL.width(word);
        let glimmer = wx - 10 + (now / SHIMMER_STEP_MS % (w as u64 + 20)) as i32;
        let mut scratch = Frame::new();
        SMALL.draw(&mut scratch, word, wx, y, ULTRA);
        for py in y..y + SMALL.h {
            for px in wx..wx + w {
                if scratch.get(px, py) != Rgb::OFF {
                    f.set(px, py, lit(if (px - glimmer).abs() <= 1 { ULTRA_SHIMMER } else { ULTRA }));
                }
            }
        }
    }
    f.set_clip(MAIN);
}

/// Rounded field for a row's text, 1 px taller than it above and below, starting at `x`.
/// Returns where the text goes.
fn draw_card(f: &mut Frame, x: i32, y: i32, w: i32, pad: i32, c: Rgb) -> i32 {
    f.set_clip(CARD_CLIP);
    let r = Rect { x0: x, y0: y - 1, x1: x + 2 * pad + w - 1, y1: y + SMALL.h };
    f.fill_rect(r, c);
    for (cx, cy) in [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1), (r.x1, r.y1)] {
        f.set(cx, cy, Rgb::OFF);
    }
    f.set_clip(MAIN);
    x + pad
}

/// What a card is filled with, given the colour its text would have when plain.
///
/// A lit card is full level, and pure white where the text was the panel's warm white: held back, it just
/// looks grey next to lit text. A tinted card is dark and saturated, never grey: the panel lifts every channel
/// that is on at all to its floor level, which makes faint grey the brightest faint colour there is and
/// washes out faint mixed hues. So a tint drops the hue's weak channels, and white becomes deep blue.
fn field_color(style: Style, c: Rgb) -> Rgb {
    let top = c.0.max(c.1).max(c.2);
    let whitish = c.0.min(c.1).min(c.2) > 150;
    match style {
        Style::Card if whitish => Rgb(255, 255, 255),
        Style::Card => c,
        _ if whitish => Rgb(0, 0, 72),
        _ => {
            let keep = |v: u8| if v as u32 * 4 >= top as u32 { (v as f32 * 0.25).round() as u8 } else { 0 };
            Rgb(keep(c.0), keep(c.1), keep(c.2))
        }
    }
}

/// Claude Code's "violet-ripple": rings expanding from the ultracode stop. Behind the wavefront the field
/// stays washed in violet, ahead of it stays dark. Square LEDs, so no terminal aspect correction.
fn draw_ripple(f: &mut Frame, ox: i32, oy: i32, elapsed_ms: u64) {
    let travel = elapsed_ms as f32 * RIPPLE_SPEED_PX_PER_MS;
    for y in MAIN.y0..=MAIN.y1 {
        for x in MAIN.x0..=MAIN.x1 {
            let d = (((x - ox).pow(2) + (y - oy).pow(2)) as f32).sqrt();
            if d > travel {
                continue;
            }
            let phase = (d - travel).rem_euclid(RIPPLE_WAVELENGTH_PX);
            let u = (1.0 + (std::f32::consts::TAU * phase / RIPPLE_WAVELENGTH_PX).cos()) / 2.0;
            f.set(x, y, RIPPLE[(u * 7.0).round() as usize]);
        }
    }
}

/// How long a label needs to pause, scroll to its end and pause there; 0 when it fits.
fn marquee_pass_ms(s: &str, avail: i32) -> u64 {
    match (SMALL.width(s) - avail).max(0) as u64 {
        0 => 0,
        over => 2 * MARQUEE_PAUSE_MS + over * MARQUEE_STEP_MS,
    }
}

/// Strip brightness. The focused block is the brightest thing in the strip; agents with nothing going on
/// recede; a blocked one keeps breathing (the only motion in the strip, so motion always means "act") but
/// peaks under the focused level.
fn block_level(status: Status, focused: bool, now: u64) -> f32 {
    match (status, focused) {
        (Status::Blocked, true) => 0.8 + 0.2 * phase(now, 900),
        (Status::Blocked, false) => 0.25 + 0.4 * phase(now, 900),
        (_, true) => 1.0,
        (Status::Working | Status::Done, false) => 0.4,
        (Status::Idle | Status::Unknown, false) => 0.18,
    }
}

/// Text that fits stays put; longer text pauses, scrolls to its end, pauses, scrolls back.
fn draw_marquee(f: &mut Frame, s: &str, x: i32, y: i32, avail: i32, c: Rgb, t: u64) {
    let over = (SMALL.width(s) - avail).max(0) as u64;
    let (pause, step) = (MARQUEE_PAUSE_MS, MARQUEE_STEP_MS);
    let p = t % (2 * pause + 2 * over * step).max(1);
    let off = if p < pause {
        0
    } else if p < pause + over * step {
        (p - pause) / step
    } else if p < 2 * pause + over * step {
        over
    } else {
        over - (p - 2 * pause - over * step) / step
    };
    SMALL.draw(f, s, x - off as i32, y, c);
}

#[allow(clippy::too_many_arguments)]
fn draw_effort(
    f: &mut Frame,
    hue: Rgb,
    ultra_ok: bool,
    from: Effort,
    target: Effort,
    moved_at: u64,
    dir: i32,
    bump_at: Option<u64>,
    committed_at: Option<u64>,
    now: u64,
) {
    let bump = bump_at.map_or(0, |t| bump_offset(now.saturating_sub(t), dir));
    let since_move = now.saturating_sub(moved_at);
    let ultra = target == Effort::Ultra;
    // On ultracode the rail takes Claude Code's covered-track colour so it reads over the ripple.
    let hue = if ultra { ULTRA_SHIMMER } else { hue };

    if ultra {
        draw_ripple(f, stop_x(Effort::Ultra, true), RAIL_Y, since_move);
    }

    // Level word enters from the side you pressed towards.
    let word = target.word();
    let wx = MAIN.x0 + (MAIN_W - BIG.width(word)) / 2 + slide_offset(since_move, &[8, 5, 2, -1, 0]) * dir + bump;
    BIG.draw(f, word, wx, 0, if ultra { Rgb(255, 255, 255) } else { WHITE });

    let (from_x, to_x) = (stop_x(from, ultra_ok), stop_x(target, ultra_ok));
    let t = (since_move as f32 / KNOB_GLIDE_MS as f32).min(1.0);
    let kx = (from_x as f32 + (to_x - from_x) as f32 * back_out(t)).round() as i32 + bump;

    // Filled capsule up to the knob; stops behind it are pin-holes, stops ahead are small dots.
    for x in RAIL_X0..kx {
        for y in RAIL_Y - 1..=RAIL_Y + 1 {
            if x == RAIL_X0 && y != RAIL_Y {
                continue;
            }
            f.set(x, y, hue);
        }
    }
    for &e in Effort::stops(ultra_ok) {
        let sx = stop_x(e, ultra_ok);
        if sx < kx - 3 {
            f.set(sx, RAIL_Y, Rgb::OFF);
        } else if sx > kx + 3 {
            f.disc(sx, RAIL_Y, 1, if e == Effort::Ultra { ULTRA } else { DIM });
        }
    }
    if ultra_ok && ULTRA_DIVIDER_X < kx - 3 {
        // The gap that sets ultracode apart from the five levels.
        f.fill_rect(Rect { x0: ULTRA_DIVIDER_X, y0: RAIL_Y - 1, x1: ULTRA_DIVIDER_X, y1: RAIL_Y + 1 }, Rgb::OFF);
    }
    let sent = committed_at.is_some_and(|t| now.saturating_sub(t) < 60);
    f.disc(kx, RAIL_Y, 3, WHITE);
    f.disc(kx, RAIL_Y, 2, if sent { WHITE } else if ultra { RIPPLE[7] } else { hue });
}

fn draw_model(f: &mut Frame, target: Model, ctx_used: u32, changed_at: u64, committed_at: Option<u64>, now: u64) {
    let hue = model_hue(target);
    let word = target.word();
    let ww = BIG.width(word);
    let armed = committed_at.is_none();
    // Beside a six-letter name there is room for the small question mark only, and the two are centred together.
    let small_mark = ww + 1 + BIG.w > MAIN_W - (MAIN_W - ww) / 2;
    let wx = MAIN.x0 + (MAIN_W - ww - if small_mark { 1 + SMALL.w } else { 0 }) / 2;

    // Card flip: the word grows out of its centre line.
    let s = (now.saturating_sub(changed_at) as f32 / FLIP_MS as f32).min(1.0);
    let flash = committed_at.is_some_and(|t| now.saturating_sub(t) < 120);
    let mut scratch = Frame::new();
    BIG.draw(&mut scratch, word, wx, 0, WHITE);
    for ry in 0..BIG.h {
        let y = 3 + ((ry - 3) as f32 * s).round() as i32;
        for x in 0..W as i32 {
            if scratch.get(x, ry) != Rgb::OFF {
                f.set(x, y, if flash { hue } else { WHITE });
            }
        }
    }
    // Armed: a blinking question mark asks for the second press.
    if armed && now.saturating_sub(changed_at) % 600 < 400 {
        match small_mark {
            false => BIG.draw(f, "?", wx + ww + 1, 0, hue),
            true => SMALL.draw(f, "?", wx + ww + 1, 2, hue),
        };
    }

    // What the switch costs: this much context gets re-read uncached.
    let cost = tokens_short(ctx_used);
    SMALL.draw(f, &cost, MAIN.x0 + (MAIN_W - SMALL.width(&cost)) / 2, 8, if armed { WHITE } else { DIM });

    // Underline is the time left to confirm: it drains while armed, and is whole once the switch is sent.
    let left = match committed_at {
        Some(_) => 1.0,
        None => 1.0 - (now.saturating_sub(changed_at) as f32 / CONFIRM_WINDOW_MS as f32).min(1.0),
    };
    let uw = (ww as f32 * left).round() as i32;
    let ux = wx + (ww - uw) / 2;
    if uw > 0 {
        f.fill_rect(Rect { x0: ux, y0: 14, x1: ux + uw - 1, y1: 15 }, if armed { hue.scale(0.5) } else { hue });
    }
}

/// Hand-authored offsets, one per ~16 ms frame: at 5 px of travel a table beats an easing function.
fn slide_offset(elapsed_ms: u64, table: &[i32]) -> i32 {
    table[((elapsed_ms / 16) as usize).min(table.len() - 1)]
}

/// End-stop / refused-input bump: +1, -1, settle.
fn bump_offset(elapsed_ms: u64, dir: i32) -> i32 {
    match elapsed_ms / 33 {
        0 => dir,
        1 => -dir,
        _ => 0,
    }
}

fn back_out(t: f32) -> f32 {
    let (c1, c3) = (1.70158, 2.70158);
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

/// 0..1 cosine wave with the given period.
fn phase(now: u64, period_ms: u64) -> f32 {
    let t = (now % period_ms) as f32 / period_ms as f32;
    0.5 + 0.5 * (t * std::f32::consts::TAU).cos()
}

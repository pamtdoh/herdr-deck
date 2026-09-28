//! The pet: a dango, a round dumpling that lives in the black space of the resting screen. It goes only where
//! nothing is lit, gives way when the text grows into it, and acts out what the focused agent is doing: it hums and
//! thinks while the agent works, munches when the context grows and sweats when it fills up, startles and asks for
//! you when the agent is blocked (and gets cross when that goes on), lights up when it is done, yawns and falls
//! asleep when it idles.
//!
//! Its means are the ones small-sprite games have always used: eyes that look where it goes (Pac-Man's ghosts), a
//! balloon beside its head for what it feels (RPG Maker, Tamagotchi), and the whole body: a hop, squash and stretch,
//! a sway, a flush of colour (Lemmings, Kirby).
//!
//! Like everything in this crate it is pure: `tick` is handed what the resting screen lights, what the focused agent
//! is doing and the time; `draw` puts the dango on top of the screen.

use crate::frame::{Frame, Rgb, H, W};
use crate::state::Status;

mod dango;

/// Whether there is a pet, and in what colour: the dango's own mint, or one of a few pastels that stay clear of the
/// text's white and of the colours the strip gives the agents.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Pet {
    #[default]
    Off,
    Mint,
    Pink,
    Peach,
    Lemon,
    Sky,
    Lilac,
}

impl Pet {
    /// The choice, its config word, what the settings page calls it, and the colour.
    pub const ALL: [(Pet, &'static str, &'static str, Rgb); 7] = [
        (Pet::Off, "off", "OFF", Rgb::OFF),
        (Pet::Mint, "mint", "MINT", Rgb(120, 225, 170)),
        (Pet::Pink, "pink", "PINK", Rgb(255, 120, 190)),
        (Pet::Peach, "peach", "PEACH", Rgb(255, 170, 110)),
        (Pet::Lemon, "lemon", "LEMON", Rgb(235, 220, 80)),
        (Pet::Sky, "sky", "SKY", Rgb(100, 175, 255)),
        (Pet::Lilac, "lilac", "LILAC", Rgb(185, 140, 255)),
    ];

    pub fn word(self) -> &'static str {
        Pet::ALL.iter().find(|p| p.0 == self).map_or("", |p| p.2)
    }

    pub fn rgb(self) -> Rgb {
        Pet::ALL.iter().find(|p| p.0 == self).map_or(Rgb::OFF, |p| p.3)
    }
}

/// What the pet goes by: the focused agent, as the resting screen shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cue {
    pub status: Status,
    pub ctx_used: u32,
    pub ctx_pct: u32,
    /// Which agent it is: a new value is the focus moving.
    pub agent: u64,
}

/// The first column the pet may use: the main area's.
pub const LEFT: i32 = 11;

/// What the resting screen lights, one bit per pixel.
#[derive(Clone, Copy, Debug, Default)]
pub struct Room([u64; H]);

impl Room {
    pub fn of(f: &Frame) -> Room {
        let mut rows = [0u64; H];
        for (y, row) in rows.iter_mut().enumerate() {
            for x in 0..W {
                if f.get(x as i32, y as i32) != Rgb::OFF {
                    *row |= 1 << x;
                }
            }
        }
        Room(rows)
    }

    fn lit(&self, x: i32, y: i32) -> bool {
        (0..W as i32).contains(&x) && (0..H as i32).contains(&y) && self.0[y as usize] >> x & 1 == 1
    }

    /// What a walking body cannot be in: anything lit, the strip's side, above and below the panel. Past its right
    /// edge is open, so the pet can walk off the panel and come back.
    fn solid(&self, x: i32, y: i32) -> bool {
        y < 0 || y >= H as i32 || x < LEFT || self.lit(x, y)
    }

    /// Whether its box fits at (x, y) with a pixel to spare to its sides and above, so that it does not grind along
    /// the text as it goes. Underneath it may touch: that is what it stands on.
    fn fits(&self, x: i32, y: i32) -> bool {
        (y - AIR..y + MH).all(|cy| (x - AIR..x + MW + AIR).all(|cx| !self.solid(cx, cy)))
    }

    fn ground(&self, x: i32, y: i32) -> bool {
        (x..x + MW).any(|cx| self.solid(cx, y + MH))
    }

    /// How far the dark reaches around a box at (x, y), a few pixels each way at most: the leftmost and rightmost
    /// columns and the top row a body there may be drawn in. It may come right up to what is lit.
    fn around(&self, x: i32, y: i32) -> (i32, i32, i32) {
        let clear = |c: i32| (y..y + MH).all(|r| !self.lit(c, r));
        let (mut lo, mut hi, mut top) = (x, x + MW - 1, y);
        while lo > LEFT && lo > x - 4 && clear(lo - 1) {
            lo -= 1;
        }
        while hi < x + MW + 3 && clear(hi + 1) {
            hi += 1;
        }
        while top > 0 && top > y - 3 && (x..x + MW).all(|c| !self.lit(c, top - 1)) {
            top -= 1;
        }
        (lo, hi, top)
    }

    /// A pixel a balloon may use: dark, on the panel, and not in a gap inside the text (lit on both sides, or above
    /// and below), where it would read as part of a letter. Beside the text is fine.
    fn roomy(&self, (x, y): (i32, i32)) -> bool {
        (LEFT..W as i32).contains(&x)
            && (0..H as i32).contains(&y)
            && !self.lit(x, y)
            && !(self.lit(x - 1, y) && self.lit(x + 1, y))
            && !(self.lit(x, y - 1) && self.lit(x, y + 1))
    }

}

/// The room the pet keeps for itself, and the air round it; its body may bulge past it when it changes shape.
const MW: i32 = 9;
const MH: i32 = 6;
const AIR: i32 = 1;

const RED: Rgb = Rgb(255, 40, 40);
const YELLOW: Rgb = Rgb(255, 210, 60);
const BLUE: Rgb = Rgb(80, 160, 255);
const GREY: Rgb = Rgb(150, 150, 160);
const LAVENDER: Rgb = Rgb(130, 130, 220);
const HEART: Rgb = Rgb(255, 50, 100);

/// A balloon beside its head.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Emote {
    Bang,
    Question,
    /// Thinking: dots rising one after another.
    Dots,
    Note,
    Heart,
    /// A drop running down the side of its head.
    Sweat,
    /// Throbbing.
    Anger,
    Bulb,
    /// Twinkling.
    Sparkle,
    Zzz,
    /// Circling its head.
    Stars,
}

/// A moment it acts out, whatever it was doing: a second or two, then back to its mood.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum React {
    /// The agent got blocked.
    Startle,
    /// The agent is done.
    Eureka,
    /// The agent went idle.
    Yawn,
    /// The context grew: tokens are food.
    Munch,
    /// The context shrank (a compact, a clear): lighter.
    Relief,
    /// The text grew into it and shoved it along.
    Dizzy,
    /// The focus moved to another agent: it looks over at the strip.
    Notice,
}

impl React {
    fn ms(self) -> u64 {
        match self {
            React::Startle => 1400,
            React::Eureka => 1800,
            React::Yawn => 1600,
            React::Munch => 1000,
            React::Relief => 1600,
            React::Dizzy => 1500,
            React::Notice => 1000,
        }
    }
}

/// No walls to squeeze a body: well off the panel on every side.
const UNBOUNDED: (i32, i32, i32) = (-100, 100, -100);

/// The pet's clock: everything it does happens on these steps.
const STEP_MS: u64 = 40;
/// Hidden longer than this (an overlay was up), it carries on from where it was rather than catch up.
const RESUME_MS: u64 = 1000;
/// How high an in-place hop goes, step by step.
const HOP: [i32; 8] = [1, 2, 2, 3, 2, 2, 1, 0];
/// Blocked: a hop per breath of the strip's blocked block; done: a hop per two breaths of its done block.
const ALARM_MS: u64 = 900;
const CHEER_MS: u64 = 1800;
/// Blocked this long, it stops asking and gets cross.
const CROSS_AFTER_MS: u64 = 20_000;
/// It munches on new context at most this often.
const MUNCH_EVERY_MS: u64 = 8000;
/// A drop in context this big is a compact or a clear.
const RELIEF_TOKENS: u32 = 10_000;

/// What the pet is after, from the focused agent's status.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mood {
    /// Working: it goes about humming and stops to think, now and then leaves the panel for a moment.
    Busy,
    /// Blocked: it comes into view, hops and sways, flushing red in time with the strip.
    Alarm,
    /// Done and not looked at yet: it comes into view and beams.
    Proud,
    /// Idle: it comes into view, settles and sleeps.
    Sleepy,
    /// Nothing known: it ambles and wonders.
    Amble,
}

impl Mood {
    fn of(s: Status) -> Mood {
        match s {
            Status::Working => Mood::Busy,
            Status::Blocked => Mood::Alarm,
            Status::Done => Mood::Proud,
            Status::Idle => Mood::Sleepy,
            Status::Unknown => Mood::Amble,
        }
    }

    fn wants_to_be_seen(self) -> bool {
        matches!(self, Mood::Alarm | Mood::Proud | Mood::Sleepy)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Act {
    /// This many pixels more, then something else.
    Walk { left: u32 },
    Stand { until: u64 },
    /// Along an arc from where it was to `to`, `at` steps in; from == to is a hop on the spot. `then`: what is left
    /// of the walk the leap was part of.
    Jump { from: (i32, i32), to: (i32, i32), at: u32, len: u32, then: u32 },
    Sleep { since: u64 },
    Cheer,
    Alarm,
    /// Off the panel until then.
    Away { until: u64 },
}

#[derive(Clone, Copy, Debug)]
pub struct PetState {
    on: bool,
    x: i32,
    y: i32,
    facing: i32,
    act: Act,
    mood: Mood,
    clock: u64,
    /// Steps since the act began: the rhythm of its slide.
    n: u32,
    landed_at: u64,
    seed: u32,
    /// What it is acting out, and since when.
    react: Option<(React, u64)>,
    /// The focused agent as last seen, and since when it has had its status.
    cue: Option<Cue>,
    status_since: u64,
    munched_at: Option<u64>,
    /// The dark around its box as of the last tick, for a body that changes shape: the columns it may be drawn in
    /// and its topmost row, a pixel of air short of anything lit.
    free: (i32, i32, i32),
    /// What the resting screen lit as of the last tick, which its balloons keep clear of too.
    room: Room,
}

impl Default for PetState {
    fn default() -> Self {
        PetState::new(false, 0)
    }
}

impl PetState {
    /// Just off the panel's right edge, on the floor, about to come in.
    fn new(on: bool, now: u64) -> PetState {
        PetState {
            on,
            x: W as i32,
            y: H as i32 - MH,
            facing: -1,
            act: Act::Walk { left: MW as u32 + 4 },
            mood: Mood::Amble,
            clock: now,
            n: 0,
            landed_at: 0,
            seed: 0x9e37_79b9 ^ now as u32,
            react: None,
            cue: None,
            status_since: now,
            munched_at: None,
            free: UNBOUNDED,
            room: Room([0; H]),
        }
    }

    /// Where its box is, while it is on or at the panel: for tests.
    #[cfg(test)]
    pub(crate) fn body(&self) -> Option<(i32, i32, i32, i32)> {
        (self.on && !matches!(self.act, Act::Away { .. })).then_some((self.x, self.y, MW, MH))
    }

    fn rand(&mut self, n: u32) -> u32 {
        let mut s = self.seed;
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        self.seed = s;
        s % n.max(1)
    }

    fn set(&mut self, act: Act) {
        self.act = act;
        self.n = 0;
    }

    /// Advances the pet to `now`, if there is one. `room` is what the resting screen lights right now, `cue` the
    /// focused agent.
    pub fn tick(&mut self, on: bool, room: &Room, cue: Cue, now: u64) {
        if on != self.on {
            *self = PetState::new(on, now);
        }
        if !on {
            return;
        }
        if now.saturating_sub(self.clock) > RESUME_MS {
            self.clock = now - STEP_MS;
        }
        self.notice(room, cue);
        while self.clock + STEP_MS <= now {
            self.clock += STEP_MS;
            self.n += 1;
            self.step(room);
        }
        (self.free, self.room) = (room.around(self.x, self.y), *room);
    }

    /// What changed about the focused agent since the last look, and what that is worth acting out.
    fn notice(&mut self, room: &Room, cue: Cue) {
        let now = self.clock;
        let before = self.cue.replace(cue);
        let react = match before {
            None => None,
            Some(b) if b.agent != cue.agent => Some(React::Notice),
            Some(b) if b.status != cue.status => match (b.status, cue.status) {
                (_, Status::Blocked) => Some(React::Startle),
                (_, Status::Done) => Some(React::Eureka),
                (_, Status::Idle) => Some(React::Yawn),
                // Woken by work.
                (Status::Idle, _) => Some(React::Startle),
                _ => None,
            },
            Some(b) if cue.ctx_used + RELIEF_TOKENS < b.ctx_used => Some(React::Relief),
            Some(b) if cue.ctx_used > b.ctx_used && self.munched_at.is_none_or(|m| now.saturating_sub(m) >= MUNCH_EVERY_MS) => {
                self.munched_at = Some(now);
                Some(React::Munch)
            }
            _ => None,
        };
        if before.is_none_or(|b| b.status != cue.status || b.agent != cue.agent) {
            self.status_since = now;
        }
        let mood = Mood::of(cue.status);
        if mood != self.mood {
            self.mood = mood;
            self.decide(room, now);
        }
        if let Some(r) = react {
            self.act_out(room, r);
        }
    }

    fn act_out(&mut self, room: &Room, r: React) {
        self.react = Some((r, self.clock));
        let on_ground = !matches!(self.act, Act::Jump { .. } | Act::Away { .. }) && room.ground(self.x, self.y);
        if matches!(r, React::Startle | React::Eureka | React::Relief) && on_ground {
            self.hop();
        }
        // Woken up: out of bed first.
        if matches!(self.act, Act::Sleep { .. }) && r != React::Yawn {
            self.set(Act::Stand { until: self.clock + r.ms() });
        }
    }

    /// How much of it sticks out past the panel's right edge.
    fn out(&self) -> i32 {
        (self.x + MW - W as i32).max(0)
    }

    fn pct(&self) -> u32 {
        self.cue.map_or(0, |c| c.ctx_pct)
    }

    /// Steps per pixel walked: brisk when it matters, slower as the context fills.
    fn pace(&self) -> u32 {
        match self.mood {
            Mood::Alarm => 2,
            Mood::Busy if self.pct() >= 90 => 8,
            Mood::Busy if self.pct() >= 70 => 6,
            Mood::Busy | Mood::Proud => 4,
            Mood::Sleepy => 5,
            Mood::Amble => 7,
        }
    }

    /// What to do next, when the last thing is done or the mood has changed.
    fn decide(&mut self, room: &Room, now: u64) {
        let out = self.out();
        // Every mood but work and not knowing wants to be seen: first back onto the panel. Well off it, it comes
        // back rather than stop out there.
        if out > 0 && self.mood.wants_to_be_seen() || out > MW / 2 {
            self.facing = -1;
            let left = out as u32 + self.rand(if self.mood.wants_to_be_seen() { 4 } else { 8 });
            return self.set(Act::Walk { left });
        }
        let grounded = room.ground(self.x, self.y);
        match self.mood {
            Mood::Alarm if grounded => self.set(Act::Alarm),
            Mood::Proud if grounded => self.set(Act::Cheer),
            Mood::Sleepy if grounded => match self.act {
                // Drowsy first, then asleep.
                Act::Stand { .. } => self.set(Act::Sleep { since: now }),
                _ => self.set(Act::Stand { until: now + 1500 }),
            },
            Mood::Busy | Mood::Amble => {
                let r = self.rand(10);
                let (busy, tired) = (self.mood == Mood::Busy, self.pct() >= 90);
                if self.rand(3) == 0 {
                    self.facing = -self.facing;
                }
                if r < if busy && !tired { 6 } else { 4 } {
                    let left = 3 + self.rand(if busy { 14 } else { 6 });
                    self.set(Act::Walk { left });
                } else if busy && !tired && r == 9 && grounded {
                    self.hop();
                } else {
                    // Stops to think (or, worn out, to get its breath back).
                    let until = now + if busy { 1200 + self.rand(1800) } else { 1500 + self.rand(3000) } as u64;
                    self.set(Act::Stand { until });
                }
            }
            _ => self.set(Act::Stand { until: now + 200 }),
        }
    }

    fn hop(&mut self) {
        let at = (self.x, self.y);
        self.set(Act::Jump { from: at, to: at, at: 0, len: HOP.len() as u32, then: 0 });
    }

    fn step(&mut self, room: &Room) {
        let now = self.clock;
        if self.react.is_some_and(|(r, since)| now.saturating_sub(since) >= r.ms()) {
            self.react = None;
        }
        if let Act::Away { until } = self.act {
            if now >= until || self.mood.wants_to_be_seen() {
                (self.x, self.y, self.facing) = (W as i32, H as i32 - MH, -1);
                let left = MW as u32 + 1 + self.rand(8);
                self.set(Act::Walk { left });
            }
            return;
        }
        // The text grew into it: it gives way towards the open edge at once, rather than be drawn over, and is
        // left dizzy by the shove.
        if !room.fits(self.x, self.y) {
            while !room.fits(self.x, self.y) && self.x < W as i32 + AIR {
                self.x += 1;
            }
            if self.x < W as i32 && self.react.is_none() {
                self.react = Some((React::Dizzy, now));
            }
            if !matches!(self.act, Act::Walk { .. }) {
                self.set(Act::Walk { left: 1 });
            }
            return;
        }
        if let Act::Jump { from, to, at, len, then } = self.act {
            return self.jump(room, (from, to), at, len, then);
        }
        if !room.ground(self.x, self.y) {
            self.y += 1;
            if room.ground(self.x, self.y) {
                self.landed_at = now;
            }
            return;
        }
        // Acting something out, it stays where it is.
        if self.react.is_some() {
            return;
        }
        let cross = now.saturating_sub(self.status_since) >= CROSS_AFTER_MS;
        match self.act {
            Act::Walk { left } => {
                if !self.n.is_multiple_of(self.pace()) {
                    return;
                }
                if left == 0 {
                    return self.decide(room, now);
                }
                self.act = Act::Walk { left: left - 1 };
                self.walk(room);
                // Walked off the right edge.
                if self.x >= W as i32 && self.facing > 0 && matches!(self.act, Act::Walk { .. }) {
                    let until = now + 1500 + self.rand(4000) as u64;
                    self.set(Act::Away { until });
                }
            }
            Act::Stand { until } if now >= until => self.decide(room, now),
            // In time with the strip: a hop per breath of a blocked block (until it is cross and stamps
            // instead), one per two of a done one.
            Act::Alarm if !cross && now % ALARM_MS < STEP_MS => self.hop(),
            Act::Cheer if now % (2 * CHEER_MS) < STEP_MS => self.hop(),
            _ => {}
        }
    }

    /// One pixel on: along the ground, up a one-pixel step, off a ledge, or leaping onto another; turning back
    /// where there is nowhere to go. Past the right edge it goes only now and then, and only when busy.
    fn walk(&mut self, room: &Room) {
        let (x, y, d) = (self.x, self.y, self.facing);
        let leaving = d > 0 && self.out() >= MW / 2;
        if leaving && (self.mood != Mood::Busy || self.rand(3) != 0) {
            self.facing = -d;
            return;
        }
        if room.fits(x + d, y) {
            if room.ground(x + d, y) || self.rand(2) == 0 {
                self.x += d;
            } else if !self.leap(room) {
                self.facing = -d;
            }
        } else if room.fits(x + d, y - 1) && room.ground(x + d, y - 1) {
            (self.x, self.y) = (x + d, y - 1);
        } else if !self.leap(room) {
            self.facing = -d;
        }
    }

    /// A ledge ahead within reach, and a clear arc to it.
    fn leap(&mut self, room: &Room) -> bool {
        let (x, y, d) = (self.x, self.y, self.facing);
        let down_first = self.rand(2) == 0;
        for k in 2..=6 {
            for j in 0..12 {
                let dy = if down_first { 4 - j } else { j - 7 };
                let to = (x + d * k, y + dy);
                let len = arc_len(k, dy);
                let clear = || {
                    (1..len).all(|i| {
                        let (ax, ay) = arc((x, y), to, i, len);
                        room.fits(ax, ay)
                    })
                };
                if dy != 0 && room.fits(to.0, to.1) && room.ground(to.0, to.1) && clear() {
                    let then = match self.act {
                        Act::Walk { left } => left,
                        _ => 0,
                    };
                    self.set(Act::Jump { from: (x, y), to, at: 0, len, then });
                    return true;
                }
            }
        }
        false
    }

    fn jump(&mut self, room: &Room, (from, to): ((i32, i32), (i32, i32)), at: u32, len: u32, then: u32) {
        let at = at + 1;
        // A hop on the spot goes as high as there is room for: under the text that may be not at all.
        let lift = |k: i32| (0..=k).rev().find(|&k| room.fits(from.0, from.1 - k)).unwrap_or(0);
        let (nx, ny) = if from == to { (from.0, from.1 - lift(HOP[(at as usize).min(HOP.len() - 1)])) } else { arc(from, to, at, len) };
        if !room.fits(nx, ny) {
            // Something came in the way: it drops from where it is.
            return self.set(Act::Walk { left: 1 });
        }
        (self.x, self.y) = (nx, ny);
        if at < len {
            self.act = Act::Jump { from, to, at, len, then };
            return;
        }
        self.landed_at = self.clock;
        let resume = match self.mood {
            // A leap was part of a walk, which goes on after it.
            _ if from != to => Act::Walk { left: then },
            Mood::Alarm => Act::Alarm,
            Mood::Proud => Act::Cheer,
            _ => Act::Walk { left: 1 + self.rand(4) },
        };
        self.set(resume);
    }

    /// Draws the pet on the resting screen, in `color`. It never covers a lit pixel.
    pub fn draw(&self, f: &mut Frame, now: u64, color: Rgb) {
        if self.on {
            dango::draw(self, f, now, color);
        }
    }
}

/// A balloon by a head whose leftmost and rightmost pixels are in columns `left` and `right` and whose top is row
/// `y`: beside it on the right, with a pixel of air between them, or on the left where the right has no room. Its
/// bottom row is level with the head's second row, so a balloon needs little more height than the pet. It may come
/// up against the text but not into it: a picture that fits on neither side is left out rather than drawn in among
/// the letters, and what drifts or circles fades where it would. `age`: how long it has been up.
fn balloon(f: &mut Frame, room: &Room, e: Emote, (left, right): (i32, i32), y: i32, age: u64, now: u64) {
    let at = |ex: i32, rows: &'static [&'static str]| {
        let n = rows.len() as i32;
        rows.iter().enumerate().flat_map(move |(r, row)| {
            row.bytes().enumerate().filter(|&(_, b)| b != b'.').map(move |(k, b)| (ex + k as i32, y + 2 - n + r as i32, b))
        })
    };
    // `#` in its colour, `o` in grey.
    let icon = |f: &mut Frame, rows: &'static [&'static str], c: Rgb| {
        if let Some(ex) = [right + 2, left - 4].into_iter().find(|&ex| at(ex, rows).all(|(x, y, _)| room.roomy((x, y)))) {
            for (x, y, b) in at(ex, rows) {
                put(f, x, y, if b == b'o' { GREY } else { c });
            }
        }
    };
    let aired = |f: &mut Frame, x: i32, y: i32, c: Rgb| {
        if room.roomy((x, y)) {
            put(f, x, y, c);
        }
    };
    let on_right = right + 5 <= W as i32;
    let ex = if on_right { right + 2 } else { left - 4 };
    match e {
        // Pops up blinking.
        Emote::Bang if age % 400 < 280 => icon(f, &[".#.", ".#.", "...", ".#."], RED),
        Emote::Bang => {}
        Emote::Question => icon(f, &["##.", "..#", ".#.", "...", ".#."], YELLOW),
        // Bobbing to the tune.
        Emote::Note if now % 600 < 300 => icon(f, &[".##", ".#.", "##.", "##.", "..."], YELLOW),
        Emote::Note => icon(f, &[".##", ".#.", "##.", "##."], YELLOW),
        Emote::Heart => icon(f, &["#.#", "###", ".#."], HEART),
        Emote::Anger if now % 500 < 250 => icon(f, &["#.#", "...", "#.#"], RED),
        Emote::Anger => icon(f, &["...", ".#.", "..."], RED),
        Emote::Bulb if age < 600 && age % 200 >= 100 => {}
        Emote::Bulb => icon(f, &["###", "###", ".o."], YELLOW),
        Emote::Sparkle if now % 300 < 150 => icon(f, &[".#.", "###", ".#."], YELLOW),
        Emote::Sparkle => icon(f, &["#.#", ".#.", "#.#"], YELLOW),
        // One dot, two, three, rising away like a thought.
        Emote::Dots => {
            let shown = (age / 400 % 4) as i32;
            for i in 0..shown.min(3) {
                aired(f, if on_right { ex + i } else { ex + 2 - i }, y + 1 - 2 * i, GREY);
            }
        }
        // Runs down the side of its head.
        Emote::Sweat => {
            let sx = if on_right { ex - 1 } else { left - 1 };
            let dy = (age / 200 % 3) as i32;
            aired(f, sx, y - 1 + dy, BLUE);
            aired(f, sx, y + dy, BLUE);
        }
        Emote::Zzz => zzz(f, room, ex - 3, y + 1, age),
        Emote::Stars => {
            let (cx, r) = ((left + ex - 1) / 2, (ex - 1 - left) as f32 / 2.0);
            for k in 0..2 {
                let a = age as f32 / 180.0 + k as f32 * std::f32::consts::PI;
                aired(f, cx + (a.cos() * r).round() as i32, y - 1 + a.sin().round() as i32, YELLOW);
            }
        }
    }
}

/// Zs rising from (x, y), one after another, drifting right, or left where the panel ends; a Z that would run
/// into the text is not drawn at all rather than in pieces.
fn zzz(f: &mut Frame, room: &Room, x: i32, y: i32, age: u64) {
    let (x, drift) = if x + 5 < W as i32 { (x, 1) } else { (x - 2, -1) };
    for k in 0..2u64 {
        if age < k * 1200 {
            continue;
        }
        let t = (age - k * 1200) % 2400;
        let rise = (t / 600) as i32;
        let c = LAVENDER.scale(1.0 - t as f32 / 2400.0 * 0.6);
        let z = [(0, 0), (1, 0), (1, 1), (0, 2), (1, 2)].map(|(dx, dy)| (x + drift * rise + dx, y - 2 - rise + dy));
        if z.iter().all(|&p| room.roomy(p)) {
            for (zx, zy) in z {
                put(f, zx, zy, c);
            }
        }
    }
}

/// 0..1 and back over `period_ms`, peaking at the start of the period like the strip's breath.
fn pulse(now: u64, period_ms: u64) -> f32 {
    let t = (now % period_ms) as f32 / period_ms as f32;
    0.5 + 0.5 * (t * std::f32::consts::TAU).cos()
}

fn mix(a: Rgb, b: Rgb, k: f32) -> Rgb {
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * k).round() as u8;
    Rgb(m(a.0, b.0), m(a.1, b.1), m(a.2, b.2))
}

/// The settings page's picture of it, bottom left at (x, bottom): its faces one after another.
pub fn preview(pet: Pet, f: &mut Frame, x: i32, bottom: i32, now: u64) {
    if pet != Pet::Off {
        dango::preview(f, x, bottom, now, pet.rgb());
    }
}

/// How long the settings page shows each of its faces.
const GALLERY_MS: u64 = 1000;

/// Only onto what is dark: the text always wins.
fn put(f: &mut Frame, x: i32, y: i32, c: Rgb) {
    if f.get(x, y) == Rgb::OFF {
        f.set(x, y, c);
    }
}

/// Steps for a leap of `dx` across and `dy` down.
fn arc_len(dx: i32, dy: i32) -> u32 {
    (dx.abs() + dy.abs() + 3) as u32
}

/// Where a leap is `i` of `len` steps in: a straight line from `from` to `to`, lifted by a parabola that takes it
/// about two pixels over the higher end (on a panel 16 px tall there is no room to spare above).
fn arc(from: (i32, i32), to: (i32, i32), i: u32, len: u32) -> (i32, i32) {
    let s = i as f32 / len as f32;
    let lift = 2.0 + (from.1 - to.1).abs() as f32 / 2.0;
    let x = from.0 as f32 + (to.0 - from.0) as f32 * s;
    let y = from.1 as f32 + (to.1 - from.1) as f32 * s - 4.0 * lift * s * (1.0 - s);
    (x.round() as i32, y.round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue(status: Status, ctx_used: u32) -> Cue {
        Cue { status, ctx_used, ctx_pct: ctx_used / 10_000, agent: 1 }
    }

    /// Ticks the way the device does, every 40 ms, from `from` up to `to`.
    fn run(p: &mut PetState, room: &Room, c: Cue, from: u64, to: u64) {
        for t in (from..=to).step_by(STEP_MS as usize) {
            p.tick(true, room, c, t);
        }
    }

    /// The pet standing on the floor of an empty room, well inside the panel, looking at a working agent; and the
    /// time.
    fn settled() -> (PetState, Room, u64) {
        let (mut p, room) = (PetState::default(), Room::default());
        let mut t = 0;
        while !(p.x + MW < W as i32 && matches!(p.act, Act::Stand { .. } | Act::Walk { .. }) && room.ground(p.x, p.y)) || t < 3000 {
            p.tick(true, &room, cue(Status::Working, 50_000), t);
            t += STEP_MS;
        }
        (p, room, t)
    }

    #[test]
    fn what_happens_to_the_agent_is_acted_out() {
        let cases = [
            (cue(Status::Blocked, 50_000), Some(React::Startle)),
            (cue(Status::Done, 50_000), Some(React::Eureka)),
            (cue(Status::Idle, 50_000), Some(React::Yawn)),
            (cue(Status::Working, 60_000), Some(React::Munch)),
            (cue(Status::Working, 17_000), Some(React::Relief)),
            (Cue { agent: 2, ..cue(Status::Working, 50_000) }, Some(React::Notice)),
            (cue(Status::Working, 50_000), None),
        ];
        for (next, expected) in cases {
            let (mut p, room, t) = settled();
            p.react = None;
            p.tick(true, &room, next, t);
            assert_eq!(p.react.map(|r| r.0), expected, "{next:?}");
        }
        // Munching is now and then, not on every report.
        let (mut p, room, t) = settled();
        p.tick(true, &room, cue(Status::Working, 60_000), t);
        p.react = None;
        run(&mut p, &room, cue(Status::Working, 70_000), t, t + 2000);
        assert_eq!(p.react, None);
        // Woken by work.
        let (mut p, room, t) = settled();
        run(&mut p, &room, cue(Status::Idle, 50_000), t, t + 8000);
        assert!(matches!(p.act, Act::Sleep { .. }), "{:?}", p.act);
        p.tick(true, &room, cue(Status::Working, 50_000), t + 8040);
        assert_eq!(p.react.map(|r| r.0), Some(React::Startle));
    }

    #[test]
    fn shoved_by_the_text_it_is_left_dizzy_where_it_fits() {
        let (mut p, room, t) = settled();
        let (x, y) = (p.x, p.y);
        let mut grown = room;
        for row in grown.0.iter_mut() {
            *row |= (1u64 << (x + 2)) - 1;
        }
        p.react = None;
        p.tick(true, &grown, cue(Status::Working, 50_000), t);
        p.tick(true, &grown, cue(Status::Working, 50_000), t + STEP_MS);
        assert!(p.x > x && grown.fits(p.x, p.y) && p.y == y, "{x} -> {}", p.x);
        assert_eq!(p.react.map(|r| r.0), Some(React::Dizzy));
    }

    #[test]
    fn with_no_headroom_a_blocked_pet_stays_put_rather_than_edge_away() {
        let (mut p, mut room, t) = settled();
        let (x, y) = (p.x, p.y);
        // Text right over its head: no room to hop.
        room.0[(y - 2) as usize] = u64::MAX;
        run(&mut p, &room, cue(Status::Blocked, 50_000), t, t + 10_000);
        assert_eq!((p.x, p.y), (x, y));
    }

    #[test]
    fn a_long_block_turns_from_worried_to_cross() {
        let (mut p, room, t) = settled();
        run(&mut p, &room, cue(Status::Blocked, 50_000), t, t + 5000);
        let worried = dango::look(&p, t + 5000);
        assert_eq!(worried.emote.map(|e| e.0), Some(Emote::Bang));
        run(&mut p, &room, cue(Status::Blocked, 50_000), t + 5040, t + 21_000);
        let cross = dango::look(&p, t + 21_000);
        assert_eq!(cross.emote.map(|e| e.0), Some(Emote::Anger));
        assert!(cross.shape.w > worried.shape.w, "puffed up: {:?} after {:?}", cross.shape, worried.shape);
    }
}

//! The dango: a round dumpling. No arms, no legs and no mouth: it slides, its front
//! reaching out and its back catching up, and its whole body is the expression. It stretches up when startled or
//! pleased, puffs up when cross, sways when it wants you, melts flat when worn out or asleep. The rest is in its two
//! eyes, which change shape and move: they look where it goes, drift up while it thinks, dart about when it is
//! worried, roll when it is dizzy, glance round when it wonders, look up when it is pleased, shut when it is
//! content.
//!
//! The outline is not a set of pictures but worked out from three numbers, so that it can take any shape between
//! them: how wide it is, how tall, and how far its top leans.

use super::*;

/// Its body at one moment.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) struct Shape {
    pub(super) w: i32,
    h: i32,
    /// How far its top leans, towards +x.
    lean: f32,
}

const fn shape(w: i32, h: i32, lean: f32) -> Shape {
    Shape { w, h, lean }
}

/// Sitting still: it fills the box the walking engine keeps room for.
const REST: Shape = shape(9, 6, 0.0);
/// Stretched up: startled, delighted, yawning.
const TALL: Shape = shape(9, 7, 0.0);
/// A slide, pixel by pixel: the front reaches out and flattens, the back catches up. Leaning the way it goes.
const STRIDE: [Shape; 4] = [shape(9, 6, 0.5), shape(10, 6, 1.0), shape(11, 5, 1.0), shape(10, 5, 0.5)];
/// Where a row stops following the dome and flattens out towards the base, from the bottom (0) to the top (1).
const BASE: f32 = 0.3;
/// Coming down from a hop: flat, then a wobble back to round.
const LANDING: [(u64, Shape); 3] = [(80, shape(12, 4, 0.0)), (160, shape(11, 5, 0.0)), (240, shape(10, 5, 0.0))];

impl Shape {
    /// Whether (x, y) is inside it, for a dango whose middle is at column `cx` and whose base is row `ground`: an
    /// ellipse from the top down to its widest, full width below that, and the bottom row a pixel in at either end,
    /// so that it sits on the ground as a ball rather than spreading over it like a slime.
    fn covers(self, cx: f32, ground: i32, x: i32, y: i32) -> bool {
        if y > ground || y <= ground - self.h {
            return false;
        }
        let (a, v) = (self.w as f32 / 2.0, (ground - y) as f32 + 0.5);
        let v = v / self.h as f32;
        let half = if v >= BASE {
            let t = (v - BASE) / (1.0 - BASE);
            a * (1.0 - t * t).sqrt()
        } else {
            a - 0.01
        };
        let half = if y == ground { half - 1.0 } else { half };
        (x as f32 - cx - self.shear(v)).abs() <= half
    }

    /// How far a row at height `v` (0 bottom, 1 top) is pushed by the lean: the whole ball leans over its base.
    fn shear(self, v: f32) -> f32 {
        self.lean * v
    }
}

/// The shape of its eyes. Where they look is a `Look`'s `gaze`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Eyes {
    /// Two pixels tall.
    Open,
    /// Three tall: startled.
    Wide,
    /// One pixel, low: lids half down.
    Drowsy,
    /// One pixel: small enough to roll round.
    Dot,
    /// `- -`
    Shut,
}

/// How it looks at one moment.
pub(super) struct Look {
    pub(super) shape: Shape,
    eyes: Eyes,
    /// Where its eyes turn: -1, 0 or 1 across (left to right) and up (-1) or down.
    gaze: (i32, i32),
    pub(super) emote: Option<(Emote, u64)>,
}

impl Look {
    fn new(shape: Shape, eyes: Eyes) -> Look {
        Look { shape, eyes, gaze: (0, 0), emote: None }
    }

    fn gazing(self, gaze: (i32, i32)) -> Look {
        Look { gaze, ..self }
    }

    fn with(self, e: Emote, age: u64) -> Look {
        Look { emote: Some((e, age)), ..self }
    }
}

/// Eyes on the move, one place after another, `ms` in each.
fn wander(now: u64, ms: u64, places: &[(i32, i32)]) -> (i32, i32) {
    places[(now / ms) as usize % places.len()]
}

/// Worried: glancing from side to side.
const DART: [(i32, i32); 4] = [(-1, 0), (1, 0), (-1, 0), (0, 0)];
/// Dizzy: round and round.
const ROLL: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];
/// Wondering: this way, that way, up.
const WONDER: [(i32, i32); 4] = [(-1, 0), (1, 0), (0, -1), (0, 0)];
/// Thinking: up and away, one side and the other.
const PONDER: [(i32, i32); 2] = [(-1, -1), (1, -1)];
/// Pleased: looking up, beaming.
const BEAM: (i32, i32) = (0, -1);

/// From what the pet is doing to how the dango looks: its moods and moments, played with the body and the eyes.
pub(super) fn look(p: &PetState, now: u64) -> Look {
    let airborne = matches!(p.act, Act::Jump { .. });
    let landed = now.saturating_sub(p.landed_at);
    let walking = matches!(p.act, Act::Walk { .. }) && p.react.is_none();
    let d = p.facing as f32;
    let landing = LANDING.iter().find(|(until, _)| landed < *until).map(|&(_, s)| s);
    // Hops stretch it going up and squash it where it lands; that comes before anything it is feeling.
    let (physical, still) = match (airborne, landing) {
        (true, _) => (Some(shape(8, 7, 0.0)), false),
        (_, Some(s)) => (Some(s), false),
        _ if walking => {
            let s = STRIDE[(p.n / p.pace()) as usize % STRIDE.len()];
            (Some(Shape { lean: s.lean * d, ..s }), false)
        }
        _ => (None, true),
    };
    let body = |s: Shape| physical.unwrap_or(s);
    let beat = |ms: u64| now % (2 * ms) < ms;
    if let Some((r, since)) = p.react {
        let t = now.saturating_sub(since);
        return match r {
            React::Startle => Look::new(body(TALL), Eyes::Wide).with(Emote::Bang, t),
            React::Eureka => Look::new(body(TALL), Eyes::Open).gazing(BEAM).with(Emote::Bulb, t),
            React::Yawn if t < 1000 => Look::new(body(TALL), Eyes::Shut),
            React::Yawn => Look::new(body(shape(10, 5, 0.0)), Eyes::Drowsy),
            // Chewing, eyes shut in bliss: cheeks out and in, the eyes riding up and down with them.
            React::Munch => Look::new(body(if (t / 160).is_multiple_of(2) { shape(10, 5, 0.0) } else { REST }), Eyes::Shut),
            // A sigh: wide and low, then round again.
            React::Relief => {
                let s = if t < 500 { shape(11, 5, 0.0) } else { REST };
                Look::new(body(s), Eyes::Shut).with(Emote::Sparkle, t)
            }
            // Reeling, eyes rolling.
            React::Dizzy => {
                let lean = if beat(150) { 1.5 } else { -1.5 };
                Look::new(body(shape(9, 6, lean)), Eyes::Dot).gazing(wander(t, 110, &ROLL)).with(Emote::Stars, t)
            }
            // It turns to the strip, where the agents are.
            React::Notice => Look::new(body(shape(9, 6, -1.0)), Eyes::Open).gazing((-1, 0)),
        };
    }
    if let Act::Sleep { since } = p.act {
        // Melted flat, breathing slowly.
        let s = if now.saturating_sub(since) % 3000 < 1500 { shape(12, 3, 0.0) } else { shape(11, 4, 0.0) };
        return Look::new(s, Eyes::Shut).with(Emote::Zzz, now.saturating_sub(since));
    }
    let since_status = now.saturating_sub(p.status_since);
    match p.mood {
        Mood::Busy => {
            let pct = p.pct();
            let mut l = match () {
                // Worn out: sagging and heaving, lids heavy.
                _ if pct >= 90 => Look::new(body(if beat(600) { shape(12, 4, 0.0) } else { shape(11, 5, 0.0) }), Eyes::Drowsy),
                _ if walking => Look::new(body(REST), Eyes::Open).gazing((p.facing, 0)),
                // Stopped to think: eyes up, drifting from one side to the other.
                _ => Look::new(body(REST), Eyes::Open).gazing(wander(now, 1400, &PONDER)),
            };
            l.emote = if pct >= 70 && now % 5000 < 1500 {
                Some((Emote::Sweat, now % 5000))
            } else if walking && now % 9000 < 1800 {
                Some((Emote::Note, now % 9000))
            } else if matches!(p.act, Act::Stand { .. }) && pct < 90 {
                Some((Emote::Dots, p.n as u64 * STEP_MS))
            } else {
                None
            };
            l
        }
        // Swaying from side to side to be seen, eyes darting; after a while puffed up and stamping, staring
        // straight out.
        Mood::Alarm if since_status >= CROSS_AFTER_MS => {
            let s = if still && beat(250) { shape(11, 5, 0.0) } else { shape(11, 6, 0.0) };
            Look::new(body(s), Eyes::Open).with(Emote::Anger, since_status)
        }
        Mood::Alarm => {
            let lean = if beat(450) { 1.2 } else { -1.2 };
            Look::new(body(shape(9, 6, lean)), Eyes::Open).gazing(wander(now, 300, &DART)).with(Emote::Bang, since_status)
        }
        // Squishing happily, beaming.
        Mood::Proud => {
            let l = Look::new(body(if still && beat(450) { shape(10, 5, 0.0) } else { REST }), Eyes::Open).gazing(BEAM);
            if now % 4000 < 1600 { l.with(Emote::Heart, now % 4000) } else { l }
        }
        // Settling down, lids heavy.
        Mood::Sleepy => Look::new(body(shape(10, 5, 0.0)), Eyes::Drowsy),
        // Tilting its head this way and that, glancing round.
        Mood::Amble => {
            let side = if beat(1200) { 1 } else { -1 };
            let l = match walking {
                true => Look::new(body(REST), Eyes::Open).gazing((p.facing, 0)),
                false => Look::new(body(shape(9, 6, side as f32)), Eyes::Open).gazing(wander(now, 600, &WONDER)),
            };
            if now % 6000 < 1500 { l.with(Emote::Question, now % 6000) } else { l }
        }
    }
}

/// Draws the dango, its box's left at `x` and its base on row `ground`, within `free` (the columns it may use and
/// its highest row), and returns the columns of its outermost pixels and its top row, for a balloon.
fn paint(f: &mut Frame, x: i32, ground: i32, facing: i32, free: (i32, i32, i32), l: &Look, color: Rgb) -> ((i32, i32), i32) {
    let (lo, hi, highest) = free;
    // Squeezed by what is lit around it, it gives way: narrower and taller between walls, lower under a ceiling.
    let w = l.shape.w.min(hi - lo + 1);
    let h = (l.shape.h + (l.shape.w - w + 1) / 2).min(ground - highest + 1).min(7);
    let s = Shape { w, h, ..l.shape };
    // An even width has no middle pixel: it is centred half a pixel towards where it faces.
    let cx = x as f32 + (MW - 1) as f32 / 2.0 + if s.w % 2 == 0 { 0.5 * facing.signum() as f32 } else { 0.0 };
    let reach = (s.w - 1) as f32 / 2.0;
    let cx = cx.clamp(lo as f32 + reach, (hi as f32 - reach).max(lo as f32 + reach));
    let top = ground - s.h + 1;
    let ey = top + (s.h as f32 * 0.45).round() as i32;
    // Two eyes in the middle of the body, mirrored exactly, a pixel or two apart.
    let mid = ((cx + s.shear(((ground - ey) as f32 + 0.5) / s.h as f32)) * 2.0).round() / 2.0;
    let sep = if mid.fract() != 0.0 { 1.5 } else { 1.0 };
    let eye = |side: i32| -> &'static [(i32, i32)] {
        match (l.eyes, side) {
            (Eyes::Open, _) => &[(0, -1), (0, 0)],
            (Eyes::Wide, _) => &[(0, -1), (0, 0), (0, 1)],
            (Eyes::Drowsy, _) => &[(0, 0)],
            (Eyes::Dot, _) => &[(0, -1)],
            (Eyes::Shut, -1) => &[(0, 0), (-1, 0)],
            (Eyes::Shut, _) => &[(0, 0), (1, 0)],
        }
    };
    // Every mark of an eye keeps two pixels of body on its outer side and one above and below; otherwise the
    // outline beside it would be a lone pixel. Looking that far, the eyes turn back towards the middle; on a narrow
    // top they sit a row lower.
    let marks = |(gx, gy): (i32, i32), ey: i32| {
        [-1, 1].into_iter().flat_map(move |side| {
            let ex = (mid + side as f32 * sep) as i32 + gx;
            eye(side).iter().map(move |&(dx, dy)| (ex + dx, ey + dy + gy, side))
        })
    };
    let clear = |gaze: (i32, i32), ey: i32| {
        marks(gaze, ey).all(|(mx, my, side)| {
            (1..=2).all(|k| s.covers(cx, ground, mx + side * k, my)) && s.covers(cx, ground, mx, my - 1) && s.covers(cx, ground, mx, my + 1)
        })
    };
    let (gx, gy) = l.gaze;
    let tries = [(0, (gx, gy)), (0, (0, gy)), (0, (gx, 0)), (0, (0, 0)), (1, (0, 0)), (2, (0, 0))];
    let (drop, gaze) = tries.into_iter().find(|&(d, g)| clear(g, ey + d)).unwrap_or((1, (0, 0)));
    let mut holes = [(i32::MIN, 0); 8];
    for (slot, (mx, my, _)) in holes.iter_mut().zip(marks(gaze, ey + drop)) {
        *slot = (mx, my);
    }
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for y in top..=ground {
        for px in x - 3..x + MW + 3 {
            if s.covers(cx, ground, px, y) {
                (lo, hi) = (lo.min(px), hi.max(px));
                if !holes.contains(&(px, y)) {
                    put(f, px, y, color);
                }
            }
        }
    }
    ((lo, hi), top)
}

pub(super) fn draw(p: &PetState, f: &mut Frame, now: u64, own: Rgb) {
    if matches!(p.act, Act::Away { .. }) {
        return;
    }
    let l = look(p, now);
    let color = match p.act {
        // Blocked: it flushes red with every breath of the strip's blocked block.
        _ if p.mood == Mood::Alarm => mix(own, RED, pulse(now, ALARM_MS)),
        Act::Sleep { .. } => own.scale(0.6),
        _ => own,
    };
    let (edges, top) = paint(f, p.x, p.y + MH - 1, p.facing, p.free, &l, color);
    if let Some((e, age)) = l.emote {
        balloon(f, &p.room, e, edges, top, age, now);
    }
}

/// The settings page's picture: every face it pulls, a second each: working, thinking, munching, sweating, worn
/// out, startled, worried, cross, delighted, pleased, relieved, dizzy, wondering, yawning, asleep.
pub(super) fn preview(f: &mut Frame, x: i32, bottom: i32, now: u64, own: Rgb) {
    let (i, t) = ((now / GALLERY_MS % 15) as usize, now % GALLERY_MS);
    let beat = |ms: u64| now % (2 * ms) < ms;
    let l = match i {
        0 => Look::new(STRIDE[(now / 200 % 4) as usize], Eyes::Open).gazing((1, 0)).with(Emote::Note, t),
        1 => Look::new(REST, Eyes::Open).gazing(wander(now, 500, &PONDER)).with(Emote::Dots, t),
        2 => Look::new(if beat(160) { shape(10, 5, 0.0) } else { REST }, Eyes::Shut),
        3 => Look::new(REST, Eyes::Open).with(Emote::Sweat, t),
        4 => Look::new(if beat(600) { shape(12, 4, 0.0) } else { shape(11, 5, 0.0) }, Eyes::Drowsy),
        5 => Look::new(TALL, Eyes::Wide).with(Emote::Bang, t),
        6 => Look::new(shape(9, 6, if beat(450) { 1.2 } else { -1.2 }), Eyes::Open).gazing(wander(now, 300, &DART)).with(Emote::Bang, t),
        7 => Look::new(if beat(250) { shape(11, 5, 0.0) } else { shape(11, 6, 0.0) }, Eyes::Open).with(Emote::Anger, t),
        8 => Look::new(TALL, Eyes::Open).gazing(BEAM).with(Emote::Bulb, t),
        9 => Look::new(if beat(450) { shape(10, 5, 0.0) } else { REST }, Eyes::Open).gazing(BEAM).with(Emote::Heart, t),
        10 => Look::new(if t < 500 { shape(11, 5, 0.0) } else { REST }, Eyes::Shut).with(Emote::Sparkle, t),
        11 => Look::new(shape(9, 6, if beat(150) { 1.5 } else { -1.5 }), Eyes::Dot).gazing(wander(now, 110, &ROLL)).with(Emote::Stars, t),
        12 => Look::new(shape(9, 6, if beat(500) { 1.0 } else { -1.0 }), Eyes::Open).gazing(wander(now, 250, &WONDER)).with(Emote::Question, t),
        13 => Look::new(TALL, Eyes::Shut),
        _ => Look::new(if now % 3000 < 1500 { shape(12, 3, 0.0) } else { shape(11, 4, 0.0) }, Eyes::Shut).with(Emote::Zzz, now % 2400),
    };
    let color = match i {
        6 | 7 => mix(own, RED, pulse(now, ALARM_MS)),
        14 => own.scale(0.6),
        _ => own,
    };
    // Room at the left for its widest shapes.
    let (edges, top) = paint(f, x + 2, bottom - 1, 1, UNBOUNDED, &l, color);
    if let Some((e, age)) = l.emote {
        balloon(f, &Room::default(), e, edges, top, age, now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(s: Shape) -> Vec<(i32, i32)> {
        let cx = 20.0 + if s.w % 2 == 0 { 0.5 } else { 0.0 };
        (15 - s.h + 1..=15)
            .map(|y| {
                let xs: Vec<i32> = (0..40).filter(|&x| s.covers(cx, 15, x, y)).collect();
                (xs[0], *xs.last().unwrap())
            })
            .collect()
    }

    fn ascii(s: Shape) -> String {
        rows(s).iter().map(|&(a, b)| (10..32).map(|x| if (a..=b).contains(&x) { '#' } else { '.' }).collect::<String>() + "\n").collect()
    }

    /// Every shape it takes is one smooth lump, round underneath: an outline that goes in and out again reads on the
    /// panel as a stray pixel rather than as part of the body.
    #[test]
    fn every_shape_is_one_round_lump() {
        let mut shapes = vec![REST, TALL, shape(8, 7, 0.0), shape(11, 6, 0.0), shape(12, 3, 0.0), shape(11, 4, 0.0)];
        shapes.extend(LANDING.map(|(_, s)| s));
        for s in STRIDE {
            shapes.extend([s, Shape { lean: -s.lean, ..s }]);
        }
        for lean in [1.2, -1.2, 1.5, -1.5, 1.0, -1.0] {
            shapes.push(shape(9, 6, lean));
        }
        if std::env::var("DANGO_SHAPES").is_ok() {
            for s in &shapes {
                println!("{s:?}\n{}", ascii(*s));
            }
        }
        for s in shapes {
            let r = rows(s);
            // Each side bulges out once and comes back in: widening down to its widest, narrowing after.
            let (lefts, rights): (Vec<i32>, Vec<i32>) = r.iter().copied().unzip();
            let rights: Vec<i32> = rights.iter().map(|x| -x).collect();
            for (side, edge) in [("left", &lefts), ("right", &rights)] {
                let widest = edge.iter().position(|e| e == edge.iter().min().unwrap()).unwrap();
                let bulges = edge[..=widest].windows(2).all(|p| p[1] <= p[0]) && edge[widest..].windows(2).all(|p| p[1] >= p[0]);
                assert!(bulges, "{s:?}: its {side} side goes in and out again:\n{}", ascii(s));
            }
            // Round underneath: the bottom row is a pixel in at both ends from the one above it.
            let (last, above) = (r[r.len() - 1], r[r.len() - 2]);
            assert!(last.0 > above.0 - 1 && last.1 < above.1 + 1 && last.1 - last.0 < above.1 - above.0, "{s:?}:\n{}", ascii(s));
        }
    }
}

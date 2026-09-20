//! The 52x16 framebuffer and its drawing primitives.

pub const W: usize = 52;
pub const H: usize = 16;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const OFF: Rgb = Rgb(0, 0, 0);

    pub fn scale(self, k: f32) -> Rgb {
        let f = |v: u8| (v as f32 * k.clamp(0.0, 1.0)).round() as u8;
        Rgb(f(self.0), f(self.1), f(self.2))
    }
}

/// Inclusive pixel rectangle.
#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x0: i32,
    pub y0: i32,
    pub x1: i32,
    pub y1: i32,
}

impl Rect {
    pub const FULL: Rect = Rect { x0: 0, y0: 0, x1: W as i32 - 1, y1: H as i32 - 1 };

    fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x0 && x <= self.x1 && y >= self.y0 && y <= self.y1
    }
}

#[derive(Clone)]
pub struct Frame {
    px: [Rgb; W * H],
    clip: Rect,
}

impl Default for Frame {
    fn default() -> Self {
        Self::new()
    }
}

impl Frame {
    pub fn new() -> Self {
        Frame { px: [Rgb::OFF; W * H], clip: Rect::FULL }
    }

    pub fn clear(&mut self) {
        self.px = [Rgb::OFF; W * H];
    }

    /// Row-major pixels, top-left first.
    pub fn pixels(&self) -> &[Rgb] {
        &self.px
    }

    /// Drawing outside the clip is dropped; lets content slide without spilling into its neighbours.
    pub fn set_clip(&mut self, clip: Rect) {
        self.clip = clip;
    }

    pub fn reset_clip(&mut self) {
        self.clip = Rect::FULL;
    }

    pub fn set(&mut self, x: i32, y: i32, c: Rgb) {
        if Rect::FULL.contains(x, y) && self.clip.contains(x, y) {
            self.px[y as usize * W + x as usize] = c;
        }
    }

    pub fn get(&self, x: i32, y: i32) -> Rgb {
        if Rect::FULL.contains(x, y) {
            self.px[y as usize * W + x as usize]
        } else {
            Rgb::OFF
        }
    }

    pub fn fill_rect(&mut self, r: Rect, c: Rgb) {
        for y in r.y0..=r.y1 {
            for x in r.x0..=r.x1 {
                self.set(x, y, c);
            }
        }
    }

    /// Filled disc of radius 1..=3 (3, 5 or 7 px across), hand-shaped so it reads round on the panel.
    pub fn disc(&mut self, cx: i32, cy: i32, r: i32, c: Rgb) {
        let widths: &[i32] = match r {
            1 => &[1, 3, 1],
            2 => &[3, 5, 5, 5, 3],
            _ => &[3, 5, 7, 7, 7, 5, 3],
        };
        let r = widths.len() as i32 / 2;
        for (i, w) in widths.iter().enumerate() {
            let y = cy - r + i as i32;
            for x in cx - w / 2..=cx + w / 2 {
                self.set(x, y, c);
            }
        }
    }

    /// `#` lit, `.` off — for tests and headless dumps.
    pub fn to_ascii(&self) -> String {
        let mut s = String::with_capacity((W + 1) * H);
        for y in 0..H {
            for x in 0..W {
                s.push(if self.px[y * W + x] == Rgb::OFF { '.' } else { '#' });
            }
            s.push('\n');
        }
        s
    }
}

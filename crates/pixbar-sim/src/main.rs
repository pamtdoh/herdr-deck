//! Desktop stand-in for the TC002: renders the UI as an LED matrix and fakes the herdr side.
//!
//!   up / down or mouse wheel   knob rotate (hold to spin)   enter   knob push; hold = settings
//!   left / right               effort buttons               space   model button (again to confirm)
//!   B  cycle focused agent's status   P  pull / plug the USB cable (the battery drops 15 % each pull)   esc  quit
//!   in settings (hold enter): up / down = page, left / right = value
//!
//! `pixbar-sim --dump` prints a scripted session as ASCII frames instead of opening a window;
//! with `PIXBAR_DUMP_DIR=<dir>` it also writes each frame as a colour PPM.

use std::time::Instant;

use minifb::{Key, KeyRepeat, Window, WindowOptions};
use pixbar_render::{Agent, Effort, Frame, Info, Input, Intent, Limit, Model, Power, Row, Show, Status, Style, Ui, World, H, W};

const CELL: usize = 16;
/// Pretend round trip to herdr + Claude Code before a change shows up in the world state.
const HOST_DELAY_MS: u64 = 400;

fn demo_world() -> World {
    let agent = |space: &str, tab: &str, status, model: &str, effort, used: u32, window: u32| Agent {
        reported: true,
        // As with Claude Code's smallest model, which has no effort levels.
        has_effort: model != "haiku",
        next_model: Some(Model::new(if model == "opus" { "fable" } else { "opus" })),
        space: space.into(),
        tab: tab.into(),
        status,
        model: Model::new(model),
        effort,
        ctx_used: used,
        ctx_window: window,
        // The 200K sessions stand in for ones without workflows: their rail has five stops.
        ultra_ok: window > 200_000,
        dir: format!("{space}-repo"),
        title: format!("Working on {space}"),
        fresh: used < 20_000,
        session: format!("{space} {tab}"),
        cost_cents: used / 70,
        limit_5h: Some(Limit { used_pct: (used / 8_000).min(100) as u8, resets_in_min: 200 }),
        limit_7d: Some(Limit { used_pct: 29, resets_in_min: 4 * 24 * 60 + 300 }),
    };
    World {
        agents: vec![
            agent("web", "2", Status::Working, "opus", Effort::XHigh, 104_000, 1_000_000),
            agent("parser", "1", Status::Blocked, "fable", Effort::High, 742_000, 1_000_000),
            agent("design-system", "1", Status::Idle, "fable", Effort::XHigh, 38_000, 1_000_000),
            agent("api", "3", Status::Done, "sonnet", Effort::Med, 186_000, 200_000),
            agent("docs", "1", Status::Working, "haiku", Effort::High, 12_000, 200_000),
        ],
        focused: 0,
    }
}

fn demo_info() -> Info {
    Info { addr: "192.0.2.7:17002".into(), ssid: "homenet".into(), hosts: vec!["DESKTOP USB".into()] }
}

/// Applies intents after a delay, the way the real bridge would confirm them.
#[derive(Default)]
struct FakeHost {
    pending: Vec<(u64, Intent)>,
}

impl FakeHost {
    fn send(&mut self, intent: Intent, now: u64) {
        self.pending.push((now + HOST_DELAY_MS, intent));
    }

    fn step(&mut self, world: &mut World, now: u64) {
        self.pending.retain(|&(due, intent)| {
            if due > now {
                return true;
            }
            match intent {
                Intent::Focus(i) => world.focused = i,
                Intent::SetEffort { agent, effort } => world.agents[agent].effort = effort,
                Intent::SetModel { agent, model } => {
                    let a = &mut world.agents[agent];
                    (a.model, a.next_model) = (model, Some(a.model));
                }
            }
            false
        });
    }
}

fn next_status(s: Status) -> Status {
    match s {
        Status::Idle => Status::Working,
        Status::Working => Status::Blocked,
        Status::Blocked => Status::Done,
        Status::Done => Status::Unknown,
        Status::Unknown => Status::Idle,
    }
}

fn dump() {
    let (mut world, mut ui, mut host, mut frame) = (demo_world(), Ui::new(), FakeHost::default(), Frame::new());
    ui.info = demo_info();
    let script: &[(u64, Option<Input>, &str)] = &[
        (3000, None, "rest"),
        (3000, Some(Input::Right), ""),
        (3040, None, "effort up, knob mid-glide"),
        (3400, None, "effort settled at MAX"),
        (3500, Some(Input::Right), ""),
        (3900, None, "past max: ULTRA, ripple starting at the knob"),
        (5600, None, "ULTRA: ripple has flooded the panel"),
        (5700, Some(Input::Right), ""),
        (5710, None, "end stop bump"),
        (9000, None, "rest: violet ULTRA tag with its shimmer"),
        (10000, Some(Input::Middle), ""),
        (10300, None, "model armed: 104K to re-read, underline draining, waiting for the second press"),
        (11000, Some(Input::Middle), ""),
        (11300, None, "second press: sent"),
        (13000, Some(Input::KnobCw), ""),
        (13040, Some(Input::KnobCw), ""),
        (13200, None, "knob: two quick clicks, picker on agent 3; focus is the bright block"),
        (19000, None, "6 s later the name is still up (linger setting)"),
        (19100, Some(Input::KnobPush), ""),
        (19200, None, "knob push: back to the resting screen at once"),
        (20000, Some(Input::KnobLong), ""),
        (20100, Some(Input::Right), ""),
        (20400, None, "settings: brightness, right pressed once"),
        (20500, Some(Input::KnobCw), ""),
        (20600, Some(Input::Right), ""),
        (20900, None, "settings: blocks, 3x3 touching; the strip is the preview"),
        (21000, Some(Input::Right), ""),
        (21010, Some(Input::Right), ""),
        (21300, None, "settings: 2x2 touching, 32 agents"),
        (21400, Some(Input::Left), ""),
        (21410, Some(Input::Left), ""),
        (21420, Some(Input::Left), ""),
        (21500, Some(Input::KnobCw), ""),
        (21800, None, "settings: row 1 shows the model, previewed live"),
        (21900, Some(Input::KnobCw), ""),
        (22000, Some(Input::Left), ""),
        (22300, None, "settings: style 1 = tint"),
        (22400, Some(Input::KnobCw), ""),
        (22410, Some(Input::KnobCw), ""),
        (22420, Some(Input::KnobCw), ""),
        (22500, Some(Input::Right), ""),
        (22900, None, "settings: name = dir"),
        (23000, Some(Input::KnobCw), ""),
        (23300, None, "settings: how long the name lingers"),
        (23400, Some(Input::KnobCw), ""),
        (23500, Some(Input::KnobCw), ""),
        (23800, None, "settings: hosts, and how each is connected"),
        (23810, Some(Input::KnobCw), ""),
        (23850, None, "settings: device, wifi network"),
        (23860, Some(Input::Right), ""),
        (23890, None, "settings: device, its address"),
        (23900, Some(Input::KnobPush), ""),
        (24200, None, "rest: model tinted, context plain"),
    ];
    let row = |show, style| Row { show, style };
    let card_variants = [
        ("cards, ultracode", [row(Show::Model, Style::Card), row(Show::Name, Style::Card)]),
        ("tints, ultracode", [row(Show::Model, Style::Tint), row(Show::Name, Style::Tint)]),
        ("model tint, context plain", [row(Show::Model, Style::Tint), row(Show::Context, Style::Plain)]),
        ("model plain, context dim", [row(Show::Model, Style::Plain), row(Show::Context, Style::Dim)]),
        ("model dim, context plain", [row(Show::Model, Style::Dim), row(Show::Context, Style::Plain)]),
        ("model plain, context tint", [row(Show::Model, Style::Plain), row(Show::Context, Style::Tint)]),
        ("name row + model card", [row(Show::Name, Style::Plain), row(Show::Model, Style::Card)]),
        ("low context: model card, context card", [row(Show::Model, Style::Card), row(Show::Context, Style::Card)]),
        ("low context: model tint, context tint", [row(Show::Model, Style::Tint), row(Show::Context, Style::Tint)]),
        ("low context: model plain, name card", [row(Show::Model, Style::Plain), row(Show::Name, Style::Card)]),
        ("low context: model tint, context dim", [row(Show::Model, Style::Tint), row(Show::Context, Style::Dim)]),
        ("name plain (full white) over model", [row(Show::Name, Style::Plain), row(Show::Model, Style::Plain)]),
        ("name dim over model", [row(Show::Name, Style::Dim), row(Show::Model, Style::Plain)]),
        ("usage windows: 5 hours over 7 days", [row(Show::Limit5h, Style::Plain), row(Show::Limit7d, Style::Dim)]),
        ("cost card over the 5-hour window as a tint", [row(Show::Cost, Style::Card), row(Show::Limit5h, Style::Tint)]),
    ];
    for &(t, input, caption) in script {
        if let Some(i) = input {
            if let Some(intent) = ui.input(&world, i, t) {
                host.send(intent, t);
            }
            continue;
        }
        // Run the timers up to `t` the way the 60 fps loop would.
        let mut now = t.saturating_sub(2500);
        while now <= t {
            host.step(&mut world, now);
            if let Some(intent) = ui.tick(&world, now) {
                host.send(intent, now);
            }
            now += 16;
        }
        ui.render(&world, t, &mut frame);
        println!("--- t={t} ms: {caption}\n{}", frame.to_ascii());
        if let Ok(dir) = std::env::var("PIXBAR_DUMP_DIR") {
            write_ppm(&frame, &format!("{dir}/frame-{t:05}.ppm"));
        }
    }
    // Battery: the cable comes out at 87 %, later the cell is nearly flat.
    let mut shot = |ui: &mut Ui, world: &World, t: u64, caption: &str| {
        ui.tick(world, t);
        ui.render(world, t, &mut frame);
        println!("--- t={t} ms: {caption}\n{}", frame.to_ascii());
        if let Ok(dir) = std::env::var("PIXBAR_DUMP_DIR") {
            write_ppm(&frame, &format!("{dir}/frame-{t:05}.ppm"));
        }
    };
    ui.set_power(Power { percent: Some(87), millivolts: Some(4160), on_usb: Some(true) }, 29_000);
    ui.set_power(Power { percent: Some(87), millivolts: Some(4050), on_usb: Some(false) }, 30_000);
    shot(&mut ui, &world, 31_000, "battery notice: cable pulled");
    ui.set_power(Power { percent: Some(9), millivolts: Some(3620), on_usb: Some(false) }, 35_000);
    shot(&mut ui, &world, 36_000, "battery notice: fell under 10 %");
    ui.set_power(Power { percent: Some(2), millivolts: Some(3540), on_usb: Some(false) }, 37_000);
    ui.tick(&world, 37_000);
    ui.tick(&world, 47_100);
    shot(&mut ui, &world, 52_300, "cell under 3.55 V for 10 s: counting down to power-off");
    ui.set_power(Power { percent: Some(3), millivolts: Some(3700), on_usb: Some(true) }, 53_000);
    shot(&mut ui, &world, 53_500, "cable back in: countdown gone, charging notice");
    let mut t = 60_000;
    ui.input(&world, Input::KnobLong, t);
    for _ in 0..2 {
        t += 100;
        ui.input(&world, Input::KnobCcw, t);
    }
    shot(&mut ui, &world, t + 400, "settings: turn off");
    ui.input(&world, Input::Right, t + 500);
    shot(&mut ui, &world, t + 700, "settings: turn off, armed");
    ui.input(&world, Input::KnobCcw, t + 800);
    shot(&mut ui, &world, t + 1200, "settings: device, battery");
    ui.input(&world, Input::KnobPush, t + 1300);

    world.focused = 0;
    ui.tick(&world, t + 2000);
    world.agents[0].effort = Effort::Ultra;
    world.agents[0].ctx_used = 760_000;
    for (i, (caption, rows)) in card_variants.into_iter().enumerate() {
        ui.settings.rows = rows;
        if caption.starts_with("low context") {
            world.agents[0].ctx_used = 104_000;
            world.agents[0].model = Model::new("opus");
            world.agents[0].effort = Effort::XHigh;
        }
        // Late enough that the name flash from the focus change above is over.
        let t = 70_000 + i as u64 * 100;
        ui.tick(&world, t);
        ui.render(&world, t, &mut frame);
        println!("--- t={t} ms: {caption}\n{}", frame.to_ascii());
        if let Ok(dir) = std::env::var("PIXBAR_DUMP_DIR") {
            write_ppm(&frame, &format!("{dir}/frame-{t:05}.ppm"));
        }
    }
}

/// Plain PPM at 12 px per LED with a 2 px dark gap, enough to judge colours and legibility.
fn write_ppm(frame: &Frame, path: &str) {
    const S: usize = 12;
    let mut out = format!("P6\n{} {}\n255\n", W * S, H * S).into_bytes();
    for y in 0..H * S {
        for x in 0..W * S {
            let led = frame.pixels()[(y / S) * W + x / S];
            let gap = x % S >= S - 2 || y % S >= S - 2;
            let off = led == pixbar_render::Rgb::OFF;
            out.extend_from_slice(&if gap { [10, 10, 10] } else if off { [22, 22, 22] } else { [led.0, led.1, led.2] });
        }
    }
    std::fs::write(path, out).expect("write ppm");
}

fn main() {
    if std::env::args().any(|a| a == "--dump") {
        return dump();
    }

    let (ww, wh) = (W * CELL, H * CELL);
    let mut window = Window::new(
        "pixbar-sim   knob: up/down/wheel + enter (hold: settings)   effort: left/right   model: space   B status",
        ww,
        wh,
        WindowOptions::default(),
    )
    .expect("open window");
    window.set_target_fps(60);

    // One LED: a disc with a little dark surround, like pixels behind the diffuser.
    let r = CELL as f32 / 2.0 - 1.5;
    let mask: Vec<bool> = (0..CELL * CELL)
        .map(|i| {
            let (dx, dy) = ((i % CELL) as f32 + 0.5 - CELL as f32 / 2.0, (i / CELL) as f32 + 0.5 - CELL as f32 / 2.0);
            dx * dx + dy * dy <= r * r
        })
        .collect();

    let (mut world, mut ui, mut host, mut frame) = (demo_world(), Ui::new(), FakeHost::default(), Frame::new());
    ui.info = demo_info();
    let mut power = Power { percent: Some(87), millivolts: Some(4160), on_usb: Some(true) };
    ui.set_power(power, 0);
    let mut buf = vec![0u32; ww * wh];
    let start = Instant::now();
    let mut wheel = 0.0f32;
    let mut enter_down: Option<(u64, bool)> = None;

    while window.is_open() && !window.is_key_down(Key::Escape) {
        let now = start.elapsed().as_millis() as u64;
        let mut inputs = Vec::new();
        // Arrow keys auto-repeat so holding one behaves like spinning the knob. Left and right repeat as the
        // device repeats its buttons: the first report of a key is the press, the rest say it is still down.
        let pressed = window.get_keys_pressed(KeyRepeat::No);
        for key in window.get_keys_pressed(KeyRepeat::Yes) {
            match key {
                Key::Up => inputs.push(Input::KnobCcw),
                Key::Down => inputs.push(Input::KnobCw),
                Key::Left if !pressed.contains(&key) => inputs.push(Input::LeftHeld),
                Key::Right if !pressed.contains(&key) => inputs.push(Input::RightHeld),
                _ => {}
            }
        }
        for key in window.get_keys_pressed(KeyRepeat::No) {
            match key {
                Key::Left => inputs.push(Input::Left),
                Key::Right => inputs.push(Input::Right),
                Key::Space => inputs.push(Input::Middle),
                Key::P => {
                    power.on_usb = power.on_usb.map(|usb| !usb);
                    if power.on_usb == Some(false) {
                        power.percent = power.percent.map(|p| p.saturating_sub(15));
                        power.millivolts = power.percent.map(|p| 3500 + p as u16 * 7);
                    }
                    ui.set_power(power, now);
                }
                Key::B => {
                    let i = world.focused;
                    world.agents[i].status = next_status(world.agents[i].status);
                }
                _ => {}
            }
        }
        // Enter is the knob's push: short on release, long-press once it has been held 600 ms.
        match (window.is_key_down(Key::Enter), enter_down) {
            (true, None) => enter_down = Some((now, false)),
            (true, Some((since, false))) if now - since >= 600 => {
                enter_down = Some((since, true));
                inputs.push(Input::KnobLong);
            }
            (false, Some((_, long))) => {
                enter_down = None;
                if !long {
                    inputs.push(Input::KnobPush);
                }
            }
            _ => {}
        }
        if let Some((_, dy)) = window.get_scroll_wheel() {
            wheel += dy;
            while wheel.abs() >= 1.0 {
                inputs.push(if wheel > 0.0 { Input::KnobCcw } else { Input::KnobCw });
                wheel -= wheel.signum();
            }
        }

        for input in inputs {
            if let Some(intent) = ui.input(&world, input, now) {
                host.send(intent, now);
            }
        }
        host.step(&mut world, now);
        if let Some(intent) = ui.tick(&world, now) {
            host.send(intent, now);
        }
        ui.render(&world, now, &mut frame);

        for (i, led) in frame.pixels().iter().enumerate() {
            let (px, py) = ((i % W) * CELL, (i / W) * CELL);
            let lit = if *led == pixbar_render::Rgb::OFF {
                0x181818
            } else {
                (led.0 as u32) << 16 | (led.1 as u32) << 8 | led.2 as u32
            };
            for (m, on) in mask.iter().enumerate() {
                buf[(py + m / CELL) * ww + px + m % CELL] = if *on { lit } else { 0x0c0c0c };
            }
        }
        window.update_with_buffer(&buf, ww, wh).expect("present frame");
    }
}

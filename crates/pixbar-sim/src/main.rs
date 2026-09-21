//! Desktop stand-in for the TC002: renders the UI as an LED matrix and fakes the herdr side.
//!
//!   up / down or mouse wheel   knob rotate (hold to spin)   enter   knob push = menu (name up: dismiss); hold = settings
//!   left / right               effort buttons               space   model button (again to confirm)
//!   B  cycle focused agent's status   P  pull / plug the USB cable (the battery drops 15 % each pull)   esc  quit
//!   in the menu (enter): up / down = turn the carousel, left / right = the entry's other choice, space = do it
//!   in settings (hold enter): up / down = page, left / right = value
//!
//! `pixbar-sim --dump` prints a scripted session as ASCII frames instead of opening a window;
//! with `PIXBAR_DUMP_DIR=<dir>` it also writes each frame as a colour PPM.

use std::time::Instant;

use minifb::{Key, KeyRepeat, Window, WindowOptions};
use pixbar_render::{Agent, Command, Effort, Frame, HostLink, Info, Input, Intent, Limit, Model, Power, Row, Show, Status, Style, Ui, World, H, W};

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
    Info {
        addr: "192.0.2.7:17002".into(),
        ssid: "homenet".into(),
        hosts: vec![HostLink { name: "DESKTOP".into(), wired: true }, HostLink { name: "LAPTOP".into(), wired: false }],
    }
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
                Intent::Run { agent, command } if agent < world.agents.len() => {
                    println!("menu: {command:?} on {}", world.agents[agent].space);
                    let a = &mut world.agents[agent];
                    match command {
                        Command::Compact => (a.ctx_used, a.fresh) = (17_000, true),
                        Command::Clear => (a.ctx_used, a.fresh, a.cost_cents) = (0, true, 0),
                        Command::CloseTab | Command::ClosePane if world.agents.len() > 1 => {
                            world.agents.remove(agent);
                            world.focused = world.focused.min(world.agents.len() - 1);
                        }
                        // Nothing of the others shows on the panel: the keyboard and herdr's window have the rest.
                        _ => {}
                    }
                }
                Intent::Run { .. } => {}
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
        (19180, None, "knob push while the name is up: back to the resting screen at once"),
        (19200, Some(Input::KnobPush), ""),
        (19280, None, "knob push at rest: the menu"),
        (19300, Some(Input::Right), ""),
        (19600, None, "menu: CLEAR, the first entry's other choice"),
        (19700, Some(Input::Middle), ""),
        (20500, None, "menu: CLEAR armed, fuses burning down, second press within 4 s does it"),
        (20600, Some(Input::KnobCw), ""),
        (20640, None, "menu: the knob dropped the armed CLEAR; one click on, the icons mid-slide"),
        (20900, None, "menu: RENAME TAB"),
        (21000, Some(Input::KnobCw), ""),
        (21300, None, "menu: SPLIT RIGHT"),
        (21400, Some(Input::Right), ""),
        (21700, None, "menu: SPLIT DOWN"),
        (21800, Some(Input::KnobCw), ""),
        (22100, None, "menu: CLOSE TAB"),
        (22200, Some(Input::Right), ""),
        (22500, None, "menu: CLOSE PANE"),
        (22600, Some(Input::KnobCw), ""),
        (23300, None, "menu: round to COMPACT, where it opens"),
        (23400, Some(Input::KnobPush), ""),
        (23500, None, "knob push again: back to the resting screen"),
        (30000, Some(Input::KnobLong), ""),
        (30100, Some(Input::Right), ""),
        (30400, None, "settings: brightness, right pressed once"),
        (30500, Some(Input::KnobCw), ""),
        (30600, Some(Input::Right), ""),
        (30900, None, "settings: blocks, 3x3 touching; the strip is the preview"),
        (31000, Some(Input::Right), ""),
        (31010, Some(Input::Right), ""),
        (31300, None, "settings: 2x2 touching, 32 agents"),
        (31400, Some(Input::Left), ""),
        (31410, Some(Input::Left), ""),
        (31420, Some(Input::Left), ""),
        (31500, Some(Input::KnobCw), ""),
        (31800, None, "settings: row 1 shows the model, previewed live"),
        (31900, Some(Input::KnobCw), ""),
        (32000, Some(Input::Left), ""),
        (32300, None, "settings: style 1 = tint"),
        (32400, Some(Input::KnobCw), ""),
        (32410, Some(Input::KnobCw), ""),
        (32420, Some(Input::KnobCw), ""),
        (32500, Some(Input::Right), ""),
        (32900, None, "settings: name = dir"),
        (33000, Some(Input::KnobCw), ""),
        (33300, None, "settings: how long the name lingers"),
        (33400, Some(Input::KnobCw), ""),
        (33500, Some(Input::KnobCw), ""),
        (33800, None, "settings: hosts, and how each is connected"),
        (33810, Some(Input::KnobCw), ""),
        (33850, None, "settings: device, wifi network"),
        (33860, Some(Input::Right), ""),
        (33890, None, "settings: device, its address"),
        (33900, Some(Input::KnobPush), ""),
        (34200, None, "rest: model tinted, context plain"),
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
    ui.set_power(Power { percent: Some(87), millivolts: Some(4160), on_usb: Some(true) }, 39_000);
    ui.set_power(Power { percent: Some(87), millivolts: Some(4050), on_usb: Some(false) }, 40_000);
    shot(&mut ui, &world, 41_000, "battery notice: cable pulled");
    ui.set_power(Power { percent: Some(9), millivolts: Some(3620), on_usb: Some(false) }, 45_000);
    shot(&mut ui, &world, 46_000, "battery notice: fell under 10 %");
    ui.set_power(Power { percent: Some(2), millivolts: Some(3540), on_usb: Some(false) }, 47_000);
    ui.tick(&world, 47_000);
    ui.tick(&world, 57_100);
    shot(&mut ui, &world, 62_300, "cell under 3.55 V for 10 s: counting down to power-off");
    ui.set_power(Power { percent: Some(3), millivolts: Some(3700), on_usb: Some(true) }, 63_000);
    shot(&mut ui, &world, 63_500, "cable back in: countdown gone, charging notice");
    let mut t = 70_000;
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
        let t = 80_000 + i as u64 * 100;
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

//! Pixbar UI: `(world, inputs, now_ms) -> 52x16 frame`. No OS access, so the same crate drives the
//! desktop simulator and the device binary.

pub mod font;
pub mod frame;
pub mod settings;
pub mod state;
pub mod ui;

pub use frame::{Frame, Rgb, H, W};
pub use state::{Agent, Effort, Limit, Model, Power, Status, World};
pub use settings::{Blocks, HostPick, NameOf, Row, Settings, Show, Style};
pub use ui::{render_notice, DeviceAction, HostLink, Info, Input, Intent, Ui};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::tokens_short;

    fn opus() -> Model {
        Model::new("opus")
    }

    fn fable() -> Model {
        Model::new("fable")
    }

    fn world() -> World {
        let agent = |space: &str, status, model: Model, effort| Agent {
            reported: true,
            has_effort: true,
            next_model: Some(if model == opus() { fable() } else { opus() }),
            space: space.into(),
            tab: "1".into(),
            status,
            model,
            effort,
            ctx_used: 104_000,
            ctx_window: 1_000_000,
            ultra_ok: true,
            dir: "web-repo".into(),
            title: String::new(),
            fresh: false,
            session: String::new(),
            cost_cents: 1_234,
            limit_5h: Some(Limit { used_pct: 12, resets_in_min: 200 }),
            limit_7d: None,
        };
        World {
            agents: vec![
                agent("web", Status::Working, opus(), Effort::XHigh),
                agent("api", Status::Blocked, fable(), Effort::High),
            ],
            focused: 0,
        }
    }

    #[test]
    fn every_word_the_ui_draws_has_all_its_glyphs() {
        for word in Effort::ALL.map(Effort::word).into_iter().chain(["OPUS", "FABLE", "SONNET", "HAIKU"]) {
            assert!(crate::font::BIG.covers(word), "BIG lacks a glyph for {word}");
            assert!(crate::font::SMALL.covers(word), "SMALL lacks a glyph for {word}");
        }
        assert!(crate::font::SMALL.covers("abcdefghijklmnopqrstuvwxyz0123456789-_./%·:?"), "addresses need the colon");
        assert!(crate::font::SMALL.covers("()+,'!"), "names carry these");
        for cents in [0, 42, 1_234, 123_456] {
            let cost = crate::state::dollars_short(cents);
            assert!(crate::font::SMALL.covers(&cost), "SMALL lacks a glyph in {cost}");
        }
        assert!(crate::font::BIG.covers("abcdefghijklmnopqrstuvwxyz0123456789?"), "settings draw numbers in BIG");
    }

    #[test]
    fn a_pending_change_does_not_reach_the_agent_that_slid_into_its_place() {
        let mut w = world();
        let mut third = w.agents[0].clone();
        third.space = "docs".into();
        third.effort = Effort::Low;
        w.agents.push(third);
        w.agents[1].status = Status::Working;
        w.focused = 1;
        let mut ui = Ui::new();
        assert_eq!(ui.input(&w, Input::Right, 1000), None, "the rail opens; the change waits out the settle time");
        // The first agent leaves before the change is sent: position 1 now holds a different session.
        w.agents.remove(0);
        w.focused = 0;
        assert_eq!(ui.tick(&w, 1100), None);
        assert_eq!(ui.tick(&w, 5000), None, "nothing is sent on anyone's behalf");
    }

    #[test]
    fn rendering_before_tick_survives_an_overlay_whose_agent_is_gone() {
        let mut w = world();
        w.agents[1].status = Status::Working;
        w.focused = 1;
        let mut ui = Ui::new();
        ui.input(&w, Input::Right, 1000);
        w.agents.remove(1);
        w.focused = 0;
        ui.render(&w, 1100, &mut Frame::new());
    }

    #[test]
    fn a_session_nobody_reported_on_shows_dashes_and_takes_no_presses() {
        let mut w = world();
        w.agents[0].reported = false;
        let mut ui = Ui::new();
        for press in [Input::Left, Input::Right, Input::Middle] {
            assert_eq!(ui.input(&w, press, 1000), None);
            assert_eq!(ui.tick(&w, 5000), None, "{press:?} must not reach a session we know nothing about");
        }
        let (mut unknown, mut known) = (Frame::new(), Frame::new());
        ui.render(&w, 9000, &mut unknown);
        w.agents[0].reported = true;
        ui.render(&w, 9000, &mut known);
        let differ = (0..H as i32).any(|y| (0..W as i32).any(|x| unknown.get(x, y) != known.get(x, y)));
        assert!(differ, "it must not look like a real Opus / xhigh / 104K session");
    }

    #[test]
    fn the_middle_button_goes_where_the_host_says_or_nowhere() {
        let mut w = world();
        w.agents[0].next_model = None;
        let mut ui = Ui::new();
        assert_eq!(ui.input(&w, Input::Middle, 0), None);
        assert_eq!(ui.input(&w, Input::Middle, 500), None, "one model in use: nothing to switch to");
        w.agents[0].next_model = Some(Model::new("Sonnet"));
        assert_eq!(ui.input(&w, Input::Middle, 9000), None, "armed");
        assert_eq!(ui.input(&w, Input::Middle, 9500), Some(Intent::SetModel { agent: 0, model: Model::new("sonnet") }));
    }

    #[test]
    fn model_names_are_the_hosts_and_fit_beside_the_effort() {
        assert_eq!(Model::new("Sonnet 5").word(), "SONNET");
        assert_eq!(Model::new("claude-3-7").word(), "CLAUDE");
        assert_eq!(serde_json::to_string(&Model::new("Opus")).unwrap(), "\"opus\"");
        assert_eq!(serde_json::from_str::<Model>("\"fable\"").unwrap(), fable());
        // `SONNET XHIGH` is five pixels too wide for the row; the rest screen must stay inside it all the same.
        let mut w = world();
        (w.agents[0].model, w.agents[0].effort) = (Model::new("sonnet"), Effort::XHigh);
        let mut f = Frame::new();
        Ui::new().render(&w, 0, &mut f);
        for y in 0..H as i32 {
            assert_eq!(f.get(W as i32 - 1, y), Rgb::OFF, "lit pixel in the last column, row {y}");
        }
    }

    #[test]
    fn token_formatting() {
        assert_eq!(tokens_short(104_321), "104K");
        assert_eq!(tokens_short(1_000_000), "1M");
        assert_eq!(tokens_short(1_240_000), "1.2M");
    }

    #[test]
    fn rapid_presses_become_one_intent_after_settling() {
        let (w, mut ui) = (world(), Ui::new());
        assert_eq!(ui.input(&w, Input::Left, 0), None);
        assert_eq!(ui.input(&w, Input::Left, 100), None);
        assert_eq!(ui.tick(&w, 500), None);
        assert_eq!(ui.tick(&w, 800), Some(Intent::SetEffort { agent: 0, effort: Effort::Med }));
        assert_eq!(ui.tick(&w, 900), None);
    }

    #[test]
    fn ultracode_is_the_stop_past_max_where_the_session_offers_it() {
        let (mut w, mut ui) = (world(), Ui::new());
        ui.input(&w, Input::Right, 0); // xhigh -> max
        ui.input(&w, Input::Right, 100); // max -> ultra
        ui.input(&w, Input::Right, 200); // end stop
        assert_eq!(ui.tick(&w, 1000), Some(Intent::SetEffort { agent: 0, effort: Effort::Ultra }));

        let mut f = Frame::new();
        ui.render(&w, 2500, &mut f);
        assert!(crate::ui::palette::RIPPLE.contains(&f.get(12, 1)), "ripple has flooded the far corner");

        w.agents[0].ultra_ok = false;
        let mut ui = Ui::new();
        ui.input(&w, Input::Right, 0);
        ui.input(&w, Input::Right, 100);
        assert_eq!(ui.tick(&w, 1000), Some(Intent::SetEffort { agent: 0, effort: Effort::Max }));
    }

    #[test]
    fn there_and_back_sends_nothing() {
        let (w, mut ui) = (world(), Ui::new());
        ui.input(&w, Input::Left, 0);
        ui.input(&w, Input::Right, 100);
        assert_eq!(ui.tick(&w, 900), None);
    }

    #[test]
    fn a_knob_click_focuses_at_once_like_tab() {
        let (w, mut ui) = (world(), Ui::new());
        assert_eq!(ui.input(&w, Input::KnobCw, 0), Some(Intent::Focus(1)));
        assert_eq!(ui.tick(&w, 500), None, "already sent");
        assert_eq!(ui.input(&w, Input::KnobCcw, 1000), Some(Intent::Focus(0)));
    }

    #[test]
    fn a_fast_spin_focuses_only_where_it_stops() {
        let (mut w, mut ui) = (world(), Ui::new());
        w.agents.push(w.agents[0].clone());
        w.agents.push(w.agents[0].clone());
        assert_eq!(ui.input(&w, Input::KnobCw, 0), Some(Intent::Focus(1)));
        assert_eq!(ui.input(&w, Input::KnobCw, 40), None);
        assert_eq!(ui.input(&w, Input::KnobCw, 80), None);
        assert_eq!(ui.tick(&w, 120), None);
        assert_eq!(ui.tick(&w, 240), Some(Intent::Focus(3)));
        assert_eq!(ui.tick(&w, 300), None);
    }

    #[test]
    fn knob_push_jumps_to_the_agent_that_needs_you() {
        let (w, mut ui) = (world(), Ui::new());
        assert_eq!(ui.input(&w, Input::KnobPush, 0), Some(Intent::Focus(1)), "agent 1 is blocked");
    }

    #[test]
    fn the_name_lingers_after_a_turn_until_the_knob_is_pushed() {
        let (mut w, mut ui) = (world(), Ui::new());
        let mut f = Frame::new();
        let picker_up = |ui: &Ui, w: &World, t: u64, f: &mut Frame| {
            ui.render(w, t, f);
            // The picker puts MODEL EFFORT on line 2, the resting screen puts it on line 1.
            (11..52).all(|x| f.get(x, 3) == Rgb::OFF || f.get(x, 3) == crate::ui::palette::WHITE)
        };
        assert_eq!(ui.input(&w, Input::KnobCw, 0), Some(Intent::Focus(1)));
        w.focused = 1;
        ui.tick(&w, 9000);
        assert!(picker_up(&ui, &w, 9000, &mut f), "still showing the name 9 s later");
        ui.tick(&w, 10_100);
        assert!(!picker_up(&ui, &w, 10_100, &mut f), "gone after the linger setting (10 s)");

        assert_eq!(ui.input(&w, Input::KnobCcw, 20_000), Some(Intent::Focus(0)));
        w.focused = 0;
        assert_eq!(ui.input(&w, Input::KnobPush, 21_000), None, "push only dismisses; agent 1 is blocked but not jumped to");
        assert!(!picker_up(&ui, &w, 21_000, &mut f));
        assert_eq!(ui.input(&w, Input::KnobPush, 22_000), Some(Intent::Focus(1)), "at rest the push jumps again");
    }

    #[test]
    fn focus_moved_from_the_keyboard_ends_a_lingering_picker() {
        let (mut w, mut ui) = (world(), Ui::new());
        w.agents.push(w.agents[0].clone());
        ui.input(&w, Input::KnobCw, 0);
        w.focused = 1;
        ui.tick(&w, 100);
        w.focused = 2; // Tab in herdr, 5 s later
        ui.tick(&w, 5000);
        assert_eq!(ui.input(&w, Input::KnobPush, 5100), Some(Intent::Focus(1)), "no picker left to dismiss: push jumps");
    }

    #[test]
    fn a_model_switch_with_context_to_lose_needs_a_second_press() {
        let (mut w, mut ui) = (world(), Ui::new());
        assert_eq!(ui.input(&w, Input::Middle, 0), None, "armed only");
        assert_eq!(ui.tick(&w, 3000), None);
        assert_eq!(ui.input(&w, Input::Middle, 3100), Some(Intent::SetModel { agent: 0, model: fable() }));
        (w.agents[0].model, w.agents[0].next_model) = (fable(), Some(opus())); // host confirms

        assert_eq!(ui.input(&w, Input::Middle, 10_000), None);
        assert_eq!(ui.tick(&w, 14_100), None, "left alone, it lapses");
        assert_eq!(ui.input(&w, Input::Middle, 14_200), None, "and the next press arms again rather than sending");
        assert_eq!(ui.input(&w, Input::Left, 14_300), None, "another button drops it");
        assert_eq!(ui.tick(&w, 15_100), Some(Intent::SetEffort { agent: 0, effort: Effort::High }));
    }

    #[test]
    fn a_new_or_freshly_compacted_session_switches_model_on_one_press() {
        let (mut w, mut ui) = (world(), Ui::new());
        w.agents[0].ctx_used = 17_577;
        assert_eq!(ui.input(&w, Input::Middle, 0), None, "little context is still context: armed, not sent");
        let mut ui = Ui::new();
        w.agents[0].fresh = true;
        assert_eq!(ui.input(&w, Input::Middle, 0), Some(Intent::SetModel { agent: 0, model: fable() }));
        assert_eq!(ui.input(&w, Input::Middle, 500), Some(Intent::SetModel { agent: 0, model: opus() }), "and back");
    }

    #[test]
    fn focus_is_the_brightest_block_and_the_rest_share_one_level() {
        let (mut w, ui) = (world(), Ui::new());
        w.agents[1].status = Status::Working;
        w.agents.push(Agent { status: Status::Idle, ..w.agents[0].clone() });
        let mut f = Frame::new();
        ui.render(&w, 5000, &mut f);
        let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
        let (focused, working, idle) = (f.get(2, 1), f.get(2, 5), f.get(2, 9));
        assert_eq!(focused, crate::ui::palette::WORKING);
        assert!(lum(focused) > 2 * lum(working));
        assert_eq!((working, idle), (crate::ui::palette::WORKING.scale(0.25), crate::ui::palette::IDLE.scale(0.25)), "told apart by colour alone");
        assert!((0..16).all(|y| f.get(0, y) == Rgb::OFF && f.get(8, y) == Rgb::OFF), "no cursor bar");
    }

    #[test]
    fn a_done_agent_breathes_under_the_focused_level_and_the_dimmest_block_stays_lit() {
        let (mut w, mut ui) = (world(), Ui::new());
        w.agents[1].status = Status::Done;
        w.agents.push(Agent { status: Status::Unknown, ..w.agents[0].clone() });
        let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
        let at = |ui: &Ui, w: &World, now: u64, y: i32| {
            let mut f = Frame::new();
            ui.render(w, now, &mut f);
            f.get(2, y)
        };
        let (peak, trough) = (lum(at(&ui, &w, 1800, 5)), lum(at(&ui, &w, 2700, 5)));
        assert!(peak > 2 * trough && trough > 0, "breathing: {peak} / {trough}");
        assert!(lum(at(&ui, &w, 1800, 1)) > peak, "and under the focused block at its peak");
        w.focused = 1;
        assert!(lum(at(&ui, &w, 2700, 5)) >= peak, "with the focus its breath begins where the other one ends");

        // The panel multiplies by BRIGHT before anything else: a channel under 100 / BRIGHTNESS_MIN is then off.
        ui.settings.blocks = Blocks { size: 3, gap: false };
        w.focused = 0;
        let mut f = Frame::new();
        ui.render(&w, 0, &mut f);
        let lit: Vec<Rgb> = (0..9).flat_map(|x| (0..16).map(move |y| (x, y))).map(|(x, y)| f.get(x, y)).filter(|c| *c != Rgb::OFF).collect();
        let floor = 100 / settings::BRIGHTNESS_MIN;
        assert!(lit.iter().all(|c| c.0.max(c.1).max(c.2) >= floor), "every block survives the lowest BRIGHT setting");
    }

    #[test]
    fn block_size_sets_how_many_agents_fit_and_the_strip_pages_with_the_focus() {
        let (mut w, mut ui) = (world(), Ui::new());
        assert_eq!(Blocks::CHOICES.map(Blocks::capacity), [6, 8, 8, 15, 15, 32], "4x4, 3x3, 2x2: spaced, then touching");
        while w.agents.len() < 10 {
            w.agents.push(w.agents[0].clone());
        }
        let lit = |ui: &Ui, w: &World| {
            let mut f = Frame::new();
            ui.render(w, 5000, &mut f);
            (0..16).flat_map(|y| (0..9).map(move |x| (x, y))).filter(|&(x, y)| f.get(x, y) != Rgb::OFF).count()
        };
        assert_eq!(lit(&ui, &w), 8 * 9);
        w.focused = 9;
        assert_eq!(lit(&ui, &w), 2 * 9, "second page: agents 8 and 9");
        ui.settings.blocks = Blocks { size: 2, gap: true };
        assert_eq!(lit(&ui, &w), 10 * 4);
        ui.settings.blocks = Blocks { size: 4, gap: true };
        assert_eq!(lit(&ui, &w), 4 * 16, "agents 6..=9");

        // Touching blocks: neighbours with the same status still tell apart, and focus is still the brightest.
        ui.settings.blocks = Blocks { size: 3, gap: false };
        w.focused = 0;
        let mut f = Frame::new();
        ui.render(&w, 5000, &mut f);
        let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
        let (focused, second, third) = (f.get(1, 1), f.get(1, 7), f.get(1, 10));
        assert_eq!(w.agents[2].status, w.agents[3].status);
        assert!(lum(focused) > lum(second) && second != third && f.get(1, 9) == third, "rows 6..=8 and 9..=11 touch");
    }

    #[test]
    fn long_press_opens_settings_knob_turns_pages_and_buttons_change_values() {
        let (w, mut ui) = (world(), Ui::new());
        assert_eq!(ui.input(&w, Input::KnobLong, 0), None);
        assert_eq!(ui.input(&w, Input::Right, 100), None, "no effort change while in settings");
        assert_eq!(ui.input(&w, Input::Right, 200), None);
        assert_eq!(ui.settings.brightness, 90);
        assert_eq!(ui.take_settings_change(), None, "not handed over until the screen closes");
        assert_eq!(ui.input(&w, Input::KnobCw, 300), None, "next page (block size), not the next agent");
        ui.input(&w, Input::Right, 400);
        assert_eq!(ui.settings.blocks, Blocks { size: 3, gap: false }, "after 3x3 spaced comes 3x3 touching");
        ui.input(&w, Input::KnobCcw, 420);
        ui.input(&w, Input::KnobCcw, 440); // wraps to the last page: device facts, read-only
        ui.input(&w, Input::Right, 460);
        let expected = Settings { brightness: 90, blocks: Blocks { size: 3, gap: false }, ..Settings::default() };
        assert_eq!(ui.settings, expected);
        ui.input(&w, Input::KnobPush, 500);
        assert_eq!(ui.take_settings_change(), Some(expected));
        assert_eq!(ui.take_settings_change(), None);
        assert_eq!(ui.input(&w, Input::KnobCw, 600), Some(Intent::Focus(1)), "knob is back to switching agents");
    }

    #[test]
    fn the_hosts_page_picks_whose_agents_the_panel_shows() {
        let (w, mut ui) = (world(), Ui::new());
        ui.info.hosts =
            vec![HostLink { name: "DESKTOP".into(), wired: true }, HostLink { name: "LAPTOP".into(), wired: false }];
        assert!(ui.settings.host.is_all(), "every host's agents, until one is picked");

        ui.input(&w, Input::KnobLong, 0);
        for t in 1..=4 {
            ui.input(&w, Input::KnobCcw, t * 10); // backwards to HOSTS, past the two actions and DEVICE
        }
        ui.input(&w, Input::Right, 100);
        assert_eq!(ui.settings.host.as_str(), "DESKTOP");
        ui.input(&w, Input::Right, 200);
        assert_eq!(ui.settings.host.as_str(), "LAPTOP");
        ui.input(&w, Input::Right, 300);
        assert!(ui.settings.host.is_all(), "past the last host it comes round to ALL");
        ui.input(&w, Input::Left, 400);
        assert_eq!(ui.settings.host.as_str(), "LAPTOP", "and the other way round");

        // A machine that is off keeps its place while it is the pick, so the panel stays its own.
        ui.info.hosts.pop();
        assert_eq!(ui.settings.host.as_str(), "LAPTOP", "gone from the link, still the pick");
        ui.input(&w, Input::Right, 500);
        assert!(ui.settings.host.is_all(), "stepping off it lands on ALL");
        ui.input(&w, Input::Left, 600);
        assert_eq!(ui.settings.host.as_str(), "DESKTOP", "only what is connected is left in the ring");

        ui.input(&w, Input::KnobPush, 700);
        let handed = ui.take_settings_change();
        assert_eq!(handed.map(|s| s.host), Some(HostPick::new("DESKTOP")), "handed over when the screen closes");
    }

    #[test]
    fn settings_survive_the_config_file_and_bad_values_fall_back() {
        let s = Settings {
            brightness: 40,
            blocks: Blocks { size: 2, gap: false },
            rows: [Row { show: Show::Name, style: Style::Card }, Row { show: Show::Model, style: Style::Tint }],
            name: NameOf::Dir,
            linger_s: 20,
            refresh_ms: 250,
            host: HostPick::new("DESKTOP"),
        };
        assert_eq!(Settings::from_config(&s.to_config()), s);
        // No host picked is the default, and writes an empty value that reads back the same way.
        let all = Settings { host: HostPick::default(), ..s };
        assert!(all.host.is_all() && Settings::from_config(&all.to_config()) == all);
        assert_eq!(Settings::from_config("brightness=250\nblocks=9\nrow1=clock\nrow2=name_pct\nlinger_s=7\nlayout=name\n"), Settings {
            brightness: 100,
            ..Settings::default()
        });
    }

    #[test]
    fn rows_are_configurable_and_a_card_is_black_text_on_a_lit_field() {
        let (w, mut ui) = (world(), Ui::new());
        let mut f = Frame::new();
        let lum = |c: Rgb| c.0 as u32 + c.1 as u32 + c.2 as u32;
        let (opus, white, full) = (crate::ui::palette::OPUS, crate::ui::palette::WHITE, Rgb(255, 255, 255));
        ui.settings.rows = [Row { show: Show::Context, style: Style::Card }, Row { show: Show::Model, style: Style::Card }];
        ui.render(&w, 5000, &mut f);
        let art = f.to_ascii();
        // Top card: rows 0..=6, full white, from the main area's edge (where plain text starts too),
        // 2 px of padding either side of the 29 px of "104K 10%", corners cut.
        assert_eq!([f.get(10, 3), f.get(11, 0), f.get(43, 0), f.get(44, 3)], [Rgb::OFF; 4], "\n{art}");
        assert_eq!([f.get(11, 3), f.get(12, 0), f.get(43, 3)], [full; 3], "\n{art}");
        assert!((11..=12).all(|x| (1..=5).all(|y| f.get(x, y) == full)) && f.get(14, 1) == Rgb::OFF, "padding, then a black 1:\n{art}");
        assert!((10..=51).all(|x| f.get(x, 7) == Rgb::OFF && f.get(x, 8) == Rgb::OFF), "gap between the cards:\n{art}");
        // Bottom row: OPUS is a chip in its hue (1 px of padding), XHIGH stays lit text right of it.
        assert_eq!([f.get(11, 12), f.get(27, 12), f.get(28, 12), f.get(27, 15)], [opus, opus, Rgb::OFF, Rgb::OFF], "\n{art}");
        assert_eq!(f.get(29, 10), white, "X of XHIGH:\n{art}");

        // Tinted: the same chip, dark and still orange rather than grey, under lit text.
        ui.settings.rows[1].style = Style::Tint;
        ui.render(&w, 5000, &mut f);
        let tint = f.get(11, 12);
        assert!(tint.2 == 0 && tint.0 > 2 * tint.1 && 3 * lum(tint) < lum(opus), "{tint:?}\n{}", f.to_ascii());
        assert!((12..27).any(|x| f.get(x, 10) == opus), "OPUS lit over it:\n{}", f.to_ascii());

        // The longest pair still fits beside a chip, using the panel's last column.
        let mut long = world();
        (long.agents[0].model, long.agents[0].effort) = (fable(), Effort::XHigh);
        ui.render(&long, 5000, &mut f);
        assert!((10..=14).any(|y| f.get(51, y) != Rgb::OFF) && f.get(32, 12) == Rgb::OFF, "\n{}", f.to_ascii());

        // Dim: the plain row, same place, under half as bright.
        ui.settings.rows[1].style = Style::Plain;
        ui.render(&w, 5000, &mut f);
        let plain = f.get(11, 10);
        ui.settings.rows[1].style = Style::Dim;
        ui.render(&w, 5000, &mut f);
        assert!(f.get(11, 10) != Rgb::OFF && 2 * lum(f.get(11, 10)) < lum(plain), "{:?} vs {plain:?}", f.get(11, 10));

        ui.settings.rows = [Row { show: Show::Name, style: Style::Plain }, Row { show: Show::Context, style: Style::Plain }];
        ui.settings.name = NameOf::Dir;
        ui.render(&w, 5000, &mut f);
        let mut expected = Frame::new();
        crate::font::SMALL.draw(&mut expected, "WEB-REPO", 11, 2, crate::ui::palette::WHITE);
        assert!((0..7).all(|y| (11..52).all(|x| f.get(x, y) == expected.get(x, y))), "name row shows the dir:\n{}", f.to_ascii());
    }

    #[test]
    fn the_context_percentage_rounds_to_the_nearest() {
        let mut a = world().agents[0].clone();
        for (used, pct) in [(37_522, 4), (34_999, 3), (35_000, 4), (234_737, 23), (995_000, 100), (1_200_000, 100), (0, 0)] {
            a.ctx_used = used;
            assert_eq!(a.ctx_pct(), pct, "{used}");
        }
        a.ctx_window = 0;
        assert_eq!(a.ctx_pct(), 0);
    }

    #[test]
    fn rows_show_the_usage_windows_the_cost_and_the_session_name() {
        use crate::ui::palette::{AMBER, DIM, WHITE};
        let (mut w, mut ui, mut f) = (world(), Ui::new(), Frame::new());
        let top = |f: &Frame, expected: &Frame| (0..7).all(|y| (11..52).all(|x| f.get(x, y) == expected.get(x, y)));
        ui.settings.rows = [Row { show: Show::Limit5h, style: Style::Plain }, Row { show: Show::Limit7d, style: Style::Plain }];
        w.agents[0].limit_5h = Some(Limit { used_pct: 72, resets_in_min: 200 });
        ui.render(&w, 5000, &mut f);
        // Used in the colour of how little is left, the time until it starts over stepping back; a window nobody
        // reported (the 7-day one here) says so.
        let mut expected = Frame::new();
        let x = crate::font::SMALL.draw(&mut expected, "5H 72%", 11, 2, AMBER);
        crate::font::SMALL.draw(&mut expected, " 3H", x, 2, DIM);
        assert!(top(&f, &expected), "\n{}", f.to_ascii());
        let mut expected = Frame::new();
        crate::font::SMALL.draw(&mut expected, "7D --", 11, 9, WHITE);
        assert!((8..16).all(|y| (11..52).all(|x| f.get(x, y) == expected.get(x, y))), "\n{}", f.to_ascii());
        assert_eq!(
            [45, 119, 48 * 60, 6 * 24 * 60 + 5].map(|m| Limit { used_pct: 0, resets_in_min: m }.resets_in()),
            ["45M", "1H", "2D", "6D"].map(String::from)
        );

        ui.settings.rows[0].show = Show::Cost;
        ui.render(&w, 5000, &mut f);
        let mut expected = Frame::new();
        crate::font::SMALL.draw(&mut expected, "$12.34", 11, 2, WHITE);
        assert!(top(&f, &expected), "\n{}", f.to_ascii());
        assert_eq!([42, 99_999, 123_456].map(crate::state::dollars_short), ["$0.42", "$999.99", "$1234"].map(String::from));

        // A session that was never named is called by its space.
        ui.settings.rows[0].show = Show::Name;
        ui.settings.name = NameOf::Session;
        for (session, shown) in [("", "WEB"), ("parser", "PARSER")] {
            w.agents[0].session = session.into();
            ui.render(&w, 5000, &mut f);
            let mut expected = Frame::new();
            crate::font::SMALL.draw(&mut expected, shown, 11, 2, WHITE);
            assert!(top(&f, &expected), "{shown}\n{}", f.to_ascii());
        }

        // Saved and read back like the older choices.
        ui.settings.rows[1].show = Show::Limit7d;
        assert_eq!(Settings::from_config(&ui.settings.to_config()), ui.settings);
    }

    #[test]
    fn a_flat_battery_counts_down_and_powers_off_unless_the_cable_goes_in() {
        let (w, mut ui) = (world(), Ui::new());
        let cell = |mv| Power { percent: Some(3), millivolts: Some(mv), on_usb: Some(false) };
        ui.set_power(cell(3540), 0);
        ui.tick(&w, 0);
        ui.tick(&w, 9_000);
        assert!(ui.take_device_action().is_none());
        ui.set_power(cell(3700), 9_500); // a sag under load that recovered: the 10 s start over
        ui.tick(&w, 9_500);
        ui.set_power(cell(3540), 10_000);
        for t in [10_000, 20_000, 30_000, 49_000] {
            ui.tick(&w, t);
            assert!(ui.take_device_action().is_none(), "{t}");
        }
        ui.tick(&w, 50_100);
        assert_eq!(ui.take_device_action(), Some(DeviceAction::PowerOff), "10 s under 3.55 V, then 30 s of countdown");
        assert_eq!(ui.take_device_action(), None, "asked for once");

        let mut ui = Ui::new();
        ui.set_power(cell(3540), 0);
        ui.tick(&w, 0);
        ui.tick(&w, 15_000);
        assert!(ui.wants_screen(15_000), "countdown is up, host or no host");
        ui.set_power(Power { on_usb: Some(true), ..cell(3540) }, 16_000);
        ui.tick(&w, 60_000);
        assert_eq!(ui.take_device_action(), None, "on USB power nothing is ever switched off");

        let mut ui = Ui::new();
        ui.set_power(Power::default(), 0); // MCU silent: no reading is not a low reading
        ui.tick(&w, 100_000);
        assert_eq!(ui.take_device_action(), None);
    }

    #[test]
    fn pulling_the_cable_shows_the_battery_but_not_over_what_you_are_doing() {
        let (w, mut ui) = (world(), Ui::new());
        let reading = |usb| Power { percent: Some(87), millivolts: Some(4100), on_usb: Some(usb) };
        ui.set_power(reading(true), 0);
        assert!(!ui.wants_screen(100), "the first reading is not news");
        ui.set_power(reading(false), 1000);
        assert!(ui.wants_screen(1100));
        let mut f = Frame::new();
        ui.render(&w, 2000, &mut f);
        assert_eq!(f.get(30, 12), crate::ui::palette::DONE, "gauge, green at 87 %:\n{}", f.to_ascii());
        ui.input(&w, Input::Right, 2100); // effort overlay wins
        ui.render(&w, 2200, &mut f);
        assert_ne!(f.get(12, 12), crate::ui::palette::DONE, "\n{}", f.to_ascii());
        assert!(!ui.wants_screen(5000), "gone after a few seconds");
    }

    #[test]
    fn turning_off_from_the_settings_takes_two_presses() {
        let (w, mut ui) = (world(), Ui::new());
        ui.input(&w, Input::KnobLong, 0);
        ui.input(&w, Input::KnobCcw, 100); // last page: back to stock
        ui.input(&w, Input::KnobCcw, 200); // before it: turn off
        ui.input(&w, Input::Right, 300);
        assert_eq!(ui.take_device_action(), None, "armed only");
        ui.tick(&w, 5000);
        ui.input(&w, Input::Right, 5100);
        assert_eq!(ui.take_device_action(), None, "the first press lapsed after 4 s: armed again, nothing done");
        ui.input(&w, Input::Left, 5200);
        ui.input(&w, Input::Right, 5300);
        assert_eq!(ui.take_device_action(), None, "left disarms");
        ui.input(&w, Input::Right, 5400);
        assert_eq!(ui.take_device_action(), Some(DeviceAction::PowerOff));
        ui.input(&w, Input::KnobLong, 6000);
        ui.input(&w, Input::KnobCcw, 6100);
        ui.input(&w, Input::Right, 6200);
        ui.input(&w, Input::Right, 6300);
        assert_eq!(ui.take_device_action(), Some(DeviceAction::Stock));
    }

    #[test]
    fn a_held_button_repeats_the_step_but_confirms_nothing() {
        // Effort: xhigh, held right runs to the end of the rail and stops there quietly.
        let (w, mut ui) = (world(), Ui::new());
        ui.input(&w, Input::Right, 0); // max
        ui.input(&w, Input::RightHeld, 400); // ultra
        for t in [550, 700, 850, 1000, 1150] {
            assert_eq!(ui.input(&w, Input::RightHeld, t), None);
        }
        // Still held against the end stop: sent 0.7 s after the last step that moved, not put off for ever.
        assert_eq!(ui.tick(&w, 1150), Some(Intent::SetEffort { agent: 0, effort: Effort::Ultra }));

        // Settings: a value runs along while held.
        let mut ui = Ui::new();
        ui.input(&w, Input::KnobLong, 0);
        let before = ui.settings.brightness;
        ui.input(&w, Input::Left, 100);
        ui.input(&w, Input::LeftHeld, 500);
        ui.input(&w, Input::LeftHeld, 650);
        assert_eq!(ui.settings.brightness, before - 3 * crate::settings::BRIGHTNESS_STEP);

        // Turning off: the second press has to be a press.
        ui.input(&w, Input::KnobCcw, 1000);
        ui.input(&w, Input::KnobCcw, 1100);
        ui.input(&w, Input::Right, 1200);
        for t in [1600, 1750, 1900] {
            ui.input(&w, Input::RightHeld, t);
        }
        assert_eq!(ui.take_device_action(), None, "resting on the button is not a second press");
        ui.input(&w, Input::Right, 2500);
        assert_eq!(ui.take_device_action(), Some(DeviceAction::PowerOff), "and it did not disarm the first either");
    }

    #[test]
    fn blocked_agent_refuses_changes() {
        let (mut w, mut ui) = (world(), Ui::new());
        w.focused = 1;
        assert_eq!(ui.input(&w, Input::Right, 0), None);
        assert_eq!(ui.input(&w, Input::Middle, 10), None);
        assert_eq!(ui.input(&w, Input::Middle, 20), None);
        assert_eq!(ui.tick(&w, 1000), None);
    }

    #[test]
    fn rest_screen_stays_inside_the_main_area() {
        let (w, ui) = (world(), Ui::new());
        let mut f = Frame::new();
        ui.render(&w, 5000, &mut f);
        let art = f.to_ascii();
        for line in art.lines() {
            assert_eq!(&line[9..11], "..", "gap between strip and main area:\n{art}");
            assert_eq!(&line[51..], ".", "right margin:\n{art}");
        }
        assert!(art.lines().nth(2).unwrap()[11..].contains('#'), "line 1 drawn:\n{art}");
    }
}

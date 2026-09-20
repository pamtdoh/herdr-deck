# Pixbar × herdr — research findings

> **A lab notebook, not documentation.** Written on 2026-09-19 for its author, before and while the first version was
> built: one person's TC002, machine, herdr 0.8.2 and Claude Code 2.1.277. It records what was measured and why
> things were decided as they were, and later sections correct earlier ones. Where it and the code disagree, the
> code and the README are right. Known to be overtaken (2026-09-20): §7's control scheme and §12's build status
> describe plans and an early state, not the program; the USB statements in §1 and §4 ("needs replug", "no way to
> do it at boot") were overturned by measurement: a bridge that is listening catches the port in the ~2.6 s it
> answers after a power-up and keeps it, with nothing flashed (README, "On another machine or network"); models
> are no longer limited to Opus and Fable.

Date: 2026-09-19. Produced by a 10-agent research run (7 researchers + 3 fact-checkers) plus
hands-on checks against the actual device and the local herdr 0.8.2 / Claude Code 2.1.277 install.
Confidence tags: **[verified]** = checked on this machine/unit, **[sourced]** = from a cited primary
source, **[untested]** = plausible, needs a hands-on check.

## 0. The short version

- The TC002 is **not an ESP32**. It is a small Linux computer (SigmaStar SSD21x, 2× Cortex-A7, 64 MB RAM)
  with a **52×16** RGB panel, open **root adb** (WiFi :5555 and USB), no firmware signing. AWTRIX 3 /
  ESPHome / WLED / esptool do not apply. "Custom firmware" = our own static ARM Linux binary.
- That binary can take over the panel **without flashing anything** (`setprop ctl.stop zkswe`, run from
  `/tmp`); a reboot always returns to stock. Zero brick risk during the whole build.
- Inputs are plain Linux evdev: knob rotate, **knob push** (a 5th input), and three top buttons
  (left / middle / right = your down / press / up).
- herdr has everything needed on the host: socket API, event stream, `agent.focus`, `pane.send_input`,
  and `pane.report_metadata` tokens (which also render in herdr's own sidebar).
- Claude Code exposes model / effort / context in the **statusLine JSON**; changing them on a live TUI
  session is only possible by typing `/model` / `/effort` into the pane (no control API exists).
- Transport: one framed TCP protocol; **WiFi primary**, **USB via `adb forward`** as the same-code
  fallback. Bluetooth is a poor fit on both ends.

## 1. Hardware [verified on the user's unit unless noted]

| Item | Fact |
|---|---|
| SoC / OS | `Zkswe_SSD21X_SPINOR`, 2× Cortex-A7, Linux 4.9.84, ZKSWE FlyThings v2.1 ("zkOS"), Android-style init + `adbd` as root |
| Memory / flash | 64 MB DRAM (~35 MB to Linux, ~16 MB free with stock app), 32 MB SPI NOR, 8 MTD partitions; `/tmp` = 16 MB tmpfs; `/data` = 8 MB jffs2 (only persistent writable fs); `/res` squashfs 100 % full |
| Panel | **52×16 = 832 RGB LEDs**, driven by a separate pixel MCU (fw V1.0.17) over `/dev/spidev0.0` (mode 0, 10 MHz) + `GPIO_35` latch. Frame = 3072 B (16 rows × (52×3 + 36 pad)). MCU double-buffers (1 frame lag; write twice to leave a still). 60 fps measured by others at ~5 % CPU; bus time ≈ 2.5 ms, so the real ceiling is 50–200 fps depending on how the vendor's "15 ms" rule is read [sourced] |
| Inputs | `/dev/input/event67` "soc:gpio_keys_1" (polled 20 ms): **103 = knob push, 108 = left, 105 = middle, 106 = right**. `/dev/input/event68` "knob_key": EV_ABS/ABS_X state codes, one detent = pair **8→1 (CW)**, **13→11 (CCW)**. Discover nodes by capability, don't hardcode 67/68. Stock app does **not** grab the devices, so a second reader (`adb shell getevent`) works on stock firmware [open/enumerate verified; event delivery untested — nobody pressed a button] |
| Radio | AIC8800DC: WiFi **2.4 GHz only**, BLE 5.2. No BlueZ userland on device. No DNS resolver, no mDNS |
| USB-C | Native USB 2.0 HS **ADB gadget** `18d1:d002 "Zkswe"` (Ulanzi's docs wrongly say mass-storage). Works even with WiFi down. **Gotcha:** a kernel kthread flips the port to host mode ≈3.5 s into every boot, stranding a cable that was already plugged in → *replug after boot*. This is exactly what the kernel log showed at 10:06 (device seen for 3 s, then gone). ~~Not fixable from userspace~~ **Corrected 2026-09-19 (measured):** the port is dual-role under `zkswe,sstar-otg`. On stock firmware it is still `usb_host` minutes after boot, and neither a replug simulated from the PC nor time brings it back. *Reading* `/sys/bus/platform/devices/soc:usbotg/usb_device` switches it to device mode and the PC enumerates it at once (reading `usb_host` / `usb_null` switches the other way; writing a role name to `otg_role` does nothing). `pixbar-device` does this at startup. Nothing in init runs from `/data`, so without reflashing `/res` there is no way to do it at boot |
| Other | 3600 mAh battery (~2 h at full brightness; level only via MCU UART `/dev/ttyS1`), real PCM speaker (WAV), mic via MCU, no RTC, no light/temp sensors, magnetic dock = power only [sourced] |
| Stock services | HTTP API :80 (no auth), adb :5555 (root, no auth), UDP beacon → :55555 every ~1 s (`Ulanzi TC002 <mac-tail>:<mac>:<serial>:<flag>`). Unit: app 1.0.5 / MCU V1.0.17, IP by DHCP |
| Locks | None. `update.img` checked by magic + device code + CRC32 + MD5 only; no signature, no secure boot [sourced] |
| Recovery | Hold reset (beside USB-C) at power-up → wipes `/data`, reflashes `update.img` **if one exists on the UDISK partition** (on this unit it does: 2.78 MB). Persistent custom apps must set `sys.zkapp.state=running` within 15 s of boot |

Level curve [sourced, from the community runtime — verify on device]: the stack maps channel value `0→0`,
`v→50+(v−1)·205/254`, i.e. any lit channel is ≥ ~20 % drive. There is no "dim" — design ghosts as
outlines/stubs, prefer geometric transitions over fades, keep text near-white, put hue in rails/underlines.

## 2. Firmware options and prior art

| # | Route | Verdict |
|---|---|---|
| A | **Own native binary** (Rust `armv7-unknown-linux-musleabihf` or Zig, static). Opens evdev + spidev + gpio35, renders locally, talks TCP to the host. Dev loop: `adb push` → `/tmp`, `setprop ctl.stop zkswe`, run; `setprop ctl.start zkswe` restores stock in ~3 s (`ctl.restart` is ignored) | **Recommended.** Full control of all inputs, fonts, 60 fps, no flash writes. In stop-zkswe mode the MCU handshake is inherited from the stock app |
| B | A + persistence by flashing the `res` partition (mtd3) | Optional, later. No A/B slot; dump mtd3 first; needs MCU version handshake (`0xFF 0x55 0x11…` on ttyS1) and the 15 s `sys.zkapp.state` rule. Alternative that avoids flashing forever: the host bridge re-pushes and starts the binary whenever it sees the stock beacon |
| C | Stock firmware + HTTP/MQTT custom app (`POST /api/custom`, `switchDiyApp`) | Display-only prototype: ~8 full frames/s, ASCII text, font height 5 or 10, 6 images + 32 draw cmds. API never reports inputs (adb `getevent` could), and the stock UI still reacts to the knob. Good for a one-evening data-plumbing spike, not the product |
| D | Build on atomicstack's Zig runtime (canvas API / Berry scripts) | Technically closest, but **no license**; Berry scripts can't own the buttons or knob-push; HTTP is 1 request/connection, 2 SSE subscribers, 10 raw frames/s. Use as documentation |
| E | sanderdw/awtrix-ng-tc002 (AWTRIX NG port, C++) | Wrong interaction model (app carousel, no outward input events); PolyForm Noncommercial. Best readable reference for SPI/evdev/MCU plumbing. Its SHA gate trips on 1 of 3 files on app 1.0.5; `--force` exists |
| F | Official FlyThings IDE | Windows-only Eclipse; skip. Ulanzi's `Z21_TC002_Demo` (GPL-3.0) is still the safest *copyable* reference code |

Prior art worth reading: [atomicstack/tc002-customisation](https://github.com/atomicstack/tc002-customisation)
(= aquarat mirror; LED-SPI.md, DEVICE.md, FIRMWARE.md, RUNTIME.md), [UlanziTechnology/Ulanzi-U-Clock-TC002](https://github.com/UlanziTechnology/Ulanzi-U-Clock-TC002)
(GPL-3.0, protocol docs + demo), [sanderdw/awtrix-ng-tc002](https://github.com/sanderdw/awtrix-ng-tc002),
puritysb/AgentDeck (MIT, 240★: agent dashboards on Stream Deck+/Pixoo/TC001 via hooks + JSONL — read-only, **no one
changes model/effort on a live session anywhere**), paultyng/agentsd (Stream Deck dial cycles Claude sessions),
nimbus-notify / vibesignal (status-light colour conventions). The TC002 ecosystem is ~6 weeks old, 13 repos, n=1 device each.

## 3. Transport

| | WiFi TCP | USB (`adb forward tcp:N tcp:N`) | BLE |
|---|---|---|---|
| Works docked / cable-free | yes | no | yes |
| Both hosts at once | yes | no (cable = host selector) | no (one central) |
| Survives device reboot unattended | yes (~16 s) | **no — needs replug** | — |
| macOS permissions | Local Network Privacy: terminal-launched OK; a bare LaunchAgent is reportedly blocked silently [untested] | loopback → exempt | Bluetooth TCC can't be granted to a headless daemon |
| Device-side effort | trivial (socket) | none (adbd already there) | write GATT over raw HCI from scratch |
| Latency | fine with `power_save off`; p99 unmeasured | est. 1–5 ms, unmeasured | ≥ ~30 ms RTT floor on macOS |

Decision: **device app listens on one TCP port with a small framed protocol; hosts connect over WiFi
(discover via a UDP beacon our app emits, same style as stock) or over USB via `adb forward` — identical host
code.** Latency barely matters because the picker renders on-device; the wire carries ~400 B agent lists,
~50 B deltas, and intents. Never frame data through `adb shell` (LF→CRLF mangling). Add keepalives.
A third option exists (USB raw-HID gadget, `f_hid` is registered) but switching gadget functions would drop adb — skip.

## 4. herdr integration [verified against 0.8.2 unless noted]

- Socket `~/.config/herdr/herdr.sock` (same path scheme on macOS [sourced]), newline-delimited JSON,
  `{"id","method","params"}` → `{"id","result"|"error"}`, protocol 20.
- **One request per connection**, then the server closes it (undocumented; verified 3 ways). Only
  `events.subscribe` stays open and streams. Subscribe with dotted names (`pane.updated`), receive underscored
  (`pane_updated`). Bootstrap: open subscription → wait for `subscription_started` → `session.snapshot` on another connection.
- `pane.updated` carries full PaneInfo incl. `agent_status` and `tokens` → the workhorse subscription.
  `pane.agent_status_changed` needs a `pane_id` and carries no tokens.
- **`pane.report_metadata`**: up to 16 tokens/call (32 kept), values ≤ 80 chars, optional `ttl_ms`, `seq`,
  plus `display_agent`, `title`, `state_labels`. Tokens come back in `agent.list`/snapshot and render in
  herdr's sidebar as `$name` (`[ui.sidebar.agents] rows = [["state_icon","workspace","tab"],["agent","$model","$effort"],["$ctx"]]`).
  This is both the data channel *and* the "feedback inside herdr". 0.9.1 adds conditional colour rules. [untested: whether a token write emits `pane.updated`; else poll snapshot at ~1 Hz]
- `agent.focus` marks the agent **seen** (done→idle, Done badge cleared) → call it only on commit, never per detent. [untested: that it moves the attached TUI's view]
- Input: `pane.send_input {pane_id, text?, keys?}` (atomic, no guard) vs `agent.prompt` (refuses when `blocked`).
  Keys: `enter`, `esc`, `ctrl+x`, … `pane.wait_for_output` and `pane.read {source:"visible"}` give closed-loop checks.
- "Agent name": your agents are currently **unnamed** (`name` absent; sidebar shows workspace + tab label, the
  `agent` token is just "claude"). Label ladder: agent name if set → workspace label → Claude `session_name`.
- Plugins (`herdr-plugin.toml`: actions, events, panes, keybindings, one-shot `[[startup]]`) **cannot supervise a daemon**.
  Use a plugin for install/keybindings/popup/"ensure daemon running"; keep the bridge a separate process.
- herdr's Claude hook (`~/.claude/hooks/herdr-agent-state.sh`, managed, v8) is the pattern to copy for socket writes; never edit it.
  `HERDR_PANE_ID` / `HERDR_SOCKET_PATH` are inherited by Claude's child processes (verified for Bash-tool children; statusline child inferred).

## 5. Claude Code: reading and changing a live session [docs + local 2.1.277]

Read:
- **statusLine stdin JSON** has `session_id`, `session_name`, `model.id`, `model.display_name`, `effort.level` (live),
  `context_window.{context_window_size, used_percentage, remaining_percentage, current_usage}` (null before first call and after `/compact`),
  `prompt_cache`, `cost`, `rate_limits`, `transcript_path`. **The bridge's only source for model, effort and context since 2026-09-19**
  (`pixbar-bridge statusline`, called from the script, keeps each session's newest input in `~/.cache/pixbar/sessions/`). An earlier note here
  said it does not re-run on a model/effort change; the code of 2.1.269, .277 and .278 says it does (re-run, debounced 300 ms, when any of
  `tokenUsage, permissionMode, vimMode, mainLoopModel, fastMode, effortValue, thinkingEnabled, prStatus` changes) [read in the binary, not
  yet seen live]. If it turns out not to: `refreshInterval` (≥1 s) in the `statusLine` setting. The messages it sees start at the last
  compact boundary, hence the null usage after `/compact`. Measured: the transcript gets a request's usage only when the reply's first
  block is written, a median 5.4 s after the status line has it (234 requests, xhigh). Existing `~/.claude/statusline-command.sh` is chainable (`input=$(cat)`); it has a latent bug (`hostname` not on PATH).
- Transcript JSONL (`message.model`, top-level `effort`, `message.usage`; no window size; skip `isSidechain`): was the source until
  2026-09-19, late by the length of a reply and blind to a change until the next reply. Still read for one thing: the `compact_boundary`
  marker's `postTokens`, the context right after a compaction.
- Hooks: `PostModelSwitch` (also fires on automatic fallback), `effort` on tool/Stop hooks, `ConfigChange` for settings writes.

Write (no control API exists for a TUI session; issue #65586 closed "not planned"; the Agent SDK's `setModel`/`applyFlagSettings` only covers SDK-hosted sessions):
- Type `/effort <low|medium|high|xhigh|max>` or `/model <opus|fable>` via `pane.send_input`. These commands are **never queued**: they run mid-turn without interrupting it.
- Hazards:
  1. An injected **Enter approves any open dialog** (user keybindings bind `enter → confirm:yes`). Gate on `agent_status != blocked` **and** a `pane.read visible` check.
  2. Typed `/model X` and `/effort X` **save as the global default** for new sessions (`max` is always session-only). Session-only needs the picker + `s`, whose row order is session-dependent → fragile.
  3. Cost: any model switch = full uncached re-read of the context; effort change is **free and dialog-free on Fable 5.1** (v2.1.260+), but on Opus 5 it invalidates the cache and shows a confirm while the cache is warm.
  4. A `PreModelSwitch` hook returning `permissionDecision: "allow"` skips the model confirm (values are allow/deny/ask; hook must answer fast — a timeout blocks the switch). Its payload includes `prompt_cache_warm`, `context_tokens`, `estimated_cache_write_usd` → usable as an on-device cost hint. No equivalent hook for the effort confirm.
  5. Text containing a space + user's `space → voice:pushToTalk` binding in tap mode; slash-autocomplete may swallow Enter (`ctrl+x enter` = queueSubmit is the safer submit). [untested]
  6. `CLAUDE_CODE_EFFORT_LEVEL` in the environment pins effort and makes changes no-ops.

## 6. Proposed architecture

```
 Claude Code session ──statusLine wrapper──► herdr pane tokens {model, effort, ctx, win}
                                                   │ (also shown in herdr's sidebar)
 herdr server ◄──socket──► pixbar-bridge (one per machine: Linux, Mac)
                               │  state: agents[{host,pane,label,status,model,effort,ctx}] + focus
                               │  intents: focus / effort / model
                               ▼ framed TCP (WiFi, or USB via adb forward)
                         pixbar-device (static ARM binary on the TC002)
                         evdev in → local 60 fps render → SPI out; optimistic UI, reconciles on state push
```

- Device accepts several bridge connections; agent list is the union, namespaced `(host, pane_id)`; intents route to the owning host.
- Renderer = pure function `(state, t) → 52×16 RGB`, no OS calls → same code builds for the device, a desktop window and WASM (browser preview). Build the simulator first.
- Bridge bootstraps the device: if it hears the *stock* beacon, `adb push` + stop zkswe + start our binary (keeps the no-flash property).
- Every state push carries a revision; device asks for resync on a gap.

## 7. Control scheme

| Input | Action |
|---|---|
| Knob rotate | Picker opens on first detent (cut, not fade); selection moves locally. After ~1.2 s dwell the picker closes and the *pixbar's* selected agent (what the buttons act on) becomes the highlighted one. herdr is not touched |
| Knob push | Focus the selected agent in herdr (`agent.focus`) — deliberate, so a timer never steals keyboard focus mid-typing. Outside the picker: focus the currently shown agent |
| Knob long-push | Jump to the next agent needing attention (blocked → done) |
| Right / Left (= up / down) | Effort +/−. Presses accumulate on the staircase overlay; one `/effort` is sent ~700 ms after the last press. Bump + thud at the ends |
| Middle (= press) | Model toggle Opus ↔ Fable. First press shows target model + cost hint (context to re-read); second press within ~2 s commits; timeout cancels |
| Middle long-press | Detail page: `104K / 1M`, cost, cache state |
| herdr focus changes (keyboard) | Pixbar selection follows (`pane_focused`) |
| Selected agent is `blocked` | Effort/model inputs are refused (bump); name pulses amber |

## 8. Display design (52×16)

Principles forced by the hardware: **hue = identity, motion = state, area = magnitude, text stays near-white.**
Only `blocked` pulses. No app carousel — stable screen, interrupt-driven overlays. Fonts: a 5×7 face (8 chars) and a
proportional 3×5 face (~13 chars); draw our own or use a permissively licensed one (the mockups below were measured against the community runtime's faces).

Resting screen "DECK" — name (5×7), model word (3×5), effort pips (3/5 shown), context hairline (row 15), status rail (x50–51):
```
     0         1         2         3         4         5
     0123456789012345678901234567890123456789012345678901
  0 |....................#............................:##|
  1 |.................................................:##|
  2 |#.##...###..#...#..##....###..#...#..###..#.##...:##|
  3 |##..#.#...#.#...#...#...#...#.#...#.#...#.##..#..:##|
  4 |#.....#####.#...#...#...#####.#.#.#.#####.#......:##|
  5 |#.....#......#.#....#...#.....#.#.#.#.....#......:##|
  6 |#......###....#....###...###...#.#...###..#......:##|
  7 |.................................................:##|
  8 |.................................................:##|
  9 |.#..##..#.#..##...............................::.:##|
 10 |#.#.#.#.#.#.#..............................::.::.:##|
 11 |#.#.##..#.#..#..........................##.::.::.:##|
 12 |#.#.#...#.#...#......................##.##.::.::.:##|
 13 |.#..#....##.##....................##.##.##.::.::.:##|
 14 |.................................................:##|
 15 |################################::::::::::::::::::##|
```
Alternative "AMBIENT": name centred + context hairline only; model/effort appear on interaction.

Knob picker "THREE-UP" — prev / **current** / next in 3×5, caret at x0, position dots on row 15:
```
  0 |....:.:.:::.::......:.:.:::.........................|
  1 |....:.:.:...:.:.....:.:..:..........................|
  2 |....:::.::..::..:::.:.:..:..........................|
  3 |....:::.:...:.:.....:.:..:..........................|
  4 |....:.:.:::.::.......::.:::.........................|
  5 |#....#..##..###.....##..###.###..#...##.###..#..##..|
  6 |##..#.#.#.#..#......#.#.#...#...#.#.#....#..#.#.#.#.|
  7 |###.###.##...#..###.##..##..##..###.#....#..#.#.##..|
  8 |##..#.#.#....#......#.#.#...#...#.#.#....#..#.#.#.#.|
  9 |#...#.#.#...###.....#.#.###.#...#.#..##..#...#..#.#.|
 10 |....::...:...::..::.................................|
 11 |....:.:.:.:.:...:...................................|
 12 |....:.:.:.:.:....:..................................|
 13 |....:.:.:.:.:.....:.................................|
 14 |....::...:...::.::..................................|
 15 |......+.........+.........#.........+.........+.....|
```
Rows slide 5 px per detent over 5 frames with offset table `[2,4,6,6,5,5]` (1 px overshoot); end-stop bump `[+1,−1,0]`;
confirm = 2-frame white flash; linger 1200 ms. Name colour = status, right-edge letter/colour = model.

Effort overlay — word in 5×7 + five-step staircase (8 px blocks on 9 px pitch, heights 3–7); unlit steps are 1 px floor stubs:
```
  0 |..................##................................|
  1 |...................#................................|
  2 |...................#....###..#...#..................|
  3 |...................#...#...#.#...#..................|
  4 |...................#...#...#.#.#.#..................|
  5 |...................#...#...#.#.#.#..................|
  6 |..................###...###...#.#...................|
  7 |....................................................|
 12 |....########........................................|
 13 |....########........................................|
 14 |....########.########.########.########.########....|
```
New step overshoots ~2 px and settles (~120 ms); `max` tints the top step and flashes once.

Model overlay — agent name recessed (3×5, top), model word in 5×7, 2 px underline in the model hue; transition = vertical "flip" (squash to centre line, grow new) with a 50 ms hit-stop before it.

Palette (values to *send*; zero unwanted channels or they get lifted to ≥50): idle `#0064ff`, working `#ff6400` (or model hue), blocked amber `#ffb020`
(cross-project convention; herdr itself uses red ×), done `#00ff32`, error `#ff0000`; Opus `#6400ff` or Anthropic-orange `#ff6a3d`, Fable teal `#00c8b4` / blue `#5aa9ff`;
text `#e0e0d0`; recessed `#121212`; context bar blue → amber >70 % → red >90 % (Claude Code's own thresholds). Status glyph vocabulary to rhyme with herdr/Claude Code: `× ◐ ✓ ○ ·`
(◐ is literally Claude Code's title spinner since 2.1.228). Consider drawing context as a *draining* bar ("N % left", Codex-style).

Sound (real speaker, one WAV at a time): no per-detent clicks — the knob's mechanical detent is the tick. Commit 1760→2640 Hz 35 ms each; effort up 2200 Hz / down 1650 Hz 30 ms; rejected 440 Hz 40 ms; model toggle 2640→1976 Hz; blocked = two 880 Hz pulses.

## 9. Unknowns → first experiments

1. `adb shell getevent -lt` while turning the knob and pressing each button: confirms codes, detents/rev, fast-spin drops, and the stock-firmware input path.
2. In an isolated `herdr --session pixbar-test` with a disposable Claude session: inject `/effort high` via `pane.send_input` — does autocomplete eat Enter, is text pasted or typed, what do the Opus confirm dialogs look like on `pane.read visible`?
3. Does `pane.report_metadata` emit `pane.updated`? Does `agent.focus` move the TUI view?
4. Push a 20-line test binary that paints the panel (validates toolchain, SPI, latch, level curve, real frame ceiling).
5. On the Mac: socket path, Local Network permission behaviour for the bridge, `adb` reachability.
6. WiFi RTT p99 to the device with power save off; `adb forward` RTT.

## 10. Decisions and verified results (added 2026-09-19, after review)

Decisions: display always follows the **herdr-focused agent** (no separate pixbar selection); typed-command default
leak was accepted but is now unnecessary (below); implementation language **Rust** (shared protocol crate for bridge + device).

**Write path — verified live** in an isolated `herdr --session` with a scratch Claude Code 2.1.277 session:

| Action | Keys via `pane.send_keys` | Result |
|---|---|---|
| Effort ± | `alt+p`, `left`/`right` × N, `s` | Picker opens with the cursor on the *current* model; effort changes; banner "…for this session only"; **half-typed draft untouched**; `~/.claude/settings.json` untouched |
| Model toggle | `alt+p`, `up`/`down` to the target row, `s` | Same: session-only, draft untouched. Target row must be found by reading the picker (`pane.read --source visible`, rows are numbered `1. Default / 2. Opus / 3. Fable ✔ / 4. Sonnet / 5. Haiku`, cursor = `❯`) |
| Mid-turn | same keys while `agent_status = working` | Picker opens, change applies, turn keeps running, draft typed mid-turn preserved |

Traps found: pressing a **number key** in the picker selects *and saves as the global default* immediately (no `s` chance) — never send digits.
`Enter` saves as default and writes the full id (`claude-fable-5-1[1m]`), not the alias. Not yet tested: the warm-cache confirm dialog on
Opus effort changes / model switches (needs a real conversation with a warm cache), and behaviour while a permission dialog is open (bridge must refuse when `blocked` anyway).

Layout direction (mockups in `docs/mockups/`, regenerate with `render.py`): left 9 px = agent strip (3×3 status blocks, 2 columns × 4, white caret marks the
focused agent); right 42 px = small name line, an **effort slider** (5 detents, track filled in the model hue, thumb = 7×7 model token with a cut-out `O`/`F`), context hairline on row 15.
Effort press: token glides to the next detent with overshoot, top line swaps to the level word for ~1.2 s. Model press: token card-flips, track re-colours with a wipe.
Knob: caret walks the strip, main area shows the hovered agent's name + model + effort; push = `agent.focus`, 1.2 s without a push = cancel.

**Layout v2 (supersedes the slider-at-rest idea above).** Resting screen = statusline-like text next to the agent strip: line 1 `MODEL EFFORT`
(model word in the model hue, effort word white; effort words are LOW/MED/HIGH/XHIGH/MAX so `FABLE XHIGH` fits 40 px), line 2 `SPACE·TAB` recessed
(10 characters fit; longer labels bounce-scroll), rows 13–14 context bar (blue → amber ≥70 % → red ≥90 %). Row budget: 16 rows hold two 3×5 text lines
plus a 4-row band, not three text lines — so the context *number* (`742K 74%`) takes over line 2 temporarily (threshold crossings, long-press) rather than living there.
Effort slider and model flip are temporary overlays like the knob picker. Slider shapes are round: candidate A "beads" (five discs on a string, passed = filled,
ahead = hollow ring, current = larger disc that glides), candidate B "rail" (capsule track, white-rimmed round knob, pin-hole detents). Mockups: `docs/mockups/*.png`.

**Layout v3 + simulator (2026-09-19).** Chosen: **rail** slider overlay; **context as a number** at rest (`104K 10%`, white → amber ≥70 % → red ≥90 %).
Three 3×5 text lines need 17 rows, the panel has 16, so the resting screen is two lines: `MODEL EFFORT` / context. The agent label (`SPACE·TAB`) takes line 2 for 2 s
whenever herdr focus changes and is the headline of the knob picker; alternative layout `NameLine` keeps the label always visible with context as percent only.
Code: `crates/pixbar-render` (pure `(world, inputs, now_ms) → frame`, unit-tested: settle-then-commit effort, two-press model confirm, push-to-focus, blocked refusal)
and `crates/pixbar-sim` (desktop LED window with a fake host that confirms intents after 400 ms; `--dump` prints scripted ASCII frames).
Run: `cargo run -p pixbar-sim` — up/down/wheel = knob, enter = knob push, left/right = effort, space = model, L = layout, B = cycle status.

**Control changes (2026-09-19, after trying the simulator).** Knob behaves like Tab: a single click sends `Focus` immediately; during a spin
(clicks < 150 ms apart) only the agent the knob stops on is focused, so herdr is not thrashed and passed-over agents are not marked seen.
Knob push is now "jump to the next agent that needs you" (blocked, then done). The model button is a direct toggle: every press flips the word at once,
the change is sent 700 ms after the last press (underline fills while pending), and pressing twice inside that window sends nothing — same settle rule as effort.

## 11. Ultracode (2026-09-19; docs + Claude Code 2.1.278 binary + live test in an isolated herdr session)

**What it is.** A session flag, not a reasoning level: it sends `xhigh` to the model and has Claude orchestrate multi-agent workflows. On Claude Code's own
slider it is the stop *past max* (picker row glyphs `○ Low ◐ Medium ● High ◉ xHigh ◈ Max ✦ Ultracode`), set apart behind a `┆` divider. It exists only where
workflows are enabled and the model/cap allows xhigh — otherwise the ring has five stops. Side effects worth knowing: in Auto mode the per-run workflow approval
prompt is skipped while ultracode is on, and workflows can spend a lot of tokens.

**Write path (verified live, draft-safe, session-only, settings untouched):** `alt+p`, `right`/`left` × N, `s`. xhigh → ultracode = `alt+p right right s`;
back = `alt+p left s` (lands on Max). Traps: the picker's effort ring **wraps** (right of Ultracode is Low), so the bridge must read the row text after `alt+p`
(the picker opens on the live level) and send the exact difference, never a blind count; `s` applies ultracode only if the slider was actually moved; never send
`up`/`down` (`s` commits the highlighted model too), digits or Enter. The binary shows no cache-confirm dialog on the picker path (the `/effort` slider has one).

**Read path.** Everything structured reports plain `xhigh`: statusLine `effort.level`, hooks, transcript `effort`/`perTurnEffort`, terminal title. Live signals are
on screen only: a static right-aligned ` ultracode ` tag in the prompt box's top border (rgb 175,135,255) and the mode line
`✦ ultracode · xhigh effort + dynamic workflows…` → bridge scrapes `pane.read visible` for sessions reporting xhigh (and ~1 s after its own `s`, to catch a
silent refusal, e.g. `CLAUDE_CODE_EFFORT_LEVEL` set). Transcript `{"type":"attachment","attachment":{"type":"ultra_effort_enter"}}` exists but is written only when
the next prompt is submitted; `ultra_effort_exit` has never been observed locally.

**Claude Code's animations (from the bundle; shimmer also measured on screen).** Ultracode's slider stop uses `violet-ripple`: rings expanding from the stop,
30 cells/s, wavelength 20 cells, raised-cosine bands quantised to 8 background colours `#3E1676 → #8C50F0`, field stays washed violet behind the wavefront, dark
ahead of it, white text on top, 80 ms tick; vertical distance is doubled only to correct terminal cell aspect. Typed keyword shimmer: a 3-cell bright window
(rgb 208,180,255 over 175,135,255) advancing one cell per 50 ms, cycle = (length + 20) steps, so roughly 0.5 s sweep then ~0.9 s rest. Siblings: `max` =
per-character rainbow cycle (100 ms/frame), `xhigh` = single-character shimmer sweep. The live `/effort` slider itself was not opened (it needs Enter), so the
ripple has not been seen on a real terminal yet.

**Pixbar implementation.** `Effort::Ultra` is a sixth stop offered when `Agent.ultra_ok`; rail re-pitched to 6 px with ULTRA detached at x=47 behind a gap; the
rail clamps (bump at the end) even though the picker wraps. Overlay on ULTRA = ripple from the knob over the main area with the exact 8-colour ramp, no aspect
doubling (square LEDs), retuned to the 40 px field (20 px/s, 13 px wavelength → same ~650 ms ring rhythm), white word, overlay lingers 2.6 s. Resting line shows
`MODEL ULTRA` with ULTRA in the tag violet and the 3-px shimmer window at the 50 ms step. Colours are Claude Code's own values; how the panel's level curve
shifts them is still unmeasured.

## 12. Build status (2026-09-19)

Working end to end on the real hardware: `pixbar-device` (513 KB static ARM binary, Rust, `armv7-unknown-linux-musleabihf` linked with rust-lld —
no cross gcc) runs from `/tmp` after `setprop ctl.stop zkswe`, drives the panel at 50 fps with zero late frames, listens on TCP 17002 and beacons
on UDP 17003. `pixbar-bridge` reads the live herdr session (snapshot + event subscription), takes model / effort / context from the Claude Code
status line data of each pane's session (§5; from the transcript until 2026-09-19), detects ultracode by the ` ultracode ` tag on the pane's screen, and executes intents.
Verified in an isolated herdr session: effort through all six stops in both directions, model both ways, mid-draft, with a warm prompt cache
(no confirm dialog appears on the picker route) — prompt draft preserved and `settings.json` byte-identical every time.
Lessons: at Max the picker's warning text pushes its footer off a short pane, so only the "Select model" title is a safe marker;
adbd kills a daemon that has not called `setsid` yet when the shell returns, so the parent lingers 300 ms.
Not yet done: statusline-fed herdr sidebar tokens (needs an edit to `~/.claude/settings.json`), macOS run, models other than Opus/Fable (shown as Opus),
more than 8 agents, brightness control from the device, hands-on confirmation of knob direction and button mapping.

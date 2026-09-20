# pixbar

A Ulanzi TC002 ("Pixbar", 52×16 LED matrix, knob + 3 buttons) as a hardware companion for [herdr](https://herdr.dev):
see every Claude Code agent's status, switch between them with the knob, and change the focused session's effort
level and model with the buttons.

Not affiliated with, endorsed by or supported by Ulanzi or Anthropic. Ulanzi and TC002 are Ulanzi's names; Claude
and Claude Code are Anthropic's. Nothing is flashed: the program runs from the panel's RAM in place of Ulanzi's
app, and switching the panel off and on brings Ulanzi's firmware back.

```
 Claude Code sessions ── status line + screen ──┐
 herdr server ◄── unix socket ──► pixbar-bridge (host) ◄── TCP, JSON lines ──► pixbar-device (on the TC002)
```

| Crate | What it is |
|---|---|
| `pixbar-render` | The whole UI as a pure function of (agents, inputs, time) → 52×16 frame. Shared by everything below |
| `pixbar-sim` | Desktop LED window with a fake host: `cargo run -p pixbar-sim` |
| `pixbar-proto` | Wire messages between bridge and device |
| `pixbar-device` | Static ARM program that replaces Ulanzi's app at runtime (nothing is flashed) |
| `pixbar-bridge` | Host daemon: reads herdr + Claude Code's status line data, focuses agents, drives the `/model` picker |

## What it needs

- An Ulanzi **TC002** (the Linux one with the 52×16 panel; the TC001 is a different, ESP32 device). Either joined
  to your WiFi with Ulanzi's app, which is how it learns a network, or on a USB cable, which needs no network.
- [herdr](https://herdr.dev) with its Claude Code integration (`herdr integration install claude`): that is how
  a pane is matched to its Claude Code session.
- Claude Code, run inside herdr panes.
- Rust 1.87 or later from [rustup](https://rustup.rs) to build it; rustup fetches the ARM target by itself
  (`rust-toolchain.toml`). No cross compiler, no adb, no libusb.
- Linux or macOS. Everything here was done on Linux; on macOS it compiles and the code paths are there
  (LaunchAgent, USB), but nobody has run it yet.

Tried with herdr 0.8.2 (socket protocol 20), Claude Code 2.1.278, and a TC002 whose `getprop` says
`ro.firmware ssd21x_ulanzi_I008`, `ro.build.date 20260527`, `ro.easyui.version 2.4.0`, `ro.system.version 2.6.2`
(`pixbar-bridge shell IP getprop` shows yours). It leans on things neither herdr nor Claude Code promises to keep:
the fields of herdr's snapshot, the words on Claude Code's `/model` picker, the status line's input. `doctor`
says which link is broken when one of them moves.

## Run it

```sh
cargo build --release                     # the bridge, with the device program inside it (rustup fetches the ARM target)
target/release/pixbar-bridge install      # copy it to ~/.local/bin, hook the status line, start the service
pixbar-bridge deploy [IP|usb]             # once per panel: start the pixbar program on it
pixbar-bridge doctor                      # every link from Claude Code to the panel, checked
```

`install` does three things, and `uninstall` takes all three away again:

- It copies the binary to `~/.local/bin/pixbar-bridge`, so that nothing points into the build directory. After a
  rebuild, run `target/release/pixbar-bridge install` again: it replaces the copy and restarts the service.
- It runs `pixbar-bridge run` as a service from login on: a systemd user unit on Linux
  (`journalctl --user -u pixbar-bridge`), a LaunchAgent on macOS (`~/Library/Logs/pixbar-bridge.log`; written but
  not yet tried on a Mac). Installed from inside a pane of a named herdr session, the service is pinned to that
  session's socket. `--no-service` leaves this out; `pixbar-bridge run` in a terminal does the same job.
- It adds a call to your status line script, the way an installer adds a PATH line to a shell profile. Model,
  effort, context, cost, usage windows and the session name all come from Claude Code's status line input, so the
  script that draws your line hands a copy of it to the bridge, right after it has read it:

  ```sh
  input=$(cat)
  # >>> pixbar-bridge >>>
  # Added by `pixbar-bridge install`, removed by `pixbar-bridge uninstall`: the panel's model, effort and context.
  printf '%s' "$input" | /home/you/.local/bin/pixbar-bridge statusline 2>/dev/null || true
  # <<< pixbar-bridge <<<
  ```

  The script stays yours and stays the status line command; `settings.json` is not touched. A second `install`
  brings the block up to date where it stands, and a call to the bridge that you had put in by hand is taken
  over in place. It knows how to add to a shell script that reads its input into a variable (`input=$(cat)`, the
  form in Claude Code's examples and the one `/statusline` writes). For anything else (a Python script, a `jq`
  one-liner in `settings.json`) it changes nothing and prints the line to add: `pixbar-bridge statusline` takes the
  JSON on its stdin and prints nothing. Only where there is no status line at all does `install` set
  `statusLine.command` in `~/.claude/settings.json`, to `pixbar-bridge statusline` by itself. `--no-statusline`
  leaves all of this out.

Claude Code runs the status line whenever the model, the effort or the token count changes, so the panel follows a
`/model` typed in the session as fast as one made with its own button, and the context figure is Claude Code's own
(what was sent plus the reply). The usage windows are the account's, so the newest report of any session counts for
all of them. The newest input of every session is kept in `~/.cache/pixbar/sessions/`. A session that was already
running when this was set up reports in at its next change; one that never does shows as Opus, high, 0K, and
`doctor` says why (a project with a `statusLine` of its own replaces yours, a folder whose trust prompt was not
accepted runs none, `disableAllHooks` switches it off).

Together with the auto-start below, switching the device on is all it takes. The bridge waits quietly while herdr
is not running.

The bridge carries the device program inside it and speaks enough of the adb protocol itself (the device's adbd on
port 5555 asks for no key), so none of this needs adb installed:

- `pixbar-bridge deploy [IP]` pushes the program and starts it, and remembers the device (by MAC, in `~/.config/pixbar/devices`).
- `pixbar-bridge run` connects to a running pixbar, found by its beacon. The device runs the program from RAM, so after
  every power-up it is back on Ulanzi's firmware; when `run` hears a device it remembers in that state, it starts the
  program again by itself. A TC002 it has never deployed to is left alone. `run --device IP` names the device instead.
- `pixbar-bridge stock [IP]` gives the panel back to Ulanzi's firmware, and so does STOCK FW in the panel's settings.
  It stays that way, service or not, until `deploy` or until the panel has been switched off and on.
- `pixbar-bridge find` lists what is on the network.
- A device running another build than the one this bridge carries is reported, not replaced: two hosts with
  different builds would otherwise push over each other. `deploy` updates it.

Several hosts may connect at once (Linux box and Mac); the panel lists all their agents and sends each request to
the host that owns the agent. The HOSTS settings page picks whose agents it shows: all of them, or one machine's.

On another machine or network (macOS: rustup and the ARM target are all it needs; untested there):

- It joins the WiFi saved by Ulanzi's setup (`/data/misc/wifi/wpa_supplicant.conf`, one network). A new network means
  going through that setup again, on stock firmware.
- Over a USB cable everything works that works over WiFi, with no network at all and no adb tool: `deploy usb`,
  `stock usb`, `log usb`, and `run`, which takes a panel on the cable before it listens for one on the network
  (`run --device usb` takes nothing else). The panel's USB-C port is the same adbd as its port 5555, and the bridge
  opens it itself. Where an adb server runs (it claims every adb device on the bus), the bridge goes through that
  server instead; it never starts one.
- The panel's port is dual-role, and Ulanzi's firmware leaves it in host mode, where a PC sees nothing. After a
  power-up it does answer for some two seconds (measured: from 3.4 s to 6.0 s after the reset). A `run` that is
  already waiting catches that, leaves a job on the panel that turns the port back, and starts the program over
  the cable once Ulanzi's app has joined the WiFi: about 20 s from power-up to a working panel, tested on Linux
  with and without an adb server. So: plug the cable in first, then switch the panel on. A panel that was
  switched on with no bridge listening stays invisible on the cable until it is switched off and on (or started
  over WiFi once: the pixbar program keeps the port in device mode for as long as it runs).
- Linux gives whoever is logged in at the machine access to adb devices from systemd 258 on. On an older one, or
  for a service that runs with nobody logged in, the bridge names the udev rule that is missing.
- Inside a herdr pane the bridge uses that pane's herdr (`HERDR_SOCKET_PATH`); anywhere else it looks where herdr
  puts its socket (`~/.config/herdr/herdr.sock`, under `XDG_CONFIG_HOME` if that is set, under `sessions/<name>/`
  with `HERDR_SESSION`). `--socket` names one.
- On any network it joins, the stock firmware serves a root adb shell on port 5555 without a password, and this
  program's port 17002 has none either.

## Controls

| Input | Action |
|---|---|
| Knob | Focus the next / previous agent in herdr, like Tab. A fast spin only focuses where it stops. The agent's name stays up for the linger time (10 s) |
| Knob push | While the name is up after a turn: back to the resting screen. Otherwise: jump to the next agent that needs you (blocked, then done) |
| Knob long-press | Settings (below) |
| Left / right | Effort down / up: LOW · MED · HIGH · XHIGH · MAX · ULTRA (ultracode). Sent 0.7 s after the last press. Held down, a button repeats like a key (after 0.4 s, then every 0.15 s), here and in the settings; a repeat never counts as the confirming press of TURN OFF / STOCK FW |
| Middle | The next model (see Models below). Shows its name with a `?` and the context that would be re-read uncached; press again within 4 s to send, leave it or press anything else to drop it. A session that has not replied yet since it started, was cleared or was compacted switches on one press |

The left strip has one block per agent, coloured by status. Focus is brightness: the focused block is at full
brightness, working / done agents at 40 %, idle ones at 18 %, and a blocked one breathes below the focused level.

Settings: the knob turns the pages, left / right change the value (as they move the effort rail), knob push closes.
Rows, styles and the name are previewed live with the focused agent's data. Kept on the device in `/data/pixbar.conf`;
reachable without a host, so the HOSTS page can tell you the address to connect to.

| Setting | Values |
|---|---|
| BRIGHT | 10–100 % |
| BLOCKS | 4x4, 3x3 or 2x2, each spaced (6 / 8 / 15 agents) or touching (8 / 15 / 32; every other block a shade darker so neighbours still tell apart). With more agents the strip pages along with the focus |
| ROW 1, ROW 2 | what each resting row shows: `FABLE XHIGH`, `104K 10%`, the name, `5H 12% 3H` or `7D 29% 4D` (how much of the account's 5-hour or 7-day usage window is used, amber from 70 %, red from 90 %, and how long until it starts over; `--` where Claude Code reports none, as with API billing), or `$12.34` (what the session has cost so far, as Claude Code reckons it: on a subscription that is the value of the tokens, not a bill) |
| STYLE 1, STYLE 2 | how that row is drawn: plain; dim (40 % brightness); card (black text on a full-bright rounded card, white or the row's colour); tint (lit text on a dark, saturated card). Cards start at the same column as plain text. On the model row the model name becomes a chip and the effort stays lit text beside it |
| NAME | what an agent is called: space, tab, both, working directory, Claude Code's session title, or the name given with `/rename` (SESS; a session never named goes by its space) |
| LINGER | how long the name stays after a knob turn: 2–20 s |
| REFRESH | how often hosts re-read model, effort and context: 0.25–5 s. herdr events (status, focus) always arrive at once |
| HOSTS | Connected hosts and how each got there (`DESKTOP USB`, `LAPTOP WIFI`). Left / right pick whose agents the panel shows: `ALL`, or one of them. The pick is kept by the host's name, so it holds when that machine connects again, by cable or WiFi; `NOT HERE` says the picked one is away, and its place is held until you step off it. While it is away the resting screen says so too, rather than leave an empty strip |
| DEVICE | read-only, left / right step through: battery (charge, and the cell voltage or "on USB"), the WiFi network it is set up for, its `ip:port`, version |
| TURN OFF | right, then right again within 4 s: powers the device off through its MCU |
| STOCK FW | the same two presses: stops this program and starts Ulanzi's firmware again (`pixbar-bridge deploy`, or switching the panel off and on, brings this one back) |

Changes are applied by typing into the session's `/model` picker (`alt+p`, arrows, `s`): session-only, leaves a
half-typed prompt alone, never writes `~/.claude/settings.json`, refused while the agent is blocked on a prompt.
Where `~/.claude/keybindings.json` has moved those keys, the bridge uses yours. Every key is followed by a look at
the screen, and anything unexpected ends the sequence with the picker closed and nothing applied.

## Models

The panel has no list of models. It draws the name the bridge makes of Claude Code's own (`Opus 5 (1M context)`
is `OPUS`, `Claude Sonnet 4.5` is `SONNET`; a name too long for the row gives way to the effort word), and the
middle button leads to the next of the models in use on your machine: the ones your sessions have been on in the
last 30 days, most recent first. With one model in use the button has nowhere to go and knocks. To choose
yourself, write the names into `~/.config/pixbar/models`, one per line, as the picker calls them (`Opus`,
`Sonnet`). A session on a model without effort levels shows none, and a session that Claude Code has not
reported on shows `--` and takes no presses. `5H` and `7D` exist for subscription accounts only; elsewhere they
read `--`. An effort stop a session turns out not to have (ultracode, without workflows) is not offered again.

## What goes over the network, and who is trusted

- Once a second the bridge sends the panel, as plain JSON over TCP: this machine's hostname, and for every
  Claude Code pane its herdr workspace and tab, the last part of its working directory, its session title and
  `/rename` name, model, effort, context use, cost, and the account's 5-hour and 7-day usage. The panel sends
  back what you asked for: focus this agent, this effort, this model. The bridge does nothing else at its
  request, and the only keys it ever puts into a session are the `/model` picker's (never a digit or Enter, which
  would save a default; never into a session that is blocked on a prompt).
- `run` connects by itself only to a panel this machine has started (`deploy`) or was told about (`trust`), known
  by its MAC, which the panel announces and says again when connected, before anything about your sessions is
  sent. That keeps the panel of the person at the next desk apart from yours. It is not a lock: a MAC can be
  read off the network and forged by anyone on it, and the link is not encrypted. On a network you do not trust,
  use the USB cable, which involves no network at all.
- The panel itself is open to its network, with or without this project: Ulanzi's firmware serves a root shell
  (adb, port 5555) without a password on any WiFi it joins, and the pixbar program's port 17002 takes any host
  that connects and lists its agents. Keep the panel on a network you would plug an unpatched gadget into.

## Going back

- `pixbar-bridge stock [IP|usb]`, or STOCK FW in the panel's settings: Ulanzi's firmware now, and it stays until
  `deploy` or the next power-up. Switching the panel off and on always ends in Ulanzi's firmware for a moment:
  nothing of this project is on its flash except its settings (`/data/pixbar.conf`, about 140 bytes).
- `pixbar-bridge uninstall`: removes the service, takes its block out of your status line script, and deletes
  `~/.local/bin/pixbar-bridge`, `~/.config/pixbar` and `~/.cache/pixbar`.
- If the panel ever does not come up at all, Ulanzi's recovery is to hold its reset button while switching it on.
  This project has never needed it.

## Battery

The device program asks the panel's MCU for the charge, the cell voltage and whether USB power is present
(`/dev/ttyS1`, 1.5 Mbaud; Ulanzi's app is the only other thing that ever does, and it is stopped). A battery notice
comes up for a few seconds when the cable goes in or out and when the charge falls through 20, 10 and 5 %, never
on top of something you are doing; while the battery is low it returns every minute. Like the stock firmware, the
program warns at 3.60 V, and once the cell has stayed under 3.55 V for 10 s without USB power it counts down 30 s and
powers the device off, so the cell is not run down to its protection cut-off.

## When something is off

`pixbar-bridge doctor` goes through every link and says what mends the broken one: the installed binary, the
service, whose status line command Claude Code runs (yours, a project's, an administrator's), herdr and whether
it knows the panes' sessions, each session's last report, the cable, the network. The usual suspects: a project
with a `statusLine` of its own, a folder whose trust prompt was never accepted (Claude Code then runs no status
line there), a network that keeps its clients apart, a firewall that drops incoming UDP.

```sh
pixbar-bridge state                         # what the device would be told, once
pixbar-bridge set-effort w1:p1 max          # drive one pane's picker directly (also set-model … opus|fable)
pixbar-bridge log [IP|usb]                  # what the device program has logged since it started
pixbar-bridge shell IP|usb 'ls /tmp'        # the panel's root shell (it has no sleep, grep, head or tail)
pixbar-device --demo --log-input            # on the device: built-in agents, print raw knob/button events
```

One static file for another Linux machine, whatever its libc:
`rustup target add x86_64-unknown-linux-musl && cargo build --release -p pixbar-bridge --target x86_64-unknown-linux-musl`.

## Where things came from

- How the LED panel is driven (the SPI frame, the latch line, the brightness curve of the vendor's stack), the MCU's
  serial framing and the recovery procedure were worked out by the people behind
  [tc002-customisation](https://github.com/aquarat/tc002-customisation); this project uses those facts and none of
  their code. `docs/research.md` says which findings are theirs and which were measured here.
- The two pixel faces (3×5 and 5×7) are spelled out in `crates/pixbar-render/src/font.rs`; no font file
  was imported.
- [`docs/research.md`](docs/research.md) is the lab notebook this grew out of, dated and written for its author: a
  record of what was measured and why things are as they are, not a description of the program as it is now.

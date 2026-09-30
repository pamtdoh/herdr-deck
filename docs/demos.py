#!/usr/bin/env python3
"""The README's animations, made from the simulator's frames:

    cargo run -p herdr-deck-sim -- --showcase /tmp/herdr-deck
    python3 docs/demos.py /tmp/herdr-deck docs/demos

The simulator writes a file per feature, each frame 52x16 RGB bytes, one every 40 ms; this draws each as the panel
looks (round LEDs behind a dark face, the unlit ones faintly there; square, so that it sits as well on a light page
as on a dark one) and writes an animated WebP of the same name: it plays and loops in a README like a GIF, with
every colour instead of 256, and it is drawn at twice the size the README shows it at, for screens with twice the
pixels. Where the simulator also wrote how herdr stands at each frame (`<name>.jsonl`), herdr is drawn above the
panel: its agents with their status, the focused one's Claude Code session and its status line, the two following
each other. Its agents and sessions are made up.

Needs Pillow, and for herdr a monospace font with Claude Code's marks (Menlo on macOS, DejaVu Sans Mono on Linux).
"""
import json
import pathlib
import sys

from PIL import Image, ImageDraw, ImageFont

S = 2  # device pixels per README pixel; everything below is laid out in README pixels
W, H = 52, 16
LED = 12  # pixels per LED
PAD = 14  # face around the matrix
PANEL_W, PANEL_H = W * LED + 2 * PAD, H * LED + 2 * PAD
FACE, UNLIT = (22, 22, 26), (34, 34, 40)

# herdr, in the colours of its own dark theme.
HERDR_H, GAP = 300, 10
BG, SIDE, LINE, SELECTED = (22, 21, 28), (27, 26, 33), (48, 46, 58), (40, 38, 54)
TEXT, DIM, FAINT, ACCENT = (222, 220, 230), (124, 122, 136), (80, 78, 92), (182, 156, 255)
CLAUDE, DONE_TEXT = (217, 119, 87), (94, 234, 212)
DOT = {"working": (242, 201, 76), "blocked": (244, 114, 182), "done": (94, 234, 212), "idle": None, "unknown": None}
SPINNER = "·✢✳✶✻✽"

# What each made-up agent was asked, and what it said.
TALK = {
    "web": ("add dark mode to the settings page", "Starting with the theme tokens, then the toggle."),
    "api": ("why is /orders so slow?", "It scans every row. An index on customer_id fixes it."),
    "docs": ("update the install guide for 0.9", "Updated INSTALL.md and the quick start."),
    "infra": ("bump the node image to 22", "Dockerfile and CI now use node:22."),
    "parser": ("support nested lists", "Adding a list stack to the tokenizer."),
    "design": ("tidy up the button styles", "Merged the three variants into one."),
}


def mono(size):
    for path in (
        "/System/Library/Fonts/Menlo.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
        "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
        "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
    ):
        try:
            return ImageFont.truetype(path, size * S)
        except OSError:
            pass
    return ImageFont.load_default(size=size * S)


FONT, SMALL = mono(12), mono(10)


def canvas(w, h, fill):
    return Image.new("RGB", (w * S, h * S), fill)


class Draw:
    """ImageDraw taking README pixels: a box covers the device pixels of every README pixel in it."""

    def __init__(self, img):
        self.d = ImageDraw.Draw(img)

    @staticmethod
    def box(b):
        x0, y0, x1, y1 = b
        return [x0 * S, y0 * S, x1 * S + S - 1, y1 * S + S - 1]

    def rectangle(self, b, fill=None, outline=None, width=1):
        self.d.rectangle(self.box(b), fill=fill, outline=outline, width=width * S)

    def rounded_rectangle(self, b, radius, fill=None):
        self.d.rounded_rectangle(self.box(b), radius=radius * S, fill=fill)

    def ellipse(self, b, fill=None, outline=None, width=1):
        self.d.ellipse(self.box(b), fill=fill, outline=outline, width=width * S)

    def line(self, b, fill):
        """Across or down only."""
        self.rectangle(b, fill=fill)

    def text(self, xy, text, font, fill, anchor=None):
        self.d.text((xy[0] * S, xy[1] * S), text, font=font, fill=fill, anchor=anchor)

    def textlength(self, text, font):
        return self.d.textlength(text, font=font) / S


def frames(path):
    data = path.read_bytes()
    size = W * H * 3
    return [data[i:i + size] for i in range(0, len(data), size)]


def panel(frame):
    img = canvas(PANEL_W, PANEL_H, FACE)
    d = Draw(img)
    for y in range(H):
        for x in range(W):
            i = (y * W + x) * 3
            c = tuple(frame[i:i + 3])
            x0, y0 = PAD + x * LED, PAD + y * LED
            # A LED lit fainter than the face (a pixel fading in or out) is its light on the face, not a dark hole.
            if c != (0, 0, 0) and all(a <= b for a, b in zip(c, UNLIT)):
                c = tuple(min(255, a + b) for a, b in zip(c, UNLIT))
            d.ellipse([x0 + 1, y0 + 1, x0 + LED - 2, y0 + LED - 2], fill=UNLIT if c == (0, 0, 0) else c)
    return img


def tokens(n):
    return f"{n / 1_000_000:.1f}M".replace(".0M", "M") if n >= 1_000_000 else f"{round(n / 1000)}k"


def badge(d, x, y, text):
    """A pill saying what was just used: the knob, a button, the keyboard."""
    w = d.textlength(text, font=SMALL) + 14
    d.rounded_rectangle([x - w, y, x, y + 16], radius=8, fill=ACCENT)
    d.text((x - w / 2, y + 8), text, font=SMALL, fill=BG, anchor="mm")


def herdr(state):
    img = canvas(PANEL_W, HERDR_H, BG)
    d = Draw(img)
    agents, focused, t = state["agents"], state["focused"], state["t"]

    # The sidebar: every agent, its status, the focused one lit.
    d.rectangle([0, 0, 168, HERDR_H - 1], fill=SIDE)
    d.line([168, 0, 168, HERDR_H - 1], fill=LINE)
    d.text((12, 10), "agents", font=SMALL, fill=DIM)
    for i, a in enumerate(agents):
        y = 30 + i * 38
        if i == focused:
            d.rectangle([0, y - 4, 167, y + 30], fill=SELECTED)
            d.rectangle([0, y - 4, 2, y + 30], fill=ACCENT)
        dot = DOT.get(a["status"])
        if dot:
            d.ellipse([12, y + 3, 19, y + 10], fill=dot)
        else:
            d.ellipse([12, y + 3, 19, y + 10], outline=DIM, width=1)
        d.text((28, y), a["space"], font=FONT, fill=TEXT)
        d.text((28, y + 16), f"{a['status']} · claude", font=SMALL, fill=DOT.get(a["status"]) or DIM)

    # The tab of the focused agent's space, and its pane.
    a = agents[focused]
    x0 = 180
    tab = f" {a['space']} "
    w = d.textlength(tab, font=FONT)
    d.rectangle([x0, 8, x0 + w + 8, 26], fill=(58, 48, 92))
    d.text((x0 + 4, 10), tab, font=FONT, fill=ACCENT)
    d.text((x0 + w + 18, 10), "+", font=FONT, fill=FAINT)
    box = [x0, 34, PANEL_W - 12, HERDR_H - 12]
    d.rectangle(box, outline=ACCENT, width=1)
    d.rectangle([x0 + 10, 28, x0 + 62, 40], fill=BG)
    d.text((x0 + 14, 28), "claude", font=SMALL, fill=ACCENT)

    # A Claude Code session, as the made-up agent left it.
    x, y = x0 + 14, 50
    d.text((x, y), "Claude Code", font=FONT, fill=TEXT)
    d.text((x + d.textlength("Claude Code ", font=FONT), y), f"~/{a['dir']}", font=FONT, fill=DIM)
    ask, said = TALK.get(a["space"], ("", ""))
    y += 30
    d.text((x, y), "❯ " + ask, font=FONT, fill=DIM)
    y += 24
    d.text((x, y), "●", font=FONT, fill=CLAUDE)
    d.text((x + 16, y), said, font=FONT, fill=TEXT)
    y += 26
    if a["status"] == "working":
        spin = SPINNER[t // 120 % len(SPINNER)]
        d.text((x, y), f"{spin} Working… ({12 + t // 1000}s · esc to interrupt)", font=FONT, fill=CLAUDE)
    elif a["status"] == "blocked":
        d.rectangle([x, y - 2, box[2] - 14, y + 58], outline=(160, 90, 130))
        d.text((x + 10, y + 4), "Bash(npm run test:e2e)", font=FONT, fill=TEXT)
        d.text((x + 10, y + 22), "Do you want to proceed?", font=FONT, fill=TEXT)
        d.text((x + 10, y + 40), "❯ 1. Yes   2. No", font=FONT, fill=DOT["blocked"])
    elif a["status"] == "done":
        d.text((x, y), "●", font=FONT, fill=DONE_TEXT)
        d.text((x + 16, y), "Done. Tests pass.", font=FONT, fill=TEXT)

    # The prompt, and under it Claude Code's status line: what the panel shows is read from here.
    py = box[3] - 48
    d.line([box[0] + 8, py - 6, box[2] - 8, py - 6], fill=LINE)
    d.text((x, py), "❯", font=FONT, fill=TEXT)
    d.rectangle([x + 14, py + 1, x + 20, py + 14], fill=DIM)
    sy = box[3] - 22
    model = a["model"].capitalize()
    left = f"{model} · {a['effort'].lower()}   ctx "
    d.text((x, sy), left, font=SMALL, fill=DIM)
    bx = x + d.textlength(left, font=SMALL)
    pct = min(100, round(a["used"] * 100 / a["window"])) if a["window"] else 0
    d.rectangle([bx, sy + 4, bx + 80, sy + 9], fill=LINE)
    d.rectangle([bx, sy + 4, bx + 80 * pct / 100, sy + 9], fill=(120, 200, 140) if pct < 70 else (240, 170, 60))
    d.text((bx + 88, sy), f"{pct}% {tokens(a['used'])}/{tokens(a['window'])}", font=SMALL, fill=DIM)

    if state["cue"] == "keyboard":
        badge(d, PANEL_W - 16, 8, "keyboard")
    return img


def both(state, frame):
    img = canvas(PANEL_W, HERDR_H + GAP + PANEL_H, BG)
    img.paste(herdr(state), (0, 0))
    img.paste(panel(frame), (0, (HERDR_H + GAP) * S))
    if state["cue"] in ("knob", "button"):
        badge(Draw(img), PANEL_W - 16, HERDR_H + GAP + PANEL_H - 22, state["cue"])
    return img


def save(imgs, path):
    """One frame for every run of equal ones, shown for as long as the run lasts."""
    kept, durations = [], []
    for img in imgs:
        if kept and img.tobytes() == kept[-1].tobytes():
            durations[-1] += 40
        else:
            kept.append(img)
            durations.append(40)
    kept[0].save(path, save_all=True, append_images=kept[1:], duration=durations, loop=0, lossless=True, method=4)


def main(src, out):
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    for scene in sorted(pathlib.Path(src).glob("*.rgb")):
        fs = frames(scene)
        states = scene.with_suffix(".jsonl")
        if states.exists():
            imgs = [both(json.loads(line), f) for line, f in zip(states.read_text().splitlines(), fs)]
        else:
            imgs = [panel(f) for f in fs]
        save(imgs, out / (scene.stem + ".webp"))


if __name__ == "__main__":
    main(*sys.argv[1:3])

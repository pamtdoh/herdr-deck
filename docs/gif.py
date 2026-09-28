#!/usr/bin/env python3
"""The README's GIF, made from the simulator's frames:

    cargo run -p pixbar-sim -- --showcase /tmp/showcase.rgb
    python3 docs/gif.py /tmp/showcase.rgb docs/pixbar.gif

The simulator writes each frame as 52x16 RGB bytes, one every 40 ms, and beside them `showcase.rgb.scenes`: for
each scene the frame it starts on, its tab and its caption. The GIF shows the panel as it looks (round LEDs behind
a dark face, the unlit ones faintly there), the scenes as tabs along the top with the current one lit and filling
up as it plays, and its caption underneath. Needs Pillow (10.1 or later, for its built-in font).
"""
import sys

from PIL import Image, ImageDraw, ImageFont

W, H = 52, 16
LED = 11  # pixels per LED
PAD = 14  # face around the matrix
PANEL_W, PANEL_H = W * LED + 2 * PAD, H * LED + 2 * PAD
TABS_H, CAPTION_H = 38, 40
BG, FACE, UNLIT = (13, 13, 16), (22, 22, 26), (34, 34, 40)
TAB_ON, TAB_OFF, ACCENT, CAPTION = (240, 240, 235), (110, 110, 118), (120, 225, 170), (200, 200, 195)


def frames(path):
    data = open(path, "rb").read()
    size = W * H * 3
    return [data[i:i + size] for i in range(0, len(data), size)]


def scenes(path):
    rows = [line.rstrip("\n").split("\t") for line in open(path) if line.strip()]
    return [(int(start), tab, caption) for start, tab, caption in rows]


def fit(text, size, width):
    """The built-in font at `size`, or smaller until `text` fits in `width`."""
    while size > 8:
        font = ImageFont.load_default(size=size)
        if font.getlength(text) <= width:
            return font
        size -= 1
    return ImageFont.load_default(size=8)


def panel(frame):
    img = Image.new("RGB", (PANEL_W, PANEL_H), BG)
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, PANEL_W - 1, PANEL_H - 1], radius=PAD, fill=FACE)
    for y in range(H):
        for x in range(W):
            i = (y * W + x) * 3
            c = tuple(frame[i:i + 3])
            x0, y0 = PAD + x * LED, PAD + y * LED
            d.ellipse([x0 + 1, y0 + 1, x0 + LED - 2, y0 + LED - 2], fill=UNLIT if c == (0, 0, 0) else c)
    return img


def main(src, out):
    fs, sc = frames(src), scenes(src + ".scenes")
    ends = [start for start, _, _ in sc[1:]] + [len(fs)]
    tabs = [tab for _, tab, _ in sc]
    tab_font = fit("   ".join(tabs), 14, PANEL_W - 24)
    gap = (PANEL_W - sum(tab_font.getlength(t) for t in tabs)) / (len(tabs) + 1)
    xs, x = [], gap
    for t in tabs:
        xs.append(x)
        x += tab_font.getlength(t) + gap
    images = []
    for n, (start, _, caption) in enumerate(sc):
        caption_font = fit(caption, 15, PANEL_W - 24)
        for i in range(start, ends[n]):
            img = Image.new("RGB", (PANEL_W, TABS_H + PANEL_H + CAPTION_H), BG)
            d = ImageDraw.Draw(img)
            for k, t in enumerate(tabs):
                d.text((xs[k], 10), t, font=tab_font, fill=TAB_ON if k == n else TAB_OFF)
            # Under the tab that is on, a bar filling up as its scene plays.
            done = (i - start + 1) / (ends[n] - start)
            w = tab_font.getlength(tabs[n])
            d.rectangle([xs[n], 29, xs[n] + w, 30], fill=(45, 45, 52))
            d.rectangle([xs[n], 29, xs[n] + w * done, 30], fill=ACCENT)
            img.paste(panel(fs[i]), (0, TABS_H))
            d.text((PANEL_W / 2, TABS_H + PANEL_H + CAPTION_H / 2), caption, font=caption_font, fill=CAPTION, anchor="mm")
            images.append(img)
    images[0].save(out, save_all=True, append_images=images[1:], duration=40, loop=0, optimize=True)


if __name__ == "__main__":
    main(*sys.argv[1:3])

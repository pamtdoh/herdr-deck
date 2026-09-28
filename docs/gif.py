#!/usr/bin/env python3
"""The README's GIF, made from the simulator's frames:

    cargo run -p pixbar-sim -- --showcase /tmp/showcase.rgb
    python3 docs/gif.py /tmp/showcase.rgb docs/pixbar.gif

Each frame is 52x16 RGB bytes, one every 40 ms. Drawn as the panel looks: round LEDs behind a dark face, the
unlit ones faintly there. Needs Pillow.
"""
import sys

from PIL import Image, ImageDraw

W, H = 52, 16
LED = 10  # pixels per LED
PAD = 14  # face around the matrix
FACE, UNLIT = (14, 14, 16), (30, 30, 34)


def frames(path):
    data = open(path, "rb").read()
    size = W * H * 3
    return [data[i:i + size] for i in range(0, len(data), size)]


def panel(frame):
    img = Image.new("RGB", (W * LED + 2 * PAD, H * LED + 2 * PAD), (0, 0, 0))
    d = ImageDraw.Draw(img)
    d.rounded_rectangle([0, 0, img.width - 1, img.height - 1], radius=PAD, fill=FACE)
    for y in range(H):
        for x in range(W):
            i = (y * W + x) * 3
            c = tuple(frame[i:i + 3])
            x0, y0 = PAD + x * LED, PAD + y * LED
            d.ellipse([x0 + 1, y0 + 1, x0 + LED - 2, y0 + LED - 2], fill=UNLIT if c == (0, 0, 0) else c)
    return img


def main(src, out):
    imgs = [panel(f) for f in frames(src)]
    imgs[0].save(out, save_all=True, append_images=imgs[1:], duration=40, loop=0, optimize=True)


if __name__ == "__main__":
    main(*sys.argv[1:3])

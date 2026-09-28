#!/usr/bin/env python3
"""The README's GIFs, made from the simulator's frames:

    cargo run -p pixbar-sim -- --showcase /tmp/pixbar
    python3 docs/gif.py /tmp/pixbar docs/gifs

The simulator writes a file per feature, each frame 52x16 RGB bytes, one every 40 ms; this draws each as the panel
looks (round LEDs behind a dark face, the unlit ones faintly there; square, so that it sits as well on a light
page as on a dark one) and writes a GIF of the same name. Needs Pillow.
"""
import pathlib
import sys

from PIL import Image, ImageDraw

W, H = 52, 16
LED = 10  # pixels per LED
PAD = 14  # face around the matrix
FACE, UNLIT = (22, 22, 26), (34, 34, 40)


def frames(path):
    data = path.read_bytes()
    size = W * H * 3
    return [data[i:i + size] for i in range(0, len(data), size)]


def panel(frame):
    img = Image.new("RGB", (W * LED + 2 * PAD, H * LED + 2 * PAD), FACE)
    d = ImageDraw.Draw(img)
    for y in range(H):
        for x in range(W):
            i = (y * W + x) * 3
            c = tuple(frame[i:i + 3])
            x0, y0 = PAD + x * LED, PAD + y * LED
            d.ellipse([x0 + 1, y0 + 1, x0 + LED - 2, y0 + LED - 2], fill=UNLIT if c == (0, 0, 0) else c)
    return img


def main(src, out):
    out = pathlib.Path(out)
    out.mkdir(parents=True, exist_ok=True)
    for scene in sorted(pathlib.Path(src).glob("*.rgb")):
        imgs = [panel(f) for f in frames(scene)]
        imgs[0].save(out / (scene.stem + ".gif"), save_all=True, append_images=imgs[1:], duration=40, loop=0, optimize=True)


if __name__ == "__main__":
    main(*sys.argv[1:3])

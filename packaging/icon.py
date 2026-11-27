"""Draws BaseCraft's app icon (original artwork, generated in code).

In the visual spirit of the Craft apps (one saturated tile colour, a cream
object with a thick black outline, slightly tilted), but drawn from scratch:
a purple tile with a tilted stack of record cards, field rows on the top
card and one highlighted field.

usage: python packaging/icon.py   -> assets/icon/basecraft-{16..1024}.png, basecraft.ico
"""
from PIL import Image, ImageDraw

S = 1024           # master size
SS = 2             # supersampling for smooth edges
TILE = (112, 64, 170)       # purple tile
CREAM = (244, 236, 214)
CREAM_BACK = (226, 216, 190)
INK = (24, 22, 28)
ACCENT = (255, 150, 40)
W = 24 * SS        # outline width


def card(size, rows, back=False):
    """One record card on a transparent layer, drawn upright."""
    w, h = size
    layer = Image.new("RGBA", (w + 2 * W, h + 2 * W), (0, 0, 0, 0))
    d = ImageDraw.Draw(layer)
    box = [W, W, W + w, W + h]
    d.rounded_rectangle(box, radius=48 * SS, fill=CREAM_BACK if back else CREAM, outline=INK, width=W)
    if back:
        return layer
    x0, y0 = W + 70 * SS, W + 70 * SS
    # Title bar.
    d.rounded_rectangle([x0, y0, x0 + 300 * SS, y0 + 44 * SS], radius=18 * SS, fill=INK)
    # Field rows: label + value box.
    y = y0 + 120 * SS
    for i in range(rows):
        d.rounded_rectangle([x0, y + 8 * SS, x0 + 110 * SS, y + 36 * SS], radius=12 * SS, fill=INK)
        fill = ACCENT if i == rows - 1 else CREAM
        d.rounded_rectangle([x0 + 140 * SS, y, W + w - 70 * SS, y + 44 * SS], radius=14 * SS,
                            fill=fill, outline=INK, width=10 * SS)
        y += 92 * SS
    return layer


def master():
    n = S * SS
    img = Image.new("RGBA", (n, n), (0, 0, 0, 0))
    ImageDraw.Draw(img).rounded_rectangle([0, 0, n - 1, n - 1], radius=230 * SS, fill=TILE + (255,))
    size = (560 * SS, 560 * SS)
    # Two cards behind, then the front card, each a little more tilted.
    for angle, dx, dy, back in [(14, 90, -40, True), (7, 45, -20, True), (-6, 0, 10, False)]:
        layer = card(size, 3, back).rotate(angle, resample=Image.BICUBIC, expand=True)
        x = (n - layer.width) // 2 + dx * SS
        y = (n - layer.height) // 2 + dy * SS + 20 * SS
        img.alpha_composite(layer, (x, y))
    return img.resize((S, S), Image.LANCZOS)


def main():
    m = master()
    for s in [16, 24, 32, 48, 64, 128, 256, 512, 1024]:
        m.resize((s, s), Image.LANCZOS).save(f"assets/icon/basecraft-{s}.png")
    m.save("assets/icon/basecraft.ico", sizes=[(s, s) for s in [16, 24, 32, 48, 64, 128, 256]])


if __name__ == "__main__":
    main()

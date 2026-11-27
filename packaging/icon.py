"""Draws BaseCraft's app icon (original artwork, generated in code).

A rounded teal tile with a white record card: a header bar and three
field rows, the last one highlighted, suggesting a form over a table.

usage: python packaging/icon.py   -> assets/icon/basecraft-{16..1024}.png, basecraft.ico
"""
from PIL import Image, ImageDraw

S = 1024  # master size


def lerp(a, b, t):
    return tuple(int(x + (y - x) * t) for x, y in zip(a, b))


def master():
    img = Image.new("RGBA", (S, S), (0, 0, 0, 0))
    # Vertical gradient tile with rounded corners.
    grad = Image.new("RGBA", (S, S))
    top, bottom = (20, 168, 168), (11, 92, 112)
    gd = ImageDraw.Draw(grad)
    for y in range(S):
        gd.line([(0, y), (S, y)], fill=lerp(top, bottom, y / S) + (255,))
    mask = Image.new("L", (S, S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([40, 40, S - 40, S - 40], radius=210, fill=255)
    img.paste(grad, (0, 0), mask)
    d = ImageDraw.Draw(img)
    # Record card.
    card = [220, 240, S - 220, S - 230]
    d.rounded_rectangle(card, radius=56, fill=(255, 255, 255, 255))
    x0, y0, x1, _ = card
    d.rounded_rectangle([x0, y0, x1, y0 + 120], radius=56, fill=(232, 246, 246, 255))
    d.rectangle([x0, y0 + 64, x1, y0 + 120], fill=(232, 246, 246, 255))
    d.ellipse([x0 + 48, y0 + 36, x0 + 96, y0 + 84], fill=(20, 168, 168, 255))
    rows = [y0 + 175, y0 + 285, y0 + 395]
    for i, ry in enumerate(rows):
        d.rounded_rectangle([x0 + 48, ry, x0 + 150, ry + 40], radius=14, fill=(160, 196, 204, 255))
        color = (255, 176, 59, 255) if i == 2 else (204, 226, 230, 255)
        d.rounded_rectangle([x0 + 180, ry, x1 - 48, ry + 40], radius=14, fill=color)
    return img


def main():
    m = master()
    sizes = [16, 24, 32, 48, 64, 128, 256, 512, 1024]
    for s in sizes:
        m.resize((s, s), Image.LANCZOS).save(f"assets/icon/basecraft-{s}.png")
    m.save("assets/icon/basecraft.ico", sizes=[(s, s) for s in [16, 24, 32, 48, 64, 128, 256]])


if __name__ == "__main__":
    main()

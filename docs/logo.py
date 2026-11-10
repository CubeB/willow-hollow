"""Renders the Willow Hollow wordmark as PNGs (card and transparent variants).

usage: python docs/logo.py OUT_DIR
"""
import math
import os
import random
import sys

from PIL import Image, ImageDraw, ImageFont

OUT = sys.argv[1]
S = 4  # supersampling
W, H = 1600, 640
FONTS = "C:/Windows/Fonts/"

# Palette: deep moss for the words, sage and willow greens for leaves, pale water.
INK = (40, 68, 54)
TAGLINE = (88, 122, 94)
BARK = (86, 112, 78)
STEM = (120, 158, 112)
WILLOW = (156, 190, 120)
WILLOW_PALE = (186, 208, 142)
LEAF_DARK = (92, 134, 92)
WATER = (112, 162, 180)
WATER_LIGHT = (168, 204, 212)
CARD_TOP = (243, 247, 237)
CARD_BOTTOM = (224, 235, 221)

# Where the letters are, so that the strands can frame them without touching.
TITLE_LEFT, TITLE_RIGHT, TITLE_TOP = 200, 1400, 236


def bezier(p0, p1, p2, p3, n=160):
    pts = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        x = u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0]
        y = u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]
        pts.append((x, y))
    return pts


def taper(d, pts, start, end, color):
    # A line whose width runs from `start` to `end`, drawn as one outline.
    left, right = [], []
    last = len(pts) - 1
    for i, (x, y) in enumerate(pts):
        ax, ay = pts[max(i - 1, 0)]
        bx, by = pts[min(i + 1, last)]
        dx, dy = bx - ax, by - ay
        length = math.hypot(dx, dy) or 1.0
        half = (start + (end - start) * i / last) / 2
        nx, ny = -dy / length * half, dx / length * half
        left.append(((x + nx) * S, (y + ny) * S))
        right.append(((x - nx) * S, (y - ny) * S))
    d.polygon(left + right[::-1], fill=color)


def leaf(d, x, y, size, angle, color):
    # A small tilted square: a voxel seen edge-on, standing in for a willow leaf.
    c, s = math.cos(angle), math.sin(angle)
    corners = []
    for dx, dy in [(-1, -1), (1, -1), (1, 1), (-1, 1)]:
        corners.append(((x + (dx * c - dy * s) * size) * S, (y + (dx * s + dy * c) * size) * S))
    d.polygon(corners, fill=color)


def strand(d, rng, x0, y0, length, lean):
    # A drooping willow strand: falls from the branch and curls out toward the tip.
    p0 = (x0, y0)
    p1 = (x0 + lean * 0.1, y0 + length * 0.4)
    p2 = (x0 + lean * 0.55, y0 + length * 0.75)
    p3 = (x0 + lean, y0 + length)
    pts = bezier(p0, p1, p2, p3)
    taper(d, pts, 2.6, 0.9, STEM)
    # Leaves along the strand, alternating sides, smaller toward the tip.
    count = max(5, round(length / 15))
    side = rng.choice((-1, 1))
    for k in range(1, count + 1):
        t = k / count
        x, y = pts[int(t * (len(pts) - 1))]
        size = (7.6 - 3.8 * t) * rng.uniform(0.88, 1.12)
        angle = math.radians(45 + 26 * side + rng.uniform(-12, 12))
        color = rng.choice((WILLOW, WILLOW, WILLOW_PALE, LEAF_DARK, LEAF_DARK))
        reach = size * 0.9 + 1.0
        leaf(d, x + side * reach, y, size, angle, color)
        side = -side


def draw_willow(d):
    rng = random.Random(11)
    # One long bough arching from off the left edge across the words, thinning to a tip.
    bough = bezier((-30, 132), (400, 8), (1120, 20), (1536, 118))
    taper(d, bough, 15, 2.0, BARK)

    # Strands are long beside the title and short above it, leaving a hollow for the words.
    for k in range(21):
        t = 0.08 + 0.87 * (k + rng.uniform(-0.3, 0.3)) / 20
        x, y = bough[int(t * (len(bough) - 1))]
        beside = x < TITLE_LEFT - 34 or x > TITLE_RIGHT + 34
        if beside:
            length = rng.uniform(250, 340)
            lean = (-1 if x < W / 2 else 1) * rng.uniform(4, 22)
        else:
            # Clear of the tops of the letters, whatever the bough's height here.
            edge = abs(x - W / 2) / (W / 2)
            length = min(TITLE_TOP - y - 12, 78 + 88 * edge + rng.uniform(-14, 22))
            lean = rng.uniform(-13, 13)
        strand(d, rng, x, y, length, lean)


def draw_title(d):
    font = None
    for regular in ("georgia.ttf", "cambria.ttc"):
        try:
            font = ImageFont.truetype(FONTS + regular, int(168 * S))
            break
        except OSError:
            continue
    text = "Willow Hollow"
    spacing = 5 * S
    # Measured through the whole string so that kerning (the W and i) survives spacing.
    start = (W * S - d.textlength(text, font=font) - spacing * (len(text) - 1)) / 2
    for i, ch in enumerate(text):
        x = start + d.textlength(text[: i + 1], font=font) - d.textlength(ch, font=font) + spacing * i
        d.text((x, 372 * S), ch, font=font, fill=INK, anchor="ls")


def draw_water(d):
    # Rings spreading on a still pool from where one leaf has landed, thinning at the ends.
    cx, cy = 800, 446
    for k, (rx, ry, width, color) in enumerate(
        [(92, 13, 3.2, WATER), (232, 27, 2.8, WATER_LIGHT), (396, 41, 2.8, WATER), (566, 54, 2.2, WATER_LIGHT)]
    ):
        # Only the near side of each ring, like water seen at a low angle.
        n = 120
        sweep = math.radians(70 - k * 4)
        pts = []
        for i in range(n + 1):
            a = math.pi / 2 + sweep * (2 * i / n - 1)
            pts.append((cx + rx * math.cos(a), cy + ry * math.sin(a)))
        half = n // 2
        taper(d, pts[half::-1], width, 0.6, color)
        taper(d, pts[half:], width, 0.6, color)
    leaf(d, cx, cy - 4, 8.5, math.radians(62), WILLOW)


def draw_tagline(d):
    try:
        small = ImageFont.truetype(FONTS + "georgiai.ttf", int(38 * S))
    except OSError:
        small = ImageFont.truetype(FONTS + "cambriai.ttf", int(38 * S))
    tag = "a quiet world of rivers"
    spacing = 1.6 * S
    x = (W * S - d.textlength(tag, font=small) - spacing * (len(tag) - 1)) / 2
    for i, ch in enumerate(tag):
        offset = d.textlength(tag[: i + 1], font=small) - d.textlength(ch, font=small) + spacing * i
        d.text((x + offset, 572 * S), ch, font=small, fill=TAGLINE, anchor="ls")


def render(transparent):
    img = Image.new("RGBA", (W * S, H * S), (0, 0, 0, 0))
    art = Image.new("RGBA", (W * S, H * S), (0, 0, 0, 0))
    d = ImageDraw.Draw(art)
    draw_willow(d)
    draw_title(d)
    draw_water(d)
    draw_tagline(d)

    mask = Image.new("L", (W * S, H * S), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, W * S - 1, H * S - 1], radius=48 * S, fill=255)
    if not transparent:
        # Soft vertical gradient card with rounded corners.
        card = Image.new("RGBA", (W * S, H * S), (0, 0, 0, 0))
        cd = ImageDraw.Draw(card)
        for y in range(H * S):
            t = y / (H * S)
            col = tuple(int(a + (b - a) * t) for a, b in zip(CARD_TOP, CARD_BOTTOM)) + (255,)
            cd.line([(0, y), (W * S, y)], fill=col)
        img.paste(card, (0, 0), mask)
    # The bough runs off the edge, so the art is clipped to the card's shape too.
    img.paste(art, (0, 0), Image.composite(art.getchannel("A"), Image.new("L", art.size, 0), mask))

    return img.resize((W, H), Image.LANCZOS)


os.makedirs(OUT, exist_ok=True)
render(False).save(os.path.join(OUT, "logo.png"))
render(True).save(os.path.join(OUT, "logo_transparent.png"))
print("ok")

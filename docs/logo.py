"""Renders the Willow Hollow wordmark as PNGs (card and transparent variants).

usage: python docs/logo.py OUT_DIR
"""
import math
import os
import sys

from PIL import Image, ImageDraw, ImageFont

OUT = sys.argv[1]
S = 4  # supersampling
W, H = 1600, 640
FONTS = "C:/Windows/Fonts/"

# Palette: deep moss for the words, sage and willow greens for leaves, pale water.
INK = (46, 74, 58)
SAGE = (120, 158, 112)
WILLOW = (156, 190, 120)
LEAF_DARK = (92, 134, 92)
WATER = (122, 170, 186)
WATER_LIGHT = (176, 208, 214)
CARD_TOP = (242, 246, 236)
CARD_BOTTOM = (226, 236, 222)


def bezier(p0, p1, p2, p3, n=120):
    pts = []
    for i in range(n + 1):
        t = i / n
        u = 1 - t
        x = u**3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t**3 * p3[0]
        y = u**3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t**3 * p3[1]
        pts.append((x, y))
    return pts


def draw_curve(d, pts, color, width):
    d.line([(x * S, y * S) for x, y in pts], fill=color, width=int(width * S), joint="curve")


def leaf(d, x, y, size, angle, color):
    # A small tilted square: a voxel seen edge-on, standing in for a willow leaf.
    c, s = math.cos(angle), math.sin(angle)
    corners = []
    for dx, dy in [(-1, -1), (1, -1), (1, 1), (-1, 1)]:
        corners.append(((x + (dx * c - dy * s) * size) * S, (y + (dx * s + dy * c) * size) * S))
    d.polygon(corners, fill=color)


def strand(d, x0, y0, length, lean, phase, thickness=2.2):
    # A drooping willow strand: starts at the branch, falls and curls slightly.
    p0 = (x0, y0)
    p1 = (x0 + lean * 0.3, y0 + length * 0.35)
    p2 = (x0 + lean * 0.9, y0 + length * 0.75)
    p3 = (x0 + lean * 1.1, y0 + length)
    pts = bezier(p0, p1, p2, p3)
    draw_curve(d, pts, SAGE, thickness)
    # Leaves along the strand, alternating sides, smaller toward the tip.
    for i in range(6, len(pts) - 4, 9):
        x, y = pts[i]
        t = i / len(pts)
        side = 1 if (i // 9 + phase) % 2 == 0 else -1
        size = 6.5 * (1.0 - 0.45 * t)
        angle = math.radians(35 * side + 20 * t)
        color = WILLOW if (i // 9 + phase) % 3 else LEAF_DARK
        leaf(d, x + side * 5, y, size, angle, color)


def render(transparent):
    img = Image.new("RGBA", (W * S, H * S), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)

    if not transparent:
        # Soft vertical gradient card with rounded corners.
        card = Image.new("RGBA", (W * S, H * S), (0, 0, 0, 0))
        cd = ImageDraw.Draw(card)
        for y in range(H * S):
            t = y / (H * S)
            col = tuple(int(a + (b - a) * t) for a, b in zip(CARD_TOP, CARD_BOTTOM)) + (255,)
            cd.line([(0, y), (W * S, y)], fill=col)
        mask = Image.new("L", (W * S, H * S), 0)
        ImageDraw.Draw(mask).rounded_rectangle([0, 0, W * S - 1, H * S - 1], radius=48 * S, fill=255)
        img.paste(card, (0, 0), mask)
        d = ImageDraw.Draw(img)

    # The branch: one long bough arcing in from the upper left over the words.
    bough = bezier((60, 95), (420, 20), (900, 60), (1240, 150))
    draw_curve(d, bough, SAGE, 5.5)
    draw_curve(d, bezier((60, 95), (420, 20), (900, 60), (1240, 150)), LEAF_DARK, 2.0)
    # Strands hanging from the bough, longer toward the middle.
    # They stop clear of the letters, which sit lower.
    for k, t in enumerate([0.1, 0.19, 0.3, 0.41, 0.52, 0.63, 0.74, 0.86]):
        i = int(t * (len(bough) - 1))
        x, y = bough[i]
        length = 72 + 72 * math.sin(t * math.pi) + (18 if k % 3 == 0 else 0)
        lean = (-1 if k % 2 else 1) * (16 + 14 * math.sin(k * 1.3))
        strand(d, x, y, length, lean, k)

    # The words.
    try:
        font = ImageFont.truetype(FONTS + "georgia.ttf", int(168 * S))
        small = ImageFont.truetype(FONTS + "georgiai.ttf", int(40 * S))
    except OSError:
        font = ImageFont.truetype(FONTS + "cambria.ttc", int(168 * S))
        small = ImageFont.truetype(FONTS + "cambriai.ttf", int(40 * S))
    text = "Willow Hollow"
    # Measure to centre, with gentle letter spacing applied by drawing per character.
    spacing = 6 * S
    widths = [d.textlength(ch, font=font) for ch in text]
    total = sum(widths) + spacing * (len(text) - 1)
    x = (W * S - total) / 2
    baseline_y = 372 * S
    for ch, w in zip(text, widths):
        d.text((x, baseline_y), ch, font=font, fill=INK, anchor="ls")
        x += w + spacing

    # Ripples beneath: three soft arcs, like rings spreading on a still pool.
    for k, (cx, span, y) in enumerate([(800, 520, 430), (800, 360, 456), (800, 200, 480)]):
        pts = bezier((cx - span, y), (cx - span * 0.5, y + 16), (cx + span * 0.5, y + 16), (cx + span, y))
        draw_curve(d, pts, WATER if k != 1 else WATER_LIGHT, 4.0 - k * 0.6)

    tag = "a quiet world of rivers"
    tw = d.textlength(tag, font=small)
    d.text(((W * S - tw) / 2, 538 * S), tag, font=small, fill=SAGE)

    return img.resize((W, H), Image.LANCZOS)


os.makedirs(OUT, exist_ok=True)
render(False).save(os.path.join(OUT, "logo.png"))
render(True).save(os.path.join(OUT, "logo_transparent.png"))
print("ok")

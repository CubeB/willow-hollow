"""Compares the screenshots in two directories, as take.sh writes them.

Usage: python imgdiff.py BEFORE AFTER [--save DIR]

For each PNG in both: the largest difference in any channel, how many pixels differ at
all and by more than 1, and the mean difference per channel. With --save, an image of
where they differ (white = no difference, darker = more) is written for each pair that
does. Exits 1 if any pair differs by more than 1 in a channel, or a shot is missing.
"""
import glob
import os
import sys

from PIL import Image, ImageChops


def compare(a_path, b_path):
    a = Image.open(a_path).convert("RGB")
    b = Image.open(b_path).convert("RGB")
    if a.size != b.size:
        return None
    diff = ImageChops.difference(a, b)
    # Per pixel, the largest of its three channel differences.
    worst = diff.getchannel(0)
    for c in (1, 2):
        worst = ImageChops.lighter(worst, diff.getchannel(c))
    hist = worst.histogram()
    total = sum(hist)
    channel_sum = sum(sum(i * n for i, n in enumerate(diff.getchannel(c).histogram())) for c in range(3))
    return {
        "max": max((i for i, n in enumerate(hist) if n), default=0),
        "differ": total - hist[0],
        "over1": total - hist[0] - hist[1],
        "pixels": total,
        "mean": channel_sum / (3 * total),
        "map": worst,
    }


def main():
    args = sys.argv[1:]
    save = None
    if "--save" in args:
        i = args.index("--save")
        save = args[i + 1]
        del args[i : i + 2]
        os.makedirs(save, exist_ok=True)
    before, after = args
    bad = 0
    print(f"{'shot':<16} {'max':>4} {'pixels differ':>14} {'by >1':>10} {'mean':>8}")
    for a_path in sorted(glob.glob(os.path.join(before, "*.png"))):
        name = os.path.basename(a_path)
        b_path = os.path.join(after, name)
        if not os.path.exists(b_path):
            print(f"{name[:-4]:<16} missing in {after}")
            bad += 1
            continue
        r = compare(a_path, b_path)
        if r is None:
            print(f"{name[:-4]:<16} sizes differ")
            bad += 1
            continue
        share = 100.0 * r["differ"] / r["pixels"]
        print(f"{name[:-4]:<16} {r['max']:>4} {r['differ']:>8} ({share:4.1f}%) {r['over1']:>10} {r['mean']:>8.4f}")
        bad += r["max"] > 1
        if save and r["differ"]:
            r["map"].point(lambda v: 255 - min(255, v * 16)).save(os.path.join(save, name))
    sys.exit(1 if bad else 0)


if __name__ == "__main__":
    main()

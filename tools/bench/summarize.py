"""Summarise a bench.sh output directory: results.csv and a table on stdout.

Usage: python tools/bench/summarize.py OUTDIR

Per run, from the game's own --frames log: median/p95/p99/worst frame ms and the smoothed
GPU pass and CPU span numbers. From the per-frame CSV, if the binary wrote one: the mean,
1% low (mean of the slowest 1% of frames), p99 and worst of the frame time, of each GPU
pass and of each CPU span and sub-span, plus hitch counts and what the hitches were
spent on.

Hitches are counted two ways. `hitch33` counts frames over 33.3 ms. `hitch_dt` counts
frames more than DETREND_MS above the median of the DETREND_N frames around them: a
moving camera crosses heavier and lighter land, which moves the 1% low without any
hitch, and this count leaves that out.

Frames 0-120 (the game's own settle window) are left out, as the log does.
"""
import csv
import glob
import os
import re
import statistics as st
import sys

SETTLE = 120
WARM_S = 30.0
DETREND_N = 60
DETREND_MS = 8.0
# GPU spans that can hold the GPU waiting for the CPU, left out of the GPU work total.
IDLE_SPANS = ("gpu_gap", "gpu_upload", "gpu_pre")


def parse_log(path):
    r = {}
    try:
        text = open(path, encoding="utf-8", errors="replace").read()
    except OSError:
        return r
    m = re.search(r"frame ms: median ([\d.]+), p95 ([\d.]+), p99 ([\d.]+), worst ([\d.]+)", text)
    if m:
        r["log_median"], r["log_p95"], r["log_p99"], r["log_worst"] = map(float, m.groups())
    m = re.search(r"GPU ([\d.]+) ms: (.*)", text)
    if m:
        r["log_gpu_total"] = float(m.group(1))
        for name, v in re.findall(r"([a-z]+(?: \d)?) ([\d.]+)", m.group(2)):
            r["log_gpu_" + name.replace(" ", "")] = float(v)
    m = re.search(r"\] CPU: (.*)", text)
    if m:
        for name, v in re.findall(r"([a-z]+) ([\d.]+)", m.group(1)):
            r["log_cpu_" + name] = float(v)
    m = re.search(r"CPU worst: (.*)", text)
    if m:
        for name, v in re.findall(r"([a-z]+) ([\d.]+)", m.group(1)):
            r["log_cpuworst_" + name] = float(v)
    m = re.search(r"all terrain in view ([\d.]+) s", text)
    if m:
        r["log_loaded_s"] = float(m.group(1))
    m = re.search(r"after (\d+) frames: ([\d.]+) ms/frame \((\d+) fps\), (\d+) chunks, ([\d.]+) M quads", text)
    if m:
        r["log_frames"] = int(m.group(1))
        r["log_quads_M"] = float(m.group(5))
    r["log_panic"] = int("panicked" in text)
    return r


def q(sorted_vals, p):
    if not sorted_vals:
        return 0.0
    return sorted_vals[min(len(sorted_vals) - 1, int((len(sorted_vals) - 1) * p))]


def low1(vals):
    """Mean of the slowest 1% (at least one value)."""
    if not vals:
        return 0.0
    s = sorted(vals, reverse=True)
    k = max(1, len(s) // 100)
    return sum(s[:k]) / k


def detrended_hitches(ft):
    """For each frame, whether it is more than DETREND_MS above the median of the
    DETREND_N frames around it (fewer at the ends of the run)."""
    half = DETREND_N // 2
    out = []
    for i, v in enumerate(ft):
        around = sorted(ft[max(0, i - half) : i + half])
        out.append(v > around[len(around) // 2] + DETREND_MS)
    return out


def parse_csv(path):
    rows = list(csv.DictReader(open(path, newline="")))
    rows = [r for r in rows if int(r["frame"]) > SETTLE]
    if len(rows) < 10:
        return {}, {}
    # dt on a row is the time since the previous frame began, so it is the previous
    # frame's length: shift it back so each frame's work lines up with its time.
    frames = []
    for a, b in zip(rows, rows[1:]):
        f = dict(a)
        f["frame_ms"] = float(b["dt_ms"])
        frames.append(f)
    all_frames_list = frames
    # Over the whole run, so that a frame's neighbours are its neighbours in time even
    # when the measured window below leaves some out.
    for f, h in zip(frames, detrended_hitches([f["frame_ms"] for f in frames])):
        f["hitch_dt"] = h
    out = {}
    # Seconds since the first frame, for each frame.
    t = 0.0
    for f in frames:
        t += f["frame_ms"] / 1000
        f["t"] = t
    # Start-up: the first WARM_S seconds. The far view simulates the nine continents
    # round the camera then (about 25 s on one thread, each followed by a burst on every
    # core), and the crows' survey runs: this is where the hitches of arriving are.
    early = [f["frame_ms"] for f in frames if f["t"] < WARM_S]
    if early:
        out.update(start_frames=len(early), start_mean_ms=sum(early) / len(early), start_low1_ms=low1(early), start_worst_ms=max(early),
                   start_hitch33=sum(v > 33.3 for v in early), start_hitch50=sum(v > 50 for v in early),
                   start_hitch_dt=sum(f["hitch_dt"] for f in frames if f["t"] < WARM_S))
    all_ft = [f["frame_ms"] for f in frames]
    out.update(all_frames=len(all_ft), all_mean_ms=sum(all_ft) / len(all_ft), all_low1_ms=low1(all_ft), all_worst_ms=max(all_ft),
               all_hitch33=sum(v > 33.3 for v in all_ft), all_hitch50=sum(v > 50 for v in all_ft),
               all_hitch_dt=sum(f["hitch_dt"] for f in frames))
    # The measured window. A static scene: once WARM_S has passed and nothing is queued
    # or being built (if 500 frames of that remain, else once loaded). A moving camera:
    # the whole run, since arriving in new country is what it measures.
    moved = abs(float(frames[-1]["x"]) - float(frames[0]["x"])) + abs(float(frames[-1]["z"]) - float(frames[0]["z"]))
    steady = [f for f in frames if int(f["queued"]) == 0 and int(f["in_flight"]) == 0]
    warm = [f for f in steady if f["t"] >= WARM_S]
    if moved >= 100:
        out["window"] = "all"
    elif len(warm) >= 500:
        frames = warm
        out["window"] = "warm"
    elif len(steady) >= 500:
        frames = steady
        out["window"] = "steady"
    else:
        out["window"] = "all"
    ft = [f["frame_ms"] for f in frames]
    s = sorted(ft)
    mean = sum(ft) / len(ft)
    out.update(
        frames=len(ft), mean_ms=mean, fps=1000 / mean, p50_ms=q(s, 0.5), p99_ms=q(s, 0.99), low1_ms=low1(ft),
        low1_fps=1000 / low1(ft), worst_ms=s[-1], hitch33=sum(v > 33.3 for v in ft), hitch50=sum(v > 50 for v in ft),
        hitch_dt=sum(f["hitch_dt"] for f in frames),
    )
    cols = [c for c in frames[0].keys() if c not in ("frame_ms", "t", "hitch_dt")]
    gcols = [c for c in cols if c.startswith("gpu_") and c != "gpu_fresh"]
    fresh = [f for f in frames if f["gpu_fresh"] == "1"]
    for c in gcols:
        v = [float(f[c]) for f in fresh]
        if v:
            out[c + "_mean"] = sum(v) / len(v)
            out[c + "_low1"] = low1(v)
            out[c + "_max"] = max(v)
    if fresh:
        # Rendering work only. The gap is the GPU idling while the CPU waits for the
        # window; upload and pre come before the frame's passes, and hold the copies of
        # what the CPU wrote (arena growth among them) but also any wait for the CPU.
        work = [sum(float(f[c]) for c in gcols if c not in IDLE_SPANS) for f in fresh]
        out["gpu_work_mean"] = sum(work) / len(work)
        out["gpu_work_low1"] = low1(work)
    for c in [c for c in cols if c.startswith("cpu_") or c.startswith("sub_")] + [c for c in ("rebuild_ms", "upload_ms", "grown_bytes") if c in cols]:
        v = [float(f[c]) for f in frames]
        out[c + "_mean"] = sum(v) / len(v)
        out[c + "_low1"] = low1(v)
        out[c + "_max"] = max(v)
    for c in ["chunks_drawn", "quads_drawn", "particles", "props", "shadow1_draws", "shadow2_draws", "shadow3_draws", "impostor_boxes", "far_tiles"]:
        v = [float(f[c]) for f in frames]
        out[c + "_mean"] = sum(v) / len(v)
    # What the hitches were spent on: frames over twice the median, the main-thread
    # sub-span that took longest in each.
    subs = [c for c in cols if c.startswith("sub_")]
    blame = {}
    for f in all_frames_list:
        if f["frame_ms"] > 2 * out["p50_ms"] and f["frame_ms"] > 25:
            top = max(subs, key=lambda c: float(f[c]))
            blame[top] = blame.get(top, 0) + 1
    out["hitch_blame"] = " ".join(f"{k[4:]}:{v}" for k, v in sorted(blame.items(), key=lambda kv: -kv[1]))
    return out, frames


def main():
    outdir = sys.argv[1]
    runs = []
    for log in sorted(glob.glob(os.path.join(outdir, "*_r*.log"))):
        base = os.path.basename(log)[:-4]
        scene, rep = base.rsplit("_r", 1)
        r = {"scene": scene, "rep": int(rep)}
        r.update(parse_log(log))
        csvp = log[:-4] + ".csv"
        if os.path.exists(csvp) and os.path.getsize(csvp) > 0:
            summary, _ = parse_csv(csvp)
            r.update(summary)
        runs.append(r)
    if not runs:
        print("no runs found")
        return
    keys = []
    for r in runs:
        for k in r:
            if k not in keys:
                keys.append(k)
    with open(os.path.join(outdir, "results.csv"), "w", newline="") as fh:
        w = csv.DictWriter(fh, fieldnames=keys)
        w.writeheader()
        for r in runs:
            w.writerow({k: (round(v, 3) if isinstance(v, float) else v) for k, v in r.items()})

    def med(rs, k):
        v = [r[k] for r in rs if isinstance(r.get(k), (int, float))]
        return st.median(v) if v else float("nan")

    def spread(rs, k):
        v = [r[k] for r in rs if isinstance(r.get(k), (int, float))]
        return (max(v) - min(v)) if len(v) > 1 else 0.0

    scenes = []
    for r in runs:
        if r["scene"] not in scenes:
            scenes.append(r["scene"])
    has_csv = any("mean_ms" in r for r in runs)
    print()
    if has_csv:
        print("Static scenes measured from 30 s after the first frame, once loaded (win=warm); S* columns = the first 30 s (start-up, loading, far view, crows survey). " "Median of reps. ms = mean frame ms (+- spread across reps); 1%low = mean of slowest 1% frames; dt8 = frames 8 ms over the median of the 60 around them; "
              "GPU = summed pass means, gap/upload/pre excluded; upl = mean (max) of the GPU's copies before the frame's passes, arena growth among them")
        print(f"{'scene':<12} {'fps':>6} {'ms':>6} {'+-':>5} {'p99':>6} {'1%low':>6} {'worst':>7} {'>33ms':>5} {'dt8':>4} {'win':>6} {'S1%low':>6} {'Sworst':>6} {'S>33':>4} {'GPU':>6} {'terr':>5} {'shad':>5} {'far':>5} {'imp':>5} {'sky':>5} {'water':>5} {'part':>5} {'bloom':>5} {'CPUmain':>7} {'life':>5} {'strm':>5} {'rend':>5} {'ui':>5} {'upl':>12}")
        for sc in scenes:
            rs = [r for r in runs if r["scene"] == sc]
            shad = med(rs, "gpu_shadow1_mean") + med(rs, "gpu_shadow2_mean") + med(rs, "gpu_shadow3_mean")
            cpu_main = sum(med(rs, f"cpu_{n}_mean") for n in ["sim", "stream", "life", "render", "ui", "submit"])
            print(
                f"{sc:<12} {med(rs,'fps'):6.1f} {med(rs,'mean_ms'):6.2f} {spread(rs,'mean_ms'):5.2f} {med(rs,'p99_ms'):6.1f} {med(rs,'low1_ms'):6.1f} {med(rs,'worst_ms'):7.1f} {med(rs,'hitch33'):5.0f} {med(rs,'hitch_dt'):4.0f} {rs[0].get('window','-'):>6} {med(rs,'start_low1_ms'):6.1f} {med(rs,'start_worst_ms'):6.1f} {med(rs,'start_hitch33'):4.0f} "
                f"{med(rs,'gpu_work_mean'):6.2f} {med(rs,'gpu_terrain_mean'):5.2f} {shad:5.2f} {med(rs,'gpu_far_mean'):5.2f} {med(rs,'gpu_impostors_mean'):5.2f} {med(rs,'gpu_sky_mean'):5.2f} "
                f"{med(rs,'gpu_water_mean'):5.2f} {med(rs,'gpu_particles_mean'):5.2f} {med(rs,'gpu_bloom_mean'):5.2f} {cpu_main:7.2f} {med(rs,'cpu_life_mean'):5.2f} {med(rs,'cpu_stream_mean'):5.2f} "
                f"{med(rs,'cpu_render_mean'):5.2f} {med(rs,'cpu_ui_mean'):5.2f} {med(rs,'gpu_upload_mean'):5.2f} ({med(rs,'gpu_upload_max'):5.1f})"
            )
    else:
        print("Median of reps, from the game's --frames log (stock binary: no per-frame CSV)")
        print(f"{'scene':<12} {'median':>7} {'+-':>5} {'p95':>6} {'p99':>6} {'worst':>7} {'GPU':>6} {'terrain':>7} {'sky':>5} {'water':>5} {'far':>5} {'shadows':>7} {'CPU life':>8} {'stream':>6} {'render':>6}")
        for sc in scenes:
            rs = [r for r in runs if r["scene"] == sc]
            shad = med(rs, "log_gpu_shadow1") + med(rs, "log_gpu_shadow2") + med(rs, "log_gpu_shadow3")
            print(
                f"{sc:<12} {med(rs,'log_median'):7.2f} {spread(rs,'log_median'):5.2f} {med(rs,'log_p95'):6.1f} {med(rs,'log_p99'):6.1f} {med(rs,'log_worst'):7.1f} {med(rs,'log_gpu_total'):6.2f} "
                f"{med(rs,'log_gpu_terrain'):7.2f} {med(rs,'log_gpu_sky'):5.2f} {med(rs,'log_gpu_water'):5.2f} {med(rs,'log_gpu_far'):5.2f} {shad:7.2f} {med(rs,'log_cpu_life'):8.2f} {med(rs,'log_cpu_stream'):6.2f} {med(rs,'log_cpu_render'):6.2f}"
            )
    print(f"\nper-run rows: {os.path.join(outdir, 'results.csv')}")


if __name__ == "__main__":
    main()

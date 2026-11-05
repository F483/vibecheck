"""Figures for docs/key-energy.md.

    uv run --with matplotlib python scripts/plot_key_energy.py ~/tmp/giantsteps-key docs/img/key-energy

Key accuracy rows are the measured GiantSteps results (see the doc); energy
level counts are fit_energy_scale.py's output. Per-genre energy is computed
from the GiantSteps audio (a few minutes) and cached in the dataset folder.
"""
import json, re, sys
from collections import defaultdict
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np, matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
from vibecheck import keyenergy


SURF, INK, INK2, GRID, GRAY = "#fcfcfb", "#0b0b0b", "#52514e", "#e4e3df", "#b9b8b3"
BLUE, BLUE_LT = "#2a78d6", "#9ec5f4"
RAMP = ["#86b6ef", "#3987e5", "#256abf", "#184f95", "#0d366b"]  # energy <=4, 5, 6, 7, >=8
plt.rcParams.update({"figure.facecolor": SURF, "axes.facecolor": SURF, "savefig.facecolor": SURF, "font.size": 11,
    "axes.edgecolor": GRID, "axes.labelcolor": INK2, "xtick.color": INK2, "ytick.color": INK2, "axes.grid": True,
    "axes.axisbelow": True, "grid.color": GRID, "grid.linewidth": 0.8, "axes.spines.top": False,
    "axes.spines.right": False, "axes.titlecolor": INK, "axes.titlesize": 13, "axes.titleweight": "bold",
    "axes.titlelocation": "left"})


def save(fig, name):
    fig.tight_layout(); fig.savefig(OUT / f"{name}.png", dpi=144); plt.close(fig)


def gs_energy():
    CACHE = GS / f".energy-{keyenergy.version().replace('/', '-')[:16]}.json"
    if CACHE.exists():
        return json.loads(CACHE.read_text())
    genre = {}
    for f in (GS / "meta").glob("*.json"):
        m = re.search(r'"genres":\s*\[\s*\{[^}]*?"name":\s*"([^"]+)"', f.read_text(errors="replace"))
        if m:
            genre[f.stem] = m.group(1)
    files = sorted(p for p in (GS / "audio").glob("*.mp3") if p.stem in genre)
    with ProcessPoolExecutor(8) as ex:
        out = list(ex.map(keyenergy.analyse_file, map(str, files), chunksize=4))
    rows = [[genre[p.stem], r[0].energy] for p, r in zip(files, out) if r]
    CACHE.write_text(json.dumps(rows))
    return rows


def key_accuracy():
    # GiantSteps, 567 tracks; Essentia rows measured the same way in the same run
    rows = [("this method", 78.1, 89.9), ("Essentia, bgate profile", 78.5, 89.8),
            ("Essentia, edma profile", 77.1, 86.4), ("Essentia, Krumhansl", 66.0, 79.4),
            ("Essentia, Temperley", 46.0, 77.8)][::-1]
    fig, ax = plt.subplots(figsize=(8, 4))
    y = np.arange(len(rows))
    for i, (name, ex, mix) in enumerate(rows):
        ours = name == "this method"
        dark, light = (BLUE, BLUE_LT) if ours else ("#6f6e69", "#d3d2cd")
        ax.barh(i, ex, color=dark, height=.62, edgecolor=SURF, linewidth=2)
        ax.barh(i, mix - ex, left=ex, color=light, height=.62, edgecolor=SURF, linewidth=2)
        w = "bold" if ours else "normal"
        ax.text(ex - 1.2, i, f"{ex:.0f}%", ha="right", va="center", color="white", fontsize=10, fontweight=w)
        ax.text(mix + 1, i, f"{mix:.0f}%", ha="left", va="center", color=INK, fontsize=10, fontweight=w)
    ax.set_yticks(y, [r[0] for r in rows])
    for t in ax.get_yticklabels():
        if t.get_text() == "this method":
            t.set_fontweight("bold"); t.set_color(INK)
    ax.set_xlim(0, 100); ax.grid(axis="y", visible=False)
    ax.xaxis.set_major_formatter(matplotlib.ticker.PercentFormatter(decimals=0))
    ax.tick_params(axis="y", length=0)
    from matplotlib.patches import Patch
    ax.legend(handles=[Patch(color=BLUE, label="exact key"), Patch(color=BLUE_LT, label="+ a mixable neighbour (fifth or relative)")],
              loc="upper center", bbox_to_anchor=(.42, -.2), ncol=2, frameon=False, fontsize=9)
    ax.set_title("Key: right 4 times in 5, mixable 9 in 10")
    ax.set_xlabel("GiantSteps Key Dataset, 567 tracks with human key labels")
    save(fig, "key-accuracy")


def templates():
    t = keyenergy.templates()
    deg = ["1", "♭2", "2", "♭3", "3", "4", "♭5", "5", "♭6", "6", "♭7", "7"]
    fig, axs = plt.subplots(1, 2, figsize=(9, 3.4), sharey=True)
    marks = {"minor": {0: "tonic", 3: "minor\nthird", 7: "fifth", 10: "minor\nseventh"},
             "major": {0: "tonic", 4: "major\nthird", 7: "fifth"}}
    for ax, mode in zip(axs, ("minor", "major")):
        w = np.array(t[mode])
        ax.bar(range(12), w, color=[BLUE if v > 0 else GRAY for v in w], width=.7, edgecolor=SURF, linewidth=2)
        ax.axhline(0, color=INK2, lw=.8)
        for k, lab in marks[mode].items():
            ax.text(k, w[k] + .15, lab, ha="center", va="bottom", fontsize=8.5, color=INK, linespacing=.95)
        ax.set_xticks(range(12), deg); ax.grid(axis="x", visible=False); ax.tick_params(axis="x", length=0)
        ax.set_title(f"{mode} keys", fontsize=11); ax.set_ylim(-2, 4.4)
    axs[0].set_ylabel("learned weight")
    fig.suptitle("What the key templates learned: tonic and fifth first, then the third",
                 x=.012, ha="left", fontsize=13, fontweight="bold", color=INK)
    save(fig, "key-templates")


def energy_scale():
    counts = {1: 7, 2: 25, 3: 60, 4: 136, 5: 501, 6: 1216, 7: 629, 8: 26, 9: 0, 10: 0}  # fit_energy_scale.py
    n = sum(counts.values()); lv = list(counts); p = np.array([counts[k] / n * 100 for k in lv])
    fig, ax = plt.subplots(figsize=(8, 3.6))
    ax.bar(lv, p, color=[RAMP[min(max(k - 4, 0), 4)] for k in lv], width=.7, edgecolor=SURF, linewidth=2)
    for k, v in zip(lv, p):
        ax.text(k, v + 1, f"{v:.0f}%" if v >= 1 else ("<1%" if v > 0 else "–"), ha="center", va="bottom", fontsize=9.5, color=INK)
    ax.set_xticks(lv); ax.grid(axis="x", visible=False); ax.tick_params(axis="x", length=0)
    ax.set_ylim(0, 54); ax.yaxis.set_major_formatter(matplotlib.ticker.PercentFormatter(decimals=0))
    ax.set_xlabel(f"energy level  ·  {n:,} reference tracks")
    ax.set_title("Energy: 6 is ordinary, one step is one standard deviation")
    save(fig, "energy-scale")


def energy_genres(rows):
    by = defaultdict(list)
    for g, e in rows:
        by[g].append(e)
    by = {g: np.array(v) for g, v in by.items() if len(v) >= 15}
    order = sorted(by, key=lambda g: by[g].mean())
    bins = [(-1, 4, "≤4"), (5, 5, "5"), (6, 6, "6"), (7, 7, "7"), (8, 99, "≥8")]
    fig, ax = plt.subplots(figsize=(8, 4.6))
    for i, g in enumerate(order):
        v, left = by[g], 0.0
        for j, (lo, hi, _) in enumerate(bins):
            w = np.mean((v >= lo) & (v <= hi)) * 100
            ax.barh(i, w, left=left, color=RAMP[j], height=.68, edgecolor=SURF, linewidth=2)
            left += w
        ax.text(101.5, i, f"{v.mean():.1f}", va="center", fontsize=9.5, color=INK)
    ax.text(101.5, len(order) - .45, "mean", va="bottom", fontsize=9, color=INK2)
    ax.set_ylim(-.6, len(order) + .15)
    ax.set_yticks(range(len(order)), [f"{g}  (n={len(by[g])})" for g in order]); ax.tick_params(axis="y", length=0)
    ax.set_xlim(0, 100); ax.grid(visible=False)
    ax.xaxis.set_major_formatter(matplotlib.ticker.PercentFormatter(decimals=0))
    from matplotlib.patches import Patch
    ax.legend(handles=[Patch(color=RAMP[j], label=b[2]) for j, b in enumerate(bins)], title="energy", ncol=5,
              loc="upper center", bbox_to_anchor=(.5, -.1), frameon=False, fontsize=9, title_fontsize=9,
              handlelength=1.2, columnspacing=1)
    fig.suptitle("Every genre spans the scale; calmer genres sit lower", x=.012, ha="left",
                 fontsize=13, fontweight="bold", color=INK)
    save(fig, "energy-genres")


if __name__ == "__main__":
    GS, OUT = Path(sys.argv[1]).expanduser(), Path(sys.argv[2])
    OUT.mkdir(parents=True, exist_ok=True)
    key_accuracy(); templates(); energy_scale(); energy_genres(gs_energy())

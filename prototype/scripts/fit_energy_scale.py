"""Compute the energy scale constants for keyenergy.py from reference audio.

    uv run python scripts/fit_energy_scale.py --collection ~/Music/Collection \\
        --sample 2000 --extra ~/tmp/giantsteps-key/audio

No ratings, labels or third-party analysis are read -- only audio. The output
is the mean and spread of each descriptor (loudness, brightness, noisiness)
over the reference tracks, and the median and spread of their summed z-score,
which places a typical (median) track at ENERGY_TYPICAL and one standard
deviation at one step. Paste the printed constants into keyenergy.py.
"""

from __future__ import annotations

import argparse
import os
import random
import sqlite3
import sys
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np

from vibecheck import audio, keyenergy


def features(path: str):
    try:
        windows = audio.excerpts(path, keyenergy.PRE)
        spectra = [keyenergy._spectra(x.astype(np.float64))
                   for x in windows if len(x) >= keyenergy.N_FFT]
        return keyenergy.energy_features(spectra)
    except Exception:
        return None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--collection", type=Path, required=True)
    ap.add_argument("--sample", type=int, default=2000)
    ap.add_argument("--extra", type=Path, action="append", default=[],
                    help="a folder of mp3s to add to the reference set")
    ap.add_argument("--seed", type=int, default=0)
    args = ap.parse_args()

    db = sqlite3.connect(f"file:{args.collection / '.vibecheck' / 'labels.db'}?mode=ro",
                         uri=True)
    live = sorted(p for (p,) in db.execute(
        "SELECT path FROM tracks WHERE missing_at IS NULL"))
    random.seed(args.seed)
    paths = [str(args.collection / p)
             for p in random.sample(live, min(args.sample, len(live)))]
    for d in args.extra:
        paths += sorted(str(p) for p in d.glob("*.mp3") if not p.name.startswith("._"))
    print(f"{len(paths)} reference tracks", flush=True)

    with ProcessPoolExecutor(min(8, os.cpu_count() or 1)) as ex:
        F = [f for f in ex.map(features, paths, chunksize=4) if f is not None]
    F = np.array(F)
    mean, std = F.mean(0), F.std(0)
    z = ((F - mean) / std).sum(1)
    mid, spread = float(np.median(z)), float(np.std(z))     # typical = median
    print(f"{len(F)} analysed\n")
    print(f"ENERGY_MEAN = ({mean[0]:.3f}, {mean[1]:.1f}, {mean[2]:.4f})"
          "    # loudness dB, centroid Hz, log flatness")
    print(f"ENERGY_STD = ({std[0]:.3f}, {std[1]:.1f}, {std[2]:.4f})")
    print(f"ENERGY_MID = {mid:.4f}")
    print(f"ENERGY_SPREAD = {spread:.4f}")
    levels = np.clip(np.round(keyenergy.ENERGY_TYPICAL + (z - mid) / spread), 1, 10)
    counts = {int(e): int((levels == e).sum()) for e in range(1, 11)}
    print("\nresulting spread on 1-10: "
          + "  ".join(f"{e}:{n}" for e, n in counts.items() if n))
    return 0


if __name__ == "__main__":
    sys.exit(main())

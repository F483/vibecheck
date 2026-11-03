"""Consistency checks for key and energy that need nobody's ratings.

    uv run python scripts/check_key_energy.py --collection ~/Music/Collection \\
        [--sample 100] [--genres ~/tmp/giantsteps-key]

1. **Encoding.** The same track re-encoded at 128 kbps should get the same key
   and the same energy: an answer that moves with the bitrate is noise.
2. **Genres.** With --genres (the GiantSteps key dataset, which carries
   Beatport genres), mean energy per genre, lowest first: a sanity check that
   the scale runs the way people would expect, not a measure of accuracy.
"""

from __future__ import annotations

import argparse
import os
import random
import re
import sqlite3
import subprocess
import sys
import tempfile
from collections import defaultdict
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np

from vibecheck import keyenergy


def reencoded(pair: tuple[str, str]):
    src, dst = pair
    p = subprocess.run(["ffmpeg", "-v", "quiet", "-y", "-i", src, "-map", "0:a",
                        "-b:a", "128k", dst], capture_output=True)
    if p.returncode != 0:
        return None
    a, b = keyenergy.analyse_file(src), keyenergy.analyse_file(dst)
    os.unlink(dst)
    return (a[0], b[0]) if a and b else None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--collection", type=Path, required=True)
    ap.add_argument("--sample", type=int, default=100)
    ap.add_argument("--genres", type=Path)
    args = ap.parse_args()
    workers = min(8, os.cpu_count() or 1)

    db = sqlite3.connect(f"file:{args.collection / '.vibecheck' / 'labels.db'}?mode=ro",
                         uri=True)
    live = sorted(p for (p,) in db.execute(
        "SELECT path FROM tracks WHERE missing_at IS NULL"))
    random.seed(1)
    picks = random.sample(live, min(args.sample, len(live)))
    with tempfile.TemporaryDirectory() as tmp:
        pairs = [(str(args.collection / p), str(Path(tmp) / f"{i}.mp3"))
                 for i, p in enumerate(picks)]
        with ProcessPoolExecutor(workers) as ex:
            res = [r for r in ex.map(reencoded, pairs) if r]
    same_key = sum(a.key == b.key for a, b in res)
    diff = np.array([abs(a.energy - b.energy) for a, b in res])
    print(f"1. re-encoded at 128 kbps, {len(res)} tracks:")
    print(f"   same key     {same_key / len(res):.1%}")
    print(f"   same energy  {np.mean(diff == 0):.1%}, within 1: {np.mean(diff <= 1):.1%}")

    if args.genres:
        genre = {}
        for f in (args.genres / "meta").glob("*.json"):
            m = re.search(r'"genres":\s*\[\s*\{[^}]*?"name":\s*"([^"]+)"',
                          f.read_text(encoding="utf-8", errors="replace"))
            if m:
                genre[f.stem] = m.group(1)
        files = sorted(p for p in (args.genres / "audio").glob("*.mp3")
                       if p.stem in genre)
        with ProcessPoolExecutor(workers) as ex:
            out = list(ex.map(keyenergy.analyse_file, map(str, files), chunksize=4))
        by = defaultdict(list)
        for p, r in zip(files, out):
            if r:
                by[genre[p.stem]].append(r[0].energy)
        print("\n2. mean energy by genre (GiantSteps), lowest first:")
        for g, v in sorted(by.items(), key=lambda kv: np.mean(kv[1])):
            if len(v) >= 15:
                print(f"   {np.mean(v):4.1f}  {g} (n={len(v)})")
    return 0


if __name__ == "__main__":
    sys.exit(main())

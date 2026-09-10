"""Progress and result of the blind relabel.

Reads the current genre tag straight from the files (the user relabels in their
DJ software, which writes tags), and compares against the labels those tracks
carried before they were cleared.

The agreement rate is the ceiling: no model trained on these labels can beat
the rate at which the labeller agrees with themselves.
"""

from __future__ import annotations

import csv
import sys
from collections import Counter, defaultdict
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, "src")
from vibecheck.index import read_genre  # noqa: E402

ROOT = Path.home() / "Music" / "Collection"


def main() -> int:
    playlist = [l.strip() for l in
                Path("out/relabel_blind.m3u8").read_text().splitlines()
                if l.startswith("/")]
    rel = [str(Path(p).relative_to(ROOT)) for p in playlist]
    old = {r["path"]: r["label"] for r in
           csv.DictReader(open("backup/labels_original.csv"))}

    with ThreadPoolExecutor(12) as ex:
        now = list(ex.map(lambda p: read_genre(ROOT / p), rel))

    done = [(p, o, n) for p, n in zip(rel, now)
            if n and (o := old.get(p)) is not None]
    print(f"relabelled so far: {len(done)}/{len(rel)}")
    if len(done) < 20:
        print("(need ~20+ before the numbers mean anything)")
        return 0

    oc = [o.split("_")[0] for _, o, _ in done]
    nc = [n.split("_")[0] for _, _, n in done]
    ol = [o.split("_")[1] if "_" in o else "?" for _, o, _ in done]
    nl = [n.split("_")[1] if "_" in n else "?" for _, _, n in done]

    ca = sum(a == b for a, b in zip(oc, nc)) / len(done)
    la = sum(a == b for a, b in zip(ol, nl)) / len(done)
    both = sum(a == b and c == d for a, b, c, d in zip(oc, nc, ol, nl)) / len(done)
    print()
    print(f"  colour agreement with your past self : {ca*100:5.1f}%")
    print(f"  level  agreement                     : {la*100:5.1f}%")
    print(f"  both                                 : {both*100:5.1f}%")
    print()
    print(f"  model accuracy for reference         :  56.0% colour, 80.1% tone")
    print(f"  -> headroom above the model          : {(ca-0.56)*100:+5.1f} points")

    swaps = Counter((a, b) for a, b in zip(oc, nc) if a != b)
    if swaps:
        print("\n  most common colour changes (old -> new):")
        for (a, b), n in swaps.most_common(8):
            print(f"    {a:8} -> {b:8}  {n}")

    bylevel = defaultdict(list)
    for (_, o, _), a, b in zip(done, oc, nc):
        bylevel[o.split("_")[1] if "_" in o else "?"].append(a == b)
    print("\n  colour agreement by the track's ORIGINAL level:")
    for L in sorted(bylevel):
        v = bylevel[L]
        print(f"    {L}: n={len(v):3d}  {100*sum(v)/len(v):5.1f}%")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

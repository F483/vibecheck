"""Deterministic stratified subset for backend comparison (design.md 8, step 3).

Every backend must be measured on the *same* tracks, or the comparison is
meaningless. Selection is by content hash, so it is reproducible and does not
drift as the library changes.

Rare labels are taken whole: at 412x imbalance, proportional sampling alone
would leave labels like Blue_A (5 tracks) with nothing on either side of the
split.
"""

from __future__ import annotations

import math
from collections import defaultdict
from pathlib import Path

from vibecheck import store

TAKE_ALL_BELOW = 40


def subset(root: Path, target: int = 2500) -> list[str]:
    lab = store.labels_db(root)
    labels = store.current_labels(lab, source="user")
    hash_by_path = dict(lab.execute("SELECT path, hash FROM tracks"))

    by_label: dict[str, list[str]] = defaultdict(list)
    for p, l in labels.items():
        by_label[l].append(p)

    small = {l: ps for l, ps in by_label.items() if len(ps) <= TAKE_ALL_BELOW}
    big = {l: ps for l, ps in by_label.items() if len(ps) > TAKE_ALL_BELOW}
    remaining = max(target - sum(len(ps) for ps in small.values()), 0)
    big_total = sum(len(ps) for ps in big.values())

    out: list[str] = [p for ps in small.values() for p in ps]
    for l, ps in big.items():
        k = max(int(math.ceil(len(ps) / big_total * remaining)), TAKE_ALL_BELOW)
        out += sorted(ps, key=lambda p: hash_by_path[p])[:k]
    return sorted(out)


if __name__ == "__main__":
    import sys
    from collections import Counter

    root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / "Music" / "Collection"
    ps = subset(root)
    lab = store.labels_db(root)
    labels = store.current_labels(lab, source="user")
    c = Counter(labels[p] for p in ps)
    print(f"subset: {len(ps)} tracks, {len(c)} labels")
    for l, n in sorted(c.items(), key=lambda kv: -kv[1]):
        print(f"  {l:<10} {n:4d}  (of {sum(1 for v in labels.values() if v == l)})")

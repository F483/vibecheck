"""Phase 0 deliverable: numbers, not software (design.md 8)."""

from __future__ import annotations

import sys
from collections import Counter
from pathlib import Path

import numpy as np
from . import embed, evaluate, store
from .config import DEFAULT


def run(root: Path, backend: str = "mfcc", reveal_test: bool = False) -> None:
    lab = store.labels_db(root)
    labels = store.current_labels(lab, source="user")
    hash_by_path = dict(lab.execute("SELECT path, hash FROM tracks"))

    paths, X = embed.load(root, backend, DEFAULT, sorted(labels))
    if not paths:
        sys.exit("no embeddings: run embed first")
    y = np.array([labels[p] for p in paths])
    hashes = [hash_by_path[p] for p in paths]

    print(f"backend {backend}   tracks {len(paths)}   dim {X.shape[1]}   "
          f"labels {len(set(y))}")

    parts = evaluate.split(hashes)
    against = evaluate.TEST if reveal_test else evaluate.VAL
    rep = evaluate.evaluate(X, y, parts, against)
    hold, train = parts == against, parts == evaluate.TRAIN

    sealed = "" if reveal_test else "   (test slice sealed)"
    print(f"\ntrain {rep.n_train}   {against} {rep.n_holdout}{sealed}")
    print(f"majority baseline  {rep.baseline * 100:5.1f}%")
    print(f"model accuracy     {rep.accuracy * 100:5.1f}%   "
          f"({(rep.accuracy - rep.baseline) * 100:+.1f} pts)")

    # diagnostics: did it learn the colour, or just the commonest level?
    model = evaluate._fit(X[train], y[train])
    pred = model.predict(X[hold])
    true = y[hold]
    for part, name in ((0, "colour"), (1, "level")):
        acc = evaluate.component_accuracy(pred, true, part)
        parts = [s.split("_")[part] for s in true]
        base = Counter(parts).most_common(1)[0][1] / len(parts)
        print(f"{name:>7} only        {acc * 100:5.1f}%   "
              f"(baseline {base * 100:.1f}%, {len(set(parts))} values)")

    print("\nper label (holdout):")
    for l, (n, acc) in sorted(rep.per_label.items(), key=lambda kv: -kv[1][0]):
        flag = "" if n >= 5 else "   (too few to mean anything)"
        print(f"  {l:<10} n={n:<4} {acc * 100:5.1f}%{flag}")

    print("\nlearning curve (training tracks -> holdout accuracy):")
    for n, acc in evaluate.learning_curve(X, y, parts, against):
        print(f"  {n:>6}  {acc * 100:5.1f}%")


if __name__ == "__main__":
    run(Path(sys.argv[1]) if len(sys.argv) > 1 else Path.home() / "Music" / "Collection",
        sys.argv[2] if len(sys.argv) > 2 else "mfcc",
        reveal_test="--test" in sys.argv)

"""Honest measurement.

The holdout is fixed, derived from the content hash, and never trained on
(design.md 3).
"""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field

import numpy as np
from sklearn.linear_model import LogisticRegression

BUCKETS = 10  # bucket 0 -> test, bucket 1 -> validation, rest -> train

TEST, VAL, TRAIN = "test", "val", "train"


def split(hashes: list[str]) -> np.ndarray:
    """Deterministic from content: stable as labels accumulate, no bookkeeping,
    and it cannot drift.

    Three ways, not two. Choosing the best of several backends against one
    holdout inflates the winner's score purely by selection, so backend and
    threshold decisions are made on `val` and `test` is looked at once, at the
    end (design.md 3).
    """
    out = []
    for h in hashes:
        b = int(h[:8], 16) % BUCKETS
        out.append(TEST if b == 0 else VAL if b == 1 else TRAIN)
    return np.array(out)


@dataclass
class Report:
    n_train: int
    n_holdout: int
    n_labels: int
    baseline: float
    accuracy: float
    per_label: dict[str, tuple[int, float]] = field(default_factory=dict)
    extras: dict[str, float] = field(default_factory=dict)


def _fit(Xtr, ytr):
    return LogisticRegression(
        max_iter=2000, C=1.0, class_weight="balanced", n_jobs=-1,
    ).fit(Xtr, ytr)


def evaluate(X: np.ndarray, y: np.ndarray, parts: np.ndarray,
             against: str = VAL) -> Report:
    hold = parts == against
    train = parts == TRAIN
    Xtr, ytr, Xte, yte = X[train], y[train], X[hold], y[hold]
    model = _fit(Xtr, ytr)
    pred = model.predict(Xte)

    majority = Counter(ytr).most_common(1)[0][0]
    baseline = float((yte == majority).mean())

    per_label: dict[str, tuple[int, float]] = {}
    for lab in sorted(set(y)):
        m = yte == lab
        if m.sum():
            per_label[lab] = (int(m.sum()), float((pred[m] == lab).mean()))

    return Report(
        n_train=len(ytr), n_holdout=len(yte), n_labels=len(set(y)),
        baseline=baseline, accuracy=float((pred == yte).mean()),
        per_label=per_label,
    )


def component_accuracy(pred: np.ndarray, true: np.ndarray, part: int) -> float:
    """Diagnostic only: split labels like 'Purple_C' on '_'.

    The app never does this — labels are opaque symbols (design.md 5). But when
    reading results it matters hugely whether a model learned the colour or just
    learned to say the commonest level.
    """
    p = [s.split("_")[part] if "_" in s else s for s in pred]
    t = [s.split("_")[part] if "_" in s else s for s in true]
    return float(np.mean([a == b for a, b in zip(p, t)]))


def learning_curve(X: np.ndarray, y: np.ndarray, parts: np.ndarray,
                   against: str = VAL,
                   fractions=(0.1, 0.2, 0.4, 0.6, 0.8, 1.0),
                   seed: int = 0) -> list[tuple[int, float]]:
    """Accuracy vs training-set size: says how many labels this problem needs."""
    rng = np.random.default_rng(seed)
    hold, train = parts == against, parts == TRAIN
    Xtr, ytr, Xte, yte = X[train], y[train], X[hold], y[hold]
    order = rng.permutation(len(ytr))
    out = []
    for f in fractions:
        idx = order[:max(int(len(order) * f), 10)]
        if len(set(ytr[idx])) < 2:
            continue
        m = _fit(Xtr[idx], ytr[idx])
        out.append((len(idx), float((m.predict(Xte) == yte).mean())))
    return out

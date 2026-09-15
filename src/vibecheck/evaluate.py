"""Honest measurement.

The holdout is fixed, derived from the content hash, and never trained on
(design.md 3).
"""

from __future__ import annotations

from collections import Counter
from dataclasses import dataclass, field

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

DEFAULT_C = 0.001

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
    """Standardise, then plain logistic regression.

    Scaling is fitted on training data only, so it cannot leak. No
    class_weight="balanced": it optimises balanced accuracy, and at this
    imbalance (412x) it costs 10-18 points of real accuracy.
    """
    return make_pipeline(
        StandardScaler(),
        # C selected by 5-fold CV on the training split, not by scoring the
        # holdout: at ~500 dims and a few thousand examples, regularisation is
        # worth ~10 points and the optimum is a flat plateau near 1e-3.
        LogisticRegression(max_iter=5000, C=DEFAULT_C),
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


@dataclass
class AxisScore:
    """One axis, measured on a holdout, in the units the app optimises."""
    axis: str
    n_values: int
    cost: float          # decisions a correct assertion saves
    misleading: float
    n: int               # holdout tracks whose truth is known for this axis
    spoke: int
    correct: int
    cost_left: float     # decisions still to make, per track

    @property
    def coverage(self) -> float:
        return self.spoke / self.n if self.n else 0.0

    @property
    def accuracy(self) -> float:
        return self.correct / self.spoke if self.spoke else 0.0


def score_axes(X: np.ndarray, y: dict, hashes: list[str], axes: list,
               against: str = VAL) -> list[AxisScore]:
    """Fit on the training slice only, then score each axis on a holdout.

    Deliberately a second fit: the model the app then uses is trained on
    everything, which is right for predicting and useless for measuring. The
    cost is one extra logistic regression per axis per round.

    Scored through the real Model, not a reimplementation of the decision rule,
    so the number recorded is the number the user experiences.
    """
    from .predict import Model

    parts = split(hashes)
    tr, ho = parts == TRAIN, parts == against
    if tr.sum() < 2 or ho.sum() == 0:
        return []

    model = Model(axes)
    model.train(X[tr], {k: v[tr] for k, v in y.items()})
    preds = model.predict(X[ho])

    out = []
    for ax in axes:
        if ax.name not in model.models:
            continue
        n = spoke = correct = 0
        left = 0.0
        for truth, pred in zip(y[ax.name][ho], preds):
            if truth is None:        # this label says nothing about this axis
                continue
            n += 1
            said = pred.values.get(ax.name)
            if said is None:
                left += ax.cost                      # still to be decided
            elif said == truth:
                spoke += 1
                correct += 1                         # nothing left to do
            else:
                spoke += 1
                left += ax.cost + ax.misleading_cost  # decide it, and undo this
        if n:
            out.append(AxisScore(ax.name, ax.n_values, ax.cost,
                                 ax.misleading_cost, n, spoke, correct,
                                 left / n))
    return out

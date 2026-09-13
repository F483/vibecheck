"""Deciding what to say about a track.

There are no confidence thresholds. Each level of the label hierarchy is scored
by the work it leaves the user -- how many binary decisions remain, log2 of the
options still open -- and the cheapest wins:

    full label right (Pink_C)     0       nothing left to decide
    colour right, level unknown   1.58    3 levels remain
    hue right                     2.58    6 remain
    tone right                    3.58    12 remain
    says nothing                  4.58    all 24

An assertion that turns out wrong is charged at the finest level where it was
*still* correct, plus `misleading_cost` -- so calling a Pink_C track Pink_B is
cheap, calling it Green_A is not. Near-misses earning partial credit is what
pushes the model toward being close rather than boldly wrong, and the cascade
from fine to coarse falls out of the arithmetic rather than being hand-tuned.

Nothing here knows what a label means. A level is just a map from the full
label to a coarser group, supplied by config.
"""

from __future__ import annotations

import math
from dataclasses import dataclass

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

C = 0.001


@dataclass
class Level:
    name: str
    group: dict[str, str]      # full label -> this level's value
    cost: float = 0.0          # decisions remaining when right at this level


@dataclass
class Prediction:
    level: str
    label: str | None
    confidence: float


def levels_from(labels: list[str], maps: list[tuple[str, dict[str, str]]]) -> list[Level]:
    """Finest first. Cost = log2(labels still possible once this is known)."""
    out = []
    for name, m in maps:
        sizes = {}
        for lab in labels:
            sizes.setdefault(m[lab], 0)
            sizes[m[lab]] += 1
        mean_remaining = sum(sizes.values()) / len(sizes)
        out.append(Level(name, m, math.log2(max(mean_remaining, 1))))
    return out


def _fit(X, y):
    return make_pipeline(StandardScaler(),
                         LogisticRegression(max_iter=5000, C=C)).fit(X, y)


class Model:
    def __init__(self, levels: list[Level]):
        self.levels = levels
        self.none_cost = math.log2(len({v for lv in levels for v in lv.group}) or 1)

    def train(self, X: np.ndarray, y: np.ndarray) -> dict:
        self.labels = sorted(set(y))
        self.none_cost = math.log2(len(self.labels))
        self.models = [_fit(X, np.array([lv.group[v] for v in y])) for lv in self.levels]
        return {"tracks": len(y), "labels": len(self.labels),
                "levels": [lv.name for lv in self.levels]}

    def _cost_if(self, asserted: str, truth: str, k: int, M: float) -> float:
        """Cost of asserting `asserted` at level k when the truth is `truth`."""
        lv = self.levels[k]
        if lv.group[truth] == asserted:
            return lv.cost
        for j in range(k + 1, len(self.levels)):
            deeper = self.levels[j]
            # was the assertion still right at this coarser level?
            same = {l for l in self.labels if lv.group[l] == asserted}
            if any(deeper.group[l] == deeper.group[truth] for l in same):
                return deeper.cost + M
        return self.none_cost + M

    def predict(self, X: np.ndarray, misleading_cost: float = 1.0) -> list[Prediction]:
        M = misleading_cost
        n = len(X)
        best = np.full(n, self.none_cost)
        chosen: list[list] = [[None, None, 0.0] for _ in range(n)]
        for k, (lv, m) in enumerate(zip(self.levels, self.models)):
            P = m.predict_proba(X)
            classes = list(m.classes_)
            for j, cand in enumerate(classes):
                cost = np.array([self._cost_if(cand, t, k, M) for t in self.labels])
                # P is over this level's groups; expand to a per-label distribution
                w = np.zeros((n, len(self.labels)))
                for li, lab in enumerate(self.labels):
                    w[:, li] = P[:, classes.index(lv.group[lab])] / \
                        sum(1 for x in self.labels if lv.group[x] == lv.group[lab])
                e = w @ cost
                better = e < best
                best[better] = e[better]
                for i in np.nonzero(better)[0]:
                    chosen[i] = [lv.name, cand, float(P[i].max())]
        return [Prediction(c[0] or "none", c[1], c[2]) for c in chosen]

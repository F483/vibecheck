"""Deciding what to say about a track.

The label is not a hierarchy, it is a set of **independent axes**. In the
reference vocabulary, hue (4 values) and tone (2 values) form a 4x2 grid whose
cells are the 8 colours, and the rating is a third axis entirely. Knowing hue
and tone *is* knowing the colour; knowing only one of them narrows the choice
without making it.

Treating that as a cascade -- tone, then hue, then colour -- was wrong, and
cost real information: a model confident about hue and tone would emit only the
hue, discarding a colour it had already determined.

Each axis is decided on its own, by expected cost. Knowing an axis of n values
saves log2(n) binary decisions; getting it wrong costs that saving back plus
`misleading_cost`, because the user has to notice and undo it. So an axis is
asserted when

    (1 - p) * (cost + misleading_cost) < cost

which is just "assert when being right is likely enough to be worth the risk".
Costs add across axes, so no axis needs to know about any other.

Nothing here knows what a label means. An axis is a map from the full label to
one of its values, supplied by config.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

C = 0.001


@dataclass
class Axis:
    name: str
    group: dict[str, str]          # full label -> this axis's value
    misleading_cost: float = 1.0
    n_values: int = 0              # how many the *vocabulary* allows
    cost: float = 0.0              # decisions saved by knowing it

    def __post_init__(self):
        # From the declared vocabulary, not from what training happened to
        # contain. A user with five ratings who has only used three still
        # faces a five-way choice, so knowing the rating is worth more than
        # the training data alone would suggest -- and the axis should not
        # become more reluctant to speak simply because it has seen less.
        n = self.n_values or len(set(self.group.values()))
        self.cost = self.cost or math.log2(max(n, 1))


@dataclass
class Prediction:
    """What was asserted on each axis, and how sure it was."""
    values: dict[str, str] = field(default_factory=dict)
    confidence: dict[str, float] = field(default_factory=dict)

    def said(self, *axes: str) -> bool:
        return all(a in self.values for a in axes)


def _fit(X, y):
    return make_pipeline(StandardScaler(),
                         LogisticRegression(max_iter=5000, C=C)).fit(X, y)


class Model:
    def __init__(self, axes: list[Axis]):
        self.axes = axes

    def train(self, X: np.ndarray, y: np.ndarray) -> dict:
        self.labels = sorted(set(y))
        self.models = {}
        trained = []
        for ax in self.axes:
            values = np.array([ax.group[v] for v in y])
            if len(set(values)) < 2:
                continue          # nothing to learn yet
            self.models[ax.name] = _fit(X, values)
            trained.append(ax.name)
        return {"tracks": len(y), "labels": len(self.labels), "axes": trained}

    def predict(self, X: np.ndarray) -> list[Prediction]:
        out = [Prediction() for _ in range(len(X))]
        for ax in self.axes:
            m = self.models.get(ax.name)
            if m is None:
                continue
            P = m.predict_proba(X)
            classes = list(m.classes_)
            M = ax.misleading_cost
            for i, row in enumerate(P):
                j = int(row.argmax())
                p = float(row[j])
                if (1 - p) * (ax.cost + M) < ax.cost:
                    out[i].values[ax.name] = classes[j]
                    out[i].confidence[ax.name] = p
        return out

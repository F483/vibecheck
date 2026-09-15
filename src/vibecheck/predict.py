"""Deciding what to say about a track.

Three independent axes: **hue** (4 values), **tone** (2), **stars** (6).
Hue and tone form a 4x2 grid whose cells are the eight colours, so knowing both
*is* knowing the colour; knowing one narrows the choice without making it. The
rating is a third axis entirely -- it can be sure a track is four stars while
only knowing it is Warm.

Treating that as a cascade -- tone, then hue, then colour -- was wrong, and cost
real information: a model confident about hue and tone would emit only the hue,
discarding a colour it had already determined.

Each axis is decided on its own, by expected cost. Knowing an axis of n values
saves log2(n) binary decisions; getting it wrong costs that saving back plus
`misleading_cost`, because the user has to notice and undo it. So an axis is
asserted when

    (1 - p) * (cost + misleading_cost) < cost

which is just "assert when being right is likely enough to be worth the risk".
Costs add across axes, so no axis needs to know about any other.

Nothing here knows what a colour *means*. It is handed targets and returns
values; which sounds go with which colour is the entire content of the model.
"""

from __future__ import annotations

import math
from dataclasses import dataclass, field

import numpy as np
from sklearn.linear_model import LogisticRegression
from sklearn.pipeline import make_pipeline
from sklearn.preprocessing import StandardScaler

from . import palette

C = 0.001

HUE, TONE, STARS = "hue", "tone", "stars"


@dataclass
class Axis:
    name: str
    n_values: int                  # what the vocabulary allows, not what was seen
    misleading_cost: float = 1.0
    cost: float = 0.0              # decisions saved by knowing it

    def __post_init__(self):
        # From the declared vocabulary, not from what training happened to
        # contain. Six star ratings exist even for someone who only ever uses
        # three, so knowing the rating is worth more than the training data
        # alone suggests -- and the axis should not become more reluctant to
        # speak simply because it has seen less.
        self.cost = self.cost or math.log2(max(self.n_values, 1))


def axes_for(misleading_cost: float,
              stars_misleading_cost: float) -> list[Axis]:
    return [
        Axis(HUE, len(palette.HUES), misleading_cost),
        Axis(TONE, len(palette.TONES), misleading_cost),
        Axis(STARS, len(palette.STAR_VALUES), stars_misleading_cost),
    ]


def targets_for(labels: list) -> dict[str, np.ndarray]:
    """store.Label -> one target array per axis. None means "says nothing".

    A label that carries no rating is not evidence about the rating. Training on
    a placeholder would make "unknown" a value the model can assert.
    """
    return {
        HUE:   np.array([l.hue for l in labels], dtype=object),
        TONE:  np.array([l.tone for l in labels], dtype=object),
        STARS: np.array([l.stars for l in labels], dtype=object),
    }


@dataclass
class Prediction:
    """What was asserted on each axis, and how sure it was.

    `top` additionally holds the best guess on *every* axis, including the ones
    that stayed silent. Nothing acts on those, but they are what makes the
    decision rule reviewable after the fact.
    """
    values: dict[str, object] = field(default_factory=dict)
    confidence: dict[str, float] = field(default_factory=dict)
    top: dict[str, tuple[object, float]] = field(default_factory=dict)

    @property
    def colour(self):
        return palette.colour_from_grid(self.values.get(HUE),
                                        self.values.get(TONE))

    @property
    def stars(self) -> int | None:
        return self.values.get(STARS)


def _fit(X, y):
    return make_pipeline(StandardScaler(),
                         LogisticRegression(max_iter=5000, C=C)).fit(X, y)


class Model:
    def __init__(self, axes: list[Axis]):
        self.axes = axes

    def train(self, X: np.ndarray, y: dict[str, np.ndarray]) -> dict:
        self.models = {}
        trained = []
        for ax in self.axes:
            known = np.array([v is not None for v in y[ax.name]])
            # out of the object array and into a real dtype: sklearn rejects
            # object-typed integers as an unknown target
            values = np.asarray(list(y[ax.name][known]))
            if len(set(values)) < 2:
                continue          # nothing to learn yet
            self.models[ax.name] = _fit(X[known], values)
            trained.append(ax.name)
        return {"tracks": len(X), "axes": trained}

    def predict(self, X: np.ndarray) -> list[Prediction]:
        out = [Prediction() for _ in range(len(X))]
        for ax in self.axes:
            m = self.models.get(ax.name)
            if m is None:
                continue
            P = m.predict_proba(X)
            # numpy scalars out: these end up bound as sqlite parameters
            classes = [c.item() if hasattr(c, "item") else c for c in m.classes_]
            M = ax.misleading_cost
            for i, row in enumerate(P):
                j = int(row.argmax())
                p = float(row[j])
                out[i].top[ax.name] = (classes[j], p)
                if (1 - p) * (ax.cost + M) < ax.cost:
                    out[i].values[ax.name] = classes[j]
                    out[i].confidence[ax.name] = p
        return out

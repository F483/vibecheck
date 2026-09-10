"""MuQ — music-native self-supervised representation.

Trained on music by masked prediction rather than by matching audio to text
captions, so none of its capacity is spent separating speech and environmental
sound from music. That matters here: the reference collection is entirely
instrumental electronic music, and CLAP's representation compresses it into
roughly 26 effective dimensions out of 512.

Runs in a separate virtualenv (.venv-muq): the `muq` package requires
transformers 4.x, and the rest of this project needs 5.x. Embedding happens
there via scripts/embed_muq.py; this class exists so the main environment can
compute the cache key and load the vectors for evaluation.
"""

from __future__ import annotations

import dataclasses

import numpy as np

from ..config import Preproc

MODEL_ID = "OpenMuQ/MuQ-large-msd-iter"
SAMPLE_RATE = 24000
WINDOW_SECONDS = 10.0
N_WINDOWS = 9


class MuQ:
    name = "muq"
    version = "1"

    def preproc(self, base: Preproc) -> Preproc:
        return dataclasses.replace(
            base, sample_rate=SAMPLE_RATE,
            excerpt_seconds=WINDOW_SECONDS, n_excerpts=N_WINDOWS,
        )

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        raise RuntimeError(
            "MuQ embeds from the isolated env: bash scripts/embed_muq.sh")

"""Embedding backends behind one interface, so they can be compared and swapped.

A backend turns excerpt PCM into one pooled vector. Nothing else in the system
knows which one produced it — the cache key records it (design.md 3).
"""

from __future__ import annotations

from typing import Protocol

import numpy as np

from ..config import Preproc


class Backend(Protocol):
    name: str
    version: str

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        """Excerpt PCM -> one L2-normalised vector for the track."""


def get(name: str) -> Backend:
    if name == "mfcc":
        from .mfcc import MFCC

        return MFCC()
    raise KeyError(f"unknown backend: {name}")

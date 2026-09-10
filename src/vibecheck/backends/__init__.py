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

    def preproc(self, base: Preproc) -> Preproc:
        """The config this backend needs.

        Models disagree about sample rate (MERT wants 24 kHz, CLAP 48 kHz), and
        the rate changes the numbers, so it must reach the cache key rather than
        being applied silently at decode time.
        """

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        """Excerpt PCM -> one vector for the track."""


def get(name: str) -> Backend:
    if name == "mfcc":
        from .mfcc import MFCC

        return MFCC()
    if name == "mert":
        from .mert import MERT

        return MERT()
    if name == "clap":
        from .clap import CLAP

        return CLAP()
    if name == "clapw":
        from .clapw import CLAPWindows

        return CLAPWindows()
    if name == "muq":
        from .muq import MuQ

        return MuQ()
    if name == "whisper":
        from .whisper import Whisper

        return Whisper()
    raise KeyError(f"unknown backend: {name}")

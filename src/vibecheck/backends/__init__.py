"""Embedding backends behind one interface, so they can be compared and swapped.

A backend turns excerpt PCM into one pooled vector. Nothing else in the system
knows which one produced it — the cache key records it (design.md 3).
"""

from __future__ import annotations

import os
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


def offline_if_cached(*ids: str) -> bool:
    """Forbid network access outright when the weights are already here.

    `local_files_only=True` stops `from_pretrained` fetching, but the Hub
    client still runs a deferred check afterwards -- which is why a fully
    cached run printed "you are sending unauthenticated requests to the HF
    Hub". This app is local; once a model is downloaded it should need no
    network at all, and should keep working on a machine that has none.

    `HF_HUB_OFFLINE` is read when huggingface_hub is imported, so this has to
    run before transformers is. It is set only when the model is genuinely
    cached, so a first run can still download.
    """
    from huggingface_hub import try_to_load_from_cache

    for mid in ids:
        if isinstance(try_to_load_from_cache(mid, "config.json"), str):
            os.environ["HF_HUB_OFFLINE"] = "1"
            return True
    return False


def cached_first(load, *ids):
    """Load from the local cache before touching the network.

    The download still happens on a first run, or if a model is missing. Every
    failure is reported: "could not load CLAP" with no reason was not enough to
    debug from.
    """
    offline_if_cached(*ids)
    errors = []
    for mid in ids:
        for local_only in (True, False):
            try:
                return mid, load(mid, local_only)
            except Exception as e:                # noqa: BLE001 - reported below
                errors.append(f"{mid} ({'cache' if local_only else 'hub'}): "
                              f"{type(e).__name__}: {e}")
    hint = ("\n  If the cache is incomplete, HF_HUB_OFFLINE=0 allows a "
            "re-download." if os.environ.get("HF_HUB_OFFLINE") == "1" else "")
    raise RuntimeError("could not load a model.\n  " + "\n  ".join(errors) + hint)

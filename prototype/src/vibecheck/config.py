"""Pinned preprocessing parameters.

Never rely on library defaults: librosa's have changed across versions, so an
implicit default is a silent behaviour change waiting for an upgrade. These
values are the port spec for a native implementation later (design.md 4.2),
and they take part in the embedding cache key.
"""

from __future__ import annotations

import hashlib
import json
from dataclasses import asdict, dataclass


@dataclass(frozen=True)
class Preproc:
    sample_rate: int = 16000
    n_excerpts: int = 3
    excerpt_seconds: float = 30.0
    # decode a little before each excerpt and drop it: mp3 seeking is
    # frame-aligned and the bit reservoir makes the first frames inexact
    seek_pad_seconds: float = 0.5

    n_fft: int = 2048
    hop_length: int = 512
    n_mels: int = 128
    fmin: float = 20.0
    fmax: float = 8000.0
    mel_scale: str = "slaney"
    n_mfcc: int = 40

    def digest(self) -> str:
        blob = json.dumps(asdict(self), sort_keys=True).encode()
        return hashlib.sha256(blob).hexdigest()[:16]


DEFAULT = Preproc()

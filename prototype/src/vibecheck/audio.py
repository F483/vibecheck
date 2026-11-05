"""Decoding. Deliberately the only place that knows how audio becomes samples.

Kept to one narrow function so swapping ffmpeg for AVFoundation later is a
one-file change (design.md 4.2).
"""

from __future__ import annotations

import subprocess

import numpy as np

from .config import Preproc


class DecodeError(RuntimeError):
    pass


def duration_seconds(path: str) -> float:
    out = subprocess.run(
        ["ffprobe", "-v", "quiet", "-show_entries", "format=duration",
         "-of", "default=nw=1:nk=1", path],
        capture_output=True, timeout=60,
    )
    try:
        return float(out.stdout.strip())
    except ValueError as e:
        raise DecodeError(f"no duration: {path}") from e


def decode(path: str, offset: float, seconds: float, sample_rate: int) -> np.ndarray:
    """Mono float32 PCM for one region. The whole audio interface."""
    out = subprocess.run(
        ["ffmpeg", "-v", "quiet", "-ss", f"{offset:.3f}", "-t", f"{seconds:.3f}",
         "-i", path, "-f", "f32le", "-ac", "1", "-ar", str(sample_rate), "-"],
        capture_output=True, timeout=300,
    )
    if out.returncode != 0:
        raise DecodeError(f"ffmpeg failed: {path}")
    return np.frombuffer(out.stdout, dtype=np.float32)


def excerpts(path: str, cfg: Preproc) -> list[np.ndarray]:
    """Sample k windows spread through the track.

    DJ tracks have long intros and outros, so a single window from the start is
    unrepresentative (design.md 4.1).
    """
    total = duration_seconds(path)
    want = cfg.excerpt_seconds
    out: list[np.ndarray] = []

    if total <= want:
        # short track: take what there is, once
        return [decode(path, 0.0, want, cfg.sample_rate)]

    for i in range(cfg.n_excerpts):
        centre = total * (i + 1) / (cfg.n_excerpts + 1)
        start = min(max(centre - want / 2, 0.0), total - want)
        pad = min(cfg.seek_pad_seconds, start)
        pcm = decode(path, start - pad, want + pad, cfg.sample_rate)
        drop = int(pad * cfg.sample_rate)
        pcm = pcm[drop:drop + int(want * cfg.sample_rate)]
        if pcm.size:
            out.append(pcm)

    if not out:
        raise DecodeError(f"no audio decoded: {path}")
    return out

"""Picking an accelerator, wherever this runs.

CUDA where there is an NVIDIA card, Metal on Apple silicon, CPU otherwise.
Nothing else in the codebase is platform-specific: decoding goes through
ffmpeg, tags through mutagen, storage through SQLite, all of which are portable.
"""

from __future__ import annotations


def pick() -> str:
    import torch

    if torch.cuda.is_available():
        return "cuda"
    if getattr(torch.backends, "mps", None) and torch.backends.mps.is_available():
        return "mps"
    return "cpu"


def empty_cache(device: str) -> None:
    """Release cached allocator memory; a no-op where it does not apply."""
    import torch

    if device == "cuda":
        torch.cuda.empty_cache()
    elif device == "mps":
        torch.mps.empty_cache()

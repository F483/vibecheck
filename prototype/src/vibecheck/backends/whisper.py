"""Whisper encoder as a feature extractor.

A deliberately *different* kind of model from the others: trained for speech
recognition and captioning rather than music representation. Published linear
probes have Whisper-derived encoders beating music-SSL models substantially on
genre (~91% vs ~79% on GTZAN) -- the plausible reason being that genre words
appear constantly in captions. Different training objective means different
mistakes, which is what makes it worth having alongside CLAP.

Whisper natively consumes 30 s at 16 kHz, so the windowing matches that exactly
rather than fighting it.
"""

from __future__ import annotations

import dataclasses

import numpy as np

from .. import device
from ..config import Preproc

MODEL_ID = "openai/whisper-small"
SAMPLE_RATE = 16000
WINDOW_SECONDS = 30.0
N_WINDOWS = 3


class Whisper:
    name = "whisper"
    version = "1"

    def __init__(self) -> None:
        self._enc = None
        self._fe = None
        self._device = None

    def preproc(self, base: Preproc) -> Preproc:
        return dataclasses.replace(
            base, sample_rate=SAMPLE_RATE,
            excerpt_seconds=WINDOW_SECONDS, n_excerpts=N_WINDOWS,
        )

    def _load(self):
        if self._enc is not None:
            return self._enc
        import torch
        from transformers import AutoFeatureExtractor, WhisperModel

        self._device = device.pick()
        from . import cached_first

        _, model = cached_first(
            lambda mid, local: WhisperModel.from_pretrained(
                mid, local_files_only=local),
            MODEL_ID)
        self._enc = model.get_encoder().to(self._device).eval()
        _, self._fe = cached_first(
            lambda mid, local: AutoFeatureExtractor.from_pretrained(
                mid, local_files_only=local),
            MODEL_ID)
        return self._enc

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        import torch

        enc = self._load()
        vecs = []
        with torch.inference_mode():
            for pcm in excerpts:
                feats = self._fe(pcm.astype(np.float32), sampling_rate=cfg.sample_rate,
                                 return_tensors="pt")
                x = feats.input_features.to(self._device)
                out = enc(x).last_hidden_state  # (1, frames, dim)
                vecs.append(out.mean(dim=1).squeeze(0).float().cpu().numpy())
        device.empty_cache(self._device)
        v = np.mean(vecs, axis=0)
        n = np.linalg.norm(v)
        return ((v / n) if n else v).astype(np.float32)

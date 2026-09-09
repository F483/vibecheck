"""LAION CLAP — audio-text contrastive model.

The second neural candidate, and mel-based rather than raw-waveform, so it
carries the full preprocessing chain that MERT avoids (design.md 4.2). Kept in
the comparison anyway: it is strong on music, and its text encoder offers a
zero-shot sanity check that does not depend on the user's labels at all.

CLAP's feature extractor truncates to 10 s, so this takes 9 x 10 s windows
rather than 3 x 30 s -- same 90 s of audio per track as the other backends,
which keeps the comparison about the model and not about how much it listened to.
"""

from __future__ import annotations

import dataclasses

import numpy as np

from ..config import Preproc

MODEL_ID = "laion/larger_clap_music_and_speech"
FALLBACK_ID = "laion/clap-htsat-unfused"
SAMPLE_RATE = 48000
WINDOW_SECONDS = 10.0
N_WINDOWS = 9


class CLAP:
    name = "clap"
    version = "1"

    def __init__(self) -> None:
        self._model = None
        self._proc = None
        self._device = None
        self._id = None

    def preproc(self, base: Preproc) -> Preproc:
        return dataclasses.replace(
            base, sample_rate=SAMPLE_RATE,
            excerpt_seconds=WINDOW_SECONDS, n_excerpts=N_WINDOWS,
        )

    def _load(self):
        if self._model is not None:
            return self._model
        import torch
        from transformers import AutoProcessor, ClapModel

        self._device = "mps" if torch.backends.mps.is_available() else "cpu"
        for model_id in (MODEL_ID, FALLBACK_ID):
            try:
                self._model = ClapModel.from_pretrained(model_id).to(self._device).eval()
                self._proc = AutoProcessor.from_pretrained(model_id)
                self._id = model_id
                break
            except Exception:  # model id may not exist; try the next
                continue
        if self._model is None:
            raise RuntimeError("could not load any CLAP checkpoint")
        return self._model

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        import torch

        model = self._load()
        inputs = self._proc(
            audios=[x.astype(np.float32) for x in excerpts],
            sampling_rate=cfg.sample_rate, return_tensors="pt",
        )
        inputs = {k: v.to(self._device) for k, v in inputs.items()}
        with torch.no_grad():
            feats = model.get_audio_features(**inputs)
        v = feats.mean(dim=0).cpu().numpy()
        n = np.linalg.norm(v)
        return ((v / n) if n else v).astype(np.float32)

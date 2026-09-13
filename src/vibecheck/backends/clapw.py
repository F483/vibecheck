"""CLAP, keeping every window separate instead of averaging them.

Two hypotheses in one backend:

1. **Averaging destroys the signal.** A six-minute DJ track is not homogeneous
   -- intro, groove, breakdown, outro. Mean-pooling nine windows into one
   vector blurs the peak section, which is plausibly what the label describes.
   Keeping windows lets the classifier train on them individually (24x the
   examples) and lets prediction vote or take the most confident window.

2. **We were only listening to a quarter of each track.** 9 x 10 s = 90 s of a
   ~6 minute track. 24 windows covers ~4 minutes.

Stored as mean and standard deviation across windows, 1024 dims per track.
"""

from __future__ import annotations

import dataclasses

import numpy as np

from ..config import Preproc

MODEL_ID = "laion/larger_clap_music_and_speech"
FALLBACK_ID = "laion/clap-htsat-unfused"
SAMPLE_RATE = 48000
WINDOW_SECONDS = 10.0
N_WINDOWS = 24
DIM = 512


class CLAPWindows:
    name = "clapw"
    version = "2"  # v1 stored all 24 window vectors; v2 stores mean+std

    def __init__(self) -> None:
        self._model = None
        self._proc = None
        self._device = None

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
        for mid in (MODEL_ID, FALLBACK_ID):
            try:
                self._model = ClapModel.from_pretrained(mid).to(self._device).eval()
                self._proc = AutoProcessor.from_pretrained(mid)
                break
            except Exception:
                continue
        if self._model is None:
            raise RuntimeError("could not load CLAP")
        return self._model

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        import torch

        model = self._load()
        inputs = self._proc(audio=[x.astype(np.float32) for x in excerpts],
                            sampling_rate=cfg.sample_rate, return_tensors="pt")
        inputs = {k: v.to(self._device) for k, v in inputs.items()}
        with torch.inference_mode():
            out = model.get_audio_features(**inputs)
        feats = out.pooler_output if hasattr(out, "pooler_output") else out
        v = feats.float().cpu().numpy()            # (windows, 512)
        v /= np.maximum(np.linalg.norm(v, axis=1, keepdims=True), 1e-8)
        if v.shape[0] < N_WINDOWS:
            v = np.vstack([v, np.repeat(v[-1:], N_WINDOWS - v.shape[0], axis=0)])
        v = v[:N_WINDOWS]
        if self._device == "mps":
            torch.mps.empty_cache()
        # Mean and standard deviation across windows, not the windows
        # themselves. Keeping all 24 cost 12x the storage and bought nothing:
        # training on individual windows scored identically to pooling them
        # (58.5% either way), and predicting from the single most confident
        # window was worse. What carries the small gain is how much a track
        # *varies*, which mean+std captures in 1024 numbers.
        return np.concatenate([v.mean(0), v.std(0)]).astype(np.float32)

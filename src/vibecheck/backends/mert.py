"""MERT-v1-95M — self-supervised music transformer.

Chosen first among the neural candidates because it takes **raw waveform**:
there is no mel spectrogram to reproduce, so the port problem of design.md 4.2
collapses to resampling alone. That matters as much as accuracy here.
"""

from __future__ import annotations

import dataclasses

import numpy as np

from ..config import Preproc

MODEL_ID = "m-a-p/MERT-v1-95M"
SAMPLE_RATE = 24000


class MERT:
    name = "mert"
    version = "1"

    def __init__(self) -> None:
        self._model = None
        self._device = None

    def preproc(self, base: Preproc) -> Preproc:
        return dataclasses.replace(base, sample_rate=SAMPLE_RATE)

    def _load(self):
        if self._model is not None:
            return self._model
        import torch
        from transformers import AutoModel

        self._device = "mps" if torch.backends.mps.is_available() else "cpu"
        model = AutoModel.from_pretrained(MODEL_ID, trust_remote_code=True)
        self._model = model.to(self._device).eval()
        return self._model

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        import torch

        model = self._load()
        vecs = []
        with torch.no_grad():
            for pcm in excerpts:
                x = torch.from_numpy(pcm).float().unsqueeze(0).to(self._device)
                out = model(x, output_hidden_states=True)
                # Mean over time for each layer, then mean over layers.
                # Published probes usually beat last-layer-only by using more
                # than the final layer; averaging keeps the dimension at 768
                # without having to pick a layer before measuring anything.
                layers = torch.stack([h.mean(dim=1) for h in out.hidden_states])
                vecs.append(layers.mean(dim=0).squeeze(0).cpu().numpy())
        v = np.mean(vecs, axis=0)
        # homogeneous dimensions here, unlike the MFCC stat vector, so L2 is
        # safe and standard for a transformer embedding
        n = np.linalg.norm(v)
        return ((v / n) if n else v).astype(np.float32)

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
        # MERT ships custom modelling code that ignores the per-call
        # output_hidden_states argument, so set it on the config instead
        model.config.output_hidden_states = True
        self._model = model.to(self._device).eval()
        return self._model

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        import torch

        model = self._load()
        vecs = []
        with torch.no_grad():
            for pcm in excerpts:
                x = torch.from_numpy(pcm).float().unsqueeze(0).to(self._device)
                out = model(x)
                # Mean-pool the final layer over time.
                # MERT's bundled modelling code does not expose intermediate
                # layers under transformers 5.x, and published probes often do
                # better from a middle layer than the last one -- so layer
                # choice is a knob worth revisiting with forward hooks *if*
                # MERT proves competitive. Not worth the complexity before that.
                vecs.append(out.last_hidden_state.mean(dim=1).squeeze(0).cpu().numpy())
        v = np.mean(vecs, axis=0)
        # homogeneous dimensions here, unlike the MFCC stat vector, so L2 is
        # safe and standard for a transformer embedding
        n = np.linalg.norm(v)
        return ((v / n) if n else v).astype(np.float32)

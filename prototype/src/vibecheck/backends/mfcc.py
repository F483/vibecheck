"""The floor. No model, no downloads, no arm64 dependency risk.

If a 200 MB neural model cannot clearly beat this on real labels, it is not
worth carrying (design.md 7).
"""

from __future__ import annotations

import librosa
import numpy as np

from ..config import Preproc


def _stats(x: np.ndarray) -> np.ndarray:
    return np.concatenate([x.mean(axis=1), x.std(axis=1)])


class MFCC:
    name = "mfcc"
    version = "2"  # v1 L2-normalised a heterogeneous vector; see below

    def preproc(self, base: Preproc) -> Preproc:
        return base

    def embed(self, excerpts: list[np.ndarray], cfg: Preproc) -> np.ndarray:
        # No L2 normalisation. These dimensions are in different units --
        # rolloff in Hz (thousands), MFCCs around +/-100, zcr in 0..1 -- so
        # dividing by one norm lets the Hz-scaled dims consume the whole vector
        # and numerically annihilates the rest. v1 did exactly that and scored
        # below the majority baseline. Per-dimension standardisation belongs in
        # the classifier, fitted on training data only.
        return np.mean([self._one(x, cfg) for x in excerpts], axis=0).astype(np.float32)

    def _one(self, pcm: np.ndarray, cfg: Preproc) -> np.ndarray:
        mel = librosa.feature.melspectrogram(
            y=pcm, sr=cfg.sample_rate, n_fft=cfg.n_fft, hop_length=cfg.hop_length,
            n_mels=cfg.n_mels, fmin=cfg.fmin, fmax=cfg.fmax, htk=(cfg.mel_scale == "htk"),
        )
        logmel = librosa.power_to_db(mel)
        mfcc = librosa.feature.mfcc(S=logmel, n_mfcc=cfg.n_mfcc)
        delta = librosa.feature.delta(mfcc)

        centroid = librosa.feature.spectral_centroid(
            y=pcm, sr=cfg.sample_rate, n_fft=cfg.n_fft, hop_length=cfg.hop_length)
        rolloff = librosa.feature.spectral_rolloff(
            y=pcm, sr=cfg.sample_rate, n_fft=cfg.n_fft, hop_length=cfg.hop_length)
        zcr = librosa.feature.zero_crossing_rate(pcm, hop_length=cfg.hop_length)
        rms = librosa.feature.rms(y=pcm, hop_length=cfg.hop_length)

        return np.concatenate([
            _stats(mfcc), _stats(delta),
            _stats(centroid), _stats(rolloff), _stats(zcr), _stats(rms),
        ])

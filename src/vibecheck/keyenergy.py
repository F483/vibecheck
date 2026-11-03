"""Key and energy, worked out from the audio. Nothing here learns at runtime.

Both come from one decode and one spectrum per track, and both are fixed
functions of the audio: the same file always gets the same answer, on any
machine, until version() changes.

**Key.** A pitch-class profile -- how much of each of the twelve notes the
track contains -- built from spectral peaks only, so drums add almost nothing,
after estimating the track's tuning (harmonic pitch class profile: Gomez 2006,
after Fujishima 1999). Each of the 24 keys scores the profile with a template
of weights, one per interval above its tonic (template key-finding:
Krumhansl & Schmuckler 1990), plus a fixed preference for minor (a key prior,
as in Temperley 2002), because dance music is mostly minor. The weights are
fitted to human-annotated tracks by multinomial logistic regression -- so that
the annotated key scores highest -- rather than averaged; see key_profiles.json
for where they come from and scripts/fit_key_profiles.py for how.

**Energy.** Three standard audio descriptors (MPEG-7, 2002): loudness,
brightness (spectral centroid) and noisiness (spectral flatness). Each is put
in units of its spread across a reference set of tracks, the three are added,
and the sum is placed on 1-10 so that a typical club track is 6 and one
standard deviation is one step. No ratings of any kind went into it -- the
constants are statistics of audio, from scripts/fit_energy_scale.py.

Written as alphanumeric keys ("8A" is A minor, "8B" is C major) and
"<key> - Energy <n>", which is how rekordbox and DJ tools commonly show them.
"""

from __future__ import annotations

import hashlib
import json
import os
import time
from concurrent.futures import ProcessPoolExecutor
from importlib import resources
from pathlib import Path
from typing import NamedTuple

import numpy as np

from . import audio, store
from .config import Preproc

ALGO = "ke1"

# 24 windows of 10 s spread through the track, as the colour model hears it.
PRE = Preproc(sample_rate=22050, n_excerpts=24, excerpt_seconds=10.0)
N_FFT = 4096
HOP = 2048

# Pitch profile (Gomez 2006).
FMIN, FMAX = 100.0, 5000.0     # Hz: below is mostly kick, above mostly hats
MAX_PEAKS = 60                 # strongest spectral peaks kept per frame
PEAK_FLOOR_DB = 60.0           # ignore peaks this far below the frame's loudest
BINS = 36                      # three per semitone
WIDTH = 4 / 3                  # semitones a peak is spread over (cos^2)
HARMONICS = 1                  # >1: a peak also counts towards f/2, f/3 ...
HARMONIC_DECAY = 0.6           # ... with weight 0.6, 0.36 (no gain measured)
LOG_FLOOR = 1e-3               # profile is compared in log, floored here

# Energy scale. Statistics of the three descriptors over reference audio --
# your collection and the GiantSteps key set, ~2,600 tracks -- with no labels
# or ratings of any kind. Recomputed by scripts/fit_energy_scale.py.
ENERGY_MEAN = (22.458, 2249.7, -4.9324)  # loudness dB, centroid Hz, log flatness
ENERGY_STD = (4.520, 753.7, 1.6096)
ENERGY_MID = 0.3813                      # the typical (median) summed z-score
ENERGY_SPREAD = 2.3585                   # its standard deviation
ENERGY_TYPICAL = 6                     # where the typical track lands on 1-10

_templates: dict | None = None


def templates() -> dict:
    """The fitted key templates (key_profiles.json), loaded on first use."""
    global _templates
    if _templates is None:
        _templates = json.loads(resources.files(__package__)
                                .joinpath("key_profiles.json").read_text())
    return _templates


def version() -> str:
    """Everything that decides an answer. Part of the cache key."""
    t = templates()
    return ALGO + "/" + hashlib.sha256(json.dumps([
        PRE.digest(), N_FFT, HOP, FMIN, FMAX, MAX_PEAKS, PEAK_FLOOR_DB, BINS,
        WIDTH, HARMONICS, HARMONIC_DECAY, LOG_FLOOR, ENERGY_MEAN, ENERGY_STD,
        ENERGY_MID,
        ENERGY_SPREAD, ENERGY_TYPICAL, t["major"], t["minor"],
        t["minor_bias"]]).encode()).hexdigest()[:12]


class Result(NamedTuple):
    key: str        # alphanumeric, "8A"
    energy: int     # 1..10


# --- notation ----------------------------------------------------------------

def alphanumeric(tonic: int, minor: bool) -> str:
    """Pitch class (C = 0) and mode -> "8A" / "8B".

    Each step round the wheel is a fifth (+7 semitones); A minor and C major
    share 8. Minor is A, major is B.
    """
    n = (7 * tonic + (5 if minor else 8)) % 12
    return f"{n or 12}{'A' if minor else 'B'}"


# --- spectrum ----------------------------------------------------------------

def _spectra(x: np.ndarray) -> np.ndarray:
    """Magnitude spectra, frames x bins, of one window."""
    win = np.hanning(N_FFT + 1)[:-1]
    frames = np.lib.stride_tricks.sliding_window_view(x, N_FFT)[::HOP] * win
    return np.abs(np.fft.rfft(frames, axis=1))


def _peaks(M: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    """Frequency and magnitude of each frame's strongest spectral peaks."""
    sr = PRE.sample_rate
    db = 20 * np.log10(M + 1e-12)
    f = np.fft.rfftfreq(N_FFT, 1 / sr)
    lo, hi = np.searchsorted(f, FMIN), np.searchsorted(f, FMAX)
    c, l, r = db[:, lo:hi], db[:, lo - 1:hi - 1], db[:, lo + 1:hi + 1]
    is_peak = (c > l) & (c >= r) & (c > db.max(1, keepdims=True) - PEAK_FLOOR_DB)
    # parabolic interpolation of the peak's true frequency and height
    den = np.where(is_peak, l - 2 * c + r, -1.0)
    off = 0.5 * (l - r) / den
    freq = (np.arange(lo, hi)[None, :] + off) * sr / N_FFT
    mag = np.where(is_peak, 10 ** ((c - 0.25 * (l - r) * off) / 20), 0.0)
    if mag.shape[1] > MAX_PEAKS:
        keep = np.argpartition(-mag, MAX_PEAKS, axis=1)[:, :MAX_PEAKS]
        mag = np.take_along_axis(mag, keep, 1)
        freq = np.take_along_axis(freq, keep, 1)
    return freq, mag


def _tuning(freq: np.ndarray, mag: np.ndarray) -> float:
    """How far the track sits from A440, in semitones (-0.5..0.5).

    Weighted circular mean of every peak's distance from the nearest
    equal-tempered note.
    """
    m = 12 * np.log2(np.maximum(freq, 1e-6) / 440.0)
    w = (mag ** 2).ravel()
    return float(np.angle(np.sum(w * np.exp(2j * np.pi * (m - np.round(m)).ravel())))
                 / (2 * np.pi))


def _hpcp(freq: np.ndarray, mag: np.ndarray, ref: float) -> np.ndarray:
    """Harmonic pitch-class profile per frame, frames x BINS, C first."""
    out = np.zeros((freq.shape[0], BINS))
    centres = np.arange(BINS) * 12.0 / BINS
    half = WIDTH / 2
    for h in range(1, HARMONICS + 1):
        pos = (12 * np.log2(np.maximum(freq / h, 1e-6) / 440.0) - ref + 9) % 12
        d = (pos[:, :, None] - centres[None, None, :] + 6) % 12 - 6
        w = np.where(np.abs(d) <= half, np.cos(np.pi / 2 * d / half) ** 2, 0.0)
        out += HARMONIC_DECAY ** (h - 1) * np.einsum("fk,fkb->fb", mag ** 2, w)
    return out


def pitch_profile(spectra: list[np.ndarray]) -> np.ndarray:
    """The track's twelve-note profile, C first, summing to 1."""
    peaks = [_peaks(M) for M in spectra if len(M)]
    freq = np.vstack([p[0] for p in peaks])
    mag = np.vstack([p[1] for p in peaks])
    P = _hpcp(freq, mag, _tuning(freq, mag))
    P /= P.max(1, keepdims=True) + 1e-12           # every frame counts equally
    p36 = P.sum(0)
    p12 = np.array([p36[(3 * k - 1) % BINS] + p36[3 * k] + p36[3 * k + 1]
                    for k in range(12)])
    return p12 / (p12.sum() + 1e-12)


def key_scores(profile: np.ndarray, major: np.ndarray, minor: np.ndarray,
               minor_bias: float) -> np.ndarray:
    """Score of each of the 24 keys: 12 major (C first), then 12 minor.

    The profile is rotated so the candidate tonic comes first, and weighed by
    the mode's template: one weight per interval above the tonic.
    """
    f = np.log(profile + LOG_FLOOR)
    rotated = np.stack([np.roll(f, -t) for t in range(12)])     # tonic x interval
    return np.concatenate([rotated @ major, rotated @ minor + minor_bias])


def key_of(profile: np.ndarray, major: np.ndarray | None = None,
           minor: np.ndarray | None = None,
           minor_bias: float | None = None) -> tuple[int, bool, float]:
    """(tonic, minor, margin over the runner-up) of the best-scoring key.

    Templates default to the shipped ones; the fitting script passes its own.
    """
    if major is None or minor is None or minor_bias is None:
        t = templates()
        major = np.asarray(t["major"]) if major is None else major
        minor = np.asarray(t["minor"]) if minor is None else minor
        minor_bias = float(t["minor_bias"]) if minor_bias is None else minor_bias
    s = key_scores(profile, major, minor, minor_bias)
    order = np.argsort(s)
    best = int(order[-1])
    return best % 12, best >= 12, float(s[best] - s[order[-2]])


# --- energy ------------------------------------------------------------------

def energy_features(spectra: list[np.ndarray]) -> tuple[float, float, float]:
    """(loudness dB, brightness Hz, noisiness log) -- medians over frames."""
    M = np.vstack([m for m in spectra if len(m)])[:, 1:]       # drop DC
    f = np.fft.rfftfreq(N_FFT, 1 / PRE.sample_rate)[1:]
    P = M ** 2 + 1e-12
    loud = 10 * np.log10(P.mean(1))
    centroid = (M * f).sum(1) / (M.sum(1) + 1e-12)
    flat = np.log(np.exp(np.log(P).mean(1)) / P.mean(1))
    return (float(np.median(loud)), float(np.median(centroid)),
            float(np.median(flat)))


def energy_score(features: tuple[float, float, float]) -> float:
    """Summed z-score of the three descriptors, against the reference set."""
    return float(sum((v - m) / s for v, m, s
                     in zip(features, ENERGY_MEAN, ENERGY_STD)))


def energy_of(features: tuple[float, float, float]) -> int:
    level = ENERGY_TYPICAL + (energy_score(features) - ENERGY_MID) / ENERGY_SPREAD
    return int(min(10, max(1, round(level))))


# --- per track ---------------------------------------------------------------

def analyse_audio(windows: list[np.ndarray]) -> tuple[Result, dict]:
    spectra = [_spectra(x.astype(np.float64)) for x in windows if len(x) >= N_FFT]
    if not spectra:
        raise audio.DecodeError("too short to analyse")
    profile = pitch_profile(spectra)
    tonic, minor, score = key_of(profile)
    feats = energy_features(spectra)
    result = Result(alphanumeric(tonic, minor), energy_of(feats))
    detail = {"profile": [round(v, 5) for v in profile.tolist()],
              "key_score": round(score, 4), "energy_features": feats,
              "energy_score": round(energy_score(feats), 4)}
    return result, detail


def analyse_file(path: str) -> tuple[Result, dict] | None:
    """None when the file cannot be decoded: one bad file must not stop a run."""
    try:
        return analyse_audio(audio.excerpts(path, PRE))
    except Exception:
        return None


def analyse_all(root: Path, rels: list[str], workers: int | None = None,
                progress_seconds: float = 15.0) -> tuple[dict[str, Result], dict]:
    """Key and energy for each track, from the cache where it can.

    Cached by audio hash and VERSION, like the embeddings: a rename costs
    nothing, and changing the method changes the version rather than mixing
    old answers with new ones.
    """
    root = root.resolve()
    ver = version()
    con = store.cache_db(root)
    by_path = dict(store.labels_db(root).execute("SELECT path, hash FROM tracks"))
    have = {h: Result(k, e) for h, k, e in con.execute(
        "SELECT hash, key, energy FROM analysis WHERE version=?", (ver,))}

    out: dict[str, Result] = {}
    todo = []
    for rel in rels:
        h = by_path.get(rel)
        if h is None:
            continue
        if h in have:
            out[rel] = have[h]
        else:
            todo.append(rel)
    stats = {"cached": len(out), "done": 0, "failed": 0}
    if not todo:
        return out, stats

    t0 = last = time.time()
    workers = workers or min(8, os.cpu_count() or 1)
    with ProcessPoolExecutor(workers) as ex:
        results = ex.map(analyse_file, [str(root / r) for r in todo], chunksize=2)
        for i, (rel, res) in enumerate(zip(todo, results), 1):
            if res is None:
                stats["failed"] += 1
            else:
                result, detail = res
                con.execute(
                    "INSERT OR REPLACE INTO analysis (hash, version, key, energy,"
                    " detail, ts) VALUES (?,?,?,?,?,?)",
                    (by_path[rel], ver, result.key, result.energy,
                     json.dumps(detail), int(time.time())))
                out[rel] = result
                stats["done"] += 1
            if i % 20 == 0:
                con.commit()
            now = time.time()
            if len(todo) > 50 and (now - last >= progress_seconds or i == len(todo)):
                last = now
                rate = i / (now - t0)
                print(f"      {i}/{len(todo)}  {rate:.1f}/s, "
                      f"~{int((len(todo) - i) / rate)}s left", flush=True)
    con.commit()
    return out, stats

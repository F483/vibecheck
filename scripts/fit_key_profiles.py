"""Fit the key templates, and measure the result against human key labels.

    uv run python scripts/fit_key_profiles.py TRAIN_DIR [--test TEST_DIR]

TRAIN_DIR is the Beatport EDM Key Dataset (https://zenodo.org/record/1101082,
CC BY-SA 4.0): audio/*.mp3 and keys/*.txt. TEST_DIR is the GiantSteps Key
Dataset (https://zenodo.org/record/1095691): audio/*.mp3 and keys_gs+/*.txt.
The two share no tracks. Neither is redistributed here; download them yourself.

What it does, all through keyenergy.py so the fit and the app cannot disagree:
  1. a pitch profile for every training track;
  2. templates -- one weight per interval above the tonic, per mode -- and the
     minor preference, fitted by multinomial logistic regression over the 24
     keys on tracks with one unambiguous key, so that the annotated key scores
     highest; the regularisation chosen by 5-fold cross-validation on the
     training set only;
  3. written to src/vibecheck/key_profiles.json;
  4. with --test: accuracy on the test set, once. Nothing is tuned on it.

Profiles are cached next to the audio (.profiles-<settings digest>.npz), so a
second run with the same settings takes seconds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from collections import Counter
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np
from scipy.optimize import minimize

from vibecheck import audio, keyenergy

PC = {"C": 0, "C#": 1, "Db": 1, "D": 2, "D#": 3, "Eb": 3, "E": 4, "F": 5,
      "F#": 6, "Gb": 6, "G": 7, "G#": 8, "Ab": 8, "A": 9, "A#": 10, "Bb": 10,
      "B": 11}
MINOR = {"minor", "dorian", "phrygian", "aeolian", "harmonic", "melodic",
         "locrian"}
MAJOR = {"major", "ionian", "mixolydian", "lydian"}
LAMBDAS = [1e-4, 1e-3, 1e-2]
OUT = Path(__file__).resolve().parents[1] / "src" / "vibecheck" / "key_profiles.json"


def parse(text: str) -> list[tuple[int, bool]]:
    """'E minor', 'F minor dorian', 'E major | E minor' -> [(tonic, minor)].

    Empty for no key ('X'), 'other', 'unknown' and anything unreadable: those
    tracks are left out rather than guessed at.
    """
    out = []
    for part in text.split("|"):
        words = part.split()
        if len(words) < 2 or words[0] not in PC:
            continue
        if words[1] in MINOR:
            out.append((PC[words[0]], True))
        elif words[1] in MAJOR:
            out.append((PC[words[0]], False))
    return out


def profile(path: str) -> np.ndarray | None:
    try:
        windows = audio.excerpts(path, keyenergy.PRE)
        spectra = [keyenergy._spectra(x.astype(np.float64))
                   for x in windows if len(x) >= keyenergy.N_FFT]
        return keyenergy.pitch_profile(spectra)
    except Exception:
        return None


def dataset(root: Path, keys_dir: str) -> tuple[np.ndarray, list]:
    """Profiles and labels for every track with a readable label."""
    labels = {}
    for f in (root / keys_dir).iterdir():
        if f.suffix == ".txt" and not f.name.startswith("._"):
            labels[f.stem] = parse(f.read_text(encoding="utf-8", errors="replace"))
    audio_files = sorted(p for p in (root / "audio").glob("*.mp3")
                         if not p.name.startswith("._"))
    k = keyenergy
    digest = hashlib.sha256(json.dumps([
        k.PRE.digest(), k.N_FFT, k.HOP, k.FMIN, k.FMAX, k.MAX_PEAKS,
        k.PEAK_FLOOR_DB, k.BINS, k.WIDTH, k.HARMONICS, k.HARMONIC_DECAY,
    ]).encode()).hexdigest()[:10]
    cache = root / f".profiles-{digest}.npz"
    if cache.exists():
        z = np.load(cache)
        have = dict(zip(z["names"], z["profiles"]))
    else:
        have = {}
    todo = [p for p in audio_files if p.stem not in have]
    if todo:
        print(f"  {len(todo)} profiles to compute in {root}", flush=True)
        with ProcessPoolExecutor(min(8, os.cpu_count() or 1)) as ex:
            for p, v in zip(todo, ex.map(profile, map(str, todo), chunksize=4)):
                if v is not None:
                    have[p.stem] = v
        np.savez(cache, names=np.array(list(have)), profiles=np.vstack(list(have.values())))
    names = [p.stem for p in audio_files if p.stem in have and labels.get(p.stem)]
    return np.vstack([have[n] for n in names]), [labels[n] for n in names]


def fit(P: np.ndarray, refs: list, lam: float):
    """Templates and minor bias maximising the annotated key's probability.

    The model is keyenergy.key_scores: score(tonic, mode) = template[mode] .
    rotated log-profile (+ bias for minor), softmax over the 24 keys, with an
    L2 penalty `lam` on the templates.
    """
    keep = [i for i, r in enumerate(refs) if len(r) == 1]
    F = np.log(P[keep] + keyenergy.LOG_FLOOR)
    R = np.stack([np.roll(F, -t, axis=1) for t in range(12)], 1)  # n x tonic x interval
    y = np.array([refs[i][0][0] + (12 if refs[i][0][1] else 0) for i in keep])
    n = len(y)

    def loss(th):
        S = np.concatenate([R @ th[:12], R @ th[12:24] + th[24]], 1)
        S = S - S.max(1, keepdims=True)
        lse = np.log(np.exp(S).sum(1))
        G = np.exp(S - lse[:, None])
        G[np.arange(n), y] -= 1
        G /= n
        grad = np.concatenate([np.einsum("nt,ntf->f", G[:, :12], R),
                               np.einsum("nt,ntf->f", G[:, 12:], R),
                               [G[:, 12:].sum()]])
        grad[:24] += 2 * lam * th[:24]
        return (lse - S[np.arange(n), y]).mean() + lam * (th[:24] ** 2).sum(), grad

    th = minimize(loss, np.zeros(25), jac=True, method="L-BFGS-B").x
    counts = Counter(bool(refs[i][0][1]) for i in keep)
    return th[:12], th[12:24], float(th[24]), counts


def score(est: tuple[int, bool], ref: tuple[int, bool]) -> tuple[str, float]:
    """MIREX key score: exact 1, fifth 0.5, relative 0.3, parallel 0.2."""
    (e, em), (r, rm) = est, ref
    if (e, em) == (r, rm):
        return "exact", 1.0
    if em == rm and (e - r) % 12 in (5, 7):
        return "fifth", 0.5
    if em != rm and (e + (3 if em else -3)) % 12 == r:
        return "relative", 0.3
    if e == r:
        return "parallel", 0.2
    return "other", 0.0


def evaluate(P, refs, major, minor, bias) -> dict:
    kinds: Counter = Counter()
    weighted = 0.0
    by_mode: dict = {True: [], False: []}
    for v, r in zip(P, refs):
        tonic, is_minor, _ = keyenergy.key_of(v, major, minor, bias)
        k, w = max((score((tonic, is_minor), a) for a in r), key=lambda x: x[1])
        kinds[k] += 1
        weighted += w
        by_mode[r[0][1]].append(k == "exact")
    n = len(refs)
    return {"n": n, "exact": kinds["exact"] / n, "weighted": weighted / n,
            "compatible": (kinds["exact"] + kinds["fifth"] + kinds["relative"]) / n,
            "minor_exact": float(np.mean(by_mode[True])) if by_mode[True] else 0.0,
            "major_exact": float(np.mean(by_mode[False])) if by_mode[False] else 0.0}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("train", type=Path)
    ap.add_argument("--test", type=Path)
    args = ap.parse_args()

    P, refs = dataset(args.train, "keys")
    print(f"training: {len(refs)} tracks with a readable key")

    fold = np.random.default_rng(0).integers(0, 5, len(refs))
    cv = {}
    for lam in LAMBDAS:
        right = 0.0
        for f in range(5):
            tr, te = fold != f, fold == f
            major, minor, bias, _ = fit(P[tr], [r for r, t in zip(refs, tr) if t], lam)
            right += evaluate(P[te], [r for r, t in zip(refs, te) if t],
                              major, minor, bias)["exact"] * te.sum()
        cv[lam] = right / len(refs)
    lam = max(cv, key=cv.get)
    print("cross-validated exact accuracy by regularisation: "
          + "  ".join(f"{k:g}:{a:.1%}" for k, a in cv.items()))

    major, minor, bias, n = fit(P, refs, lam)
    OUT.write_text(json.dumps({
        "about": "Key templates for keyenergy.py: per mode, one weight per "
                 "interval above the tonic, applied to the log pitch profile; "
                 "and the preference for minor keys. Fitted by multinomial "
                 "logistic regression on human-annotated tracks.",
        "source": "Beatport EDM Key Dataset, Angel Faraldo et al., "
                  "https://zenodo.org/record/1101082",
        "license": "Statistics derived from a dataset licensed CC BY-SA 4.0, "
                   "credited here.",
        "fitted_by": "scripts/fit_key_profiles.py",
        "algo": keyenergy.ALGO,
        "tracks": {"major": n[False], "minor": n[True]},
        "regularisation": lam,
        "minor_bias": round(bias, 6),
        "major": [round(x, 6) for x in major.tolist()],
        "minor": [round(x, 6) for x in minor.tolist()],
    }, indent=2) + "\n")
    print(f"wrote {OUT}  ({n[False]} major and {n[True]} minor tracks, "
          f"regularisation {lam:g}, minor bias {bias:.3f})")

    if args.test:
        Pt, rt = dataset(args.test, "keys_gs+")
        r = evaluate(Pt, rt, major, minor, bias)
        print(f"\ntest, {r['n']} tracks: exact {r['exact']:.1%}, MIREX weighted "
              f"{r['weighted']:.1%}, mix-compatible {r['compatible']:.1%}  "
              f"(minor {r['minor_exact']:.1%}, major {r['major_exact']:.1%})")
    return 0


if __name__ == "__main__":
    sys.exit(main())

"""Cue points: where a track changes, learned from your own cues, and where to
put cues around that.

Two halves, kept apart on purpose:

1. **Transition points** -- the bars where the song changes enough to mix
   around. Learned from the cues you set yourself, so it is your sense of
   where a track turns, not a rule. Four per track: a mix-in, two in between,
   a mix-out.

2. **Placement** -- plain settings, nothing learned: how many bars ahead of
   each transition point to cue (`lead`), which hot-cue slots are loops, and
   whether memory cues copy the hot cues. The default puts one cue *at* each
   transition. `lead = [16, 8]` puts a pair 16 and 8 bars ahead of it, for
   starting the next track a phrase or two before the change.

How a transition point is found:

- Bars come from rekordbox's own beat grid (the TEMPO entries in the xml), so
  every cue lands on a downbeat rekordbox agrees with. No beat or downbeat
  detection here.
- Each bar gets a summary of its sound: a 64-band spectrum, the twelve notes,
  loudness, brightness, onset strength.
- The track's sections are found by Laplacian structural segmentation of the
  bar-by-bar self-similarity matrix (McFee & Ellis 2014), at several section
  counts, and separately on the whole spectrum, the bass, the upper spectrum
  and the notes. Sections are clusters of bars that sound alike, wherever and
  however long they are -- not fixed windows compared with each other.
- A small bidirectional recurrent network reads every bar of the track at once
  and scores each bar as a transition, a mix-in and a mix-out. It is fitted to
  the transition points behind your own cues.

Measured on 646 of the author's tracks, held out by track, with cues placed as
16/8-bar pairs: 49% of hand-set cues are reproduced to the exact bar, and 63%
of tracks get at least half of their cues exactly right. A miss is most often
exactly 8 or 16 bars off: the right change, a different phrase (docs/cues.md).
"""

from __future__ import annotations

import hashlib
import io
import json
import os
import re
import time
from concurrent.futures import ProcessPoolExecutor
from importlib import resources
from pathlib import Path
from typing import NamedTuple

import numpy as np

from . import audio, store

ALGO = "cue1"

SR = 22050
N_FFT, HOP, N_MELS = 2048, 512, 64
KS = (3, 4, 5, 6, 8)            # section counts, whole spectrum + notes
VIEW_KS = (4, 6, 8)             # section counts, per view
TRANSITIONS = 4                 # mix-in, two between, mix-out
GAP = 8                         # bars between two transition points, at least
SLOTS = "ABCDEFGH"
CUE_RGB, LOOP_RGB = (40, 226, 20), (255, 140, 0)
MODEL_FILE = "cue_model.pt"

# Cues another tool generated: every mark named "Cue 1", "Cue 2", ... A cue set
# by hand in rekordbox has no name. A track whose cues are all generated may
# have them replaced; a track with any cue of yours is never touched.
GENERATED = re.compile(r"Cue \d+")


def version() -> str:
    """Everything that decides the per-bar features. Part of the cache key."""
    return ALGO + "/" + hashlib.sha256(json.dumps(
        [SR, N_FFT, HOP, N_MELS, KS, VIEW_KS]).encode()).hexdigest()[:12]


# --- the grid ----------------------------------------------------------------

Tempo = tuple[float, float, int]     # start s, bpm, beat in bar (1-4)


def downbeats(tempos: list[Tempo], duration: float) -> np.ndarray:
    """Every bar start of rekordbox's grid, in seconds.

    Each TEMPO entry runs until the next; the first is extended backwards to
    the start of the track, as rekordbox draws it.
    """
    times, pos = [], []
    for i, (ini, bpm, beat) in enumerate(tempos):
        end = tempos[i + 1][0] if i + 1 < len(tempos) else duration
        step = 60.0 / bpm
        for k in range(max(int(np.floor((end - ini) / step - 1e-6)) + 1, 0)):
            times.append(ini + k * step)
            pos.append((beat - 1 + k) % 4)
    ini, bpm, beat = tempos[0]
    step = 60.0 / bpm
    t, p, pre = ini - step, (beat - 2) % 4, []
    while t >= 0:
        pre.append((t, p))
        t, p = t - step, (p - 1) % 4
    pre.reverse()
    t = np.array([x for x, _ in pre] + times)
    p = np.array([y for _, y in pre] + pos)
    return t[p == 0]


def grid_digest(tempos: list[Tempo]) -> str:
    return hashlib.sha256(json.dumps(
        [[round(a, 3), round(b, 3), c] for a, b, c in tempos]).encode()).hexdigest()[:12]


def bar_time(down: np.ndarray, b: int) -> float:
    if b < len(down):
        return float(down[b])
    step = float(np.median(np.diff(down[-16:])))
    return float(down[-1] + (b - len(down) + 1) * step)


def bar_of(down: np.ndarray, seconds: float) -> int | None:
    """The bar a cue starts on, or None if it is not near a downbeat."""
    b = int(np.argmin(np.abs(down - seconds)))
    if abs(down[b] - seconds) > 0.6 * float(np.median(np.diff(down))):
        return None
    return b


# --- per-bar sound -----------------------------------------------------------

def bar_audio(y: np.ndarray, down: np.ndarray) -> dict[str, np.ndarray]:
    """Spectrum, notes, loudness, brightness and onsets, averaged per bar."""
    import librosa

    S = np.abs(librosa.stft(y, n_fft=N_FFT, hop_length=HOP)) ** 2
    mel = librosa.power_to_db(librosa.feature.melspectrogram(
        S=S, sr=SR, n_mels=N_MELS), ref=1.0)
    chroma = librosa.feature.chroma_stft(S=S, sr=SR)
    rms = librosa.power_to_db(S.sum(0) + 1e-10)
    cen = librosa.feature.spectral_centroid(S=np.sqrt(S), sr=SR)[0]
    ons = librosa.onset.onset_strength(S=mel, sr=SR, hop_length=HOP)
    ft = librosa.frames_to_time(np.arange(S.shape[1]), sr=SR, hop_length=HOP)
    edges = np.append(down, down[-1] + (down[-1] - down[-2]))
    idx = np.searchsorted(ft, edges)

    def per_bar(M):
        M = np.atleast_2d(M)
        return np.stack([M[:, a:max(b, a + 1)].mean(1)
                         for a, b in zip(idx[:-1], idx[1:])], 0)

    return {"mel": per_bar(mel), "chroma": per_bar(chroma),
            "rms": per_bar(rms)[:, 0], "centroid": per_bar(cen)[:, 0],
            "onset": per_bar(ons)[:, 0]}


def segment(X: np.ndarray, ks) -> dict[int, np.ndarray]:
    """Laplacian structural segmentation (McFee & Ellis 2014): a section label
    per bar, for each section count in ks."""
    import scipy.ndimage
    import scipy.sparse.csgraph
    from sklearn.cluster import KMeans

    n = len(X)
    Xn = (X - X.mean(0)) / (X.std(0) + 1e-6)
    d = ((Xn[:, None, :] - Xn[None, :, :]) ** 2).sum(-1)
    k = max(3, int(2 * np.ceil(np.sqrt(n))))
    sig = np.median(np.sort(d, 1)[:, 1:k + 1]) + 1e-9
    W = np.exp(-d / sig)
    # recurrence: mutual nearest neighbours only
    nn_ = np.argsort(d, 1)[:, 1:k + 1]
    M = np.zeros_like(W, dtype=bool)
    M[np.arange(n)[:, None], nn_] = True
    M &= M.T
    R = scipy.ndimage.median_filter(W * M, size=(1, 3))
    # path: each bar to the next
    step = np.diag(W, 1)
    P = np.diag(step, 1) + np.diag(step, -1)
    dr, dp = R.sum(1), P.sum(1)
    mu = dp.dot(dp + dr) / (((dp + dr) ** 2).sum() + 1e-9)
    L = scipy.sparse.csgraph.laplacian(mu * R + (1 - mu) * P, normed=True)
    _, ev = np.linalg.eigh(L)
    ev = scipy.ndimage.median_filter(ev, size=(5, 1))
    out = {}
    for kk in ks:
        Y = ev[:, :kk] / (np.linalg.norm(ev[:, :kk], axis=1, keepdims=True) + 1e-9)
        out[kk] = KMeans(kk, n_init=4, random_state=0).fit_predict(Y)
    return out


def features(bars: dict[str, np.ndarray]) -> np.ndarray:
    """bars x features: where the bar sits, how it sounds relative to the track,
    and how it relates to the track's sections."""
    mel, chroma = bars["mel"], bars["chroma"]
    rms, cen, ons = bars["rms"], bars["centroid"], bars["onset"]
    ok = ~(np.isnan(mel).any(1) | np.isnan(chroma).any(1) | np.isnan(rms)
           | np.isnan(cen) | np.isnan(ons))
    n = int(np.flatnonzero(ok).max()) + 1          # drop bars past the audio
    mel, chroma, rms, cen, ons = (
        np.where(np.isnan(v[:n]), np.nanmedian(v[:n], 0), v[:n])
        for v in (mel, chroma, rms, cen, ons))
    b = np.arange(n)
    med = lambda v: v - np.median(v)
    low, high = mel[:, :8].mean(1), mel[:, 40:].mean(1)
    F: dict[str, np.ndarray] = {
        "pos": b / n, "bar": b.astype(float), "to_end": (n - b).astype(float),
        "nbars": np.full(n, float(n)), "phase32mod": (b % 32).astype(float),
        "rms": med(rms), "low": med(low), "high": med(high),
        "cen": med(np.log(cen + 1)), "ons": ons / (np.median(ons) + 1e-9)}
    for p in (4, 8, 16, 32):
        F[f"phase{p}"] = (b % p == 0).astype(float)

    nb = np.zeros(n)
    for kk, lab in segment(np.hstack([mel, chroma * 10]), KS).items():
        bd = np.r_[True, lab[1:] != lab[:-1]]
        nb += bd
        F[f"dist{kk}"] = np.min(np.abs(b[:, None] - np.flatnonzero(bd)[None, :]), 1)
        sid = np.cumsum(bd) - 1
        s_rms = np.array([rms[sid == s].mean() for s in range(sid.max() + 1)])
        s_low = np.array([low[sid == s].mean() for s in range(sid.max() + 1)])
        F[f"seg_rms{kk}"] = s_rms[sid] - np.median(rms)
        F[f"seg_low{kk}"] = s_low[sid] - np.median(low)
        F[f"seg_rise{kk}"] = (s_rms - np.r_[s_rms[0], s_rms[:-1]])[sid] * bd
        F[f"seg_lowrise{kk}"] = (s_low - np.r_[s_low[0], s_low[:-1]])[sid] * bd
        F[f"seg_first{kk}"] = (sid == 0).astype(float)
    F["nbound"] = nb

    views = {"bass": mel[:, :12], "timbre": mel[:, 12:], "notes": chroma * 10}
    for v, X in views.items():
        vb = np.zeros(n)
        for kk, lab in segment(X, VIEW_KS).items():
            bd = np.r_[True, lab[1:] != lab[:-1]]
            vb += bd
            F[f"{v}_dist{kk}"] = np.min(np.abs(b[:, None] - np.flatnonzero(bd)[None, :]), 1)
        F[f"{v}_nbound"] = vb

    bands = mel.reshape(n, 8, 8).mean(2)
    cols = [F[k] for k in sorted(F)]
    return np.hstack([np.stack(cols, 1), bands - np.median(bands, 0),
                      chroma - chroma.mean(0)]).astype(np.float32)


# --- the network -------------------------------------------------------------

def _net(d: int, h: int = 64):
    import torch.nn as nn

    class Net(nn.Module):
        """Every bar of a track at once; per bar: transition, mix-in, mix-out."""
        def __init__(self):
            super().__init__()
            self.inp = nn.Sequential(nn.Linear(d, h), nn.GELU(), nn.Dropout(0.2))
            self.rnn = nn.GRU(h, h, num_layers=2, batch_first=True,
                              bidirectional=True, dropout=0.2)
            self.out = nn.Linear(2 * h, 3)

        def forward(self, x):
            return self.out(self.rnn(self.inp(x))[0])

    return Net()


def pick(score: np.ndarray, k: int, fixed=()) -> list[int]:
    """The k best bars, at least GAP apart, keeping any already fixed."""
    out = list(fixed)
    for b in np.argsort(-score):
        if len(out) >= k:
            break
        if all(abs(int(b) - o) >= GAP for o in out):
            out.append(int(b))
    return sorted(out)


def decode(scores: np.ndarray, k: int = TRANSITIONS) -> list[int]:
    """Mix-in first, mix-out after it, the rest from the transition score."""
    n = len(scores)
    t, f_, l_ = scores[:, 0], scores[:, 1], scores[:, 2]
    f = int(np.argmax(f_))
    if f + GAP >= n:
        return pick(t, k)
    lo = l_.copy()
    lo[:f + GAP] = -np.inf
    last = int(np.argmax(lo))
    mid = t.copy()
    mid[:f + GAP] = -np.inf
    mid[max(last - GAP + 1, 0):] = -np.inf
    return pick(mid, k, fixed=(f, last))


class Model:
    """An ensemble of networks plus the input scaling they were trained with."""

    def __init__(self, state: dict):
        import torch

        self.state = state
        self.nets = []
        for sd in state["nets"]:
            m = _net(state["d"])
            m.load_state_dict(sd)
            m.eval()
            self.nets.append(m)
        self.mu = state["mu"].numpy() if hasattr(state["mu"], "numpy") else state["mu"]
        self.sd = state["sd"].numpy() if hasattr(state["sd"], "numpy") else state["sd"]
        self.torch = torch

    @property
    def n_tracks(self) -> int:
        return int(self.state["n_tracks"])

    @property
    def source(self) -> str:
        return str(self.state["source"])

    def scores(self, F: np.ndarray) -> np.ndarray:
        torch = self.torch
        x = torch.tensor((F - self.mu) / self.sd, dtype=torch.float32)[None]
        with torch.no_grad():
            o = np.mean([m(x)[0].numpy() for m in self.nets], 0)
        return o

    def transitions(self, F: np.ndarray) -> list[int]:
        return decode(self.scores(F))

    def save(self, path: Path) -> None:
        import torch
        torch.save(self.state, path)

    @classmethod
    def load(cls, root: Path | None = None) -> "Model | None":
        """Yours, once you have trained one; otherwise the one shipped; None
        if there is neither."""
        import torch

        own = store.vibecheck_dir(root) / MODEL_FILE if root else None
        shipped = resources.files(__package__).joinpath(MODEL_FILE)
        if own and own.exists():
            src = own.read_bytes()
        elif shipped.is_file():
            src = shipped.read_bytes()
        else:
            return None
        return cls(torch.load(io.BytesIO(src), weights_only=True))


def fit(data: list[tuple[np.ndarray, list[int]]], source: str, seeds: int = 3,
        epochs: int = 60, batch: int = 16, progress=print) -> Model:
    """Train on (features, transition bars) per track."""
    import torch
    import torch.nn.functional as fn

    A = [F for F, _ in data]
    mu = np.vstack(A).mean(0)
    sd = np.vstack(A).std(0) + 1e-6
    X = [torch.tensor((a - mu) / sd, dtype=torch.float32) for a in A]
    T = [t for _, t in data]
    d = A[0].shape[1]
    torch.set_num_threads(min(8, os.cpu_count() or 1))
    nets = []
    for seed in range(seeds):
        t0 = time.time()
        torch.manual_seed(seed)
        m = _net(d)
        opt = torch.optim.AdamW(m.parameters(), 2e-3, weight_decay=1e-2)
        for ep in range(epochs):
            m.train()
            order = np.random.default_rng(seed * 100 + ep).permutation(len(X))
            for j in range(0, len(order), batch):
                b = order[j:j + batch]
                L = max(len(X[i]) for i in b)
                x = torch.zeros(len(b), L, d)
                mask = torch.zeros(len(b), L, dtype=torch.bool)
                yt = torch.zeros(len(b), L)
                for r, i in enumerate(b):
                    x[r, :len(X[i])] = X[i]
                    mask[r, :len(X[i])] = True
                    yt[r, T[i]] = 1
                o = m(x)
                lt = fn.binary_cross_entropy_with_logits(
                    o[..., 0], yt, pos_weight=torch.tensor(20.0),
                    reduction="none")[mask].mean()
                lf = fn.cross_entropy(o[..., 1].masked_fill(~mask, -1e9),
                                      torch.tensor([T[i][0] for i in b]))
                ll = fn.cross_entropy(o[..., 2].masked_fill(~mask, -1e9),
                                      torch.tensor([T[i][-1] for i in b]))
                opt.zero_grad()
                (lt + lf + ll).backward()
                opt.step()
        m.eval()
        nets.append(m.state_dict())
        progress(f"      network {seed + 1}/{seeds} trained ({time.time() - t0:.0f}s)")
    return Model({"nets": nets, "mu": torch.tensor(mu), "sd": torch.tensor(sd),
                  "d": d, "n_tracks": len(data), "source": source,
                  "version": version()})


# --- your cues, both ways ----------------------------------------------------

class Marks(NamedTuple):
    """A track as the rekordbox xml has it: grid, length and every cue."""
    tempos: list[Tempo]
    duration: float
    marks: list[dict]        # POSITION_MARK attributes, as strings


def generated(marks: list[dict]) -> bool:
    """Every cue on the track was named by a tool, none set by hand."""
    return bool(marks) and all(GENERATED.fullmatch(m.get("Name", "")) for m in marks)


def writable(t: Marks) -> bool:
    """Nothing of yours to lose: no cues, only generated ones, or a lone memory
    cue on the first downbeat -- what clearing generated cues leaves behind,
    since rekordbox ignores an empty set on import, and what a placed set
    starts with anyway."""
    m = t.marks
    if not m or generated(m):
        return True
    if len(m) == 1 and int(m[0].get("Num", 0)) == -1 and t.tempos:
        first = downbeats(t.tempos, t.duration + 2.0)[:1]
        return len(first) == 1 and abs(float(m[0]["Start"]) - first[0]) < 0.01
    return False


def hot_starts(marks: list[dict]) -> list[float]:
    return sorted(round(float(m["Start"]), 2) for m in marks
                  if int(m.get("Num", -1)) >= 0)


def transitions_from(bars: list[int], lead: list[int]) -> list[int]:
    """The transition points a set of cue bars was placed around.

    With lead [16, 8], a cue at bar c and another at c + 8 mean a transition
    at c + 16. Cues not part of such a group say nothing and are dropped.
    """
    have, used, out = set(bars), set(), []
    lead = sorted(lead, reverse=True)
    for c in sorted(bars):
        t = c + lead[0]
        group = {t - l for l in lead}
        if c in used or not group <= have or group & used:
            continue
        used |= group
        out.append(t)
    return sorted(out)


def place(T: list[int], down: np.ndarray, lead: list[int],
          loops: dict[str, int], memory: bool) -> list[dict]:
    """POSITION_MARK attributes for transition points T.

    Each transition gets one hot cue per lead, in consecutive slots. The first
    takes the front slots (A, B for a pair), the last the back ones (G, H), the
    rest go between in time order. Memory cues copy the hot cues, plus one on
    the first downbeat.
    """
    w = len(lead)
    T = sorted(T)
    if len(T) * w > len(SLOTS):
        T = T[:len(SLOTS) // w - 1] + T[-1:]
    slots = []
    for j, t in enumerate(T):
        last = j == len(T) - 1 and len(T) > 1
        base = len(SLOTS) - w if last and w > 1 else j * w
        slots += [(base + i, t - lead[i]) for i in range(w)]

    hot, seen = [], set()
    for num, b in slots:
        b = max(b, 0)
        if b in seen:
            continue
        seen.add(b)
        loop = loops.get(SLOTS[num])
        m = {"Name": "", "Type": "4" if loop else "0",
             "Start": f"{bar_time(down, b):.3f}", "Num": str(num)}
        if loop:
            m["End"] = f"{bar_time(down, b + loop):.3f}"
        r, g, bl = LOOP_RGB if loop else CUE_RGB
        hot.append(m | {"Red": str(r), "Green": str(g), "Blue": str(bl)})
    if not memory:
        return hot
    first = {"Name": "", "Type": "0", "Start": f"{bar_time(down, 0):.3f}", "Num": "-1"}
    copies = [{k: v for k, v in m.items() if k not in ("Red", "Green", "Blue")}
              | {"Num": "-1"} for m in hot]
    return hot + [first] + [c for c in copies if c["Start"] != first["Start"]]


# --- many tracks -------------------------------------------------------------

def _job(args) -> tuple[str, np.ndarray | None]:
    import warnings

    # empty bars past the audio and near-silent sections are expected, and
    # dealt with in features(); their numpy and sklearn warnings are noise
    warnings.simplefilter("ignore")
    rel, path, tempos, duration = args
    try:
        y = audio.decode(path, 0.0, duration + 2.0, SR)
        if len(y) < SR * 30:
            return rel, None
        down = downbeats(tempos, len(y) / SR)
        if len(down) < 16:
            return rel, None
        return rel, features(bar_audio(y, down))
    except Exception:
        return rel, None


def features_all(root: Path, tracks: dict[str, Marks], workers: int | None = None,
                 progress_seconds: float = 15.0) -> tuple[dict[str, np.ndarray], dict]:
    """Per-bar features for each track, from the cache where it can.

    Cached by audio hash, method version and grid: moving the grid in
    rekordbox moves every bar, so it is a different answer.
    """
    root = root.resolve()
    ver = version()
    con = store.cache_db(root)
    con.execute("""CREATE TABLE IF NOT EXISTS cue_bars (
        hash TEXT NOT NULL, version TEXT NOT NULL, grid TEXT NOT NULL,
        bars INTEGER NOT NULL, dim INTEGER NOT NULL, data BLOB NOT NULL,
        ts INTEGER NOT NULL, PRIMARY KEY (hash, version, grid))""")
    by_path = dict(store.labels_db(root).execute("SELECT path, hash FROM tracks"))
    out, todo = {}, []
    for rel, t in tracks.items():
        h = by_path.get(rel)
        if h is None or not t.tempos:
            continue
        row = con.execute("SELECT bars, dim, data FROM cue_bars WHERE hash=? AND "
                          "version=? AND grid=?",
                          (h, ver, grid_digest(t.tempos))).fetchone()
        if row:
            out[rel] = np.frombuffer(row[2], np.float32).reshape(row[0], row[1])
        else:
            todo.append(rel)
    stats = {"cached": len(out), "done": 0, "failed": 0}
    if not todo:
        return out, stats

    t0 = last = time.time()
    jobs = [(rel, str(root / rel), tracks[rel].tempos, tracks[rel].duration)
            for rel in todo]
    with ProcessPoolExecutor(workers or min(8, os.cpu_count() or 1)) as ex:
        for i, (rel, F) in enumerate(ex.map(_job, jobs, chunksize=2), 1):
            if F is None:
                stats["failed"] += 1
            else:
                con.execute("INSERT OR REPLACE INTO cue_bars VALUES (?,?,?,?,?,?,?)",
                            (by_path[rel], ver, grid_digest(tracks[rel].tempos),
                             F.shape[0], F.shape[1], F.tobytes(), int(time.time())))
                out[rel] = F
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


def yours(tracks: dict[str, Marks], written: dict[str, list[float]],
          lead: list[int]) -> dict[str, list[int]]:
    """Tracks whose cues you set, and the transition points behind them.

    Generated cues are not yours. Neither are cues this app wrote that you have
    left as they were: learning from those would be learning from itself. Once
    you move one, the track is yours again.
    """
    out = {}
    for rel, t in tracks.items():
        if not t.tempos or not any(int(m.get("Num", -1)) >= 0 for m in t.marks):
            continue
        if generated(t.marks):
            continue
        if rel in written and hot_starts(t.marks) == written[rel]:
            continue
        down = downbeats(t.tempos, t.duration)
        if len(down) < 16:
            continue
        bars = {bar_of(down, float(m["Start"])) for m in t.marks
                if int(m.get("Num", -1)) >= 0}
        T = transitions_from(sorted(b for b in bars if b is not None), lead)
        if len(T) >= 3:
            out[rel] = T
    return out

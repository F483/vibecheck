"""Embed with MuQ from the isolated venv (.venv-muq, transformers 4.x).

Writes into the same cache.db as every other backend, under the same key
scheme, so evaluation in the main environment is unchanged.
"""

from __future__ import annotations

import sys
import time
from pathlib import Path

import numpy as np
import torch

sys.path.insert(0, "src")
from vibecheck import audio, store  # noqa: E402  (numpy + sqlite3 + ffmpeg only)
from vibecheck.backends.muq import MODEL_ID, MuQ  # noqa: E402
from vibecheck.config import DEFAULT  # noqa: E402


def load_model(device: str):
    from muq import MuQ as MuQNet

    m = MuQNet.from_pretrained(MODEL_ID).eval().to(device)
    return m


def embed_track(m, path: str, cfg, device: str) -> np.ndarray:
    # All 9 windows in one forward pass: ~18% faster than one at a time, since
    # the GPU is already saturated and this only removes launch overhead.
    # fp16 would be the real win but crashes MPS with a Metal assertion.
    ws = audio.excerpts(path, cfg)
    # Short files (loop packs) yield windows of unequal length; batching needs a
    # rectangular array, so trim to the shortest. Sequential inference tolerated
    # ragged windows, batching does not.
    n = min(len(w) for w in ws)
    windows = np.stack([w[:n] for w in ws])
    x = torch.from_numpy(windows).float().to(device)
    with torch.inference_mode():
        out = m(x, output_hidden_states=True)
    # mean over time for each layer, then mean over layers: MuQ exposes all
    # 13, and probes generally do better from more than the final one
    layers = torch.stack([h.mean(dim=1) for h in out.hidden_states])
    vecs = layers.mean(dim=0).float().cpu().numpy()
    if device == "mps":
        torch.mps.empty_cache()
    v = np.mean(vecs, axis=0)
    n = np.linalg.norm(v)
    return ((v / n) if n else v).astype(np.float32)


def main(limit: int | None, which: str = "labelled") -> int:
    root = Path.home() / "Music" / "Collection"
    b = MuQ()
    cfg = b.preproc(DEFAULT)
    lab = store.labels_db(root)
    cache = store.cache_db(root)

    labels = store.current_labels(lab, source="user")
    if which == "subset":
        frozen = Path("scripts/phase0_subset.txt")
        keep = {l for l in frozen.read_text().splitlines() if l}
        labels = {p: v for p, v in labels.items() if p in keep}
    hash_by_path = dict(lab.execute("SELECT path, hash FROM tracks"))
    have = {h for (h,) in cache.execute(
        "SELECT hash FROM embeddings WHERE backend=? AND version=? AND preproc=?",
        (b.name, b.version, cfg.digest()))}
    todo = [p for p in sorted(labels)
            if hash_by_path.get(p) and hash_by_path[p] not in have]
    remaining = len(todo)
    if limit:
        todo = todo[:limit]
    print(f"muq: {remaining} to embed, doing {len(todo)}", flush=True)
    if not todo:
        print("CHUNK done 0 remaining 0")
        return 0

    device = "mps" if torch.backends.mps.is_available() else "cpu"
    m = load_model(device)
    t0 = time.time()
    done = failed = 0
    for i, rel in enumerate(todo, 1):
        try:
            v = embed_track(m, str(root / rel), cfg, device)
        except Exception as e:
            failed += 1
            print(f"  failed {rel[:60]}: {type(e).__name__}", flush=True)
            continue
        cache.execute(
            "INSERT OR REPLACE INTO embeddings (hash, backend, version, preproc,"
            " dim, vec, ts) VALUES (?,?,?,?,?,?,?)",
            (hash_by_path[rel], b.name, b.version, cfg.digest(), v.shape[0],
             v.tobytes(), int(time.time())))
        done += 1
        if i % 50 == 0:
            cache.commit()
            r = i / (time.time() - t0)
            print(f"  {i}/{len(todo)}  {r:.1f}/s  eta {(len(todo)-i)/r/60:.1f}min",
                  flush=True)
    cache.commit()
    print(f"CHUNK done {done} failed {failed} remaining {remaining - done}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(int(sys.argv[1]) if len(sys.argv) > 1 else None,
                          sys.argv[2] if len(sys.argv) > 2 else "labelled"))

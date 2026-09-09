"""Batch embedding with a content-addressed cache.

The cache is the reason a backend comparison is affordable: embed once, then
re-run evaluation as often as you like.
"""

from __future__ import annotations

import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np

from . import audio, backends, store
from .config import Preproc

_W: dict = {}


def _init(backend_name: str, cfg: Preproc) -> None:
    _W["backend"] = backends.get(backend_name)
    _W["cfg"] = cfg


def _work(item: tuple[str, str]) -> tuple[str, np.ndarray | None, str | None]:
    rel, abspath = item
    try:
        xs = audio.excerpts(abspath, _W["cfg"])
        return rel, _W["backend"].embed(xs, _W["cfg"]), None
    except Exception as e:  # a corrupt file must not kill the run
        return rel, None, f"{type(e).__name__}: {e}"


def embed_all(root: Path, backend_name: str, cfg: Preproc, paths: list[str],
              workers: int = 8, progress_every: int = 250) -> dict[str, int]:
    root = root.resolve()
    b = backends.get(backend_name)
    con = store.cache_db(root)
    lab = store.labels_db(root)

    hash_by_path = dict(lab.execute("SELECT path, hash FROM tracks"))
    have = {
        h for (h,) in con.execute(
            "SELECT hash FROM embeddings WHERE backend=? AND version=? AND preproc=?",
            (b.name, b.version, cfg.digest()))
    }
    todo = [p for p in paths if hash_by_path.get(p) and hash_by_path[p] not in have]

    stats = {"total": len(paths), "cached": len(paths) - len(todo), "done": 0, "failed": 0}
    if not todo:
        return stats

    t0 = time.time()
    items = [(p, str(root / p)) for p in todo]
    with ProcessPoolExecutor(workers, initializer=_init,
                             initargs=(backend_name, cfg)) as ex:
        for i, (rel, vec, err) in enumerate(ex.map(_work, items, chunksize=8), 1):
            if vec is None:
                stats["failed"] += 1
                continue
            con.execute(
                "INSERT OR REPLACE INTO embeddings "
                "(hash, backend, version, preproc, dim, vec, ts) VALUES (?,?,?,?,?,?,?)",
                (hash_by_path[rel], b.name, b.version, cfg.digest(),
                 vec.shape[0], vec.astype(np.float32).tobytes(), int(time.time())),
            )
            stats["done"] += 1
            if i % progress_every == 0:
                con.commit()
                rate = i / (time.time() - t0)
                eta = (len(items) - i) / rate / 60
                print(f"  {i}/{len(items)}  {rate:.1f}/s  eta {eta:.1f}min", flush=True)

    con.commit()
    con.close()
    return stats


def load(root: Path, backend_name: str, cfg: Preproc,
         paths: list[str]) -> tuple[list[str], np.ndarray]:
    root = root.resolve()
    b = backends.get(backend_name)
    lab = store.labels_db(root)
    con = store.cache_db(root)
    hash_by_path = dict(lab.execute("SELECT path, hash FROM tracks"))
    rows = {
        h: np.frombuffer(v, dtype=np.float32)
        for h, v in con.execute(
            "SELECT hash, vec FROM embeddings WHERE backend=? AND version=? AND preproc=?",
            (b.name, b.version, cfg.digest()))
    }
    keep, vecs = [], []
    for p in paths:
        v = rows.get(hash_by_path.get(p, ""))
        if v is not None:
            keep.append(p)
            vecs.append(v)
    return keep, (np.vstack(vecs) if vecs else np.zeros((0, 0), np.float32))

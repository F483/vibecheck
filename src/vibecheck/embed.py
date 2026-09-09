"""Batch embedding with a content-addressed cache.

The cache is the reason a backend comparison is affordable: embed once, then
re-run evaluation as often as you like.
"""

from __future__ import annotations

import resource
import time
from concurrent.futures import ProcessPoolExecutor
from pathlib import Path

import numpy as np

from . import audio, backends, store
from .config import Preproc

_W: dict = {}


def _init(backend_name: str, cfg: Preproc) -> None:
    b = backends.get(backend_name)
    _W["backend"] = b
    _W["cfg"] = b.preproc(cfg)


def _work(item: tuple[str, str]) -> tuple[str, np.ndarray | None, str | None]:
    rel, abspath = item
    try:
        xs = audio.excerpts(abspath, _W["cfg"])
        return rel, _W["backend"].embed(xs, _W["cfg"]), None
    except Exception as e:  # a corrupt file must not kill the run
        return rel, None, f"{type(e).__name__}: {e}"


def embed_all(root: Path, backend_name: str, cfg: Preproc, paths: list[str],
              workers: int = 8, progress_every: int = 250,
              max_tasks_per_child: int = 150,
              limit: int | None = None) -> dict[str, int]:
    """`max_tasks_per_child` recycles the worker periodically.

    Neural backends grow their memory over a long run -- the MPS caching
    allocator fragments and `empty_cache()` does not fully give it back -- and
    on a 16 GB machine that eventually gets the process killed. Restarting the
    worker every N tracks bounds it for the price of reloading the model.
    """
    root = root.resolve()
    b = backends.get(backend_name)
    cfg = b.preproc(cfg)
    con = store.cache_db(root)
    lab = store.labels_db(root)

    hash_by_path = dict(lab.execute("SELECT path, hash FROM tracks"))
    have = {
        h for (h,) in con.execute(
            "SELECT hash FROM embeddings WHERE backend=? AND version=? AND preproc=?",
            (b.name, b.version, cfg.digest()))
    }
    todo = [p for p in paths if hash_by_path.get(p) and hash_by_path[p] not in have]
    # count what is genuinely cached before `limit` truncates the work list,
    # or the stats claim everything not scheduled this run is already done
    stats = {"total": len(paths), "cached": len(paths) - len(todo),
             "remaining": len(todo), "done": 0, "failed": 0}
    if limit:
        todo = todo[:limit]
    if not todo:
        return stats

    t0 = time.time()
    items = [(p, str(root / p)) for p in todo]

    if workers <= 1:
        # Inline. A pool of one buys nothing, and worse: when the OS kills the
        # worker (which happens on a 16 GB machine running a transformer),
        # ProcessPoolExecutor hangs instead of raising, so the run silently
        # stops making progress. Inline means an OOM kill takes down the whole
        # process visibly, and the cache makes the next invocation resume.
        _init(backend_name, cfg)
        results = (_work(it) for it in items)
    else:
        pool = ProcessPoolExecutor(workers, initializer=_init,
                                   initargs=(backend_name, cfg),
                                   max_tasks_per_child=max_tasks_per_child)
        results = pool.map(_work, items, chunksize=8)

    try:
        for i, (rel, vec, err) in enumerate(results, 1):
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
                rss = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss / 1e9
                print(f"  {i}/{len(items)}  {rate:.1f}/s  eta {eta:.1f}min  "
                      f"rss {rss:.1f}GB", flush=True)
    finally:
        if workers > 1:
            pool.shutdown()

    con.commit()
    con.close()
    return stats


def load(root: Path, backend_name: str, cfg: Preproc,
         paths: list[str]) -> tuple[list[str], np.ndarray]:
    root = root.resolve()
    b = backends.get(backend_name)
    cfg = b.preproc(cfg)
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

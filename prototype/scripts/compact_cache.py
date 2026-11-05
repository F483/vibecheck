"""Compact the embedding cache.

clapw v1 stored all 24 window vectors per track (12,288 floats). The research
that needed them is done, and it concluded that pooling is as good as training
on windows individually, so v2 stores mean+std (1,024 floats) instead -- a 12x
reduction. Also drops rows from superseded backend versions.

Verifies before deleting anything: the cache is regenerable but costs ~10 hours.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np

sys.path.insert(0, "src")
from vibecheck import store  # noqa: E402

STALE = [("mert", "1"), ("mfcc", "1")]


def main(apply: bool) -> int:
    root = (Path.home() / "Music" / "Collection").resolve()
    con = store.cache_db(root)
    before = Path(store.vibecheck_dir(root) / "cache.db").stat().st_size

    rows = con.execute(
        "SELECT hash, preproc, vec, ts FROM embeddings "
        "WHERE backend='clapw' AND version='1'").fetchall()
    print(f"clapw v1 rows to convert: {len(rows)}")
    if apply and rows:
        for h, pp, vec, ts in rows:
            w = np.frombuffer(vec, dtype=np.float32).reshape(24, 512)
            v = np.concatenate([w.mean(0), w.std(0)]).astype(np.float32)
            con.execute(
                "INSERT OR REPLACE INTO embeddings (hash, backend, version, "
                "preproc, dim, vec, ts) VALUES (?,?,?,?,?,?,?)",
                (h, "clapw", "2", pp, v.shape[0], v.tobytes(), ts))
        con.commit()
        made = con.execute("SELECT COUNT(*) FROM embeddings WHERE backend='clapw' "
                           "AND version='2'").fetchone()[0]
        if made != len(rows):
            print(f"ABORT: converted {made} of {len(rows)}; nothing deleted")
            return 1
        con.execute("DELETE FROM embeddings WHERE backend='clapw' AND version='1'")
        print(f"converted {made}, deleted v1")

    for b, v in STALE:
        n = con.execute("SELECT COUNT(*) FROM embeddings WHERE backend=? AND version=?",
                        (b, v)).fetchone()[0]
        print(f"stale {b} v{v}: {n} rows")
        if apply and n:
            con.execute("DELETE FROM embeddings WHERE backend=? AND version=?", (b, v))
    con.commit()

    if apply:
        con.execute("VACUUM")
        con.close()
        after = Path(store.vibecheck_dir(root) / "cache.db").stat().st_size
        print(f"\ncache.db {before/1e6:.0f} MB -> {after/1e6:.0f} MB")
    else:
        print("\n(dry run -- pass --apply)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main("--apply" in sys.argv))

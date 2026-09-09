"""Migrate the stores to tag-independent audio hashes.

The old hash covered a fixed 256 KB head window, which includes the ID3v2 tag.
Every tag write therefore changed a track's identity and orphaned its cached
embedding. This recomputes identity from the audio region only and remaps the
existing rows so nothing has to be re-embedded.
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

from vibecheck import store

EMB_COLS = "hash, backend, version, preproc, dim, vec, ts"


def main(root: Path) -> int:
    root = root.resolve()
    d = store.vibecheck_dir(root)
    for name in ("labels.db", "cache.db"):
        shutil.copy2(d / name, d / f"{name}.bak")
    print(f"backed up to {d}/*.bak")

    lab = store.labels_db(root)
    rows = lab.execute("SELECT path, hash, size FROM tracks").fetchall()

    remap: dict[str, str] = {}
    missing = 0
    for i, (rel, old, size) in enumerate(rows, 1):
        f = root / rel
        if not f.exists():
            missing += 1
            continue
        try:
            new = store.partial_hash(f, f.stat().st_size)
        except OSError:
            missing += 1
            continue
        if new != old:
            remap[old] = new
        if i % 2000 == 0:
            print(f"  hashed {i}/{len(rows)}", flush=True)
    print(f"tracks {len(rows)}, changed {len(remap)}, unreadable {missing}")

    cache = store.cache_db(root)
    cache.execute(f"CREATE TABLE IF NOT EXISTS embeddings_new AS SELECT {EMB_COLS} "
                  "FROM embeddings WHERE 0")
    cache.execute("CREATE UNIQUE INDEX IF NOT EXISTS embeddings_new_pk "
                  "ON embeddings_new (hash, backend, version, preproc)")
    moved = 0
    for h, b, v, pp, dim, vec, ts in cache.execute(f"SELECT {EMB_COLS} FROM embeddings"):
        nh = remap.get(h, h)
        moved += nh != h
        cache.execute(
            "INSERT OR REPLACE INTO embeddings_new (hash, backend, version, preproc, "
            "dim, vec, ts) VALUES (?,?,?,?,?,?,?)", (nh, b, v, pp, dim, vec, ts))
    cache.execute("DROP TABLE embeddings")
    cache.execute("ALTER TABLE embeddings_new RENAME TO embeddings")
    cache.commit()
    print(f"embeddings remapped: {moved}")

    for table in ("tracks", "label_log"):
        for old, new in remap.items():
            lab.execute(f"UPDATE {table} SET hash=? WHERE hash=?", (new, old))
    lab.commit()
    print("labels.db updated")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(Path(sys.argv[1]) if len(sys.argv) > 1
                          else Path.home() / "Music" / "Collection"))

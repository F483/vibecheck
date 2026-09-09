"""State lives in the collection root, under .vibecheck/ (design.md 7).

Two files on purpose: labels.db is hours of listening and irreplaceable;
cache.db is CPU time and can be deleted whenever a backend changes.
"""

from __future__ import annotations

import hashlib
import os
import sqlite3
import time
from pathlib import Path

HEAD_TAIL_BYTES = 256 * 1024


def vibecheck_dir(root: Path) -> Path:
    d = root / ".vibecheck"
    d.mkdir(exist_ok=True)
    return d


def _connect(path: Path) -> sqlite3.Connection:
    con = sqlite3.connect(path)
    # rollback journal, not WAL: one process, batch writes, and a single file
    # at rest instead of -wal/-shm litter in the collection (design.md 7)
    con.execute("PRAGMA journal_mode=DELETE")
    con.execute("PRAGMA synchronous=FULL")
    return con


def labels_db(root: Path) -> sqlite3.Connection:
    con = _connect(vibecheck_dir(root) / "labels.db")
    con.executescript("""
        CREATE TABLE IF NOT EXISTS tracks (
          path     TEXT PRIMARY KEY,   -- relative to collection root
          hash     TEXT NOT NULL,
          size     INTEGER NOT NULL,
          mtime    REAL NOT NULL,
          seen_at  INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS tracks_hash ON tracks(hash);

        -- append-only: never UPDATE, never DELETE. Newest row per path wins.
        CREATE TABLE IF NOT EXISTS label_log (
          id     INTEGER PRIMARY KEY,
          path   TEXT NOT NULL,
          hash   TEXT NOT NULL,
          label  TEXT,                 -- NULL = label removed
          source TEXT NOT NULL,        -- 'user' | 'model'
          ts     INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS label_log_path ON label_log(path, id);
    """)
    return con


def cache_db(root: Path) -> sqlite3.Connection:
    con = _connect(vibecheck_dir(root) / "cache.db")
    con.executescript("""
        -- keyed by content AND by everything that could change the numbers:
        -- backend, its version, and the preprocessing digest (design.md 3)
        CREATE TABLE IF NOT EXISTS embeddings (
          hash    TEXT NOT NULL,
          backend TEXT NOT NULL,
          version TEXT NOT NULL,
          preproc TEXT NOT NULL,
          dim     INTEGER NOT NULL,
          vec     BLOB NOT NULL,
          ts      INTEGER NOT NULL,
          PRIMARY KEY (hash, backend, version, preproc)
        );
    """)
    return con


def partial_hash(path: Path, size: int) -> str:
    """Cheap content identity: size + head + tail.

    Full hashing would read 199 GB; this reads 512 KB per file and is ample for
    telling files apart and surviving renames.
    """
    h = hashlib.sha256(str(size).encode())
    with open(path, "rb") as f:
        h.update(f.read(HEAD_TAIL_BYTES))
        if size > 2 * HEAD_TAIL_BYTES:
            f.seek(-HEAD_TAIL_BYTES, os.SEEK_END)
            h.update(f.read(HEAD_TAIL_BYTES))
    return h.hexdigest()[:32]


def current_labels(con: sqlite3.Connection, source: str | None = None) -> dict[str, str]:
    """Newest label per path, skipping removals."""
    q = """
        SELECT l.path, l.label FROM label_log l
        JOIN (SELECT path, MAX(id) AS id FROM label_log GROUP BY path) m
          ON l.id = m.id
        WHERE l.label IS NOT NULL
    """
    if source:
        q += " AND l.source = ?"
        rows = con.execute(q, (source,)).fetchall()
    else:
        rows = con.execute(q).fetchall()
    return dict(rows)


def log_label(con: sqlite3.Connection, path: str, hash_: str, label: str | None,
              source: str) -> None:
    con.execute(
        "INSERT INTO label_log (path, hash, label, source, ts) VALUES (?,?,?,?,?)",
        (path, hash_, label, source, int(time.time())),
    )

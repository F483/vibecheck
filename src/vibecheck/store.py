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
    # at rest instead of -wal/-shm litter in the collection (README design §7)
    con.execute("PRAGMA journal_mode=DELETE")
    con.execute("PRAGMA synchronous=FULL")
    # writers are brief but the chunked embedding loop means several processes
    # touch these files; wait for the lock instead of failing the run
    con.execute("PRAGMA busy_timeout=60000")
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


def _id3v2_length(f) -> int:
    """Bytes occupied by a leading ID3v2 tag, or 0 if there is none."""
    head = f.read(10)
    if len(head) < 10 or head[:3] != b"ID3":
        return 0
    flags = head[5]
    size = 0
    for b in head[6:10]:  # synchsafe: 7 bits per byte
        size = (size << 7) | (b & 0x7F)
    return 10 + size + (10 if flags & 0x10 else 0)  # optional footer


def _has_id3v1(f, size: int) -> bool:
    if size < 128:
        return False
    f.seek(-128, os.SEEK_END)
    return f.read(3) == b"TAG"


def partial_hash(path: Path, size: int) -> str:
    """Content identity of the *audio*, deliberately ignoring tags.

    Tags live at the start (ID3v2) and end (ID3v1) of an mp3, and this app
    writes tags as its normal operation. Hashing them would mean every label
    written or corrected changes the hash, orphaning the cached embedding and
    forcing a re-embed of audio that did not change -- 1.6s of GPU per
    correction with a transformer backend. So the tag regions are skipped and
    the hash covers audio only: it survives tag edits by construction.

    It also fixes a real collision: two copies of one track whose genre frames
    sat just past a fixed 256 KB head window hashed identically while carrying
    different labels. Skipping tags removes that whole class of accident.

    Full hashing would read 199 GB; this reads 512 KB per file, which is ample
    to tell tracks apart and survives renames and moves.
    """
    with open(path, "rb") as f:
        start = _id3v2_length(f)
        end = size - (128 if _has_id3v1(f, size) else 0)
        span = max(end - start, 0)

        h = hashlib.sha256(str(span).encode())
        f.seek(start)
        h.update(f.read(min(HEAD_TAIL_BYTES, span)))
        if span > 2 * HEAD_TAIL_BYTES:
            f.seek(end - HEAD_TAIL_BYTES)
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

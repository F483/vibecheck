"""Scanning: find tracks, identify them, pick up label changes.

Corrections are detected, not declared: a tag that differs from the last value
recorded is a user edit (design.md 6). A tag that still matches what the app
itself wrote is not — otherwise the model retrains on its own guesses.
"""

from __future__ import annotations

import json
import subprocess
import time
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

from . import store


@dataclass
class ScanStats:
    files: int = 0
    new: int = 0
    changed: int = 0
    labels_added: int = 0
    labels_changed: int = 0
    unreadable: int = 0


def read_genre(path: Path) -> str | None:
    r = subprocess.run(
        ["ffprobe", "-v", "quiet", "-show_entries", "format_tags",
         "-of", "json", str(path)],
        capture_output=True, timeout=60,
    )
    if r.returncode != 0:
        return None
    tags = json.loads(r.stdout or b"{}").get("format", {}).get("tags", {})
    for k, v in tags.items():
        if k.lower() == "genre":
            v = (v or "").strip()
            return v or None
    return None


def scan(root: Path, workers: int = 12) -> ScanStats:
    root = root.resolve()
    con = store.labels_db(root)
    st = ScanStats()

    known = {
        p: (h, sz, mt) for p, h, sz, mt in
        con.execute("SELECT path, hash, size, mtime FROM tracks")
    }
    last = store.current_labels(con)

    files = sorted(p for p in root.rglob("*") if p.suffix.lower() == ".mp3")
    st.files = len(files)

    def probe(p: Path):
        rel = str(p.relative_to(root))
        stat = p.stat()
        prev = known.get(rel)
        unchanged = prev and prev[1] == stat.st_size and prev[2] == stat.st_mtime
        # only hash and re-read tags where the file actually moved
        if unchanged:
            return rel, prev[0], stat, None, True
        try:
            h = store.partial_hash(p, stat.st_size)
        except OSError:
            return rel, None, stat, None, False
        return rel, h, stat, read_genre(p), False

    with ThreadPoolExecutor(workers) as ex:
        for rel, h, stat, genre, unchanged in ex.map(probe, files):
            if h is None:
                st.unreadable += 1
                continue
            if rel not in known:
                st.new += 1
            elif not unchanged:
                st.changed += 1
            con.execute(
                "INSERT INTO tracks (path, hash, size, mtime, seen_at) VALUES (?,?,?,?,?) "
                "ON CONFLICT(path) DO UPDATE SET hash=?, size=?, mtime=?, seen_at=?",
                (rel, h, stat.st_size, stat.st_mtime, int(time.time()),
                 h, stat.st_size, stat.st_mtime, int(time.time())),
            )
            if unchanged:
                continue
            prev_label = last.get(rel)
            if genre != prev_label:
                store.log_label(con, rel, h, genre, "user")
                if prev_label is None:
                    st.labels_added += 1
                else:
                    st.labels_changed += 1

    con.commit()
    con.close()
    return st

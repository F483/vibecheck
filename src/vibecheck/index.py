"""Scanning: find tracks, identify them, notice what moved.

Reading labels out of genre tags is opt-in (`--adopt-tags`). The app writes
genre tags itself when `debug.write_genre_tags` is on, and writes the colour
without the rating, so a tag that differs from the record is as likely to be
this app's own output as a correction -- adopting it would quietly promote a
model guess to training data. Corrections come through `sync`, which knows
which round they answer.

A file is identified by a hash of its audio, so a rename is a rename and not a
new track that lost its labels. Files that disappear are marked, never deleted:
the collection's size on any past date is `first_seen` and `missing_at`, and a
row removed today is a chart that cannot be drawn next month.
"""

from __future__ import annotations

import time
from collections import defaultdict
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

from . import store, tags


@dataclass
class ScanStats:
    files: int = 0
    new: int = 0
    moved: int = 0
    changed: int = 0
    missing: int = 0
    returned: int = 0
    labels_added: int = 0
    tags_differ: int = 0
    unreadable: int = 0


def scan(root: Path, workers: int = 12,
         adopt_tags: bool = False) -> ScanStats:
    root = root.resolve()
    con = store.labels_db(root)
    st = ScanStats()
    now = int(time.time())

    known = {
        p: (i, h, sz, mt, miss) for i, p, h, sz, mt, miss in
        con.execute("SELECT id, path, hash, size, mtime, missing_at FROM tracks")
    }
    answered = store.answered(con)
    current = {p: l.colour for p, l in store.current(con).items()}

    files = sorted(p for p in root.rglob("*") if p.suffix.lower() == ".mp3")
    st.files = len(files)
    present = {str(p.relative_to(root)) for p in files}

    # A path that vanished and a hash that reappears elsewhere is one file
    # moving. Only unambiguous cases count: with two copies of the same audio
    # there is no way to say which one moved, and guessing would attach one
    # track's listening history to another.
    orphan: dict[str, list[str]] = defaultdict(list)
    for rel, (_, h, *_rest) in known.items():
        if rel not in present:
            orphan[h].append(rel)
    claimed: set[str] = set()

    def probe(p: Path):
        rel = str(p.relative_to(root))
        stat = p.stat()
        prev = known.get(rel)
        unchanged = prev and prev[2] == stat.st_size and prev[3] == stat.st_mtime
        # only hash and re-read tags where the file actually changed
        if unchanged:
            return rel, prev[1], stat, None, True
        try:
            h = store.partial_hash(p, stat.st_size)
        except OSError:
            return rel, None, stat, None, False
        return rel, h, stat, tags.read(p), False

    with ThreadPoolExecutor(workers) as ex:
        for rel, h, stat, genre, unchanged in ex.map(probe, files):
            if h is None:
                st.unreadable += 1
                continue
            prev = known.get(rel)

            if prev is None:
                moved_from = [o for o in orphan.get(h, []) if o not in claimed]
                if len(moved_from) == 1:
                    was = moved_from[0]
                    claimed.add(was)
                    con.execute(
                        "UPDATE tracks SET path=?, size=?, mtime=?, last_seen=?,"
                        " missing_at=NULL WHERE id=?",
                        (rel, stat.st_size, stat.st_mtime, now, known[was][0]))
                    st.moved += 1
                    continue          # same file, same labels, nothing to adopt
                con.execute(
                    "INSERT INTO tracks (path, hash, size, mtime, first_seen, "
                    "last_seen) VALUES (?,?,?,?,?,?)",
                    (rel, h, stat.st_size, stat.st_mtime, now, now))
                st.new += 1
            else:
                if prev[4] is not None:
                    st.returned += 1
                if not unchanged:
                    st.changed += 1
                con.execute(
                    "UPDATE tracks SET hash=?, size=?, mtime=?, last_seen=?, "
                    "missing_at=NULL WHERE id=?",
                    (h, stat.st_size, stat.st_mtime, now, prev[0]))
                if unchanged:
                    continue

            if genre is None:
                continue
            if adopt_tags and rel not in answered:
                # bootstrap: a tag on a track nothing has an opinion about is
                # the user's own work, from whatever they labelled with before
                tid = con.execute("SELECT id FROM tracks WHERE path=?",
                                  (rel,)).fetchone()[0]
                store.log_label(con, tid, store.Label(genre, None), "user")
                st.labels_added += 1
            elif adopt_tags or genre != current.get(rel):
                # Deliberately not adopted. The app writes genre tags itself
                # (debug.write_genre_tags) and writes the colour without the
                # rating, so a differing tag is as likely to be this app's own
                # output as a correction -- and adopting it would demote a
                # model guess into training data. Corrections come through
                # `sync`, which knows which round they answer.
                st.tags_differ += 1

    gone = [known[rel] for rel in known
            if rel not in present and rel not in claimed and known[rel][4] is None]
    for row in gone:
        con.execute("UPDATE tracks SET missing_at=? WHERE id=?", (now, row[0]))
    st.missing = len(gone)

    con.commit()
    con.close()
    return st

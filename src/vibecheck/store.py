"""State lives in the collection root, under .vibecheck/ (design.md 7).

Two files on purpose: labels.db is hours of listening and irreplaceable;
cache.db is CPU time and can be deleted whenever a backend changes.

labels.db is a record of what happened, not a snapshot of where things stand.
Everything derivable is derived: the current label is the newest row in
`labels`, the collection's size on any past date comes from `first_seen` and
`missing_at`, and how well a round did comes from joining what the model said
to what the user said afterwards. Nothing is overwritten, because a number
overwritten today is a chart that cannot be drawn next month.
"""

from __future__ import annotations

import hashlib
import os
import re
import sqlite3
import time
import unicodedata
from pathlib import Path
from typing import NamedTuple

from . import palette

HEAD_TAIL_BYTES = 256 * 1024

SCHEMA_VERSION = 3


def vibecheck_dir(root: Path) -> Path:
    d = root / ".vibecheck"
    d.mkdir(exist_ok=True)
    return d


def _connect(path: Path) -> sqlite3.Connection:
    con = sqlite3.connect(path)
    # busy_timeout FIRST: setting journal_mode itself needs a lock, so a
    # timeout set afterwards does not protect the statement most likely to
    # block against a concurrent writer.
    con.execute("PRAGMA busy_timeout=60000")
    con.execute("PRAGMA foreign_keys=ON")
    # rollback journal, not WAL: one process, batch writes, and a single file
    # at rest instead of -wal/-shm litter in the collection (README design §7).
    # DELETE is already SQLite's default, so this is defensive -- and it cannot
    # be set while another connection holds the database, which busy_timeout
    # does not cover. A reader must not fail because a writer is mid-commit.
    try:
        con.execute("PRAGMA journal_mode=DELETE")
        con.execute("PRAGMA synchronous=FULL")
    except sqlite3.OperationalError:
        pass
    return con


MIGRATIONS = "migrations"


def _steps() -> list[tuple[int, str, str]]:
    """(version, name, sql) for every migration, in order.

    Read through importlib.resources rather than a path relative to this file:
    an installed app has no source tree beside it.
    """
    from importlib import resources

    out = []
    for f in resources.files(__package__).joinpath(MIGRATIONS).iterdir():
        m = re.fullmatch(r"(\d{4})_(.+)\.sql", f.name)
        if m:
            out.append((int(m.group(1)), m.group(2), f.read_text()))
    return sorted(out)


SCHEMA_VERSION = max(v for v, _, _ in _steps())
BASELINE = min(v for v, _, _ in _steps())

# v1 predates the version stamp; this is the table only it had.
UNSTAMPED = {"label_log": 1}


def migrate(con: sqlite3.Connection, path: Path | None = None,
            dry_run: bool = False) -> list[str]:
    """Bring a database up to SCHEMA_VERSION. Returns what it did.

    `PRAGMA user_version` is the only record of where a database stands: a
    SQLite built-in that rusqlite, Dart and Node all read in one line, so the
    runner can be replaced later without the history having to be reconciled.

    Each step runs in its own transaction, and SQLite rolls back DDL, so a step
    that fails leaves the database exactly where it was.
    """
    have = con.execute("PRAGMA user_version").fetchone()[0]
    for table, version in UNSTAMPED.items():
        if not have and con.execute(
                "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?",
                (table,)).fetchone():
            have = version
    if have and have < BASELINE:
        # The baseline builds the schema from nothing, so it is only valid for
        # an empty database. Running it over an older one would find every
        # table already present and do nothing at all, then stamp the version
        # as though it had worked -- which is how a database ends up claiming
        # v3 with v2 columns in it.
        raise SystemExit(
            f"{path or 'this database'} is schema v{have}, which this build "
            f"cannot upgrade: the oldest it knows how to build from is "
            f"v{BASELINE}.\n  Move it aside and rescan. Nothing has been "
            f"changed.")
    if have > SCHEMA_VERSION:
        raise SystemExit(
            f"{path or 'this database'} is schema v{have}, newer than this "
            f"build's v{SCHEMA_VERSION}.\n  Update vibecheck. Nothing has "
            f"been changed.")

    todo = [(v, n, sql) for v, n, sql in _steps() if v > have]
    if not todo or dry_run:
        return [f"{v:04d}_{n}" for v, n, _ in todo]

    if have and path is not None:
        # Irreplaceable: rounds, predictions and fits exist nowhere else. The
        # copy costs milliseconds and is the difference between a failed
        # migration being an inconvenience and being a loss.
        backup = path.with_suffix(f".db.pre-v{have}")
        if not backup.exists():
            backup.write_bytes(path.read_bytes())

    done = []
    for version, name, sql in todo:
        try:
            con.execute("BEGIN")
            con.executescript(sql)
            con.execute(f"PRAGMA user_version={version}")
            con.commit()
        except Exception:
            con.rollback()
            raise
        done.append(f"{version:04d}_{name}")
    return done


def _check_palette(con: sqlite3.Connection) -> None:
    """The palette lives in two places; this is what keeps them honest.

    palette.py is the source, the `colours` table is the copy a query can
    reach. They can only diverge by editing one against an existing database,
    which is exactly the case worth catching loudly.
    """
    rows = con.execute("SELECT id, name, hue, tone, rgb FROM colours "
                       "ORDER BY id").fetchall()
    if rows != [tuple(c) for c in palette.COLOURS]:
        raise SystemExit(
            "the colours table does not match palette.py.\n  This database "
            "was built by a different build; migrate it rather than editing "
            "the palette.")


def labels_db(root: Path) -> sqlite3.Connection:
    path = vibecheck_dir(root) / "labels.db"
    con = _connect(path)
    if con.execute("PRAGMA user_version").fetchone()[0] != SCHEMA_VERSION:
        # opening must not write once it is current, or a second connection in
        # the same process deadlocks against the first
        migrate(con, path)
    _check_palette(con)
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


def norm(rel: str) -> str:
    """Canonical form for comparing paths that came from elsewhere.

    macOS stores filenames decomposed (NFD) while other software -- Rekordbox's
    XML export among them -- writes them composed (NFC). The same file then has
    two different string forms that never compare equal, and 665 tracks with
    accented names silently fail to match.
    """
    return unicodedata.normalize("NFC", rel)


def path_index(con: sqlite3.Connection) -> dict[str, str]:
    """Map any normalisation of a path back to the form stored in the index."""
    return {norm(p): p for (p,) in con.execute("SELECT path FROM tracks")}


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


# --- reading ---------------------------------------------------------------

class Label(NamedTuple):
    """One complete statement about a track. None means "said nothing"."""
    colour: str | None            # colour name, e.g. 'Pink'
    stars: int | None             # 0..5; 0 is rekordbox's "unrated"

    @property
    def hue(self) -> str | None:
        return palette.BY_NAME[self.colour].hue if self.colour else None

    @property
    def tone(self) -> str | None:
        return palette.BY_NAME[self.colour].tone if self.colour else None

    def __bool__(self) -> bool:
        """False when nothing was said: no colour, and rekordbox's "unrated".

        A blank track that stays blank is not an event. A track that *had* a
        colour and no longer does is -- that is a deliberate answer, and the
        caller decides which case it is looking at.
        """
        return self.colour is not None or bool(self.stars)


def track_ids(con: sqlite3.Connection) -> dict[str, int]:
    return dict(con.execute("SELECT path, id FROM tracks"))


def hash_by_path(con: sqlite3.Connection) -> dict[str, str]:
    return dict(con.execute("SELECT path, hash FROM tracks"))


def current(con: sqlite3.Connection,
            source: str | None = None) -> dict[str, Label]:
    """Where each track stands, for tracks anything has been said about."""
    q = "SELECT path, colour, stars FROM current WHERE label_id IS NOT NULL"
    args: tuple = ()
    if source:
        q, args = q + " AND source = ?", (source,)
    return {p: Label(c, s) for p, c, s in con.execute(q, args)
            if c is not None or s is not None}


def answered(con: sqlite3.Connection) -> set[str]:
    """Tracks with any statement at all, including a deliberate blank."""
    return {p for (p,) in con.execute(
        "SELECT path FROM current WHERE label_id IS NOT NULL")}


def round_of(con: sqlite3.Connection) -> dict[str, int]:
    """Which round last predicted each track.

    A correction belongs to the round that provoked it, so sync and scan carry
    this onto the user's row -- otherwise the two halves of a round (what was
    said, what was fixed) cannot be joined afterwards.
    """
    return dict(con.execute("""
        SELECT t.path, l.round_id FROM labels l
        JOIN tracks t ON t.id = l.track_id
        JOIN (SELECT track_id, MAX(id) AS id FROM labels
              WHERE source = 'model' GROUP BY track_id) m ON l.id = m.id
        WHERE l.round_id IS NOT NULL
    """))


# --- writing ---------------------------------------------------------------

def log_label(con: sqlite3.Connection, track_id: int, label: Label,
              source: str, round_id: int | None = None) -> None:
    cid = palette.BY_NAME[label.colour].id if label.colour else None
    con.execute(
        "INSERT INTO labels (track_id, colour_id, stars, source, round_id, ts) "
        "VALUES (?,?,?,?,?,?)",
        (track_id, cid, label.stars, source, round_id, int(time.time())),
    )


def open_round(con: sqlite3.Connection, name: str, size: int, backend: str,
               encoder: str, n_labels: int) -> int:
    return con.execute(
        "INSERT INTO rounds (name, started, size, backend, encoder, n_labels) "
        "VALUES (?,?,?,?,?,?)",
        (name, int(time.time()), size, backend, encoder, n_labels),
    ).lastrowid


def close_round(con: sqlite3.Connection, round_id: int) -> None:
    """First sync that reads a round back closes it; later syncs do not move it."""
    con.execute("UPDATE rounds SET closed = ? WHERE id = ? AND closed IS NULL",
                (int(time.time()), round_id))


def log_predictions(con: sqlite3.Connection, round_id: int,
                    rows: list[tuple[int, object]]) -> None:
    """rows: (track_id, predict.Prediction)."""
    out = []
    for tid, p in rows:
        hue, hue_p = p.top["hue"]
        tone, tone_p = p.top["tone"]
        stars, stars_p = p.top["stars"]
        col = palette.colour_from_grid(p.values.get("hue"), p.values.get("tone"))
        out.append((round_id, tid,
                    hue, hue_p, int("hue" in p.values),
                    tone, tone_p, int("tone" in p.values),
                    int(stars), stars_p, int("stars" in p.values),
                    col.id if col else None))
    con.executemany(
        "INSERT OR REPLACE INTO predictions (round_id, track_id, hue, hue_p, "
        "hue_said, tone, tone_p, tone_said, stars, stars_p, stars_said, "
        "colour_id) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)", out)


def log_fit(con: sqlite3.Connection, round_id: int | None, backend: str,
            encoder: str, slice_: str, n_train: int, n_labels: int,
            scores: list) -> int:
    """`scores` are evaluate.AxisScore; returns the fit id."""
    fit = con.execute(
        "INSERT INTO fits (round_id, ts, backend, encoder, slice, n_train, "
        "n_holdout, n_labels) VALUES (?,?,?,?,?,?,?,?)",
        (round_id, int(time.time()), backend, encoder, slice_, n_train,
         max((s.n for s in scores), default=0), n_labels),
    ).lastrowid
    con.executemany(
        "INSERT INTO fit_axes (fit_id, axis, n_values, cost, misleading, n, "
        "spoke, correct, cost_left) VALUES (?,?,?,?,?,?,?,?,?)",
        [(fit, s.axis, s.n_values, s.cost, s.misleading, s.n, s.spoke,
          s.correct, s.cost_left) for s in scores],
    )
    return fit

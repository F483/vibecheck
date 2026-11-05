"""A migration nobody has run against real old data is a guess.

These tests exist so that every future schema step is replayed, from every
version that has ever existed in the wild, before it reaches anyone's
collection. Adding a migration means adding a fixture: a database built by the
*previous* build, with rows in it, committed here.
"""

from __future__ import annotations

import sqlite3
from pathlib import Path

import pytest

from vibecheck import palette, store

FIXTURES = Path(__file__).parent / "fixtures"


def fresh(tmp_path: Path) -> sqlite3.Connection:
    return store.labels_db(tmp_path)


def test_fresh_database_lands_on_the_current_version(tmp_path):
    con = fresh(tmp_path)
    assert con.execute("PRAGMA user_version").fetchone()[0] == store.SCHEMA_VERSION


def test_migrations_are_numbered_by_the_version_they_produce(tmp_path):
    steps = store._steps()
    assert steps, "there must be at least a baseline"
    versions = [v for v, _, _ in steps]
    assert versions == sorted(set(versions)), "duplicate or unordered versions"
    assert versions[-1] == store.SCHEMA_VERSION


def test_opening_twice_changes_nothing(tmp_path):
    fresh(tmp_path).close()
    before = (tmp_path / ".vibecheck" / "labels.db").read_bytes()
    fresh(tmp_path).close()
    assert (tmp_path / ".vibecheck" / "labels.db").read_bytes() == before


def test_the_colours_table_matches_the_palette(tmp_path):
    con = fresh(tmp_path)
    rows = con.execute("SELECT id, name, hue, tone, rgb FROM colours "
                       "ORDER BY id").fetchall()
    assert rows == [tuple(c) for c in palette.COLOURS]


def test_a_divergent_palette_is_refused(tmp_path):
    con = fresh(tmp_path)
    con.execute("UPDATE colours SET name='Crimson' WHERE id=1")
    con.commit()
    with pytest.raises(SystemExit, match="does not match palette"):
        store.labels_db(tmp_path)


def test_a_newer_database_is_refused_not_downgraded(tmp_path):
    con = fresh(tmp_path)
    con.execute(f"PRAGMA user_version={store.SCHEMA_VERSION + 1}")
    con.commit()
    with pytest.raises(SystemExit, match="newer than this build"):
        store.labels_db(tmp_path)


def test_an_unstamped_schema_is_recognised_and_refused(tmp_path):
    """v1 predates PRAGMA user_version; it is identified by its own tables."""
    d = tmp_path / ".vibecheck"
    d.mkdir()
    con = sqlite3.connect(d / "labels.db")
    con.execute("CREATE TABLE label_log (id INTEGER PRIMARY KEY, label TEXT)")
    con.commit()
    con.close()
    with pytest.raises(SystemExit, match="schema v1"):
        store.labels_db(tmp_path)


def test_a_pre_baseline_schema_is_refused_not_silently_stamped(tmp_path):
    """The bug this test exists for.

    The baseline builds the schema from nothing. Run over an older database it
    would find every table already there, do nothing, and stamp the version as
    though it had worked -- leaving a database that claims the current schema
    while holding the previous one's columns. It must refuse instead.
    """
    d = tmp_path / ".vibecheck"
    d.mkdir()
    con = sqlite3.connect(d / "labels.db")
    con.execute("CREATE TABLE labels (id INTEGER PRIMARY KEY, label TEXT)")
    con.execute(f"PRAGMA user_version={store.BASELINE - 1}")
    con.commit()
    con.close()

    with pytest.raises(SystemExit, match="cannot upgrade"):
        store.labels_db(tmp_path)

    after = sqlite3.connect(d / "labels.db")
    assert after.execute("PRAGMA user_version").fetchone()[0] == store.BASELINE - 1
    assert [c[1] for c in after.execute("PRAGMA table_info(labels)")] == \
        ["id", "label"], "the refused database was modified"


def test_the_baseline_refuses_to_run_over_anything(tmp_path):
    """No IF NOT EXISTS in the baseline: it must fail rather than no-op."""
    d = tmp_path / ".vibecheck"
    d.mkdir()
    con = store._connect(d / "labels.db")
    con.execute("CREATE TABLE tracks (id INTEGER PRIMARY KEY)")
    con.commit()
    with pytest.raises(sqlite3.OperationalError, match="already exists"):
        store.migrate(con, d / "labels.db")


# Empty until a schema older than the baseline has actually shipped to someone.
# Adding a migration means adding a fixture here: a small database built by the
# *previous* build, with rows in it, and no personal paths in it.
@pytest.mark.parametrize("fixture", sorted(FIXTURES.glob("v*.db")))
def test_every_shipped_version_upgrades_with_its_rows_intact(fixture, tmp_path):
    """Replay a real database from an older build, and count what survives."""
    d = tmp_path / ".vibecheck"
    d.mkdir()
    (d / "labels.db").write_bytes(fixture.read_bytes())

    before = sqlite3.connect(d / "labels.db")
    counts = {t: before.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0]
              for t in ("tracks", "labels")}
    before.close()

    con = store.labels_db(tmp_path)
    assert con.execute("PRAGMA user_version").fetchone()[0] == store.SCHEMA_VERSION
    for table, n in counts.items():
        assert con.execute(f"SELECT COUNT(*) FROM {table}").fetchone()[0] >= n, \
            f"{table} lost rows migrating {fixture.name}"
    # and the backup the runner is required to leave behind
    assert list(d.glob("labels.db.pre-v*")), "no backup was written"

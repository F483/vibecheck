-- Baseline: the schema as of 2026-09-15.
--
-- The number is the version the database is AT once this has been applied, not
-- a sequence position. There is no 0001 or 0002: those schemas never left this
-- machine, and both were retired by hand rather than migrated. A database at
-- v1 or v2 is refused, not upgraded -- see store.migrate().
--
-- No IF NOT EXISTS: a migration that silently does nothing is worse than
-- one that fails. This must only ever run against an empty database.
--
-- Plain SQL on purpose. The runner that applies this is thirty lines of Python
-- today and could be rusqlite, Dart or Node tomorrow; `PRAGMA user_version` is
-- a SQLite built-in that all of them read. Nothing here presupposes which.

-- The palette, seeded once. Fixed by rekordbox, not by taste (palette.py).
CREATE TABLE colours (
  id   INTEGER PRIMARY KEY,           -- 1..8, rekordbox's own order
  name TEXT NOT NULL UNIQUE,
  hue  TEXT NOT NULL,                 -- one of four
  tone TEXT NOT NULL,                 -- one of two; hue x tone is the grid
  rgb  TEXT NOT NULL UNIQUE           -- as rekordbox writes it
);

-- Files. `id` is the identity everything else refers to, so a rename or a tag
-- write does not orphan a label: scan matches a new path to an existing row by
-- audio hash and updates the path in place.
CREATE TABLE tracks (
  id         INTEGER PRIMARY KEY,
  path       TEXT NOT NULL UNIQUE,    -- relative to collection root, NFC
  hash       TEXT NOT NULL,           -- audio only; survives tag writes
  size       INTEGER NOT NULL,
  mtime      REAL NOT NULL,
  first_seen INTEGER NOT NULL,        -- set once, never updated
  last_seen  INTEGER NOT NULL,
  missing_at INTEGER                  -- when a scan stopped finding it
);
CREATE INDEX tracks_hash ON tracks(hash);
CREATE INDEX tracks_first_seen ON tracks(first_seen);

-- A round: predict a batch, correct it, feed it back. Named after its
-- playlist so the file on disk and the row here are obviously the same thing.
CREATE TABLE rounds (
  id       INTEGER PRIMARY KEY,
  name     TEXT NOT NULL UNIQUE,
  started  INTEGER NOT NULL,
  closed   INTEGER,                   -- first sync that read it back
  size     INTEGER NOT NULL,          -- tracks in the batch
  backend  TEXT NOT NULL,
  encoder  TEXT NOT NULL,             -- backend/version/preproc: why a curve moved
  n_labels INTEGER NOT NULL           -- labels available when it was made
);

-- Append-only. Never UPDATE, never DELETE; the newest row per track wins.
--
-- One row is one complete statement about a track, which is how the data
-- arrives: rekordbox hands over a colour and a rating together. NULL means the
-- statement said nothing on that axis -- a round that was sure of the colour
-- and not the rating. `stars = 0` is different: that is rekordbox's "unrated",
-- an answer rather than a silence.
--
-- Having no row at all is different again: never asked, or never answered. An
-- unlabelled collection and a deliberately-unlabelled one must not look
-- identical in the statistics.
CREATE TABLE labels (
  id        INTEGER PRIMARY KEY,
  track_id  INTEGER NOT NULL REFERENCES tracks(id),
  colour_id INTEGER REFERENCES colours(id),
  stars     INTEGER,
  source    TEXT NOT NULL,            -- 'user' | 'model'
  round_id  INTEGER REFERENCES rounds(id),   -- NULL: outside any round
  ts        INTEGER NOT NULL,
  CHECK (stars IS NULL OR stars BETWEEN 0 AND 5),
  CHECK (source IN ('user', 'model'))
);
CREATE INDEX labels_track ON labels(track_id, id);
CREATE INDEX labels_round ON labels(round_id);

-- What the model thought on each axis, before the decision rule was applied.
-- The best guess and its probability are recorded even where the axis stayed
-- silent: without them, "what would a different misleading_cost have done?"
-- can only be answered by re-running a model that has since learned from the
-- answers. `colour_id` is set exactly when hue and tone both spoke.
CREATE TABLE predictions (
  round_id   INTEGER NOT NULL REFERENCES rounds(id),
  track_id   INTEGER NOT NULL REFERENCES tracks(id),
  hue        TEXT    NOT NULL,
  hue_p      REAL    NOT NULL,
  hue_said   INTEGER NOT NULL,
  tone       TEXT    NOT NULL,
  tone_p     REAL    NOT NULL,
  tone_said  INTEGER NOT NULL,
  stars      INTEGER NOT NULL,
  stars_p    REAL    NOT NULL,
  stars_said INTEGER NOT NULL,
  colour_id  INTEGER REFERENCES colours(id),
  PRIMARY KEY (round_id, track_id)
);

-- One row per fit, measured on the hash-derived holdout by a model that never
-- saw it. Separate from `rounds` because a fit can also be a plain measurement
-- -- the once-only test-slice reading at the end, or a re-measurement of
-- history.
CREATE TABLE fits (
  id        INTEGER PRIMARY KEY,
  round_id  INTEGER REFERENCES rounds(id),
  ts        INTEGER NOT NULL,
  backend   TEXT NOT NULL,
  encoder   TEXT NOT NULL,
  slice     TEXT NOT NULL,            -- 'val' | 'test'
  n_train   INTEGER NOT NULL,         -- rows actually fitted
  n_holdout INTEGER NOT NULL,
  n_labels  INTEGER NOT NULL
);

CREATE TABLE fit_axes (
  fit_id     INTEGER NOT NULL REFERENCES fits(id),
  axis       TEXT NOT NULL,           -- 'hue' | 'tone' | 'stars'
  n_values   INTEGER NOT NULL,
  cost       REAL NOT NULL,           -- decisions saved by knowing this axis
  misleading REAL NOT NULL,           -- the dial in force
  n          INTEGER NOT NULL,        -- holdout tracks whose truth is known
  spoke      INTEGER NOT NULL,
  correct    INTEGER NOT NULL,
  cost_left  REAL NOT NULL,           -- decisions still to make, per track
  PRIMARY KEY (fit_id, axis)
);

-- Where every track stands now, with hue and tone spelled out so a statistic
-- never has to know how a colour decomposes.
CREATE VIEW current AS
SELECT t.id AS track_id, t.path, t.hash, t.missing_at,
       l.id AS label_id, l.colour_id, c.name AS colour, c.hue, c.tone,
       l.stars, l.source, l.round_id, l.ts
FROM tracks t
LEFT JOIN labels l
  ON l.id = (SELECT MAX(id) FROM labels WHERE track_id = t.id)
LEFT JOIN colours c ON c.id = l.colour_id;

-- The palette, seeded. Fixed by rekordbox, not by taste (palette.py),
-- and verified against it on every open.
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (1, 'Red', 'Warm', 'Dark', '0xFF0000');
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (2, 'Orange', 'Warm', 'Light', '0xFFA500');
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (3, 'Yellow', 'Acidic', 'Light', '0xFFFF00');
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (4, 'Green', 'Acidic', 'Dark', '0x00FF00');
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (5, 'Aqua', 'Cool', 'Light', '0x25FDE9');
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (6, 'Blue', 'Cool', 'Dark', '0x0000FF');
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (7, 'Purple', 'Vibrant', 'Dark', '0x660099');
INSERT INTO colours (id, name, hue, tone, rgb) VALUES (8, 'Pink', 'Vibrant', 'Light', '0xFF007F');

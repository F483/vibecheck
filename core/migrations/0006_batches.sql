-- What came in from rekordbox and what went out to it, for the history page:
-- one row per sync ('in') and per export ('out'). Kept for viewing only;
-- nothing reads it to decide anything.
CREATE TABLE batches (
  id      INTEGER PRIMARY KEY,
  dir     TEXT NOT NULL CHECK (dir IN ('in', 'out')),
  ts      INTEGER NOT NULL,            -- when it ran, unix seconds
  name    TEXT NOT NULL,               -- out: the playlist; in: the xml's file name
  xml_ts  INTEGER,                     -- in: when rekordbox wrote the xml
  counts  TEXT NOT NULL                -- JSON, the summary as the app showed it
);

-- The tracks of a batch. in: what = 'added' | 'corrected' | 'played', with the
-- label before and after. out: what = 'exported', with what was written, and
-- fate = 'played' | 'corrected' | 'untouched' once a sync has seen it.
CREATE TABLE batch_tracks (
  batch_id      INTEGER NOT NULL REFERENCES batches(id),
  track_id      INTEGER NOT NULL REFERENCES tracks(id),
  what          TEXT NOT NULL,
  colour_before INTEGER,
  stars_before  INTEGER,
  colour        INTEGER,
  stars         INTEGER,
  key           TEXT,
  energy        INTEGER,
  cues          INTEGER,
  fate          TEXT,
  PRIMARY KEY (batch_id, track_id)
);

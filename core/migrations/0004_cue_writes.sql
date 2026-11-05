-- Cues this app wrote into the rekordbox xml.
--
-- Cues carry no author: a cue set by hand and one written here look the same
-- in the export. This is the record that tells them apart. A track whose hot
-- cues still sit exactly where they were written has not been reviewed, and
-- learning from it would be learning from the model's own guesses; once one
-- is moved, the track is a correction like any other.
--
-- Append-only, like `labels`: the newest row per track is what was last
-- written.
CREATE TABLE cue_writes (
  id          INTEGER PRIMARY KEY,
  track_id    INTEGER NOT NULL REFERENCES tracks(id),
  round_id    INTEGER REFERENCES rounds(id),   -- NULL: written outside a round
  model       TEXT NOT NULL,                   -- feature version / model source
  transitions TEXT NOT NULL,                   -- JSON bar indices
  hot         TEXT NOT NULL,                   -- JSON hot cue starts, seconds
  ts          INTEGER NOT NULL
);
CREATE INDEX cue_writes_track ON cue_writes(track_id, id);

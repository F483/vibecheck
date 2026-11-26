-- Plays across the whole collection (docs/label-weights.md). Before, a sync only compared
-- the play counts of the last export's tracks, so a play of anything exported earlier, or
-- never, was lost.
--
-- The play count rekordbox showed per track when last seen: at the last sync, or at the
-- export that wrote the track (its label changes there). The next sync compares against it.
CREATE TABLE play_counts (
  track_id INTEGER PRIMARY KEY REFERENCES tracks(id),
  plays    INTEGER NOT NULL,
  ts       INTEGER NOT NULL
);

-- A sync saw the count rise: played with `label_id`, the label it had here and in rekordbox
-- then; NULL when rekordbox had another (labelled here since, not exported yet) or none.
-- Append-only.
CREATE TABLE plays (
  id        INTEGER PRIMARY KEY,
  track_id  INTEGER NOT NULL REFERENCES tracks(id),
  label_id  INTEGER REFERENCES labels(id),
  count_was INTEGER NOT NULL,
  count_now INTEGER NOT NULL,
  ts        INTEGER NOT NULL
);
CREATE INDEX plays_track ON plays(track_id);

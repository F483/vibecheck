-- rekordbox's cues per track as the last sync (or the export that wrote them) saw them
-- (docs/cues.md). A sync that finds them different has seen you edit them in rekordbox:
-- `changed` is when, and the track's cues count as confirmed from then on. Without it, an
-- edit of cues you set before vibecheck looked like the old cues, trusted as existing.
CREATE TABLE cues_seen (
  track_id INTEGER PRIMARY KEY REFERENCES tracks(id),
  cues     TEXT NOT NULL,  -- `cues::cue_key`: every cue, rounded to 10 ms, sorted
  ts       INTEGER NOT NULL,
  changed  INTEGER         -- when a sync last saw them change; NULL: never
);

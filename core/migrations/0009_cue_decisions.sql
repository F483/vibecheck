-- Cues you set in the player (docs/cues.md): the hot and memory cues as you left them, and
-- the suggestion on screen when you did, so trust can tell a kept suggestion from a change.
-- Export writes them in place of whatever rekordbox had (a fresh start, decided 2026-11-24).
--
-- Append-only, like `labels`: the newest row per track is your cues for it.
CREATE TABLE cue_decisions (
  id       INTEGER PRIMARY KEY,
  track_id INTEGER NOT NULL REFERENCES tracks(id),
  cues     TEXT NOT NULL,  -- JSON [{"num": 0..7 hot cue A..H, -1 memory, "start": s, "end": s or null}]
  shown    TEXT,           -- JSON, the same, as suggested on screen then; NULL: nothing shown
  card     TEXT,           -- which card asked, as labels.card
  ts       INTEGER NOT NULL
);
CREATE INDEX cue_decisions_track ON cue_decisions(track_id, id);

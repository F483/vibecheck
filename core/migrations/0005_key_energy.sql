-- Key and energy as you corrected them. Otherwise they are worked out from the
-- audio (cache.db `analysis`, which can be recomputed); a correction is yours
-- and cannot.
--
-- The first Rust-only schema: the Python prototype stays at v4 and refuses this.
--
-- Append-only, like `labels`: one row is everything said about the track's key
-- and energy at once, and the newest row per track wins. NULL: not stated.
CREATE TABLE key_energy (
  id       INTEGER PRIMARY KEY,
  track_id INTEGER NOT NULL REFERENCES tracks(id),
  key      TEXT,                     -- alphanumeric, "8A"
  energy   INTEGER,                  -- 1..10
  ts       INTEGER NOT NULL,
  CHECK (energy IS NULL OR energy BETWEEN 1 AND 10)
);
CREATE INDEX key_energy_track ON key_energy(track_id, id);

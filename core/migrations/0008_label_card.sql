-- How a label came to be, beyond its source (docs/fast-track.md):
--   shown        the colour on screen when it was made, palette id; NULL = asked blind
--   shown_stars  the stars on screen then; NULL = none shown
--   card         which card asked: calibration, unsure, conflict, old label, re-ask, fast,
--                blind, relisten, dropped, library; for origin 'xml': existing (there before
--                vibecheck), corrected (changed in rekordbox), confirmed (left as exported)
-- Training weights a label by these (trust), and only blind labels measure accuracy.
ALTER TABLE labels ADD COLUMN shown INTEGER REFERENCES colours(id);
ALTER TABLE labels ADD COLUMN shown_stars INTEGER;
ALTER TABLE labels ADD COLUMN card TEXT;

-- Older rekordbox labels: which were corrections, from the history rows of the sync
-- that read them (logged within the same minute).
UPDATE labels SET card = 'corrected'
WHERE origin = 'xml' AND card IS NULL AND EXISTS (
  SELECT 1 FROM batch_tracks bt JOIN batches b ON b.id = bt.batch_id
  WHERE b.dir = 'in' AND bt.track_id = labels.track_id AND bt.what = 'corrected'
    AND abs(b.ts - labels.ts) <= 120);
UPDATE labels SET card = 'existing'
WHERE origin = 'xml' AND card IS NULL AND EXISTS (
  SELECT 1 FROM batch_tracks bt JOIN batches b ON b.id = bt.batch_id
  WHERE b.dir = 'in' AND bt.track_id = labels.track_id AND bt.what = 'added'
    AND abs(b.ts - labels.ts) <= 120);

DROP VIEW current;
CREATE VIEW current AS
SELECT t.id AS track_id, t.path, t.hash, t.missing_at,
       l.id AS label_id, l.colour_id, c.name AS colour, c.hue, c.tone,
       l.stars, l.source, l.round_id, l.ts, l.origin, l.shown, l.shown_stars, l.card
FROM tracks t
LEFT JOIN labels l
  ON l.id = (SELECT MAX(id) FROM labels WHERE track_id = t.id)
LEFT JOIN colours c ON c.id = l.colour_id;

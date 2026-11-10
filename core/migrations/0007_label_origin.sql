-- Where a label came from: 'xml' when sync read it from rekordbox, NULL when it
-- was made here. A sync leaves alone a track whose label was made here and not
-- exported yet: rekordbox cannot know it, so its value there is no correction.
ALTER TABLE labels ADD COLUMN origin TEXT;

DROP VIEW current;
CREATE VIEW current AS
SELECT t.id AS track_id, t.path, t.hash, t.missing_at,
       l.id AS label_id, l.colour_id, c.name AS colour, c.hue, c.tone,
       l.stars, l.source, l.round_id, l.ts, l.origin
FROM tracks t
LEFT JOIN labels l
  ON l.id = (SELECT MAX(id) FROM labels WHERE track_id = t.id)
LEFT JOIN colours c ON c.id = l.colour_id;

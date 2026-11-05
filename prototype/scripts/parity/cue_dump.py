"""For tracks with cached cue features: Python's transitions from them."""
import sys, json, numpy as np
from pathlib import Path
from vibecheck import cues, store, rekordbox
root = Path.home() / "Music/Collection"
marks = rekordbox.read_marks(Path.home() / "Documents/rekordbox.xml", root)
con = store.cache_db(root)
hb = dict(store.labels_db(root).execute("SELECT path, hash FROM tracks"))
have = {(h, g) for h, g in con.execute("SELECT hash, grid FROM cue_bars WHERE version=?", (cues.version(),))}
rels = sorted(r for r, m in marks.items() if m.tempos and (hb.get(r), cues.grid_digest(m.tempos)) in have)
rels = rels[::max(len(rels) // int(sys.argv[2]), 1)][:int(sys.argv[2])]
F, _ = cues.features_all(root, {r: marks[r] for r in rels})
model = cues.Model.load(root)
out = [{"rel": r, "T": model.transitions(F[r]), "scores": model.scores(F[r])[:5].tolist(), "grid": cues.grid_digest(marks[r].tempos)} for r in rels]
json.dump(out, open(sys.argv[1], "w"))
print(len(out), "tracks")

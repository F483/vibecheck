"""Make 60 entries, write them with the prototype, save entries for Rust."""
import sys, json, random, shutil
from pathlib import Path
from vibecheck import rekordbox, cues, palette
from vibecheck.store import Label
root = Path.home() / "Music/Collection"
src = Path(sys.argv[1]); out_py = Path(sys.argv[2]); ent_json = Path(sys.argv[3])
marks = rekordbox.read_marks(src, root)
random.seed(3)
rels = random.sample(sorted(r for r, m in marks.items() if m.tempos), 60)
entries, js = [], []
for i, r in enumerate(rels):
    m = marks[r]
    col = random.choice(palette.COLOURS).name if i % 3 else None
    st = random.choice([None, 0, 3, 5]) if i % 4 else 4
    key = random.choice(["8A", "1B", "12A", None])
    en = random.randint(1, 10) if key else None
    cu = None
    if i % 2 == 0:
        down = cues.downbeats(m.tempos, m.duration + 2.0)
        cu = cues.place([16, 40, 64, 96], down, [16, 8], {"A": 16, "G": 16}, True)
    lvl = ["colour", "hue", "tone", "none"][i % 4]
    entries.append(rekordbox.Entry(r, lvl, Label(col, st), key, en, cu))
    js.append({"rel": r, "level": lvl, "colour": col, "stars": st, "key": key, "energy": en, "cues": [list(c.items()) for c in cu] if cu else None})
entries.append(rekordbox.Entry("not/in/the/collection.mp3", "none", Label("Red", 2)))
js.append({"rel": "not/in/the/collection.mp3", "level": "none", "colour": "Red", "stars": 2, "key": None, "energy": None, "cues": None})
st = rekordbox.write(src, out_py, root, entries, name="vibecheck-parity")
json.dump(js, open(ent_json, "w"))
print({k: v for k, v in st.items() if k != "backup"})

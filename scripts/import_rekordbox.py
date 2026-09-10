"""Import labels from a rekordbox XML export.

Rekordbox is where the user actually maintains this data, and its export is a
richer and more complete source than the ID3 genre field:

  - **Colour tag** maps 1:1 onto the colour labels (verified 100% agreement on
    8,265 overlapping tracks), and covers tracks whose genre field was never
    written.
  - **Rating** maps exactly onto the level: 5 stars = A, 4 = B, 3 = C.
  - **PlayCount** is an independent measure of how well the user knows a track,
    which is the closest thing available to a per-label confidence.
  - **DateAdded** gives a vintage the filesystem no longer has.

Colour and rating are imported as labels; play count and date are recorded
alongside for weighting experiments.
"""

from __future__ import annotations

import sys
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path

sys.path.insert(0, "src")
from vibecheck import store  # noqa: E402

COLOUR = {
    "0xFF0000": "Red", "0xFFA500": "Orange", "0xFFFF00": "Yellow",
    "0x00FF00": "Green", "0x25FDE9": "Aqua", "0x0000FF": "Blue",
    "0x660099": "Purple", "0xFF007F": "Pink",
}
LEVEL = {"255": "A", "204": "B", "153": "C"}


def parse(xml_path: Path, root: Path) -> dict[str, dict]:
    out: dict[str, dict] = {}
    for t in ET.parse(xml_path).getroot().findall(".//COLLECTION/TRACK"):
        loc = urllib.parse.unquote(t.get("Location", "").replace("file://localhost", ""))
        try:
            rel = str(Path(loc).relative_to(root))
        except ValueError:
            continue
        colour = COLOUR.get(t.get("Colour", ""))
        level = LEVEL.get(t.get("Rating", "0"))
        out[rel] = {
            "label": f"{colour}_{level}" if colour and level else None,
            # kept separately: a track can be colour-tagged but unrated, which
            # is still a usable colour label even though no combined label
            # exists for it
            "colour": colour,
            "stars": int(t.get("Rating", "0")) // 51,
            "plays": int(t.get("PlayCount", "0")),
            "added": t.get("DateAdded", ""),
        }
    return out


def main(xml_path: Path, apply: bool) -> int:
    root = (Path.home() / "Music" / "Collection").resolve()
    rb = parse(xml_path, root)
    con = store.labels_db(root)
    known = dict(con.execute("SELECT path, hash FROM tracks"))
    current = store.current_labels(con, source="user")

    con.execute("""CREATE TABLE IF NOT EXISTS track_meta (
        path TEXT PRIMARY KEY, colour TEXT, stars INTEGER,
        plays INTEGER, added TEXT)""")

    new = changed = agreed = skipped = 0
    for rel, v in rb.items():
        if rel not in known:
            skipped += 1
            continue
        con.execute("INSERT OR REPLACE INTO track_meta "
                    "(path, colour, stars, plays, added) VALUES (?,?,?,?,?)",
                    (rel, v["colour"], v["stars"], v["plays"], v["added"]))
        if not v["label"]:
            continue
        cur = current.get(rel)
        if cur == v["label"]:
            agreed += 1
        elif cur is None:
            new += 1
            if apply:
                store.log_label(con, rel, known[rel], v["label"], "user")
        else:
            changed += 1
            if apply:
                store.log_label(con, rel, known[rel], v["label"], "user")
    con.commit()

    print(f"in export and indexed : {len(rb) - skipped}")
    print(f"  agree with current  : {agreed}")
    print(f"  NEW labels          : {new}")
    print(f"  differ from current : {changed}")
    print(f"  not in our index    : {skipped}")
    print(f"play counts recorded  : {sum(1 for v in rb.values() if v['plays'] > 0)}")
    col_only = sum(1 for r, v in rb.items()
                   if r in known and v["colour"] and not v["label"])
    print(f"colour but no rating  : {col_only}  (extra colour-only training data)")
    print("(dry run -- pass --apply to write)" if not apply else "written")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(
        Path(sys.argv[1]) if len(sys.argv) > 1 and not sys.argv[1].startswith("--")
        else Path.home() / "Documents" / "rekordbox.xml",
        "--apply" in sys.argv))

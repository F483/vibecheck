"""Write a rekordbox XML that clears colour and rating on everything except
the labels the database currently holds.

Colour and star rating live in rekordbox's own database, not in the files, so
they can only be changed through rekordbox. This writes an export with those
fields emptied and a playlist of the affected tracks, to be imported.

Whether importing an XML actually *clears* a field, as opposed to only setting
non-empty ones, is not documented anywhere I can find -- so start with --limit
on a handful of tracks and check in rekordbox before doing the whole library.
"""

from __future__ import annotations

import datetime as dt
import sys
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path

sys.path.insert(0, "src")
from vibecheck import store  # noqa: E402


def main(xml_in: Path, xml_out: Path, limit: int, keep_confirmed: bool) -> int:
    root = (Path.home() / "Music" / "Collection").resolve()
    con = store.labels_db(root)
    keep = set(store.current_labels(con, source="user")) if keep_confirmed else set()
    keep = {store.norm(k) for k in keep}

    tree = ET.parse(xml_in)
    el_root = tree.getroot()
    targets = []
    for t in el_root.findall(".//COLLECTION/TRACK"):
        loc = urllib.parse.unquote(t.get("Location", "").replace("file://localhost", ""))
        try:
            rel = store.norm(str(Path(loc).relative_to(root)))
        except ValueError:
            continue
        if rel in keep:
            continue
        if not t.get("Colour") and t.get("Rating", "0") == "0":
            continue
        targets.append(t)

    if limit:
        targets = targets[:limit]
    for t in targets:
        t.set("Colour", "")
        t.set("Rating", "0")

    playlists = el_root.find("PLAYLISTS/NODE")
    node = ET.SubElement(playlists, "NODE",
                         {"Name": f"vibecheck reset {dt.date.today():%Y-%m-%d}",
                          "Type": "1", "KeyType": "0",
                          "Entries": str(len(targets))})
    for t in targets:
        ET.SubElement(node, "TRACK", {"Key": t.get("TrackID")})

    tree.write(xml_out, encoding="UTF-8", xml_declaration=True)
    print(f"{len(targets)} tracks set to no colour and no rating")
    print(f"keeping {len(keep)} confirmed labels untouched")
    print(f"wrote {xml_out}")
    print("import in rekordbox, then check whether the fields actually cleared")
    return 0


if __name__ == "__main__":
    a = sys.argv[1:]
    limit = int(a[a.index("--limit") + 1]) if "--limit" in a else 0
    raise SystemExit(main(
        Path("~/Documents/rekordbox.xml").expanduser(),
        Path(a[0]) if a and not a[0].startswith("--") else Path("reset.xml"),
        limit, "--include-confirmed" not in a))

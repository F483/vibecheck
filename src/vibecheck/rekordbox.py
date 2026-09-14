"""Write a rekordbox XML with predictions applied, for import.

Rekordbox watches one XML file (Preferences > Advanced > Database > rekordbox
xml), shows it in the sidebar, and re-reads it on refresh. Mixed In Key works
by rewriting that same file in place, and so does this: write back to the
watched path, hit refresh in rekordbox, and the new playlist is there.

Writing somewhere else produces a file rekordbox never looks at, which is why
this updates in place -- keeping a .bak of what was there before.

One flat playlist per batch, named with the time so several a day do not
collide. How specific each prediction was shows in the genre tag rather than in
the playlist structure -- a track reading "Vibrant" got a hue, one reading
"Pink_C" got the lot.
"""

from __future__ import annotations

import datetime as dt
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path

from . import store

STARS = {5: "255", 4: "204", 3: "153", 2: "102", 1: "51", 0: "0"}


def _locations(root_el, collection: Path) -> dict[str, ET.Element]:
    out = {}
    for t in root_el.findall(".//COLLECTION/TRACK"):
        loc = urllib.parse.unquote(t.get("Location", "").replace("file://localhost", ""))
        try:
            out[store.norm(str(Path(loc).relative_to(collection)))] = t
        except ValueError:
            continue
    return out


def read_labels(xml_in: Path, collection: Path, colours: dict[str, str],
                ratings: dict[str, int], separator: str = "_",
                only: set[str] | None = None) -> dict[str, str]:
    """Labels as rekordbox currently has them: colour tag plus star rating.

    `only` restricts the read, and callers should almost always pass it. The
    export carries the whole library, including tracks whose colour predates
    whatever scheme is current, and adopting all of it wholesale would quietly
    resurrect a retired vocabulary.
    """
    by_hex = {v: k for k, v in colours.items()}
    by_star = {STARS[v]: k for k, v in ratings.items()}
    out: dict[str, str] = {}
    for t in ET.parse(xml_in).getroot().findall(".//COLLECTION/TRACK"):
        loc = urllib.parse.unquote(t.get("Location", "").replace("file://localhost", ""))
        try:
            rel = store.norm(str(Path(loc).relative_to(collection)))
        except ValueError:
            continue
        if only is not None and rel not in only:
            continue
        colour = by_hex.get(t.get("Colour", ""))
        if not colour:
            continue
        level = by_star.get(t.get("Rating", "0"))
        out[rel] = f"{colour}{separator}{level}" if level else colour
    return out


def write(xml_in: Path, xml_out: Path, collection: Path,
          predictions: list[tuple[str, str, str | None]],
          colours: dict[str, str], ratings: dict[str, int],
          separator: str = "_", name: str | None = None) -> dict:
    """predictions: (relative path, level, label)

    `colours` maps a label's colour part to a rekordbox hex value, `ratings`
    maps its level part to a star count. Both come from config -- nothing here
    knows what a label means.
    """
    tree = ET.parse(xml_in)
    root_el = tree.getroot()
    by_path = _locations(root_el, collection)

    groups: dict[str, list[str]] = {}
    stats = {"coloured": 0, "rated": 0, "not_in_xml": 0}

    for rel, level, label in predictions:
        el = by_path.get(store.norm(rel))
        if el is None:
            stats["not_in_xml"] += 1
            continue
        key = el.get("TrackID")
        groups.setdefault(level, []).append(key)
        if label is None:
            continue
        head, _, tail = label.partition(separator)
        if head in colours:
            el.set("Colour", colours[head])
            stats["coloured"] += 1
        if tail and tail in ratings:
            el.set("Rating", STARS[ratings[tail]])
            stats["rated"] += 1

    keys = [k for level in ("full", "colour", "hue", "tone", "none")
            for k in groups.get(level, [])]
    playlists = root_el.find("PLAYLISTS/NODE")
    node = ET.SubElement(playlists, "NODE",
                         {"Name": name or f"vibecheck-{dt.datetime.now():%Y%m%d-%H%M}",
                          "Type": "1", "KeyType": "0",
                          "Entries": str(len(keys))})
    for k in keys:
        ET.SubElement(node, "TRACK", {"Key": k})
    playlists.set("Count", str(len(list(playlists))))

    if xml_out == xml_in:
        backup = xml_in.with_suffix(xml_in.suffix + ".bak")
        backup.write_bytes(xml_in.read_bytes())
        stats["backup"] = str(backup)
    tree.write(xml_out, encoding="UTF-8", xml_declaration=True)
    stats["playlists"] = {k: len(v) for k, v in groups.items()}
    return stats

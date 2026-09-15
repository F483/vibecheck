"""Write a rekordbox XML with predictions applied, for import.

Rekordbox watches one XML file (Preferences > Advanced > Database > rekordbox
xml), shows it in the sidebar, and re-reads it on refresh. Mixed In Key works
by rewriting that same file in place, and so does this: write back to the
watched path, hit refresh in rekordbox, and the new playlist is there.

Writing somewhere else produces a file rekordbox never looks at, which is why
this updates in place -- keeping a .bak of what was there before.

One flat playlist per batch, named with the time so several a day do not
collide. Tracks are ordered by how specific the prediction was, so the ones the
model committed to come first.
"""

from __future__ import annotations

import datetime as dt
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path

from . import palette, store
from .store import Label


def _locations(root_el, collection: Path) -> dict[str, ET.Element]:
    out = {}
    for t in root_el.findall(".//COLLECTION/TRACK"):
        loc = urllib.parse.unquote(t.get("Location", "").replace("file://localhost", ""))
        try:
            out[store.norm(str(Path(loc).relative_to(collection)))] = t
        except ValueError:
            continue
    return out


def read_labels(xml_in: Path, collection: Path,
                only: set[str] | None = None) -> dict[str, Label]:
    """Colour and star rating as rekordbox currently has them.

    A track rekordbox knows is a complete statement: it has a colour or it has
    none, it has stars or it has none. That is why one label row holds both.

    `only` restricts the read, and callers should almost always pass it. The
    export carries the whole library, including tracks whose colour predates
    whatever scheme is current, and adopting all of it wholesale would quietly
    resurrect a retired vocabulary.
    """
    out: dict[str, Label] = {}
    for t in ET.parse(xml_in).getroot().findall(".//COLLECTION/TRACK"):
        loc = urllib.parse.unquote(t.get("Location", "").replace("file://localhost", ""))
        try:
            rel = store.norm(str(Path(loc).relative_to(collection)))
        except ValueError:
            continue
        if only is not None and rel not in only:
            continue
        colour = palette.BY_RGB.get(t.get("Colour", ""))
        out[rel] = Label(colour.name if colour else None,
                         palette.stars_from_rating(t.get("Rating")))
    return out


def write(xml_in: Path, xml_out: Path, collection: Path,
          predictions: list[tuple[str, str, Label]],
          name: str | None = None) -> dict:
    """predictions: (relative path, how specific it was, what to write)."""
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
        groups.setdefault(level, []).append(el.get("TrackID"))
        if label.colour:
            el.set("Colour", palette.BY_NAME[label.colour].rgb)
            stats["coloured"] += 1
        if label.stars:
            el.set("Rating", palette.rating_from_stars(label.stars))
            stats["rated"] += 1

    keys = [k for level in ("colour", "hue", "tone", "none")
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

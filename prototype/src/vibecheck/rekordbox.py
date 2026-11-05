"""Write a rekordbox XML with predictions applied, for import.

Rekordbox watches one XML file (Preferences > Advanced > Database > rekordbox
xml), shows it in the sidebar, and re-reads it on refresh. So this rewrites
that same file in place: write back to the watched path, hit refresh in
rekordbox, and the new playlist is there.

Writing somewhere else produces a file rekordbox never looks at, which is why
this updates in place -- keeping a .bak of what was there before.

One flat playlist per batch, named with the time so several a day do not
collide. Tracks are ordered by how specific the prediction was, so the ones the
model committed to come first.
"""

from __future__ import annotations

import datetime as dt
import re
import urllib.parse
import xml.etree.ElementTree as ET
from pathlib import Path
from typing import NamedTuple

from . import cues as cues_mod, palette, store
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


def read_marks(xml_in: Path, collection: Path,
               only: set[str] | None = None) -> dict[str, "cues_mod.Marks"]:
    """Each track's beat grid, length and cues, as rekordbox has them."""
    out = {}
    for t in ET.parse(xml_in).getroot().findall(".//COLLECTION/TRACK"):
        loc = urllib.parse.unquote(t.get("Location", "").replace("file://localhost", ""))
        try:
            rel = store.norm(str(Path(loc).relative_to(collection)))
        except ValueError:
            continue
        if only is not None and rel not in only:
            continue
        tempos = [(float(x.get("Inizio")), float(x.get("Bpm")), int(x.get("Battito")))
                  for x in t.findall("TEMPO")]
        out[rel] = cues_mod.Marks(tempos, float(t.get("TotalTime") or 0),
                                  [dict(p.attrib) for p in t.findall("POSITION_MARK")])
    return out


class Entry(NamedTuple):
    """One track's worth of writing. None anywhere means "leave that alone"."""
    rel: str
    level: str = "none"            # how far the colour got; orders the playlist
    label: Label | None = None     # colour and rating
    key: str | None = None         # alphanumeric, "8A"
    energy: int | None = None      # 1..10
    cues: list[dict] | None = None  # every POSITION_MARK the track should have


# A leading "<key> - Energy <n>", whoever wrote it. Replaced, never read: the
# values in it are not this app's and are never used for anything.
_KEY_ENERGY = re.compile(r"^\s*\d{1,2}[AB]\s*-\s*Energy\s*\d{1,2}\b\s*")


def comment_with(old: str | None, key: str, energy: int) -> str:
    """The comment with its key-energy prefix replaced, everything else kept.

    "5A - Energy 7 /* Atmo / Rytm */" -> "8A - Energy 6 /* Atmo / Rytm */".
    """
    rest = _KEY_ENERGY.sub("", old or "", count=1).strip()
    head = f"{key} - Energy {energy}"
    return f"{head} {rest}" if rest else head


def write(xml_in: Path, xml_out: Path, collection: Path,
          entries: list[Entry], name: str | None = None, *,
          tonality: bool = True, comment: bool = True,
          changed_only: bool = False) -> dict:
    """Apply entries and add one playlist holding them.

    `tonality` and `comment` say whether key and energy reach those fields.
    `changed_only` keeps the playlist to tracks something was actually changed
    on, which is what a whole-collection pass wants and a labelling round,
    where every track is to be reviewed, does not.
    """
    tree = ET.parse(xml_in)
    root_el = tree.getroot()
    by_path = _locations(root_el, collection)

    groups: dict[str, list[str]] = {}
    stats = {"coloured": 0, "rated": 0, "keyed": 0, "commented": 0,
             "cued": 0, "not_in_xml": 0, "unchanged": 0}

    def put(el, attr, value, counter) -> bool:
        if el.get(attr) == value:
            return False
        el.set(attr, value)
        stats[counter] += 1
        return True

    for e in entries:
        el = by_path.get(store.norm(e.rel))
        if el is None:
            stats["not_in_xml"] += 1
            continue
        changed = False
        if e.label and e.label.colour:
            changed |= put(el, "Colour", palette.BY_NAME[e.label.colour].rgb,
                           "coloured")
        if e.label and e.label.stars:
            changed |= put(el, "Rating", palette.rating_from_stars(e.label.stars),
                           "rated")
        if e.key and tonality:
            changed |= put(el, "Tonality", e.key, "keyed")
        if e.key and e.energy is not None and comment:
            changed |= put(el, "Comments",
                           comment_with(el.get("Comments"), e.key, e.energy),
                           "commented")
        if e.cues is not None:
            # Importing replaces all of a track's cues with the ones here, so
            # this is the whole set, not an addition to it.
            old = [dict(p.attrib) for p in el.findall("POSITION_MARK")]
            if old != e.cues:
                for p in el.findall("POSITION_MARK"):
                    el.remove(p)
                for m in e.cues:
                    ET.SubElement(el, "POSITION_MARK", m)
                stats["cued"] += 1
                changed = True
        if changed_only and not changed:
            stats["unchanged"] += 1
            continue
        groups.setdefault(e.level, []).append(el.get("TrackID"))

    keys = [k for level in ("colour", "hue", "tone", "none")
            for k in groups.get(level, [])]
    stats["playlists"] = {k: len(v) for k, v in groups.items()}
    if changed_only and not keys:
        return stats              # nothing to import: leave the file alone
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
    return stats

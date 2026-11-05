"""Write a rekordbox XML carrying the labels this app is confident of.

Use after clearing colour and rating in rekordbox by hand: importing this puts
the confirmed labels back as real colour tags and star ratings, and nothing
else. Relies only on import *setting* fields, which is known to work, rather
than on clearing them, which is not.
"""

from __future__ import annotations

import sys
from pathlib import Path

sys.path.insert(0, "src")
import tomllib  # noqa: E402

from vibecheck import rekordbox, store  # noqa: E402


def main(out: Path | None) -> int:
    root = (Path.home() / "Music" / "Collection").resolve()
    cfg = tomllib.loads(Path("src/vibecheck/default_config.toml").read_text())
    labels = store.current(store.labels_db(root), source='user')
    preds = [(rel, "full", lbl) for rel, lbl in sorted(labels.items())]
    src = Path("~/Documents/rekordbox.xml").expanduser()
    out = out or src
    st = rekordbox.write(
        src, out, root, preds,
        cfg["rekordbox"]["colours"], cfg["rekordbox"]["ratings"],
        cfg["labels"].get("separator", "_"))
    print(f"{len(preds)} confirmed labels")
    print(f"   {st['coloured']} colours set, {st['rated']} ratings set"
          + (f", {st['not_in_xml']} not found in the export"
             if st["not_in_xml"] else ""))
    print(f"wrote {out}")
    if st.get("backup"):
        print(f"   previous export kept at {Path(st['backup']).name}")
    print("   refresh the rekordbox xml node in rekordbox's sidebar")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(Path(sys.argv[1]) if len(sys.argv) > 1 else None))

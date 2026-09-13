"""Start the label history over from the fresh relabelling.

The 8,619 accumulated labels are on a scale the maintainer has moved past --
measured, training on them scores *worse* than training on the 300 fresh ones
(1.60 vs 1.31 cost per track). They are not deleted from the database's history
table; the current-label view is simply rebuilt from the fresh set only.

Embeddings are untouched: they cost hours and do not depend on labels.
"""

from __future__ import annotations

import shutil
import sys
from pathlib import Path

sys.path.insert(0, "src")
from vibecheck import playlist, store, tags  # noqa: E402


def main(apply: bool) -> int:
    root = (Path.home() / "Music" / "Collection").resolve()
    d = store.vibecheck_dir(root)
    con = store.labels_db(root)
    hb = dict(con.execute("SELECT path, hash FROM tracks"))

    keep = playlist.read(Path("out/relabel_blind.m3u8"), root)
    fresh = {rel: lbl for rel in keep
             if rel in hb and (lbl := tags.read(root / rel))}
    before = len(store.current_labels(con, source="user"))
    print(f"current labels: {before}")
    print(f"fresh labels to keep: {len(fresh)}")

    if not apply:
        print("(dry run -- pass --apply)")
        return 0
    if len(fresh) < 250:
        print("ABORT: fewer fresh labels than expected; refusing to wipe")
        return 1

    shutil.copy2(d / "labels.db", d / "labels.db.before-reset")
    con.execute("DELETE FROM label_log")
    for rel, lbl in fresh.items():
        store.log_label(con, rel, hb[rel], lbl, "user")
    con.commit()
    after = store.current_labels(con, source="user")
    print(f"label history rebuilt: {len(after)} labels")
    print(f"backup at {d / 'labels.db.before-reset'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main("--apply" in sys.argv))

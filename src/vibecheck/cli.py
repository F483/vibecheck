"""Three commands.

    vibecheck scan              find tracks and pick up label changes
    vibecheck status            where things stand
    vibecheck label [N]         pick N unlabelled tracks, label what it can,
                                write a playlist for you to correct
    vibecheck sync [playlist]   read your corrections back and retrain
    vibecheck discard [playlist]  throw a batch away and free its tracks
    vibecheck clear ...         housekeeping: remove genre tags

The loop is: label -> correct in your DJ software -> sync -> repeat.
"""

from __future__ import annotations

import argparse
import datetime as dt
import random
import sys
import tomllib
from pathlib import Path

import numpy as np

from . import embed, index, playlist, store, tags
from .config import DEFAULT
from .predict import Model, levels_from

BACKEND = "clapw"


def load_config(root: Path) -> dict:
    user = store.vibecheck_dir(root) / "config.toml"
    src = user if user.exists() else Path(__file__).with_name("default_config.toml")
    return tomllib.loads(src.read_text())


def build_levels(cfg: dict, labels: list[str]):
    """Finest to coarsest, from the observed vocabulary plus config groupings.

    The app still never interprets a label. Config supplies the separator that
    splits a tag into its parts, and the groupings over the leading part.
    """
    sep = cfg["labels"].get("separator", "")
    head = (lambda l: l.split(sep)[0]) if sep else (lambda l: l)
    maps = [("full", {l: l for l in labels})]
    if sep and any(sep in l for l in labels):
        maps.append(("colour", {l: head(l) for l in labels}))
    for entry in cfg.get("hierarchy", []):
        g = {c: grp for grp, members in entry.items() if grp != "name"
             for c in members}
        if all(head(l) in g for l in labels):
            maps.append((entry["name"], {l: g[head(l)] for l in labels}))
    return levels_from(labels, maps)


def labelled(root: Path, cfg: dict) -> dict[str, str]:
    """Labels the user stands behind -- the only thing worth training on."""
    return store.current_labels(store.labels_db(root), source="user")


def spoken_for(root: Path) -> set[str]:
    """Tracks that already carry a label from anyone, including this app.

    A model-written label means the track is out in a batch the user is
    partway through correcting. Selecting it again would overwrite a correction
    that has not been synced yet -- silently, since the tag looks like
    something the app wrote in the first place.
    """
    return set(store.current_labels(store.labels_db(root)))


def train(root: Path, cfg: dict):
    lab = labelled(root, cfg)
    if len(lab) < 20:
        sys.exit(f"only {len(lab)} labels; label some tracks first")
    paths, X = embed.load(root, BACKEND, DEFAULT, sorted(lab))
    y = np.array([lab[p] for p in paths])
    m = Model(build_levels(cfg, sorted(set(y))))
    info = m.train(X, y)
    return m, info


def cmd_scan(root: Path, cfg: dict, args) -> None:
    print(f"scanning {root}", flush=True)
    st = index.scan(root)
    print(f"   {st.files} mp3 files, {st.new} new, {st.changed} changed")
    print(f"   labels: {st.labels_added} added, {st.labels_changed} corrected"
          + (f", {st.unreadable} unreadable" if st.unreadable else ""))


def cmd_status(root: Path, cfg: dict, args) -> None:
    con = store.labels_db(root)
    total = con.execute("SELECT COUNT(*) FROM tracks").fetchone()[0]
    lab = labelled(root, cfg)
    cache = store.cache_db(root)
    have = {h for (h,) in cache.execute(
        "SELECT hash FROM embeddings WHERE backend=?", (BACKEND,))}
    hb = dict(con.execute("SELECT path, hash FROM tracks"))
    print(f"collection      {total} tracks")
    print(f"labelled        {len(lab)}")
    print(f"unlabelled      {total - len(lab)}")
    print(f"embedded        {sum(1 for p in hb if hb[p] in have)}")
    if lab:
        from collections import Counter
        for l, n in Counter(lab.values()).most_common(8):
            print(f"   {l:12} {n}")


def cmd_label(root: Path, cfg: dict, args) -> None:
    con = store.labels_db(root)
    hb = dict(con.execute("SELECT path, hash FROM tracks"))
    confirmed = set(labelled(root, cfg))
    taken = confirmed if args.include_unconfirmed else spoken_for(root)
    pool = [p for p in sorted(hb) if p not in taken]
    if not pool:
        sys.exit("nothing left unlabelled")
    out_now = len(spoken_for(root)) - len(confirmed)
    if out_now and not args.include_unconfirmed:
        print(f"note: {out_now} tracks are out in a batch awaiting your "
              f"corrections and are excluded (--include-unconfirmed to reuse)")
    elif out_now:
        print(f"note: including {out_now} tracks that already carry an "
              f"unconfirmed label")
    random.seed(args.seed)
    batch = sorted(random.sample(pool, min(args.count, len(pool))))
    print(f"[1/4] selected {len(batch)} of {len(pool)} unlabelled tracks")

    print(f"[2/4] listening to the tracks that are new to it "
          f"(about 3 seconds each)", flush=True)
    st = embed.embed_all(root, BACKEND, DEFAULT, batch, workers=1)
    print(f"      {st['done']} embedded, {st['cached']} already known"
          + (f", {st['failed']} failed" if st["failed"] else ""))

    print("[3/4] training on what you have labelled so far", flush=True)
    model, info = train(root, cfg)
    print(f"      {info['tracks']} tracks, {info['labels']} labels, "
          f"levels: {' > '.join(reversed(info['levels']))}")

    print("[4/4] deciding what it can say about each track", flush=True)
    paths, X = embed.load(root, BACKEND, DEFAULT, batch)
    preds = model.predict(X, cfg["predict"]["misleading_cost"])

    # tags this app did not write, and does not know about: another tool's
    # labels, or an earlier scheme. Overwriting them silently would destroy
    # work the database has no record of.
    foreign = [r for r in paths
               if r not in spoken_for(root) and tags.read(root / r)]
    if foreign and not args.dry_run:
        print(f"      note: {len(foreign)} of these already carry a genre tag "
              f"this app did not write; it will be replaced")

    entries, counts = [], {}
    for rel, p in zip(paths, preds):
        counts[p.level] = counts.get(p.level, 0) + 1
        if p.label is not None and not args.dry_run:
            before = tags.read(root / rel)
            store.log_label(con, rel, hb[rel], p.label, "model")
            if before != p.label:
                tags.write(root / rel, p.label)
        entries.append((rel, f"{p.label or '?':12} | {Path(rel).name}"))
    con.commit()

    out = Path(args.output or
               f"batch-{dt.datetime.now():%Y%m%d-%H%M}.m3u8")
    risky = playlist.write(out, root, entries)
    print(f"\nwrote {out}")
    if risky:
        print(f"   note: {len(risky)} path(s) contain '#', which Rekordbox "
              f"treats as a comment and will skip:")
        for r in risky[:3]:
            print(f"     {r}")
    for k, v in sorted(counts.items(), key=lambda kv: -kv[1]):
        print(f"   {k:7} {v}")
    print("\ncorrect them in your DJ software, then: vibecheck sync " + str(out))


def cmd_discard(root: Path, cfg: dict, args) -> None:
    """Undo a batch: forget its unconfirmed labels and clear the tags it wrote.

    Only tags still holding exactly what the app wrote are cleared. A tag the
    user has since changed is a correction, not a discard, so it is kept and
    reported -- losing it would be indistinguishable from the app never having
    predicted the track.
    """
    con = store.labels_db(root)
    hb = dict(con.execute("SELECT path, hash FROM tracks"))
    canon = store.path_index(con)
    confirmed = set(labelled(root, cfg))
    rows = {p: l for p, l in con.execute(
        """SELECT l.path, l.label FROM label_log l
           JOIN (SELECT path, MAX(id) id FROM label_log GROUP BY path) m
             ON l.id = m.id WHERE l.source = 'model'""")}

    if args.input:
        rels = [canon.get(store.norm(r), r)
                for r in playlist.read(Path(args.input), root)]
    else:
        rels = list(rows)

    cleared = kept = 0
    for rel in rels:
        if rel in confirmed or rel not in rows:
            continue
        now = tags.read(root / rel)
        if now != rows[rel]:
            kept += 1          # user has edited it: that is a correction
            continue
        if not args.dry_run:
            tags.write(root / rel, None)
            store.log_label(con, rel, hb[rel], None, "model")
        cleared += 1
    con.commit()
    print(f"discarded {cleared} unconfirmed labels"
          + (f"; kept {kept} you had already corrected (sync to keep them)"
             if kept else ""))
    if args.dry_run:
        print("(dry run -- nothing changed)")


def cmd_clear(root: Path, cfg: dict, args) -> None:
    """Remove genre tags. Housekeeping, and destructive, so it asks twice.

    Anything cleared is first written to .vibecheck/cleared-<date>.csv, because
    a tag the database never knew about has no other record anywhere.
    """
    import csv
    import datetime as dt

    con = store.labels_db(root)
    hb = dict(con.execute("SELECT path, hash FROM tracks"))
    canon = store.path_index(con)
    confirmed = set(labelled(root, cfg))
    known = set(store.current_labels(con))

    scope = ([canon.get(store.norm(r), r)
              for r in playlist.read(Path(args.input), root)]
             if args.input else sorted(hb))

    def wanted(rel: str) -> bool:
        if args.all:
            return True
        if args.unknown and rel not in known:
            return True
        if args.unconfirmed and rel in known and rel not in confirmed:
            return True
        if args.confirmed and rel in confirmed:
            return True
        return False

    if not (args.all or args.unknown or args.unconfirmed or args.confirmed):
        sys.exit("choose what to clear: --unknown, --unconfirmed, "
                 "--confirmed or --all")

    targets = []
    for rel in scope:
        if not wanted(rel):
            continue
        tag = tags.read(root / rel)
        if tag:
            targets.append((rel, tag))

    print(f"would clear {len(targets)} genre tags")
    for rel, tag in targets[:5]:
        print(f"   {tag:12} {Path(rel).name[:56]}")
    if len(targets) > 5:
        print(f"   ... and {len(targets) - 5} more")
    if not args.apply:
        print("\n(nothing changed -- pass --apply to do it)")
        return

    out = store.vibecheck_dir(root) / f"cleared-{dt.datetime.now():%Y%m%d-%H%M}.csv"
    with open(out, "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["path", "label"])
        w.writerows(targets)
    for rel, _ in targets:
        tags.write(root / rel, None)
        if rel in known:
            store.log_label(con, rel, hb[rel], None, "user")
    con.commit()
    print(f"cleared {len(targets)}; saved to {out}")


def cmd_sync(root: Path, cfg: dict, args) -> None:
    con = store.labels_db(root)
    hb = dict(con.execute("SELECT path, hash FROM tracks"))
    last_user = store.current_labels(con, source="user")
    last_any = store.current_labels(con)

    src = args.input
    if src is None:
        batches = sorted(Path(".").glob("batch-*.m3u8"))
        if batches:
            src = str(batches[-1])
            print(f"using most recent batch: {src}")
        elif not args.all:
            sys.exit(
                "no batch playlist given and none found.\n"
                "  Pass one, or --all to read every tag in the collection.\n"
                "  --all adopts whatever the files currently say, which may\n"
                "  include labels from other software or from an earlier\n"
                "  scheme you have since moved on from.")
    canon = store.path_index(con)
    raw = playlist.read(Path(src), root) if src else sorted(hb)
    rels = [canon.get(store.norm(r), r) for r in raw]
    args.input = src
    added = changed = confirmed = missing = 0
    for rel in rels:
        if rel not in hb:
            missing += 1
            continue
        now = tags.read(root / rel)
        if now is None:
            continue
        if now != last_any.get(rel):
            store.log_label(con, rel, hb[rel], now, "user")
            changed += 1 if rel in last_user else 0
            added += 0 if rel in last_user else 1
        elif args.input and rel not in last_user:
            # in a batch the user reviewed: an unchanged model label is now theirs
            store.log_label(con, rel, hb[rel], now, "user")
            confirmed += 1
    con.commit()
    print(f"new labels {added}, corrections {changed}, confirmed {confirmed}"
          + (f", not in index {missing}" if missing else ""))
    print(f"labelled now: {len(labelled(root, cfg))}")


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(prog="vibecheck")
    ap.add_argument("--root", default=str(Path.home() / "Music" / "Collection"))
    sub = ap.add_subparsers(dest="cmd", required=True)
    sub.add_parser("scan")
    sub.add_parser("status")
    p = sub.add_parser("label")
    p.add_argument("count", nargs="?", type=int, default=300)
    p.add_argument("--output", default=None,
                   help="default: batch-<date>-<time>.m3u8")
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--dry-run", action="store_true",
                   help="predict and write the playlist, but change nothing")
    p.add_argument("--include-unconfirmed", action="store_true",
                   help="also pick tracks that already carry an unconfirmed "
                        "label from a previous batch")
    p = sub.add_parser("clear")
    p.add_argument("input", nargs="?", help="restrict to a playlist")
    p.add_argument("--unknown", action="store_true",
                   help="tags this app has no record of")
    p.add_argument("--unconfirmed", action="store_true",
                   help="labels it wrote that you have not confirmed")
    p.add_argument("--confirmed", action="store_true",
                   help="your own labels (destroys work)")
    p.add_argument("--all", action="store_true", help="every genre tag")
    p.add_argument("--apply", action="store_true", help="actually do it")
    p = sub.add_parser("discard")
    p.add_argument("input", nargs="?", help="playlist; default: every "
                                            "unconfirmed label")
    p.add_argument("--dry-run", action="store_true")
    p = sub.add_parser("sync")
    p.add_argument("input", nargs="?")
    p.add_argument("--all", action="store_true",
                   help="read every tag in the collection, not just a batch")
    args = ap.parse_args(argv)
    root = Path(args.root).resolve()
    cfg = load_config(root)
    return {"scan": cmd_scan, "status": cmd_status, "label": cmd_label,
            "discard": cmd_discard, "clear": cmd_clear,
            "sync": cmd_sync}[args.cmd](
        root, cfg, args) or 0


if __name__ == "__main__":
    raise SystemExit(main())

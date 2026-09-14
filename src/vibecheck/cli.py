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

from . import embed, index, playlist, rekordbox, store, tags
from .config import DEFAULT
from .predict import Axis, Model

BACKEND = "clapw"


def load_config(root: Path) -> dict:
    user = store.vibecheck_dir(root) / "config.toml"
    src = user if user.exists() else Path(__file__).with_name("default_config.toml")
    return tomllib.loads(src.read_text())


def build_axes(cfg: dict, labels: list[str]) -> tuple[list, dict]:
    """Axes from config, plus the grid that turns axis values back into a label.

    The app still never interprets a label: config says where the string
    divides and which values group together, and the grid is derived from what
    the labels actually are.
    """
    sep = cfg["labels"].get("separator", "")
    head = (lambda l: l.split(sep)[0]) if sep else (lambda l: l)
    tail = (lambda l: l.split(sep)[1] if sep and sep in l else None)
    mc = cfg["predict"]["misleading_cost"]
    rmc = cfg["predict"].get("rating_misleading_cost", mc)

    axes = []
    for entry in cfg.get("hierarchy", []):
        g = {c: grp for grp, members in entry.items() if grp != "name"
             for c in members}
        if all(head(l) in g for l in labels):
            groups = {grp for grp in entry if grp != "name"}
            axes.append(Axis(entry["name"], {l: g[head(l)] for l in labels},
                             mc, n_values=len(groups)))
    if any(tail(l) for l in labels):
        declared = cfg["labels"].get("levels") or []
        axes.append(Axis("rating", {l: tail(l) or "?" for l in labels}, rmc,
                         n_values=len(declared)))

    # (hue, tone) -> colour, read off the labels themselves
    named = [a.name for a in axes if a.name != "rating"]
    grid = {}
    for l in labels:
        key = tuple(next(a for a in axes if a.name == n).group[l] for n in named)
        grid[key] = head(l)
    return axes, {"axes": named, "grid": grid}


def compose(pred, spec: dict) -> tuple[str | None, str]:
    """The most specific thing the prediction supports, and what level that is."""
    names = spec["axes"]
    if pred.said(*names):
        key = tuple(pred.values[n] for n in names)
        got = spec["grid"].get(key)
        if got:
            return got, "colour"
    for n in names:                      # a single axis, most specific first
        if n in pred.values:
            return pred.values[n], n
    return None, "none"


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
    labs = sorted(set(y))
    axes, spec = build_axes(cfg, labs)
    m = Model(axes)
    info = m.train(X, y)
    return m, info, spec


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
    model, info, spec = train(root, cfg)
    print(f"      {info['tracks']} tracks, {info['labels']} labels, "
          f"axes: {', '.join(info['axes'])}")

    print("[4/4] deciding what it can say about each track", flush=True)
    paths, X = embed.load(root, BACKEND, DEFAULT, batch)
    preds = model.predict(X)

    # tags this app did not write, and does not know about: another tool's
    # labels, or an earlier scheme. Overwriting them silently would destroy
    # work the database has no record of.
    foreign = [r for r in paths
               if r not in spoken_for(root) and tags.read(root / r)]
    if foreign and not args.dry_run:
        print(f"      note: {len(foreign)} of these already carry a genre tag "
              f"this app did not write; it will be replaced")

    write_tags = (args.write_tags if args.write_tags is not None
                  else cfg.get("debug", {}).get("write_genre_tags", False))
    sep = cfg["labels"].get("separator", "_")
    entries, counts = [], {}
    rated = 0
    for rel, p in zip(paths, preds):
        label, level = compose(p, spec)
        rating = p.values.get("rating")
        counts[level] = counts.get(level, 0) + 1
        rated += rating is not None
        full = f"{label}{sep}{rating}" if label and rating and level == "colour" \
            else label
        if full is not None and not args.dry_run:
            store.log_label(con, rel, hb[rel], full, "model")
            if write_tags and tags.read(root / rel) != label:
                tags.write(root / rel, label)
        shown = f"{label or '?'}{' ' + rating if rating else ''}"
        entries.append((rel, f"{shown:14} | {Path(rel).name}"))
    con.commit()

    out = Path(args.output) if args.output else (
        Path("out") / f"batch-{dt.datetime.now():%Y%m%d-%H%M}.m3u8")
    out.parent.mkdir(parents=True, exist_ok=True)
    if args.rekordbox and not args.dry_run:
        src = Path(args.rekordbox).expanduser()
        if not src.exists():
            sys.exit(f"no rekordbox export at {src}\n"
                     "  export one from rekordbox (File > Export Collection in "
                     "xml format),\n  or pass --rekordbox <path>, or "
                     "--write-tags to use genre tags instead.")
        # in place: rekordbox only ever re-reads the file it is pointed at
        dst = src if args.in_place else out.with_suffix(".xml")
        st = rekordbox.write(
            src, dst, root,
            [(rel, compose(p, spec)[1],
              (lambda l, r: f"{l}{sep}{r}" if l and r else l)(
                  compose(p, spec)[0], p.values.get("rating")))
             for rel, p in zip(paths, preds)],
            cfg.get("rekordbox", {}).get("colours", {}),
            cfg.get("rekordbox", {}).get("ratings", {}),
            cfg["labels"].get("separator", "_"))
        print(f"\nwrote {dst}")
        print(f"   {st['coloured']} colours set, {st['rated']} ratings set"
              + (f", {st['not_in_xml']} not found in the export"
                 if st["not_in_xml"] else ""))
        if st.get("backup"):
            print(f"   previous export kept at {Path(st['backup']).name}")
        if args.in_place:
            print("   refresh the rekordbox xml node in rekordbox's sidebar; "
                  "the batch appears as a playlist")
        else:
            print("   point rekordbox at this file, or use --in-place to "
                  "update the one it already watches")

    risky = playlist.write(out, root, entries)
    print(f"\nwrote {out}")
    if risky:
        print(f"   note: {len(risky)} path(s) contain '#', which Rekordbox "
              f"treats as a comment and will skip:")
        for r in risky[:3]:
            print(f"     {r}")
    for k, v in sorted(counts.items(), key=lambda kv: -kv[1]):
        print(f"   {k:7} {v}")
    print(f"   rating  {rated} (independent of the colour)")
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

    canon = store.path_index(con)
    unconfirmed = {p for p in last_any if p not in last_user}

    if args.tags:
        src = args.input
        if src is None:
            batches = sorted(Path("out").glob("batch-*.m3u8"))
            if batches:
                src = str(batches[-1])
                print(f"using most recent batch: {src}")
            elif not args.all:
                sys.exit("no batch playlist given and none found; pass one, "
                         "or --all to read every tag in the collection.")
        raw = playlist.read(Path(src), root) if src else sorted(hb)
        rels = [canon.get(store.norm(r), r) for r in raw]
        reading = {rel: tags.read(root / rel) for rel in rels}
    else:
        xml = Path(args.rekordbox).expanduser()
        if not xml.exists():
            sys.exit(f"no rekordbox export at {xml}\n"
                     "  export one after correcting (File > Export Collection "
                     "in xml format),\n  or pass --tags to read genre tags "
                     "instead.")
        # An export older than the batch cannot contain the corrections, and
        # reading it would adopt whatever the tracks looked like *before* the
        # batch was made -- silently, as though the user had confirmed it.
        newest_batch = con.execute(
            "SELECT MAX(ts) FROM label_log WHERE source='model'").fetchone()[0]
        if newest_batch and xml.stat().st_mtime < newest_batch and not args.stale_ok:
            import datetime as _dt
            sys.exit(
                f"{xml.name} was exported "
                f"{_dt.datetime.fromtimestamp(xml.stat().st_mtime):%Y-%m-%d %H:%M}"
                f", before the current batch was made "
                f"({_dt.datetime.fromtimestamp(newest_batch):%Y-%m-%d %H:%M}).\n"
                "  It cannot contain your corrections. Export a fresh one from\n"
                "  rekordbox first, or pass --stale-ok if you really mean it.")
        scope = (None if args.all else
                 {store.norm(r) for r in (
                     [canon.get(store.norm(x), x)
                      for x in playlist.read(Path(args.input), root)]
                     if args.input else unconfirmed)})
        if scope is not None and not scope:
            print("nothing awaiting correction")
            return
        raw = rekordbox.read_labels(
            xml, root, cfg.get("rekordbox", {}).get("colours", {}),
            cfg.get("rekordbox", {}).get("ratings", {}),
            cfg["labels"].get("separator", "_"), only=scope)
        reading = {canon.get(k, k): v for k, v in raw.items()}
        print(f"reading {len(reading)} tracks from {xml.name}")

    added = changed = confirmed = missing = 0
    for rel, now in reading.items():
        if rel not in hb:
            missing += 1
            continue
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


USAGE = """\
the loop
  vibecheck scan                     find your tracks (once, and after adding music)
  vibecheck label 300                pick 300 unlabelled tracks, label what it
                                     can, and write the colours and star
                                     ratings into the rekordbox xml
  ... refresh the rekordbox xml node in rekordbox, import the new playlist,
      correct what is wrong, then export the collection again ...
  vibecheck sync                     read your corrections back and retrain

  Each round it learns from your corrections, so each round you correct less.
  It needs roughly 200 labels of your own before it is much use -- label a first
  batch by hand, or let it guess and correct everything.

examples
  vibecheck status                        counts, and how many labels you have
  vibecheck label 50 --dry-run            see what it would say, change nothing
  vibecheck label 300 --include-unconfirmed
                                          reuse tracks from a batch you have
                                          not corrected yet
  vibecheck discard out/batch-….m3u8      throw a batch away, free its tracks
  vibecheck sync out/batch-….m3u8         sync one batch specifically
  vibecheck sync --tags                   read genre tags instead of the export
  vibecheck clear --unknown               strip genre tags it has no record of
  vibecheck --root /Volumes/DJ/Music status
                                          work on a different collection

what it touches
  the ID3 genre tag of tracks it labels, and <collection>/.vibecheck/ .
  Nothing else. Track identity is a hash of the audio, so writing a label never
  changes what a track is.
"""


def main(argv=None) -> int:
    ap = argparse.ArgumentParser(
        prog="vibecheck",
        description="Learns how you label music, from the audio, and applies "
                    "it to the rest of your collection.",
        epilog=USAGE,
        formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--root", default=str(Path.home() / "Music" / "Collection"),
                    help="your music collection (default: %(default)s)")
    sub = ap.add_subparsers(dest="cmd", metavar="command")

    sub.add_parser("scan", help="find tracks and pick up label changes",
                   description="Walk the collection, index new or moved files, "
                               "and record any labels changed outside the app.")
    sub.add_parser("status", help="counts, labels, and what is embedded")

    p = sub.add_parser("label", help="label a batch of unlabelled tracks",
                       description="Pick unlabelled tracks, say what it can "
                                   "about each, write the tags and a playlist "
                                   "for you to correct.")
    p.add_argument("count", nargs="?", type=int, default=300,
                   help="how many tracks (default: %(default)s)")
    p.add_argument("--output", default=None,
                   help="playlist path (default: out/batch-<date>-<time>.m3u8)")
    p.add_argument("--seed", type=int, default=None,
                   help="fix the random selection, for reproducibility")
    p.add_argument("--dry-run", action="store_true",
                   help="predict and write the playlist, but change nothing")
    p.add_argument("--include-unconfirmed", action="store_true",
                   help="also pick tracks that already carry an unconfirmed "
                        "label from an earlier batch")
    p.add_argument("--rekordbox", metavar="XML",
                   default="~/Documents/rekordbox.xml",
                   help="rekordbox export to build the batch from "
                        "(default: %(default)s)")
    p.add_argument("--no-rekordbox", dest="rekordbox", action="store_const",
                   const=None, help="skip the rekordbox XML")
    p.add_argument("--in-place", action="store_true", default=True,
                   help="rewrite the rekordbox export itself, so a refresh in "
                        "rekordbox shows the batch (default)")
    p.add_argument("--separate-file", dest="in_place", action="store_false",
                   help="write a new xml instead of updating in place")
    p.add_argument("--write-tags", action="store_true", default=None,
                   help="also write the ID3 genre tag, showing how specific "
                        "each prediction was (default: debug.write_genre_tags "
                        "in config)")
    p.add_argument("--no-write-tags", dest="write_tags", action="store_false",
                   help="do not touch genre tags")

    p = sub.add_parser("sync", help="read your corrections back and retrain",
                       description="Read the genre tags of a batch, record "
                                   "what you changed, and confirm what you "
                                   "left alone.")
    p.add_argument("input", nargs="?",
                   help="playlist, to restrict which tracks are read")
    p.add_argument("--rekordbox", metavar="XML",
                   default="~/Documents/rekordbox.xml",
                   help="rekordbox export to read (default: %(default)s)")
    p.add_argument("--tags", action="store_true",
                   help="read ID3 genre tags instead of the rekordbox export")
    p.add_argument("--all", action="store_true",
                   help="read the whole collection, not just what is awaiting "
                        "correction")
    p.add_argument("--stale-ok", action="store_true",
                   help="read an export older than the current batch")

    p = sub.add_parser("discard", help="throw a batch away, free its tracks",
                       description="Forget a batch's unconfirmed labels and "
                                   "clear the tags it wrote. Tags you have "
                                   "since corrected are kept.")
    p.add_argument("input", nargs="?",
                   help="playlist (default: every unconfirmed label)")
    p.add_argument("--dry-run", action="store_true")

    p = sub.add_parser("clear", help="housekeeping: remove genre tags",
                       description="Remove genre tags by category. Saves what "
                                   "it removes to .vibecheck/cleared-<date>.csv "
                                   "first.")
    p.add_argument("input", nargs="?", help="restrict to a playlist")
    p.add_argument("--unknown", action="store_true",
                   help="tags this app has no record of")
    p.add_argument("--unconfirmed", action="store_true",
                   help="labels it wrote that you have not confirmed")
    p.add_argument("--confirmed", action="store_true",
                   help="your own labels (destroys work)")
    p.add_argument("--all", action="store_true", help="every genre tag")
    p.add_argument("--apply", action="store_true",
                   help="actually do it (otherwise only reports)")

    args = ap.parse_args(argv)
    if not args.cmd:
        ap.print_help()
        return 0
    root = Path(args.root).resolve()
    cfg = load_config(root)
    return {"scan": cmd_scan, "status": cmd_status, "label": cmd_label,
            "discard": cmd_discard, "clear": cmd_clear,
            "sync": cmd_sync}[args.cmd](root, cfg, args) or 0


if __name__ == "__main__":
    raise SystemExit(main())

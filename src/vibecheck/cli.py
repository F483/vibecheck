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

from . import embed, evaluate, index, palette, playlist, rekordbox, store, tags
from .config import DEFAULT
from .predict import Model, axes_for, targets_for
from .store import Label

BACKEND = "clapw"


def load_config(root: Path) -> dict:
    user = store.vibecheck_dir(root) / "config.toml"
    src = user if user.exists() else Path(__file__).with_name("default_config.toml")
    return tomllib.loads(src.read_text())


def build_axes(cfg: dict) -> list:
    """The three axes, with the two dials config actually owns."""
    mc = cfg["predict"]["misleading_cost"]
    return axes_for(mc, cfg["predict"].get("rating_misleading_cost", mc))


def specificity(pred) -> str:
    """How far the prediction got: a colour, one axis of it, or nothing."""
    if pred.colour:
        return "colour"
    for ax in ("hue", "tone"):
        if ax in pred.values:
            return ax
    return "none"


def tag_text(pred) -> str | None:
    """What the debug genre tag should say.

    The most specific thing the prediction supports -- "Pink", or "Vibrant",
    or "Dark". That is the tag's whole purpose: rekordbox's colour swatch can
    only show a colour, so a prediction that got as far as the hue or the tone
    is invisible without this.
    """
    if pred.colour:
        return pred.colour.name
    return pred.values.get("hue") or pred.values.get("tone")


def labelled(root: Path, cfg: dict) -> dict[str, Label]:
    """Labels the user stands behind -- the only thing worth training on."""
    return store.current(store.labels_db(root), source="user")


def spoken_for(root: Path) -> set[str]:
    """Tracks that already carry a label from anyone, including this app.

    A model-written label means the track is out in a batch the user is
    partway through correcting. Selecting it again would overwrite a correction
    that has not been synced yet -- silently, since the tag looks like
    something the app wrote in the first place.
    """
    return set(store.current(store.labels_db(root)))


def train(root: Path, cfg: dict, measure: bool = True):
    lab = labelled(root, cfg)
    if len(lab) < 20:
        sys.exit(f"only {len(lab)} labels; label some tracks first")
    paths, X = embed.load(root, BACKEND, DEFAULT, sorted(lab))
    y = targets_for([lab[p] for p in paths])
    axes = build_axes(cfg)
    m = Model(axes)
    info = m.train(X, y)
    # Measured before the model is used, on a slice it was not fitted on, so
    # the round has an honest number attached to it in the database. Skipping
    # this saves one fit per axis and loses the only record of the round.
    hb = store.hash_by_path(store.labels_db(root))
    parts = evaluate.split([hb[p] for p in paths])
    info["n_train"] = int((parts == evaluate.TRAIN).sum())
    info["labels"] = len({(l.colour, l.stars) for l in lab.values()})
    info["scores"] = (evaluate.score_axes(X, y, [hb[p] for p in paths], axes)
                      if measure else [])
    return m, info


def cmd_scan(root: Path, cfg: dict, args) -> None:
    print(f"scanning {root}", flush=True)
    st = index.scan(root, adopt_tags=args.adopt_tags)
    parts = [f"{st.files} mp3 files", f"{st.new} new"]
    for n, what in ((st.moved, "moved"), (st.changed, "changed"),
                    (st.missing, "now missing"), (st.returned, "back"),
                    (st.unreadable, "unreadable")):
        if n:
            parts.append(f"{n} {what}")
    print("   " + ", ".join(parts))
    if st.labels_added:
        print(f"   adopted {st.labels_added} genre tags as your labels")
    if st.tags_differ:
        print(f"   {st.tags_differ} genre tags were not read"
              + ("" if args.adopt_tags else
                 " (--adopt-tags to take them as your labels,"
                 " or sync to read corrections)"))


def cmd_status(root: Path, cfg: dict, args) -> None:
    from collections import Counter

    con = store.labels_db(root)
    total, gone = con.execute(
        "SELECT SUM(missing_at IS NULL), SUM(missing_at IS NOT NULL) "
        "FROM tracks").fetchone()
    total, gone = total or 0, gone or 0
    lab = labelled(root, cfg)
    cache = store.cache_db(root)
    have = {h for (h,) in cache.execute(
        "SELECT hash FROM embeddings WHERE backend=?", (BACKEND,))}
    hb = store.hash_by_path(con)

    print(f"collection      {total} tracks"
          + (f"   ({gone} no longer on disk)" if gone else ""))
    print(f"labelled        {len(lab)}")
    print(f"unlabelled      {total - len(lab)}")
    print(f"embedded        {sum(1 for p in hb if hb[p] in have)}")

    if lab:
        parts = evaluate.split([hb[p] for p in lab if p in hb])
        held = int((parts != evaluate.TRAIN).sum())
        print(f"holdout         {held} of {len(lab)} labels"
              + ("   (none yet: nothing can be measured)" if not held else ""))

    last = con.execute("""
        SELECT f.id, f.ts, f.n_train, r.name FROM fits f
        LEFT JOIN rounds r ON r.id = f.round_id ORDER BY f.id DESC LIMIT 1
    """).fetchone()
    if last:
        fid, ts, n, name = last
        left, hand = con.execute(
            "SELECT SUM(cost_left), SUM(cost) FROM fit_axes WHERE fit_id=?",
            (fid,)).fetchone()
        print(f"last measured   {left:.2f} of {hand:.2f} decisions left per "
              f"track   ({name or 'no round'}, {n} labels, "
              f"{dt.datetime.fromtimestamp(ts):%Y-%m-%d})")

    rounds = con.execute("SELECT name, size, n_labels, closed FROM rounds "
                         "ORDER BY id").fetchall()
    if rounds:
        print(f"rounds          {len(rounds)}")
        for name, size, n, closed in rounds[-5:]:
            print(f"   {name:22} {size:4} tracks, knew {n:5}"
                  + ("" if closed else "   (open)"))

    if lab:
        print("colours")
        for c in palette.COLOURS:
            n = sum(1 for v in lab.values() if v.colour == c.name)
            if n:
                print(f"   {c.name:8} {n:5}   {c.hue}/{c.tone}")
        print("stars")
        by_star = Counter(v.stars for v in lab.values() if v.stars is not None)
        for k in sorted(by_star, reverse=True):
            print(f"   {'*' * k or '-':8} {by_star[k]:5}")


def cmd_label(root: Path, cfg: dict, args) -> None:
    con = store.labels_db(root)
    ids = store.track_ids(con)
    confirmed = set(labelled(root, cfg))
    taken = confirmed if args.include_unconfirmed else spoken_for(root)
    live = {p for (p,) in con.execute(
        "SELECT path FROM tracks WHERE missing_at IS NULL")}
    pool = [p for p in sorted(live) if p not in taken]
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
    # One name for the round, generated once and passed everywhere: the
    # playlist file, the rekordbox playlist, the round row, the label rows and
    # the predictions all carry it. Two names generated at two points in the
    # run drift apart by however long the embedding takes.
    out = Path(args.output) if args.output else (
        Path("out") / f"vibecheck-{dt.datetime.now():%Y%m%d-%H%M}.m3u8")
    name = out.stem
    print(f"[1/4] selected {len(batch)} of {len(pool)} unlabelled tracks "
          f"as {name}")

    print(f"[2/4] listening to the tracks that are new to it "
          f"(about 3 seconds each)", flush=True)
    st = embed.embed_all(root, BACKEND, DEFAULT, batch, workers=1)
    print(f"      {st['done']} embedded, {st['cached']} already known"
          + (f", {st['failed']} failed" if st["failed"] else ""))

    print("[3/4] training on what you have labelled so far", flush=True)
    model, info = train(root, cfg)
    print(f"      {info['tracks']} tracks, {info['labels']} distinct labels, "
          f"axes: {', '.join(info['axes'])}")
    for sc in info["scores"]:
        print(f"      {sc.axis:7} speaks on {sc.coverage * 100:3.0f}% at "
              f"{sc.accuracy * 100:3.0f}%, leaving {sc.cost_left:.2f} of "
              f"{sc.cost:.2f} decisions   (holdout n={sc.n})")
    if not info["scores"]:
        print("      not scored: no labelled track falls in the measurement "
              "slice yet, so this round goes unrecorded")

    encoder = embed.fingerprint(BACKEND, DEFAULT)
    rid = None if args.dry_run else store.open_round(
        con, name, len(batch), BACKEND, encoder, info["tracks"])

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
    entries, counts = [], {}
    rated = 0
    for rel, p in zip(paths, preds):
        colour, stars = p.colour, p.stars
        label = Label(colour.name if colour else None, stars)
        counts[specificity(p)] = counts.get(specificity(p), 0) + 1
        rated += stars is not None
        if label and not args.dry_run:
            store.log_label(con, ids[rel], label, "model", round_id=rid)
        # debug only, and never the rating: that has a field of its own
        text = tag_text(p)
        if write_tags and not args.dry_run and tags.read(root / rel) != text:
            tags.write(root / rel, text)
        shown = f"{label.colour or '?'}{' ' + '*' * stars if stars else ''}"
        entries.append((rel, f"{shown:14} | {Path(rel).name}"))

    if not args.dry_run:
        # every axis, asserted or not: this is the record that makes the
        # threshold reviewable later, and it cannot be recovered afterwards
        store.log_predictions(con, rid, [(ids[rel], p)
                                         for rel, p in zip(paths, preds)])
        if info["scores"]:      # a fit with nothing measured would plot as zero
            store.log_fit(con, rid, BACKEND, encoder, evaluate.VAL,
                          info["n_train"], info["labels"], info["scores"])
    con.commit()

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
            [(rel, specificity(p),
              Label(p.colour.name if p.colour else None, p.stars))
             for rel, p in zip(paths, preds)], name=name)
        print(f"\nwrote {dst}")
        print(f"   {st['coloured']} colours set, {st['rated']} ratings set"
              + (f", {st['not_in_xml']} not found in the export"
                 if st["not_in_xml"] else ""))
        if st.get("backup"):
            print(f"   previous export kept at {Path(st['backup']).name}")
        if args.in_place:
            print(f"   refresh the rekordbox xml node in rekordbox's sidebar; "
                  f"the batch appears as the playlist {name}")
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
    print(f"   stars   {rated} (independent of the colour)")
    print("\ncorrect them in your DJ software, export it again, then:"
          "\n   vibecheck sync")


def cmd_discard(root: Path, cfg: dict, args) -> None:
    """Undo a batch: forget its unconfirmed labels and clear the tags it wrote.

    Only tags still holding exactly what the app wrote are cleared. A tag the
    user has since changed is a correction, not a discard, so it is kept and
    reported -- losing it would be indistinguishable from the app never having
    predicted the track.

    What this cannot undo is the colours and ratings already imported into
    rekordbox: that is rekordbox's database, not a file this app owns. The
    batch playlist is still there to select and clear.
    """
    con = store.labels_db(root)
    ids = store.track_ids(con)
    canon = store.path_index(con)
    confirmed = set(labelled(root, cfg))
    rows = {p: (c, r) for p, c, r in con.execute(
        "SELECT path, colour, round_id FROM current WHERE source = 'model'")}

    if args.input:
        rels = [canon.get(store.norm(r), r)
                for r in playlist.read(Path(args.input), root)]
    else:
        rels = list(rows)

    cleared = kept = 0
    for rel in rels:
        if rel in confirmed or rel not in rows:
            continue
        was, rid = rows[rel]
        now = tags.read(root / rel)
        if now is not None and now != was:
            kept += 1          # user has edited it: that is a correction
            continue
        if not args.dry_run:
            if now is not None:
                tags.write(root / rel, None)
            store.log_label(con, ids[rel], Label(None, None), "model",
                            round_id=rid)
        cleared += 1
    con.commit()
    print(f"discarded {cleared} unconfirmed labels"
          + (f"; kept {kept} you had already corrected (sync to keep them)"
             if kept else ""))
    if cleared and not args.dry_run:
        print("   colours and ratings already imported into rekordbox are "
              "rekordbox's;\n   select the batch playlist there to clear them")
    if args.dry_run:
        print("(dry run -- nothing changed)")


def cmd_migrate(root: Path, cfg: dict, args) -> None:
    """Bring labels.db up to the current schema, or say what that would take."""
    path = store.vibecheck_dir(root) / "labels.db"
    if not path.exists():
        print(f"no database at {path}; run scan to create one")
        return
    con = store._connect(path)
    have = con.execute("PRAGMA user_version").fetchone()[0]
    todo = store.migrate(con, path, dry_run=True)
    print(f"{path}")
    print(f"   at schema v{have}, this build is v{store.SCHEMA_VERSION}")
    if not todo:
        print("   up to date, nothing to do")
        return
    print(f"   {len(todo)} migration(s) to apply: {', '.join(todo)}")
    if not args.apply:
        print("\n(nothing changed -- pass --apply to migrate)")
        return
    done = store.migrate(con, path)
    con.commit()
    print(f"   applied {', '.join(done)}; now v{store.SCHEMA_VERSION}")
    print(f"   previous database kept at {path.with_suffix(f'.db.pre-v{have}').name}")


def cmd_clear(root: Path, cfg: dict, args) -> None:
    """Remove genre tags. Housekeeping, so it asks twice.

    The genre tag is debug output (debug.write_genre_tags): rekordbox's colour
    and rating are where a label actually lives. So clearing a tag says nothing
    about the label and must not touch it -- the categories below only choose
    *which* tracks' tags to strip.

    Anything cleared is first written to .vibecheck/cleared-<date>.csv, because
    a tag the database never knew about has no other record anywhere.
    """
    import csv
    import datetime as dt

    con = store.labels_db(root)
    ids = store.track_ids(con)
    canon = store.path_index(con)
    confirmed = set(labelled(root, cfg))
    known = set(store.current(con))

    scope = ([canon.get(store.norm(r), r)
              for r in playlist.read(Path(args.input), root)]
             if args.input else sorted(ids))

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
    print(f"cleared {len(targets)}; saved to {out}")
    print("   labels are untouched: colour and rating live in rekordbox")


def cmd_sync(root: Path, cfg: dict, args) -> None:
    """Read your corrections back.

    Two kinds of evidence, and they need different treatment:

    A value that **differs** from what the database holds is unambiguous --
    only you could have changed it -- so it is taken whatever the scope.

    A value that **matches** a prediction is ambiguous: you reviewed it and
    agreed, or you have not looked yet. Adopting those without evidence of
    review is what once imported 8,595 retired labels. The evidence is the open
    round: the database knows which tracks are out in a batch you have not
    finished, so no playlist has to be named for the common case.

    A colour on a track the database has no record of is the third case --
    another tool's work, or an older scheme. Ignored unless it is in the
    reviewed scope, or you ask for it with --all.
    """
    con = store.labels_db(root)
    ids = store.track_ids(con)
    last_user = store.current(con, source="user")
    last_any = store.current(con)
    canon = store.path_index(con)

    # Tracks out in a round you have not finished: the reviewed scope, unless
    # you name a playlist or say --all.
    reviewed = {p for (p,) in con.execute("""
        SELECT t.path FROM predictions p JOIN tracks t ON t.id = p.track_id
        JOIN rounds r ON r.id = p.round_id WHERE r.closed IS NULL""")}
    if args.input:
        reviewed = {canon.get(store.norm(x), x)
                    for x in playlist.read(Path(args.input), root)}

    if args.tags:
        rels = sorted(reviewed) if (reviewed and not args.all) else sorted(ids)
        # A genre tag is only a label if it names one of the eight. Anything
        # else is the debug tag showing a hue or a tone, or a real genre the
        # user set themselves -- neither is a colour, and adopting either
        # would invent a ninth.
        reading, foreign = {}, 0
        for rel in rels:
            tag = tags.read(root / rel)
            if tag and tag not in palette.BY_NAME:
                foreign += 1
                continue
            reading[rel] = Label(tag, None)
        if foreign:
            print(f"ignoring {foreign} genre tags that do not name a colour")
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
            "SELECT MAX(ts) FROM labels WHERE source='model'").fetchone()[0]
        if newest_batch and xml.stat().st_mtime < newest_batch and not args.stale_ok:
            import datetime as _dt
            sys.exit(
                f"{xml.name} was exported "
                f"{_dt.datetime.fromtimestamp(xml.stat().st_mtime):%Y-%m-%d %H:%M}"
                f", before the current batch was made "
                f"({_dt.datetime.fromtimestamp(newest_batch):%Y-%m-%d %H:%M}).\n"
                "  It cannot contain your corrections. Export a fresh one from\n"
                "  rekordbox first, or pass --stale-ok if you really mean it.")
        # The whole export, always: a correction to a track that is not in any
        # batch is still a correction, and scoping the read is how those got
        # missed.
        raw = rekordbox.read_labels(xml, root, only=None)
        reading = {canon.get(k, k): v for k, v in raw.items()}
        print(f"reading {xml.name}: {sum(1 for v in reading.values() if v)} "
              f"tracks carry a colour or rating")

    of_round = store.round_of(con)
    touched: set[int] = set()
    added = changed = confirmed = missing = unknown = 0
    for rel, now in reading.items():
        if rel not in ids:
            missing += 1
            continue
        prev = last_any.get(rel)
        in_scope = args.all or rel in reviewed
        if not now and prev is None:
            continue          # blank before, blank now: not an event
        if now == prev:
            if in_scope and rel not in last_user:
                # reviewed, and you left it alone: the model's guess is yours
                store.log_label(con, ids[rel], now, "user",
                                round_id=of_round.get(rel))
                touched.add(of_round.get(rel))
                confirmed += 1
            continue
        if prev is None and not in_scope:
            unknown += 1      # a colour this app has no record of
            continue
        # Only the axes the previous statement actually made can be
        # contradicted. Rekordbox reports an unrated track as zero stars, so a
        # round that asserted a colour and stayed silent on the rating comes
        # back looking different without the user having touched anything.
        contradicts = prev is not None and any(
            getattr(prev, f) is not None and getattr(now, f) != getattr(prev, f)
            for f in ("colour", "stars"))
        store.log_label(con, ids[rel], now, "user", round_id=of_round.get(rel))
        touched.add(of_round.get(rel))
        if prev is None:
            added += 1
        elif contradicts:
            changed += 1
        else:
            confirmed += 1
    for rid in touched - {None}:
        store.close_round(con, rid)
    con.commit()
    print(f"new labels {added}, corrections {changed}, confirmed {confirmed}"
          + (f", not in index {missing}" if missing else ""))
    if unknown:
        print(f"ignored {unknown} tracks carrying a colour this app has no "
              f"record of (--all to adopt them as yours)")
    print(f"labelled now: {len(labelled(root, cfg))}")


USAGE = """\
the loop
  vibecheck scan                     find your tracks (once, and after adding music)
  vibecheck label 300                pick 300 unlabelled tracks, label what it
                                     can, and write the colours and star
                                     ratings into the rekordbox xml
  ... refresh the rekordbox xml node in rekordbox, import the new playlist,
      correct what is wrong, then export the collection again ...
  vibecheck sync                     read your corrections back

  Each round it learns from your corrections, so each round you correct less.
  The model is refitted at the start of the next `label`, not by `sync`.
  It needs roughly 200 labels of your own before it is much use -- label a first
  batch by hand, or let it guess and correct everything.

examples
  vibecheck status                        counts, and how many labels you have
  vibecheck label 50 --dry-run            see what it would say, change nothing
  vibecheck label 300 --include-unconfirmed
                                          reuse tracks from a batch you have
                                          not corrected yet
  vibecheck discard out/vibecheck-….m3u8  throw a batch away, free its tracks
  vibecheck sync                          read your corrections back
  vibecheck sync out/vibecheck-….m3u8     name the batch you reviewed
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

    p = sub.add_parser("scan", help="find tracks, and notice what moved",
                       description="Walk the collection and index new, moved "
                                   "or missing files. Labels are not read from "
                                   "tags unless you ask.")
    p.add_argument("--adopt-tags", action="store_true",
                   help="also take existing genre tags as your own labels "
                        "(bootstrapping from another tool)")
    sub.add_parser("status", help="counts, labels, and what is embedded")

    p = sub.add_parser("label", help="label a batch of unlabelled tracks",
                       description="Pick unlabelled tracks, say what it can "
                                   "about each, write the tags and a playlist "
                                   "for you to correct.")
    p.add_argument("count", nargs="?", type=int, default=300,
                   help="how many tracks (default: %(default)s)")
    p.add_argument("--output", default=None,
                   help="playlist path (default: "
                        "out/vibecheck-<date>-<time>.m3u8). The stem names the "
                        "round everywhere: this file, the rekordbox playlist, "
                        "and the database.")
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
                   help="also write the prediction into the ID3 genre tag, for "
                        "a tags-only workflow (default: "
                        "debug.write_genre_tags in config, normally off)")
    p.add_argument("--no-write-tags", dest="write_tags", action="store_false",
                   help="do not touch genre tags")

    p = sub.add_parser("sync", help="read your corrections back and retrain",
                       description="Read the genre tags of a batch, record "
                                   "what you changed, and confirm what you "
                                   "left alone.")
    p.add_argument("input", nargs="?",
                   help="playlist naming the tracks you reviewed (default: "
                        "whatever is out in an unfinished round)")
    p.add_argument("--rekordbox", metavar="XML",
                   default="~/Documents/rekordbox.xml",
                   help="rekordbox export to read (default: %(default)s)")
    p.add_argument("--tags", action="store_true",
                   help="read ID3 genre tags instead of the rekordbox export")
    p.add_argument("--all", action="store_true",
                   help="treat every track in the export as reviewed, "
                        "including colours this app has no record of")
    p.add_argument("--stale-ok", action="store_true",
                   help="read an export older than the current batch")

    p = sub.add_parser("discard", help="throw a batch away, free its tracks",
                       description="Forget a batch's unconfirmed labels and "
                                   "clear the tags it wrote. Tags you have "
                                   "since corrected are kept.")
    p.add_argument("input", nargs="?",
                   help="playlist (default: every unconfirmed label)")
    p.add_argument("--dry-run", action="store_true")

    p = sub.add_parser("migrate", help="bring the database up to date",
                       description="Apply any pending schema migrations. The "
                                   "app does this on its own when it opens the "
                                   "database; this is for looking first.")
    p.add_argument("--apply", action="store_true",
                   help="actually migrate (otherwise only reports)")

    p = sub.add_parser("clear", help="housekeeping: remove genre tags",
                       description="Remove ID3 genre tags by category. This is "
                                   "debug output only -- your colours and star "
                                   "ratings live in rekordbox and are not "
                                   "touched. Saves what it removes to "
                                   ".vibecheck/cleared-<date>.csv first.")
    p.add_argument("input", nargs="?",
                   help="a playlist, to restrict which tracks are considered")
    p.add_argument("--unknown", action="store_true",
                   help="tags on tracks this app has no label for")
    p.add_argument("--unconfirmed", action="store_true",
                   help="tags on tracks whose label is still the model's")
    p.add_argument("--confirmed", action="store_true",
                   help="tags on tracks you have labelled yourself")
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
            "discard": cmd_discard, "clear": cmd_clear, "migrate": cmd_migrate,
            "sync": cmd_sync}[args.cmd](root, cfg, args) or 0


if __name__ == "__main__":
    raise SystemExit(main())

"""Commands.

    vibecheck scan              find tracks, notice what moved
    vibecheck status            where things stand
    vibecheck label [N]         pick N unlabelled tracks, label what it can,
                                add key, energy and cues, write a playlist for you
                                to correct
    vibecheck sync [playlist]   read your corrections back
    vibecheck discard [playlist]  throw a batch away and free its tracks
    vibecheck analyse ...       key and energy alone, for chosen tracks or all
    vibecheck cues ...          cue points alone, for chosen tracks; --train

The loop is: label -> correct in rekordbox -> export -> label again.
"""

from __future__ import annotations

import argparse
import datetime as dt
import random
import sys
import tomllib
from pathlib import Path

import numpy as np

from . import cues, embed, evaluate, index, keyenergy, palette, rekordbox, store
from . import playlist as playlist_mod
from .config import DEFAULT
from .predict import Model, axes_for, targets_for
from .store import Label

BACKEND = "clapw"


def load_config(root: Path) -> dict:
    """The defaults, with whatever <collection>/.vibecheck/config.toml sets on
    top, setting by setting -- so a config file only needs what it changes."""
    cfg = tomllib.loads(Path(__file__).with_name("default_config.toml").read_text())
    user = store.vibecheck_dir(root) / "config.toml"
    if user.exists():
        for k, v in tomllib.loads(user.read_text()).items():
            cfg[k] = {**cfg.get(k, {}), **v} if isinstance(v, dict) else v
    return cfg


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


def writes_key_energy(cfg: dict) -> bool:
    rk = cfg.get("rekordbox", {})
    return rk.get("write_tonality", True) or rk.get("write_comment", True)


def writes_cues(cfg: dict) -> bool:
    return bool(cfg.get("cues", {}).get("write", True))


def retrain_cues(root: Path, cfg: dict, marks: dict[str, cues.Marks],
                 force: bool = False) -> bool:
    """Learn cue placement again from your own cues, if there are enough new
    ones (or if forced). Returns whether it did."""
    c = cfg.get("cues", {})
    written = store.cues_written(store.labels_db(root))
    own = cues.yours(marks, written, list(c.get("lead", [0])))
    model = cues.Model.load(root)
    known = model.n_tracks if model else 0
    if not force and len(own) < known + int(c.get("retrain_after", 50)):
        return False
    print(f"      learning from your cues on {len(own)} tracks (the model in use "
          f"knew {known}); a few minutes per network", flush=True)
    F, st = cues.features_all(root, {r: marks[r] for r in own})
    data = []
    for r, f in F.items():
        T = [t for t in own[r] if t < len(f)]
        if len(T) >= 3:
            data.append((f, T))
    if len(data) < 30:
        print(f"      only {len(data)} usable tracks; keeping the model in use")
        return False
    model = cues.fit(data, source="yours")
    model.save(store.vibecheck_dir(root) / cues.MODEL_FILE)
    print(f"      saved, trained on {len(data)} tracks")
    return True


def plan_cues(root: Path, cfg: dict, rels: list[str],
              marks: dict[str, cues.Marks]) -> tuple[dict, dict]:
    """rel -> (POSITION_MARKs, transition bars) for tracks that may get cues:
    none yet, or only generated ones."""
    c = cfg.get("cues", {})
    ok = {r: marks[r] for r in rels
          if r in marks and marks[r].tempos and cues.writable(marks[r])}
    model = cues.Model.load(root)
    if model is None:
        sys.exit("no cue model: run `vibecheck cues --train` once, or set "
                 "write = false under [cues] in the config")
    F, st = cues.features_all(root, ok)
    st["skipped"] = sum(1 for r in rels if r in marks and r not in ok)
    st["model"] = f"{cues.version()}/{model.source}-{model.n_tracks}"
    out = {}
    for r, f in F.items():
        T = model.transitions(f)
        down = cues.downbeats(ok[r].tempos, ok[r].duration + 2.0)
        out[r] = (cues.place(T, down, list(c.get("lead", [0])),
                             dict(c.get("loops", {})), bool(c.get("memory", True))), T)
    return out, st


def log_cues(root: Path, plan: dict, model: str, round_id: int | None) -> None:
    con = store.labels_db(root)
    ids = store.track_ids(con)
    for rel, (marks, T) in plan.items():
        store.log_cue_write(con, ids[rel], round_id, model, T, cues.hot_starts(marks))
    con.commit()


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
    st = index.scan(root)
    parts = [f"{st.files} mp3 files", f"{st.new} new"]
    for n, what in ((st.moved, "moved"), (st.changed, "changed"),
                    (st.missing, "now missing"), (st.returned, "back"),
                    (st.unreadable, "unreadable")):
        if n:
            parts.append(f"{n} {what}")
    print("   " + ", ".join(parts))


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
    # Read corrections first, always. The model is fitted from whatever is in
    # the database at this moment, so labelling before syncing silently trains
    # on stale data and wastes the round. Doing it here rather than asking the
    # user to remember the order is the whole reason `sync` is rarely typed.
    syncing = bool(args.sync and not args.dry_run and args.rekordbox)
    with_ke = writes_key_energy(cfg)
    with_cues = writes_cues(cfg) and bool(args.rekordbox) and not args.dry_run

    steps, done = 4 + syncing + with_ke + with_cues, 0

    def step(msg: str) -> None:
        nonlocal done
        done += 1
        print(f"[{done}/{steps}] {msg}", flush=True)

    if syncing:
        step("reading your corrections")
        try:
            n = read_corrections(root, cfg, xml=args.rekordbox)
            report_corrections(n)
        except FileNotFoundError:
            print("      no rekordbox export found; skipping")
        except StaleExport as e:
            # Not fatal: a batch can legitimately be generated while an earlier
            # one is still being corrected. But say so loudly, because the
            # alternative reading is that the corrections were taken and were
            # not.
            print(f"      SKIPPED: {e}.")
            print("      Export a fresh xml from rekordbox and run again if "
                  "you meant to include corrections.")

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
    step(f"selected {len(batch)} of {len(pool)} unlabelled tracks as {name}")

    step("listening to the tracks that are new to it (about 3 seconds each)")
    st = embed.embed_all(root, BACKEND, DEFAULT, batch, workers=1)
    print(f"      {st['done']} embedded, {st['cached']} already known"
          + (f", {st['failed']} failed" if st["failed"] else ""))

    ke: dict = {}
    if with_ke:
        step("working out key and energy")
        ke, st = keyenergy.analyse_all(root, batch)
        print(f"      {st['done']} analysed, {st['cached']} already known"
              + (f", {st['failed']} failed" if st["failed"] else ""))

    plan: dict = {}
    if with_cues:
        step("placing cue points")
        src = Path(args.rekordbox).expanduser()
        if not src.exists():
            print("      no rekordbox export found; skipping")
        else:
            marks = rekordbox.read_marks(src, root)
            retrain_cues(root, cfg, marks)
            plan, cst = plan_cues(root, cfg, batch, marks)
            print(f"      {len(plan)} tracks get cues"
                  + (f", {cst['skipped']} keep the cues they have"
                     if cst["skipped"] else "")
                  + (f", {cst['failed']} failed" if cst["failed"] else ""))

    step("training on what you have labelled so far")
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

    step("deciding what it can say about each track")
    paths, X = embed.load(root, BACKEND, DEFAULT, batch)
    preds = model.predict(X)

    entries, counts = [], {}
    rated = 0
    for rel, p in zip(paths, preds):
        colour, stars = p.colour, p.stars
        label = Label(colour.name if colour else None, stars)
        counts[specificity(p)] = counts.get(specificity(p), 0) + 1
        rated += stars is not None
        if label and not args.dry_run:
            store.log_label(con, ids[rel], label, "model", round_id=rid)
        shown = f"{label.colour or '?'}{' ' + '*' * stars if stars else ''}"
        k = ke.get(rel)
        shown += f"  {k.key} - Energy {k.energy}" if k else ""
        entries.append((rel, f"{shown:30} | {Path(rel).name}"))

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
                     "xml format),\n  or pass --rekordbox <path>.")
        # in place: rekordbox only ever re-reads the file it is pointed at
        dst = src if args.in_place else out.with_suffix(".xml")
        rk = cfg.get("rekordbox", {})
        st = rekordbox.write(
            src, dst, root,
            [rekordbox.Entry(rel, specificity(p),
                             Label(p.colour.name if p.colour else None, p.stars),
                             ke[rel].key if rel in ke else None,
                             ke[rel].energy if rel in ke else None,
                             plan[rel][0] if rel in plan else None)
             for rel, p in zip(paths, preds)], name=name,
            tonality=rk.get("write_tonality", True),
            comment=rk.get("write_comment", True))
        print(f"\nwrote {dst}")
        if plan:
            log_cues(root, plan, cst["model"], rid)
        print(f"   {st['coloured']} colours set, {st['rated']} ratings set"
              + (f", {st['keyed']} keys, {st['commented']} energy comments"
                 if with_ke else "")
              + (f", cues on {st['cued']} tracks" if plan else "")
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

    risky = playlist_mod.write(out, root, entries)
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
    """Undo a batch: forget its unconfirmed labels, freeing its tracks.

    Only the database is touched. Corrections you have already made reach it
    through `sync`, so sync first if you want to keep them. What this cannot
    undo is what was already imported into rekordbox -- that is rekordbox's
    database, not a file this app owns. The batch playlist is still there to
    select and clear.
    """
    con = store.labels_db(root)
    ids = store.track_ids(con)
    canon = store.path_index(con)
    confirmed = set(labelled(root, cfg))
    rows = dict(con.execute(
        "SELECT path, round_id FROM current WHERE source = 'model'"))

    if args.input:
        rels = [canon.get(store.norm(r), r)
                for r in playlist_mod.read(Path(args.input), root)]
    else:
        rels = list(rows)

    cleared = 0
    for rel in rels:
        if rel in confirmed or rel not in rows:
            continue
        if not args.dry_run:
            store.log_label(con, ids[rel], Label(None, None), "model",
                            round_id=rows[rel])
        cleared += 1
    con.commit()
    print(f"discarded {cleared} unconfirmed labels")
    if cleared and not args.dry_run:
        print("   what was already imported into rekordbox is rekordbox's;\n"
              "   select the batch playlist there to clear it")
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


def cmd_analyse(root: Path, cfg: dict, args) -> None:
    """Key and energy outside a labelling round: some tracks, or all of them.

    For testing, and for switching a whole collection over at once. Writes the
    same two fields a round does, and a playlist of only the tracks it changed,
    so importing that playlist is the whole of the change and nothing else.
    """
    from collections import Counter

    con = store.labels_db(root)
    canon = store.path_index(con)
    live = sorted(p for (p,) in con.execute(
        "SELECT path FROM tracks WHERE missing_at IS NULL"))
    if args.input:
        want = {canon.get(store.norm(r), r)
                for r in playlist_mod.read(Path(args.input), root)}
        rels = [p for p in live if p in want]
    elif args.all:
        rels = live
    else:
        sys.exit("choose the tracks: a playlist, or --all")

    print(f"key and energy for {len(rels)} tracks", flush=True)
    ke, st = keyenergy.analyse_all(root, rels)
    print(f"   {st['done']} analysed, {st['cached']} already known"
          + (f", {st['failed']} failed" if st["failed"] else ""))
    energy = Counter(r.energy for r in ke.values())
    print("   energy  " + "  ".join(f"{e}:{energy[e]}" for e in range(1, 11)
                                     if energy[e]))
    keys = Counter(r.key for r in ke.values())
    print("   keys    " + "  ".join(f"{k}:{n}" for k, n in keys.most_common(8))
          + ("  ..." if len(keys) > 8 else ""))

    if not args.rekordbox:
        return
    src = Path(args.rekordbox).expanduser()
    if not src.exists():
        sys.exit(f"no rekordbox export at {src}")
    exported = dt.datetime.fromtimestamp(src.stat().st_mtime)
    rk = cfg.get("rekordbox", {})
    name = f"vibecheck-analyse-{dt.datetime.now():%Y%m%d-%H%M}"
    entries = [rekordbox.Entry(rel, key=r.key, energy=r.energy)
               for rel, r in ke.items()]

    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        dst = Path(tmp) / "dry.xml" if args.dry_run else src
        st = rekordbox.write(src, dst, root, entries, name=name,
                             tonality=rk.get("write_tonality", True),
                             comment=rk.get("write_comment", True),
                             changed_only=True)
    changed = sum(st["playlists"].values())
    verb = "would change" if args.dry_run else "changed"
    print(f"\n{verb} {changed} tracks ({st['keyed']} keys, {st['commented']} "
          f"comments); {st['unchanged']} already up to date"
          + (f", {st['not_in_xml']} not in the export"
             if st["not_in_xml"] else ""))
    if args.dry_run:
        print("(dry run -- nothing written)")
    elif changed:
        print(f"wrote {src}, previous kept as {src.name}.bak\n"
              f"   refresh the rekordbox xml node, then import the playlist "
              f"{name}.\n   Importing applies every field this export holds "
              f"for those tracks,\n   so it should be fresh "
              f"(this one was exported {exported:%Y-%m-%d %H:%M}).")
    else:
        print("nothing to change; the export was left alone")


def cmd_cues(root: Path, cfg: dict, args) -> None:
    """Cue points outside a labelling round, and learning them again.

    Writes only on tracks with no cues or generated ones, and a playlist of
    only the tracks it changed, so importing that playlist is the whole change.
    """
    src = Path(args.rekordbox).expanduser()
    if not src.exists():
        sys.exit(f"no rekordbox export at {src}\n"
                 "  export one from rekordbox (File > Export Collection in xml "
                 "format).")
    print(f"reading {src}", flush=True)
    marks = rekordbox.read_marks(src, root)
    if args.train:
        retrain_cues(root, cfg, marks, force=True)
        return

    con = store.labels_db(root)
    canon = store.path_index(con)
    live = sorted(p for (p,) in con.execute(
        "SELECT path FROM tracks WHERE missing_at IS NULL"))
    if args.input:
        want = {canon.get(store.norm(r), r)
                for r in playlist_mod.read(Path(args.input), root)}
        rels = [p for p in live if p in want]
    elif args.all:
        rels = live
    else:
        # every track this app has labelled: the ones you are working through
        have = set(store.current(con))
        rels = [p for p in live if p in have]

    print(f"cue points for {len(rels)} tracks", flush=True)
    plan, st = plan_cues(root, cfg, rels, marks)
    print(f"   {len(plan)} placed, {st['cached']} from the cache"
          + (f", {st['skipped']} keep the cues they have" if st["skipped"] else "")
          + (f", {st['failed']} failed" if st["failed"] else ""))
    if not plan:
        print("nothing to change; the export was left alone")
        return
    exported = dt.datetime.fromtimestamp(src.stat().st_mtime)
    name = f"vibecheck-cues-{dt.datetime.now():%Y%m%d-%H%M}"
    entries = [rekordbox.Entry(rel, cues=m) for rel, (m, _) in plan.items()]

    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        dst = Path(tmp) / "dry.xml" if args.dry_run else src
        wst = rekordbox.write(src, dst, root, entries, name=name, changed_only=True)
    if args.dry_run:
        print(f"would set cues on {wst['cued']} tracks\n(dry run -- nothing written)")
        return
    log_cues(root, plan, st["model"], None)
    print(f"\ncues set on {wst['cued']} tracks; wrote {src}, previous kept as "
          f"{src.name}.bak\n   refresh the rekordbox xml node, then import the "
          f"playlist {name}.\n   Importing applies every field this export holds "
          f"for those tracks,\n   so it should be fresh "
          f"(this one was exported {exported:%Y-%m-%d %H:%M}).")


class StaleExport(Exception):
    """The rekordbox export predates the batch, so it cannot hold corrections."""


def read_corrections(root: Path, cfg: dict, *, xml: str,
                     playlist: str | None = None,
                     adopt_all: bool = False, stale_ok: bool = False) -> dict:
    """Read what the user changed, and record it. Returns counts.

    Two kinds of evidence, and they need different treatment:

    A value that **differs** from what the database holds is unambiguous --
    only the user could have changed it -- so it is taken whatever the scope.

    A value that **matches** a prediction is ambiguous: the user reviewed it
    and agreed, or has not looked yet. Adopting those without evidence of
    review is what once imported 8,595 retired labels. The evidence is the open
    round: the database knows which tracks are out in a batch that has not been
    synced, so no playlist has to be named for the common case.

    A colour on a track the database has no record of is the third case --
    another tool's work, or an older scheme. Ignored unless it is in the
    reviewed scope, or `adopt_all` is set.
    """
    con = store.labels_db(root)
    ids = store.track_ids(con)
    last_user = store.current(con, source="user")
    last_any = store.current(con)
    canon = store.path_index(con)

    # Tracks out in a round that has not been read back: the reviewed scope,
    # unless a playlist is named or adopt_all is set.
    reviewed = {p for (p,) in con.execute("""
        SELECT t.path FROM predictions p JOIN tracks t ON t.id = p.track_id
        JOIN rounds r ON r.id = p.round_id WHERE r.closed IS NULL""")}
    if playlist:
        reviewed = {canon.get(store.norm(x), x)
                    for x in playlist_mod.read(Path(playlist), root)}

    path = Path(xml).expanduser()
    if not path.exists():
        raise FileNotFoundError(path)
    # An export older than the batch cannot contain the corrections, and
    # reading it would adopt whatever the tracks looked like *before* the
    # batch was made -- silently, as though the user had confirmed it.
    newest = con.execute(
        "SELECT MAX(ts) FROM labels WHERE source='model'").fetchone()[0]
    if newest and path.stat().st_mtime < newest and not stale_ok:
        raise StaleExport(
            f"{path.name} was exported "
            f"{dt.datetime.fromtimestamp(path.stat().st_mtime):%Y-%m-%d %H:%M}"
            f", before the current batch was made "
            f"({dt.datetime.fromtimestamp(newest):%Y-%m-%d %H:%M})")
    # The whole export, always: a correction to a track outside the current
    # batch is still a correction, and scoping the read is how those got
    # missed.
    raw = rekordbox.read_labels(path, root, only=None)
    reading = {canon.get(k, k): v for k, v in raw.items()}
    source_name = path.name

    of_round = store.round_of(con)
    touched: set[int] = set()
    n = {"added": 0, "changed": 0, "confirmed": 0, "missing": 0, "unknown": 0,
         "read": sum(1 for v in reading.values() if v), "source": source_name}
    for rel, now in reading.items():
        if rel not in ids:
            n["missing"] += 1
            continue
        prev = last_any.get(rel)
        in_scope = adopt_all or rel in reviewed
        if not now and prev is None:
            continue          # blank before, blank now: not an event
        if now == prev:
            if in_scope and rel not in last_user:
                # reviewed, and left alone: the model's guess is now theirs
                store.log_label(con, ids[rel], now, "user",
                                round_id=of_round.get(rel))
                touched.add(of_round.get(rel))
                n["confirmed"] += 1
            continue
        if prev is None and not in_scope:
            n["unknown"] += 1      # a colour this app has no record of
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
            n["added"] += 1
        elif contradicts:
            n["changed"] += 1
        else:
            n["confirmed"] += 1
    for rid in touched - {None}:
        store.close_round(con, rid)
    con.commit()
    return n


def report_corrections(n: dict) -> None:
    print(f"      {n['added']} new, {n['changed']} corrected, "
          f"{n['confirmed']} confirmed"
          + (f", {n['missing']} not in the collection" if n["missing"] else ""))
    if n["unknown"]:
        print(f"      ignored {n['unknown']} tracks carrying a colour this app "
              f"has no record of (sync --all to adopt them)")


def cmd_sync(root: Path, cfg: dict, args) -> None:
    try:
        n = read_corrections(root, cfg, xml=args.rekordbox,
                             playlist=args.input, adopt_all=args.all,
                             stale_ok=args.stale_ok)
    except FileNotFoundError as e:
        sys.exit(f"no rekordbox export at {e}\n"
                 "  export one after correcting (File > Export Collection in "
                 "xml format).")
    except StaleExport as e:
        sys.exit(f"{e}.\n  It cannot contain your corrections. Export a fresh "
                 f"one from\n  rekordbox first, or pass --stale-ok if you "
                 f"really mean it.")
    print(f"read {n['source']}: {n['read']} tracks carry a colour or rating")
    report_corrections(n)
    print(f"labelled now: {len(labelled(root, cfg))}")


USAGE = """\
the loop
  vibecheck scan                     find your tracks (once, and after adding music)
  vibecheck label 100                read your corrections, learn from them,
                                     then label 100 more and write the colours,
                                     star ratings, key, energy and cue points
                                     into the rekordbox xml
  ... refresh the rekordbox xml node in rekordbox, import the new playlist,
      correct what is wrong while you listen, export the collection again,
      and run the same command again ...

  That is the whole loop: one command, repeated. Each round it learns from
  everything you have corrected so far, so each round you correct less.

  `vibecheck sync` does only the reading part, for when you have relabelled
  something and do not want a new batch yet.
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
  vibecheck analyse --all --dry-run       key and energy for everything: what
                                          would change
  vibecheck analyse out/vibecheck-….m3u8  key and energy for one playlist
  vibecheck cues                          cue points for every track it has
                                          labelled that has none of yours
  vibecheck cues --train                  learn cue placement from your cues now
  vibecheck --root /Volumes/DJ/Music status
                                          work on a different collection

what it touches
  the rekordbox xml you point it at, and <collection>/.vibecheck/ .
  Never your music files. Nothing changes in rekordbox until you import.
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

    sub.add_parser("scan", help="find tracks, and notice what moved",
                   description="Walk the collection and index new, moved or "
                               "missing files.")
    sub.add_parser("status", help="counts, labels, and what is embedded")

    p = sub.add_parser("label", help="label a batch of unlabelled tracks",
                       description="Pick unlabelled tracks, say what it can "
                                   "about each, add key, energy and cue points, "
                                   "and write "
                                   "them and a playlist into the rekordbox xml "
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
    p.add_argument("--no-sync", dest="sync", action="store_false", default=True,
                   help="do not read your corrections first (they will not be "
                        "learned from this round)")

    p = sub.add_parser("sync", help="read your corrections back and retrain",
                       description="Read the rekordbox export, record what "
                                   "you changed, and confirm what you left "
                                   "alone in the batch you reviewed.")
    p.add_argument("input", nargs="?",
                   help="playlist naming the tracks you reviewed (default: "
                        "whatever is out in an unfinished round)")
    p.add_argument("--rekordbox", metavar="XML",
                   default="~/Documents/rekordbox.xml",
                   help="rekordbox export to read (default: %(default)s)")
    p.add_argument("--all", action="store_true",
                   help="treat every track in the export as reviewed, "
                        "including colours this app has no record of")
    p.add_argument("--stale-ok", action="store_true",
                   help="read an export older than the current batch")

    p = sub.add_parser("discard", help="throw a batch away, free its tracks",
                       description="Forget a batch's unconfirmed labels. Sync "
                                   "first to keep corrections you have made.")
    p.add_argument("input", nargs="?",
                   help="playlist (default: every unconfirmed label)")
    p.add_argument("--dry-run", action="store_true")

    p = sub.add_parser("migrate", help="bring the database up to date",
                       description="Apply any pending schema migrations. The "
                                   "app does this on its own when it opens the "
                                   "database; this is for looking first.")
    p.add_argument("--apply", action="store_true",
                   help="actually migrate (otherwise only reports)")

    p = sub.add_parser("analyse", help="key and energy alone, for chosen tracks",
                       description="Work out key and energy and write them "
                                   "into the rekordbox xml, with a playlist of "
                                   "only the tracks that changed. `label` "
                                   "already does this for every batch; this is "
                                   "for testing, and for switching a whole "
                                   "collection over at once.")
    p.add_argument("input", nargs="?", help="playlist of tracks (m3u8)")
    p.add_argument("--all", action="store_true",
                   help="every track in the collection")
    p.add_argument("--dry-run", action="store_true",
                   help="work it out and report, write nothing")
    p.add_argument("--rekordbox", metavar="XML",
                   default="~/Documents/rekordbox.xml",
                   help="rekordbox export to write into (default: %(default)s)")
    p.add_argument("--no-rekordbox", dest="rekordbox", action="store_const",
                   const=None, help="only work it out and report")

    p = sub.add_parser("cues", help="cue points alone, for chosen tracks",
                       description="Place cue points and write them into the "
                                   "rekordbox xml, with a playlist of only the "
                                   "tracks that changed. Only tracks with no "
                                   "cues, or generated ones, are touched. "
                                   "`label` already does this for every batch.")
    p.add_argument("input", nargs="?", help="playlist of tracks (m3u8) "
                   "(default: every track this app has labelled)")
    p.add_argument("--all", action="store_true",
                   help="every track in the collection")
    p.add_argument("--train", action="store_true",
                   help="learn cue placement from your own cues now, and stop")
    p.add_argument("--dry-run", action="store_true",
                   help="work it out and report, write nothing")
    p.add_argument("--rekordbox", metavar="XML",
                   default="~/Documents/rekordbox.xml",
                   help="rekordbox export to read and write (default: %(default)s)")

    args = ap.parse_args(argv)
    if not args.cmd:
        ap.print_help()
        return 0
    root = Path(args.root).resolve()
    cfg = load_config(root)
    return {"scan": cmd_scan, "status": cmd_status, "label": cmd_label,
            "discard": cmd_discard, "analyse": cmd_analyse, "cues": cmd_cues,
            "migrate": cmd_migrate, "sync": cmd_sync}[args.cmd](root, cfg, args) or 0


if __name__ == "__main__":
    raise SystemExit(main())

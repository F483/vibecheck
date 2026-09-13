"""Three commands.

    vibecheck status            where things stand
    vibecheck label [N]         pick N unlabelled tracks, label what it can,
                                write a playlist for you to correct
    vibecheck sync [playlist]   read your corrections back and retrain

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

from . import embed, playlist, store, tags
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
    return store.current_labels(store.labels_db(root), source="user")


def train(root: Path, cfg: dict):
    lab = labelled(root, cfg)
    if len(lab) < 20:
        sys.exit(f"only {len(lab)} labels; label some tracks first")
    paths, X = embed.load(root, BACKEND, DEFAULT, sorted(lab))
    y = np.array([lab[p] for p in paths])
    m = Model(build_levels(cfg, sorted(set(y))))
    info = m.train(X, y)
    return m, info


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
    lab = labelled(root, cfg)
    hb = dict(con.execute("SELECT path, hash FROM tracks"))
    pool = [p for p in sorted(hb) if p not in lab]
    if not pool:
        sys.exit("nothing left unlabelled")
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
    playlist.write(out, root, entries)
    print(f"\nwrote {out}")
    for k, v in sorted(counts.items(), key=lambda kv: -kv[1]):
        print(f"   {k:7} {v}")
    print("\ncorrect them in your DJ software, then: vibecheck sync " + str(out))


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
    rels = playlist.read(Path(src), root) if src else sorted(hb)
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
    sub.add_parser("status")
    p = sub.add_parser("label")
    p.add_argument("count", nargs="?", type=int, default=300)
    p.add_argument("--output", default=None,
                   help="default: batch-<date>-<time>.m3u8")
    p.add_argument("--seed", type=int, default=None)
    p.add_argument("--dry-run", action="store_true")
    p = sub.add_parser("sync")
    p.add_argument("input", nargs="?")
    args = ap.parse_args(argv)
    root = Path(args.root).resolve()
    cfg = load_config(root)
    return {"status": cmd_status, "label": cmd_label, "sync": cmd_sync}[args.cmd](
        root, cfg, args) or 0


if __name__ == "__main__":
    raise SystemExit(main())

#!/usr/bin/env bash
# Embed a track set in bounded chunks, one process per chunk.
#
# One process per chunk because a transformer backend on a 16 GB machine gets
# OOM-killed sooner or later. A kill then costs at most one chunk: the cache is
# content-addressed, so the next process resumes where the last one stopped.
#
# usage: embed_run.sh <backend> [subset|labelled|all] [chunk] [rounds]
set -u
backend="${1:?backend}"
which="${2:-labelled}"
chunk="${3:-300}"
rounds="${4:-60}"

for i in $(seq "$rounds"); do
  out=$(uv run python -c "
import sys; sys.path.insert(0,'scripts')
from pathlib import Path
from vibecheck import embed, store
from vibecheck.config import DEFAULT
root = Path.home()/'Music'/'Collection'
which = '$which'
if which == 'subset':
    from subset import subset
    paths = subset(root)
elif which == 'labelled':
    paths = sorted(store.current_labels(store.labels_db(root), source='user'))
else:
    paths = sorted(p for (p,) in store.labels_db(root).execute('SELECT path FROM tracks'))
st = embed.embed_all(root, '$backend', DEFAULT, paths,
                     workers=1, progress_every=100, limit=$chunk)
print('CHUNK done', st['done'], 'failed', st['failed'],
      'remaining', st['remaining'] - st['done'])
" 2>&1 | grep -E "^(CHUNK|  [0-9])")
  echo "[$(date +%H:%M:%S)] round $i"
  echo "$out"
  echo "$out" | grep -q "remaining 0" && { echo "ALL DONE"; exit 0; }
  echo "$out" | grep -q "^CHUNK" || echo "  (chunk produced no result - process died, continuing)"
done
echo "ROUNDS EXHAUSTED"

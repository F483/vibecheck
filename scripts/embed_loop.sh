#!/usr/bin/env bash
# Embed in bounded chunks, one process per chunk.
#
# A transformer backend on a 16 GB machine gets OOM-killed sooner or later.
# One process per chunk means a kill costs at most one chunk: the cache is
# content-addressed, so the next process simply resumes.
set -u
backend="${1:?backend}"
chunk="${2:-300}"
rounds="${3:-40}"
for i in $(seq "$rounds"); do
  out=$(uv run python -c "
import sys; sys.path.insert(0,'scripts')
from pathlib import Path
from subset import subset
from vibecheck import embed
from vibecheck.config import DEFAULT
root = Path.home()/'Music'/'Collection'
st = embed.embed_all(root, '$backend', DEFAULT, subset(root),
                     workers=1, progress_every=100, limit=$chunk)
print('CHUNK', st['done'], 'remaining', st['remaining'] - st['done'])
" 2>&1 | grep -E "^(CHUNK|  [0-9])")
  echo "$out"
  echo "$out" | grep -q "remaining 0" && { echo "ALL DONE"; break; }
done

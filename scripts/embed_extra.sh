#!/usr/bin/env bash
# Embed the colour-tagged-but-unrated tracks imported from rekordbox.
set -u
for i in $(seq 20); do
  out=$(uv run python -c "
from pathlib import Path
from vibecheck import embed
from vibecheck.config import DEFAULT
paths=[l for l in Path('out/extra_colour_tracks.txt').read_text().splitlines() if l]
st=embed.embed_all(Path.home()/'Music'/'Collection','clap',DEFAULT,paths,
                   workers=1,progress_every=100,limit=200)
print('CHUNK done',st['done'],'remaining',st['remaining']-st['done'])
" 2>&1 | grep -E "^(CHUNK|  [0-9])")
  echo "[$(date +%H:%M:%S)] $out"
  echo "$out" | grep -q "remaining 0" && { echo "ALL DONE"; exit 0; }
done

#!/usr/bin/env bash
# MuQ runs in its own venv (transformers 4.x). Chunked like the others so an
# OOM kill costs one chunk rather than the run.
set -u
chunk="${1:-300}"; rounds="${2:-40}"; which="${3:-labelled}"
for i in $(seq "$rounds"); do
  out=$(VIRTUAL_ENV=.venv-muq uv run --no-project python scripts/embed_muq.py "$chunk" "$which" 2>&1 \
        | grep -E "^(CHUNK|  [0-9]|  failed)")
  echo "[$(date +%H:%M:%S)] round $i"; echo "$out"
  echo "$out" | grep -q "remaining 0" && { echo "ALL DONE"; exit 0; }
done
echo "ROUNDS EXHAUSTED"

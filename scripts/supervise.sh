#!/usr/bin/env bash
# Watchdog for the overnight run.
#
# Both silent-hang failures we hit (DataLoader workers and ProcessPoolExecutor)
# looked identical from outside: process alive, 0% CPU, no output, no error.
# So the health check is "has the log advanced recently", not "is it alive".
set -u
LOG=out/finetune.log
STALL=1800          # no log output for 30 min = stalled
MAX_RESTARTS=3
restarts=0

log() { echo "[$(date '+%m-%d %H:%M:%S')] $*" >> out/supervise.log; }
log "supervisor started"

while true; do
  sleep 300
  if ! pgrep -f finetune_clap >/dev/null; then
    log "fine-tuning process gone"
    if grep -q "BEST val accuracy" "$LOG" 2>/dev/null; then
      log "completed normally"
    else
      log "exited without completing -- see $LOG"
    fi
    break
  fi
  age=$(( $(date +%s) - $(stat -f %m "$LOG") ))
  if [ "$age" -gt "$STALL" ]; then
    if [ "$restarts" -ge "$MAX_RESTARTS" ]; then
      log "stalled again after $restarts restarts; giving up, leaving it alone"
      break
    fi
    restarts=$((restarts + 1))
    log "no output for ${age}s -- restart $restarts/$MAX_RESTARTS"
    pkill -f finetune_clap; sleep 10
    nohup caffeinate -i uv run python scripts/finetune_clap.py \
      --epochs 6 --batch-tracks 4 >> "$LOG" 2>&1 &
  fi
done

# whatever happened above, use the freed GPU for the queued work
log "starting extra-colour embed"
caffeinate -i bash scripts/embed_extra.sh >> out/extra_embed.log 2>&1
log "extra-colour embed finished"

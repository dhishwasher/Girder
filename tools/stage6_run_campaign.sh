#!/usr/bin/env bash
# Stage 6 serial campaign driver. Pins come from docs/competitor-benchmark/stage6-freeze.json.
# One campaign at a time; memory is checked before each; a failed or blocked campaign is logged and the
# run continues (never retried with weaker limits).
set -u
cd "$(dirname "$0")/.."
ROOT=/mnt/chromeos/removable/MOVESPEED/girder-competitor-benchmark-v1
A=$ROOT/acquisition
RUN=$ROOT/runs/stage6-run1
WORK=$ROOT/work/stage6-run1
LOG=$RUN/matrix-log.jsonl
mkdir -p "$RUN" "$WORK"
CBM=$(mktemp -d /tmp/cbm-stage6-XXXXXX); cp "$A/codebase-memory-mcp-0.10.8/codebase-memory-mcp" "$CBM/" && chmod +x "$CBM/codebase-memory-mcp"
bin() { case "$1" in
  girder|girder-watch) echo "$A/girder-0.4.0/girder";;
  ripwire) echo "$A/ripwire-0.5.0/ripwire-0.5.0-linux-x64/ripwire";;
  codebase-memory-mcp) echo "$CBM/codebase-memory-mcp";;
  code-review-graph) echo "${CRG_BIN:-}";; esac; }
avail() { awk '/MemAvailable/{print $2*1024}' /proc/meminfo; }
for product in girder girder-watch ripwire codebase-memory-mcp code-review-graph; do
  for fixture in modest-rust modest-python modest-typescript-tsx modest-go; do
    b=$(bin "$product")
    if [ -z "$b" ] || [ ! -x "$b" ]; then
      echo "{\"event\":\"skipped\",\"product\":\"$product\",\"fixture\":\"$fixture\",\"reason\":\"binary unavailable\",\"at\":\"$(date -u +%FT%TZ)\"}" >> "$LOG"; continue; fi
    tries=0; while [ "$(avail)" -lt 805306368 ] && [ $tries -lt 30 ]; do sleep 10; tries=$((tries+1)); done
    echo "{\"event\":\"start\",\"product\":\"$product\",\"fixture\":\"$fixture\",\"avail\":$(avail),\"at\":\"$(date -u +%FT%TZ)\"}" >> "$LOG"
    PYTHONPATH=. timeout 1900 python3 -m tools.competitor_benchmark.campaign --product "$product" --fixture "$fixture" \
      --binary "$b" --work-root "$WORK/$product-$fixture" --output "$RUN/$product-$fixture" > "$RUN/$product-$fixture.stdout" 2>&1
    rc=$?
    echo "{\"event\":\"end\",\"product\":\"$product\",\"fixture\":\"$fixture\",\"exit\":$rc,\"at\":\"$(date -u +%FT%TZ)\"}" >> "$LOG"
    sleep 5
  done
done
echo "{\"event\":\"done\",\"at\":\"$(date -u +%FT%TZ)\"}" >> "$LOG"

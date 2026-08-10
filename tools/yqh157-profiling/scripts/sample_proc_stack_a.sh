#!/bin/bash
# Sample holod RSS/CPU inside each container via crun.
# Env: YQH157_LAB (required-ish), EXPCTL_STATE_ROOT (default: $YQH157_LAB/state)
set -euo pipefail
LAB="${YQH157_LAB:-${YQH157_WD:-/home/cnic/work/yqh157-real-profiling}}"
STATE_ROOT="${EXPCTL_STATE_ROOT:-$LAB/state}"
OUT="${YQH157_OUT:-$LAB/evidence/proc/stack_a}"
PREFIX="${YQH157_PREFIX:-yqh157}"
META="${YQH157_TOPO_META:-$LAB/generated/topology-meta.json}"
mkdir -p "$OUT"
# sample holod inside each container via crun
CRUN=$(sudo find "$STATE_ROOT" -type d -name crun-state 2>/dev/null | head -1)
echo "CRUN=$CRUN LAB=$LAB" | tee "$OUT/crun_root.txt"
TS=$(date -u +%Y-%m-%dT%H:%M:%SZ)
echo "ts=$TS" > "$OUT/sample_meta.txt"
: > "$OUT/raw_ps_all.txt"
: > "$OUT/summary.csv"
echo "node,pid,rss_kb,pcpu,vmrss,vmhwm,vmsize" > "$OUT/summary.csv"

if [ -z "${CRUN:-}" ]; then
  echo "no crun root under $STATE_ROOT" | tee -a "$OUT/sample_meta.txt"
  exit 1
fi

if [ ! -f "$META" ]; then
  echo "missing topology meta: $META" | tee -a "$OUT/sample_meta.txt"
  exit 1
fi

for name in $(python3 -c "import json;print(\" \".join(json.load(open(\"$META\"))[\"nodes\"]))"); do
  cname="${PREFIX}-${name}"
  st=$(sudo crun --root "$CRUN" state "$cname" 2>/dev/null || true)
  if [ -z "$st" ]; then
    echo "$name,MISSING,,,,,,," >> "$OUT/summary.csv"
    continue
  fi
  pid=$(echo "$st" | python3 -c "import sys,json;print(json.load(sys.stdin).get(\"pid\",\"\"))")
  holod_pid=$(sudo nsenter -t "$pid" -m -p pgrep -x holod 2>/dev/null | head -1 || true)
  if [ -z "$holod_pid" ]; then
    holod_pid=$(pgrep -P "$pid" -x holod 2>/dev/null | head -1 || true)
  fi
  if [ -z "$holod_pid" ]; then
    echo "$name,$pid,NO_HOLOD,,,,," >> "$OUT/summary.csv"
    continue
  fi
  psline=$(sudo nsenter -t "$pid" -m -p ps -o pid,rss,pcpu,comm -C holod 2>/dev/null | tail -n +2 | head -1 || true)
  echo "$name $psline" >> "$OUT/raw_ps_all.txt"
  rss=$(echo "$psline" | awk "{print \$2}")
  pcpu=$(echo "$psline" | awk "{print \$3}")
  ipid=$(echo "$psline" | awk "{print \$1}")
  status=$(sudo nsenter -t "$pid" -m -p cat /proc/${ipid}/status 2>/dev/null || true)
  echo "$status" > "$OUT/${name}_proc_status.txt"
  vmrss=$(echo "$status" | awk "/^VmRSS:/{print \$2}")
  vmhwm=$(echo "$status" | awk "/^VmHWM:/{print \$2}")
  vmsize=$(echo "$status" | awk "/^VmSize:/{print \$2}")
  echo "$name,$ipid,$rss,$pcpu,$vmrss,$vmhwm,$vmsize" >> "$OUT/summary.csv"
done
echo "done sample" | tee -a "$OUT/sample_meta.txt"
wc -l "$OUT/summary.csv"

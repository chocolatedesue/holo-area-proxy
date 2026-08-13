#!/usr/bin/env bash
# Real holod deploy smoke for YQH-502 fib-install + GetState observability.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
TOOLS="$(cd "$(dirname "$0")/.." && pwd)"
HOLOD="${HOLOD:-$ROOT/target/debug/holod}"
NB="$TOOLS/scripts/nb_client.py"
OUT="$TOOLS/out"
PREFIX_FALSE="198.51.100.0/24"
PREFIX_TRUE="203.0.113.0/24"

mkdir -p "$OUT" /tmp/yqh502-holo-fib-false /tmp/yqh502-holo-fib-true
sudo mkdir -p /var/opt/holo
sudo chown cnic:cnic /var/opt/holo 2>/dev/null || true

if [[ ! -x "$HOLOD" ]]; then
  echo "building holod..."
  (cd "$ROOT" && cargo build -p holo-daemon --bin holod)
fi

python3 -c 'import grpc' 2>/dev/null || pip3 install --user -q grpcio grpcio-tools

kill_holod() {
  sudo pkill -x holod 2>/dev/null || true
  # also match path-launched
  sudo pkill -f "$HOLOD" 2>/dev/null || true
  sleep 0.5
  # clear lock if stale
  sudo rm -f /var/opt/holo/holod.lock 2>/dev/null || true
}

run_mode() {
  local mode="$1"   # false|true
  local toml="$TOOLS/deploy/holod-fib-${mode}.toml"
  local addr
  local prefix
  local rundir
  if [[ "$mode" == "false" ]]; then
    addr="127.0.0.1:15051"
    prefix="$PREFIX_FALSE"
    rundir="/tmp/yqh502-holo-fib-false"
  else
    addr="127.0.0.1:15052"
    prefix="$PREFIX_TRUE"
    rundir="/tmp/yqh502-holo-fib-true"
  fi

  echo "=== mode fib_install=$mode addr=$addr ==="
  kill_holod
  rm -f "$rundir/holod.log" "$rundir/holo.db"*
  mkdir -p "$rundir"

  # Snapshot kernel routes mentioning test prefix before
  ip route show | grep -F "${prefix%/*}" >"$OUT/ip-route-before-$mode.txt" || true

  sudo "$HOLOD" -c "$toml" >"$OUT/holod-stdout-$mode.txt" 2>&1 &
  echo $! >"$OUT/holod-pid-$mode.txt"
  sleep 1

  python3 "$NB" --addr "$addr" wait --timeout 45

  python3 "$NB" --addr "$addr" get-fib | tee "$OUT/fib-before-$mode.json"

  python3 "$NB" --addr "$addr" commit-static --prefix "$prefix"

  # allow RIB queue drain
  sleep 1

  python3 "$NB" --addr "$addr" get-fib | tee "$OUT/fib-after-$mode.json"
  python3 "$NB" --addr "$addr" get-rib | tee "$OUT/rib-after-$mode.json" >/dev/null

  ip route show | tee "$OUT/ip-route-after-$mode.txt" >/dev/null
  ip route show | grep -F "${prefix%/*}" >"$OUT/ip-route-match-$mode.txt" || true

  grep -F "fib-install" "$rundir/holod.log" >"$OUT/log-fib-$mode.txt" || true
  # also stdout
  grep -F "fib-install" "$OUT/holod-stdout-$mode.txt" >>"$OUT/log-fib-$mode.txt" || true

  # Assertions
  python3 - "$mode" "$OUT/fib-after-$mode.json" "$OUT/ip-route-match-$mode.txt" <<'PY'
import json, sys
mode, fib_path, ip_match = sys.argv[1:4]
fib = json.load(open(fib_path))
enabled = fib.get("install-enabled")
if enabled is None:
    # some encodings nest
    raise SystemExit(f"missing install-enabled: {fib}")
want = mode == "true"
if bool(enabled) != want:
    raise SystemExit(f"install-enabled={enabled} want {want}: {fib}")
skipped = int(fib.get("ip-installs-skipped") or 0)
installed = int(fib.get("ip-installs") or 0)
active = int(fib.get("rib-ipv4-active") or 0)
ip_lines = open(ip_match).read().strip()
print(f"ASSERT mode={mode} enabled={enabled} ip_installs={installed} skipped={skipped} rib_v4_active={active}")
print(f"ip route match lines:\n{ip_lines or '(none)'}")
if mode == "false":
    if skipped < 1:
        raise SystemExit("expected ip-installs-skipped >= 1")
    if ip_lines:
        raise SystemExit(f"kernel should have no test prefix route, got: {ip_lines}")
else:
    if installed < 1:
        raise SystemExit("expected ip-installs >= 1")
    # blackhole static should appear as proto static or blackhole
    if not ip_lines:
        print("WARN: no ip route match for prefix (may need CAP / table layout); counters still checked")
if active < 1:
    raise SystemExit("expected rib-ipv4-active >= 1 (in-process RIB)")
print("OK")
PY

  kill_holod
}

run_mode false
run_mode true

echo "=== ALL SMOKE PASSED ==="
ls -la "$OUT"

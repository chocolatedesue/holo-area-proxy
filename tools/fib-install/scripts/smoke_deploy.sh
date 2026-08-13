#!/usr/bin/env bash
# Real holod deploy smoke for YQH-502 fib-install + GetState observability.
# true mode HARD-FAILS if kernel install errors or test prefix missing from FIB.
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
  sudo pkill -f "$HOLOD" 2>/dev/null || true
  sleep 0.5
  sudo rm -f /var/opt/holo/holod.lock 2>/dev/null || true
}

# Collect kernel lines for a destination (main + all tables).
match_kernel_prefix() {
  local host="$1"
  local outf="$2"
  {
    ip route show
    ip -4 route show table all 2>/dev/null || true
  } | grep -F "$host" | sort -u >"$outf" || true
}

run_mode() {
  local mode="$1"   # false|true
  local toml="$TOOLS/deploy/holod-fib-${mode}.toml"
  local addr prefix rundir
  if [[ "$mode" == "false" ]]; then
    addr="127.0.0.1:15051"
    prefix="$PREFIX_FALSE"
    rundir="/tmp/yqh502-holo-fib-false"
  else
    addr="127.0.0.1:15052"
    prefix="$PREFIX_TRUE"
    rundir="/tmp/yqh502-holo-fib-true"
  fi
  local host="${prefix%/*}"

  echo "=== mode fib_install=$mode addr=$addr prefix=$prefix ==="
  kill_holod
  # Drop leftover test routes from prior runs
  sudo ip route del blackhole "$prefix" 2>/dev/null || true
  sudo ip route del "$prefix" 2>/dev/null || true
  rm -f "$rundir/holod.log" "$rundir/holo.db"*
  mkdir -p "$rundir"

  match_kernel_prefix "$host" "$OUT/ip-route-before-$mode.txt"

  sudo "$HOLOD" -c "$toml" >"$OUT/holod-stdout-$mode.txt" 2>&1 &
  echo $! >"$OUT/holod-pid-$mode.txt"
  sleep 1

  python3 "$NB" --addr "$addr" wait --timeout 45

  python3 "$NB" --addr "$addr" get-fib | tee "$OUT/fib-before-$mode.json"

  python3 "$NB" --addr "$addr" commit-static --prefix "$prefix"

  # allow RIB queue + netlink drain
  sleep 1.5

  python3 "$NB" --addr "$addr" get-fib | tee "$OUT/fib-after-$mode.json"
  python3 "$NB" --addr "$addr" get-rib | tee "$OUT/rib-after-$mode.json" >/dev/null

  {
    ip route show
    ip -4 route show table all 2>/dev/null || true
  } | tee "$OUT/ip-route-after-$mode.txt" >/dev/null
  match_kernel_prefix "$host" "$OUT/ip-route-match-$mode.txt"

  grep -F "fib-install" "$rundir/holod.log" >"$OUT/log-fib-$mode.txt" || true
  grep -F "fib-install" "$OUT/holod-stdout-$mode.txt" >>"$OUT/log-fib-$mode.txt" || true
  grep -E "failed to install route|failed to uninstall route" \
    "$OUT/holod-stdout-$mode.txt" >"$OUT/log-netlink-err-$mode.txt" || true

  # Hard assertions (no soft WARN pass for true mode)
  python3 - "$mode" "$OUT/fib-after-$mode.json" \
    "$OUT/ip-route-match-$mode.txt" "$OUT/log-netlink-err-$mode.txt" \
    "$OUT/holod-stdout-$mode.txt" <<'PY'
import json, sys
mode, fib_path, ip_match, err_path, stdout_path = sys.argv[1:6]
fib = json.load(open(fib_path))
enabled = fib.get("install-enabled")
if enabled is None:
    raise SystemExit(f"missing install-enabled: {fib}")
want = mode == "true"
if bool(enabled) != want:
    raise SystemExit(f"install-enabled={enabled} want {want}: {fib}")

skipped = int(fib.get("ip-installs-skipped") or 0)
installed = int(fib.get("ip-installs") or 0)
active = int(fib.get("rib-ipv4-active") or 0)
ip_lines = open(ip_match).read().strip()
err_lines = open(err_path).read().strip()
stdout = open(stdout_path).read()

print(
    f"ASSERT mode={mode} enabled={enabled} "
    f"ip_installs={installed} skipped={skipped} rib_v4_active={active}"
)
print(f"ip route match lines:\n{ip_lines or '(none)'}")
if err_lines:
    print(f"netlink errors:\n{err_lines}")

if mode == "false":
    if skipped < 1:
        raise SystemExit("expected ip-installs-skipped >= 1")
    if ip_lines:
        raise SystemExit(
            f"kernel should have no test prefix route when fib_install=false, got:\n{ip_lines}"
        )
    if "failed to install route" in stdout:
        raise SystemExit("unexpected install error log when fib_install=false")
else:
    if installed < 1:
        raise SystemExit("expected ip-installs >= 1 (enqueued)")
    if "failed to install route" in stdout:
        raise SystemExit(
            "holod logged failed to install route — kernel install did not succeed"
        )
    if err_lines:
        raise SystemExit(f"netlink error log non-empty:\n{err_lines}")
    if not ip_lines:
        raise SystemExit(
            "expected kernel FIB to contain test prefix (ip route / table all)"
        )
    # Prefer blackhole/static evidence when present
    low = ip_lines.lower()
    if "blackhole" not in low and "proto static" not in low and "static" not in low:
        print("NOTE: match lines lack blackhole/static token; accepting any host match")

if active < 1:
    raise SystemExit("expected rib-ipv4-active >= 1 (in-process RIB)")
print("OK")
PY

  kill_holod
  # cleanup test prefix after true mode so host stays clean
  if [[ "$mode" == "true" ]]; then
    sudo ip route del blackhole "$prefix" 2>/dev/null || true
    sudo ip route del "$prefix" 2>/dev/null || true
  fi
}

run_mode false
run_mode true

echo "=== ALL SMOKE PASSED ==="
ls -la "$OUT"

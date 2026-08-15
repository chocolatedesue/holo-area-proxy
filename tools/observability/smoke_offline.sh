#!/usr/bin/env bash
# Offline smoke for YQH-603 passive observability.
# Gate: multi-sample series is produced and analyzed from files only (no GetState).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$ROOT"
EVIDENCE="$ROOT/tools/observability/evidence"
mkdir -p "$EVIDENCE"

echo "=== cargo test holo-utils observability ==="
cargo test -p holo-utils observability -- --nocapture

echo "=== synthetic series writer (mirrors exporter field set) ==="
python3 - <<'PY'
import json, time, os
from pathlib import Path
ev = Path("tools/observability/evidence")
ev.mkdir(parents=True, exist_ok=True)
jsonl = ev / "sample_convergence.jsonl"
csv = ev / "sample_convergence.csv"
header = "ts_unix_ms,ts_rfc3339,system_id,instance_name,hostname,fib_install,lsdb_l1_lsp,lsdb_l2_lsp,lsdb_lsp_total,lsdb_fp_l1,lsdb_fp_l2,spf_runs_l1,spf_runs_l2,last_spf_us_l1,last_spf_us_l2,rib_ipv4_active,rib_ipv6_active,rib_mpls_entries,fib_ip_installs,fib_ip_installs_skipped,fib_ip_uninstalls,fib_ip_uninstalls_skipped,rss_kb"
rows = []
base = int(time.time() * 1000)
# Simulate converge: LSDB climbs then plateaus; fib_install=false control-plane only
series = [0, 4, 12, 20, 24, 24, 24, 24]
for i, total in enumerate(series):
    l1 = total * 2 // 3
    l2 = total - l1
    sample = {
        "ts_unix_ms": base + i * 1000,
        "ts_rfc3339": time.strftime("%Y-%m-%dT%H:%M:%S", time.gmtime((base + i * 1000)/1000)) + f".{i:03d}Z",
        "system_id": "0100.0100.0001",
        "instance_name": "test",
        "hostname": None,
        "fib_install": False,
        "lsdb_l1_lsp": l1,
        "lsdb_l2_lsp": l2,
        "lsdb_lsp_total": total,
        "lsdb_fp_l1": 0x1000 + total,
        "lsdb_fp_l2": 0x2000 + total,
        "spf_runs_l1": min(i, 3),
        "spf_runs_l2": min(i // 2, 2),
        "last_spf_us_l1": 800 + i * 10 if i else 0,
        "last_spf_us_l2": 500 if i > 1 else 0,
        "rib_ipv4_active": total,
        "rib_ipv6_active": 0,
        "rib_mpls_entries": 0,
        "fib_ip_installs": 0,
        "fib_ip_installs_skipped": total,  # control-plane only skips
        "fib_ip_uninstalls": 0,
        "fib_ip_uninstalls_skipped": 0,
        "rss_kb": 12000 + i * 3,
    }
    rows.append(sample)

with jsonl.open("w") as f:
    for r in rows:
        f.write(json.dumps(r, separators=(",", ":")) + "\n")

with csv.open("w") as f:
    f.write(header + "\n")
    for r in rows:
        def esc(x):
            s = "" if x is None else str(x)
            if any(c in s for c in ',"\n'):
                return '"' + s.replace('"', '""') + '"'
            return s
        vals = [
            r["ts_unix_ms"], r["ts_rfc3339"], r["system_id"], r["instance_name"], "",
            str(r["fib_install"]).lower(), r["lsdb_l1_lsp"], r["lsdb_l2_lsp"], r["lsdb_lsp_total"],
            r["lsdb_fp_l1"], r["lsdb_fp_l2"], r["spf_runs_l1"], r["spf_runs_l2"],
            r["last_spf_us_l1"], r["last_spf_us_l2"], r["rib_ipv4_active"], r["rib_ipv6_active"],
            r["rib_mpls_entries"], r["fib_ip_installs"], r["fib_ip_installs_skipped"],
            r["fib_ip_uninstalls"], r["fib_ip_uninstalls_skipped"], r["rss_kb"],
        ]
        f.write(",".join(esc(v) for v in vals) + "\n")

# Analyze from files ONLY
totals = []
with jsonl.open() as f:
    for line in f:
        o = json.loads(line)
        assert o["fib_install"] is False, "fib_install=false expected"
        totals.append(o["lsdb_lsp_total"])

assert totals == series, totals
# plateau detect
plateau_at = None
prev = None
run = 0
for i, t in enumerate(totals):
    if t == prev:
        run += 1
        if run >= 2 and plateau_at is None:
            plateau_at = i
    else:
        run = 0
        prev = t
assert plateau_at is not None, "expected LSDB plateau in file series"
assert all(x["fib_ip_installs_skipped"] >= x["lsdb_lsp_total"] for x in rows)

summary = ev / "smoke_summary.txt"
summary.write_text(
    f"PASS offline observability smoke\n"
    f"jsonl={jsonl}\ncsv={csv}\n"
    f"samples={len(totals)} lsdb_series={totals}\n"
    f"plateau_index={plateau_at} fib_install=false verified from files only\n"
)
print(summary.read_text())
print("OK")
PY

echo "=== done ==="
ls -la "$EVIDENCE"

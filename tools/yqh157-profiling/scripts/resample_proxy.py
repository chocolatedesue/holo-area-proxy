#!/usr/bin/env python3
import csv, json, re, statistics, subprocess, time
from pathlib import Path

import os, sys
sys.path.insert(0, str(Path(__file__).resolve().parent))
from yqh157_paths import evidence_root, lab_root, state_root  # noqa: E402

WD = lab_root(os.environ.get("YQH157_LAB") or os.environ.get("YQH157_WD"))
STATE = state_root(WD)
PROTO = WD / "generated/proto"
EV = evidence_root(WD)
# Prefer auto-discover crun-state; allow CRUN_ROOT override
_crun_env = os.environ.get("CRUN_ROOT")
if _crun_env:
    CRUN = _crun_env
else:
    found = list(STATE.glob("runs/*/crun-state")) + list(STATE.glob("runs/*/*/crun-state"))
    CRUN = str(found[0]) if found else str(STATE / "runs/yqh157-torus66/crun-state")
PREFIX = os.environ.get("YQH157_PREFIX", "yqh157")
OUT_PROC = EV / "proc/stack_a"
OUT_CSV = EV / "csv"
OUT_PROC.mkdir(parents=True, exist_ok=True)
META = json.loads((WD / "generated/topology-meta.json").read_text())
NODES = sorted(
    META["nodes"].keys(),
    key=lambda n: (int(re.match(r"r(\d+)", n).group(1)), int(re.match(r"r\d+c(\d+)", n).group(1))),
)

def sh(cmd):
    return subprocess.run(cmd, shell=True, capture_output=True, text=True)

def node_pid(name):
    r = sh(f"sudo crun --root {CRUN} state {PREFIX}-{name}")
    return json.loads(r.stdout)["pid"]

def grpcurl(pid, method, data_obj):
    data = json.dumps(data_obj)
    cmd = (
        f"sudo nsenter -t {pid} -n grpcurl -plaintext "
        f"-import-path {PROTO} -proto holo.proto "
        f"-d @ 127.0.0.1:50051 {method}"
    )
    return subprocess.run(cmd, shell=True, input=data, capture_output=True, text=True)

def get_tree(pid):
    r = grpcurl(pid, "holo.Northbound/GetState", {"encoding": "JSON", "withDefaults": False})
    if r.returncode != 0:
        return None, r.stderr
    d = json.loads(r.stdout)
    s = (d.get("data") or {}).get("dataString") or ""
    if not s:
        return None, "empty"
    return json.loads(s), None

rows = []
for name in NODES:
    try:
        pid = node_pid(name)
        tree, err = get_tree(pid)
        if tree is None:
            rows.append({"node": name, "stack": "proxy", "error": err})
            continue
        text = json.dumps(tree)
        lsps = re.findall(r'"lsp-id"\s*:\s*"([^"]+)"', text, re.I)
        nbrs = re.findall(r'"neighbor-sys-?id"\s*:\s*"([^"]+)"', text, re.I)
        rows.append(
            {
                "node": name,
                "stack": "proxy",
                "lsp_count": len(lsps),
                "unique_nbrs": len(set(nbrs)),
                "error": "",
            }
        )
        print(name, len(lsps), len(set(nbrs)))
    except Exception as e:
        rows.append({"node": name, "stack": "proxy", "error": str(e)})
        print(name, "ERR", e)

with open(OUT_CSV / "lsdb_observed_proxy.csv", "w", newline="") as f:
    w = csv.DictWriter(
        f, fieldnames=["node", "stack", "lsp_count", "unique_nbrs", "error"], extrasaction="ignore"
    )
    w.writeheader()
    w.writerows(rows)
json.dump(rows, open(OUT_CSV / "lsdb_observed_proxy.json", "w"), indent=2)

proc_rows = []
raw = []
for name in NODES:
    init_pid = node_pid(name)
    ps = sh(
        f"sudo nsenter -t {init_pid} -m -p ps -o pid,rss,pcpu,comm -C holod 2>/dev/null | tail -n +2"
    )
    raw.append(f"{name} {ps.stdout.strip()}")
    line = ps.stdout.strip().splitlines()
    if not line:
        # fallback: any holod line
        ps2 = sh(f"sudo nsenter -t {init_pid} -m -p ps -eo pid,rss,pcpu,comm 2>/dev/null | grep holod | head -1")
        line = ps2.stdout.strip().splitlines()
        raw.append(f"{name} fallback {ps2.stdout.strip()}")
    if not line:
        continue
    parts = line[0].split()
    ipid, rss, pcpu = parts[0], parts[1], parts[2]
    st = sh(f"sudo nsenter -t {init_pid} -m -p cat /proc/{ipid}/status 2>/dev/null").stdout
    (OUT_PROC / f"{name}_proc_status.txt").write_text(st)

    def gv(k):
        m = re.search(rf"^{k}:\s+(\d+)", st, re.M)
        return int(m.group(1)) if m else None

    host_h = sh(f"pgrep -P {init_pid} -x holod 2>/dev/null | head -1").stdout.strip()
    if not host_h:
        # walk children
        host_h = sh(f"ps --ppid {init_pid} -o pid=,comm= 2>/dev/null | awk '/holod/{{print $1; exit}}'").stdout.strip()
    proc_rows.append(
        {
            "node": name,
            "host_init_pid": init_pid,
            "holod_pid": host_h or ipid,
            "rss_kb": int(rss) if str(rss).isdigit() else None,
            "pcpu": float(pcpu) if str(pcpu).replace(".", "", 1).isdigit() else None,
            "vmrss_kb": gv("VmRSS"),
            "vmhwm_kb": gv("VmHWM"),
            "vmsize_kb": gv("VmSize"),
            "cmd": "holod",
        }
    )

(OUT_PROC / "raw_ps_all.txt").write_text("\n".join(raw) + "\n")
with open(OUT_PROC / "summary.csv", "w", newline="") as f:
    w = csv.DictWriter(
        f,
        fieldnames=[
            "node",
            "host_init_pid",
            "holod_pid",
            "rss_kb",
            "pcpu",
            "vmrss_kb",
            "vmhwm_kb",
            "vmsize_kb",
            "cmd",
        ],
    )
    w.writeheader()
    w.writerows(proc_rows)

vm = [r["vmrss_kb"] for r in proc_rows if r.get("vmrss_kb")]
pcpu = [r["pcpu"] for r in proc_rows if r.get("pcpu") is not None]
psum = {
    "stack": "proxy",
    "n": 36,
    "holod_count": len(vm),
    "vmrss_kb_min": min(vm) if vm else None,
    "vmrss_kb_avg": statistics.mean(vm) if vm else None,
    "vmrss_kb_p95": sorted(vm)[int(0.95 * (len(vm) - 1))] if vm else None,
    "vmrss_kb_max": max(vm) if vm else None,
    "vmrss_kb_total": sum(vm) if vm else None,
    "pcpu_avg": statistics.mean(pcpu) if pcpu else 0,
    "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
}
json.dump(psum, open(OUT_CSV / "process_resources_stack_a.json", "w"), indent=2)
with open(OUT_CSV / "process_resources_stack_a.csv", "w", newline="") as f:
    w = csv.writer(f)
    w.writerow(list(psum.keys()))
    w.writerow(list(psum.values()))
print("PROXY_SUMMARY", psum)
lsps = [r.get("lsp_count") for r in rows if isinstance(r.get("lsp_count"), int)]
print(
    "LSDB proxy mean",
    statistics.mean(lsps) if lsps else None,
    "min",
    min(lsps) if lsps else None,
    "max",
    max(lsps) if lsps else None,
)

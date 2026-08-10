#!/usr/bin/env python3
"""Apply flat stack, commit YANG, wait converge, sample proc+lsdb. Prefix yqh157f."""
from __future__ import annotations
import csv, json, os, re, statistics, subprocess, sys, time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from yqh157_paths import evidence_root, expctl_bin, lab_root, state_root  # noqa: E402

EXP = "yqh157-flat-torus66"
PREFIX = "yqh157f"

# set in configure()
WD = STATE = EXPCTL = MAN = CFG = GEN = PROTO_DIR = EV = None
OUT_PROC = OUT_LOG = OUT_CSV = OUT_GOLD = None
META = {}
NODES = []


def configure(lab=None, expctl=None, state=None):
    global WD, STATE, EXPCTL, MAN, CFG, GEN, PROTO_DIR, EV
    global OUT_PROC, OUT_LOG, OUT_CSV, OUT_GOLD, META, NODES
    WD = lab_root(lab)
    STATE = state_root(WD, state)
    EXPCTL = expctl_bin(expctl)
    MAN = WD / "manifest" / f"{EXP}.yaml"
    CFG = WD / "configs_flat"
    GEN = WD / "generated_flat"
    PROTO_DIR = WD / "generated" / "proto"  # reuse proto from proxy gen
    EV = evidence_root(WD)
    OUT_PROC = EV / "proc" / "stack_a_flat"
    OUT_LOG = EV / "run_logs"
    OUT_CSV = EV / "csv"
    OUT_GOLD = EV / "gold_flat"
    for d in (OUT_PROC, OUT_LOG, OUT_CSV, OUT_GOLD, OUT_PROC / "status"):
        d.mkdir(parents=True, exist_ok=True)
    os.environ["EXPCTL_STATE_ROOT"] = str(STATE)
    META = json.loads((GEN / "topology-meta.json").read_text())
    NODES = sorted(
        META["nodes"].keys(),
        key=lambda n: (
            int(re.match(r"r(\d+)", n).group(1)),
            int(re.match(r"r\d+c(\d+)", n).group(1)),
        ),
    )


def sh(cmd, check=False, timeout=None):
    r = subprocess.run(cmd, shell=True, capture_output=True, text=True, timeout=timeout)
    if check and r.returncode != 0:
        raise RuntimeError(f"fail {cmd}\n{r.stderr[:800]}\n{r.stdout[:800]}")
    return r


def log(msg):
    line = f"[{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}] {msg}"
    print(line, flush=True)
    with open(OUT_LOG / "stack_a_flat_main.log", "a") as f:
        f.write(line + "\n")


def find_crun_for_prefix():
    cand = sorted((STATE / "runs" / EXP).glob("*/crun-state"), key=lambda p: p.stat().st_mtime if p.exists() else 0)
    if cand:
        path = str(cand[-1])
        n = int(sh(f"sudo crun --root {path} list 2>/dev/null | wc -l").stdout.strip() or 0)
        return path, n
    roots = sh(f"sudo find {STATE} -type d -name crun-state 2>/dev/null").stdout.strip().splitlines()
    best, best_n, best_m = None, 0, 0
    for path in roots:
        lst = sh(f"sudo crun --root {path} list 2>/dev/null").stdout
        n = sum(1 for ln in lst.splitlines() if f"{PREFIX}-r" in ln)
        m = os.path.getmtime(path) if os.path.exists(path) else 0
        if n > best_n or (n == best_n and m >= best_m and n > 0):
            best, best_n, best_m = path, n, m
    return best, best_n

def node_pid(crun, name):
    cname = f"{PREFIX}-{name}"
    r = sh(f"sudo crun --root {crun} state {cname}")
    if r.returncode != 0:
        raise RuntimeError(f"no state {cname}: {r.stderr}")
    return json.loads(r.stdout)["pid"]


def wait_grpc(pid, timeout=90):
    t0 = time.time()
    while time.time() - t0 < timeout:
        r = sh(f"sudo nsenter -t {pid} -n bash -c 'echo >/dev/tcp/127.0.0.1/50051' 2>/dev/null")
        if r.returncode == 0:
            return True
        time.sleep(0.5)
    return False


def grpcurl(pid, method, data_obj):
    data = json.dumps(data_obj)
    cmd = (
        f"sudo nsenter -t {pid} -n grpcurl -plaintext "
        f"-import-path {PROTO_DIR} -proto holo.proto "
        f"-d @ 127.0.0.1:50051 {method}"
    )
    return subprocess.run(cmd, shell=True, input=data, capture_output=True, text=True)


def commit_node(pid, name):
    cfg = (CFG / f"{name}.json").read_text()
    payload = {
        "operation": "REPLACE",
        "config": {"encoding": "JSON", "dataString": cfg},
        "comment": f"yqh157-flat-{name}",
    }
    r2 = grpcurl(pid, "holo.Northbound/Commit", payload)
    (OUT_GOLD / f"{name}-commit-resp.json").write_text(r2.stdout + ("\nERR\n" + r2.stderr if r2.returncode else ""))
    if r2.returncode != 0:
        (OUT_GOLD / f"{name}-commit.err").write_text(r2.stderr)
    return r2.returncode == 0, (r2.stdout or r2.stderr)[:200]


def get_state(pid):
    r = grpcurl(pid, "holo.Northbound/GetState", {"encoding": "JSON", "withDefaults": False})
    if r.returncode != 0:
        return None, r.stderr
    try:
        d = json.loads(r.stdout)
    except Exception as e:
        return None, str(e)
    s = (d.get("data") or {}).get("dataString") or (d.get("data") or {}).get("data_string") or ""
    if not s:
        return d, "no dataString"
    try:
        return json.loads(s), None
    except Exception:
        return s, "tree parse fail"


def count_lsdb(tree):
    text = json.dumps(tree) if not isinstance(tree, str) else tree
    lsps = re.findall(r'"lsp-id"\s*:\s*"([^"]+)"', text, re.I)
    nbrs = re.findall(r'"neighbor-sys-?id"\s*:\s*"([^"]+)"', text, re.I)
    return len(lsps), len(set(nbrs)), sorted(set(lsps))[:5]


def sample_proc(crun):
    rows = []
    raw = []
    for name in NODES:
        try:
            init_pid = node_pid(crun, name)
        except Exception as e:
            rows.append({"node": name, "error": str(e)})
            continue
        # holod via host children or nsenter
        ps = sh(f"sudo nsenter -t {init_pid} -m -p ps -o pid,rss,pcpu,comm -C holod 2>/dev/null | tail -n +2")
        raw.append(f"{name} {ps.stdout.strip()}")
        line = ps.stdout.strip().splitlines()
        if not line:
            rows.append({"node": name, "error": "NO_HOLOD", "init_pid": init_pid})
            continue
        parts = line[0].split()
        ipid, rss, pcpu = parts[0], parts[1], parts[2]
        st = sh(f"sudo nsenter -t {init_pid} -m -p cat /proc/{ipid}/status 2>/dev/null").stdout
        (OUT_PROC / "status" / f"{name}_holod.status").write_text(st)
        (OUT_PROC / f"{name}_proc_status.txt").write_text(st)
        def gv(key):
            m = re.search(rf"^{key}:\s+(\d+)", st, re.M)
            return int(m.group(1)) if m else None
        # host-visible holod pid
        host_h = sh(f"pgrep -P {init_pid} -x holod 2>/dev/null | head -1").stdout.strip()
        rows.append({
            "node": name,
            "host_init_pid": init_pid,
            "holod_pid": host_h or ipid,
            "rss_kb": int(rss) if rss.isdigit() else None,
            "pcpu": float(pcpu) if pcpu.replace(".", "", 1).isdigit() else None,
            "vmrss_kb": gv("VmRSS"),
            "vmhwm_kb": gv("VmHWM"),
            "vmsize_kb": gv("VmSize"),
            "cmd": "holod",
        })
    (OUT_PROC / "raw_ps_all.txt").write_text("\n".join(raw) + "\n")
    # csv
    fields = ["node", "host_init_pid", "holod_pid", "rss_kb", "pcpu", "vmrss_kb", "vmhwm_kb", "vmsize_kb", "cmd", "error"]
    with open(OUT_PROC / "summary.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=fields, extrasaction="ignore")
        w.writeheader()
        for r in rows:
            w.writerow(r)
    with open(OUT_PROC / "summary.json", "w") as f:
        json.dump(rows, f, indent=2)
    return rows


def ping_check(crun):
    """Simple main-path pings like gold."""
    # pairs: (src, dst_ip, label)
    pairs = [
        ("r1c1", "10.64.1.6", "I1"),
        ("r3c1", "10.64.3.6", "I2"),
        ("r5c1", "10.64.5.6", "I3"),
        ("r1c1", "10.64.3.3", "X01"),
        ("r3c3", "10.64.5.3", "X12"),
        ("r5c3", "10.64.1.3", "X20"),
        ("r1c1", "10.64.6.6", "far"),
    ]
    loss = {}
    for src, dip, lab in pairs:
        try:
            pid = node_pid(crun, src)
        except Exception:
            loss[lab] = 100
            continue
        r = sh(f"sudo nsenter -t {pid} -n ping -c 3 -W 1 {dip} 2>&1", timeout=20)
        (OUT_GOLD / f"ping_{lab}.txt").write_text(r.stdout + r.stderr)
        m = re.search(r"(\d+)% packet loss", r.stdout)
        loss[lab] = int(m.group(1)) if m else (0 if r.returncode == 0 else 100)
    return loss


def main():
    t_all = time.time()
    log("=== stack_a_flat start ===")
    # ensure image
    img = os.environ.get("HOLO_IMAGE", "docker.io/library/holo-bundle:yqh135-ee60831")
    sh(f"sudo podman image exists {img} || sudo podman pull {img}")
    # delete only flat exp if leftover
    r = sh(f"sudo env EXPCTL_STATE_ROOT={STATE} {EXPCTL} get -o json")
    (OUT_LOG / "stack_a_flat_expctl_pre.json").write_text(r.stdout)
    if EXP in r.stdout:
        log(f"delete leftover {EXP}")
        sh(f"sudo env EXPCTL_STATE_ROOT={STATE} {EXPCTL} delete {EXP} -o json")
        time.sleep(2)

    ts0 = time.time()
    log("apply start")
    r = sh(f"sudo env EXPCTL_STATE_ROOT={STATE} {EXPCTL} apply -f {MAN} -o json", timeout=600)
    (OUT_LOG / "stack_a_flat_apply.json").write_text(r.stdout + "\n" + r.stderr)
    (OUT_LOG / "stack_a_flat_apply.exit").write_text(str(r.returncode))
    log(f"apply exit={r.returncode} wall={time.time()-ts0:.1f}s")
    if r.returncode != 0:
        log("APPLY FAILED")
        sys.exit(2)

    r = sh(f"sudo env EXPCTL_STATE_ROOT={STATE} {EXPCTL} describe {EXP} -o json")
    (OUT_LOG / "stack_a_flat_describe.json").write_text(r.stdout)
    log("describe done")

    crun, ncon = find_crun_for_prefix()
    log(f"CRUN={crun} ncon~{ncon}")
    (OUT_PROC / "crun_root.txt").write_text(str(crun))
    assert crun

    # wait grpc all
    ready = 0
    for name in NODES:
        try:
            pid = node_pid(crun, name)
            if wait_grpc(pid, 60):
                ready += 1
            else:
                log(f"no grpc {name}")
        except Exception as e:
            log(f"pid fail {name}: {e}")
    log(f"grpc ready {ready}/{len(NODES)}")
    if ready < 30:
        log("too few grpc; abort")
        sys.exit(3)

    # commit all
    t_commit = time.time()
    ok_c = 0
    for name in NODES:
        pid = node_pid(crun, name)
        ok, snippet = commit_node(pid, name)
        if ok:
            ok_c += 1
        else:
            log(f"commit fail {name}: {snippet}")
    log(f"commit ok {ok_c}/{len(NODES)} wall={time.time()-t_commit:.1f}s")
    (OUT_LOG / "stack_a_flat_commit_summary.txt").write_text(f"ok={ok_c}/{len(NODES)}\n")

    # wait convergence: poll LSDB + pings
    t_conv0 = time.time()
    last_lsdb = []
    converged = False
    for round_i in range(24):  # up to ~4 min
        time.sleep(10)
        samples = []
        for name in ["r1c1", "r1c3", "r3c3", "r5c3", "r6c6", "r2c2", "r4c4", "r1c6"]:
            try:
                pid = node_pid(crun, name)
                tree, err = get_state(pid)
                if tree is None:
                    samples.append({"node": name, "err": err})
                    continue
                if isinstance(tree, dict):
                    (OUT_GOLD / f"{name}-state-tree.json").write_text(json.dumps(tree)[:500000])
                lc, nc, _ = count_lsdb(tree)
                samples.append({"node": name, "lsp_count": lc, "unique_nbrs": nc})
            except Exception as e:
                samples.append({"node": name, "err": str(e)})
        last_lsdb = samples
        (OUT_LOG / f"stack_a_flat_lsdb_round{round_i}.json").write_text(json.dumps(samples, indent=2))
        lsps = [s.get("lsp_count", 0) for s in samples if "lsp_count" in s]
        nbrs = [s.get("unique_nbrs", 0) for s in samples if "unique_nbrs" in s]
        log(f"round {round_i} lsps={lsps} nbrs={nbrs}")
        # converge heuristic: all sampled have lsp>=12 and nbrs>=2
        if lsps and min(lsps) >= 12 and nbrs and min(nbrs) >= 2:
            # also ping
            loss = ping_check(crun)
            log(f"ping loss {loss}")
            main_ok = all(loss.get(k, 100) == 0 for k in ("I1", "I2", "I3"))
            if main_ok or (min(lsps) >= 20 and statistics.mean(lsps) >= 20):
                converged = True
                break
    t_conv = time.time() - t_conv0
    log(f"converge_wall_s={t_conv:.1f} converged={converged}")

    # final full LSDB sample all nodes
    lsdb_rows = []
    for name in NODES:
        try:
            pid = node_pid(crun, name)
            tree, err = get_state(pid)
            if tree is None:
                lsdb_rows.append({"node": name, "error": err})
                continue
            lc, nc, sample = count_lsdb(tree)
            lsdb_rows.append({"node": name, "lsp_count": lc, "unique_nbrs": nc, "lsp_sample": "|".join(sample)})
        except Exception as e:
            lsdb_rows.append({"node": name, "error": str(e)})
    with open(OUT_CSV / "lsdb_observed_flat.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["node", "lsp_count", "unique_nbrs", "lsp_sample", "error"], extrasaction="ignore")
        w.writeheader()
        for r in lsdb_rows:
            w.writerow(r)
    with open(OUT_CSV / "lsdb_observed_flat.json", "w") as f:
        json.dump(lsdb_rows, f, indent=2)
    # also copy as lsdb_observed.* primary for flat
    with open(OUT_CSV / "lsdb_observed.csv", "w", newline="") as f:
        w = csv.DictWriter(f, fieldnames=["node", "stack", "lsp_count", "unique_nbrs", "error"], extrasaction="ignore")
        w.writeheader()
        for r in lsdb_rows:
            w.writerow({"node": r.get("node"), "stack": "flat", "lsp_count": r.get("lsp_count"), "unique_nbrs": r.get("unique_nbrs"), "error": r.get("error")})

    # process sample
    log("sample proc")
    rows = sample_proc(crun)
    vm = [r["vmrss_kb"] for r in rows if r.get("vmrss_kb")]
    pcpu = [r["pcpu"] for r in rows if r.get("pcpu") is not None]
    summary = {
        "stack": "flat",
        "n": len(NODES),
        "holod_count": len(vm),
        "vmrss_kb_min": min(vm) if vm else None,
        "vmrss_kb_avg": statistics.mean(vm) if vm else None,
        "vmrss_kb_p95": sorted(vm)[int(0.95 * (len(vm) - 1))] if vm else None,
        "vmrss_kb_max": max(vm) if vm else None,
        "vmrss_kb_total": sum(vm) if vm else None,
        "pcpu_avg": statistics.mean(pcpu) if pcpu else None,
        "converge_wall_s": t_conv,
        "converged": converged,
        "lsdb_sample": last_lsdb,
        "apply_to_end_s": time.time() - t_all,
    }
    loss = ping_check(crun)
    summary["ping_loss_pct"] = loss
    summary["main_path_ok"] = all(loss.get(k, 100) == 0 for k in ("I1", "I2", "I3"))
    (OUT_CSV / "process_resources_stack_a_flat.json").write_text(json.dumps(summary, indent=2))
    with open(OUT_CSV / "process_resources_stack_a_flat.csv", "w", newline="") as f:
        w = csv.writer(f)
        w.writerow(["stack", "n", "holod_count", "vmrss_kb_avg", "vmrss_kb_p95", "vmrss_kb_min", "vmrss_kb_max", "vmrss_kb_total", "pcpu_avg", "converge_wall_s", "main_path_ok"])
        w.writerow(["flat", summary["n"], summary["holod_count"], summary["vmrss_kb_avg"], summary["vmrss_kb_p95"], summary["vmrss_kb_min"], summary["vmrss_kb_max"], summary["vmrss_kb_total"], summary["pcpu_avg"], summary["converge_wall_s"], summary["main_path_ok"]])
    (OUT_LOG / "stack_a_flat_convergence.json").write_text(json.dumps(summary, indent=2))
    (OUT_PROC / "sample_meta.txt").write_text(json.dumps(summary, indent=2))
    log(f"DONE summary={json.dumps(summary)}")
    print(json.dumps(summary, indent=2))
    return 0


def _parse_args(argv=None):
    import argparse
    ap = argparse.ArgumentParser(description="Apply flat stack and sample proc+lsdb")
    ap.add_argument("--lab", default=None)
    ap.add_argument("--expctl", default=None)
    ap.add_argument("--state-root", default=None)
    return ap.parse_args(argv)


if __name__ == "__main__":
    args = _parse_args()
    configure(lab=args.lab, expctl=args.expctl, state=args.state_root)
    sys.exit(main() or 0)

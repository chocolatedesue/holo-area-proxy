#!/usr/bin/env python3
"""YQH-174/157: per-container holod CPU/RSS timeseries from apply through converge + steady.

Usage:
  YQH157_LAB=/path/to/lab python3 run_timeseries.py flat
  python3 run_timeseries.py proxy --lab /path/to/lab --expctl /path/to/expctl
  python3 run_timeseries.py both

Env: YQH157_LAB (or YQH157_WD), EXPCTL, EXPCTL_STATE_ROOT
Guardrails: do not destroy yqh116/103/135 labs; Phase1 is flat vs proxy only (no FRR main claim).
"""
from __future__ import annotations

import argparse
import csv
import json
import os
import re
import statistics
import subprocess
import sys
import threading
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from yqh157_paths import evidence_root, expctl_bin, lab_root, state_root  # noqa: E402

INTERVAL_S = 3.0
STEADY_AFTER_S = 90.0  # post-converge sampling window

# Filled by configure()
WD: Path
STATE: Path
EXPCTL: Path
PROTO_DIR: Path
EV: Path
OUT_LOG: Path
STACKS: dict


def configure(lab=None, expctl=None, state=None):
    """Resolve lab paths from CLI/env. Call once from main before run_stack."""
    global WD, STATE, EXPCTL, PROTO_DIR, EV, OUT_LOG, STACKS
    WD = lab_root(lab)
    STATE = state_root(WD, state)
    EXPCTL = expctl_bin(expctl)
    PROTO_DIR = WD / "generated" / "proto"
    EV = evidence_root(WD)
    OUT_LOG = EV / "run_logs"
    OUT_LOG.mkdir(parents=True, exist_ok=True)
    os.environ["EXPCTL_STATE_ROOT"] = str(STATE)
    STACKS = {
        "flat": {
            "exp": "yqh157-flat-torus66",
            "prefix": "yqh157f",
            "man": WD / "manifest" / "yqh157-flat-torus66.yaml",
            "cfg": WD / "configs_flat",
            "gen": WD / "generated_flat",
            "out": EV / "timeseries" / "stack_flat",
            "stack_name": "flat",
        },
        "proxy": {
            "exp": "yqh157-torus66",
            "prefix": "yqh157",
            "man": WD / "manifest" / "yqh157-torus66.yaml",
            "cfg": WD / "configs",
            "gen": WD / "generated",
            "out": EV / "timeseries" / "stack_proxy",
            "stack_name": "proxy",
        },
    }


def sh(cmd, check=False, timeout=None, input_text=None):
    r = subprocess.run(
        cmd,
        shell=True,
        capture_output=True,
        text=True,
        timeout=timeout,
        input=input_text,
    )
    if check and r.returncode != 0:
        raise RuntimeError(f"fail {cmd}\n{r.stderr[:800]}\n{r.stdout[:800]}")
    return r


def log(msg, log_path: Path):
    line = f"[{time.strftime('%Y-%m-%dT%H:%M:%SZ', time.gmtime())}] {msg}"
    print(line, flush=True)
    with open(log_path, "a") as f:
        f.write(line + "\n")


def load_nodes(gen: Path):
    meta = json.loads((gen / "topology-meta.json").read_text())
    nodes = sorted(
        meta["nodes"].keys(),
        key=lambda n: (
            int(re.match(r"r(\d+)", n).group(1)),
            int(re.match(r"r\d+c(\d+)", n).group(1)),
        ),
    )
    return nodes


def find_crun(exp: str, prefix: str):
    cand = sorted(
        (STATE / "runs" / exp).glob("*/crun-state"),
        key=lambda p: p.stat().st_mtime if p.exists() else 0,
    )
    if cand:
        path = str(cand[-1])
        n = int(sh(f"sudo crun --root {path} list 2>/dev/null | wc -l").stdout.strip() or 0)
        if n > 0:
            return path, n
    roots = sh(f"sudo find {STATE} -type d -name crun-state 2>/dev/null").stdout.strip().splitlines()
    best, best_n, best_m = None, 0, 0
    for path in roots:
        lst = sh(f"sudo crun --root {path} list 2>/dev/null").stdout
        n = sum(1 for ln in lst.splitlines() if f"{prefix}-r" in ln)
        m = os.path.getmtime(path) if os.path.exists(path) else 0
        if n > best_n or (n == best_n and m >= best_m and n > 0):
            best, best_n, best_m = path, n, m
    return best, best_n


def node_init_pid(crun: str, prefix: str, name: str):
    cname = f"{prefix}-{name}"
    r = sh(f"sudo crun --root {crun} state {cname}")
    if r.returncode != 0:
        return None
    try:
        return int(json.loads(r.stdout)["pid"])
    except Exception:
        return None


def holod_pid_for_init(init_pid: int):
    r = sh(f"pgrep -P {init_pid} -x holod 2>/dev/null")
    pids = [int(x) for x in r.stdout.split() if x.strip().isdigit()]
    if pids:
        return pids[0]
    r = sh(f"pgrep -P {init_pid} 2>/dev/null")
    for c in r.stdout.split():
        if not c.isdigit():
            continue
        r2 = sh(f"ps -p {c} -o comm= 2>/dev/null")
        if r2.stdout.strip() == "holod":
            return int(c)
        r3 = sh(f"pgrep -P {c} -x holod 2>/dev/null")
        if r3.stdout.strip().isdigit():
            return int(r3.stdout.strip())
    return None


def read_rss_pcpu(pid: int):
    rss_kb = None
    pcpu = None
    try:
        st = Path(f"/proc/{pid}/status").read_text()
        m = re.search(r"^VmRSS:\s+(\d+)", st, re.M)
        if m:
            rss_kb = int(m.group(1))
    except Exception:
        pass
    r = sh(f"ps -p {pid} -o pcpu= 2>/dev/null")
    s = r.stdout.strip()
    if s:
        try:
            pcpu = float(s)
        except ValueError:
            pass
    return rss_kb, pcpu


class Sampler:
    def __init__(self, stack_name: str, prefix: str, nodes: list, out_csv: Path, log_path: Path):
        self.stack_name = stack_name
        self.prefix = prefix
        self.nodes = nodes
        self.out_csv = out_csv
        self.log_path = log_path
        self.crun = None
        self.stop = threading.Event()
        self.phase = "pre"
        self.lock = threading.Lock()
        self.sample_count = 0
        self.row_count = 0
        self.thread = None
        self.start_ts = None
        self.end_ts = None
        self._init_cache = {}  # node -> init_pid
        self._refresh_every = 5
        self.out_csv.parent.mkdir(parents=True, exist_ok=True)
        self._fh = open(self.out_csv, "w", newline="")
        self._w = csv.DictWriter(
            self._fh,
            fieldnames=["ts", "ts_iso", "stack", "node", "pid", "rss_kb", "pcpu", "phase_hint"],
        )
        self._w.writeheader()
        self._fh.flush()

    def set_phase(self, phase: str):
        with self.lock:
            self.phase = phase

    def set_crun(self, crun: str):
        with self.lock:
            self.crun = crun

    def _one_round(self):
        with self.lock:
            crun = self.crun
            phase = self.phase
        ts = time.time()
        ts_iso = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(ts))
        if not crun:
            self.sample_count += 1
            return
        node_pids = {}
        refresh = (self.sample_count % self._refresh_every == 0)
        for name in self.nodes:
            init = self._init_cache.get(name) if not refresh else None
            if init is None or not Path(f"/proc/{init}").exists():
                init = node_init_pid(crun, self.prefix, name)
                if init is not None:
                    self._init_cache[name] = init
            if init is None:
                continue
            hpid = holod_pid_for_init(init)
            if hpid is None:
                continue
            node_pids[name] = hpid
        if not node_pids:
            self.sample_count += 1
            return
        # bulk ps
        pid_list = ",".join(str(p) for p in node_pids.values())
        ps = sh(f"ps -p {pid_list} -o pid=,rss=,pcpu= 2>/dev/null")
        by_pid = {}
        for line in ps.stdout.splitlines():
            parts = line.split()
            if len(parts) >= 3:
                try:
                    by_pid[int(parts[0])] = (int(parts[1]), float(parts[2]))
                except ValueError:
                    continue
        for name, hpid in node_pids.items():
            if hpid in by_pid:
                rss, pcpu = by_pid[hpid]
            else:
                rss, pcpu = read_rss_pcpu(hpid)
            if rss is None and pcpu is None:
                continue
            row = {
                "ts": f"{ts:.3f}",
                "ts_iso": ts_iso,
                "stack": self.stack_name,
                "node": name,
                "pid": hpid,
                "rss_kb": rss if rss is not None else "",
                "pcpu": pcpu if pcpu is not None else "",
                "phase_hint": phase,
            }
            self._w.writerow(row)
            self.row_count += 1
        self._fh.flush()
        self.sample_count += 1

    def _loop(self):
        self.start_ts = time.time()
        while not self.stop.is_set():
            t0 = time.time()
            try:
                self._one_round()
            except Exception as e:
                log(f"sampler err: {e}", self.log_path)
            elapsed = time.time() - t0
            wait = max(0.05, INTERVAL_S - elapsed)
            self.stop.wait(wait)
        try:
            self._one_round()
        except Exception:
            pass
        self.end_ts = time.time()
        self._fh.close()

    def start(self):
        self.thread = threading.Thread(target=self._loop, name="ts-sampler", daemon=True)
        self.thread.start()

    def stop_join(self):
        self.stop.set()
        if self.thread:
            self.thread.join(timeout=60)


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


def commit_node(pid, name, cfg_dir: Path):
    cfg = (cfg_dir / f"{name}.json").read_text()
    payload = {
        "operation": "REPLACE",
        "config": {"encoding": "JSON", "dataString": cfg},
        "comment": f"yqh174-ts-{name}",
    }
    r2 = grpcurl(pid, "holo.Northbound/Commit", payload)
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
    return len(lsps), len(set(nbrs))


def ping_check(crun, prefix, pairs=None):
    if pairs is None:
        pairs = [
            ("r1c1", "10.64.1.6", "I1"),
            ("r3c1", "10.64.3.6", "I2"),
            ("r5c1", "10.64.5.6", "I3"),
        ]
    loss = {}
    for src, dip, lab in pairs:
        init = node_init_pid(crun, prefix, src)
        if init is None:
            loss[lab] = 100
            continue
        r = sh(f"sudo nsenter -t {init} -n ping -c 3 -W 1 {dip} 2>&1", timeout=20)
        m = re.search(r"(\d+)% packet loss", r.stdout)
        loss[lab] = int(m.group(1)) if m else (0 if r.returncode == 0 else 100)
    return loss


def run_stack(key: str):
    cfg = STACKS[key]
    exp = cfg["exp"]
    prefix = cfg["prefix"]
    out = cfg["out"]
    out.mkdir(parents=True, exist_ok=True)
    OUT_LOG.mkdir(parents=True, exist_ok=True)
    log_path = OUT_LOG / f"timeseries_{key}.log"
    log_path.write_text("")

    nodes = load_nodes(cfg["gen"])
    stack_name = cfg["stack_name"]
    os.environ["EXPCTL_STATE_ROOT"] = str(STATE)

    samples_csv = out / "samples.csv"
    sampler = Sampler(stack_name, prefix, nodes, samples_csv, log_path)

    meta = {
        "stack": stack_name,
        "exp": exp,
        "prefix": prefix,
        "interval_s": INTERVAL_S,
        "node_count": len(nodes),
        "steady_after_s": STEADY_AFTER_S,
        "host": sh("hostname").stdout.strip(),
        "timestamps": {},
        "converge": {},
    }

    log(f"=== timeseries {key} start ===", log_path)
    sh(
        "sudo podman image exists docker.io/library/holo-bundle:yqh135-ee60831 || "
        "sudo podman pull docker.io/library/holo-bundle:yqh135-ee60831"
    )

    r = sh(f"sudo env EXPCTL_STATE_ROOT={STATE} {EXPCTL} get -o json")
    (OUT_LOG / f"timeseries_{key}_expctl_pre.json").write_text(r.stdout)
    try:
        exps = json.loads(r.stdout) if r.stdout.strip() else []
        names = {e.get("name"): e.get("phase") for e in exps}
    except Exception:
        names = {}
    if names.get(exp) and names.get(exp) != "Deleted":
        log(f"delete leftover {exp} phase={names.get(exp)}", log_path)
        sh(f"sudo env EXPCTL_STATE_ROOT={STATE} {EXPCTL} delete {exp} -o json", timeout=300)
        time.sleep(3)

    sampler.set_phase("pre_apply")
    sampler.start()
    meta["timestamps"]["sampler_start"] = time.time()
    meta["timestamps"]["sampler_start_iso"] = time.strftime(
        "%Y-%m-%dT%H:%M:%SZ", time.gmtime(meta["timestamps"]["sampler_start"])
    )

    sampler.set_phase("apply")
    t_apply0 = time.time()
    meta["timestamps"]["apply_start"] = t_apply0
    meta["timestamps"]["apply_start_iso"] = time.strftime(
        "%Y-%m-%dT%H:%M:%SZ", time.gmtime(t_apply0)
    )
    log("apply start", log_path)
    r = sh(
        f"sudo env EXPCTL_STATE_ROOT={STATE} {EXPCTL} apply -f {cfg['man']} -o json",
        timeout=600,
    )
    (OUT_LOG / f"timeseries_{key}_apply.json").write_text(r.stdout + "\n" + r.stderr)
    (OUT_LOG / f"timeseries_{key}_apply.exit").write_text(str(r.returncode))
    t_apply1 = time.time()
    meta["timestamps"]["apply_end"] = t_apply1
    meta["timestamps"]["apply_wall_s"] = t_apply1 - t_apply0
    log(f"apply exit={r.returncode} wall={t_apply1 - t_apply0:.1f}s", log_path)
    if r.returncode != 0:
        sampler.set_phase("apply_failed")
        sampler.stop_join()
        meta["error"] = "apply_failed"
        (out / "meta.json").write_text(json.dumps(meta, indent=2))
        return 2

    crun, ncon = find_crun(exp, prefix)
    log(f"CRUN={crun} ncon~{ncon}", log_path)
    if not crun:
        sampler.stop_join()
        meta["error"] = "no_crun"
        (out / "meta.json").write_text(json.dumps(meta, indent=2))
        return 3
    sampler.set_crun(crun)
    sampler.set_phase("post_apply_wait_grpc")
    (out / "crun_root.txt").write_text(str(crun))

    ready = 0
    for name in nodes:
        init = node_init_pid(crun, prefix, name)
        if init and wait_grpc(init, 60):
            ready += 1
        else:
            log(f"no grpc {name}", log_path)
    log(f"grpc ready {ready}/{len(nodes)}", log_path)
    meta["grpc_ready"] = ready
    if ready < 30:
        sampler.set_phase("grpc_fail")
        sampler.stop_join()
        meta["error"] = "too_few_grpc"
        (out / "meta.json").write_text(json.dumps(meta, indent=2))
        return 4

    sampler.set_phase("commit")
    t_commit0 = time.time()
    meta["timestamps"]["commit_start"] = t_commit0
    ok_c = 0
    for name in nodes:
        init = node_init_pid(crun, prefix, name)
        if not init:
            continue
        ok, snip = commit_node(init, name, cfg["cfg"])
        if ok:
            ok_c += 1
        else:
            log(f"commit fail {name}: {snip}", log_path)
    t_commit1 = time.time()
    meta["timestamps"]["commit_end"] = t_commit1
    meta["commit_ok"] = f"{ok_c}/{len(nodes)}"
    log(f"commit ok {ok_c}/{len(nodes)} wall={t_commit1 - t_commit0:.1f}s", log_path)

    sampler.set_phase("converge_wait")
    t_conv0 = time.time()
    meta["timestamps"]["converge_poll_start"] = t_conv0
    converged = False
    last_lsdb = []
    converge_ts = None
    thr = 12 if key == "flat" else 10
    for round_i in range(30):
        time.sleep(10)
        samples = []
        probe = ["r1c1", "r1c3", "r3c3", "r5c3", "r6c6", "r2c2", "r4c4", "r1c6"]
        for name in probe:
            init = node_init_pid(crun, prefix, name)
            if not init:
                samples.append({"node": name, "err": "no_pid"})
                continue
            tree, err = get_state(init)
            if tree is None:
                samples.append({"node": name, "err": err})
                continue
            lc, nc = count_lsdb(tree)
            samples.append({"node": name, "lsp_count": lc, "unique_nbrs": nc})
        last_lsdb = samples
        lsps = [s.get("lsp_count", 0) for s in samples if "lsp_count" in s]
        nbrs = [s.get("unique_nbrs", 0) for s in samples if "unique_nbrs" in s]
        log(f"round {round_i} lsps={lsps} nbrs={nbrs}", log_path)
        if lsps and min(lsps) >= thr and nbrs and min(nbrs) >= 2:
            loss = ping_check(crun, prefix)
            log(f"ping loss {loss}", log_path)
            main_ok = all(loss.get(k, 100) == 0 for k in ("I1", "I2", "I3"))
            if main_ok or (min(lsps) >= thr + 5 and statistics.mean(lsps) >= thr + 5):
                converged = True
                converge_ts = time.time()
                break

    if converge_ts is None:
        converge_ts = time.time()
    meta["timestamps"]["converge_ts"] = converge_ts
    meta["timestamps"]["converge_ts_iso"] = time.strftime(
        "%Y-%m-%dT%H:%M:%SZ", time.gmtime(converge_ts)
    )
    loss_final = ping_check(crun, prefix) if converged else {}
    meta["converge"] = {
        "converged": converged,
        "poll_wall_s": converge_ts - t_conv0,
        "apply_to_converge_s": converge_ts - t_apply0,
        "lsdb_sample": last_lsdb,
        "ping_loss": loss_final,
    }
    log(
        f"converge_ts={meta['timestamps']['converge_ts_iso']} "
        f"converged={converged} apply_to_conv={meta['converge']['apply_to_converge_s']:.1f}s",
        log_path,
    )

    sampler.set_phase("steady")
    log(f"steady sampling {STEADY_AFTER_S}s", log_path)
    time.sleep(STEADY_AFTER_S)

    sampler.set_phase("done")
    sampler.stop_join()
    meta["timestamps"]["sampler_end"] = sampler.end_ts or time.time()
    meta["timestamps"]["sampler_end_iso"] = time.strftime(
        "%Y-%m-%dT%H:%M:%SZ", time.gmtime(meta["timestamps"]["sampler_end"])
    )
    meta["timestamps"]["start"] = meta["timestamps"]["apply_start"]
    meta["timestamps"]["end"] = meta["timestamps"]["sampler_end"]
    meta["sample_rounds"] = sampler.sample_count
    meta["sample_count"] = sampler.row_count
    meta["duration_s"] = meta["timestamps"]["end"] - meta["timestamps"]["start"]

    try:
        with open(samples_csv) as f:
            rows = list(csv.DictReader(f))
        tss = sorted(set(r["ts"] for r in rows))
        meta["unique_ts_count"] = len(tss)
        if tss:
            per = {}
            for r in rows:
                per.setdefault(r["ts"], 0)
                per[r["ts"]] += 1
            meta["nodes_per_round_mean"] = statistics.mean(per.values())
            meta["nodes_per_round_min"] = min(per.values())
            meta["nodes_per_round_max"] = max(per.values())
            meta["integrity_multi_ts"] = len(tss) >= 15
            rss_vals = [int(r["rss_kb"]) for r in rows if r.get("rss_kb") not in ("", None)]
            if len(rss_vals) > 10:
                meta["rss_kb_min"] = min(rss_vals)
                meta["rss_kb_max"] = max(rss_vals)
                meta["rss_kb_mean"] = statistics.mean(rss_vals)
            pcpus = [float(r["pcpu"]) for r in rows if r.get("pcpu") not in ("", None)]
            if pcpus:
                meta["pcpu_max_observed"] = max(pcpus)
                meta["pcpu_mean_all"] = statistics.mean(pcpus)
    except Exception as e:
        meta["integrity_error"] = str(e)

    (out / "meta.json").write_text(json.dumps(meta, indent=2))
    log(f"DONE rows={sampler.row_count} rounds={sampler.sample_count} meta written", log_path)
    print(json.dumps(meta, indent=2))
    return 0 if converged else 5


def main():
    ap = argparse.ArgumentParser(description="YQH-157 real multi_ts holod sampling (flat/proxy)")
    ap.add_argument("stack", choices=["flat", "proxy", "both"])
    ap.add_argument("--lab", default=None, help="Lab root ($YQH157_LAB)")
    ap.add_argument("--expctl", default=None, help="expctl binary ($EXPCTL)")
    ap.add_argument("--state-root", default=None, help="EXPCTL_STATE_ROOT (default: <lab>/state)")
    args = ap.parse_args()
    configure(lab=args.lab, expctl=args.expctl, state=args.state_root)
    keys = ["flat", "proxy"] if args.stack == "both" else [args.stack]
    rc = 0
    for k in keys:
        r = run_stack(k)
        if r != 0:
            rc = r
    return rc


if __name__ == "__main__":
    sys.exit(main())

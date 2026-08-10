# YQH-157 profiling tools

Lab-side scripts for **real** flat vs proxy holod process/timeseries sampling and figures
(Phase 1 of YQH-157). Checked into the product worktree so agents can reuse them.

**Agent entrypoint:** install/load skill `yqh157-area-proxy-profiling`
(`.grok/skills/…/SKILL.md` or `~/.grok/skills/…`). In-repo copy:
`tools/yqh157-profiling/skill/SKILL.md` (versioned; does not replace discoverable paths).

## Layout

```
tools/yqh157-profiling/
  README.md
  skill/SKILL.md            # versioned skill copy (install to .grok/skills for discovery)
  examples/                 # tiny CSV/meta snippets only (not full evidence packs)
  scripts/
    yqh157_paths.py         # shared path + HOLO_IMAGE resolution
    run_timeseries.py       # apply + multi_ts sample flat|proxy|both
    plot_timeseries.py      # fig_ts_* CPU/RSS plots
    generate_flat.py        # Area Proxy OFF configs + manifest
    generate_all.py         # proxy (Area Proxy ON) configs + manifest
    run_stack_a_flat.py     # flat apply + single-shot proc/lsdb sample
    run_stack_a.sh          # proxy apply helper
    sample_proc_stack_a.sh  # one-shot holod RSS/CPU via crun
    validate_gold.sh        # gold ping/state checks
    start-node.sh           # container entry (proxy prefix /opt/yqh157)
    start-node-flat.sh      # container entry (flat prefix /opt/yqh157f)
    node_exec.sh            # crun exec helper
    resample_proxy.py       # re-sample running proxy stack
```

## Hosts

| Role | Host | Notes |
|------|------|--------|
| Coordination / git / Multica | **yqh2** | product worktree below |
| Lab (expctl, podman, evidence) | **yqh1** via `ssh yqh1` | do not run long apply from wrong host |

Product worktree (yqh2): `/home/cnic/work/area-proxy-holo-yqh157-profiling`  
Branch: `yqh157-profiling` · Fork: `https://github.com/chocolatedesue/holo-area-proxy.git`  
Baseline tools commit: `e7b6c5c` (later follow-up SHAs OK if documented).

## Lab path convention (yqh1)

| Variable | Default (legacy fallback) | **YQH-184 / independent Stage B** |
|----------|---------------------------|-------------------------------------|
| `YQH157_LAB` or `YQH157_WD` | `/home/cnic/work/yqh157-real-profiling` | **`/home/cnic/work/yqh184-profiling-repro` (required)** |
| `EXPCTL` | `/home/cnic/work/smu/build/linux/arm64/release/expctl` | same |
| `EXPCTL_STATE_ROOT` | `$YQH157_LAB/state` | **must** be under the independent lab |
| `HOLO_IMAGE` | `docker.io/library/holo-bundle:yqh135-ee60831` | override both stacks together if needed |
| `CRUN_ROOT` | auto under state | optional |

### WARNING — do not destroy the YQH-157 truth lab

- **YQH-157 truth** (`/home/cnic/work/yqh157-real-profiling/`) is **read-only contrast** for Stage B.
  Do **not** set `YQH157_LAB` / `EXPCTL_STATE_ROOT` there for destructive re-apply or delete.
- Script defaults still fall back to the truth path when env is unset (legacy). **Skill and
  Stage B must always export the independent lab.**
- `run_timeseries.py` may `expctl delete` the **same experiment name** under the **active**
  `EXPCTL_STATE_ROOT`. Shared state root with the truth lab can wipe Ready stacks.
- **Do not** point these at yqh116 / yqh103 / yqh135 labs. **Do not** `expctl delete` or destroy those experiments.
- **Do not** `git add` lab `evidence/` into this product tree (`?? evidence/` must stay untracked).

Typical lab tree on yqh1:

```
$YQH157_LAB/
  configs/ configs_flat/
  generated/ generated_flat/
  manifest/
  state/                 # EXPCTL_STATE_ROOT
  evidence/
    timeseries/stack_flat|stack_proxy/{samples.csv,meta.json}
    figures/fig_ts_*.{png,svg}
    proc/ csv/ gold/ gold_flat/ run_logs/
    REPRO_REPORT.md      # Stage B
```

## Dependencies

- Host: Linux, `python3`, `matplotlib`, `numpy`, `sudo`, `crun`, `podman`, `nsenter`, `grpcurl` (for YANG commit paths)
- Image: `docker.io/library/holo-bundle:yqh135-ee60831` (or set `HOLO_IMAGE`)
- expctl from smu build (arm64 path above on yqh1)

```bash
python3 -m pip install --user matplotlib numpy
```

## Phase 1 scope (flat vs proxy only)

| Stack | Experiment | Prefix | Area Proxy |
|-------|------------|--------|------------|
| flat  | `yqh157-flat-torus66` | `yqh157f` | OFF |
| proxy | `yqh157-torus66` | `yqh157` | ON |

- Only dial **area-proxy enabled** between stacks (`generate_flat` vs `generate_all`); keep N=36 / image comparable.
- **No synthetic / fake timeseries.** Gate on real `multi_ts`: `meta.json` → `integrity_multi_ts` true, many distinct `ts` values, holod RSS/CPU from live containers.
- **No FRR-as-main-conclusion** in Phase 1; FRR stack-B scripts stay out of the critical path.
- Protect other labs: never destroy yqh116 / 103 / 135 state.
- Do not push upstream `holo-routing/holo`; fork push only.

## Quick start (independent lab)

### 1. Generate configs (once per lab)

```bash
export YQH157_LAB=/home/cnic/work/yqh184-profiling-repro
export EXPCTL=/home/cnic/work/smu/build/linux/arm64/release/expctl
export EXPCTL_STATE_ROOT="$YQH157_LAB/state"
export HOLO_IMAGE=docker.io/library/holo-bundle:yqh135-ee60831

cd tools/yqh157-profiling/scripts
python3 generate_all.py --lab "$YQH157_LAB"   # proxy configs + manifest
python3 generate_flat.py --lab "$YQH157_LAB"  # flat configs + manifest (does not overwrite proxy)
```

### 2. Timeseries sample (real multi_ts)

Interval **2–5 s** (default 3):

```bash
python3 run_timeseries.py flat  --lab "$YQH157_LAB" --expctl "$EXPCTL" --interval 3
python3 run_timeseries.py proxy --lab "$YQH157_LAB" --expctl "$EXPCTL" --interval 3
# or
python3 run_timeseries.py both --lab "$YQH157_LAB" --expctl "$EXPCTL" --interval 3
# optional: --image "$HOLO_IMAGE"
```

Outputs:

- `$YQH157_LAB/evidence/timeseries/stack_{flat,proxy}/samples.csv`
- `$YQH157_LAB/evidence/timeseries/stack_{flat,proxy}/meta.json`

### 3. Plot

```bash
python3 plot_timeseries.py --lab "$YQH157_LAB"
```

Writes `fig_ts_cpu_flat_vs_proxy`, `fig_ts_rss_flat_vs_proxy`, and per-node variants (png+svg) under `evidence/figures/`.

### 4. One-shot proc sample

```bash
./sample_proc_stack_a.sh
python3 run_stack_a_flat.py --lab "$YQH157_LAB"
```

## Gate checklist (before treating numbers as evidence)

1. `meta.json` has `integrity_multi_ts: true` (or ≥15 unique timestamps).
2. `samples.csv` has many rows × 36 nodes; RSS not constant-zero.
3. Figures regenerated from that CSV (this tree’s `plot_timeseries.py`).
4. Lab is the **intended** independent path; no cross-lab delete; truth lab untouched.
5. `interval_s` in [2, 5].

## Related product code

Profiling feature + `compute_spt` bench live on branch `yqh157-profiling`. This `tools/`
tree is the lab orchestration companion, not the daemon itself.

## Skill

| Install path | Purpose |
|--------------|---------|
| Multica workdir `.grok/skills/yqh157-area-proxy-profiling/SKILL.md` | Discoverable in issue runs |
| `~/.grok/skills/yqh157-area-proxy-profiling/SKILL.md` | Cross-session Grok |
| `tools/yqh157-profiling/skill/SKILL.md` | Versioned with this commit |

Stage A must complete (skill loadable + this tree committed/pushed) before Stage B long runs.

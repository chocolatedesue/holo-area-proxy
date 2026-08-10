# YQH-157 profiling tools

Lab-side scripts for **real** flat vs proxy holod process/timeseries sampling and figures
(Phase 1 of YQH-157). Checked into the product worktree so agents can reuse them.

## Layout

```
tools/yqh157-profiling/
  README.md
  examples/                 # tiny CSV/meta snippets only (not full evidence packs)
  scripts/
    yqh157_paths.py         # shared path resolution
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

## Lab path convention (yqh1)

Default lab root (override with env/flags):

| Variable | Default | Meaning |
|----------|---------|---------|
| `YQH157_LAB` or `YQH157_WD` | `/home/cnic/work/yqh157-real-profiling` | configs, generated, state, evidence |
| `EXPCTL` | `/home/cnic/work/smu/build/linux/arm64/release/expctl` | smu expctl binary |
| `EXPCTL_STATE_ROOT` | `$YQH157_LAB/state` | expctl state (also set by runners) |
| `CRUN_ROOT` | auto under state | optional override for resample |

**Do not** point these at yqh116 / yqh103 / yqh135 labs. **Do not** `expctl delete` or destroy those experiments.

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

- **No synthetic / fake timeseries.** Gate on real `multi_ts`: `meta.json` → `integrity_multi_ts` true, many distinct `ts` values, holod RSS/CPU from live containers.
- **No FRR-as-main-conclusion** in Phase 1; FRR stack-B scripts stay out of the critical path.
- Protect other labs: never destroy yqh116 / 103 / 135 state.

## Quick start

### 1. Generate configs (once per lab)

```bash
export YQH157_LAB=/home/cnic/work/yqh157-real-profiling
export EXPCTL=/home/cnic/work/smu/build/linux/arm64/release/expctl

cd tools/yqh157-profiling/scripts
python3 generate_all.py          # proxy configs + manifest
python3 generate_flat.py         # flat configs + manifest (does not overwrite proxy)
```

### 2. Timeseries sample (real multi_ts)

```bash
# one stack or both (long-running; needs sudo + lab)
python3 run_timeseries.py flat
python3 run_timeseries.py proxy
# or
python3 run_timeseries.py both --lab "$YQH157_LAB" --expctl "$EXPCTL"
```

Outputs:

- `$YQH157_LAB/evidence/timeseries/stack_{flat,proxy}/samples.csv`
- `$YQH157_LAB/evidence/timeseries/stack_{flat,proxy}/meta.json`

### 3. Plot

```bash
python3 plot_timeseries.py --lab "$YQH157_LAB"
# or explicit:
python3 plot_timeseries.py \
  --ts-dir "$YQH157_LAB/evidence/timeseries" \
  --out-dir "$YQH157_LAB/evidence/figures"
```

Writes `fig_ts_cpu_flat_vs_proxy`, `fig_ts_rss_flat_vs_proxy`, and per-node variants (png+svg) under `evidence/figures/`.

### 4. One-shot proc sample

```bash
# after a stack is up
./sample_proc_stack_a.sh
# or flat full apply+sample:
python3 run_stack_a_flat.py --lab "$YQH157_LAB"
```

## Gate checklist (before treating numbers as evidence)

1. `meta.json` has `integrity_multi_ts: true` (or ≥15 unique timestamps).
2. `samples.csv` has many rows × 36 nodes; RSS not constant-zero.
3. Figures regenerated from that CSV (this tree’s `plot_timeseries.py`).
4. Lab is yqh157 only; no cross-lab delete.

## Related product code

Profiling feature + `compute_spt` bench live on branch `yqh157-profiling` (baseline commit with feature, e.g. `e54d24e`). This `tools/` tree is the lab orchestration companion, not the daemon itself.

## Skill

Agent skill: Multica workdir `.grok/skills/yqh157-area-proxy-profiling/SKILL.md` (see that file for the step recipe).

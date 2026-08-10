---
name: yqh157-area-proxy-profiling
description: >-
  Reproduce YQH-157 real flat-vs-proxy holod multi_ts CPU/RSS sampling and
  fig_ts plots via yqh2 + ssh yqh1. Use when running multi_ts capture,
  regenerating fig_ts_*, applying yqh157-flat-torus66 / yqh157-torus66,
  wiring tools/yqh157-profiling, or YQH-184 independent repro on
  yqh184-profiling-repro. Do not use for FRR main claims, protected labs,
  or Stage B before Stage A gates pass.
---

# YQH-157 Area Proxy profiling (flat vs proxy)

Agent-operable recipe for **real** holo flat-vs-proxy process/timeseries profiling
(YQH-157 Phase 1 method; YQH-184 independent repro). Scripts live in the product
tree; this skill is the step list with completion criteria.

## When to use

- Re-run **real** flat vs proxy holod CPU/RSS timeseries (`multi_ts`) on yqh1.
- Regenerate `fig_ts_*` from existing `evidence/timeseries/`.
- Independent Stage B repro (YQH-184): new lab + REPRO_REPORT, skill as sole ops guide.
- Wire or re-sync `tools/yqh157-profiling` scripts into a lab workdir.

## When not to use

- FRR stack-B as the **main** performance claim (Phase 1 = holo flat vs proxy only).
- Destroying, deleting, or reusing **yqh116** / **yqh103** / **yqh135** lab state.
- Overwriting YQH-157 truth lab `yqh157-real-profiling` for destructive re-apply.
- Fabricating CSV rows or “demo” timeseries for figures.
- Treating isomorphic / synthetic SPF numbers as SPF main conclusions.
- Pushing to upstream `holo-routing/holo` (fork push only: `chocolatedesue/holo-area-proxy`).
- Overturning YQH-157 measurement conclusions; this skill verifies **reproducibility**.

## Hosts and trees

| Role | Host | Path / value |
|------|------|----------------|
| Coordination / Multica / git | **yqh2** | this workspace; product worktree below |
| Lab (containers, expctl, evidence) | **yqh1** via `ssh yqh1` | lab root only on yqh1 |
| Product worktree (yqh2) | yqh2 | `/home/cnic/work/area-proxy-holo-yqh157-profiling` |
| Branch | — | `yqh157-profiling` |
| Tools commit (baseline) | — | `e7b6c5c` (`tools: add YQH-157 profiling…`); later SHAs OK if documented |
| Fork remote | — | `https://github.com/chocolatedesue/holo-area-proxy.git` |
| Scripts (canonical) | product tree | `tools/yqh157-profiling/scripts/` |
| Tools README | product tree | `tools/yqh157-profiling/README.md` |
| YQH-157 truth evidence (read-only) | yqh1 | `/home/cnic/work/yqh157-real-profiling/evidence/` |

**Rule:** coordinate and edit product git on yqh2; run expctl / podman / sampling on yqh1. Do not mix-write a product tree that lives only on the other host unless skill steps say so.

## Canonical env (Stage B / independent repro — mandatory)

| Variable | Required value (independent lab) | Notes |
|----------|----------------------------------|--------|
| `YQH157_LAB` | **`/home/cnic/work/yqh184-profiling-repro`** | **Forced for YQH-184 Stage B.** Never default to truth lab for destructive apply/delete. |
| `YQH157_WD` | same as `YQH157_LAB` | alias accepted by scripts |
| `EXPCTL` | `/home/cnic/work/smu/build/linux/arm64/release/expctl` | yqh1 arm64 smu build |
| `EXPCTL_STATE_ROOT` | `$YQH157_LAB/state` | **independent state** — isolates delete/get from truth lab Ready stacks |
| `HOLO_IMAGE` | `docker.io/library/holo-bundle:yqh135-ee60831` | confirm `podman image exists` on yqh1 |
| `CRUN_ROOT` | optional under state | only if resample needs override |

**Truth lab (read-only contrast only):** `/home/cnic/work/yqh157-real-profiling/` — may read `evidence/`; do **not** set `YQH157_LAB` or `EXPCTL_STATE_ROOT` there for Stage B re-apply.

Experiment names (same strings; isolation is via **state root** + lab dir):

| Stack | Experiment | Prefix | Area Proxy |
|-------|------------|--------|------------|
| flat  | `yqh157-flat-torus66` | `yqh157f` | **OFF** (`area-proxy.enabled` false / flat generate) |
| proxy | `yqh157-torus66` | `yqh157` | **ON** |

N default: **36** (6×6 torus, three bands). Only dial **area-proxy on/off** between stacks (via `generate_flat` vs `generate_all`); keep topology/N/image comparable.

## Stage gates

| Gate | Meaning | Pass when |
|------|---------|-----------|
| **A1** | Skill discoverable | This `SKILL.md` exists under workdir `.grok/skills/yqh157-area-proxy-profiling/` and/or `~/.grok/skills/yqh157-area-proxy-profiling/` |
| **A2** | Skill operable | This file covers hosts, trees, independent lab, steps + completion, TS 2–5s, plots, REPRO template, OCI/expctl, bans, A/B gates |
| **A3** | Scripts remote-checkable | `tools/yqh157-profiling/` on fork branch `yqh157-profiling`; `git ls-remote` shows commit SHA |
| **B\*** | Independent repro | **Only after A1–A3 pass.** N=36 E2E, RSS/LSDB table, ≥2 TS figures, 简体中文 `REPRO_REPORT.md`, numbers same order of magnitude as YQH-157 or explained |

**Do not start Stage B long runs until A1–A3 pass.**

---

## Steps (Stage B ops — follow in order)

### 0. Preconditions (yqh2 + yqh1)

On **yqh2**:

```bash
hostname   # expect yqh2
test -d /home/cnic/work/area-proxy-holo-yqh157-profiling/tools/yqh157-profiling/scripts
cd /home/cnic/work/area-proxy-holo-yqh157-profiling
git rev-parse --short HEAD   # note SHA (baseline e7b6c5c or later tools commit)
git remote -v                # origin → chocolatedesue/holo-area-proxy
```

On **yqh1** (via ssh):

```bash
ssh yqh1 'test -x /home/cnic/work/smu/build/linux/arm64/release/expctl && \
  sudo podman image exists docker.io/library/holo-bundle:yqh135-ee60831 && \
  python3 -c "import matplotlib,numpy"'
```

Completion:

- [ ] yqh2 product tree + scripts dir exist
- [ ] yqh1: expctl executable, HOLO image present, matplotlib+numpy import OK
- [ ] A1–A3 already PASS (or this run is only finishing Stage A — stop before step 3 apply)

### 1. Create independent lab + sync scripts (yqh1)

```bash
ssh yqh1
export YQH157_LAB=/home/cnic/work/yqh184-profiling-repro
export EXPCTL=/home/cnic/work/smu/build/linux/arm64/release/expctl
export EXPCTL_STATE_ROOT="$YQH157_LAB/state"
export HOLO_IMAGE=docker.io/library/holo-bundle:yqh135-ee60831
mkdir -p "$YQH157_LAB"/{configs,configs_flat,generated,generated_flat,manifest,state,evidence,scripts,generated/proto}

# Product tree is on yqh2 — rsync scripts (yqh1 usually has no product worktree):
#   from yqh2: rsync -a tools/yqh157-profiling/scripts/ yqh1:$YQH157_LAB/scripts/
#   from yqh2: rsync -a proto/ yqh1:$YQH157_LAB/generated/proto/
SCRIPTS="$YQH157_LAB/scripts"
cd "$SCRIPTS"
test -f "$YQH157_LAB/generated/proto/holo.proto"   # required for YANG commit (grpcurl -import-path)
```

**Gotchas (Stage B verified):**

1. **Proto:** without `$YQH157_LAB/generated/proto/holo.proto`, every commit fails (`holo.proto does not reside in any import path`).
2. **Mount paths:** manifests must use **absolute** lab paths for binds. Relative `./scripts` resolves under `manifest/` and breaks apply.
3. **`/var/log`:** holod needs a **writable** `/var/log` bind (`generated/varlog/<node>`). Missing it → RO rootfs panic, grpc 0/36. `generate_flat` already does this; `generate_all` must too (fixed in tools ≥ `2015518`).

Completion:

- [ ] `test -d "$YQH157_LAB" && test -d "$EXPCTL_STATE_ROOT"`
- [ ] `YQH157_LAB` is **not** `yqh157-real-profiling`
- [ ] `test -f "$SCRIPTS/run_timeseries.py"`
- [ ] `test -f "$YQH157_LAB/generated/proto/holo.proto"`

### 2. Generate configs (flat OFF / proxy ON)

```bash
cd "$SCRIPTS"
export YQH157_LAB EXPCTL EXPCTL_STATE_ROOT HOLO_IMAGE
python3 generate_all.py --lab "$YQH157_LAB"    # proxy ON
python3 generate_flat.py --lab "$YQH157_LAB"   # flat OFF; does not clobber proxy configs
```

**Dial semantics:** only Area Proxy enablement differs (`generate_flat` vs `generate_all`). Same N=36 torus, same image tag unless you intentionally change `HOLO_IMAGE` for both.

Completion:

- [ ] `$YQH157_LAB/manifest/yqh157-flat-torus66.yaml` exists
- [ ] `$YQH157_LAB/manifest/yqh157-torus66.yaml` exists
- [ ] configs under `configs_flat/` and `configs/` present

### 3. Real timeseries sample (apply → commit → converge → sample → delete own exp)

Interval **2–5 s** (default **3**). Long-running; needs sudo on yqh1.

```bash
# one stack or both
python3 run_timeseries.py flat  --lab "$YQH157_LAB" --expctl "$EXPCTL" --interval 3
python3 run_timeseries.py proxy --lab "$YQH157_LAB" --expctl "$EXPCTL" --interval 3
# or: python3 run_timeseries.py both --lab "$YQH157_LAB" --expctl "$EXPCTL" --interval 3
```

What the runner does (do not skip integrity):

1. Ensures HOLO image present.
2. `expctl get` under **this** `EXPCTL_STATE_ROOT` only; may `delete` leftover **same experiment name** in **this** state root (not truth lab).
3. Starts multi_ts sampler; `expctl apply` → YANG/commit path → wait converge → steady window → stop.
4. Writes `samples.csv` + `meta.json`; may delete own experiment at end (see runner).

Completion (each stack):

- [ ] Exit code 0 (converge OK; non-zero → stop and diagnose, do not plot as gold)
- [ ] `$YQH157_LAB/evidence/timeseries/stack_{flat,proxy}/{samples.csv,meta.json}`
- [ ] `meta.json`: `integrity_multi_ts` **true** (or ≥15 unique `ts`)
- [ ] `node_count` = 36; RSS not all zeros
- [ ] `interval_s` in **[2, 5]**

### 4. Plot (≥2 main figures)

```bash
python3 plot_timeseries.py --lab "$YQH157_LAB"
```

Completion:

- [ ] exit 0
- [ ] `$YQH157_LAB/evidence/figures/fig_ts_cpu_flat_vs_proxy.png` (+ `.svg`)
- [ ] `$YQH157_LAB/evidence/figures/fig_ts_rss_flat_vs_proxy.png` (+ `.svg`)

### 5. Steady RSS / LSDB table (for REPRO_REPORT)

From `meta.json` / end-of-run samples and any gold/proc helpers:

```bash
# optional one-shot after stack up (if not using only multi_ts meta):
# ./sample_proc_stack_a.sh
# python3 run_stack_a_flat.py --lab "$YQH157_LAB"
```

Build a small table: stack × mean/max holod RSS × LSDB-related metric if collected. Prefer numbers from **this** lab’s evidence; contrast YQH-157 truth `evidence/` **read-only**.

Completion:

- [ ] Table has flat and proxy rows with concrete numbers (or explicit “not collected” + reason)
- [ ] No fabricated cells

### 6. Write `REPRO_REPORT.md` (简体中文)

Path: `$YQH157_LAB/evidence/REPRO_REPORT.md` (or issue attachment). Use template below.

Completion:

- [ ] All template sections present
- [ ] SHAs, lab path, image tag recorded
- [ ] Δ vs YQH-157 explained if not same order of magnitude
- [ ] No FRR-as-main-conclusion; no isomorphic-as-SPF-main

### 7. Cleanup (own lab only)

```bash
# Only experiments in THIS state root / THIS lab:
sudo env EXPCTL_STATE_ROOT="$EXPCTL_STATE_ROOT" "$EXPCTL" get -o json
# delete only yqh157-flat-torus66 / yqh157-torus66 if still present AND state root is yqh184 path
# NEVER: destroy/delete yqh116* yqh103* yqh135*
# NEVER: point EXPCTL_STATE_ROOT at yqh157-real-profiling for delete during Stage B
```

Completion:

- [ ] Only own lab experiments touched
- [ ] Truth lab Ready stacks untouched

---

## REPRO_REPORT.md template (简体中文)

```markdown
# YQH-184 / YQH-157 profiling 独立复现报告

## 1. 范围与结论摘要
- 对比：holo flat vs holo proxy（Area Proxy off/on），N=36
- 一句话结论：（RSS/CPU 量级与趋势；是否与 YQH-157 同量级）

## 2. 环境
| 项 | 值 |
|----|-----|
| 协调宿主 | yqh2 |
| lab 宿主 | yqh1 |
| YQH157_LAB | /home/cnic/work/yqh184-profiling-repro |
| EXPCTL_STATE_ROOT | …/state |
| HOLO_IMAGE | docker.io/library/holo-bundle:yqh135-ee60831 |
| 产品树 SHA | （yqh157-profiling 分支） |
| 脚本路径 | tools/yqh157-profiling/scripts |
| 采样 interval_s | 3（2–5） |

## 3. 操作摘要
- generate_flat / generate_all
- run_timeseries flat + proxy
- plot_timeseries
- （可选）稳态 proc/LSDB

## 4. 收敛与完整性
| Stack | converge | integrity_multi_ts | rows / unique ts | interval_s |
|-------|----------|--------------------|------------------|------------|
| flat  |          |                    |                  |            |
| proxy |          |                    |                  |            |

## 5. 稳态 RSS / LSDB 对照表
| Stack | holod RSS (mean/max) | LSDB 相关 | 来源文件 |
|-------|----------------------|-----------|----------|
| flat  |                      |           |          |
| proxy |                      |           |          |

## 6. 时间序列图
- evidence/figures/fig_ts_cpu_flat_vs_proxy.png
- evidence/figures/fig_ts_rss_flat_vs_proxy.png

## 7. 与 YQH-157 真源对照
- 真源只读：/home/cnic/work/yqh157-real-profiling/evidence/
- 同量级？偏差解释？

## 8. 禁止项自检
- [ ] 未 destroy yqh116/103/135
- [ ] 未覆盖真源 lab 做破坏性 re-apply
- [ ] 未以 FRR 作主结论
- [ ] 未编造 CSV
- [ ] 未 push upstream holo-routing/holo

## 9. 证据路径清单
- …
```

---

## OCI / expctl isolation (hard rules)

1. **Image:** `HOLO_IMAGE=docker.io/library/holo-bundle:yqh135-ee60831` unless both stacks intentionally retargeted together.
2. **sudo:** expctl apply/delete/get and podman/crun paths need sudo on yqh1.
3. **State root:** always `EXPCTL_STATE_ROOT=$YQH157_LAB/state` for the active lab. Truth lab and yqh184 lab must not share state root during Stage B.
4. **Delete scope:** only experiment names created by this recipe (`yqh157-flat-torus66`, `yqh157-torus66`) **and** only under the active state root. Prefer `delete` over `destroy`. Never delete `yqh116*` / `yqh103*` / `yqh135*`.
5. **Ready leftovers on truth lab:** leave them; do not “clean up” YQH-157 Ready stacks during Stage A or B.
6. **Scripts source of truth:** in-repo `tools/yqh157-profiling/` (not lab-only FRR/bypass one-offs).

## Guardrails (positive targets)

- Operate only under the **independent** `$YQH157_LAB` + matching `EXPCTL_STATE_ROOT` for Stage B.
- Prefer `run_timeseries.py` multi_ts evidence; single `ps` snapshots are secondary.
- Gate numbers on `meta.json` integrity before quoting in reports.
- Commit scripts under `tools/yqh157-profiling/`; keep large raw `evidence/` out of git (`?? evidence/` must not be `git add`ed).
- Push only `chocolatedesue/holo-area-proxy` branch `yqh157-profiling`.

## Forbidden (checklist)

| Ban | Why |
|-----|-----|
| Stage B before A1–A3 | Hard gate |
| `YQH157_LAB=…/yqh157-real-profiling` for Stage B apply/delete | Destroys truth |
| destroy/delete yqh116 / yqh103 / yqh135 | Protected labs |
| FRR as Phase-1 main claim | Out of scope |
| Fabricate CSV / demo TS | Invalid evidence |
| isomorphic as SPF main conclusion | Wrong claim class |
| push `holo-routing/holo` | Upstream protected |
| `git add` lab `evidence/` into product tree | Repo bloat / secrets risk |

## Reference

- Full flags and layout: `tools/yqh157-profiling/README.md` on branch `yqh157-profiling`.
- Skill install paths: workdir `.grok/skills/yqh157-area-proxy-profiling/SKILL.md`; sync `~/.grok/skills/yqh157-area-proxy-profiling/` for cross-session Grok.
- Optional in-repo copy: `tools/yqh157-profiling/skill/SKILL.md` (versioned with tools commit; does not replace discoverable skill paths).

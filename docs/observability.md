# Passive control-plane observability (JSONL / CSV)

Human requirement (YQH-595 / YQH-603): open a switch, run a scenario, then **only pull log files** for convergence analysis — no GetState polling loop.

## Recommended source tip (YQH-613)

Default entry for new labs, builds, and docs is **`main` at tip >= `4ee13d7`** (full: `4ee13d7a5c9ebfa6342c469ff0738eb4902dbc4a`, YQH-610 squash merge of passive observability). Do **not** treat `feat/yqh603-passive-observability` / `56770f6` as the default checkout after merge.

```bash
git clone https://github.com/chocolatedesue/holo-area-proxy.git
cd holo-area-proxy
git checkout main
git merge-base --is-ancestor 4ee13d7a5c9ebfa6342c469ff0738eb4902dbc4a HEAD \
  && git rev-parse --short HEAD \
  || { echo 'FAIL: HEAD must be main@>=4ee13d7'; exit 1; }
```

## Switch (default **off**)

In `holod.toml`:

```toml
[observability]
  enabled = true
  dir = "/var/log/holo"
  file_prefix = "holod-metrics"
  interval_ms = 1000
  format = "both"          # jsonl | csv | both
  include_process = true   # best-effort RSS kB from /proc/self/status
```

Works together with control-plane-only mode:

```toml
[routing]
  fib_install = false
```

When `enabled = false` (default), no metric files are opened and protocol hot paths skip the observability publish (cheap `Option` check on a shared `Arc`).

## Output files

| File | Content |
|------|---------|
| `{dir}/{file_prefix}.jsonl` | One JSON object per line (JSON Lines) |
| `{dir}/{file_prefix}.csv` | Header row + one sample per line |

On each holod start with observability enabled, files are **truncated** and rewritten (one clean run per process). Configuration lives in `holod.toml` (survives crashes independently of metric files).

## Field table

| Field | Type | Meaning |
|-------|------|---------|
| `ts_unix_ms` | u64 | Sample wall time, Unix epoch milliseconds |
| `ts_rfc3339` | string | Same instant, RFC3339 |
| `system_id` | string? | IS-IS System-ID (`XXXX.XXXX.XXXX`) when configured |
| `instance_name` | string? | IS-IS instance name |
| `hostname` | string? | Best-effort hostname if published |
| `fib_install` | bool | `[routing].fib_install` at start |
| `lsdb_l1_lsp` | u64 | Non-expired L1 LSP count |
| `lsdb_l2_lsp` | u64 | Non-expired L2 LSP count |
| `lsdb_lsp_total` | u64 | L1+L2 |
| `lsdb_fp_l1` / `lsdb_fp_l2` | u64 | LSDB fingerprint (change detector) |
| `spf_runs_l1` / `spf_runs_l2` | u64 | Cumulative SPF completions |
| `last_spf_us_l1` / `last_spf_us_l2` | u64 | Last SPF duration (µs) |
| `rib_ipv4_active` / `rib_ipv6_active` / `rib_mpls_entries` | u64 | In-process RIB sizes |
| `fib_ip_installs` / `fib_ip_installs_skipped` | u64 | Netlink IP add enqueued / skipped |
| `fib_ip_uninstalls` / `fib_ip_uninstalls_skipped` | u64 | Netlink IP del |
| `rss_kb` | u64? | Process RSS kB (optional) |

## Analysis workflow (no GetState)

```bash
# 1) Enable switch in holod.toml (see above), start holod / lab
# 2) Run scenario (bring links, wait converge, optional flap)
# 3) Stop or leave running — only read files:

# LSDB total time series from JSONL
jq -r '[.ts_unix_ms, .lsdb_lsp_total, .rib_ipv4_active, .last_spf_us_l1] | @tsv' \
  /var/log/holo/holod-metrics.jsonl

# First time LSDB total stopped increasing (crude converge marker)
jq -r '.lsdb_lsp_total' /var/log/holo/holod-metrics.jsonl | awk 'NR==1{p=$1; next} {if($1==p){c++}else{c=0;p=$1}} c>=3{print NR; exit}'

# CSV path
column -t -s, /var/log/holo/holod-metrics.csv | less -S
```

## Local smoke (no root / no topology)

```bash
cargo test -p holo-utils observability -- --nocapture
bash tools/observability/smoke_offline.sh
```

This writes sample JSONL/CSV under `tools/observability/evidence/` and checks that a multi-sample series is parseable **only from files**.

## Deploy smoke (optional, with holod)

Use `tools/fib-install` patterns: set `observability.enabled=true` and `routing.fib_install=false` in a temp toml, start holod, wait a few intervals, pull `{dir}/{file_prefix}.jsonl`. Assert `fib_install==false` and growing `ts_unix_ms` without calling `GetState`.

## SERNES / metrics_ndjson

This feature is **source-side** on **main@>=4ee13d7** (yqh1 / holo-area-proxy). SERNES container images may still lack the export until rebuilt; that rebuild is a **separate** ticket (YQH-605). Do not pin labs to the pre-merge feature branch for source tip.

## Implementation map

| Piece | Path |
|-------|------|
| Metrics + exporter | `holo-utils/src/observability.rs` |
| Config | `holo-daemon/src/config.rs`, `holod.toml` `[observability]` |
| Hub on shared | `holo-protocol` `InstanceShared.observability` |
| RIB/FIB publish | `holo-routing` on `RibUpdate` |
| LSDB / SPF / sys-id | `holo-isis` lsdb / spf / instance |

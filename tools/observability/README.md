# holod domain metrics (NDJSON)

Passive, machine-parseable control-plane metrics for offline analysis
(YQH-595 product follow-up).

## Operator flow

1. Enable in `holod.toml`:

```toml
[observability]
  metrics_ndjson_enabled = true
  metrics_ndjson_path = "/var/log/holod-metrics.jsonl"

[routing]
  fib_install = false   # optional: control-plane only, skip kernel FIB
```

2. Run the topology / experiment as usual.
3. Pull **only** the metrics file (or container bind-mount) and analyse — **no**
   GetState / gRPC polling required for these events.

## Event kinds (`schema` = `holo.metrics.v1`)

| `kind` | When | Key fields |
|--------|------|------------|
| `observability.start` | Sink opens | `path` |
| `isis.spf.finish` | After each SPF run | `instance`, `level`, `spf_type`, `duration_us`, `schedule_to_start_us`, `trigger_lsp_count`, `spf_runs` |
| `routing.rib.batch` | After non-empty RIB update queue drain | `fib_install`, `prefixes_touched`, `rib_ipv4_active`, `ip_installs` / `ip_installs_skipped`, … |

One JSON object per line (NDJSON). Convert subsets to CSV with `jq` if needed:

```bash
# SPF durations
jq -r 'select(.kind=="isis.spf.finish") | [.ts,.instance,.level,.duration_us,.spf_runs] | @csv' \
  holod-metrics.jsonl

# RIB / skip counters over time
jq -r 'select(.kind=="routing.rib.batch") | [.ts,.rib_ipv4_active,.ip_installs_skipped,.fib_install] | @csv' \
  holod-metrics.jsonl
```

## Default off

When `metrics_ndjson_enabled = false` (default), emit paths are cheap no-ops
(`is_enabled()` false). No extra files, no format changes to normal logs.

## Not the same as

| Feature | Purpose |
|---------|---------|
| `[logging] style = "json"` | Generic tracing lines (noisy, not domain schema) |
| `[event_recorder]` | Protocol instance message replay for bug reports |
| GetState `/holo-routing:fib` | Pull counters (active API) |

## Convergence wall-clock

`isis.spf.finish` gives **per-SPF compute time**. Experiment-wide convergence
(first steady RIB size / LSDB) is derived offline: first/last timestamps of
`routing.rib.batch` or `isis.spf.finish` series for your definition.

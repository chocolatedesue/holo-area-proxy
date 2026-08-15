# IMPLEMENTATION_REPORT — YQH-603 Passive holod observability

## Summary

Implemented **default-off** passive metrics export: when `[observability] enabled = true`, holod samples control-plane counters on a fixed interval and writes **JSON Lines** and/or **CSV**. Analysis workflow is **open switch → run → read files only** (no GetState loop). Compatible with `fib_install = false`.

## Deliverables

| Item | Location |
|------|----------|
| Core metrics + exporter + unit test | `holo-utils/src/observability.rs` |
| Config | `holo-daemon` + `holod.toml` `[observability]` |
| Shared hub | `InstanceShared.observability` |
| RIB/FIB publish | `holo-routing` on RibUpdate |
| LSDB/SPF/sys-id | `holo-isis` |
| Docs | `docs/observability.md` |
| Offline smoke + samples | `tools/observability/` |
| Example deploy toml | `tools/observability/deploy/holod-obs-fib-false.toml` |

## Acceptance

- [x] Switch default off; docs field table
- [x] Offline smoke: multi-sample series from files only (`smoke_offline.sh`)
- [x] `fib_install=false` present in sample series + example toml (combinable)
- [x] Reviewer path: `docs/observability.md` + smoke script

## SERNES follow-up

Recommend a separate ticket to rebuild SERNES holod images once this merges, to replace missing `metrics_ndjson` with this export (or both). **Out of scope for this PR.**

## Claim grades

| Claim | Grade |
|-------|-------|
| Code paths implement passive JSONL/CSV | verified (source + unit test) |
| Full multi-node lab on yqh1 this run | not run (offline smoke only) |
| SERNES image ships feature | not done — needs rebuild ticket |

//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//

//! Process-global **domain metrics** sink (NDJSON).
//!
//! When enabled via `holod.toml` `[observability]`, holod writes one JSON object
//! per line to a dedicated file. Operators open the switch, run the experiment,
//! then pull the file for offline analysis — no GetState polling required for
//! these events.
//!
//! Schema: every event includes `"schema":"holo.metrics.v1"` and `"kind"`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::Utc;
use serde_json::{Value, json};
use tracing::warn;

const SCHEMA: &str = "holo.metrics.v1";

struct Sink {
    path: PathBuf,
    file: File,
}

static SINK: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();

/// Initialise the global NDJSON sink. Safe to call once at holod startup.
///
/// When `enabled` is false, subsequent [`emit`] calls are no-ops.
pub fn init(enabled: bool, path: impl AsRef<Path>) {
    let path = path.as_ref().to_path_buf();
    let sink = if !enabled {
        None
    } else {
        match open_sink(&path) {
            Ok(file) => {
                let mut s = Sink {
                    path: path.clone(),
                    file,
                };
                // Bootstrap line so the file is non-empty and discoverable.
                let _ = write_line(
                    &mut s,
                    &json!({
                        "schema": SCHEMA,
                        "ts": now_ts(),
                        "kind": "observability.start",
                        "path": path.display().to_string(),
                    }),
                );
                Some(s)
            }
            Err(error) => {
                warn!(
                    %error,
                    path = %path.display(),
                    "metrics NDJSON sink disabled: failed to open file"
                );
                None
            }
        }
    };

    // Allow re-init (tests; production calls once at startup).
    let cell = SINK.get_or_init(|| Mutex::new(None));
    match cell.lock() {
        Ok(mut guard) => *guard = sink,
        Err(poisoned) => {
            *poisoned.into_inner() = sink;
        }
    }
}

/// Whether the sink is active (enabled and successfully opened).
pub fn is_enabled() -> bool {
    SINK.get()
        .and_then(|m| m.lock().ok())
        .map(|g| g.is_some())
        .unwrap_or(false)
}

/// Emit one domain event as a single NDJSON line.
///
/// Injects `schema` and `ts` when missing. No-op when the sink is off.
pub fn emit(mut event: Value) {
    let Some(lock) = SINK.get() else {
        return;
    };
    let Ok(mut guard) = lock.lock() else {
        return;
    };
    let Some(sink) = guard.as_mut() else {
        return;
    };

    if let Some(obj) = event.as_object_mut() {
        obj.entry("schema")
            .or_insert_with(|| Value::String(SCHEMA.to_owned()));
        obj.entry("ts")
            .or_insert_with(|| Value::String(now_ts()));
    }

    if let Err(error) = write_line(sink, &event) {
        warn!(
            %error,
            path = %sink.path.display(),
            "failed to write metrics NDJSON line"
        );
    }
}

/// Convenience: SPF finish event (IS-IS control-plane timing).
pub fn emit_isis_spf_finish(
    instance: &str,
    level: &str,
    spf_type: &str,
    duration_us: u64,
    schedule_to_start_us: Option<u64>,
    trigger_lsp_count: usize,
    spf_runs: u32,
) {
    emit(json!({
        "kind": "isis.spf.finish",
        "protocol": "isis",
        "instance": instance,
        "level": level,
        "spf_type": spf_type,
        "duration_us": duration_us,
        "schedule_to_start_us": schedule_to_start_us,
        "trigger_lsp_count": trigger_lsp_count,
        "spf_runs": spf_runs,
    }));
}

/// Convenience: end-of-batch RIB / FIB counters (after processing update queue).
pub fn emit_routing_rib_batch(
    fib_install: bool,
    prefixes_touched: usize,
    mpls_touched: usize,
    rib_ipv4_active: u64,
    rib_ipv6_active: u64,
    rib_mpls_entries: u64,
    ip_installs: u64,
    ip_installs_skipped: u64,
    ip_uninstalls: u64,
    ip_uninstalls_skipped: u64,
    mpls_installs: u64,
    mpls_installs_skipped: u64,
    mpls_uninstalls: u64,
    mpls_uninstalls_skipped: u64,
) {
    if prefixes_touched == 0 && mpls_touched == 0 {
        return;
    }
    emit(json!({
        "kind": "routing.rib.batch",
        "protocol": "routing",
        "fib_install": fib_install,
        "prefixes_touched": prefixes_touched,
        "mpls_touched": mpls_touched,
        "rib_ipv4_active": rib_ipv4_active,
        "rib_ipv6_active": rib_ipv6_active,
        "rib_mpls_entries": rib_mpls_entries,
        "ip_installs": ip_installs,
        "ip_installs_skipped": ip_installs_skipped,
        "ip_uninstalls": ip_uninstalls,
        "ip_uninstalls_skipped": ip_uninstalls_skipped,
        "mpls_installs": mpls_installs,
        "mpls_installs_skipped": mpls_installs_skipped,
        "mpls_uninstalls": mpls_uninstalls,
        "mpls_uninstalls_skipped": mpls_uninstalls_skipped,
    }));
}

fn open_sink(path: &Path) -> std::io::Result<File> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
}

fn write_line(sink: &mut Sink, event: &Value) -> std::io::Result<()> {
    let line = serde_json::to_string(event)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(sink.file, "{line}")?;
    sink.file.flush()?;
    Ok(())
}

fn now_ts() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::sync::Mutex;

    // Serialise tests that touch the global sink.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn disabled_sink_is_noop() {
        let _g = TEST_LOCK.lock().unwrap();
        let path = tempfile_dir().join("off.jsonl");
        init(false, &path);
        assert!(!is_enabled());
        emit(json!({"kind": "test"}));
        assert!(!path.exists() || std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) == 0);
    }

    #[test]
    fn enabled_writes_ndjson_lines() {
        let _g = TEST_LOCK.lock().unwrap();
        let dir = tempfile_dir();
        let path = dir.join("metrics.jsonl");
        init(true, &path);
        assert!(is_enabled());
        emit_isis_spf_finish("rt1", "L1", "full", 42, Some(10), 2, 1);
        emit_routing_rib_batch(
            false, 3, 0, 10, 2, 0, 0, 3, 0, 0, 0, 0, 0, 0,
        );

        let f = File::open(&path).expect("open metrics");
        let lines: Vec<String> = BufReader::new(f)
            .lines()
            .map(|l| l.expect("line"))
            .collect();
        assert!(lines.len() >= 3, "expected start + 2 events, got {lines:?}");
        let start: Value = serde_json::from_str(&lines[0]).unwrap();
        assert_eq!(start["kind"], "observability.start");
        assert_eq!(start["schema"], SCHEMA);
        let spf: Value = serde_json::from_str(&lines[1]).unwrap();
        assert_eq!(spf["kind"], "isis.spf.finish");
        assert_eq!(spf["duration_us"], 42);
        let rib: Value = serde_json::from_str(&lines[2]).unwrap();
        assert_eq!(rib["kind"], "routing.rib.batch");
        assert_eq!(rib["fib_install"], false);
        assert_eq!(rib["ip_installs_skipped"], 3);

        // Leave sink off so other crates' tests are unaffected if they share process.
        init(false, dir.join("off.jsonl"));
    }

    fn tempfile_dir() -> PathBuf {
        let mut d = std::env::temp_dir();
        d.push(format!(
            "holo-metrics-test-{}-{}",
            std::process::id(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }
}

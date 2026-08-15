//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//
// Passive control-plane metrics export (JSON Lines and/or CSV).
// Default off. When enabled, a background task samples atomics on a fixed
// interval and appends one row per sample — no GetState polling required.
//

use std::fs::{File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tracing::{info, warn};

/// holod.toml `[observability]` section.
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Master switch (default false — zero overhead when off).
    pub enabled: bool,
    /// Directory for metric files (must be writable by holod user).
    pub dir: String,
    /// Basename without extension. Writes `{file_prefix}.jsonl` / `.csv`.
    pub file_prefix: String,
    /// Sample period in milliseconds (minimum 100).
    pub interval_ms: u64,
    /// Output format: `jsonl`, `csv`, or `both`.
    pub format: Format,
    /// Best-effort process RSS (kB) from `/proc/self/status` on Linux.
    pub include_process: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Format {
    Jsonl,
    Csv,
    #[default]
    Both,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            enabled: false,
            dir: "/var/log/holo".to_owned(),
            file_prefix: "holod-metrics".to_owned(),
            interval_ms: 1000,
            format: Format::Both,
            include_process: true,
        }
    }
}

/// Process-wide counters and identity for passive export.
///
/// Hot paths only touch atomics / short mutexes when observability is enabled
/// and a hub is installed on [`crate` consumers via shared Arc].
#[derive(Debug, Default)]
pub struct Metrics {
    pub system_id: Mutex<Option<String>>,
    pub instance_name: Mutex<Option<String>>,
    pub hostname: Mutex<Option<String>>,

    pub fib_install: AtomicBool,

    pub lsdb_l1_lsp: AtomicU64,
    pub lsdb_l2_lsp: AtomicU64,
    pub lsdb_fp_l1: AtomicU64,
    pub lsdb_fp_l2: AtomicU64,

    pub spf_runs_l1: AtomicU64,
    pub spf_runs_l2: AtomicU64,
    pub last_spf_us_l1: AtomicU64,
    pub last_spf_us_l2: AtomicU64,

    pub rib_ipv4_active: AtomicU64,
    pub rib_ipv6_active: AtomicU64,
    pub rib_mpls_entries: AtomicU64,

    pub fib_ip_installs: AtomicU64,
    pub fib_ip_installs_skipped: AtomicU64,
    pub fib_ip_uninstalls: AtomicU64,
    pub fib_ip_uninstalls_skipped: AtomicU64,
}

/// One sample row written to disk.
#[derive(Clone, Debug, Serialize)]
pub struct Sample {
    pub ts_unix_ms: u64,
    pub ts_rfc3339: String,
    pub system_id: Option<String>,
    pub instance_name: Option<String>,
    pub hostname: Option<String>,
    pub fib_install: bool,
    pub lsdb_l1_lsp: u64,
    pub lsdb_l2_lsp: u64,
    pub lsdb_lsp_total: u64,
    pub lsdb_fp_l1: u64,
    pub lsdb_fp_l2: u64,
    pub spf_runs_l1: u64,
    pub spf_runs_l2: u64,
    pub last_spf_us_l1: u64,
    pub last_spf_us_l2: u64,
    pub rib_ipv4_active: u64,
    pub rib_ipv6_active: u64,
    pub rib_mpls_entries: u64,
    pub fib_ip_installs: u64,
    pub fib_ip_installs_skipped: u64,
    pub fib_ip_uninstalls: u64,
    pub fib_ip_uninstalls_skipped: u64,
    /// Best-effort RSS in kB; null/omitted when unavailable or disabled.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rss_kb: Option<u64>,
}

impl Metrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_system_id(&self, id: impl Into<String>) {
        if let Ok(mut g) = self.system_id.lock() {
            *g = Some(id.into());
        }
    }

    pub fn set_instance_name(&self, name: impl Into<String>) {
        if let Ok(mut g) = self.instance_name.lock() {
            *g = Some(name.into());
        }
    }

    pub fn set_hostname(&self, name: impl Into<String>) {
        if let Ok(mut g) = self.hostname.lock() {
            *g = Some(name.into());
        }
    }

    pub fn set_fib_install(&self, enabled: bool) {
        self.fib_install.store(enabled, Ordering::Relaxed);
    }

    /// Publish absolute LSDB LSP count + fingerprint for L1 (1) or L2 (2).
    pub fn set_lsdb(&self, level: u8, lsp_count: u64, fingerprint: u64) {
        match level {
            1 => {
                self.lsdb_l1_lsp.store(lsp_count, Ordering::Relaxed);
                self.lsdb_fp_l1.store(fingerprint, Ordering::Relaxed);
            }
            2 => {
                self.lsdb_l2_lsp.store(lsp_count, Ordering::Relaxed);
                self.lsdb_fp_l2.store(fingerprint, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn record_spf(&self, level: u8, duration_us: u64) {
        match level {
            1 => {
                self.spf_runs_l1.fetch_add(1, Ordering::Relaxed);
                self.last_spf_us_l1.store(duration_us, Ordering::Relaxed);
            }
            2 => {
                self.spf_runs_l2.fetch_add(1, Ordering::Relaxed);
                self.last_spf_us_l2.store(duration_us, Ordering::Relaxed);
            }
            _ => {}
        }
    }

    pub fn set_rib_sizes(&self, v4: u64, v6: u64, mpls: u64) {
        self.rib_ipv4_active.store(v4, Ordering::Relaxed);
        self.rib_ipv6_active.store(v6, Ordering::Relaxed);
        self.rib_mpls_entries.store(mpls, Ordering::Relaxed);
    }

    pub fn set_fib_counters(
        &self,
        ip_installs: u64,
        ip_installs_skipped: u64,
        ip_uninstalls: u64,
        ip_uninstalls_skipped: u64,
    ) {
        self.fib_ip_installs.store(ip_installs, Ordering::Relaxed);
        self.fib_ip_installs_skipped
            .store(ip_installs_skipped, Ordering::Relaxed);
        self.fib_ip_uninstalls
            .store(ip_uninstalls, Ordering::Relaxed);
        self.fib_ip_uninstalls_skipped
            .store(ip_uninstalls_skipped, Ordering::Relaxed);
    }

    pub fn snapshot(&self, include_process: bool) -> Sample {
        let now = SystemTime::now();
        let ts_unix_ms = now
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let ts_rfc3339 = chrono_rfc3339(now);

        let lsdb_l1 = self.lsdb_l1_lsp.load(Ordering::Relaxed);
        let lsdb_l2 = self.lsdb_l2_lsp.load(Ordering::Relaxed);

        Sample {
            ts_unix_ms,
            ts_rfc3339,
            system_id: self.system_id.lock().ok().and_then(|g| g.clone()),
            instance_name: self
                .instance_name
                .lock()
                .ok()
                .and_then(|g| g.clone()),
            hostname: self.hostname.lock().ok().and_then(|g| g.clone()),
            fib_install: self.fib_install.load(Ordering::Relaxed),
            lsdb_l1_lsp: lsdb_l1,
            lsdb_l2_lsp: lsdb_l2,
            lsdb_lsp_total: lsdb_l1.saturating_add(lsdb_l2),
            lsdb_fp_l1: self.lsdb_fp_l1.load(Ordering::Relaxed),
            lsdb_fp_l2: self.lsdb_fp_l2.load(Ordering::Relaxed),
            spf_runs_l1: self.spf_runs_l1.load(Ordering::Relaxed),
            spf_runs_l2: self.spf_runs_l2.load(Ordering::Relaxed),
            last_spf_us_l1: self.last_spf_us_l1.load(Ordering::Relaxed),
            last_spf_us_l2: self.last_spf_us_l2.load(Ordering::Relaxed),
            rib_ipv4_active: self.rib_ipv4_active.load(Ordering::Relaxed),
            rib_ipv6_active: self.rib_ipv6_active.load(Ordering::Relaxed),
            rib_mpls_entries: self.rib_mpls_entries.load(Ordering::Relaxed),
            fib_ip_installs: self.fib_ip_installs.load(Ordering::Relaxed),
            fib_ip_installs_skipped: self
                .fib_ip_installs_skipped
                .load(Ordering::Relaxed),
            fib_ip_uninstalls: self.fib_ip_uninstalls.load(Ordering::Relaxed),
            fib_ip_uninstalls_skipped: self
                .fib_ip_uninstalls_skipped
                .load(Ordering::Relaxed),
            rss_kb: if include_process { read_rss_kb() } else { None },
        }
    }
}

fn chrono_rfc3339(now: SystemTime) -> String {
    // Prefer chrono when available via workspace; format manually if needed.
    use chrono::{DateTime, Utc};
    let dt: DateTime<Utc> = now.into();
    dt.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// Best-effort Linux RSS from `/proc/self/status` (kB).
pub fn read_rss_kb() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb);
        }
    }
    None
}

const CSV_HEADER: &str = "ts_unix_ms,ts_rfc3339,system_id,instance_name,hostname,fib_install,lsdb_l1_lsp,lsdb_l2_lsp,lsdb_lsp_total,lsdb_fp_l1,lsdb_fp_l2,spf_runs_l1,spf_runs_l2,last_spf_us_l1,last_spf_us_l2,rib_ipv4_active,rib_ipv6_active,rib_mpls_entries,fib_ip_installs,fib_ip_installs_skipped,fib_ip_uninstalls,fib_ip_uninstalls_skipped,rss_kb";

impl Sample {
    pub fn to_csv_row(&self) -> String {
        fn esc(s: &str) -> String {
            if s.contains(',') || s.contains('"') || s.contains('\n') {
                format!("\"{}\"", s.replace('"', "\"\""))
            } else {
                s.to_owned()
            }
        }
        let sid = self.system_id.as_deref().unwrap_or("");
        let iname = self.instance_name.as_deref().unwrap_or("");
        let host = self.hostname.as_deref().unwrap_or("");
        format!(
            "{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
            self.ts_unix_ms,
            esc(&self.ts_rfc3339),
            esc(sid),
            esc(iname),
            esc(host),
            self.fib_install,
            self.lsdb_l1_lsp,
            self.lsdb_l2_lsp,
            self.lsdb_lsp_total,
            self.lsdb_fp_l1,
            self.lsdb_fp_l2,
            self.spf_runs_l1,
            self.spf_runs_l2,
            self.last_spf_us_l1,
            self.last_spf_us_l2,
            self.rib_ipv4_active,
            self.rib_ipv6_active,
            self.rib_mpls_entries,
            self.fib_ip_installs,
            self.fib_ip_installs_skipped,
            self.fib_ip_uninstalls,
            self.fib_ip_uninstalls_skipped,
            self.rss_kb.map(|v| v.to_string()).unwrap_or_default()
        )
    }
}

/// Opens metric files and writes samples.
pub struct Exporter {
    jsonl: Option<BufWriter<File>>,
    csv: Option<BufWriter<File>>,
    jsonl_path: Option<PathBuf>,
    csv_path: Option<PathBuf>,
}

impl Exporter {
    pub fn open(config: &Config) -> std::io::Result<Self> {
        std::fs::create_dir_all(&config.dir)?;
        let prefix = Path::new(&config.dir).join(&config.file_prefix);

        let mut jsonl = None;
        let mut jsonl_path = None;
        let mut csv = None;
        let mut csv_path = None;

        let want_jsonl = matches!(config.format, Format::Jsonl | Format::Both);
        let want_csv = matches!(config.format, Format::Csv | Format::Both);

        if want_jsonl {
            let path = prefix.with_extension("jsonl");
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;
            jsonl = Some(BufWriter::new(file));
            jsonl_path = Some(path);
        }
        if want_csv {
            let path = prefix.with_extension("csv");
            let file = OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .open(&path)?;
            let mut w = BufWriter::new(file);
            writeln!(w, "{CSV_HEADER}")?;
            w.flush()?;
            csv = Some(w);
            csv_path = Some(path);
        }

        Ok(Self {
            jsonl,
            csv,
            jsonl_path,
            csv_path,
        })
    }

    pub fn paths(&self) -> (Option<&Path>, Option<&Path>) {
        (self.jsonl_path.as_deref(), self.csv_path.as_deref())
    }

    pub fn write_sample(&mut self, sample: &Sample) -> std::io::Result<()> {
        if let Some(w) = self.jsonl.as_mut() {
            let line =
                serde_json::to_string(sample).map_err(std::io::Error::other)?;
            writeln!(w, "{line}")?;
            w.flush()?;
        }
        if let Some(w) = self.csv.as_mut() {
            writeln!(w, "{}", sample.to_csv_row())?;
            w.flush()?;
        }
        Ok(())
    }
}

/// Spawn detached background exporter when `config.enabled`.
///
/// Returns `Some(metrics)` hub for protocols to publish into; `None` when off.
pub fn maybe_start(config: &Config) -> Option<Arc<Metrics>> {
    if !config.enabled {
        return None;
    }

    let metrics = Arc::new(Metrics::new());
    let metrics_bg = metrics.clone();
    let config = config.clone();
    let interval = Duration::from_millis(config.interval_ms.max(100));

    // Detached so it lives for the process lifetime (handle drop would cancel).
    let mut task = crate::task::Task::spawn(async move {
        let mut exporter = match Exporter::open(&config) {
            Ok(e) => e,
            Err(error) => {
                warn!(%error, "observability: failed to open metric files");
                return;
            }
        };
        let (jp, cp) = exporter.paths();
        info!(
            jsonl = ?jp.map(|p| p.display().to_string()),
            csv = ?cp.map(|p| p.display().to_string()),
            interval_ms = interval.as_millis() as u64,
            "observability: passive metrics export enabled"
        );

        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // First tick completes immediately — write an initial baseline row.
        loop {
            ticker.tick().await;
            let sample = metrics_bg.snapshot(config.include_process);
            if let Err(error) = exporter.write_sample(&sample) {
                warn!(%error, "observability: write failed");
            }
        }
    });
    task.detach();

    Some(metrics)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn csv_and_jsonl_roundtrip_fields() {
        let dir = std::env::temp_dir()
            .join(format!("holo-obs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        let cfg = Config {
            enabled: true,
            dir: dir.to_string_lossy().into_owned(),
            file_prefix: "smoke".into(),
            interval_ms: 100,
            format: Format::Both,
            include_process: false,
        };

        let metrics = Metrics::new();
        metrics.set_system_id("0100.0100.0001");
        metrics.set_instance_name("test");
        metrics.set_fib_install(false);
        metrics.set_lsdb(1, 10, 0xabc);
        metrics.set_lsdb(2, 5, 0xdef);
        metrics.record_spf(1, 1234);
        metrics.set_rib_sizes(7, 3, 0);
        metrics.set_fib_counters(0, 2, 0, 1);

        let mut exp = Exporter::open(&cfg).unwrap();
        let s1 = metrics.snapshot(false);
        exp.write_sample(&s1).unwrap();
        metrics.set_lsdb(1, 12, 0xabcd);
        let s2 = metrics.snapshot(false);
        exp.write_sample(&s2).unwrap();
        drop(exp);

        let jsonl_path = dir.join("smoke.jsonl");
        let csv_path = dir.join("smoke.csv");
        let jsonl = fs::read_to_string(&jsonl_path).unwrap();
        let csv = fs::read_to_string(&csv_path).unwrap();

        let lines: Vec<_> = jsonl.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2);
        let v: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v["lsdb_l1_lsp"], 12);
        assert_eq!(v["lsdb_lsp_total"], 17);
        assert_eq!(v["fib_install"], false);
        assert_eq!(v["spf_runs_l1"], 1);
        assert_eq!(v["last_spf_us_l1"], 1234);
        assert_eq!(v["rib_ipv4_active"], 7);
        assert_eq!(v["fib_ip_installs_skipped"], 2);
        assert_eq!(v["system_id"], "0100.0100.0001");

        let mut csv_lines = csv.lines();
        assert_eq!(csv_lines.next().unwrap(), CSV_HEADER);
        let row2 = csv_lines.nth(1).unwrap();
        assert!(row2.contains("0100.0100.0001"));
        assert!(row2.contains(",12,"));
        assert!(row2.contains("false"));

        // Convergence series: only files needed — lsdb grew 10→12.
        let totals: Vec<u64> = lines
            .iter()
            .map(|l| {
                let v: serde_json::Value = serde_json::from_str(l).unwrap();
                v["lsdb_lsp_total"].as_u64().unwrap()
            })
            .collect();
        assert_eq!(totals, vec![15, 17]);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_config_disabled() {
        let c = Config::default();
        assert!(!c.enabled);
        assert_eq!(c.interval_ms, 1000);
        assert!(matches!(c.format, Format::Both));
    }
}

//! Real `compute_spt` microbench + CSV export (YQH-157 / YQH-162).
//!
//! ```text
//! cargo bench -p holo-isis --bench spf_spt --features profiling,testing -- --noplot
//! ```
//!
//! Writes `scale_real_spf.csv` / `.json` under CARGO_TARGET_TMPDIR or cwd/evidence.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use criterion::{Criterion, criterion_group, criterion_main};
use holo_isis::spf_fixture::{self, MeasureRow};

fn evidence_dir() -> PathBuf {
    if let Ok(p) = std::env::var("YQH157_EVIDENCE") {
        return PathBuf::from(p);
    }
    if let Ok(p) = std::env::var("CARGO_TARGET_TMPDIR") {
        let d = PathBuf::from(p).join("yqh157_spf");
        let _ = fs::create_dir_all(&d);
        return d;
    }
    let d = PathBuf::from("evidence");
    let _ = fs::create_dir_all(&d);
    d
}

fn export_csv(rows: &[MeasureRow]) {
    let dir = evidence_dir();
    let _ = fs::create_dir_all(&dir);
    let csv_path = dir.join("scale_real_spf.csv");
    let json_path = dir.join("scale_real_spf.json");

    let mut csv = String::from(
        "n,n_lsps,iters,flat_ns_per_call,proxy_ns_per_call,flat_total_ns,proxy_total_ns,flat_verts,proxy_verts,spf_method,ts_unix\n",
    );
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut json_items = Vec::new();
    for r in rows {
        csv.push_str(&format!(
            "{},{},{},{:.3},{:.3},{},{},{},{},{},{}\n",
            r.n,
            r.n_lsps,
            r.iters,
            r.flat_ns_per_call(),
            r.proxy_ns_per_call(),
            r.flat_total_ns,
            r.proxy_total_ns,
            r.flat_verts,
            r.proxy_verts,
            r.spf_method,
            ts,
        ));
        json_items.push(format!(
            "{{\"n\":{},\"n_lsps\":{},\"iters\":{},\"flat_ns_per_call\":{:.3},\"proxy_ns_per_call\":{:.3},\"flat_total_ns\":{},\"proxy_total_ns\":{},\"flat_verts\":{},\"proxy_verts\":{},\"spf_method\":\"{}\",\"ts_unix\":{}}}",
            r.n,
            r.n_lsps,
            r.iters,
            r.flat_ns_per_call(),
            r.proxy_ns_per_call(),
            r.flat_total_ns,
            r.proxy_total_ns,
            r.flat_verts,
            r.proxy_verts,
            r.spf_method,
            ts,
        ));
    }
    fs::write(&csv_path, csv).expect("write csv");
    fs::write(&json_path, format!("[{}]", json_items.join(","))).expect("write json");
    eprintln!("wrote {} and {}", csv_path.display(), json_path.display());
}

fn criterion_benchmark(c: &mut Criterion) {
    // Deterministic wall-clock export (primary deliverable for YQH-157).
    let sizes = [16usize, 36, 64, 100];
    let iters = 200u32;
    let mut rows = Vec::new();
    for n in sizes {
        let row = spf_fixture::measure_flat_vs_proxy(n, iters);
        eprintln!(
            "N={}: flat={:.0}ns proxy={:.0}ns verts={}/{} method={}",
            row.n,
            row.flat_ns_per_call(),
            row.proxy_ns_per_call(),
            row.flat_verts,
            row.proxy_verts,
            row.spf_method
        );
        assert_eq!(row.spf_method, "real_compute_spt");
        assert!(row.flat_verts >= 3, "SPT too small — fixture broken?");
        rows.push(row);
    }
    export_csv(&rows);

    // Criterion microbench on N=36 flat vs proxy (secondary).
    let mut group = c.benchmark_group("compute_spt_ring36");
    group.bench_function("flat", |b| {
        let mut world = spf_fixture::build_ring_world(36, true);
        world.set_area_proxy(false, None);
        b.iter(|| {
            let (spt, n) = world.run_compute_spt();
            std::hint::black_box((spt, n));
        });
    });
    group.bench_function("proxy", |b| {
        let mut world = spf_fixture::build_ring_world(36, true);
        let proxy = holo_isis::packet::SystemId::from([0, 0, 0, 0, 0, 0xa1]);
        world.set_area_proxy(true, Some(proxy));
        b.iter(|| {
            let (spt, n) = world.run_compute_spt();
            std::hint::black_box((spt, n));
        });
    });
    group.finish();
}

criterion_group!(benches, criterion_benchmark);
criterion_main!(benches);

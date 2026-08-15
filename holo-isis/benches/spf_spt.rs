//! Criterion bench: real `compute_spt` flat vs Area Proxy (YQH-162).
//!
//! Run:
//!   cargo bench -p holo-isis --bench spf_spt --features profiling -- --noplot
//!
//! Optional env:
//!   YQH162_BENCH_OUT=/path/to/dir  — write CSV/JSON samples there

use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use holo_isis::spf_profiling::{
    collect_scale_samples, write_samples_csv, write_samples_json, SpfProfilingHarness,
    SPF_METHOD_REAL,
};

fn bench_flat_vs_proxy(c: &mut Criterion) {
    // Smoke: ensure real path produces a non-trivial SPT.
    {
        let mut h = SpfProfilingHarness::torus(6, 6);
        let flat = h.sample(false);
        let proxy = h.sample(true);
        assert_eq!(flat.spf_method, SPF_METHOD_REAL);
        assert_eq!(proxy.spf_method, SPF_METHOD_REAL);
        assert!(
            flat.spt_vertices >= 36,
            "flat SPT too small: {}",
            flat.spt_vertices
        );
        eprintln!(
            "spf_spt smoke flat verts={} {:.3}us proxy verts={} {:.3}us method={}",
            flat.spt_vertices,
            flat.duration_us,
            proxy.spt_vertices,
            proxy.duration_us,
            SPF_METHOD_REAL
        );
    }

    let mut group = c.benchmark_group("real_compute_spt");
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(8));
    group.warm_up_time(Duration::from_secs(2));

    for &(rows, cols) in &[(4usize, 4), (6, 6), (8, 8)] {
        let n = rows * cols;
        let label_flat = format!("flat_n{n}");
        let label_proxy = format!("proxy_n{n}");

        group.bench_function(&label_flat, |b| {
            let mut h = SpfProfilingHarness::torus(rows, cols);
            b.iter(|| {
                let v = h.run_batch(false, 1);
                black_box(v)
            })
        });
        group.bench_function(&label_proxy, |b| {
            let mut h = SpfProfilingHarness::torus(rows, cols);
            b.iter(|| {
                let v = h.run_batch(true, 1);
                black_box(v)
            })
        });
    }
    group.finish();

    let out_dir = std::env::var("YQH162_BENCH_OUT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/yqh162-spf-spt")
        });
    let _ = std::fs::create_dir_all(&out_dir);
    let sizes = [(4usize, 4), (6, 6), (8, 8), (10, 10)];
    let samples = collect_scale_samples(&sizes, 5);
    let csv = out_dir.join("spf_spt_real_compute_spt.csv");
    let json = out_dir.join("spf_spt_real_compute_spt.json");
    write_samples_csv(&csv, &samples).expect("write csv");
    write_samples_json(&json, &samples).expect("write json");
    eprintln!(
        "wrote {} samples to {} and {}",
        samples.len(),
        csv.display(),
        json.display()
    );
}

criterion_group!(benches, bench_flat_vs_proxy);
criterion_main!(benches);

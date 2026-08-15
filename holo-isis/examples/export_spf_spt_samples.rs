//! One-shot exporter: real compute_spt flat/proxy samples → CSV/JSON
//! cargo run -p holo-isis --example export_spf_spt_samples --features profiling --release -- /out/dir

use std::env;
use std::path::PathBuf;

use holo_isis::spf_profiling::{
    SPF_METHOD_REAL, collect_scale_samples, write_samples_csv,
    write_samples_json,
};

fn main() {
    let out = env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/yqh162-spf-spt"));
    std::fs::create_dir_all(&out).unwrap();
    let sizes = [(4usize, 4), (6, 6), (8, 8), (10, 10)];
    let samples = collect_scale_samples(&sizes, 20);
    assert!(samples.iter().all(|s| s.spf_method == SPF_METHOD_REAL));
    let csv = out.join("spf_spt_real_compute_spt.csv");
    let json = out.join("spf_spt_real_compute_spt.json");
    write_samples_csv(&csv, &samples).unwrap();
    write_samples_json(&json, &samples).unwrap();
    // Summary by (n, mode)
    use std::collections::BTreeMap;
    let mut acc: BTreeMap<(usize, String), Vec<f64>> = BTreeMap::new();
    for s in &samples {
        acc.entry((s.n_nodes, s.mode.clone()))
            .or_default()
            .push(s.duration_us);
    }
    println!("spf_method={}", SPF_METHOD_REAL);
    println!(
        "wrote {} samples -> {} {}",
        samples.len(),
        csv.display(),
        json.display()
    );
    for ((n, mode), xs) in acc {
        let mean = xs.iter().sum::<f64>() / xs.len() as f64;
        let min = xs.iter().cloned().fold(f64::INFINITY, f64::min);
        let max = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        let verts = samples
            .iter()
            .find(|s| s.n_nodes == n && s.mode == mode)
            .map(|s| s.spt_vertices)
            .unwrap();
        println!(
            "n={n} mode={mode} verts={verts} mean_us={mean:.3} min_us={min:.3} max_us={max:.3} reps={}",
            xs.len()
        );
    }
}

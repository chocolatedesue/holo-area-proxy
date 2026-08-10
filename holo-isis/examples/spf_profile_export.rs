//! cargo run -p holo-isis --example spf_profile_export --features profiling,testing --release
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    let out = env::var("YQH157_EVIDENCE").unwrap_or_else(|_| "evidence".into());
    let dir = PathBuf::from(&out);
    fs::create_dir_all(&dir).unwrap();
    let sizes: Vec<usize> = env::var("SIZES")
        .ok()
        .map(|s| s.split(',').filter_map(|x| x.parse().ok()).collect())
        .unwrap_or_else(|| vec![16, 36, 64, 100]);
    let iters: u32 = env::var("ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(200);
    println!("export sizes={sizes:?} iters={iters}");
    let mut csv = String::from(
        "n,n_lsps,iters,flat_ns_per_call,proxy_ns_per_call,flat_total_ns,proxy_total_ns,flat_verts,proxy_verts,spf_method,ts_unix\n",
    );
    let ts = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
    let mut json_items = Vec::new();
    for n in sizes {
        println!("measuring n={n} ...");
        let row = holo_isis::spf_fixture::measure_flat_vs_proxy(n, iters);
        println!(
            "  flat={:.1}ns proxy={:.1}ns verts={}/{} method={}",
            row.flat_ns_per_call(),
            row.proxy_ns_per_call(),
            row.flat_verts,
            row.proxy_verts,
            row.spf_method
        );
        assert_eq!(row.spf_method, "real_compute_spt");
        assert!(row.flat_verts >= 3, "SPT too small n={n} verts={}", row.flat_verts);
        csv.push_str(&format!(
            "{},{},{},{:.3},{:.3},{},{},{},{},{},{}\n",
            row.n, row.n_lsps, row.iters, row.flat_ns_per_call(), row.proxy_ns_per_call(),
            row.flat_total_ns, row.proxy_total_ns, row.flat_verts, row.proxy_verts, row.spf_method, ts
        ));
        json_items.push(format!(
            "{{\"n\":{},\"n_lsps\":{},\"iters\":{},\"flat_ns_per_call\":{:.3},\"proxy_ns_per_call\":{:.3},\"flat_total_ns\":{},\"proxy_total_ns\":{},\"flat_verts\":{},\"proxy_verts\":{},\"spf_method\":\"{}\",\"ts_unix\":{}}}",
            row.n, row.n_lsps, row.iters, row.flat_ns_per_call(), row.proxy_ns_per_call(),
            row.flat_total_ns, row.proxy_total_ns, row.flat_verts, row.proxy_verts, row.spf_method, ts
        ));
    }
    let csv_path = dir.join("scale_real_spf.csv");
    let json_path = dir.join("scale_real_spf.json");
    fs::write(&csv_path, &csv).unwrap();
    fs::write(&json_path, format!("[{}]", json_items.join(","))).unwrap();
    println!("wrote {}", csv_path.display());
    println!("wrote {}", json_path.display());
}

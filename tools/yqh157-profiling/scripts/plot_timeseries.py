#!/usr/bin/env python3
"""Plot CPU/RSS timeseries for flat vs proxy with converge markers.

Defaults:
  input:  $YQH157_LAB/evidence/timeseries/stack_{flat,proxy}/
  output: $YQH157_LAB/evidence/figures/fig_ts_*

Usage:
  YQH157_LAB=/path/to/lab python3 plot_timeseries.py
  python3 plot_timeseries.py --lab /path/to/lab --out-dir ./figures
"""
from __future__ import annotations

import argparse
import csv
import json
import sys
from pathlib import Path

import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
import numpy as np

# allow running from any cwd
sys.path.insert(0, str(Path(__file__).resolve().parent))
from yqh157_paths import evidence_root, lab_root  # noqa: E402


def load_stack(ts_root: Path, name: str):
    d = ts_root / f"stack_{name}"
    meta = json.loads((d / "meta.json").read_text())
    with open(d / "samples.csv") as f:
        rows = list(csv.DictReader(f))
    return meta, rows


def aggregate(rows, t0):
    by_ts = {}
    for r in rows:
        ts = float(r["ts"])
        by_ts.setdefault(ts, []).append(r)
    out = []
    for ts in sorted(by_ts):
        rs = by_ts[ts]
        pcpus = [float(x["pcpu"]) for x in rs if x.get("pcpu") not in ("", None)]
        rsss = [float(x["rss_kb"]) for x in rs if x.get("rss_kb") not in ("", None)]
        if not pcpus and not rsss:
            continue
        out.append(
            {
                "t": ts - t0,
                "ts": ts,
                "pcpu_avg": float(np.mean(pcpus)) if pcpus else np.nan,
                "pcpu_min": float(np.min(pcpus)) if pcpus else np.nan,
                "pcpu_max": float(np.max(pcpus)) if pcpus else np.nan,
                "rss_avg": float(np.mean(rsss)) if rsss else np.nan,
                "rss_min": float(np.min(rsss)) if rsss else np.nan,
                "rss_max": float(np.max(rsss)) if rsss else np.nan,
                "n": len(rs),
            }
        )
    return out


def plot_metric(fig_dir, flat_agg, proxy_agg, flat_meta, proxy_meta, metric, ylabel, out_base):
    fig, ax = plt.subplots(figsize=(10, 5))
    for agg, meta, color, label in [
        (flat_agg, flat_meta, "#1f77b4", "flat"),
        (proxy_agg, proxy_meta, "#d62728", "proxy"),
    ]:
        if not agg:
            continue
        t = [a["t"] for a in agg]
        avg = [a[f"{metric}_avg"] for a in agg]
        mn = [a[f"{metric}_min"] for a in agg]
        mx = [a[f"{metric}_max"] for a in agg]
        ax.plot(t, avg, color=color, label=f"{label} avg", linewidth=1.8)
        ax.fill_between(t, mn, mx, color=color, alpha=0.18, label=f"{label} min-max")
        conv = meta.get("timestamps", {}).get("converge_ts")
        t0 = meta.get("timestamps", {}).get("apply_start") or meta.get("timestamps", {}).get(
            "start"
        )
        if conv and t0:
            ax.axvline(conv - t0, color=color, linestyle="--", alpha=0.7, linewidth=1.2)
    ax.set_xlabel("Time since apply start (s)")
    ax.set_ylabel(ylabel)
    ax.set_title(f"holod {ylabel} vs time (N=36 torus, interval=3s)")
    ax.grid(True, alpha=0.3)
    ax.legend(loc="best", fontsize=8)
    ax.text(
        0.02,
        0.98,
        "dashed vertical = converge_ts per stack",
        transform=ax.transAxes,
        va="top",
        fontsize=8,
        color="gray",
    )
    fig.tight_layout()
    for ext in ("png", "svg"):
        fig.savefig(fig_dir / f"{out_base}.{ext}", dpi=140)
    plt.close(fig)


def plot_per_node(fig_dir, rows, meta, stack, metric, ylabel, out_base, pick=None):
    if pick is None:
        pick = ["r1c1", "r2c2", "r3c3", "r4c4", "r5c5", "r6c6"]
    t0 = meta.get("timestamps", {}).get("apply_start") or meta.get("timestamps", {}).get("start")
    fig, ax = plt.subplots(figsize=(10, 5))
    for node in pick:
        pts = [
            (float(r["ts"]) - t0, float(r[metric]))
            for r in rows
            if r["node"] == node and r.get(metric) not in ("", None)
        ]
        if not pts:
            continue
        pts.sort()
        ax.plot([p[0] for p in pts], [p[1] for p in pts], linewidth=1.0, label=node, alpha=0.85)
    conv = meta.get("timestamps", {}).get("converge_ts")
    if conv and t0:
        ax.axvline(conv - t0, color="black", linestyle="--", alpha=0.6, label="converge")
    ax.set_xlabel("Time since apply start (s)")
    ax.set_ylabel(ylabel)
    ax.set_title(f"{stack}: per-node holod {ylabel}")
    ax.grid(True, alpha=0.3)
    ax.legend(loc="best", fontsize=8, ncol=2)
    fig.tight_layout()
    for ext in ("png", "svg"):
        fig.savefig(fig_dir / f"{out_base}.{ext}", dpi=140)
    plt.close(fig)


def main(argv=None):
    ap = argparse.ArgumentParser(description="Plot YQH-157 flat vs proxy holod timeseries")
    ap.add_argument(
        "--lab",
        default=None,
        help="Lab root (default: $YQH157_LAB or $YQH157_WD or /home/cnic/work/yqh157-real-profiling)",
    )
    ap.add_argument(
        "--evidence",
        default=None,
        help="Evidence root (default: <lab>/evidence)",
    )
    ap.add_argument(
        "--ts-dir",
        default=None,
        help="Timeseries root containing stack_flat/ and stack_proxy/ (default: <evidence>/timeseries)",
    )
    ap.add_argument(
        "--out-dir",
        default=None,
        help="Figure output directory (default: <evidence>/figures)",
    )
    args = ap.parse_args(argv)

    lab = lab_root(args.lab)
    ev = evidence_root(lab, args.evidence)
    ts_root = Path(args.ts_dir).expanduser().resolve() if args.ts_dir else ev / "timeseries"
    fig_dir = Path(args.out_dir).expanduser().resolve() if args.out_dir else ev / "figures"
    fig_dir.mkdir(parents=True, exist_ok=True)

    flat_meta, flat_rows = load_stack(ts_root, "flat")
    proxy_meta, proxy_rows = load_stack(ts_root, "proxy")
    f0 = flat_meta["timestamps"]["apply_start"]
    p0 = proxy_meta["timestamps"]["apply_start"]
    flat_agg = aggregate(flat_rows, f0)
    proxy_agg = aggregate(proxy_rows, p0)

    plot_metric(
        fig_dir,
        flat_agg,
        proxy_agg,
        flat_meta,
        proxy_meta,
        "pcpu",
        "CPU %",
        "fig_ts_cpu_flat_vs_proxy",
    )
    plot_metric(
        fig_dir,
        flat_agg,
        proxy_agg,
        flat_meta,
        proxy_meta,
        "rss",
        "RSS (KB)",
        "fig_ts_rss_flat_vs_proxy",
    )

    plot_per_node(fig_dir, flat_rows, flat_meta, "flat", "pcpu", "CPU %", "fig_ts_cpu_pernode_flat")
    plot_per_node(
        fig_dir, proxy_rows, proxy_meta, "proxy", "pcpu", "CPU %", "fig_ts_cpu_pernode_proxy"
    )
    plot_per_node(
        fig_dir, flat_rows, flat_meta, "flat", "rss_kb", "RSS (KB)", "fig_ts_rss_pernode_flat"
    )
    plot_per_node(
        fig_dir, proxy_rows, proxy_meta, "proxy", "rss_kb", "RSS (KB)", "fig_ts_rss_pernode_proxy"
    )

    for name, agg in [("flat", flat_agg), ("proxy", proxy_agg)]:
        p = ts_root / f"stack_{name}" / "aggregate.csv"
        with open(p, "w", newline="") as f:
            w = csv.DictWriter(
                f,
                fieldnames=[
                    "t_rel_s",
                    "ts",
                    "pcpu_avg",
                    "pcpu_min",
                    "pcpu_max",
                    "rss_avg",
                    "rss_min",
                    "rss_max",
                    "n",
                ],
            )
            w.writeheader()
            for a in agg:
                w.writerow(
                    {
                        "t_rel_s": f"{a['t']:.3f}",
                        "ts": f"{a['ts']:.3f}",
                        "pcpu_avg": a["pcpu_avg"],
                        "pcpu_min": a["pcpu_min"],
                        "pcpu_max": a["pcpu_max"],
                        "rss_avg": a["rss_avg"],
                        "rss_min": a["rss_min"],
                        "rss_max": a["rss_max"],
                        "n": a["n"],
                    }
                )
    print("plots written to", fig_dir)
    print("flat rounds", len(flat_agg), "rows", len(flat_rows))
    print("proxy rounds", len(proxy_agg), "rows", len(proxy_rows))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

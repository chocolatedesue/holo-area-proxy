//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//
// YQH-162 / YQH-157: measurement-only helpers for real compute_spt profiling.
// Enabled solely via `feature = "profiling"`. Not part of the production path.
//

//! Real-`compute_spt` profiling fixtures and runners.
//!
//! Fixture LSPs use the same `Lsp` / TLV types as the protocol
//! (`ext_is_reach`, protocols-supported, optional Area Proxy TLVs). Every
//! exported row is labelled `spf_method=real_compute_spt`.

use std::time::{Duration, Instant};

use holo_protocol::{InstanceChannelsTx, InstanceShared, ProtocolInstance};
use holo_utils::ibus::{self, IbusChannelsRx};
use holo_utils::ip::AddressFamily;
use tokio::sync::mpsc;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::collections::{Arena, Interfaces};
use crate::instance::{Instance, InstanceState, ProtocolInputChannelsRx};
use crate::northbound::configuration::{
    AddressFamilyCfg, InstanceCfg, LevelsCfgWithDefault, MetricType,
};
use crate::packet::iana::{MtId, Nlpid};
use crate::packet::pdu::{Lsp, LspFlags, LspTlvs};
use crate::packet::tlv::{AreaProxyTlv, IsReach, IsReachStlvs};
use crate::packet::{LanId, LevelNumber, LspId, SystemId};
use crate::spf::{self, MetricMode, Spt};
#[cfg(feature = "testing")]
use crate::tasks::messages::ProtocolOutputMsg;

/// Forced label for every exported measurement row from this module.
pub const SPF_METHOD_REAL: &str = "real_compute_spt";

/// One wall-clock sample of a real `compute_spt` invocation.
#[derive(Clone, Debug, serde::Serialize)]
pub struct SpfSptSample {
    pub spf_method: String,
    pub mode: String,
    pub n_nodes: usize,
    pub rows: usize,
    pub cols: usize,
    pub area_proxy_enabled: bool,
    pub root_system_id: String,
    pub spt_vertices: usize,
    pub duration_ns: u128,
    pub duration_us: f64,
}

/// In-process harness holding a wired IS-IS `Instance` with an L2 LSDB of real
/// `Lsp` PDUs (torus mesh + Proxy SID).
pub struct SpfProfilingHarness {
    instance: Instance,
    _nb_rx: UnboundedReceiver<holo_northbound::api::provider::Notification>,
    _ibus_rx: IbusChannelsRx,
    _proto_rx: ProtocolInputChannelsRx,
    #[cfg(feature = "testing")]
    _proto_out_rx: tokio::sync::mpsc::Receiver<ProtocolOutputMsg>,
    rows: usize,
    cols: usize,
    root: SystemId,
    proxy: SystemId,
}

impl SpfProfilingHarness {
    /// Build an `R×C` torus of bidirectional wide-metric IS-reach LSPs.
    ///
    /// * Grid nodes are Inside routers (Area Proxy TLV present).
    /// * Proxy SID peers with the edge column so flat vs proxy differs.
    /// * `metric_type = Wide` so `ext_is_reach` feeds `vertex_edges`.
    pub fn torus(rows: usize, cols: usize) -> Self {
        assert!(rows >= 2 && cols >= 2, "torus needs at least 2x2");

        let (nb_tx, nb_rx) = mpsc::unbounded_channel();
        let (ibus_tx, ibus_rx) = ibus::ibus_channels();
        let (proto_tx, proto_rx) = Instance::protocol_input_channels();

        #[cfg(feature = "testing")]
        let (proto_out_tx, proto_out_rx) =
            mpsc::channel::<ProtocolOutputMsg>(4);
        let tx = InstanceChannelsTx {
            nb: nb_tx,
            ibus: ibus_tx,
            protocol_input: proto_tx,
            #[cfg(feature = "testing")]
            protocol_output: proto_out_tx,
        };

        let mut instance = Instance::new(
            "spf-profiling".to_owned(),
            InstanceShared::default(),
            tx,
        );
        instance.state = Some(InstanceState::new(1));
        instance.config = profiling_instance_cfg();

        let proxy = sid_bytes([0x00, 0x00, 0x00, 0x00, 0x00, 0xa1]);
        instance.config.area_proxy.proxy_system_id = Some(proxy);

        {
            let (view, arenas) = instance.as_up().expect("state present");
            let lsdb = view.state.lsdb.get_mut(LevelNumber::L2);

            for r in 0..rows {
                for c in 0..cols {
                    let me = grid_sid(r, c, cols);
                    let mut nbrs = vec![
                        grid_sid(r, (c + 1) % cols, cols),
                        grid_sid(r, (c + cols - 1) % cols, cols),
                        grid_sid((r + 1) % rows, c, cols),
                        grid_sid((r + rows - 1) % rows, c, cols),
                    ];
                    if c == 0 {
                        nbrs.push(proxy);
                    }
                    let lsp = make_node_lsp(me, &nbrs, true);
                    lsdb.insert_for_profiling(&mut arenas.lsp_entries, lsp);
                }
            }

            let mut proxy_nbrs = Vec::new();
            for r in 0..rows {
                proxy_nbrs.push(grid_sid(r, 0, cols));
            }
            let proxy_lsp = make_node_lsp(proxy, &proxy_nbrs, false);
            lsdb.insert_for_profiling(&mut arenas.lsp_entries, proxy_lsp);
        }

        let root = grid_sid(0, 0, cols);
        SpfProfilingHarness {
            instance,
            _nb_rx: nb_rx,
            _ibus_rx: ibus_rx,
            _proto_rx: proto_rx,
            #[cfg(feature = "testing")]
            _proto_out_rx: proto_out_rx,
            rows,
            cols,
            root,
            proxy,
        }
    }

    pub fn n_nodes(&self) -> usize {
        self.rows * self.cols
    }

    pub fn root(&self) -> SystemId {
        self.root
    }

    pub fn proxy(&self) -> SystemId {
        self.proxy
    }

    /// Run one real `compute_spt` with Area Proxy on or off.
    pub fn run_once(&mut self, area_proxy_enabled: bool) -> (usize, Duration) {
        self.instance.config.area_proxy.enabled = area_proxy_enabled;
        if area_proxy_enabled {
            self.instance.config.area_proxy.proxy_system_id = Some(self.proxy);
        }

        let root = self.root;
        let (view, arenas) = self.instance.as_up().expect("state present");
        let empty_ifaces = Interfaces::default();
        let empty_adjs: Arena<crate::adjacency::Adjacency> = Arena::default();

        let t0 = Instant::now();
        let spt = spf::compute_spt_for_profiling(
            LevelNumber::L2,
            root,
            false,
            Some(MtId::Standard),
            MetricMode::Normal,
            &view,
            &empty_ifaces,
            &empty_adjs,
            &arenas.lsp_entries,
        );
        let dt = t0.elapsed();
        let verts = spt.iter().count();
        let _ = view;
        (verts, dt)
    }

    pub fn run_batch(&mut self, area_proxy_enabled: bool, iters: u64) -> usize {
        let mut last = 0;
        for _ in 0..iters {
            let (v, _) = self.run_once(area_proxy_enabled);
            last = v;
        }
        last
    }

    pub fn sample(&mut self, area_proxy_enabled: bool) -> SpfSptSample {
        let (verts, dt) = self.run_once(area_proxy_enabled);
        let mode = if area_proxy_enabled { "proxy" } else { "flat" };
        SpfSptSample {
            spf_method: SPF_METHOD_REAL.to_owned(),
            mode: mode.to_owned(),
            n_nodes: self.n_nodes(),
            rows: self.rows,
            cols: self.cols,
            area_proxy_enabled,
            root_system_id: format_sid(self.root),
            spt_vertices: verts,
            duration_ns: dt.as_nanos(),
            duration_us: dt.as_secs_f64() * 1_000_000.0,
        }
    }
}

/// Public entry used by external benches (always real path).
pub fn compute_spt_timed(
    harness: &mut SpfProfilingHarness,
    area_proxy_enabled: bool,
) -> SpfSptSample {
    harness.sample(area_proxy_enabled)
}

fn profiling_instance_cfg() -> InstanceCfg {
    let mut cfg = InstanceCfg::default();
    cfg.metric_type = LevelsCfgWithDefault::with_all(MetricType::Wide);
    cfg.afs.insert(
        AddressFamily::Ipv4,
        AddressFamilyCfg {
            enabled: true,
            redistribution: Default::default(),
        },
    );
    cfg.afs.insert(
        AddressFamily::Ipv6,
        AddressFamilyCfg {
            enabled: false,
            redistribution: Default::default(),
        },
    );
    cfg.area_proxy.enabled = false;
    cfg
}

fn grid_sid(r: usize, c: usize, cols: usize) -> SystemId {
    let n = (r * cols + c + 1) as u32;
    let b = n.to_be_bytes();
    sid_bytes([
        0x00,
        0x00,
        b[1],
        b[2],
        b[3],
        ((r as u8) << 4) | (c as u8 & 0x0f),
    ])
}

fn sid_bytes(b: [u8; 6]) -> SystemId {
    SystemId::from(b)
}

fn format_sid(s: SystemId) -> String {
    let b: &[u8; 6] = s.as_ref();
    format!(
        "{:02x}{:02x}.{:02x}{:02x}.{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5]
    )
}

fn make_node_lsp(me: SystemId, nbrs: &[SystemId], inside: bool) -> Lsp {
    let reaches: Vec<IsReach> = nbrs
        .iter()
        .map(|n| IsReach {
            neighbor: LanId::from((*n, 0u8)),
            metric: 10,
            sub_tlvs: IsReachStlvs::default(),
        })
        .collect();

    let mut tlvs = LspTlvs::new(
        [Nlpid::Ipv4 as u8],
        vec![],
        vec![],
        vec![],
        [],
        None,
        Some(1492),
        [],
        reaches,
        [],
        [],
        [],
        [],
        [],
        [],
        None,
        [],
        [],
        [],
        None,
    );
    if inside {
        tlvs.area_proxy = Some(AreaProxyTlv {
            sub_tlvs: Default::default(),
        });
    }

    let flags = LspFlags::IS_TYPE1 | LspFlags::IS_TYPE2;
    Lsp::new(
        LevelNumber::L2,
        1200,
        LspId::from((me, 0u8, 0u8)),
        1,
        flags,
        tlvs,
        None,
    )
}

/// Collect multi-size flat/proxy samples for CSV export.
pub fn collect_scale_samples(
    sizes: &[(usize, usize)],
    repeats: usize,
) -> Vec<SpfSptSample> {
    let mut out = Vec::new();
    for &(rows, cols) in sizes {
        let mut h = SpfProfilingHarness::torus(rows, cols);
        for _ in 0..repeats {
            out.push(h.sample(false));
            out.push(h.sample(true));
        }
    }
    out
}

/// Write samples as CSV. Always emits `spf_method=real_compute_spt`.
pub fn write_samples_csv(
    path: &std::path::Path,
    samples: &[SpfSptSample],
) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    writeln!(
        f,
        "spf_method,mode,n_nodes,rows,cols,area_proxy_enabled,root_system_id,spt_vertices,duration_ns,duration_us"
    )?;
    for s in samples {
        assert_eq!(s.spf_method, SPF_METHOD_REAL);
        writeln!(
            f,
            "{},{},{},{},{},{},{},{},{},{:.6}",
            s.spf_method,
            s.mode,
            s.n_nodes,
            s.rows,
            s.cols,
            s.area_proxy_enabled,
            s.root_system_id,
            s.spt_vertices,
            s.duration_ns,
            s.duration_us
        )?;
    }
    Ok(())
}

/// Write samples as JSON array.
pub fn write_samples_json(
    path: &std::path::Path,
    samples: &[SpfSptSample],
) -> std::io::Result<()> {
    let s = serde_json::to_string_pretty(samples)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, s)
}

#[allow(dead_code)]
fn _spt_touch(spt: &Spt) -> usize {
    spt.iter().count()
}

#[cfg(test)]
mod export_tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn export_samples_to_env_dir() {
        let out = std::env::var("YQH162_BENCH_OUT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("target/yqh162-spf-spt"));
        std::fs::create_dir_all(&out).unwrap();
        let sizes = [(4usize, 4), (6, 6), (8, 8), (10, 10)];
        let samples = collect_scale_samples(&sizes, 20);
        assert!(samples.iter().all(|s| s.spf_method == SPF_METHOD_REAL));
        let flat36 = samples
            .iter()
            .find(|s| s.n_nodes == 36 && s.mode == "flat")
            .unwrap();
        assert!(flat36.spt_vertices >= 36, "verts={}", flat36.spt_vertices);
        write_samples_csv(&out.join("spf_spt_real_compute_spt.csv"), &samples)
            .unwrap();
        write_samples_json(
            &out.join("spf_spt_real_compute_spt.json"),
            &samples,
        )
        .unwrap();
        eprintln!("exported {} samples to {}", samples.len(), out.display());
    }
}

//
// Measurement-only LSDB fixture builder for real `compute_spt` benches (YQH-157).
// Gated by feature = "profiling". Builds genuine `Lsp` TLVs (ext-IS-reach),
// not an isomorphic Dijkstra stand-in.
//

#![cfg(feature = "profiling")]

use std::time::Instant;

use holo_protocol::{InstanceChannelsTx, InstanceShared, ProtocolInstance};
use holo_utils::ibus;
use tokio::sync::mpsc;

use crate::instance::{
    Instance, InstanceArenas, InstanceState, InstanceSys, InstanceUpView,
};
use crate::northbound::configuration::{InstanceAreaProxyCfg, InstanceCfg};
use crate::packet::iana::{MtId, Nlpid};
use crate::packet::pdu::{Lsp, LspFlags, LspTlvs};
use crate::packet::tlv::{IsReach, IsReachStlvs};
use crate::packet::{LanId, LevelNumber, LspId, SystemId};
use crate::spf::{self, MetricMode, Spt};

/// Holds owned state required to invoke `compute_spt` repeatedly.
pub struct SpfBenchWorld {
    pub name: String,
    pub system: InstanceSys,
    pub config: InstanceCfg,
    pub state: InstanceState,
    pub arenas: InstanceArenas,
    pub tx: InstanceChannelsTx<Instance>,
    pub shared: InstanceShared,
    /// Keep channel receivers alive so senders stay open.
    _nb_rx: mpsc::UnboundedReceiver<holo_northbound::api::provider::Notification>,
    _ibus_rx: ibus::IbusChannelsRx,
    _proto_rx: crate::instance::ProtocolInputChannelsRx,
    #[cfg(feature = "testing")]
    _proto_out_rx: mpsc::Receiver<crate::tasks::messages::ProtocolOutputMsg>,
    pub root: SystemId,
    pub n_nodes: usize,
    pub n_lsps: usize,
}

impl SpfBenchWorld {
    pub fn set_area_proxy(&mut self, enabled: bool, proxy: Option<SystemId>) {
        self.config.area_proxy.enabled = enabled;
        self.config.area_proxy.proxy_system_id = proxy;
        if enabled {
            self.config.area_proxy.role =
                crate::northbound::configuration::AreaProxyRole::Inside;
        }
    }

    /// Run real `compute_spt` once; returns SPT vertex count.
    pub fn run_compute_spt(&mut self) -> (Spt, usize) {
        let root = self.root;
        // Partial borrow: view borrows name/system/config/state/tx/shared;
        // arenas is a separate field.
        let spt = {
            let view = InstanceUpView {
                name: &self.name,
                system: &self.system,
                config: &self.config,
                state: &mut self.state,
                tx: &self.tx,
                shared: &self.shared,
            };
            spf::compute_spt_for_profiling(
                LevelNumber::L2,
                root,
                false, // local=false: root from LSDB
                Some(MtId::Standard),
                MetricMode::Normal,
                &view,
                &self.arenas.interfaces,
                &self.arenas.adjacencies,
                &self.arenas.lsp_entries,
            )
        };
        let n = spf::spt_vertex_count(&spt);
        (spt, n)
    }

    /// Wall-clock ns for `iters` calls (not criterion; for CSV export).
    pub fn time_ns(&mut self, iters: u32) -> u128 {
        let _ = self.run_compute_spt(); // warmup
        let t0 = Instant::now();
        for _ in 0..iters {
            let (spt, _) = self.run_compute_spt();
            std::hint::black_box(spt);
        }
        t0.elapsed().as_nanos()
    }
}

fn sid(byte: u8) -> SystemId {
    SystemId::from([0, 0, 0, 0, 0, byte])
}

fn proxy_sid() -> SystemId {
    SystemId::from([0, 0, 0, 0, 0, 0xa1])
}

fn make_is_reach(neighbor: SystemId, metric: u32) -> IsReach {
    IsReach {
        neighbor: LanId::from((neighbor, 0)),
        metric,
        sub_tlvs: IsReachStlvs::default(),
    }
}

fn make_node_lsp(sys: SystemId, neighbors: &[SystemId], seqno: u32) -> Lsp {
    let reaches: Vec<IsReach> = neighbors
        .iter()
        .copied()
        .map(|n| make_is_reach(n, 10))
        .collect();
    let tlvs = LspTlvs::new(
        [Nlpid::Ipv4 as u8, Nlpid::Ipv6 as u8],
        vec![],
        vec![],
        std::iter::empty(),
        std::iter::empty(),
        None,
        None,
        std::iter::empty(),
        reaches,
        std::iter::empty(),
        std::iter::empty(),
        std::iter::empty(),
        std::iter::empty(),
        std::iter::empty(),
        std::iter::empty(),
        None,
        std::iter::empty(),
        std::iter::empty(),
        std::iter::empty(),
        None,
    );
    let flags = LspFlags::IS_TYPE1 | LspFlags::IS_TYPE2;
    Lsp::new(
        LevelNumber::L2,
        1200,
        LspId::from((sys, 0u8, 0u8)),
        seqno,
        flags,
        tlvs,
        None,
    )
}

/// Build a bidirectional ring of `n` nodes plus an optional Area Proxy LSP
/// attached to node 0. Same LSDB is used for flat vs proxy; only the
/// `area_proxy` config bit differs at measurement time.
pub fn build_ring_world(n: usize, with_proxy_lsp: bool) -> SpfBenchWorld {
    assert!(n >= 3 && n <= 250, "n out of supported range");

    let (nb_tx, nb_rx) = mpsc::unbounded_channel();
    let (ibus_tx, ibus_rx) = ibus::ibus_channels();
    let (proto_tx, proto_rx) = <Instance as ProtocolInstance>::protocol_input_channels();
    #[cfg(feature = "testing")]
    let (proto_out_tx, proto_out_rx) = mpsc::channel(4);

    let tx: InstanceChannelsTx<Instance> = InstanceChannelsTx::new(
        nb_tx,
        ibus_tx,
        proto_tx,
        #[cfg(feature = "testing")]
        proto_out_tx,
    );
    // Clone purge sender before we borrow tx into world (UnboundedSender is cheap).
    let lsp_purgep = tx.protocol_input.lsp_purge.clone();

    // Default InstanceCfg uses yang metric-type (wide-only on this fork).
    let mut config = InstanceCfg::default();
    config.area_proxy = InstanceAreaProxyCfg::default();

    let mut state = InstanceState {
        boot_count: 1,
        circuit_id_allocator: Default::default(),
        hostnames: Default::default(),
        lsdb: Default::default(),
        lsp_orig_last: None,
        lsp_orig_backoff: None,
        lsp_orig_pending: None,
        spf_sched: Default::default(),
        spt: Default::default(),
        flooding_reduction: Default::default(),
        rib_single: Default::default(),
        rib_multi: Default::default(),
        summaries: Default::default(),
        counters: Default::default(),
        discontinuity_time: chrono::Utc::now(),
        lsp_log: Default::default(),
        lsp_log_next_id: 0,
        spf_log: Default::default(),
        spf_log_next_id: 0,
    };

    let mut arenas = InstanceArenas::default();
    let level = LevelNumber::L2;

    // Ring: node i <-> i±1 (mod n). Node ids 1..=n.
    let nodes: Vec<SystemId> = (1..=n).map(|i| sid(i as u8)).collect();
    let mut n_lsps = 0usize;

    for i in 0..n {
        let me = nodes[i];
        let left = nodes[(i + n - 1) % n];
        let right = nodes[(i + 1) % n];
        let mut neigh = vec![left, right];
        if with_proxy_lsp && i == 0 {
            neigh.push(proxy_sid());
        }
        let lsp = make_node_lsp(me, &neigh, 1);
        state.lsdb.get_mut(level).insert(
            &mut arenas.lsp_entries,
            level,
            lsp,
            &lsp_purgep,
        );
        n_lsps += 1;
    }

    if with_proxy_lsp {
        let lsp = make_node_lsp(proxy_sid(), &[nodes[0]], 1);
        state.lsdb.get_mut(level).insert(
            &mut arenas.lsp_entries,
            level,
            lsp,
            &lsp_purgep,
        );
        n_lsps += 1;
    }

    SpfBenchWorld {
        name: format!("prof-ring-{n}"),
        system: InstanceSys::default(),
        config,
        state,
        arenas,
        tx,
        shared: InstanceShared::default(),
        _nb_rx: nb_rx,
        _ibus_rx: ibus_rx,
        _proto_rx: proto_rx,
        #[cfg(feature = "testing")]
        _proto_out_rx: proto_out_rx,
        root: nodes[0],
        n_nodes: n,
        n_lsps,
    }
}

/// Convenience: time flat vs proxy on the same ring LSDB.
pub fn measure_flat_vs_proxy(n: usize, iters: u32) -> MeasureRow {
    let mut world = build_ring_world(n, true);
    let proxy = proxy_sid();

    world.set_area_proxy(false, None);
    let flat_ns = world.time_ns(iters);
    let (_, flat_verts) = world.run_compute_spt();

    world.set_area_proxy(true, Some(proxy));
    let proxy_ns = world.time_ns(iters);
    let (_, proxy_verts) = world.run_compute_spt();

    MeasureRow {
        n,
        n_lsps: world.n_lsps,
        iters,
        flat_total_ns: flat_ns,
        proxy_total_ns: proxy_ns,
        flat_verts,
        proxy_verts,
        spf_method: "real_compute_spt",
    }
}

#[derive(Clone, Debug)]
pub struct MeasureRow {
    pub n: usize,
    pub n_lsps: usize,
    pub iters: u32,
    pub flat_total_ns: u128,
    pub proxy_total_ns: u128,
    pub flat_verts: usize,
    pub proxy_verts: usize,
    pub spf_method: &'static str,
}

impl MeasureRow {
    pub fn flat_ns_per_call(&self) -> f64 {
        self.flat_total_ns as f64 / self.iters as f64
    }
    pub fn proxy_ns_per_call(&self) -> f64 {
        self.proxy_total_ns as f64 / self.iters as f64
    }
}

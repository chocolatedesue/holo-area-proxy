//
// Copyright (c) The Holo Core Contributors
//
// SPDX-License-Identifier: MIT
//

//! RFC 9666 Area Proxy helpers: Proxy LSP synthesis and Inside Edge filtering.

use std::collections::BTreeMap;

use holo_utils::ip::AddressFamily;
use ipnetwork::{Ipv4Network, Ipv6Network};

use crate::collections::{Arena, Lsdb};
use crate::lsdb::LspEntry;
use crate::debug::LspPurgeReason;
use crate::instance::{InstanceArenas, InstanceUpView};
use crate::interface::Interface;
use crate::lsdb;
use crate::northbound::configuration::{AreaProxyRole, InstanceCfg};
use crate::packet::iana::Nlpid;
use crate::packet::pdu::{Lsp, LspFlags, LspTlvs};
use crate::packet::tlv::{AreaProxyTlv, Ipv4Reach, Ipv6Reach, IsReach};
use crate::packet::{LevelNumber, LspId, SystemId};

/// Returns true when Area Proxy is enabled on the instance.
pub(crate) fn is_enabled(cfg: &InstanceCfg) -> bool {
    cfg.area_proxy.enabled
}

/// True when this instance should originate TLV 20 on L2 LSP fragment 0.
pub(crate) fn should_originate_tlv20(cfg: &InstanceCfg) -> bool {
    cfg.area_proxy.enabled
}

/// True when this instance acts as Area Leader (or static leader) for Proxy LSP.
pub(crate) fn is_leader(cfg: &InstanceCfg) -> bool {
    if !cfg.area_proxy.enabled {
        return false;
    }
    matches!(
        cfg.area_proxy.role,
        AreaProxyRole::Leader | AreaProxyRole::Static
    )
}

/// True when the interface is configured as outside-facing under Area Proxy.
pub(crate) fn is_outside_facing(iface: &Interface) -> bool {
    iface.config.is_outside_facing()
}

/// Effective IIH source system-id for an interface.
///
/// On outside-facing interfaces of edge/leader/static routers, use the Area
/// Proxy System ID when configured; otherwise fall back to the real system-id.
/// Missing `proxy-system-id` must not panic.
pub(crate) fn hello_source(
    cfg: &InstanceCfg,
    iface: &Interface,
) -> Option<SystemId> {
    hello_source_for(cfg, is_outside_facing(iface))
}

/// Same as [`hello_source`], taking an explicit outside-facing flag.
pub(crate) fn hello_source_for(
    cfg: &InstanceCfg,
    outside_facing: bool,
) -> Option<SystemId> {
    let local = cfg.system_id?;
    if !cfg.area_proxy.enabled || !outside_facing {
        return Some(local);
    }
    if cfg.area_proxy.uses_proxy_sid_on_outside() {
        Some(cfg.area_proxy.proxy_system_id.unwrap_or(local))
    } else {
        Some(local)
    }
}

/// Returns true when `system_id` is a local identity for three-way adjacency
/// validation on this interface.
///
/// RFC 5303 three-way hellos echo the neighbor's IIH source system-id. On
/// outside-facing Area Proxy edges that source IIH with the Proxy SID, Outside
/// will therefore echo the Proxy SID — not the edge's real system-id. Accept
/// either identity so P2P three-way can complete (RFC 9666 §5.1).
pub(crate) fn is_local_hello_identity(
    cfg: &InstanceCfg,
    iface: &Interface,
    system_id: SystemId,
) -> bool {
    is_local_hello_identity_for(cfg, is_outside_facing(iface), system_id)
}

/// Same as [`is_local_hello_identity`], taking an explicit outside-facing flag.
pub(crate) fn is_local_hello_identity_for(
    cfg: &InstanceCfg,
    outside_facing: bool,
    system_id: SystemId,
) -> bool {
    if let Some(local) = cfg.system_id
        && system_id == local
    {
        return true;
    }
    if let Some(source) = hello_source_for(cfg, outside_facing)
        && system_id == source
    {
        return true;
    }
    false
}

/// SNP (CSNP/PSNP) source system-id for an interface.
pub(crate) fn snp_source_system_id(
    cfg: &InstanceCfg,
    iface: &Interface,
) -> Option<SystemId> {
    hello_source(cfg, iface)
}

/// RFC 9666 §5.2 R1: L2 LSP source appears in the L1 LSDB.
pub(crate) fn system_id_in_l1_lsdb(
    lsdb_l1: &Lsdb,
    lsp_entries: &Arena<LspEntry>,
    system_id: SystemId,
) -> bool {
    lsdb_l1
        .iter_for_system_id(lsp_entries, system_id)
        .any(|lse| lse.data.rem_lifetime != 0 && lse.data.seqno != 0)
}

/// RFC 9666 §5.2: whether an LSP may be flooded out `iface`.
///
/// Filtering applies only on outside-facing interfaces when Area Proxy is
/// enabled. The Proxy LSP itself is always allowed out outside-facing links.
pub(crate) fn may_flood_lsp(
    cfg: &InstanceCfg,
    outside_facing: bool,
    level: LevelNumber,
    lsp: &Lsp,
    lsdb_l1: &Lsdb,
    lsp_entries: &Arena<LspEntry>,
) -> bool {
    if !cfg.area_proxy.enabled || level != LevelNumber::L2 {
        return true;
    }
    if !outside_facing {
        return true;
    }

    // Allow the Proxy LSP (source = proxy system id) out outside interfaces.
    if let Some(proxy_sid) = cfg.area_proxy.proxy_system_id
        && lsp.lsp_id.system_id == proxy_sid
    {
        return true;
    }

    // R2: LSP containing Area Proxy TLV must not leak outside.
    if lsp.tlvs.area_proxy.is_some() {
        return false;
    }

    // R1: L2 LSP whose source appears in L1 LSDB must not leak outside.
    if system_id_in_l1_lsdb(lsdb_l1, lsp_entries, lsp.lsp_id.system_id) {
        return false;
    }

    true
}

/// Build Area Proxy TLV for local L2 LSP fragment 0.
pub(crate) fn build_local_area_proxy_tlv(
    cfg: &InstanceCfg,
) -> Option<AreaProxyTlv> {
    if !should_originate_tlv20(cfg) {
        return None;
    }
    Some(AreaProxyTlv::with_proxy_system_id(
        cfg.area_proxy.proxy_system_id,
    ))
}

/// Build Proxy LSP TLVs from Inside Edge L2 LSPs (leader path).
///
/// Collects extended IS reachability toward outside neighbors and lowest-metric
/// prefix reachability. Does **not** include TLV 20.
pub(crate) fn build_proxy_lsp_tlvs(
    instance: &InstanceUpView<'_>,
    lsp_entries: &Arena<LspEntry>,
) -> Option<LspTlvs> {
    let cfg = instance.config;
    if !is_leader(cfg) {
        return None;
    }
    let proxy_sid = cfg.area_proxy.proxy_system_id?;

    let lsdb = instance.state.lsdb.get(LevelNumber::L2);
    let mut ext_is: BTreeMap<[u8; 7], IsReach> = BTreeMap::new();
    let mut ipv4: BTreeMap<Ipv4Network, Ipv4Reach> = BTreeMap::new();
    let mut ipv6: BTreeMap<Ipv6Network, Ipv6Reach> = BTreeMap::new();
    let mut protocols: Vec<u8> = vec![];

    for lse in lsdb.iter(lsp_entries) {
        let lsp = &lse.data;
        if let Some(local) = cfg.system_id
            && lsp.lsp_id.system_id == local
        {
            continue;
        }
        if lsp.lsp_id.system_id == proxy_sid {
            continue;
        }
        // Only consider LSPs that advertise Area Proxy readiness (inside nodes).
        if lsp.tlvs.area_proxy.is_none() {
            continue;
        }
        if lsp.rem_lifetime == 0 || lsp.seqno == 0 {
            continue;
        }

        for p in lsp.tlvs.protocols_supported() {
            if !protocols.contains(&p) {
                protocols.push(p);
            }
        }

        for nbr in lsp.tlvs.ext_is_reach() {
            let nbr_sid = nbr.neighbor.system_id;
            let nbr_is_inside = lsdb
                .iter_for_system_id(lsp_entries, nbr_sid)
                .any(|e| e.data.tlvs.area_proxy.is_some());
            if nbr_is_inside {
                continue;
            }
            let key = {
                let mut k = [0u8; 7];
                let b: &[u8; 6] = nbr.neighbor.system_id.as_ref();
                k[..6].copy_from_slice(b);
                k[6] = nbr.neighbor.pseudonode;
                k
            };
            ext_is
                .entry(key)
                .and_modify(|existing| {
                    if nbr.metric < existing.metric {
                        *existing = nbr.clone();
                    }
                })
                .or_insert_with(|| nbr.clone());
        }

        for pfx in lsp.tlvs.ext_ipv4_reach() {
            ipv4.entry(pfx.prefix)
                .and_modify(|existing| {
                    if pfx.metric < existing.metric {
                        *existing = pfx.clone();
                    }
                })
                .or_insert_with(|| pfx.clone());
        }

        for pfx in lsp.tlvs.ipv6_reach() {
            ipv6.entry(pfx.prefix)
                .and_modify(|existing| {
                    if pfx.metric < existing.metric {
                        *existing = pfx.clone();
                    }
                })
                .or_insert_with(|| pfx.clone());
        }
    }

    // MUST have at least one IS neighbor.
    if ext_is.is_empty() {
        return None;
    }

    if protocols.is_empty() {
        if cfg.is_af_enabled(AddressFamily::Ipv4) {
            protocols.push(Nlpid::Ipv4 as u8);
        }
        if cfg.is_af_enabled(AddressFamily::Ipv6) {
            protocols.push(Nlpid::Ipv6 as u8);
        }
    }

    let hostname = {
        let b: &[u8; 6] = proxy_sid.as_ref();
        Some(format!(
            "proxy-{:02x}{:02x}.{:02x}{:02x}.{:02x}{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        ))
    };

    let mut tlvs = LspTlvs::new(
        protocols,
        vec![],
        vec![],
        cfg.area_addrs.clone(),
        [],
        hostname,
        Some(cfg.lsp_mtu),
        [],
        ext_is.into_values(),
        [],
        [],
        [],
        [],
        ipv4.into_values(),
        [],
        None,
        [],
        ipv6.into_values(),
        [],
        None,
    );
    // Explicitly no area_proxy on Proxy LSP.
    tlvs.area_proxy = None;
    Some(tlvs)
}

/// Originate or refresh the Proxy LSP on the leader.
pub(crate) fn originate_proxy_lsp(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
) {
    if !is_leader(instance.config) {
        purge_proxy_lsp(instance, arenas);
        return;
    }
    let Some(proxy_sid) = instance.config.area_proxy.proxy_system_id else {
        // Missing proxy-system-id: do not panic; purge any stale Proxy LSP.
        purge_proxy_lsp(instance, arenas);
        return;
    };

    let Some(tlvs) = build_proxy_lsp_tlvs(instance, &arenas.lsp_entries) else {
        purge_proxy_lsp(instance, arenas);
        return;
    };

    let level = LevelNumber::L2;
    let lsp_id = LspId::from((proxy_sid, 0u8, 0u8));
    let lsdb = instance.state.lsdb.get(level);
    let old = lsdb
        .get_by_lspid(&arenas.lsp_entries, &lsp_id)
        .map(|(_, lse)| &lse.data);

    if let Some(old_lsp) = old
        && old_lsp.tlvs == tlvs
        && old_lsp.rem_lifetime != 0
    {
        return;
    }

    let seqno = old.map(|l| l.seqno.wrapping_add(1)).unwrap_or(1);
    let flags = LspFlags::IS_TYPE2;
    let auth = instance
        .config
        .auth
        .all
        .method(&instance.shared.keychains);
    let lsp = Lsp::new(
        level,
        instance.config.lsp_lifetime,
        lsp_id,
        seqno,
        flags,
        tlvs,
        auth.as_ref().and_then(|a| a.get_key_send()),
    );

    lsdb::lsp_originate(instance, arenas, level, lsp);
}

/// Purge any existing Proxy LSP fragments for the configured proxy SID.
pub(crate) fn purge_proxy_lsp(
    instance: &mut InstanceUpView<'_>,
    arenas: &mut InstanceArenas,
) {
    let Some(proxy_sid) = instance.config.area_proxy.proxy_system_id else {
        return;
    };
    let level = LevelNumber::L2;
    let lsdb = instance.state.lsdb.get(level);
    let ids: Vec<_> = lsdb
        .iter_for_system_id(&arenas.lsp_entries, proxy_sid)
        .map(|lse| lse.id)
        .collect();
    for id in ids {
        instance
            .tx
            .protocol_input
            .lsp_purge(level, id, LspPurgeReason::Removed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::northbound::configuration::{AreaProxyRole, InstanceCfg};

    fn sid(bytes: [u8; 6]) -> SystemId {
        SystemId::from(bytes)
    }

    fn edge_cfg(real: SystemId, proxy: SystemId) -> InstanceCfg {
        let mut cfg = InstanceCfg::default();
        cfg.system_id = Some(real);
        cfg.area_proxy.enabled = true;
        cfg.area_proxy.role = AreaProxyRole::Edge;
        cfg.area_proxy.proxy_system_id = Some(proxy);
        cfg
    }

    #[test]
    fn hello_source_uses_proxy_sid_on_outside_edge() {
        let real = sid([0, 0, 0, 0, 0, 2]);
        let proxy = sid([0, 0, 0, 0, 0, 0xa0]);
        let cfg = edge_cfg(real, proxy);

        assert_eq!(hello_source_for(&cfg, true), Some(proxy));
        assert_eq!(hello_source_for(&cfg, false), Some(real));
    }

    #[test]
    fn local_hello_identity_accepts_proxy_sid_on_outside() {
        let real = sid([0, 0, 0, 0, 0, 2]);
        let proxy = sid([0, 0, 0, 0, 0, 0xa0]);
        let other = sid([0, 0, 0, 0, 0, 0x11]);
        let cfg = edge_cfg(real, proxy);

        // Outside-facing: real SID and Proxy SID are both local.
        assert!(is_local_hello_identity_for(&cfg, true, real));
        assert!(is_local_hello_identity_for(&cfg, true, proxy));
        assert!(!is_local_hello_identity_for(&cfg, true, other));

        // Inside-facing: only the real SID is local (IIH still uses real SID).
        assert!(is_local_hello_identity_for(&cfg, false, real));
        assert!(!is_local_hello_identity_for(&cfg, false, proxy));
        assert!(!is_local_hello_identity_for(&cfg, false, other));
    }
}

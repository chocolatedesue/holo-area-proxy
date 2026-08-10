#!/usr/bin/env python3
"""Generate 6x6 torus holod configs + expanded smu manifest for YQH-135."""
from __future__ import annotations
import sys
import ipaddress
import json
import os
import re
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from yqh157_paths import lab_root  # noqa: E402
import argparse as _argparse

def _cli_lab():
    ap = _argparse.ArgumentParser(add_help=False)
    ap.add_argument("--lab", default=None)
    args, _ = ap.parse_known_args()
    return lab_root(args.lab or os.environ.get("YQH157_LAB") or os.environ.get("YQH157_WD"))

WD = _cli_lab()
ROWS, COLS = 6, 6
BAND_AREA = {0: "49.0001", 1: "49.0002", 2: "49.0003"}
BAND_PROXY = {0: "0000.0000.00a1", 1: "0000.0000.00a2", 2: "0000.0000.00a3"}
LEADERS = {(1, 3), (3, 3), (5, 3)}


def band_of(r: int) -> int:
    return (r - 1) // 2


def sys_id(r: int, c: int) -> str:
    return f"0000.00{r:02d}.00{c:02d}"


def lo_ip(r: int, c: int) -> str:
    return f"10.64.{r}.{c}"


def node_name(r: int, c: int) -> str:
    return f"r{r}c{c}"


def outside_if(r: int) -> str:
    if r in (1, 3, 5):
        return "eth-u"
    return "eth-d"


def gen_links():
    links = []
    for r in range(1, ROWS + 1):
        for c in range(1, COLS + 1):
            n = node_name(r, c)
            rc = c + 1 if c < COLS else 1
            rn = node_name(r, rc)
            links.append((n, "eth-r", rn, "eth-l"))
            rr = r + 1 if r < ROWS else 1
            dn = node_name(rr, c)
            links.append((n, "eth-d", dn, "eth-u"))
    return links


def assign_p2p(links):
    addrs = {}
    base = int(ipaddress.IPv4Address("172.16.0.0"))
    for i, (ln, li, rn, ri) in enumerate(links):
        net = base + i * 2
        lip = str(ipaddress.IPv4Address(net))
        rip = str(ipaddress.IPv4Address(net + 1))
        addrs[(ln, li)] = (lip, 31)
        addrs[(rn, ri)] = (rip, 31)
    return addrs


def if_block(name: str, ip: str, plen: int):
    return {
        "name": name,
        "type": "iana-if-type:ethernetCsmacd",
        "ietf-ip:ipv4": {"address": [{"ip": ip, "prefix-length": plen}]},
        "ietf-ip:ipv6": {},
    }


def isis_if(name: str, facing):
    d = {
        "name": name,
        "enabled": True,
        "interface-type": "point-to-point",
        "hello-interval": {"value": 3},
        "address-families": {
            "address-family-list": [{"address-family": "ipv4"}]
        },
    }
    if facing is not None:
        d["holo-isis:facing"] = facing
    return d


def build_node_json(r: int, c: int, p2p: dict) -> dict:
    name = node_name(r, c)
    b = band_of(r)
    role = (
        "holo-isis:area-proxy-role-leader"
        if (r, c) in LEADERS
        else "holo-isis:area-proxy-role-edge"
    )
    out_if = outside_if(r)
    lo = lo_ip(r, c)
    intf_list = [if_block("lo", lo, 32)]
    isis_list = [isis_if("lo", None)]
    for ifn in ("eth-l", "eth-r", "eth-u", "eth-d"):
        ip, plen = p2p[(name, ifn)]
        intf_list.append(if_block(ifn, ip, plen))
        facing = "outside" if ifn == out_if else "inside"
        isis_list.append(isis_if(ifn, facing))
    return {
        "ietf-interfaces:interfaces": {"interface": intf_list},
        "ietf-routing:routing": {
            "control-plane-protocols": {
                "control-plane-protocol": [
                    {
                        "type": "ietf-isis:isis",
                        "name": "main",
                        "ietf-isis:isis": {
                            "enabled": True,
                            "level-type": "level-all",
                            "system-id": sys_id(r, c),
                            "area-address": [BAND_AREA[b]],
                            "metric-type": {"value": "wide-only"},
                            "interfaces": {"interface": isis_list},
                            "holo-isis:area-proxy": {
                                "enabled": True,
                                "role": role,
                                "proxy-system-id": BAND_PROXY[b],
                            },
                        },
                    }
                ]
            }
        },
        "ietf-system:system": {"hostname": name},
    }


def write_underlay_sh(r: int, c: int, p2p: dict, out_path: Path):
    """Per-node pure bash underlay (no python in container)."""
    name = node_name(r, c)
    lines = ["#!/bin/bash", "set -e", "ip link set lo up || true"]
    lines.append(f"ip addr replace {lo_ip(r,c)}/32 dev lo")
    for ifn in ("eth-l", "eth-r", "eth-u", "eth-d"):
        ip, plen = p2p[(name, ifn)]
        lines.append(f"ip link set {ifn} up || true")
        lines.append(f"ip addr replace {ip}/{plen} dev {ifn}")
    lines.append("sysctl -w net.ipv4.ip_forward=1 >/dev/null 2>&1 || true")
    out_path.write_text("\n".join(lines) + "\n")
    out_path.chmod(0o755)


def write_yaml_manifest(nodes, links, image: str) -> str:
    def nkey(n):
        m = re.match(r"r(\d+)c(\d+)", n)
        return (int(m.group(1)), int(m.group(2)))

    lines = [
        "name: yqh157-torus66",
        "prefix: yqh157",
        "execution:",
        "  algorithm: stream-mal",
        "  seed: 135",
        "topology:",
        "  defaults:",
        "    kind: linux",
        f"    image: {image}",
        "  nodes:",
    ]
    for name in sorted(nodes, key=nkey):
        lines += [
            f"    {name}:",
            f"      image: {image}",
            f'      command: "bash /opt/yqh157/scripts/start-node.sh {name}"',
            "      mounts:",
            "        - source: ./scripts",
            "          target: /opt/yqh157/scripts",
            "          readOnly: true",
            "        - source: ./configs",
            "          target: /opt/yqh157/configs",
            "          readOnly: true",
            "        - source: ./generated/underlay",
            "          target: /opt/yqh157/underlay",
            "          readOnly: true",
            f"        - source: ./generated/var/{name}",
            "          target: /var/opt/holo",
            "          readOnly: false",
        ]
    lines.append("  links:")
    for ln, li, rn, ri in links:
        lines.append(f'    - endpoints: ["{ln}:{li}", "{rn}:{ri}"]')
    lines += [
        "runtime:",
        "  adapter: local",
        "  imageManager: podman",
        "  containerRuntime: crun",
        "  networkManager: netlink",
        "  linkKind: veth",
        "  enableLinkUp: true",
        "  groupLinkUp: false",
        "  autoStaticRoutes: false",
        "  parallelism:",
        "    prepareBundles: 0",
        "    nodeOperations: 4",
        "  storage:",
        "    tmpfsBundles: false",
        "    rootfsCacheRoot: /var/tmp/expctl_rootfs_cache",
        "",
    ]
    return "\n".join(lines)


def main():
    links = gen_links()
    assert len(links) == 72
    p2p = assign_p2p(links)
    cfg_dir = WD / "configs"
    und_dir = WD / "generated" / "underlay"
    cfg_dir.mkdir(parents=True, exist_ok=True)
    und_dir.mkdir(parents=True, exist_ok=True)
    meta = {"nodes": {}, "links": [], "p2p": {}, "leaders": []}
    for r in range(1, 7):
        for c in range(1, 7):
            n = node_name(r, c)
            j = build_node_json(r, c, p2p)
            (cfg_dir / f"{n}.json").write_text(json.dumps(j, indent=2) + "\n")
            write_underlay_sh(r, c, p2p, und_dir / f"{n}.sh")
            (WD / "generated" / "var" / n).mkdir(parents=True, exist_ok=True)
            meta["nodes"][n] = {
                "row": r, "col": c, "band": band_of(r),
                "area": BAND_AREA[band_of(r)],
                "proxy": BAND_PROXY[band_of(r)],
                "sys_id": sys_id(r, c), "lo": lo_ip(r, c),
                "outside_if": outside_if(r),
                "role": "leader" if (r, c) in LEADERS else "edge",
            }
            for ifn in ("eth-l", "eth-r", "eth-u", "eth-d"):
                ip, plen = p2p[(n, ifn)]
                meta["p2p"][f"{n}:{ifn}"] = f"{ip}/{plen}"
    meta["leaders"] = [node_name(r, c) for r, c in sorted(LEADERS)]
    for ln, li, rn, ri in links:
        meta["links"].append({"endpoints": [f"{ln}:{li}", f"{rn}:{ri}"]})
    image = "holo-bundle:yqh135-ee60831"
    (WD / "manifest").mkdir(parents=True, exist_ok=True)
    (WD / "manifest" / "yqh157-torus66.yaml").write_text(
        write_yaml_manifest(meta["nodes"], links, image)
    )
    (WD / "generated" / "topology-meta.json").write_text(json.dumps(meta, indent=2) + "\n")
    print("ok", len(meta["nodes"]), "nodes", len(links), "links")


if __name__ == "__main__":
    main()

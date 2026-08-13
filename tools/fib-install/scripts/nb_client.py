#!/usr/bin/env python3
"""Minimal Holo northbound gRPC client for fib-install smoke tests.

Generates stubs next to this script on first run if missing.
Usage:
  nb_client.py get-fib --addr 127.0.0.1:15051
  nb_client.py commit-static --addr 127.0.0.1:15051 --prefix 198.51.100.0/24
  nb_client.py get-rib --addr 127.0.0.1:15051
"""
from __future__ import annotations

import argparse
import json
import os
import sys
import time
from pathlib import Path

SCRIPT_DIR = Path(__file__).resolve().parent
PROTO_DIR = SCRIPT_DIR.parents[2] / "proto"
GEN_DIR = SCRIPT_DIR / "_gen"


def ensure_stubs() -> None:
    GEN_DIR.mkdir(parents=True, exist_ok=True)
    marker = GEN_DIR / "holo_pb2.py"
    if marker.exists():
        return
    try:
        from grpc_tools import protoc
    except ImportError as e:
        raise SystemExit(
            "need grpcio + grpcio-tools: pip install --user grpcio grpcio-tools"
        ) from e
    rc = protoc.main(
        [
            "protoc",
            f"-I{PROTO_DIR}",
            f"--python_out={GEN_DIR}",
            f"--grpc_python_out={GEN_DIR}",
            str(PROTO_DIR / "holo.proto"),
        ]
    )
    if rc != 0:
        raise SystemExit(f"protoc failed: {rc}")
    (GEN_DIR / "__init__.py").write_text("")


def import_holo():
    ensure_stubs()
    sys.path.insert(0, str(GEN_DIR))
    import grpc
    import holo_pb2
    import holo_pb2_grpc

    return grpc, holo_pb2, holo_pb2_grpc


def channel(addr: str):
    grpc, holo_pb2, holo_pb2_grpc = import_holo()
    ch = grpc.insecure_channel(addr)
    stub = holo_pb2_grpc.NorthboundStub(ch)
    return grpc, holo_pb2, stub


def path_from_xpath(xpath: str):
    """Very small xpath → Path converter: /a/b/c → elems a,b,c (no keys)."""
    _, holo_pb2, _ = import_holo()
    elems = []
    for part in xpath.strip("/").split("/"):
        if not part:
            continue
        # strip module prefix if present: holo-routing:fib → fib still wrong;
        # holod expects full node names as in schema; pass as-is.
        elems.append(holo_pb2.PathElem(name=part, key={}))
    return holo_pb2.Path(elem=elems)


def get_state(addr: str, xpath: str | None, with_defaults: bool = True) -> str:
    grpc, holo_pb2, stub = channel(addr)
    req = holo_pb2.GetStateRequest(
        encoding=holo_pb2.JSON,
        with_defaults=with_defaults,
        path=path_from_xpath(xpath) if xpath else None,
    )
    try:
        resp = stub.GetState(req, timeout=10)
    except grpc.RpcError as e:
        raise SystemExit(f"GetState failed: {e.code()} {e.details()}") from e
    data = resp.data
    if data.WhichOneof("data") == "data_string":
        return data.data_string
    return ""


def commit_merge(addr: str, config_json: str, comment: str = "") -> int:
    grpc, holo_pb2, stub = channel(addr)
    req = holo_pb2.CommitRequest(
        operation=holo_pb2.CommitRequest.MERGE,
        config=holo_pb2.DataTree(
            encoding=holo_pb2.JSON,
            data_string=config_json,
        ),
        comment=comment,
        confirmed_timeout=0,
    )
    try:
        resp = stub.Commit(req, timeout=30)
    except grpc.RpcError as e:
        raise SystemExit(f"Commit failed: {e.code()} {e.details()}") from e
    return int(resp.transaction_id)


STATIC_ROUTE_TMPL = """
{{
  "ietf-routing:routing": {{
    "control-plane-protocols": {{
      "control-plane-protocol": [
        {{
          "type": "ietf-routing:static",
          "name": "default",
          "static-routes": {{
            "ietf-ipv4-unicast-routing:ipv4": {{
              "route": [
                {{
                  "destination-prefix": "{prefix}",
                  "next-hop": {{
                    "special-next-hop": "blackhole"
                  }}
                }}
              ]
            }}
          }}
        }}
      ]
    }}
  }}
}}
"""


def cmd_get_fib(args: argparse.Namespace) -> None:
    # Full routing state then extract fib; path filtering can be picky.
    raw = get_state(args.addr, None)
    try:
        doc = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        print(raw)
        return
    routing = doc.get("ietf-routing:routing") or doc.get("routing") or doc
    fib = routing.get("holo-routing:fib") or routing.get("fib")
    if fib is None:
        # print full for debug
        print(json.dumps(doc, indent=2)[:4000])
        raise SystemExit("fib container not found in GetState")
    print(json.dumps(fib, indent=2, sort_keys=True))


def cmd_get_rib(args: argparse.Namespace) -> None:
    raw = get_state(args.addr, None)
    try:
        doc = json.loads(raw) if raw else {}
    except json.JSONDecodeError:
        print(raw)
        return
    routing = doc.get("ietf-routing:routing") or doc.get("routing") or doc
    ribs = routing.get("ribs") or {}
    print(json.dumps(ribs, indent=2, sort_keys=True)[:8000])


def cmd_commit_static(args: argparse.Namespace) -> None:
    cfg = STATIC_ROUTE_TMPL.format(prefix=args.prefix)
    tid = commit_merge(args.addr, cfg, comment="yqh502 fib-install smoke")
    print(f"commit ok transaction_id={tid}")


def cmd_wait(args: argparse.Namespace) -> None:
    grpc, _, stub = channel(args.addr)
    deadline = time.time() + args.timeout
    last = None
    while time.time() < deadline:
        try:
            stub.Capabilities(import_holo()[1].CapabilitiesRequest(), timeout=2)
            print("ready")
            return
        except Exception as e:  # noqa: BLE001
            last = e
            time.sleep(0.3)
    raise SystemExit(f"holod not ready: {last}")


def main() -> None:
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--addr", default="127.0.0.1:15051")
    sub = p.add_subparsers(dest="cmd", required=True)

    s = sub.add_parser("get-fib")
    s.set_defaults(func=cmd_get_fib)

    s = sub.add_parser("get-rib")
    s.set_defaults(func=cmd_get_rib)

    s = sub.add_parser("commit-static")
    s.add_argument("--prefix", default="198.51.100.0/24")
    s.set_defaults(func=cmd_commit_static)

    s = sub.add_parser("wait")
    s.add_argument("--timeout", type=float, default=30)
    s.set_defaults(func=cmd_wait)

    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()

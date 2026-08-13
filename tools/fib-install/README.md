# fib-install deploy smoke + observability

## Query API (high-performance counters)

gRPC `Northbound.GetState` → JSON path under:

`/ietf-routing:routing/holo-routing:fib`

Leaves (atomic Relaxed counters on RIB hot path):

| leaf | meaning |
|------|---------|
| `install-enabled` | holod.toml `[routing].fib_install` |
| `ip-installs` / `ip-installs-skipped` | netlink IP add performed / skipped |
| `ip-uninstalls` / `ip-uninstalls-skipped` | netlink IP del |
| `mpls-*` | MPLS equivalents |
| `rib-ipv4-active` / `rib-ipv6-active` / `rib-mpls-entries` | in-process RIB sizes |

Example:

```bash
python3 tools/fib-install/scripts/nb_client.py --addr 127.0.0.1:15051 get-fib
```

## Real deploy smoke

Needs root (holod), `libyang`, `grpcio`:

```bash
cargo build -p holo-daemon --bin holod
bash tools/fib-install/scripts/smoke_deploy.sh
```

Runs `fib_install=false` then `true`: commit blackhole static route, assert GetState counters and kernel has no prefix when disabled.

Evidence from last local run: `tools/fib-install/evidence/`.

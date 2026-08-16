# Passive observability tools (YQH-603)

See [`docs/observability.md`](../../docs/observability.md).

```bash
bash tools/observability/smoke_offline.sh
```

Evidence lands in `tools/observability/evidence/`.

Example holod.toml (fib_install=false + obs on): `deploy/holod-obs-fib-false.toml`.

## Recommended tip

Source default after YQH-610: **`main` >= `4ee13d7`**. Checkout:

```bash
git clone https://github.com/chocolatedesue/holo-area-proxy.git && cd holo-area-proxy && git checkout main
```

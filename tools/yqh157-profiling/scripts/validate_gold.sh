#!/bin/bash
set -euo pipefail
WD="${YQH157_LAB:-${YQH157_WD:-/home/cnic/work/yqh157-real-profiling}}"
STATE_ROOT="${EXPCTL_STATE_ROOT:-$WD/state}"
PROTO="$WD/generated/proto/holo.proto"
EV="$WD/evidence/gold"
mkdir -p "$EV"
export EXPCTL_STATE_ROOT="$STATE_ROOT"
CRUN_ROOT=$(find "$STATE_ROOT" -type d -name crun-state 2>/dev/null | head -1)
META="$WD/generated/topology-meta.json"

node_pid() {
  sudo crun --root "$CRUN_ROOT" state "yqh157-$1" 2>/dev/null | python3 -c "import sys,json; print(json.load(sys.stdin)['pid'])"
}

nexec() {
  local n="$1"; shift
  local pid; pid=$(node_pid "$n")
  sudo nsenter -t "$pid" -n -m -p --wd=/ "$@"
}

get_state() {
  local n="$1"
  local pid; pid=$(node_pid "$n")
  sudo nsenter -t "$pid" -n grpcurl -plaintext \
    -import-path "$(dirname "$PROTO")" -proto holo.proto \
    -d '{"encoding":"JSON","withDefaults":false}' \
    127.0.0.1:50051 holo.Northbound/GetState
}

echo "=== adj sample ===" | tee "$EV/adj-sample.txt"
for n in r1c1 r1c3 r2c3 r3c3 r4c1 r5c3 r6c6; do
  echo "--- $n ---" | tee -a "$EV/adj-sample.txt"
  get_state "$n" > "$EV/${n}-getstate.json" 2>"$EV/${n}-getstate.err" || true
  python3 - "$EV/${n}-getstate.json" "$n" <<'PY' | tee -a "$EV/adj-sample.txt"
import json,sys,re
raw=open(sys.argv[1]).read()
node=sys.argv[2]
try:
  d=json.loads(raw)
except Exception as e:
  print(node,"parse_err",e); sys.exit(0)
s=(d.get("data") or {}).get("dataString") or (d.get("data") or {}).get("data_string") or ""
if not s:
  print(node,"no dataString keys",list(d.keys())[:10]); sys.exit(0)
open(sys.argv[1].replace("getstate.json","state-tree.json"),"w").write(s)
tree=json.loads(s)
# walk for adjacencies
ups=[]
def walk(o,path=""):
  if isinstance(o,dict):
    if "neighbor-sys-id" in o or "neighbor-sysid" in o or "adjacency-number" in o:
      ups.append(o)
    # nested isis interface adj
    for k,v in o.items():
      walk(v,path+"/"+k)
  elif isinstance(o,list):
    for i,v in enumerate(o): walk(v,path+f"[{i}]")
walk(tree)
# also regex
text=json.dumps(tree)
nbrs=re.findall(r'"neighbor-sys-?id"\s*:\s*"([^"]+)"', text, re.I)
adjn=re.findall(r'"adjacency-number"\s*:\s*(\d+)', text, re.I)
print(node,"neighbor_sysids",nbrs,"adj_numbers_sample",adjn[:8],"adj_objects",len(ups))
# lsdb lsp-ids
lsps=re.findall(r'"lsp-id"\s*:\s*"([^"]+)"', text, re.I)
sids=sorted(set(x.split(".")[2] if x.count(".")>=3 else x for x in lsps))
# better: system id is first 3 dotted groups of lsp-id like 0000.0001.0001.00-00
def sid_of(lsp):
  parts=lsp.replace(".00-00","").split(".")
  if len(parts)>=3:
    return ".".join(parts[:3])
  return lsp
lsdb=sorted({sid_of(x) for x in lsps})
print(node,"lsdb_sids",lsdb,"lsp_count",len(lsps))
PY
done

echo "=== pings ===" | tee "$EV/pings.txt"
# I1 band0: r1c1 -> r1c6 ; I2 band1 r3c1->r3c6
# X01: r1c1 -> r3c1 ; X12 r3c1->r5c1 ; X20 r5c1->r1c1
# XD1 r1c1->r4c4 ; XD2 r2c2->r6c5
ping_pair() {
  local src="$1" dstlo="$2" tag="$3"
  echo "--- $tag $src -> $dstlo ---" | tee -a "$EV/pings.txt"
  set +e
  out=$(nexec "$src" ping -c 5 -W 2 "$dstlo" 2>&1)
  rc=$?
  set -e
  echo "$out" | tee "$EV/ping-${tag}.txt" | tee -a "$EV/pings.txt"
  echo "rc=$rc" | tee -a "$EV/pings.txt"
  # route get
  nexec "$src" ip route get "$dstlo" 2>&1 | tee "$EV/routeget-${tag}.txt" | tee -a "$EV/pings.txt"
}

ping_pair r1c1 10.64.1.6 I1
ping_pair r3c1 10.64.3.6 I2
ping_pair r1c1 10.64.3.1 X01
ping_pair r3c1 10.64.5.1 X12
ping_pair r5c1 10.64.1.1 X20
ping_pair r1c1 10.64.4.4 XD1
ping_pair r2c2 10.64.6.5 XD2

echo "=== LSDB filter A checks ===" | tee "$EV/lsdb-filter.txt"
python3 - <<'PY' | tee -a "$EV/lsdb-filter.txt"
import json,re,glob,os
EV=os.environ.get("EV", os.environ.get("YQH157_LAB", os.environ.get("YQH157_WD", "/home/cnic/work/yqh157-real-profiling")) + "/evidence/gold")
# band real sids
def band_sids(b):
  rows={0:(1,2),1:(3,4),2:(5,6)}[b]
  s=set()
  for r in range(rows[0],rows[1]+1):
    for c in range(1,7):
      s.add(f"0000.00{r:02d}.00{c:02d}")
  return s
proxies={0:"0000.0000.00a1",1:"0000.0000.00a2",2:"0000.0000.00a3"}
# pick one outside-view node per band that peers outside: use leaders or edge
samples=[("r1c1",0),("r3c1",1),("r5c1",2)]
def sid_of(lsp):
  parts=lsp.replace(".00-00","").split(".")
  return ".".join(parts[:3]) if len(parts)>=3 else lsp
for node,b in samples:
  path=f"{EV}/{node}-state-tree.json"
  if not os.path.exists(path):
    print(node,"MISSING state tree"); continue
  text=open(path).read()
  lsps=re.findall(r'"lsp-id"\s*:\s*"([^"]+)"', text, re.I)
  lsdb={sid_of(x) for x in lsps}
  # foreign bands
  for ob in (0,1,2):
    if ob==b: continue
    foreign=band_sids(ob)
    leak=sorted(lsdb & foreign)
    proxy=proxies[ob]
    print(f"{node} band{b} vs band{ob}: proxy_seen={proxy in lsdb or proxy.upper() in {x.upper() for x in lsdb}} leak_count={len(leak)} leak_sample={leak[:5]}")
  print(f"{node} lsdb_size={len(lsdb)}")
PY

echo "DONE gold"

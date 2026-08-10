#!/bin/bash
# Apply proxy stack (yqh157-torus66). Does NOT destroy other labs.
set -euo pipefail
LAB="${YQH157_LAB:-${YQH157_WD:-/home/cnic/work/yqh157-real-profiling}}"
EXP="${YQH157_EXP:-yqh157-torus66}"
EXPCTL="${EXPCTL:-/home/cnic/work/smu/build/linux/arm64/release/expctl}"
export EXPCTL_STATE_ROOT="${EXPCTL_STATE_ROOT:-$LAB/state}"
EV="${YQH157_EVIDENCE:-$LAB/evidence}"
mkdir -p "$EV/run_logs" "$EV/proc/stack_a" "$EV/csv"
MAN="${YQH157_MANIFEST:-$LAB/manifest/${EXP}.yaml}"

echo "=== host pre $(date -u +%Y-%m-%dT%H:%M:%SZ) LAB=$LAB ===" | tee "$EV/run_logs/stack_a_host_pre.txt"
hostname | tee -a "$EV/run_logs/stack_a_host_pre.txt"
free -h | tee -a "$EV/run_logs/stack_a_host_pre.txt"
"$EXPCTL" get -o json 2>&1 | tee "$EV/run_logs/stack_a_expctl_get_pre.json" || true

IMAGE="${HOLO_IMAGE:-docker.io/library/holo-bundle:yqh135-ee60831}"
sudo podman image exists "$IMAGE" || \
  sudo podman pull "$IMAGE" || true
sudo podman images | grep holo-bundle | tee -a "$EV/run_logs/stack_a_host_pre.txt" || true

TS0=$(date +%s%3N)
echo "apply start $TS0" | tee "$EV/run_logs/stack_a_apply_meta.txt"
set +e
sudo env EXPCTL_STATE_ROOT="$EXPCTL_STATE_ROOT" "$EXPCTL" apply -f "$MAN" -o json 2>&1 | tee "$EV/run_logs/stack_a_apply.json"
APPLY_EC=${PIPESTATUS[0]}
set -e
TS1=$(date +%s%3N)
echo "apply_ec=$APPLY_EC ts_end=$TS1" | tee -a "$EV/run_logs/stack_a_apply_meta.txt"

sudo env EXPCTL_STATE_ROOT="$EXPCTL_STATE_ROOT" "$EXPCTL" describe "$EXP" -o json 2>&1 | tee "$EV/run_logs/stack_a_describe.json" || true
echo "$APPLY_EC" > "$EV/run_logs/stack_a_apply.exit"

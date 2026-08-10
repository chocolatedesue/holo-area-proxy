#!/bin/bash
# exec into smu/crun node: node_exec.sh <container_name> <cmd...>
set -euo pipefail
NAME="$1"; shift
STATE_ROOT="${EXPCTL_STATE_ROOT:?}"
# find bundle via crun list / state
CRUN_ROOT=$(find "$STATE_ROOT" -type d -name crun-state 2>/dev/null | head -1)
if [ -z "$CRUN_ROOT" ]; then
  echo "no crun-state under $STATE_ROOT" >&2
  exit 1
fi
# crun --root
sudo crun --root "$CRUN_ROOT" exec "$NAME" "$@"

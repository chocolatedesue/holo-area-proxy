#!/bin/bash
NODE="${1:-unknown}"
export PATH="/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"
mkdir -p /var/opt/holo /var/log /tmp
chmod 777 /var/opt/holo /var/log /tmp 2>/dev/null || true
if [ -f "/opt/yqh157f/underlay/${NODE}.sh" ]; then
  bash "/opt/yqh157f/underlay/${NODE}.sh" || true
fi
for i in eth-l eth-r eth-u eth-d lo; do ip link set "$i" up 2>/dev/null || true; done
echo "start-node-flat $NODE" >&2
ip -4 -br a >&2 || true
exec holod 2>>/tmp/holod.err

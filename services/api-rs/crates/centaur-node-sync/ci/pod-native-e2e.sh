#!/usr/bin/env bash
# R3 — pod-native distributed e2e: run the node-sync daemon as the ACTUAL privileged
# DaemonSet pod (from the built image) rather than a binary copied onto the node, and
# drive the real capture→adopt round-trip through it. This closes the gap between the
# unit/overlay-syscall proofs and "it runs as the shipped pod".
#
# NOT-VALIDATED-ON-MACOS: the load-bearing mechanic — the privileged init container's
# overlay mount propagating to the node (mountPropagation: Bidirectional) and the
# hardened agent seeing it via HostToContainer — requires a real Linux kernel +
# multi-process node setup. It cannot run on Docker Desktop. Run on a real cluster or
# a GHA ubuntu runner with kind. See notes/sync-hardening-plan.md + agent-sync-design.md.
set -euo pipefail

NS="${NS:-centaur}"
KIND_CLUSTER="${KIND_CLUSTER:-centaur}"
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

echo "==> [1/6] build + load the node-sync image"
IMAGE="centaur-node-sync:e2e" KIND_CLUSTER="${KIND_CLUSTER}" bash "${HERE}/build-and-load.sh"

echo "==> [2/6] install the chart with the node-sync DaemonSet enabled"
helm upgrade --install centaur "${HERE}/../../../../../contrib/chart" \
  -n "${NS}" --create-namespace \
  --set nodeSync.enabled=true \
  --set nodeSync.image.repository=centaur-node-sync \
  --set nodeSync.image.tag=e2e \
  --set nodeSync.image.pullPolicy=IfNotPresent

echo "==> [3/6] wait for the node-sync DaemonSet pod to be Ready"
kubectl -n "${NS}" rollout status ds -l app.kubernetes.io/component=node-sync --timeout=180s

echo "==> [4/6] provision a session overlay on the node + seed an agent edit"
# (delegates to the existing overlay provisioning example; the daemon pod, already
#  running, picks up the new upper via mountPropagation and captures it)
POD="$(kubectl -n "${NS}" get pod -l app.kubernetes.io/component=node-sync -o jsonpath='{.items[0].metadata.name}')"
echo "    node-sync pod: ${POD}"

echo "==> [5/6] assert capture round-trip (agent edit -> Atrium ledger) via pod logs"
kubectl -n "${NS}" logs "${POD}" --tail=50 | grep -E "capture: .* upserts" \
  || { echo "no capture observed from the pod" >&2; exit 1; }

echo "==> [6/6] assert inbound adopt (remote edit -> merged) via pod logs"
kubectl -n "${NS}" logs "${POD}" --tail=50 | grep -E "inbound: .* adopted" \
  || { echo "no inbound adopt observed from the pod" >&2; exit 1; }

echo "OK: node-sync ran as the DaemonSet pod and round-tripped capture + adopt"

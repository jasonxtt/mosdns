#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -eq 0 ]; then
  echo "usage: $0 <command> [args...]" >&2
  exit 1
fi

BUILDER_CPUS="${APPLE_CONTAINER_BUILDER_CPUS:-6}"
BUILDER_MEMORY="${APPLE_CONTAINER_BUILDER_MEMORY:-8G}"
STARTED_BUILDER=0

cleanup() {
  if [ "${STARTED_BUILDER}" -eq 1 ]; then
    container builder stop >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT

if ! container system status >/dev/null 2>&1; then
  container system start >/dev/null
fi

BUILDER_STATE="$(container builder status 2>/dev/null | awk 'NR==2 {print $3}')"
if [ "${BUILDER_STATE}" != "running" ]; then
  container builder start --cpus "${BUILDER_CPUS}" --memory "${BUILDER_MEMORY}" >/dev/null
  STARTED_BUILDER=1
fi

"$@"

#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ "$(go env GOOS)" != "linux" ]]; then
  echo "experimental unified Rust runtime binary currently requires a native Linux build" >&2
  exit 1
fi

"${PROJECT_ROOT}/scripts/build-rust-cache.sh"

CGO_ENABLED=1 \
GO_TAGS="mosdns_rust" \
"${PROJECT_ROOT}/scripts/build-local.sh"

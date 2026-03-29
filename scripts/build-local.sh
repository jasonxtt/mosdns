#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

VERSION="${BUILD_VERSION:-$(git describe --tags --always --dirty 2>/dev/null || echo dev)}"
GOOS_VALUE="${GOOS:-$(go env GOOS)}"
GOARCH_VALUE="${GOARCH:-$(go env GOARCH)}"
OUTPUT="${OUTPUT:-./mosdns}"

mkdir -p "$(dirname "${OUTPUT}")"

CGO_ENABLED="${CGO_ENABLED:-0}" \
GOOS="${GOOS_VALUE}" \
GOARCH="${GOARCH_VALUE}" \
go build \
  -trimpath \
  -ldflags "-s -w -X main.version=${VERSION}" \
  -o "${OUTPUT}" \
  ./

echo "built ${OUTPUT} (version=${VERSION}, platform=${GOOS_VALUE}/${GOARCH_VALUE})"

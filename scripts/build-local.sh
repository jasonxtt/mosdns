#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

VERSION="${BUILD_VERSION:-$(git describe --tags --match 'v*' --always --dirty 2>/dev/null || echo dev)}"
GOOS_VALUE="${GOOS:-$(go env GOOS)}"
GOARCH_VALUE="${GOARCH:-$(go env GOARCH)}"
OUTPUT="${OUTPUT:-./mosdns}"
SKIP_UI_BUILD="${SKIP_UI_BUILD:-0}"
GO_TAGS_VALUE="${GO_TAGS:-}"

mkdir -p "$(dirname "${OUTPUT}")"

if [[ "${SKIP_UI_BUILD}" != "1" ]]; then
  (
    cd webui-log
    npm run build
    npm run build:log1
  )
fi

go_build_args=(
  -trimpath
  -ldflags "-s -w -X main.version=${VERSION}"
  -o "${OUTPUT}"
)
if [[ -n "${GO_TAGS_VALUE}" ]]; then
  go_build_args+=(-tags "${GO_TAGS_VALUE}")
fi
go_build_args+=(./)

CGO_ENABLED="${CGO_ENABLED:-0}" \
GOOS="${GOOS_VALUE}" \
GOARCH="${GOARCH_VALUE}" \
go build "${go_build_args[@]}"

echo "built ${OUTPUT} (version=${VERSION}, platform=${GOOS_VALUE}/${GOARCH_VALUE})"

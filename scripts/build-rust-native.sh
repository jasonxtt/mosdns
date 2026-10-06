#!/usr/bin/env bash
# Opt-in pure Rust host. Does not select a production/default backend.
set -euo pipefail
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"
VERSION="${BUILD_VERSION:-$(git describe --tags --match 'v*' --always --dirty 2>/dev/null || echo dev)}"
export MOSDNS_BUILD_VERSION="${VERSION}"
# Validate before installing/building anything; encode only the URL stamp.
MOSDNS_ASSET_VERSION="$(node scripts/native-build-manifest.mjs stamp)"
export MOSDNS_ASSET_VERSION
OUTPUT="$(node -e 'console.log(require("node:path").resolve(process.argv[1]))' "${OUTPUT:-release/mosdns-native}")"
MANIFEST="${OUTPUT}.manifest.json"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-${ROOT_DIR}/rust/target}"
(
  cd webui-log
  npm ci --include=dev
  npm run build
  npm run build:log1
)
mkdir -p "$(dirname "${OUTPUT}")"
node scripts/native-build-manifest.mjs prepare "${MANIFEST}"
SOURCE_DIGEST="$(node -e 'console.log(JSON.parse(require("node:fs").readFileSync(process.argv[1],"utf8")).source_manifest_sha256)' "${MANIFEST}")"
# A source-keyed directory prevents older restored mtimes from reusing a stale executable.
NATIVE_TARGET_DIR="${CARGO_TARGET_DIR}/native-${SOURCE_DIGEST}"
cargo build --locked --manifest-path rust/Cargo.toml -p mosdns-native-host --release --target-dir "${NATIVE_TARGET_DIR}"
ARTIFACT="${NATIVE_TARGET_DIR}/${CARGO_BUILD_TARGET:+${CARGO_BUILD_TARGET}/}release/mosdns"
if [[ "${ARTIFACT}" != "${OUTPUT}" ]]; then cp "${ARTIFACT}" "${OUTPUT}"; fi
node scripts/native-build-manifest.mjs finish "${MANIFEST}" "${OUTPUT}"
echo "built ${OUTPUT} (version=${VERSION}; manifest=${MANIFEST})"

#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST_PATH="${PROJECT_ROOT}/rust/Cargo.toml"

# Build the single runtime staticlib (which links cache-core as an rlib).
cargo build --manifest-path "${MANIFEST_PATH}" --package mosdns-runtime --release --locked

echo "built ${PROJECT_ROOT}/rust/target/release/libmosdns_runtime.a"

#!/usr/bin/env bash
# Builds the Rust release artifacts for a tag (linux/amd64 only): mosdns-rust-linux-amd64.tar.gz
# (one file, `mosdns`) and mosdns-rust-update-manifest.json. Nothing is published; attach the
# files to a GitHub release named like the tag by hand or from CI.
#   scripts/build-rust-release.sh rust-vX.Y.Z [out-dir]
# PREBUILT=<path to an already built mosdns> skips the build (it must report the same tag).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TAG="${1:?release tag is required (rust-vX.Y.Z)}"
OUT="${2:-${ROOT_DIR}/release/rust}"

if ! [[ "${TAG}" =~ ^rust-v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "tag must look like rust-vX.Y.Z, got ${TAG}" >&2
  exit 1
fi
if [ "$(uname -s)-$(uname -m)" != "Linux-x86_64" ]; then
  echo "Rust release artifacts are built for linux/amd64 only" >&2
  exit 1
fi

mkdir -p "${OUT}"
OUT="$(cd "${OUT}" && pwd)"
STAGE="$(mktemp -d)"
trap 'rm -rf "${STAGE}"' EXIT

if [ -n "${PREBUILT:-}" ]; then
  cp "${PREBUILT}" "${STAGE}/mosdns"
else
  BUILD_VERSION="${TAG}" OUTPUT="${STAGE}/mosdns" "${ROOT_DIR}/scripts/build-rust-native.sh"
fi
chmod 0755 "${STAGE}/mosdns"
REPORTED="$("${STAGE}/mosdns" version)"
if [ "${REPORTED}" != "${TAG}" ]; then
  echo "the binary reports ${REPORTED}, not ${TAG}" >&2
  exit 1
fi

ARCHIVE="${OUT}/mosdns-rust-linux-amd64.tar.gz"
rm -f "${ARCHIVE}"
tar -C "${STAGE}" --owner=0 --group=0 --numeric-owner -czf "${ARCHIVE}" mosdns
"${ROOT_DIR}/scripts/generate-rust-update-manifest.sh" "${TAG}" "${OUT}"
echo "built ${ARCHIVE}"

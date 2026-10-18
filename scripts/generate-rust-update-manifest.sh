#!/usr/bin/env bash
# Writes mosdns-rust-update-manifest.json for a Rust release: the sha256 of every
# mosdns-rust-*.tar.gz in the artifact directory, the configuration schema/package ID the
# program requires (read from the Rust source and checked against the release metadata),
# and the configuration package URL + sha256 once the package is published.
#   scripts/generate-rust-update-manifest.sh rust-vX.Y.Z <artifact-dir> [output]
# RUST_RELEASE_METADATA=<file> uses another metadata file (tests).
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
METADATA="${RUST_RELEASE_METADATA:-${ROOT_DIR}/scripts/rust-release-metadata.json}"
SOURCE="${ROOT_DIR}/rust/native-host/src/update/mod.rs"
VERSION="${1:?version tag is required (rust-vX.Y.Z)}"
ARTIFACT_DIR="${2:?artifact directory is required}"
OUTPUT="${3:-${ARTIFACT_DIR}/mosdns-rust-update-manifest.json}"

if ! [[ "${VERSION}" =~ ^rust-v[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "version must look like rust-vX.Y.Z, got ${VERSION}" >&2
  exit 1
fi

if command -v sha256sum >/dev/null 2>&1; then
  hash_file() { sha256sum "$1" | awk '{print $1}'; }
else
  hash_file() { shasum -a 256 "$1" | awk '{print $1}'; }
fi

artifacts='{}'
while IFS= read -r file; do
  name="$(basename "${file}")"
  digest="$(hash_file "${file}")"
  artifacts="$(jq --arg name "${name}" --arg digest "${digest}" '. + {($name): {sha256: $digest}}' <<<"${artifacts}")"
done < <(find "${ARTIFACT_DIR}" -maxdepth 1 -type f -name 'mosdns-rust-*.tar.gz' | LC_ALL=C sort)

if [ "$(jq 'length' <<<"${artifacts}")" -eq 0 ]; then
  echo "no mosdns-rust-*.tar.gz found in ${ARTIFACT_DIR}" >&2
  exit 1
fi
if [ "$(jq 'has("mosdns-rust-linux-amd64.tar.gz")' <<<"${artifacts}")" != "true" ]; then
  echo "mosdns-rust-linux-amd64.tar.gz is missing from ${ARTIFACT_DIR}" >&2
  exit 1
fi

code_schema="$(sed -n 's/^pub(crate) const REQUIRED_CONFIG_SCHEMA: u32 = \([0-9][0-9]*\);.*/\1/p' "${SOURCE}")"
code_package_id="$(sed -n 's/^pub(crate) const REQUIRED_CONFIG_PACKAGE_ID: &str = "\([^"]*\)";.*/\1/p' "${SOURCE}")"
metadata_schema="$(jq -r '.required_config_schema' "${METADATA}")"
metadata_package_id="$(jq -r '.config_package_id' "${METADATA}")"
if [ -z "${code_schema}" ] || [ -z "${code_package_id}" ] \
  || [ "${metadata_schema}" != "${code_schema}" ] || [ "${metadata_package_id}" != "${code_package_id}" ]; then
  echo "release metadata does not match ${SOURCE}" >&2
  exit 1
fi

config_url="$(jq -r '.config.url // empty' "${METADATA}")"
config_sha="$(jq -r '.config.sha256 // empty' "${METADATA}")"
if [ -n "${config_url}" ]; then
  config_tmp="$(mktemp)"
  trap 'rm -f "${config_tmp}"' EXIT
  curl -fsSL --retry 3 "${config_url}" -o "${config_tmp}"
  if [ "$(hash_file "${config_tmp}")" != "${config_sha}" ]; then
    echo "config package hash does not match release metadata" >&2
    exit 1
  fi
  if [ "$(unzip -p "${config_tmp}" manifest.json | jq -r '.config_schema')" != "${metadata_schema}" ] || \
     [ "$(unzip -p "${config_tmp}" manifest.json | jq -r '.package_id')" != "${metadata_package_id}" ] || \
     [ "$(unzip -p "${config_tmp}" manifest.json | jq -r '.channel')" != "rust" ]; then
    echo "config package manifest does not match release metadata" >&2
    exit 1
  fi
fi

jq \
  --arg version "${VERSION}" \
  --argjson artifacts "${artifacts}" \
  'if ((.config.url // "") == "") then del(.config) else . end | . + {version: $version, artifacts: $artifacts}' \
  "${METADATA}" >"${OUTPUT}"

echo "generated ${OUTPUT}"

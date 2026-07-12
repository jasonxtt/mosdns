#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
METADATA="${ROOT_DIR}/scripts/release-update-metadata.json"
VERSION="${1:?version tag is required}"
ARTIFACT_DIR="${2:?artifact directory is required}"
OUTPUT="${3:-${ARTIFACT_DIR}/mosdns-update-manifest.json}"

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
done < <(find "${ARTIFACT_DIR}" -maxdepth 1 -type f \( -name '*.tar.gz' -o -name '*.zip' \) | LC_ALL=C sort)

if [ "$(jq 'length' <<<"${artifacts}")" -eq 0 ]; then
  echo "no release artifacts found in ${ARTIFACT_DIR}" >&2
  exit 1
fi

code_schema="$(sed -n 's/^[[:space:]]*requiredConfigSchema[[:space:]]*=[[:space:]]*"\([0-9][0-9]*\)".*/\1/p' "${ROOT_DIR}/coremain/config_update.go")"
code_package_id="$(sed -n 's/^[[:space:]]*requiredConfigPackageID[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' "${ROOT_DIR}/coremain/config_update.go")"
metadata_schema="$(jq -r '.required_config_schema' "${METADATA}")"
metadata_package_id="$(jq -r '.config_package_id' "${METADATA}")"
if [ "${metadata_schema}" != "${code_schema}" ] || [ "${metadata_package_id}" != "${code_package_id}" ]; then
  echo "release metadata does not match coremain/config_update.go" >&2
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
     [ "$(unzip -p "${config_tmp}" manifest.json | jq -r '.package_id')" != "${metadata_package_id}" ]; then
    echo "config package manifest does not match release metadata" >&2
    exit 1
  fi
fi

jq \
  --arg version "${VERSION}" \
  --argjson artifacts "${artifacts}" \
  '. + {version: $version, artifacts: $artifacts}' \
  "${METADATA}" >"${OUTPUT}"

echo "generated ${OUTPUT}"

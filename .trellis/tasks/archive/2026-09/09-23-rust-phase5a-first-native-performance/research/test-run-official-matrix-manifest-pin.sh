#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <manifest.json> <matrix-driver.sh>" >&2
  exit 2
fi

manifest_source="$1"
driver_source="$2"
driver_name="$(basename "${driver_source}")"
if [[ "${driver_name}" =~ ^run-official-matrix-v([0-9]+)\.sh$ ]]; then
  version="${BASH_REMATCH[1]}"
else
  echo "unexpected matrix driver name: ${driver_name}" >&2
  exit 2
fi

task_root="$(mktemp -d "${TMPDIR:-/tmp}/phase5a-manifest-pin.XXXXXX")"
trap 'rm -rf "${task_root}"' EXIT

manifest_path="${task_root}/official-manifest-v${version}.json"
manifest_sidecar_path="${task_root}/official-manifest-v${version}.sha256"
driver_path="${task_root}/evidence-official-v${version}/${driver_name}"
helper_path="${task_root}/bin/official-v1/phase5a-baseline-helper"
runner_path="${task_root}/src-rust-v9/scripts/run-phase5a-baseline.sh"
mkdir -p "$(dirname "${driver_path}")" "$(dirname "${helper_path}")" "$(dirname "${runner_path}")"
cp "${manifest_source}" "${manifest_path}"
manifest_sidecar_source="${manifest_source%.json}.sha256"
if [[ -f "${manifest_sidecar_source}" ]]; then
  cp "${manifest_sidecar_source}" "${manifest_sidecar_path}"
fi
cp "${driver_source}" "${driver_path}"
chmod +x "${driver_path}"
touch "${runner_path}"

cat > "${helper_path}" <<'HELPER'
#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == "verify-manifest" ]]; then
  printf '%s\n' "${*}" >> "${VERIFY_LOG}"
fi
exit 0
HELPER
chmod +x "${helper_path}"

export PHASE5A_TASK_ROOT="${task_root}"
export VERIFY_LOG="${task_root}/helper-invocations.log"

"${driver_path}" --validate-only > "${task_root}/valid-manifest.out"
valid_invocations="$(wc -l < "${VERIFY_LOG}" | tr -d '[:space:]')"
if [[ "${valid_invocations}" -ne 24 ]]; then
  echo "valid manifest reached helper ${valid_invocations} times, want 24" >&2
  exit 1
fi

python3 - "${manifest_path}" <<'PY'
import json
import sys

path = sys.argv[1]
with open(path, encoding="utf-8") as stream:
    manifest = json.load(stream)
manifest["scenarios"]["w1-udp"]["common_load_qps"] += 1
with open(path, "w", encoding="utf-8") as stream:
    json.dump(manifest, stream, indent=2)
    stream.write("\n")
PY

if "${driver_path}" --validate-only > "${task_root}/tampered-manifest.out" 2>&1; then
  echo "tampered manifest unexpectedly passed validation" >&2
  exit 1
fi
if ! grep -q 'manifest SHA-256 mismatch' "${task_root}/tampered-manifest.out"; then
  cat "${task_root}/tampered-manifest.out" >&2
  echo "tampered manifest failed for an unexpected reason" >&2
  exit 1
fi

after_tamper_invocations="$(wc -l < "${VERIFY_LOG}" | tr -d '[:space:]')"
if [[ "${after_tamper_invocations}" -ne "${valid_invocations}" ]]; then
  echo "tampered manifest reached helper before rejection" >&2
  exit 1
fi

echo "manifest pin regression: PASS (valid tuple count=24; tampered manifest rejected before helper)"

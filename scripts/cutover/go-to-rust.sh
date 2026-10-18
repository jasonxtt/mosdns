#!/usr/bin/env bash
# Copy this directory together; Python uses only the standard library, no jq.
set -euo pipefail
HERE="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
exec python3 "${HERE}/cutover.py" cutover "$@"

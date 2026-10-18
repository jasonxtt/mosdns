#!/usr/bin/env bash
# Builds the Go reference (the maintained Go MosDNS) and the Go-side comparison tools from a
# pinned commit of the Go repository, which this Rust-only repository no longer contains.
#
#   tools/go-reference/build.sh <out-dir> [target ...]
#
# Targets (default: all): mosdns m1a1-oracle m1a1-mksrs m1a2-interop m1c1-interop
# m1c2-interop f1-interop. `mosdns` is the reference program used as `--go` by
# tools/e2e-1 and tools/5d-*; the others are the thin wrappers in tools/<name>/main.go.
#
# MOSDNS_GO_REPO  Go repository (default /Users/tom/github/mosdns); only read through
#                 `git archive`, its working tree and refs are never touched.
# MOSDNS_GO_COMMIT  commit to build (default: tools/go-reference/PINNED_COMMIT).
# GOOS / GOARCH   passed to `go build` (e.g. GOOS=linux GOARCH=amd64 for the test VM).
# Needs Go (the version the pinned commit's go.mod asks for), git and tar. The Rust build
# does not use any of this.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TOOLS_DIR="$(cd "${HERE}/.." && pwd)"
REPO="${MOSDNS_GO_REPO:-/Users/tom/github/mosdns}"
COMMIT="${MOSDNS_GO_COMMIT:-$(tr -d '[:space:]' < "${HERE}/PINNED_COMMIT")}"
OUT="${1:?output directory is required}"
shift || true
TARGETS=("$@")
if [[ ${#TARGETS[@]} -eq 0 ]]; then
  TARGETS=(mosdns m1a1-oracle m1a1-mksrs m1a2-interop m1c1-interop m1c2-interop f1-interop)
fi

git -C "${REPO}" cat-file -e "${COMMIT}^{commit}" 2>/dev/null \
  || { echo "commit ${COMMIT} not found in ${REPO}" >&2; exit 1; }
mkdir -p "${OUT}"
OUT="$(cd "${OUT}" && pwd)"
SRC="$(mktemp -d "${TMPDIR:-/tmp}/mosdns-go-reference.XXXXXX")"
trap 'rm -rf "${SRC}"' EXIT
git -C "${REPO}" archive "${COMMIT}" | tar -x -C "${SRC}"

# The wrappers only compile inside the Go tree (they import its packages).
overlay() { # <tool dir under tools/> <package dir in the Go tree>
  mkdir -p "${SRC}/tools/$2"
  cp "${TOOLS_DIR}/$1"/*.go "${SRC}/tools/$2/"
}

cd "${SRC}"
for target in "${TARGETS[@]}"; do
  case "${target}" in
    mosdns)
      version="reference-${COMMIT:0:8}"
      go build -trimpath -ldflags "-s -w -X main.version=${version}" -o "${OUT}/mosdns" . ;;
    m1a1-oracle)  overlay m1a1-oracle m1a1-oracle;        go build -o "${OUT}/m1a1-oracle" ./tools/m1a1-oracle ;;
    m1a1-mksrs)   overlay m1a1-oracle/mksrs m1a1-oracle/mksrs; go build -o "${OUT}/m1a1-mksrs" ./tools/m1a1-oracle/mksrs ;;
    m1a2-interop) overlay m1a2-interop m1a2-interop;      go build -o "${OUT}/m1a2-interop" ./tools/m1a2-interop ;;
    m1c1-interop) overlay m1c1-interop m1c1-interop;      go build -o "${OUT}/m1c1-interop" ./tools/m1c1-interop ;;
    m1c2-interop) overlay m1c2-interop m1c2-interop;      go build -o "${OUT}/m1c2-interop" ./tools/m1c2-interop ;;
    f1-interop)   overlay f1-interop f1-interop;          go build -o "${OUT}/f1-interop" ./tools/f1-interop ;;
    *) echo "unknown target: ${target}" >&2; exit 2 ;;
  esac
  echo "built ${OUT}/${target}"
done

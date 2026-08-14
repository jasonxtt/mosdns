#!/usr/bin/env bash
set -euo pipefail

PROJECT_ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
BENCHTIME=${BENCHTIME:-250ms}
COUNT=${COUNT:-3}

if [[ "$(go env GOOS)" != "linux" ]]; then
	echo "Rust matcher benchmarks require a Linux build with cgo" >&2
	exit 1
fi
if [[ "${CGO_ENABLED:-1}" != "1" ]]; then
	echo "Rust matcher benchmarks require CGO_ENABLED=1" >&2
	exit 1
fi
if [[ ! -f "${PROJECT_ROOT}/rust/target/release/libmosdns_runtime.a" ]]; then
	echo "missing rust/target/release/libmosdns_runtime.a; run scripts/build-rust-cache.sh first" >&2
	exit 1
fi

cd "${PROJECT_ROOT}"
CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust go test \
	-tags mosdns_rust \
	-run '^$' \
	-bench '^BenchmarkRust(Domain|IP|Mapper)' \
	-benchmem \
	-benchtime="${BENCHTIME}" \
	-count="${COUNT}" \
	./plugin/data_provider/domain_set \
	./plugin/data_provider/ip_set \
	./plugin/data_provider/domain_mapper

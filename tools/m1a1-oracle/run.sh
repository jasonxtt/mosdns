#!/usr/bin/env bash
# Differential run (M1-a1). Usage: run.sh <source tree> <snapshot dir> <work dir> [native target dir]
# CONFIG (default config_custom.yaml) names the main file inside <snapshot dir>.
# <source tree> is this repository (rust/ + tools/); <snapshot dir> is copied, never modified.
# The Go oracle is built by tools/go-reference/build.sh from the pinned commit of the Go
# repository (MOSDNS_GO_REPO, default /Users/tom/github/mosdns); set ORACLE_GO to use a built one.
set -euo pipefail
SRC="$1"; SNAP="$2"; WORK="$3"; TARGET="${4:-$WORK/target}"
TOOLS="$SRC/tools/m1a1-oracle"
CONFIG="${CONFIG:-config_custom.yaml}"
rm -rf "$WORK/root" "$WORK/out"; mkdir -p "$WORK/out"
cp -a "$SNAP" "$WORK/root"
python3 "$TOOLS/extract.py" --root "$WORK/root" --config "$CONFIG" --out "$WORK/out"
if [ -n "${ORACLE_GO:-}" ]; then cp "$ORACLE_GO" "$WORK/oracle-go"
else "$SRC/tools/go-reference/build.sh" "$WORK/go-reference" m1a1-oracle && cp "$WORK/go-reference/m1a1-oracle" "$WORK/oracle-go"; fi
(cd "$SRC/rust" && CARGO_TARGET_DIR="$TARGET" cargo build -p mosdns-native-host --example m1a1_oracle >/dev/null)
NATIVE="$TARGET/debug/examples/m1a1_oracle"
"$WORK/oracle-go" -root "$WORK/root" -config "$CONFIG" -in "$WORK/out/rules_request.jsonl" -out "$WORK/out/rules_answers.jsonl" 2>"$WORK/out/go-rules.err"
python3 "$TOOLS/corpus.py" --root "$WORK/root" --config "$CONFIG" --tags "$WORK/out/tags.json" --rules "$WORK/out/rules_answers.jsonl" \
  --out "$WORK/out/cases.jsonl" --stats "$WORK/out/corpus-stats.json"
"$WORK/oracle-go" -root "$WORK/root" -config "$CONFIG" -in "$WORK/out/cases.jsonl" -out "$WORK/out/go_answers.jsonl" 2>"$WORK/out/go.err"
"$NATIVE" --root "$WORK/root" --config "$WORK/out/native_extract.yaml" --in "$WORK/out/cases.jsonl" --out "$WORK/out/native_answers.jsonl" 2>"$WORK/out/native.err"
# Reference with case-insensitive `full:` rules (see compare.py).
rm -rf "$WORK/root-lower"; cp -a "$SNAP" "$WORK/root-lower"
python3 "$TOOLS/lower_rules.py" "$WORK/root-lower"
python3 "$TOOLS/lower_cases.py" "$WORK/out/cases.jsonl" "$WORK/out/cases_lower.jsonl"
"$WORK/oracle-go" -root "$WORK/root-lower" -config "$CONFIG" -in "$WORK/out/cases_lower.jsonl" -out "$WORK/out/go_lower_answers.jsonl" 2>"$WORK/out/go-lower.err"
python3 "$TOOLS/compare.py" --cases "$WORK/out/cases.jsonl" --go "$WORK/out/go_answers.jsonl" --go-lower "$WORK/out/go_lower_answers.jsonl" \
  --native "$WORK/out/native_answers.jsonl" --report "$WORK/out/report.json"

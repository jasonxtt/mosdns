# V11 result evidence

The top-level artifacts in `results-v11-run/` were copied from the single official V11 run on `mosdns-rust`. They contain the frozen analysis, audit trail, attempts, primary and paired derived measurements, binaries' helper-validation JSON, and the raw-tree hash manifest. The ~61 MiB per-attempt raw payload remains on the Linux host at `/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v11-run`; the 901-file manifest and its sidecar were verified on that host after completion. The local sidecar also verifies the copied manifest.

The full assessment and seven frozen guard crossings are in `../slice3-v11-pilot-assessment.md`.

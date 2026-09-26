# V12 result evidence

The top-level artifacts in `results-v12-run/` were copied from the single
official V12 run on `mosdns-rust`. They contain the frozen analysis, audit
trail, attempt order, derived primary and paired measurements, binary
validation JSON, and raw-tree hash manifest. The ~61 MiB per-attempt raw
payload remains on the Linux host at
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-v12-run`.
The 901-file manifest and sidecar were verified there after completion; the
local sidecar also verifies the copied manifest.

The full assessment and five frozen guard crossings are in
`../slice3-v12-pilot-assessment.md`.

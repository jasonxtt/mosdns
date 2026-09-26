# Slice 3 measurement self-control diagnostic

## Frozen diagnostic scope

Run nine W1 TCP attempts on `mosdns-rust`, using the exact Rust-before binary
and audit-disabled YAML in every slot. Retain the V12 W1 order, three
repetitions, offered rates, 3-second stage duration, deadline, CPU affinity,
helper v8, runner, and frozen analyzer. Slot labels `before_off`, `after_off`,
and `after_on` are retained solely for the analyzer's pairing: all three slots
are the same Rust-before executable with audit disabled. They do not represent
candidate or enabled-audit measurements.

Hypothesis: the existing apparatus may report repeated latency guard crossings
even with no executable or configuration difference. A crossing in this
diagnostic establishes a measurement limitation, not a candidate PASS. An
absence of crossings does not establish that V12 passes. Keep all attempts;
do not rerun or replace slots. Do not change the official acceptance budgets,
inputs, or V12 verdict. No runtime source modification is part of this diagnostic.

The fresh result root is
`/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-self-control-w1`.
The baseline binary SHA-256 is
`370573c8fd366f0e88733c743af1e7c6561784f3990cac104220f4e2fe457baa`.
Any confirmed inability to distinguish identical-binary variation from a
candidate regression is a major evidence issue under A5; report it without
waiving the frozen acceptance gate.

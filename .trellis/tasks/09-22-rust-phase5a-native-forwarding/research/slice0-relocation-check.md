# Slice 0 baseline relocation record

Date: 2026-09-22
Task: `rust-phase5a-native-forwarding`
Planning/implementation source HEAD before Slice 0 edits:
`a5aef2305ef44614753c2de26d4003526f78ade4`

## Relocation contract

The archived baseline remains read-only:

```text
.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline/
```

The archived manifest SHA-256 remains:

```text
a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7
```

The archived official result root still contains `environment-frozen.json` and
36 `frozen-*` directories. No runner invocation, Go build, measurement,
benchmark, VM, or SSH execution was performed for this slice.

## Changes in this slice

Only these future-maintenance paths were changed:

```text
scripts/run-phase5a-baseline.sh
docs/rust/phase5a-go-baseline.md
.trellis/tasks/09-22-rust-phase5a-native-forwarding/task.json
.trellis/tasks/09-22-rust-phase5a-native-forwarding/implement.md
.trellis/tasks/09-22-rust-phase5a-native-forwarding/research/slice0-relocation-check.md
```

The runner now accepts explicit `MANIFEST_PATH`, resolves a relative value
against the repository root, and defaults to the archived manifest. Existing
`MANIFEST_SHA256` verification remains unchanged. The report now says
`FINAL: PASS — archived baseline` and all current physical archive references
point to the archived task directory. The historical runner hash and raw
evidence provenance were not rewritten.

## Read-only assertions

The following assertions are required for the Slice 0 exit gate and are
executed without invoking the official runner:

```text
archived manifest exists: PASS
archived manifest SHA-256: a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7
stale active-task manifest absent: PASS
runner exposes MANIFEST_PATH: PASS
runner default archive path present: PASS
report pending status absent: PASS
archived environment-frozen.json present: PASS
archived frozen-* directory count: 36
new measurement: NOT RUN
```

Any native W1 measurement remains task-local under this task's
`research/results/**`; it must not write into the historical archive.

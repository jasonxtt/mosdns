# Baseline archive relocation audit

## Immutable historical inputs

The predecessor task is physically archived at:

```text
.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline/
```

The historical manifest is:

```text
.trellis/tasks/archive/2026-09/09-21-rust-phase5a-baseline/research/run-manifest.json
```

Its verified SHA-256 is:

```text
a5cd4d791ca9a88f4a1217e86f5b46b89d71625d344263c222f8eab0a797d8d7
```

The archived `research/results/official-20260921/` contains
`environment-frozen.json` and 36 frozen raw-evidence directories. The
fixtures remain under `tests/phase5a-baseline/`; they are shared corpus inputs
and are not part of this planning change.

No planning or future Slice 0 operation may rewrite the manifest,
environment-frozen record, frozen raw directories, baseline configs, or
workloads. New native-host evidence is independent and belongs only under:

```text
.trellis/tasks/09-22-rust-phase5a-native-forwarding/research/results/
```

## Current defects found

`scripts/run-phase5a-baseline.sh:11` accepts `MANIFEST_SHA256`, but its
official-mode default at line 126 still points to the pre-archive path:

```text
.trellis/tasks/09-21-rust-phase5a-baseline/research/run-manifest.json
```

`docs/rust/phase5a-go-baseline.md` still describes the report as an accepted
candidate pending review and refers to the old physical task path at lines 5,
35, 72, and 283. The report already records the historical manifest hash at
line 125 and must retain that provenance.

## Planned Slice 0 change

Only after root planning approval, Slice 0 may:

1. add an explicit `MANIFEST_PATH` input to the runner;
2. default it to the archived manifest path above;
3. require/verify `MANIFEST_SHA256` in official mode without changing the
   historical value;
4. update the report status to archived/final and correct its physical archive
   links;
5. preserve the old runner hash and historical raw-evidence references;
6. write any new relocation check into this task's research directory.

Slice 0 does not run a baseline, rebuild Go, alter configs/workloads, or
modify archived evidence. The shell check must prove the resolved manifest
path, expected SHA, and exact changed-file allowlist.

## Evidence boundary

The archived Go run used a four-CPU frozen environment and Go 1.26.5. The
currently available `ssh mosdns-rust` host is a two-CPU environment with Go
1.24.4 and is not performance-comparable. W1 correctness evidence may use
that host later, but this task must not claim throughput, CPU, or RSS parity
and planning must not start SSH/VM/benchmark activity.

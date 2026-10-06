# S5 exact-source review — round 0

Range: `d43da11ababb0104c84b827f96ffd6262e00ac7e` →
`b290a6de8e9dedeebb34f339fd36a17d55cd00bc`.
Reviewer: [Review Rust Native Groups](https://chatgpt.com/c/6ac058cd-6138-83e8-af54-758358f73006).

Verbatim result:

```text
P1-1: rust/native-host/src/api.rs:2450,2549,2600,2697,2794,2828 hard-codes every new S5 mutation to compile profile.base_dir.join("config.yaml") instead of the actual root path supplied to HostAssembly::from_config_file(path). A managed host started from another filename therefore either fails all mutations when <base_dir>/config.yaml is absent, or—more seriously—can compile/publish against a different decoy config.yaml while restart still uses the original root file. This breaks the S3/S5 contract that a mutation compiles the complete candidate for the running configuration. Retain the actual startup config path in managed runtime state and use it for all six mutation paths; add an HTTP regression using a non-config.yaml root, preferably with a distinct decoy config.yaml. Existing S5 tests uniformly start from config.yaml, so the 375/0/3 validation does not cover this case. [open]
FINAL: FAIL
```

The finding is actionable and remains open. S5 is not accepted; implement the
configuration-root ownership fix, add a real HTTP regression that distinguishes
the active root from a decoy `config.yaml`, rerun isolated validation, and
re-review the exact repaired source with the same finding ID. S6–S7 remain gated.

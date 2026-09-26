# M2 unit 2 calibration identity

Protocol/tooling reviewed at `9932a76aa0c5b3a55bdda71780b97f5d77e84866`.
Reviewer PASS evidence is recorded in `m2-review-ledger.md`; authorization
is the 2026-09-26 user request and the frozen range in `measurement-revision-v2.md`.
No benchmark slot has run when this identity is recorded.

| Artifact | SHA-256 |
|---|---|
| Linux helper v9 | `254500ec00527850f0f137bcc7e9ac26ff4953d6600bfc00eb7a7fbc2afc436d` |
| Helper source main.go | `397bdacf708a47894362d42b0cc12575a6c41ca34714db8d3d0be393ccbd654a` |
| M2 runner | `ba864a787b639aa293a8e4a9981be03ca80d7a874f6cc182ed1a5889d77ad021` |
| M2 driver | `7cf2084fd6ad9ec5aea94dd0a4b1595e506d3e7d7a1a9418ca620ef803cf6db4` |
| Qualification analyzer | `e416ccea63f6e324da002cbfe8775d0db743a911330ee8119206ae0a6d85d7c7` |
| Reviewed protocol | `67ba8bea79bfee6a2835f03108bea7a5268862975ede91adbfb8294ca96ef361` |

These hashes were reverified on the Linux host before the first attempt.
The driver verifies exact baseline, runner, helper, helper source, original
V12 order plan, unchanged original paired analyzer, all four YAML files and
all three workloads. Both 27-slot batch plans are generated and hashed before
the first attempt. Every slot uses the archived Rust-before binary and audit
off; there is no candidate or enabled-audit claim. M2 profile and GOMAXPROCS=1
are enforced and recorded per attempt.

Official control result root: `/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-m2-calibration`.
Separate zero-attempt preflight root: `/root/mosdns-rust-phase5a-native-query-observability-545ba29/results-m2-preflight`.

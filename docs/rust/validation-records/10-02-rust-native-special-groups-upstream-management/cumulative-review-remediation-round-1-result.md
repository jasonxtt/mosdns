# Cumulative P1-1 re-review result — round 1

Dedicated reviewer, exact head `ccc147bb35aab2301633325c563368ac8457dbba`,
tree `3aab66be7ab39fcf6878d85c417430bda3ca41f7`, complete base `79d93ae1...`.

> P1-1: rust/cache-core/src/lib.rs:307-308,381 still makes response origin strictly in-memory: NativeAttachment is deliberately excluded from snapshots, and every PreparedNativeEntry reconstructed from a snapshot/dump gets attachment: None. This is broader than the permitted “old wire-only import has unknown origin” case recorded in cumulative-p1-1-remediation.md:26-27: dumps written by the remediated code are also wire-only, so a normal save/shutdown/restart converts a currently provenance-bearing group-cache entry into an originless hit. That hit again cannot satisfy the frozen miss/hit supplier contract at docs/rust/contracts/native-special-groups.md:59,95. The new live proof and tests correctly close the in-memory miss→hit/audit-off/owner-reuse cases and keep attempts empty, but no save→restart→hit proof exists because the current persistence path necessarily drops the origin. Preserve origin for newly persisted cache entries in a backward-compatible way (while continuing to treat genuinely legacy wire-only imports as unknown), and add a restart hit regression asserting entry/peer/transport survive with no fabricated attempt or ECS echo. [open] FINAL: FAIL

Finding is valid and remains open under the same P1-1 identity. The prior successful
in-memory tests remain evidence, but this object is not whole-task PASS.

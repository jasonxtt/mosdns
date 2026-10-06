# S7 exact-source review request — round 1

**State:** sent once on 2026-10-04 to the bound C2C reviewer; waiting for an
explicit final result. No supplemental message has been sent.

Exact audit object: accepted S6 parent
`eec6c4447e221ea304893dd8ddaf5b761a35cdb3` → S7 candidate
`59b1612ad3629150cee68a68c8e0d19a7ed5968d`, tree
`8bb0d3fdecfb47ecbb27ded84ec5132fd6f3ffb6`. Reviewer binding:
[Review Rust Native Groups](https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6ac058cd-6138-83e8-af54-758358f73006).

The exact reviewed range is scoped to the S7 whole-chain test, S7 isolated
build/test/DNS/HTTP/browser evidence, generated UI entrypoint timestamps, and
the public contract/plan/validation handoff. The tested-source manifest has 527
inputs; local, isolated-host and candidate-tree SHA-256 values match. Its source
map SHA-256 is
`5d598287cf6cc6ad63b1627c322f8deafe5f364bfdc20792ecfc10ae6b667c7a`; manifest
file SHA-256 is
`cd32bcce468bd898c2d98e2023b74e51f5280af3068b6b5f244572ad968256c2`.
The evidence manifest covers 38 retained S7 artifacts. Its evidence-map
SHA-256 is
`d1e65b0e06e2fbcef282aca9c805f7693e82e90eee261df9f43de1ed9572437f`; manifest
file SHA-256 is
`113a4d870fa4b111b06f894a2ca6cbe60986ee17e29fe0e6c61ef144c89e2274`.
The transmitted control message was 936 bytes with SHA-256
`48b05a0731d138b9063b76ce0bc8b4bb81d43b706577892576e6c6c5ea13451a`.

## Atomic C2C request

```text
[C2C]
MODE: REVIEW_ONLY
STATE: REVIEW
CONTROLLER: TRELLIS
TASK_ID: 10-02-rust-native-special-groups-upstream-management
UNIT: Slice 7
BASE_SHA: eec6c4447e221ea304893dd8ddaf5b761a35cdb3
HEAD_SHA: 59b1612ad3629150cee68a68c8e0d19a7ed5968d
TREE_SHA: 8bb0d3fdecfb47ecbb27ded84ec5132fd6f3ffb6
PATHS: exact range; whole-chain test, S7 evidence, public contract/plan/validation handoff.
VALIDATION: workspace 1179 passed/0 failed/3 parent-invoked probes; fmt/strict Clippy, native build, DoT 27/DoH 39, Vue 619/612, controlled DNS/HTTP/browser passed. Initial disk-full failure retained.
SOURCE: 527/527 local/host/tree hashes match; source-map SHA256 5d598287cf6cc6ad63b1627c322f8deafe5f364bfdc20792ecfc10ae6b667c7a; see s7-status.md.
REVIEW: use git_compare on this exact range. Check frozen S7 behavior and evidence; make no edits, executions, or Trellis writes. Return stable findings and exactly one final line: FINAL: PASS or FINAL: FAIL.
```

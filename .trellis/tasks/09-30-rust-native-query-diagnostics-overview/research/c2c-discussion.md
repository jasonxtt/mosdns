# C2C planning discussion — 2026-09-30

New chat inside the existing mosdns-rust Project:
https://chatgpt.com/g/g-p-6ab6002418488191a95a80a83575f0ac-mosdns-rust/c/6abc98bf-387c-83ee-969d-eeae4641e5e7

The chat's workspace_info confirmed mosdns-rust / rust / 9f6dfdb2 and read
AGENTS.md before discussion. Only the exact mosdns-rust connector was used.
No file bodies/diffs/logs were pasted. First response hit a visible stream
error; the same message's Retry completed. No second chat/connector was created.

## Initial response

ChatGPT returned substantive STATE: PLAN and supported one bundled PRD because
all six deliverables share a terminal/read model. This is advisory planning,
not committed-range review or implementation FINAL: PASS.

| Advice | Source verification and disposition |
| --- | --- |
| Freeze final-wire decode errors/all-or-none | Accepted: contracts now fix answer_details_status and optional closed answer_decode_error; decoder at final listener capture seam; no raw wire retained |
| Domain rank uses cross-field q and can mix unrelated rows | Verified OverviewManager:682-696, Go q OR predicate. Accepted exact native logs/domain route with shared rank predicate and 404-only legacy fallback; old q/domain semantics unchanged |
| Atomic generation evidence, candidate before complete rule | Verified QnameMatcher bool-only and engine mutation before later matcher. Accepted immutable provider-generation evidence; separate candidate/committed data, no later generation lookup |
| Overlap exact-rule provenance | Kept boolean/group order; reduced this batch to provider-generation/inline-YAML precision, exact provider literal deferred. No invented first literal or altered MixMatcher ordering |
| Final selected supplier not last successful attempt | Accepted explicit causal final-wire supplier, configured flow_setter metadata separate from factual endpoint; parent/cache/local replacement tests |
| Replace open read options with immutable handles + bounded workers | Accepted two jobs/no unbounded queue/503; permit lives through actual worker exit, off-thread scan and encoding. Removed speculative chunk/version retry framework |
| Variable answer memory is under-specified | Accepted three-part structural proof, large-record memory projection and sized near-full progress screen; no maximal400k allocation or hidden truncation |
| Freeze trace format/exhaustion | Accepted lowercase nonce/counter format length51; allocation at admission, no wrap; failure/nonce-injection tests |
| Header facts versus extended RCODE/full decode | Accepted safe header separation, closed decode reason, extended OPT code tests |
| Unknown provenance ranks | Accepted omit unknown categorical keys, count true unmatched sentinel, retain rows/overall totals; explicit approval-list accuracy deviation |
| Preserve Go top300 slowest history | Verified Go independent heap. Accepted shared records/history across ordinary eviction; clear/resize reset; deterministic tie proposal |
| Query URL decoding/duplicates | Accepted bounded form parsing/Unicode/+ and strict malformed400, scalar-first and repeated clients; strict error is explicit proposed deviation |
| Vue refresh errors coupled by Promise.all | Accepted per-panel errors and honest unsupported sections; preserve successful Go behavior |
| Avoid enormous Cartesian E2E | Accepted focused parser/provenance/API invariants + single realistic integrated chain and browser proof |
| Avoid tracing bus/search index/speculative retry design | Accepted small typed seam and existing owned host/API; no generic framework |

The exact final proposed compatibility contracts must be approved by the user
in the subsequent execution conversation. C2C cannot approve those choices for
the user. Final follow-up discussion outcome is appended below after reread.

## Follow-up assessment and final repairs

The same chat re-read the revised set and returned STATE: PLAN. It said the
major public choices now align and, after three finite corrections, no further
substantive planning ambiguity needs another design round. It explicitly did
not claim an implementation or committed-range PASS.

- Replaced nonexistent native-host server.rs with actual udp.rs/tcp.rs
  (verified with rg --files); these own the final capture seam.
- Listed public two-job/503 behavior explicitly in contracts approval choices.
- Verified upstream-core Cargo.toml:21 and resolver/bootstrap.rs:68/79 use
  getrandom::fill. Froze the same direct native-host getrandom dependency;
  included native-host/Cargo.toml and explicit nonce/counter/ID tests.
- Added explicit all-or-none diagnostic test mapping. Exact provider-line
  provenance remains consistently deferred, as the chat confirmed.

These final repairs affect only task planning artifacts. No third discussion
round is required for these source-confirmed clerical/test-traceability fixes.
Task remains planning; user requested stop and handoff. No product tests were
run and no final implementation acceptance is claimed.

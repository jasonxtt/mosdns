# Cumulative P1-1 remediation

The first cumulative review of `79d93ae1b3b3253a2d09563444b251aad18eb5df`
through `1495c508fe5ead3dc2ba9fa6c7c435a0a220cf1e` returned explicit FAIL.
P1-1 is valid: the original real DNS regression asserted the supplier only on
misses, and the retained S7 HTTP proof shows missing supplier provenance on hits.
This record is preparation for a same-ID exact-source cumulative re-review;
it does not claim reviewer PASS.

## Implementation and ownership

- A response carries its actual supplying entry, selected peer and transport.
  Native entry identities are allocated when compiling the snapshot, independently
  of audit enablement and executable numbering in later snapshots.
- Cache publication atomically attaches this immutable origin to the same bounded
  cache entry as the normalized answer. Replacement, eviction and flush own both;
  there is no separate unbounded metadata map. Existing admission/generation,
  dependency closure and stale-writer fencing remain the publication gates.
- Cache hits restore that response origin. Audit output reports the supplying
  entry/peer/transport without copying historical attempts or starting a new
  upstream request. Cache remains the response source, so supplier provenance does
  not grant ECS echo permission.
- CNAME normalization, TTL and scoped wire decoration preserve origin. Explicit
  response replacement, including identical-byte replacement, clears it. Root,
  branch, detached lazy refresh and direct handler cache paths share this behavior.
- Existing persistent wire-only dumps and transitional ABI formats are unchanged.
  An imported entry has no recorded historical origin; the host does not invent
  one. The attachment is native memory metadata, bounded by the cache entry count.
  This limitation is explicit and covered by an import regression.

## RED and retained failed attempts

The enhanced original test fails against the old code (`None` versus the real
supplier). Both ordinary and audit-disabled warmup fail after verifying immutable
baseline inputs and forcing their recompilation. An earlier baseline attempt
reused newer cached build artifacts due to archive timestamps; it is retained and
explicitly rejected as RED/GREEN evidence. No passing baseline result is claimed.

The implementation passes both miss→hit supplier regressions and the real HTTP
whole-chain owner-reuse/invalidation tests. Intermediate failures are retained:
an unsupported hash-map key, an old miss selector that also matched the newly
correct hit, and strict Clippy findings for a missing `must_use` annotation and
an oversized test helper. These were corrected without warning suppression.

Final isolated verification is complete: fmt and strict workspace all-targets
Clippy pass; native-host 379/0/3, workspace libraries 409/0/3 and complete workspace
1,182/0/3 pass (79 targets, including doc tests). The three ignored probe
entrypoints are explicitly invoked by parent tests. Native binary build passes.
[Command results](evidence/cumulative-p1-1-final-checks-retry2.json) and
[counts](evidence/cumulative-p1-1-counts.json) identify the complete logs.

Real controlled UDP/TCP DNS and HTTP proof sends 10 queries but observes only
6 peer requests. Lower supplier, updated lower supplier and unchanged higher
supplier all retain the same selected entry/peer/transport on hits, with empty
current attempts. Upstream save succeeds; unsupported activation is rejected.
[Assertions](evidence/cumulative-p1-1-live-retry1/assertions.json) and
[HTTP audit](evidence/cumulative-p1-1-live-retry1/live-proof.json) preserve the
actual output. The first fresh fixture lacked its cache directory and used a
UDP peer for a TCP upstream; its failed run remains visible beside the corrected
fixture. All task-owned proof processes were stopped by the runner. Prior S7 browser/Vue/Go evidence remains
applicable to identical frontend and Go input bytes; this remediation changes nine
Rust inputs and does not claim a new browser run.

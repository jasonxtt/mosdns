# Native UDP response size and truncation

## 1. Scope / trigger

The final UDP listener presentation after sequence execution, full cache
publication and response OPT/ECS handling. TCP, DoH and upstream transport
fallback retain their own existing behavior.

## 2. Signatures

`udp_response::payload_limit(original_query)` obtains the client OPT UDP size,
with 512 as the minimum/default. `prepare_response(owned_full_wire, limit,
original_query)` returns final UDP wire plus whether any records were removed.
It applies `limit_response`, with a minimal TC reply on decoding/encoding errors.

## 3. Contracts

- Use the original client's OPT, not an execution-time ECS/forwarding query.
  No OPT means 512; advertisements below 512 also mean 512.
- A response already within the budget keeps its original wire allocation
  and encoding. It may already contain legal name compression; no requirement
  forces uncompression just to reproduce the Go packer's encoding choice.
- An oversized response uses name compression and whole-record prefixes in
  Question, Answer, Authority, Additional order. Reserve the complete response
  OPT before ordinary records and restore it at the end. At the first record
  that cannot fit, omit it and all remaining records/sections. Any omission
  sets TC; never clear an existing TC. Compression alone does not set TC.
- A response with a final TSIG is preserved without repacking/truncating,
  matching the Go truncation exception and retaining its signed bytes.
- If an oversized response cannot be decoded/re-encoded, or question plus OPT
  cannot fit, send a minimal truncated reply: preserve the response header and
  set TC, set QDCOUNT to 1 and AN/NS/AR to 0, then append the admitted original
  question. Expand question-name pointers so they cannot refer to omitted data.
  This bounded reply lets the client retry TCP against the full cache.
- Cache entries and response policy execution remain full and independent of
  downstream size. A later TCP or larger-EDNS client can retrieve the complete
  response from the same cache; a small-client truncated response is never
  published as the cache value.
- Eligible detailed audit records reflect the final wire's TC and retained
  answers. Route, upstream, cache status and transport outcome remain factual.
  Admission-disabled detailed auditing does not add a response-details parse.

Go mapping: `pkg/server_handler/entry_handler.go` and miekg/dns v1.1.72
`msg_truncate.go`. These are behavioral references in the Go repository/module
cache, not dependencies of the Rust runtime.

## 4. Validation / error matrix

| Input | Required result |
| --- | --- |
| No OPT, oversized answer | At most 512 bytes, whole-record prefix, TC if omitted |
| EDNS 1232 / 4096 | Independent per-client limits, minimum 512 |
| Encoded response exactly at limit | Original bytes/allocation retained |
| Uncompressed response too big, compressed full response fits | Keep every record, do not introduce TC |
| Only Additional overflows | Preserve earlier sections, set TC |
| First Answer RR exceeds budget | Keep question/OPT, drop all ordinary sections, set TC |
| Response OPT consumes remaining space | Reserve OPT, omit ordinary RR if needed |
| Existing upstream TC | Preserve TC through both fast and slow paths |
| Cache followed by large EDNS/TCP | Full original cache response available |
| Typed decoder rejects oversized response / mandatory data cannot fit | Minimal question-only TC reply; final audit TC=true |

## 5. Good / base / bad cases

Good: one cached 96-address answer gives small UDP clients a TC prefix and
large-EDNS/TCP clients all 96 addresses, with matching final audit records.
Base: a fitting signed or ordinary response retains its existing bytes.
Bad: slicing an encoded byte buffer, evicting complete answers from the cache,
or auditing pre-truncation answers as though they were sent.

## 6. Required tests

`udp_response` unit tests cover boundaries, compression rescue, section stop,
OPT reservation, prior TC, minimum/EDNS budgets and TSIG. The real socket test
`udp_budget_is_per_client_while_cache_and_tcp_keep_the_full_response` covers
cache publication, cross-client limits, unchanged TCP and final audit fields.
`udp_decode_failure_sends_tc_and_audits_it_while_tcp_keeps_full_cache` checks
the same boundary with a synthetic RR rejected by the typed decoder.
The optional offline Go oracle compares section counts/TC and reports byte
encoding differences separately. Captured private samples stay outside the
repository; public evidence contains only lengths/counts/conclusions.

## 7. Wrong vs correct

Wrong: impose the UDP budget in the common executor before cache publication,
or rely on an upstream's UDP size as the downstream client's limit.
Correct: keep full execution/cache state and apply this client's budget once
at the UDP listener boundary, then project final audit facts.

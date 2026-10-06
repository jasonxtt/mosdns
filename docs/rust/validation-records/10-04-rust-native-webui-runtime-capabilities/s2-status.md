# S2 embedded static UI

Build-time deterministic recursive manifest borrows binary-owned asset bytes. Existing API origin serves roots and nested assets, without a new listener or runtime source dependency. GET/HEAD, MIME/nosniff, no-cache strong SHA256 ETag and weak/comma/wildcard conditional matching; bodyless304 reports representation length. Missing/reserved API paths never fall through to HTML. Safe decoded path components, method405/Allow and /log/ redirect. Four nonblocking static slots, <=64KiB chunks, 5s header deadline and 10s static response deadline with shutdown cancellation. Ordinary management writers retained.

VM evidence: initial root404 RED retained; HTTP12 PASS, bounded-slot/cancel and decoding2 PASS, strict Clippy/fmt PASS. Two initial Clippy findings (generated path formatting and test extension comparison) fixed with logs retained. Real static requests leave DNS metrics/audit unchanged. Independent tiny build-script fixture proves nested/deterministic embedding and missing root/reference/symlink/unexpected-key rejection. Exact source and current embedded asset hashes match VM in s2-tested-source.json. Existing accepted Vue bundles used; fresh ordered builds are S6.

Review pending on exact S1→S2 object. No external mount implementation or S3 work yet.

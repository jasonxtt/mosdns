# S3 — product keys, clocks and metadata (2026-10-02)

Implementation complete, independent C2C review pending. S4–S7 remain unimplemented.

- Native keys use flags/QTYPE/text length/text name, with miekg label escaping and preserved case/root dot. AD/CD/DO are independent. IN only, ECS and malformed/unsupported EDNS bypass; text names over 255 bytes bypass. Existing compressed-query key behavior stays supported.
- Core owns original wall timestamps together with monotonic runtime timestamps, without a shadow store or ABI extension. Original wall times survive lookup/wall rollback; lazy NOERROR retention starts at storage time. Positive lazy config stays refused until S4.
- OPT removal uses hickory-proto decode/re-encode, preserving known name-bearing records when pointer offsets change. A CNAME after the removed OPT referencing its root owner is tested. Miss response is unchanged.
- CIDR scalar/list configuration is accepted. Invalid CIDRs warn and skip; any answer A/AAAA in a valid excluded prefix prevents publication.
- Ordinary request, branch and direct policy-cache paths capture successor domain_set and restore nonempty metadata on hit. A real UDP miss/hit final audit test verifies restoration.

Validation on SSH mosdns-rust, task-exclusive directory only:
- Red product-key test failed on original private format; its expected escaped-name length was corrected from 9 to 8 before green.
- Focused lifecycle/catalog/config/W2 checks passed (initial clippy failed on missing must_use, repaired).
- Full cache-core/native-host run first failed 3 old slice1-cache expectations: EDNS bypass/OPT reject were obsolete; compressed key regression was real and fixed. Retained logs /tmp/cache-s3-check-all.log and /tmp/cache-s3-check-all-r2.log locally.
- Re-run cache-core/native-host: **225 passed, 0 failed**. Clippy then found only let_and_return in config, repaired without behavior change.
- Final fmt/check and clippy all targets for cache-core/native-host passed; lifecycle 6 + catalog 11 + config 13 = **30 passed**.
- Evidence logs published through c2c record iteration 7. No local compilation, commit, push or deployment.

Hickory-proto 0.25.2 is used as a DNS codec. Dependency fetch failed on certificate/TLS errors; existing cached package archives were transferred to the task host, and it resolved only the 12 additional dependencies offline from the original lockfile. The repository lockfile is the host's resulting lockfile.

# Rust matcher Phase 2 expansion evidence

Date: `2026-08-14`

This is a repeatable Linux+cgo measurement for the provider and
`domain_mapper` bridge added by `08-13-rust-matcher-phase2-expansion`. It is
evidence for the opt-in path only; default builds and runtime selection remain
Go-only.

## Environment and fixture

- Host: `mos-test` / `10.0.0.91`, Linux x86_64, four online CPUs
- Go: `go1.24.4 linux/amd64`
- Rust: `rustc 1.95.0`
- Source: isolated `/tmp/mosdns-rust-slice5.58Ydys`; no installed service or
  production configuration was used

The benchmark uses repository-generated, deterministic rules:

- Domain: 512 rules (128 each of `full`, `domain`, `regexp`, and `keyword`),
  15,231 newline-joined input bytes, and 512 Rust index entries.
- IP: 512 prefixes (256 IPv4 `/24` and 256 IPv6 `/48`), 8,065 joined input
  bytes, and 512 Rust index entries.
- Valued mapper: the same 512-rule shape with marks, context marks, tags, and
  source metadata. `result_bytes` is the logical decoded result size for the
  fixed query vector; it is not Rust heap size.

The command was:

```text
scripts/build-rust-cache.sh
CGO_ENABLED=1 MOSDNS_MATCHER_BACKEND=rust BENCHTIME=250ms COUNT=3 \
  scripts/benchmark-rust-matchers.sh
```

`-benchmem` reports Go-observed allocations. Rust build B/op therefore does
not measure Rust allocator bytes. Lookup cgo counts include the transitional
Go string/FFI boundary.

## Three-run results

Values below are the three raw runs printed by Go; medians are included for
quick comparison. Throughput is only the reciprocal of median `ns/op`, not a
production load result.

| Fixture / operation | Go ns/op (runs; median) | Rust ns/op (runs; median) | Rust cgo calls/op | Rust result/index metric |
| --- | ---: | ---: | ---: | ---: |
| Domain build | 711621 / 640595 / 618373; **640595** | 1173562 / 1162488 / 1315899; **1173562** | — | 512 entries |
| Domain lookup | 3463 / 3218 / 3111; **3218** | 2613 / 2811 / 2711; **2711** | 1.000 | 512 entries |
| IP build | 35939 / 36373 / 35466; **35939** | 66981 / 61844 / 79462; **66981** | — | 512 entries |
| IP lookup | 50.83 / 48.73 / 44.53; **48.73** | 276.9 / 271.7 / 218.4; **271.7** | 1.000 | 512 entries |
| Valued mapper build | 1374812 / 1471632 / 1637119; **1471632** | 2717866 / 2743309 / 2736454; **2736454** | — | 21 fixture result bytes |
| Valued mapper lookup | 5155 / 4903 / 5930; **5155** | 15438 / 14869 / 14511; **14869** | 2.000 | 26 logical result bytes |

Allocation samples from the same runs were:

| Fixture / operation | Go B/op | Rust bridge B/op | Go allocs/op | Rust bridge allocs/op |
| --- | ---: | ---: | ---: | ---: |
| Domain build | 1,035,680 | 32,808 | 11,328 | 4 |
| Domain lookup | 0 | 29 | 0 | 2 |
| IP build | 55,936 | 16,424 | 12 | 4 |
| IP lookup | 0 | 32 | 0 | 3 |
| Valued mapper build | 1,455,576 | 153,024 | 18,761 | 21 |
| Valued mapper lookup | 112 | 208 | 5 | 8 |

The bridge is slower to build, and IP plus valued-mapper lookup pays a visible
cgo/decoding cost. These measurements keep Rust experimental; they do not
authorize a default-backend switch or a native query-path redesign in this
task.

## Limitations

- `index_entries` is the accepted rule/prefix count, not allocated index bytes.
- `result_bytes` is a logical decoded payload indicator; per-snapshot Rust RSS
  and allocator statistics are not exposed by this ABI.
- The fixture is single-process and small. It does not claim p50/p95/p99,
  production QPS, CPU utilization, or long-run memory behavior.
- The fixed-fixture benchmark is independent of UI packaging. A separate
  Linux host gate built the experimental binary with the normal embedded Vue
  assets and ran the isolated smoke; see the test-host record.

# M1: run the production `config_all` natively

Status: planning draft, revised 2026-10-06 after a user decision. No implementation authorization.

This file replaces the earlier "native core query primitives" draft. The local
task id `10-06-rust-native-core-query-primitives` is kept for continuity, but
its scope is now the M1 milestone below.

## User decisions (2026-10-06)

- The first production replacement is gated on the user's own config package
  `config_all` (`../file/mosdns/config/config_all`), not on all 71 registered
  plugins. It still has to pass 5D, Phase 6 and explicit user confirmation.
- Registered plugins and parameters that `config_all` does not use are deferred
  until after the first replacement. They are not removed from the ledger.
- Review strength stays as it is.
- Production `mosdns` is 1 vCPU / 957 MB. The concurrency-model spike is
  cancelled and the single-threaded `current_thread` + `LocalSet` model stays.
  Memory is now a first-class goal next to latency; 5D gets an RSS budget.

## Goal

The native host loads `config_all` without modification and serves it, with
native evidence for every plugin, parameter, API and persisted state the
package uses.

## Order

1. **`check` command and ledger convergence** — done (`7c7f02a7`, task
   `10-06-rust-native-config-check-ledger`, archived).
2. **M1-0 config-root paths and unmanaged loading** — done (`26accd14`).
3. **Merge `main`** (v0.7.1 → v0.7.3) — done (`a879755a`); drift recorded in
   `docs/rust/validation-records/10-06-merge-main-v0.7.3/drift.md`.
4. **M1 batches.** The current gap list is `docs/rust/m1-gap-list.md` (driven
   by `check` output); the execution order and status live in
   `docs/rust/next-stage-plan.md`. Summary:

| Batch | Scope |
| --- | --- |
| M1-a1 providers, query side | `domain_set_light` (11), `sd_set_light` (4), `si_set` (1), `adguard_rule` (moved from M1-b: `unified_matcher1` references it), SRS decoding, `domain_mapper` (2), `mark` matcher |
| M1-a2 provider management | shared online rule-source management (CRUD, download, auto update, socks5) for `sd_set_light`/`si_set`/`adguard_rule`; `domain_set_light` editing; hot mapper rebuild |
| M1-b upstream and response | standalone `aliapi` (5; needs SOCKS5 upstreams), `rewrite`, `webinfo`, `rcode`, `tag_setter`, `drop_resp`, `sleep`, `fallback` bare-tag form |
| M1-c output and requery | `domain_output` (8), `requery` |
| M1-e special_groups v0.7.3 | managed generator schema-4 shape, group upstream references and owned upstreams |

`cname` and `mark` as standalone primitives were dropped from M1-b: `config_all`
does not use `cname`, and `mark` is covered by M1-a1. `cname_remover` is already
accepted natively. The package's upstreams use only https, tls and udp, so H3/DoQ
is not on the M1 critical path.

## Deferred until after the first replacement

Plugins `config_all` does not use, including `qclass`, `ptr_ip`, `random`,
`has_wanted_ans`, `black_hole`, `env`, `string_exp`, `arbitrary`,
`rate_limiter` and `reverse_lookup`. The `check` output decides the final list.

## Completion meaning

M1 is complete when `check` reports no gaps for `config_all` and real-config
E2E passes on the VM. It does not close 5D, Phase 6, production readiness or
the remaining unused plugins.

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

1. **`check` command and ledger convergence** (separate task
   `10-06-rust-native-config-check-ledger`). Add `mosdns check -c <config>`,
   which reports every unsupported type or parameter instead of stopping at
   the first error. Its output for `config_all` becomes the M1 work list, and
   `feature-coverage.md` is updated from it.
2. **Merge `main`** (v0.7.1 → v0.7.3) before implementing providers and
   `aliapi`.
3. **M1 batches.** Counts below come from a YAML text scan on 2026-10-06; the
   `check` output takes precedence.

| Batch | Scope |
| --- | --- |
| M1-a providers | `domain_set_light` (11 instances), `sd_set_light` (4), `domain_mapper` (2), `si_set` (1), SRS/geodata formats |
| M1-b upstream and response | standalone `aliapi` (5 instances; confirm signed-API fields separately), `rewrite`, `adguard_rule`, `webinfo`; the primitives `config_all` actually uses: `rcode`, `cname`, `mark`, `tag_setter`, `drop_resp`, `cname_remover`, `sleep` |
| M1-c output and requery | `domain_output` (8 instances), `requery` |

The package's upstreams use only https, tls and udp, so H3/DoQ is not on the
M1 critical path.

## Deferred until after the first replacement

Plugins `config_all` does not use, including `qclass`, `ptr_ip`, `random`,
`has_wanted_ans`, `black_hole`, `env`, `string_exp`, `arbitrary`,
`rate_limiter` and `reverse_lookup`. The `check` output decides the final list.

## Completion meaning

M1 is complete when `check` reports no gaps for `config_all` and real-config
E2E passes on the VM. It does not close 5D, Phase 6, production readiness or
the remaining unused plugins.

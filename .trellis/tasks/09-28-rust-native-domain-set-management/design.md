# Rust-native domain_set management — design

## Boundary and ownership

This is the first 5C control-plane slice on the Rust-native host. The compiled
native `domain_set` currently owns an immutable query matcher loaded before
listeners start. The selected design must add a provider owner for a live rule
generation, a file persistence boundary, and a plugin-scoped HTTP handler. The
handler must publish a replacement to the same matcher used by DNS execution.

Candidate implementation areas are `rust/native-host/Cargo.toml`,
`src/config.rs`, `src/assembly.rs`, existing native API/router code if present,
and a focused management module plus HTTP/DNS integration tests. Confirm actual
crate boundaries and Trellis specs before implementation; do not create a
parallel query matcher or a generic API framework for this one plugin.

## Source contract to preserve

Current Go evidence:

- `plugin/data_provider/domain_set/domain_set.go` mounts `/show`, `/save`, and
  `/post` on the plugin router; `coremain/mosdns.go` mounts that router under
  `/plugins/{tag}`.
- `/show` returns UTF-8 text/plain and writes each live rule followed by a
  newline.
- `/save` is a GET that writes the current rules; it returns 500 with a
  visible error if no file is configured or persistence fails, and empty 200
  on success.
- `/post` decodes `{ "values": [...] }`; malformed JSON returns 400
  `invalid JSON`. It requires the selected rule file to have a `.txt` suffix,
  builds a candidate generation, persists it, and only then replaces the live
  matcher/rule snapshot. Persistence failure returns 500 and leaves the old
  published generation live. Success returns 200 with a replacement count.
- Per-rule `MixMatcher.Add` errors are skipped by the current POST handler.
  Characterize this exact behavior against current rule grammar and preserve it
  for the chosen profile; do not accidentally return a partially compiled
  candidate with different contents.
- `webui-log/src/components/ListManager.vue` reads `?limit=10000` from
  `/show` and posts `{ values }` to `/post`.

The Rust implementation should use safe file replacement and explicit
generation ownership; it does not need to reproduce Go's direct truncate/write
implementation. Preserve user-visible API behavior and failed-update safety.

## Management-eligible configuration profile

Before implementation, compare current Go's `ruleFile` selection (first
configured file), update behavior, and rule composition with the native config
model. Freeze the initial Rust-native supported profile in this document and
tests. Prefer a single explicit UTF-8 `.txt` file as the writable source. If a
configuration contains multiple files, SRS, expressions, or `sets` such that
posting would discard or fail to persist part of the query matcher, either:

1. preserve and test the Go-observable contract exactly for that shape; or
2. reject management for that shape visibly while leaving the existing
   load-time query behavior intact.

Do not silently expose `/post` that mutates only part of a composite rule set.
Do not claim complete P02 management from the initial profile.

## Data flow and publication

```text
POST body → parse UI payload → apply characterized per-rule acceptance
          → compile complete candidate generation
          → persist selected file safely
          → atomically swap provider generation
          → subsequent DNS query uses new matcher
```

`GET /show` snapshots one published generation and formats its rules. `GET
/save` persists one generation snapshot. On `POST`, a persistence error leaves
both old disk content and old in-memory generation intact. Prefer an atomic
same-directory temporary write + flush/sync + rename strategy appropriate for
the existing filesystem contract; test the failure boundary without leaving
temporary files. Do not hold a DNS hot-path lock while doing file I/O.

Concurrent readers should use an immutable generation handle (for example an
`Arc` snapshot) or a minimal equivalent already established in the host. One
publish operation changes the complete rule list and matcher together. Old
matcher generations remain alive until existing queries release them; disposal
must not invalidate an in-flight read.

## HTTP and lifecycle boundary

Use one real HTTP router mounted at `/plugins/{tag}` for integration coverage.
Keep method/path/status/body/content-type behavior explicit. The HTTP server
and DNS listener lifecycle must have one owner in `HostAssembly`/runtime close
so a single shutdown waits for both listener tasks and permits port rebind.
Avoid making the HTTP handler itself own an independent generation copy.

## Observable behavior slices and mock boundary

1. **HTTP semantics:** real HTTP client → native mux → plugin handler. Assert
   path, method, status, content type, response body, and actual file effects.
   Do not mock routing or handler dispatch.
2. **Atomic successful update:** post A→B using a temp file, query through a
   real loopback DNS listener, and prove B matches while A no longer matches;
   restart a fresh assembly and prove B still matches.
3. **Rejected update:** malformed JSON and injected storage error return
   compatible errors; verify byte-for-byte old file content and next-query A
   match. Use the real parser, compiler, publisher, router, and DNS path.
4. **Concurrent publication:** coordinate real query readers around one update;
   every observed result maps to complete generation A or B. A deterministic
   barrier or scheduler hook is acceptable, but do not replace the matcher or
   generation publisher with mocks.
5. **Lifecycle:** start API and DNS on loopback ephemeral/free ports, close the
   host, and prove both addresses can be rebound. Do not make a graceful signal
   claim outside the tested owned-host close contract.

The only injected failure should be the narrow persistence operation needed to
force a write error. Use a temp directory for normal tests. A real-process
integration should exercise the actual file-backed provider and listener
assembly.

## Compatibility and limits

This task proves the selected plugin API and runtime rule-edit loop only. It
does not finish WebUI workflows, all plugin endpoints, all config package
shapes, all formats/providers, audit/metrics, cache persistence, or 5C. Update
feature coverage only for evidence produced by this task.

# S6 browser/DNS proof — 2026-10-06

This is a disposable local proof against the ordered native artifact recorded
in [native-release-manifest-final.json](native-release-manifest-final.json).
It does not select a production/default backend or perform deployment.

## Browser-served shells

The binary served both same-origin entrypoints through the in-app browser:

- `/` returned 200 and rendered the Vue dashboard. System settings showed the
  configured `switch17 · routing/custom` inventory row and the native generic
  value control.
- `/log` returned 200 and rendered the compatibility shell. Its “解析行为”
  panel showed the same configured `switch17 · routing/custom` instance.
- The main shell saved ` A ` through the real control and showed the canonical
  readback. The compatibility shell was refreshed against the same binary and
  showed the same custom tag/value path; no product-specific FakeIP/RealIP
  label was invented.

## Exact value and admission behavior

Against the same running artifact and config generation `0`:

- JSON POST of ` A ` returned 200; a subsequent GET returned the exact bytes
  `20 41 20` (leading and trailing spaces preserved).
- JSON POST of ` A \n` returned 200; a subsequent GET returned the exact bytes
  `20 41 20 0a` (leading/trailing spaces and newline preserved).
- A stale `X-Mosdns-Config-Generation: 999` POST returned 409 and did not
  change the value.
- A disposable restart with a missing state parent exposed the configured
  instance as readable but not writable; both the capability inventory and
  the main browser shell showed the restart/rebind reason, the input and Save
  control were disabled, and POST returned 403.

## DNS and lifecycle evidence

- A controlled local UDP peer on `127.0.0.1:19999` answered a real query sent
  through the binary DNS listener on `127.0.0.1:18781`; `dig` received
  `example.com. 60 IN A 1.2.3.4`.
- `switch_declarations`: 18 passed, including the all-seventeen declaration,
  reserved-bit, immutable admission-facts, and snapshot/branch checks.
- `slice6_management_http`: 14 passed, including real HTTP/DNS, capability
  inventory, encoded custom tags, persistence, shutdown/rebind, and error
  admission behavior.

All runtime files and listeners used for this proof were disposable local
artifacts; no production host, port 53, push, or cutover was involved.

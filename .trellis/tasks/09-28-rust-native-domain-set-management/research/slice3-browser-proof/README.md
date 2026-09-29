# Slice 3 browser proof (maintained Vue local-rule page -> native API -> file -> DNS)

Isolated, reproducible end-to-end proof for the last behavior slice of
`09-28-rust-native-domain-set-management`. Everything runs on loopback with
probed free ports; nothing touches a deployed service, `/cus/mosdns`, or the
main worktree's generated assets.

## What runs

| Component | What it really is |
| --- | --- |
| DNS + management host | the real Rust-native binary `rust/target/debug/mosdns start -c <fixture>` |
| UI | the real maintained Vite dev server for `webui-log`, with `MOSDNS_DEV_TARGET` explicitly set to the isolated native HTTP port |
| Browser | real headless Google Chrome (CDP over `--remote-debugging-port`), not a DOM stub |
| DNS oracle | real UDP queries to the isolated DNS listener (`reject 3` vs `reject 0`) |
| Failure injection | CDP `Fetch` request interception (a controlled browser-transport fault), which is the boundary the task design allows |

Vite's `closeBundle` stamps the tracked generated assets (`coremain/www/log.html`,
`coremain/www/assets/vue-log/index.html`) even when it runs as a dev server, so
the harness serves the UI from a **disposable copy** of `webui-log` plus a copy
of `coremain/www` and asserts afterwards that the main worktree copies are
byte-identical.

## Run it

```bash
cd /Users/tom/github/mosdns-rust/rust && cargo build -p mosdns-native-host
cd /Users/tom/github/mosdns-rust/webui-log && npm ci
cd /Users/tom/github/mosdns-rust
node .trellis/tasks/09-28-rust-native-domain-set-management/research/slice3-browser-proof/harness.mjs
```

`--keep` keeps the disposable work directory; `--output <path>` writes the
evidence JSON elsewhere. The default evidence file is `evidence.json` next to
this file.

## Checks proven (54/54 on 2026-09-29, remediated revision)

The first run of this harness proved 45 checks; the review remediation added the
differing-canonical scenario below and the harness now proves 54. The 45-check run is superseded
history; `evidence.json` is the 54-check remediated run.

Additional check proven after remediation:

- POST 200 followed by a failing canonical `/show` for a tag, then the server content changed behind
  the UI's back: the next save retries the canonical read, keeps the local draft in the editor, stays
  dirty, reports "server content re-read, local edit preserved" (and does **not** claim a server-side
  adjustment), does not silently adopt the server content, and only the next explicit save submits
  the local edit.

- The Vite dev proxy destination is the isolated native host: `/api/v1/special-groups`
  and `/plugins/blocklist/show?limit=10000` answer through Vite exactly as the
  direct native port does.
- Real navigation: Rules -> 本地规则 -> 黑名单 loads the configured tag.
- Edit + save through the page changes the rule file and **the next real UDP
  query** (`ui-added.example` -> NXDOMAIN, unrelated name -> NOERROR).
- A page refresh and a native-host restart both retain the accepted rules.
- A submitted invalid rule (`regexp:[`) is skipped by the server; the editor
  shows the server's canonical rules and the user is told the content was
  adjusted, so the submitted text is never presented as canonical.
- A failed POST keeps the draft, leaves the file untouched and reports an error.
- POST 200 followed by a failing canonical `/show`: that tag keeps a recoverable
  draft, is reported as "server save succeeded, current contents unconfirmed",
  is excluded from the confirmed count (`已保存 1 个列表`), is visibly marked, and
  the other dirty tag in the same save reconciles independently.
- A later save retries the canonical read, reconciles without repeating the POST,
  clears the uncertainty and clears the dirty marker.
- Both isolated listeners are released after shutdown, and the main worktree's
  generated assets are untouched.

## Known unrelated noise

`pageErrors` in the evidence file contains 404 polling errors from the Overview
card (`DnsOverviewCard` -> `/api/...` dashboard endpoints that this scoped task
does not implement). They are unrelated to the local-rule flow and are recorded
rather than hidden.

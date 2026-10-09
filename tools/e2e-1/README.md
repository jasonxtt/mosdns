# E2E-1 full-chain differential

VM Linux root, Python 3 + PyYAML, iproute2, util-linux, OpenSSL required.
Build binaries on the test VM before running; this tool never builds or deploys.
Inputs are config_all/config_rust plus snapshot/{rule,gen,srs,adguard,webinfo}.
The output directory must not exist. Keep inputs and outputs outside the repo.

```sh
python3 tools/e2e-1/run.py --inputs /private/input --work /private/run \
  --go /private/mosdns-go --native /private/mosdns-native \
  --migrate /private/mosdns-settings-migrate --names 2000 \
  --corpus /private/round1-corpus.json
```

Each host runs with its mocks in a loopback-only netns and private /tmp mount.
Original system mosdns is not touched. All processes/namespaces are cleaned on
success/failure; the private run directory is retained for analysis. Archive
needed replay evidence outside the repository, then remove the VM task workspace.
Raw results contain private names and ECS: only index-based summaries belong in
repository evidence. The mapping is protocol-preserving, uses process-local TLS
trust, and refuses unknown enabled protocols/groups or Go YAML fallback.
The tool supplements real rules with reserved example.com probes in both disposable
copies (.test is already blocked by the snapshot). Every configured audit-enabled
UDP port has per-query correlation; other ports provide DNS/mock evidence only.
Query errors, missing audit,
incomplete streams and unexpected inputs never silently count as agreement.
Comparison first emits unclassified differences; it is not an acceptance verdict.
The pinned corpus must equal the independently prepared corpus and is copied
byte-for-byte. TTL checks cover non-OPT Answer/Authority/Additional sections:
rewrite TTL5; mock maximum300 (3099 wrapper5); warm cache aging against elapsed
monotonic time, with one second of rounding, unless the mock journal proves a
fresh invocation. Empty local responses have no TTL; frozen FakeSOA TTL300 is
checked, and unrecognized local records fail rather than receive a generic cap.
`ttl-policy.jsonl` contains sanitized per-index results. `compared.jsonl` is
private: audit `final_upstream_group` uses enabled entry/group aliases, including
the migrated google-foreignecs rename. Ambiguous/unmapped aliases fail closed;
raw `final_upstream` contributes only a separate diagnostic count.
Classification and external review follow the task PRD and frozen contracts.

```sh
cd tools/e2e-1 && python3 -m unittest test_codec.py
```
# M1-e synthetic special groups

`special_groups.py run --work <fresh VM directory> --go <Go binary> --native
<native binary> --browser <absolute special_groups_browser.mjs>` creates one
loopback-only namespace per host. It uses synthetic rules and A/AAAA mocks,
saves two groups through HTTP, checks main-flow priority over DDNS and a
custom-only port, compares cold/warm DNS and correlated audit routes, compares
schema-4 bytes except only per-group upstream blocks, and imports each host's
state into the other. Browser smoke performs Vue create/edit/delete with source
references on native `/` and `/log`, and Go `/`. Playwright can be supplied via
`PLAYWRIGHT_MODULE`; its browser must already be cached. No public networking is
available inside these namespaces. Finally all owned processes and namespaces
are stopped, and the original mosdns service identity is checked.

Run tool guards on the VM with `python3 -m unittest discover -s tools/e2e-1`.

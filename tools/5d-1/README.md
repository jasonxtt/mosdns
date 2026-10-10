# 5D-1 single-core baseline

VM Linux root, existing Python3/PyYAML, iproute2, util-linux and OpenSSL.
No downloads, installs, builds or deployment performed by this tool. Build
exact pinned source offline with the repository scripts first (Vue before Go).
All paths below are private VM directories outside any repository checkout.

```sh
python3 tools/5d-1/run.py prepare --work /private/prepared \
  --inputs /private/inputs --migrate /private/mosdns-settings-migrate
python3 tools/5d-1/run.py run --work /private/prepared --mode oracle \
  --output /private/oracle-run --binary /private/mosdns-go
python3 tools/5d-1/run.py run --work /private/prepared --role go \
  --binary /private/mosdns-go --output /private/go-r1 --seed 501
```

Inputs: config_all/config_rust and snapshot/{rule,gen,srs,adguard,webinfo}.
E2E-1 preparation preserves protocol/proxy and rule data in disposable copies,
with reserved example.com probes. All top_domains counts contribute, 98%
weighted production +2% block/rewrite/unmatched, A70/AAAA30 (assumption).
Private Go oracle preserves rcode/question/answer/authority/additional; only TTL
is removed for semantic comparison (separate bounded0..300 check). It is a
functional reference, not an independent whole-product correctness proof.
Prior E2E covers correlated audit routing; this load keeps audit on but does
not scrape per-query audit. No performance oracle request is counted as load.

Each run creates its own loopback-only netns and private /tmp mount, mock and
sender CPU1, SUT/all inherited threads CPU0, Go GOMAXPROCS1/default GC.
Mock latency per exchange: domestic10ms/foreign80ms/foreignecs80ms/cnfake1ms/
nocnfake1ms. Same TLS trust and restricted SOCKS5 as E2E-1. Fresh cache dumps,
same audit/cache/log settings, serial hosts. Original service identity checked
before/after; owned process groups/netns cleaned even on failure. Runtime roots
and evidence are retained for private archiving, then delete the owned VM root.

Default ladder: warmup100qps10s;20/100/500/1000/2000/4000/8000/16000 each30s
(first5s settling); immediately recovery100qps60s. All random indices, IDs and
wire packets prepared before timing. Absolute monotonic schedule,128 sockets,
unique socket/ID pairs, no response-paced sending or catch-up burst for slots
over5ms late.500ms deadline from planned slot. Client unsent/send error counted
separately; timeouts/wrong/late remain in denominators. Received quantiles are
reported alongside completion rate, never a standalone overload success claim.
Both planned-to-finish and sent-to-finish latency retained. CPU/RSS/FD/thread,
per-CPU busy/steal and helper resources sampled250ms; metrics collected outside
timed ladder. Raw requests are private and include corpus indices and times.

Run Go >=3 repetitions (501/502/503); stop and freeze thresholds through the
user before native formal measurement. Native tool smoke only:

```sh
python3 tools/5d-1/run.py run --work /private/prepared --role native --smoke \
  --binary /private/mosdns-native --output /private/native-smoke \
  --rates 20 --duration 3 --warmup 1 --recovery 3
```

Native formal run requires `--freeze-record /private/frozen-thresholds.md`.
The file requirement is an execution guard; user approval remains authoritative.
After freeze repeat fresh Go/native interleaved; retain initial Go and new
control observations. Capacity cannot be inferred at sender-limited rates.
Noise is recorded, not a tool-driven invalidation/retry gate.

One previously approved E2E AAAA cell has a different native expected response
under the frozen `full:` case-insensitivity contract (2026-10-07). Pin it from
accepted private E2E evidence with `approved_case.py --corpus ... --evidence ...
--nov6 ... --oracle ... --native-oracle ... --output ...`, then pass that file
as `run.py --oracle /private/native-oracle.json` for native. Every other cell
remains unchanged; no response field or broad case class is waived. Evidence,
corpus and output are private. The sender implementation is identical to the
Go baseline; public provenance records only the index and hashes.

Tool tests (inside fresh loopback namespace, port53 must be unused):

```sh
unshare --net sh -c 'ip link set lo up; exec python3 -m unittest discover -s tools/5d-1'
python3 -m unittest discover -s tools/e2e-1
```

Do not publish private corpus/oracle/requests/mock logs/full metrics/configs.
Public summaries use only numeric statistics, hashes and source identities.

5D-fix-A uses `fix_a_matrix.py --root ... --freeze-record ...`: F followed by
normal20/100/500, three Go/native pairs in G/N,N/G,G/N order, original timing
and byte-identical frozen JSON. It pins tools, binaries and both oracles before
the first sample and retains every attempt; no automatic retry/resume.
`approved_cells.py` prospectively extends only the exact462 cells accepted by
the user after the 5D-1 Claude review. It checks all three prior probes against
the immutable ASCII uppercase `full:` no-v4/no-v6 rules; candidate replies are
not an input. The earlier single approved AAAA cell remains pinned. Unchanged
cells and Go oracle keep their original strict full-section comparison.

Scenario F (user frozen2026-10-09) adds `--scenario-f --rates 100` to run.py.
In disposable roots only, it adds an identical reserved-suffix foreign branch
after `$top_domains` (after the production outer cache), before A/AAAA routing;
foreign is one trusted DoH endpoint through the restricted SOCKS5 proxy. It
keeps production policy/mapper/cache/statistics/audit before that branch, but
does not claim coverage of the ordinary fake-IP routing decision. Unique names
are sequential across warmup,stable and recovery, with A70/AAAA30. No name is
reused; analytical full-section oracle pins mock foreign addresses and question.
Confirm zero cache hits, only foreign:https DNS exchanges and allowed SOCKS
connections. The100QPS stable25s window sets F latency thresholds; F's60s tail
is continued unique-name load, not an overload-recovery experiment.

`matrix.py --root /private/task-root --freeze-record /private/frozen.json`
serially runs three paired repetitions of main,extension,F, with Go/native,
native/Go,Go/native order. It hashes the immutable freeze and saves the exact
plan/completion manifest. Initial Go/F baseline and new Go controls remain
separate. Formal sender tail classification uses the same semantic/TTL/deadline
test as the timed loop, including replies after the final stage ends; it does
not label an on-time drain response late merely because draining has started.

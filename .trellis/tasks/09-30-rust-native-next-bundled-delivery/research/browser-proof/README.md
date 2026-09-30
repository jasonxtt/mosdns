# Slice 3 browser proof

This evidence covers the maintained `webui-log` Vue source against an isolated
Rust-native host. It intentionally does not claim full OverviewManager,
SystemControlManager, QueryManager, or `/log` compatibility.

## Fixture and topology

- Vue source: exact working-tree `webui-log/` copied to a disposable `/tmp`
  directory; `npm ci --ignore-scripts` and `npm run build` ran only there.
- Vite: `127.0.0.1:25173`, with `MOSDNS_DEV_TARGET=http://127.0.0.1:25099`.
- Native UDP host: `127.0.0.1:25053`, HTTP `127.0.0.1:25099`, fixture
  `fixture-udp.yaml`, upstream `udp://1.1.1.1:53`.
- Native TCP host: `127.0.0.1:25054`, HTTP `127.0.0.1:25100`, fixture
  `fixture-tcp.yaml`, upstream `tcp://1.1.1.1:53`.
- Native state root was an isolated temporary directory; no live MosDNS
  service or `/cus/mosdns` path was used.

## Observations

1. Real UDP `dig @127.0.0.1 -p 25053 example.com A` returned
   `172.66.147.243` and `104.20.23.154`. The Vue DNS card then showed
   `1`, `109.78 ms`, and a `最近查询` row `example.com A ... 109.78 ms`.
2. Clicking `查询趋势` opened all five native window results: 1 hour, 6
   hours, 24 hours, 3 days, and 7 days, each showing request count 1 and the
   same average duration. The card had no audit API warning. The containing
   Overview still showed the expected deferred `加载概览失败: HTTP 404 Not
   Found` from unsupported rank endpoints.
3. Clicking `停止审计` showed `已停止`. A second real UDP query still
   returned the same DNS answers, while v2 stats stayed at
   `{"total_queries":1,"average_duration_ms":91.305}` before and after it.
   Clicking `启动审计` returned the panel to `运行中`.
4. The clear and capacity controls opened their explicit confirmation UI. The
   destructive confirmation was not auto-submitted; equivalent real v1 calls
   were then made against the same host: `POST /api/v1/audit/clear` reset v2
   stats to zero while capture stayed true, and
   `POST /api/v1/audit/capacity` with `{"capacity":7}` returned success. The
   System panel reread `7`, and the isolated settings bytes were exactly
   `{"capacity":7}` apart from formatting.
5. After stopping and starting a fresh native process with the same config,
   `GET /api/v1/audit/capacity` still returned `{"capacity":7}` and a real
   UDP query still answered.
6. With the native process deliberately stopped, clicking the UI stop action
   produced visible `切换审计状态失败: HTTP 500 Internal Server Error` and
   `最近一次操作失败: HTTP 500 Internal Server Error`; the UI did not change
   the displayed running state optimistically. The native process was then
   restarted and the audit panel reread normally.
7. Real TCP `dig +tcp @127.0.0.1 -p 25054 example.com A` returned the same
   answers. Its independent v2 read returned one retained record with
   `query_name: example.com`, `query_type: A`, `client_ip: 127.0.0.1`, and a
   numeric `duration_ms`.

## Build and cleanup evidence

- Disposable build passed with Vite 7.3.2, 616 transformed modules, and no
  output written to the main worktree's `coremain/www`.
- Native process and Vite were run only on loopback and were stopped after the
  proof. The temporary fixture directories are disposable `/tmp` paths.

## Independent completion recheck — 2026-09-30

The original proof above is retained as the execution agent's record. This
recheck used committed product HEAD `d7fc11ec3b061a5fe811fe4980e75d3d54b59ee0`
on the `mosdns-rust` VM. The source was copied with `git archive HEAD` into the
isolated `/root/mosdns-audit-verify-d7fc11ec` directory. Native UDP and HTTP
listened on VM loopback `127.0.0.1:25053` and `127.0.0.1:25099`; Vite listened
on `127.0.0.1:25173` with
`MOSDNS_DEV_TARGET=http://127.0.0.1:25099`. The local in-app browser reached
Vite through a task-owned SSH forward of local port 25173 to VM port 25173.
The tunnel process was PID 89127; the second native and npm launcher processes
were PIDs 542091 and 542092. No service or `/cus/mosdns` state was changed.

- `CARGO_PROFILE_DEV_DEBUG=0 CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 cargo
  build -p mosdns-native-host --bin mosdns` passed. `npm ci --ignore-scripts`
  and `npm run build` passed in the disposable VM copy (Vite 7.3.2, 616
  transformed modules), leaving main-checkout embedded assets untouched.
- The first `cargo clippy` attempt failed because the archive omitted the
  compile-time `tests/phase5a-baseline` fixture. After copying that fixture
  from the same HEAD, `cargo fmt --all -- --check`, `cargo clippy --workspace
  --all-targets -- -D warnings`, and `cargo test --workspace` all exited 0.
- A real VM `dig` query to UDP port 25053 answered. The browser DNS card
  subsequently showed one retained query and a recent-log row. In the System
  audit panel, the actual **clear** confirmation was submitted; its success
  message appeared, the v2 stats response returned zero, and the Overview DNS
  card reread as zero records with `0.00 ms` and no recent rows.
- The actual **capacity** confirmation was submitted with `7`. The panel
  showed success and capacity 7; the API returned 7 and isolated
  `webinfo/audit_settings.json` recorded 7. A fresh native process with the
  same config returned capacity 7, answered another real UDP query, and the
  browser panel reread 7. Start/stop state was also exercised through the
  panel; stopping retained capture state through clear, then starting restored
  `运行中`.
- The containing Overview still showed the deferred rank endpoint
  `加载概览失败: HTTP 404`. This is outside the DNS card and audit panel slice.
  The native process, Vite and local tunnel were stopped; task ports were no
  longer listening and `mosdns.service` remained active.

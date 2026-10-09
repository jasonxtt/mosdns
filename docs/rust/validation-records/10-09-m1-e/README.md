# M1-e standard special groups

2026-10-09; baseline `8fb054116000df96235a2e7c847c51f0f8d7832a` on `rust`.
Implementation and VM validation are complete; external Claude review is pending.
This record does not authorize production replacement.

Standard file configurations including `sub_config/special_groups.yaml` now
resolve v0.7.3 group state against YAML and native typed settings, expose group
CRUD/source APIs and capabilities, and apply complete generations with listener
changes. `native_management` remains frozen. The authority JSON/settings rename
commits after derived YAML; failure restores old derived bytes, startup repairs
from authority. See [group contract](../../contracts/native-special-groups.md)
and [settings contract](../../contracts/native-settings.md).

User-approved proxy rule: private use_proxy=true binds the sole socks5_proxy;
zero/multiple objects refuse with missing/ambiguous reasons, never silently
direct. The sole object's explicit empty address means no proxy as in M1-f1.
References inherit their source's resolved proxy and bootstrap defaults.

## Verification

All builds/tests ran on SSH alias mosdns-rust, Cargo jobs2, npm dependencies
installed offline. Vue bundles preceded Rust/Go builds.

| Gate | Result |
|---|---|
| fmt / strict workspace all-target Clippy | exit0 |
| Final workspace | 96 suites; 1468 passed, 0 failed, 4 existing ignored |
| Cache-audit repair native-host regression | 630 passed, 0 failed, 4 existing ignored |
| Focused group lifecycle / proxy / unrelated cache | 6 passed, 0 failed |
| M1-0 serial / Node / Python guards | 16/0; 37/0; 10/0 |
| Both Vue bundles, source-keyed native release, Go build | exit0 |
| Final native `check` on read-only config_rust copy | gaps=0 cascades=0 globals=0 definitions=128 |
| Two-group isolated differential and cross-import | 12 cold/warm A/AAAA rows per host, plus 12 after cross-import; DNS and schema outside upstream blocks equal |
| Actual browser workflow | native `/`, native `/log`, Go `/`: create/edit/delete, source reference, private upstream, custom-only port; no page errors |

The synthetic differential uses loopback-only namespaces and mock A/AAAA
upstreams. It saves a main-flow and a custom-only group, checks DDNS priority,
compares schema-4 bytes except the permitted actual-upstream block, DNS/audit
rows, and bidirectional state import. Browser smoke operates actual Vue editors.
Run4 completed after repairing the native source-list load. The same shared
component now requests the catalog only when `groups.upstreams` is supported;
the legacy Go branch remains unchanged. Build manifest verification matched all
348 input files. Native SHA256
`c3ce1bdb00b1e0a363e312cb8f635b5f7388c56b2364271c0600baed82f37a25`;
Go SHA256 `aacccb921ca65edc0f38160f6baa3c0e1bd6c33007e5e5a4505f875e56d0d3f1`.

Raw audits are not byte-equal: 10/12 rows differ. Eight neutral-domain rows use
Go's `unmatched_rule` versus native empty/absent fields; four group warm rows
have Go's missing final sequence/upstream versus native actual cache sequence
and supplier (two rows overlap). The strict oracle checks every expected route,
maps only the two known unique fixture supplier identities, refuses lost special
labels/unknown routes, and preserves all raw differences. This classification
is evidence submitted to the external reviewer, not a review PASS.

## Intentional differences and limits

Native hot apply replaces Go's delayed process restart. Actual entries and
unique native runtime identities live in generated upstream blocks; other
schema-4 bytes must match Go. Standard native does not consume Go override
files; older implicit owned-entry migration is outside this v0.7.3 state gate.
Typed private proxies follow the approved sole-object rule. Cache hits explicitly
retain stored labels across named sequence boundaries; Go's absent warm route
fields are not copied into native's richer provenance. Failed cleanup after a
committed deletion stops the host and reports committed cleanup failure; it
does not falsely claim a precommit rollback.

Intermediate failures are retained in task research: invalid initial RED fixture,
async test destructor, lint corrections, real bootstrap inheritance RED/GREEN,
the superseded startup-read-only assumption, an AddrInUse test run followed by
serial success, Node missing dependencies before offline npm ci, a text-response
differential client failure, the real warm-cache audit RED/GREEN, and a browser
source-list failure fixed before final success. No failure record was replaced
with PASS.

No commit, push, deploy, Go product/config package edits or production/default
switch. All four differential runs stopped their owned processes/namespaces and
confirmed the original service unchanged. Final workspace removal evidence and
review handoff are recorded with this task's research.
At `2026-10-09T08:10:06.477179+00:00` the owned VM workspace was removed;
no owned processes/namespaces remained and the original service stayed active
with the same PID. See [final cleanup](cleanup-final.json),
[differential result](result.json), [raw classified differences](audit-differences.json),
[native browser](browser-native.json), [Go browser](browser-go.json),
and [local/VM source closure](source-closure.json). Private detailed logs and
screenshots remain in the task's evidence archive outside the repository.

## 外部审查结论（2026-10-09）

Claude 第 1 轮 **FINAL: PASS**（无 P0/P1）。遗留 P2：
- 提交后清理已删除组的文件失败会停止 host，且启动时孤儿清理失败也会阻止启动；应改为非致命（告警、在 health/API 标注、稍后重试）——并入 5D 可用性加固；
- 无 mapper 默认值的配置中，原生 `domain_set` 为空而 Go 写 `unmatched_rule` 哨兵（`effective_tag` 一致）——与 E2E-1 的审计显示差异一并写入 `native-audit-control.md`。

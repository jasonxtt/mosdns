# DoH HTTP/2 scheme 与 authority

基线 `5a5ee9ec`，分支 `rust`。实现与规定验证完成，**等待外部 Claude 审查**。
未提交、推送、部署或发布；Shadow-1 继续暂停。

一次性与复用 DoH 请求均按协商协议构造：h2 显式 HTTP/2 版本、https scheme、
服务 URL authority（含非默认端口）和原 path/query，省略普通 Host；
h1 继续 origin-form + Host。数值拨号、SOCKS、SNI、证书信任与连接池策略不变。
契约：[DoH reuse](../../contracts/rust-migration.md#scenario-native-doh-bounded-concurrent-reuse-2026-10-09-revision)、
[h2 validation](../../contracts/quality-guidelines.md#scenario-native-doh-http2-child-ownership)。

## RED / GREEN 与 VM 检查

旧代码叠加严格 h2 夹具后，一次性非默认端口与复用 SOCKS 两条回归各 1 项失败，
均为实际协议拒绝后的发送错误；修改实现后均通过。
新增 3 项，覆盖一次性数值 dial、一次性 SOCKS 与复用 SOCKS 的非默认服务端口。
严格服务端检查 h2 URI 的 scheme/authority/path、拒绝额外 Host，复用场景证明
连续 3 请求只建 1 个连接；一次性 2 请求建立 2 个连接。

仅在指定 VM、离线、2 jobs；未在本机编译。189 个 Rust 源文件与工作树 hash 一致。

| 检查 | 结果 |
| --- | --- |
| DoH 聚焦 4 套件 | 65 通过、0 失败 |
| fmt / workspace-all-targets Clippy -D warnings | 通过 |
| workspace 全量（92 个目标） | **1507 通过、0 失败、1 既有忽略** |
| 相对基线 | 通过数 1504→1507，新增 3 项 |
| native-host | 327 通过、1 既有忽略，无变化 |
| 标准 native 构建脚本 | UI 与 Rust 顺序离线构建成功，release 阶段 2m32s |
| 完整配置检查 | gaps=0、cascades=0、globals=0、definitions=128 |

唯一忽略为父测试启动的 switch_crash_child；没有新增忽略。
候选二进制 SHA256：`14eaa1c13b956e83ab5172c88405950c34d1326c854b134023bfecc7c019a487`。
初次 Clippy 发现测试 helper 行数与冗余闭包问题，修正后上述最终检查全部通过。

## rtdns 真实私有夹具

同一真实服务/代理的独立无缓存夹具，只使用 loopback 高端口与私有候选程序。
原始配置、端点、DNS wire、pcap 和 TLS keylog 均留在仓库外。

| 路径 | 查询 | NOERROR | HTTP / 观察结果 |
| --- | --- | --- | --- |
| 原代理 + 数值拨号 | 3 | 3 | 回应关联与问题匹配 3/3 |
| 原代理 + 代理解析域名 | 3 | 3 | 回应关联与问题匹配 3/3 |
| h2 观察中继 | 3 | 3 | HTTP 200 ×3，RST_STREAM ×0 |
| h1 观察中继对照 | 3 | 3 | 原始 HTTP 200 ×3，origin-form/Host 正确 3/3 |

h2 解密 GET 3/3 为 https scheme、正确服务 authority、完整原 path/query、无普通 Host。
中继前后端均协商 h2，真实后端证书校验成功；不重写头部。两个协议各 1 连接承载 3 查询，
中继错误 0。h1 计数以原始解密流为准（tshark JSON 仅列出 2 个 response code）。

规则下载明确仅 HTTP/1.1；更新/配置下载等沿用该客户端。HTTP/3 本来使用完整 URI。
没有其它产品 HTTP/2 客户端同类缺陷需要修复。

## 清理与边界

任务进程 0，构建 VM 临时目录已删除，证据在仓库外私有归档。
rtdns 主服务 PID/重启次数/安装程序 hash 均未变；Shadow guard 三计数均为 0，
回放命名空间无进程。未连接 mos/esxi，未修改既有 Shadow 工具或恢复影子测试。
结果仅证明本次协议修复与所列验证，不代表生产切换或长期稳定性验收。

## Planner verdict (2026-10-19)
Round 1 `FINAL: PASS`. Found by Shadow-1 real traffic: 11/44 SERVFAIL against a strict DoH upstream. The diagnosis
and the real recheck are recorded only outside the repository.

# Source audit — revision 2, planning only

基线 rust/11bd56c40d255d6ae93b0a2eba1c85214300b149。2026-09-27 本地只读源码与配置包；未 SSH/构建/查询。

| 证据 | 本批决定 |
| --- | --- |
| native config.rs:141–145,201–202,230–234,compile_w3 | 去掉按 3/4/6 插件/固定序列选场景；真正编译可组合定义 |
| native assembly.rs ForwardCatalog | 复用 ID→owner 和统一 close，不新建动态注册框架 |
| native execution.rs:216,231,297–323,MachineStep::Complete | 去掉 forward 数控制错误策略、固定 entry、root-only cache token；组合必须涉及执行边界 |
| native cache.rs CACHE_CAPACITY/NativeCache | 原 W2 参数固定；将 size 接到已有有界缓存，不改 key/TTL/admission 模型 |
| sequence-core program.rs ExecutableSpec/engine.rs ScopeKind,finish_scope | 无 direct named Call；Jump 同 scope、Try 捕获 exit，均不能代替普通 child call；在既有 machine 最小扩展 |
| sequence-core program.rs inline lowering | exec list 已有独立 scope，保留产品语义，不在 host 重演 AST |
| matcher-core mix.rs MixMatcher/add | full/domain/regexp/keyword 已有，复用规则引擎；补文件/多条规则和 host matcher 接入 |
| coremain/mosdns.go:427–447/config.go | Go include 先加载，按声明文件 baseDir 解析；本批支持实际单层形式，嵌套显式延期 |
| plugin/data_provider/domain_set 与 native matchers.rs | 定向确认 rules/files/注释/路径；不把当前单 full expression 当最终产品限制 |
| plugin/executable/sequence/sequence.go:195/config.go/built_in.go | direct Exec 是单独 walker，child accept/reject 返回，exit 传播，try 捕获 exit；只提取用户行为 |
| plugin/executable/cache/cache.go:381,490–512 | cache 包裹其后继，返回后保存；不能统一保存根终态响应；错误残留不发布的安全修正需说明 |
| plugin/executable/forward/forward.go:198–227 | 显式再次 forward 实际执行；不能引入 HasResp 隐式跳过 |
| native execution response_from_state/dns-core header/Phase3B R3 | synthesized u16→u8/低4bit wire 边界；支持常用 0..15，其他明确 unsupported，完整 12-bit 不从最终范围删除 |
| config_lite_all/config_custom.yaml:10–48,76–87; sub_config/forward_nocn.yaml:17–23 | include/reject/direct child/cache 是实际依赖；fixture 仍须标明 aliapi/switch/lazy 等裁剪与替换 |
| docs/rust/phase5a-measurement-reliability.md | 已停止，不补跑；容量/恢复/热点未知，不凭此改 Send/runtime |

## Compatibility classification

- preserve：配置/规则顺序、命名调用 scope、cache 后继、wire/路由/基础审计、取消/期限/停止。
- 本批扩展：direct child、单 cache 的真实组合、常用 reject、顶层 include、provider files/exps、qtype/has_resp。rev1 的延期不再阻挡这些必需能力。
- 明确后续：完整配置包、多 cache/lazy/dump、扩展 RCODE、嵌套 include、高级上游/其他 plugin、API/状态；不静默接受，不删覆盖项。
- implementation-only：Go 递归/interface/fast-path、动态 quick-setup registry、固定 W1/W3 图识别，不复制。
- 定向技术核对：规则文件相对路径/格式、cache 受控完成通知和响应资格；不能改变 A1–A5 再声称实现完成。

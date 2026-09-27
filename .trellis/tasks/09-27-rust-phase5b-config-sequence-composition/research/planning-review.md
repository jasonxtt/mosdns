# Local planning consistency — revision 2

用户于 2026-09-27 要求按“真实链路优先、避免过度设计”修改规划。三份文档已收敛；这是一致性检查，不是独立 review PASS 或产品验收。

- 目标从放宽测试图改为一条实际配置派生的规则/child/cache/upstream 链；替换和裁剪已注明。
- R1–R5/A1–A6 保留编号，重新映射新增能力；直接调用/cache 后继/常用 reject 不再全延期。
- 移除六 slice/四门禁，改为跑通链路、兼容故障、Linux 交付三步；第一小目标是实际运行结果。
- 保留必要语义、故障/资源检查及最终审查；普通功能修复可合理重验，不复用正式实验的重跑预算。
- 性能分日常检查、代表链诊断、最终完整验收；不每批建设矩阵，不承诺未测指标。
- 旧规划输入 planning-input-v1.json 仅为 revision 1 历史记录，已被本版替代；不生成每次编辑的新版 digest，不把它作为本版准入条件。
- 保留 planning、implementation_authorized=false、reviewer=unassigned、auto-commit=false；未启动实现/SSH/测量/外部审查。

独立规划审查与实现授权尚未发生。本轮只校验文档、链接、metadata，执行检查见 implement.md。

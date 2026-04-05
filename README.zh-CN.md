# mosdns

[English README](README.md)

这是一个基于 `yyysuo/mosdns` 的增强版 fork，重点补充了 WebUI 对“专属分流组 / 专属上游 / 在线规则自动下载 / 上游热重载”的支持。

这个分支的目标不是重写 `mosdns`，而是在保留上游整体结构和配置思路的前提下，补齐更适合长期在线维护 DNS 分流规则的工作流。

## 配置包

这个 fork 维护中的配置包放在：

- [`mosdns/config/config_all.zip`](https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip)
- [`mosdns/config/config_up.zip`](https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_up.zip)

其中：

- `config_all.zip` 用于新部署或整套模板替换
- `config_up.zip` 用于现有部署的增量配置更新

旧的 `config_tom.zip` 模板已经下线。

完整配置包解压后的运行目录应为：

- `/cus/mosdns`

## 上游来源

- 上游仓库：`yyysuo/mosdns`

## 当前增强点

### 专属分流组

- 可在 WebUI 中创建“专属分流组”
- 按当前实现，理论上最多支持 10 个专属分流组
- 每个组都可以绑定自己的：
  - 在线规则
  - 上游组
  - 缓存

### 规则和上游联动

- 在规则管理中直接把规则归入某个专属分流组
- 命中该组的域名会优先走绑定的专属上游

### 在线规则保存后自动下载

- 新增在线分流规则后自动下载 `.srs`
- 不再需要手动点击“更新”

### 上游热重载

- WebUI 保存上游组后即时生效
- 不再需要手动重启 `mosdns`

### 查询日志显示优化

- 日志中按当前组名显示专属分流组
- 同时补充匹配来源、最终上游组和最终上游，便于排查

## 适用场景

- 家庭网络分流
- 旁路 DNS
- 代理环境下的域名分流维护
- 需要通过 WebUI 长期维护 geosite / srs 规则的场景

## 相对上游的改动

当前已经落地的增强包括：

- 专属分流组 API 与 WebUI
- 上游配置热重载
- 在线规则自动下载
- 查询日志中的专属分流组友好显示

详细说明见：

- [相对上游的改动说明](docs/fork_diff_summary_zh.md)

## 发布状态

当前正式发布版本为：

- `v0.1.8`

这个 fork 现在已经按持续维护的 WebUI 增强分支在发布，不再是早期预览版定位。

## 文档

- [项目简介草案](docs/github_project_intro_zh.md)
- [相对上游的改动说明](docs/fork_diff_summary_zh.md)
- [GitHub 发布前清单](docs/github_release_checklist_zh.md)

## 致谢

本项目基于：

- `yyysuo/mosdns`

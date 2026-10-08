# GitHub 发布前清单

## Linux / Docker 共用主线的发布检查

- Linux 与 Docker 在 `main` 共用核心代码和 WebUI，通用改动无需同步 `docker` 分支。
- 发布正式 `vX.Y.Z` 标签前，构建两套 Vue 资源，再运行 `go test ./...`、`scripts/test-docker-release.sh` 和镜像运行检查。
- 在 Actions 中配置 `DOCKERHUB_USERNAME`、`DOCKERHUB_TOKEN`，不要将凭据写入仓库。
- 标签推送后分别确认 Linux Release 和 Docker 工作流成功；两条流水线独立，单个平台失败不代表全部发布成功。
- 核对 Linux 包和容器内的程序版本相同；Docker 镜像 revision 应等于版本标签对应的提交，manifest 包含 amd64、arm64。
- 首次迁移发布应使用旧 Docker 数据卷验证升级，并检查 `/`、`/log`、DNS、配置持久化及 host/bridge 模式行为。
- 迁移验收前保留旧 Docker 分支、镜像和工作目录。回退时使用原镜像及对应配置备份。

## 一、仓库准备

- 确认 fork 来源写清楚：`yyysuo/mosdns`
- 确认保留上游许可证与版权说明
- 确认仓库名
- 确认仓库简介
- 确认默认分支名称

## 二、README 至少要有的内容

- 项目简介
- 和上游的关系
- 当前增强点
- 适用场景
- 快速开始
- WebUI 主要能力说明
- 配置兼容性说明
- 已知限制
- 致谢 / 上游链接

## 三、发布前最好补上的文件

- `README.md`
- `CHANGELOG.md`
- `docs/github_project_intro_zh.md`
- `docs/fork_diff_summary_zh.md`
- `docs/github_release_checklist_zh.md`

## 四、发布说明建议

建议说明重点：

- 新增专属分流组
- 新增上游热重载
- 新增规则保存后自动下载
- 新增查询日志中对专属分流组的友好显示

## 五、推送前需要再确认的内容

- 是否保留旧 UI 入口
- 是否保留备用页面 `log_plain.html`
- 是否把当前配置样例一起开源
- 是否需要单独放一个样例配置仓库
- 是否要在 README 中附带截图

## 六、我建议的仓库结构

- 源码仓库只放程序和文档
- 用户自己的实际配置不要直接混在源码仓库里
- 如果要提供样例配置，建议单独放：
  - `examples/`
  - 或独立配置仓库

## 七、推送到 GitHub 前的最后动作

1. 清理不想公开的本地调试内容
2. 再检查一次 `git diff`
3. 写一版正式 `README.md`
4. 写当前版本对应的 `CHANGELOG.md`
5. 确认是否要保留中文为主，还是中英双语
6. 再决定是直接 push 到 fork，还是先建新仓库再导入

## 八、建议的首版 README 目录

1. 项目简介
2. 和上游的关系
3. 主要增强点
4. 使用场景
5. WebUI 功能概览
6. 安装 / 编译
7. 配置说明
8. 已知问题
9. 致谢

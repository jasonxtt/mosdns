# mosdns Docker 部署

本文档说明如何把当前 fork 以标准容器方式部署。容器版保留现有运行目录和配置包结构：

- 容器内运行目录固定为 `/cus/mosdns`
- 配置、运行时状态、备份、生成列表都继续写入 `/cus/mosdns`
- 升级方式改为更新镜像并重建容器，不再通过 WebUI 覆盖容器内二进制

## 开发与自动发布

Linux 原生版和 Docker 容器版共用 `main` 的核心代码及 WebUI。通用修改在主项目完成；容器差异通过 `MOSDNS_CONTAINER_MODE` 等环境变量启用。旧 `docker` 分支保留作迁移回退，完成迁移验收后不再用于日常开发或同步。OpenWrt 和 lite 继续使用各自的分支流程。

正式 `vX.Y.Z` 标签触发两个独立工作流：Linux 发布包沿用现有流程，Docker 自动构建并推送 `linux/amd64`、`linux/arm64` 到 `jasonxtt/mosdns-t:vX.Y.Z` 和 `jasonxtt/mosdns-t:latest`。两者均从标签对应的提交构建。Docker 发布验证版本格式和该提交属于 `main`，然后运行后端测试、镜像构建及容器运行检查。普通 main 提交、预览版和其他版本线标签不会发布正式镜像。

相关 PR 只构建、测试 `amd64` 镜像，不推送，不使用 Docker Hub 凭据。Docker 发布失败不会阻断 Linux 发布；发布后应分别核对两条工作流结果，失败时修复后重跑 Docker 工作流。

首次自动发布前，在 GitHub 仓库的 **Settings → Secrets and variables → Actions** 配置 `DOCKERHUB_USERNAME` 和 `DOCKERHUB_TOKEN`；token 需具备 `jasonxtt/mosdns-t` 的推送权限。凭据只保存在 Actions Secrets 中。

正式镜像内的程序版本与 Linux 包一致，均为完整 `vX.Y.Z`。镜像的 `org.opencontainers.image.revision` 记录源提交 SHA；构建时间记录在 `org.opencontainers.image.created`。

本地运行镜像回归检查：

```bash
python3 scripts/test-container.py mosdns:docker --platform linux/amd64
# Apple container 上运行 arm64 镜像：
python3 scripts/test-container.py mosdns:docker --engine container --platform linux/arm64
```

测试使用临时配置和数据目录，覆盖空数据卷自动初始化、TCP/UDP DNS、两套 WebUI、容器能力限制、自重启与数据卷复用。Docker 上验证实际 host 网络；Apple container 上通过 VM 网络验证 host 模式能力标志。下载回退和非空目录保护另由 Go 测试覆盖。

重要说明：

- 常见部署会依赖同机的 `sing-box` / `mihomo` / fakeip DNS 之类的伴生服务。
- 如果配置文件里仍然写着 `127.0.0.1:1053`、`127.0.0.1:6666` 这类地址，bridge 模式容器内会把它们解释成 `mosdns` 容器自己，而不是宿主机或其他容器。
- 这会导致 WebUI 能打开，但真实 DNS 查询返回 `SERVFAIL`。

## 1. 构建镜像

在仓库根目录执行：

```bash
docker build \
  --build-arg VERSION="$(git describe --tags --match 'v*' --abbrev=0)" \
  --build-arg BUILD_DATE="$(date -u +%Y%m%d)" \
  --build-arg VCS_REF="$(git rev-parse HEAD)" \
  -t mosdns:docker .
```

如需多架构构建，可使用：

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  --build-arg VERSION="$(git describe --tags --match 'v*' --abbrev=0)" \
  --build-arg BUILD_DATE="$(date -u +%Y%m%d)" \
  --build-arg VCS_REF="$(git rev-parse HEAD)" \
  -f Dockerfile_buildx \
  -t mosdns:docker .
```

如果不传 `VERSION`，程序版本为 `dev`。`BUILD_DATE` 和 `VCS_REF` 只用于镜像元数据，不再拼接到程序版本中。本地构建不会推送镜像。

### 1.1 在 macOS 上节省 `apple/container` 资源

自动发布默认由 GitHub Actions 执行。以下 Apple 本地发布脚本仍保留，适合手动补发镜像或排查构建。

如果这台 Mac 平时不运行容器，只在发布 Docker Hub 时临时使用 `apple/container`，不要让 builder 常驻。

对这个仓库，推荐让 AI/代理直接执行 [scripts/publish-dockerhub-apple.sh](../scripts/publish-dockerhub-apple.sh)，不要让用户手动拼构建、推送、manifest 命令。

这个脚本会自动完成：

- 按当前 `Dockerfile_buildx` 构建 `linux/amd64` 和 `linux/arm64`
- 推送 `:<version>-amd64` 和 `:<version>-arm64`
- 合成并校验多架构 `:<version>`
- 在 `PUSH_LATEST=1` 时同步更新并校验 `:latest`
- 默认在发布成功后自动删除 `:<version>-amd64` 和 `:<version>-arm64`
- 默认 `VERSION` 同时作为镜像构建版本与 Hub tag；单独补发镜像时可额外传 `IMAGE_VERSION`
- 构建阶段只在需要时临时启动 builder，结束后自动停止

默认使用：

- `IMAGE_REPO=docker.io/jasonxtt/mosdns-t`
- `VERSION=$(git describe --tags --match 'v*' --abbrev=0)`
- `PUSH_LATEST=0`
- `KEEP_ARCH_TAGS=0`

AI 常用发布方式：

```bash
VERSION=v0.6.3 ./scripts/publish-dockerhub-apple.sh
```

如果只是补发镜像，程序版本仍保持 `v0.6.3`，但 Docker Hub 需要一个新的 tag，可分开传：

```bash
VERSION=v0.6.3 IMAGE_VERSION=v0.6.3-d1 ./scripts/publish-dockerhub-apple.sh
```

这会让镜像发布到 `:v0.6.3-d1`，但构建时仍沿用 `v0.6.3` 这条主版本线。

如果这次发布也要更新 `latest`：

```bash
VERSION=v0.6.3 PUSH_LATEST=1 ./scripts/publish-dockerhub-apple.sh
```

补发镜像同时更新 `latest` 时也是同理：

```bash
VERSION=v0.6.3 IMAGE_VERSION=v0.6.3-d1 PUSH_LATEST=1 ./scripts/publish-dockerhub-apple.sh
```

如果确实需要暂时保留 `-amd64` / `-arm64` tag，才显式覆盖：

```bash
VERSION=v0.6.3 PUSH_LATEST=1 KEEP_ARCH_TAGS=1 ./scripts/publish-dockerhub-apple.sh
```

底层资源控制由 [scripts/with-apple-builder.sh](../scripts/with-apple-builder.sh) 负责：

- 需要时自动启动 builder
- 命令结束后自动停止 builder
- 平时不发布时，不需要手动保持 `container` 的 8G builder 运行

只有在需要排查脚本内部行为时，才需要直接看这一层。正常发布不需要用户手动调用它。

例如把多架构构建和推送包在一次 builder 会话里：

```bash
./scripts/with-apple-builder.sh bash -lc '
  container build \
    --platform linux/amd64 \
    -f Dockerfile_buildx \
    --build-arg VERSION=v0.6.3 \
    --build-arg BUILD_DATE="$(date -u +%Y%m%d)" \
    --build-arg VCS_REF="$(git rev-parse --short=7 HEAD)" \
    -t jasonxtt/mosdns-t:tmp-v0.6.3-amd64 .

  container build \
    --platform linux/arm64 \
    -f Dockerfile_buildx \
    --build-arg VERSION=v0.6.3 \
    --build-arg BUILD_DATE="$(date -u +%Y%m%d)" \
    --build-arg VCS_REF="$(git rev-parse --short=7 HEAD)" \
    -t jasonxtt/mosdns-t:tmp-v0.6.3-arm64 .

  container image push jasonxtt/mosdns-t:tmp-v0.6.3-amd64
  container image push jasonxtt/mosdns-t:tmp-v0.6.3-arm64
'
```

然后再单独合成并检查多架构 manifest：

```bash
docker buildx imagetools create \
  -t jasonxtt/mosdns-t:v0.6.3 \
  jasonxtt/mosdns-t:tmp-v0.6.3-amd64 \
  jasonxtt/mosdns-t:tmp-v0.6.3-arm64

docker buildx imagetools inspect jasonxtt/mosdns-t:v0.6.3
```

如需调整 builder 资源，可临时覆盖：

```bash
APPLE_CONTAINER_BUILDER_CPUS=4 \
APPLE_CONTAINER_BUILDER_MEMORY=6G \
./scripts/with-apple-builder.sh <你的命令>
```

## 2. 准备运行目录

### 新部署

新镜像默认已内置以下环境变量：

```text
MOSDNS_CONTAINER_MODE=1
MOSDNS_CONTAINER_NETWORK_MODE=bridge
MOSDNS_AUTO_INIT=1
MOSDNS_CONFIG_INIT_URL=https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip
```

因此新部署时只需要准备一个空目录并挂载到 `/cus/mosdns`。容器首次启动如果发现：

- `/cus/mosdns/config_custom.yaml` 不存在
- `/cus/mosdns` 是空目录

就会自动下载并解压默认 `config_all.zip` 到该目录。

镜像内置自动初始化默认会按下面顺序回退：

- `https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip`
- `https://cdn.jsdelivr.net/gh/jasonxtt/file@main/mosdns/config/config_all.zip`
- `https://ghproxy.net/https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip`

例如：

```bash
mkdir -p ./mosdns-data
docker compose up -d
```

注意：

- 自动初始化只会在“空目录且缺少主配置”时触发
- 如果目录里已经有文件但没有 `config_custom.yaml`，容器会直接报错退出，不会擅自覆盖
- 如果部署环境无法访问 GitHub，仍可手动把 `config_all.zip` 解压到宿主机目录后再启动

### 旧部署迁移

如果宿主机上已经有现成的 `/cus/mosdns`，直接 bind mount 到容器即可，不需要改目录结构，也不会触发自动初始化。

## 3. 标准 bridge 模式

主示例见仓库根目录 [docker-compose.yml.example](../docker-compose.yml.example)。

关键点：

- 发布 `53/tcp`、`53/udp`、`9099/tcp`
- `restart: unless-stopped`
- 宿主机目录挂载到 `/cus/mosdns`
- bridge 模式不需要额外环境变量，镜像内默认就是这个模式

bridge 模式适合这些场景：

- 你的 `/cus/mosdns` 配置已经把上游改成容器内可达的地址
- 伴生服务本身也容器化了，并且你会把配置改成容器服务名
- 或者你明确使用 `host.docker.internal` / 宿主机 IP，而不是 `127.0.0.1`

启动方式：

```bash
cp docker-compose.yml.example docker-compose.yml
docker compose up -d
```

bridge 模式下的端口行为：

- WebUI 不支持在页面里直接修改监听端口
- 专属分流组仍可设置自定义监听端口
- 但这类端口只会先在容器内监听
- 如需让宿主机或局域网客户端访问，还需要手动给该端口补上 `tcp` / `udp` 映射

## 3.1 发布到 Docker Hub 后的标准 Compose

如果镜像已经发布到 Docker Hub，可直接使用 [docker-compose.image.yml.example](../docker-compose.image.yml.example)。

关键点不变：

- `53:53/tcp`
- `53:53/udp`
- `9099:9099/tcp`
- `./mosdns-data:/cus/mosdns`

示例默认使用 `jasonxtt/mosdns-t:latest`；需要固定版本时改成对应的 `vX.Y.Z` 标签。

## 4. Linux host 网络模式

补充示例见 [docker-compose.host.yml.example](../docker-compose.host.yml.example)。

这个模式只适合确实需要 host 网络的 Linux 环境。注意：

- 不再配置 `ports`
- 容器会直接占用宿主机的监听端口
- 更容易与宿主机已有 DNS / Web 服务冲突
- 如果你当前配置包里大量依赖同机 `127.0.0.1` 上的伴生服务，这个模式通常更省改动
- 需显式设置 `MOSDNS_CONTAINER_NETWORK_MODE=host`

## 5. WebUI 与更新行为

容器版默认设置：

```text
MOSDNS_CONTAINER_MODE=1
MOSDNS_CONTAINER_NETWORK_MODE=bridge
MOSDNS_AUTO_INIT=1
MOSDNS_CONFIG_INIT_URL=https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip
```

容器模式下：

- WebUI 仍可检查新版本
- WebUI 不允许直接下载并覆盖容器内二进制
- bridge 模式下，WebUI 不允许直接修改监听端口
- host 模式下，WebUI 可以直接修改监听端口
- WebUI 手动配置包导出和远程覆盖不可用；规则、上游等日常配置仍可在 WebUI 中保存
- 启动时必需的配置 schema 迁移保持原有机制，用户数据仍存放在挂载卷中
- bridge 模式下，专属分流组可设置自定义监听端口，但保存后仍需同步补齐容器端口映射
- 新部署时，空目录会自动初始化默认配置包；已有配置目录不会被覆盖

这不代表所有默认业务流都天然可用。

如果你的运行配置依赖外部伴生服务，仍需要先保证：

- companion 本身可达
- `/cus/mosdns` 中的上游目标地址从容器视角也可达

如果需要升级容器版：

1. 构建或拉取新镜像
2. 重建容器
3. 继续复用原来的 `/cus/mosdns` 挂载目录

## 6. 日志说明

当前容器版不会自动重写外部 YAML 日志配置。

如果希望更符合容器习惯，可以在外部 `config_custom.yaml` / `sub_config/*.yaml` 中把日志目标改为 stdout/stderr，然后重新启动容器。

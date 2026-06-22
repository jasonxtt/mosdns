# mosdns Docker 部署

本文档说明如何把当前 fork 以标准容器方式部署。容器版保留现有运行目录和配置包结构：

- 容器内运行目录固定为 `/cus/mosdns`
- 配置、运行时状态、备份、生成列表都继续写入 `/cus/mosdns`
- 升级方式改为更新镜像并重建容器，不再通过 WebUI 覆盖容器内二进制

重要说明：

- 当前维护中的默认 `config_all.zip` 不是“单容器自包含”模板。
- 常见部署会依赖同机的 `sing-box` / `mihomo` / fakeip DNS 之类的伴生服务。
- 如果配置文件里仍然写着 `127.0.0.1:1053`、`127.0.0.1:6666` 这类地址，bridge 模式容器内会把它们解释成 `mosdns` 容器自己，而不是宿主机或其他容器。
- 这会导致 WebUI 能打开，但真实 DNS 查询返回 `SERVFAIL`。

## 1. 构建镜像

在仓库根目录执行：

```bash
docker build \
  --build-arg VERSION=v0.5.0 \
  --build-arg BUILD_DATE="$(date -u +%Y%m%d)" \
  --build-arg VCS_REF="$(git rev-parse --short=7 HEAD)" \
  -t mosdns:docker .
```

如需多架构构建，可使用：

```bash
docker buildx build \
  --platform linux/amd64,linux/arm64 \
  --build-arg VERSION=v0.5.0 \
  --build-arg BUILD_DATE="$(date -u +%Y%m%d)" \
  --build-arg VCS_REF="$(git rev-parse --short=7 HEAD)" \
  -f Dockerfile_buildx \
  -t mosdns:docker .
```

如果不传这些参数，镜像里的版本信息会回退成 `dev-日期-nogithash`，不影响运行，但会影响 WebUI 中的版本展示。

## 2. 准备运行目录

### 新部署

先把完整配置包解压到宿主机目录，再挂载到容器内的 `/cus/mosdns`：

```bash
mkdir -p ./mosdns-data
curl -L https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip -o /tmp/config_all.zip
unzip /tmp/config_all.zip -d ./mosdns-data
```

解压后应保证主配置文件位于：

```text
./mosdns-data/config_custom.yaml
```

### 旧部署迁移

如果宿主机上已经有现成的 `/cus/mosdns`，直接 bind mount 到容器即可，不需要改目录结构。

## 3. 标准 bridge 模式

主示例见仓库根目录 [docker-compose.yml.example](../docker-compose.yml.example)。

关键点：

- 发布 `53/tcp`、`53/udp`、`9099/tcp`
- `restart: unless-stopped`
- 宿主机目录挂载到 `/cus/mosdns`
- 默认 `MOSDNS_CONTAINER_NETWORK_MODE=bridge`

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

只需要把示例中的镜像名替换成你的实际仓库地址和标签。

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
```

容器模式下：

- WebUI 仍可检查新版本
- WebUI 不允许直接下载并覆盖容器内二进制
- bridge 模式下，WebUI 不允许直接修改监听端口
- host 模式下，WebUI 可以直接修改监听端口
- 配置在线更新仍可用，因为它更新的是挂载卷中的 `/cus/mosdns`
- bridge 模式下，专属分流组可设置自定义监听端口，但保存后仍需同步补齐容器端口映射

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

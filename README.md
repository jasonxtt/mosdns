# MosDNS-T

MosDNS-T 是基于 [`yyysuo/mosdns`](https://github.com/yyysuo/mosdns) 持续维护的 DNS 分流增强版，面向家庭网络、旁路由和透明代理环境，重构了 WebUI，并增加专属分流组、规则维护、查询诊断及多平台部署支持。

## 项目优势

- 内置相对完善的国内外域名分流策略，一次判定后自动生成直连域名、代理域名列表，后续分流优先采信，越用越快。
- 可通过 WebUI 轻松维护白名单、灰名单、DDNS 域名、DNS 重定向，并可设置缓存开关、IPv4 优先、指定客户端直连或代理。
- 支持 FakeIP 与 Redir-Host 两种 DNS 分流模式，可适配不同透明代理方案。
- 支持专属分流组、专属上游和独立缓存，可将指定域名单独交给特定 DNS 线路处理。

## WebUI 预览

点击缩略图可查看原图。

<p>
  <a href="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-16.png"><img src="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-16.png" width="31%" alt="MosDNS-T WebUI 预览 1"></a>
  <a href="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-2.png"><img src="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-2.png" width="31%" alt="MosDNS-T WebUI 预览 2"></a>
  <a href="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-3.png"><img src="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-3.png" width="31%" alt="MosDNS-T WebUI 预览 3"></a>
</p>
<p>
  <a href="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-4.png"><img src="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-4.png" width="31%" alt="MosDNS-T WebUI 预览 4"></a>
  <a href="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-17.png"><img src="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-17.png" width="31%" alt="MosDNS-T WebUI 预览 5"></a>
  <a href="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-18.png"><img src="https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-18.png" width="31%" alt="MosDNS-T WebUI 预览 6"></a>
</p>

## 多版本支持

当前提供 **Linux 原生版、Docker 容器版、OpenWrt / ImmortalWrt 版**。三种版本共享主要核心分流能力和 WebUI，主要区别在安装方式、服务管理与网络环境。

| 版本 | 适合场景 | 安装与管理方式 | 主要说明 |
| --- | --- | --- | --- |
| **Linux 原生版** | Debian / Ubuntu 虚拟机、旁路由、专用 DNS 主机 | 安装脚本 + systemd | 功能最完整，适合与 sing-box / mihomo 配合使用 |
| **Docker 容器版** | Unraid、NAS、Docker 主机 | Docker Hub + Compose | 支持 `amd64`、`arm64`，配置持久化，更新镜像即可升级 |
| **OpenWrt 版** | OpenWrt / ImmortalWrt 主路由或旁路由 | 软件源 + LuCI | 自动安装核心与 LuCI，由 procd 管理，可接入 dnsmasq |

## Linux 原生版安装
**步骤 1：** 新建 Debian 或 Ubuntu 虚拟机，运行安装脚本

```bash
wget --quiet --show-progress -O /mnt/main_install.sh https://raw.githubusercontent.com/jasonxtt/LinuxScripts/main/AIO/Scripts/main_install.sh && chmod +x /mnt/main_install.sh && /mnt/main_install.sh
```

**步骤 2：** 输入 `5` ，再输入 `1` ，安装 mosdns

![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-14.png)

**步骤 3：** 按提示输入以下信息：

1. sing-box / mihomo 提供的 SOCKS 代理 `IP:端口`（例如 `127.0.0.1:8888`）
2. 选择默认使用的 DNS 分流模式，安装完成后也可在 WebUI 的「系统设置」中切换
3. 输入 sing-box / mihomo 监听的 DNS 端口，用于获取 FakeIP（例如 `127.0.0.1:6666`）

![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-15.png)

**步骤 4：** 安装完成后，WebUI 地址为 `http://IP:9099`，例如 `http://10.0.0.3:9099`

## Docker 容器版部署

Docker Hub 镜像：[`jasonxtt/mosdns-t`](https://hub.docker.com/r/jasonxtt/mosdns-t)

容器版继续使用 `/cus/mosdns` 作为运行目录。将该目录挂载到宿主机后，配置、规则、缓存、生成列表和 WebUI 状态都可持久保留。

主要特性：

- 支持 `linux/amd64` 与 `linux/arm64`
- 支持 bridge 网络和 Linux host 网络
- 空配置目录首次启动时自动初始化默认配置
- WebUI 配置包在线更新仍可使用
- 程序升级通过拉取新镜像并重建容器完成，原有挂载配置不会丢失

bridge 模式下，配置中的 `127.0.0.1` 指向容器自身，不是宿主机上的 sing-box / mihomo。伴生服务地址应填写容器可访问的服务名、宿主机地址或局域网地址。

详细部署方式、Compose 示例和网络模式说明见：[Docker 容器化部署文档](https://github.com/jasonxtt/mosdns/blob/docker/docs/docker_deployment_zh.md)。

## OpenWrt / ImmortalWrt 部署

在路由器 SSH 终端中使用 root 用户执行：

```sh
wget -qO- https://jasonxtt.github.io/mosdns/install.sh | sh
```

如果原地址无法访问，可改用以下加速地址：

```sh
wget -qO- https://cdn.jsdelivr.net/gh/jasonxtt/mosdns@openwrt/openwrt/repository/install.sh | sh
```
或
```sh
wget -qO- https://ghproxy.net/https://raw.githubusercontent.com/jasonxtt/mosdns/openwrt/openwrt/repository/install.sh | sh
```

安装脚本会自动识别 `apk` 或 `opkg`，添加 MosDNS-T 签名软件源，并安装或升级：

- `mosdns-t`
- `luci-app-mosdns-t`

安装完成后，LuCI 入口位于：**服务 → MosDNS-T**。

当前软件源支持：

- OpenWrt / ImmortalWrt 25.12：APK
- OpenWrt 24.10：IPK
- `x86_64`
- `aarch64_cortex-a53`
- `aarch64_generic`

OpenWrt 版由 procd 管理服务，并针对路由器环境提供 LuCI、dnsmasq 接入、配置持久化和软件包升级流程。源码与构建说明位于 [`openwrt` 分支](https://github.com/jasonxtt/mosdns/tree/openwrt/openwrt)。

## Docker 容器化部署

当前仓库已经提供标准容器化支持，容器版继续使用 `/cus/mosdns` 作为运行目录，并保留现有配置包与 WebUI 工作流。

- Docker 部署文档：[Docker 容器化部署](docs/docker_deployment_zh.md)
- bridge 网络 Compose 示例：[docker-compose.yml.example](docker-compose.yml.example)
- Docker Hub 镜像 Compose 示例：[docker-compose.image.yml.example](docker-compose.image.yml.example)
- Linux host 网络 Compose 示例：[docker-compose.host.yml.example](docker-compose.host.yml.example)

容器版的重要约束：

- 新部署时，空的 `/cus/mosdns` 挂载目录会在首次启动时自动初始化默认配置包
- WebUI 仍可检查新版本
- 容器内不支持直接更新二进制，升级方式改为更新镜像并重建容器
- 容器内不支持直接修改 WebUI 端口，端口应通过 Compose 或 `docker run` 映射管理

## 专属分流组简介及设置

“专属分流组”可以理解为一组独立的域名分流槽位。命中这个组的域名，会优先走它绑定的专属上游、专属缓存和对应的规则入口，适合把某一类域名单独交给特定 DNS 线路处理。

在当前 WebUI 中，常见设置流程是：

- 进入 `上游设置`
- 点击 `新增专属分流组`
- 为分流组命名，例如 `腾讯上游`
- 点击 `添加上游DNS`，所属组选择刚才添加的 `腾讯上游`，再填写协议、服务器地址等参数
- 在 `规则管理` 中配置要命中这个组的域名：
  - `本地规则` 可直接手工录入域名
  - `订阅规则` 可添加在线规则集，并把类型选择为对应的专属分流组
- 命中该专属分流组的域名，会优先走它绑定的上游组及对应缓存

这套机制的核心作用，是把某一批域名独立交给指定线路处理，而不混在默认国内 / 国外出口逻辑里。

## 配置包

这个 fork 维护中的配置包放在：

- [`mosdns/config/config_all.zip`](https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_all.zip)
- [`mosdns/config/config_up.zip`](https://raw.githubusercontent.com/jasonxtt/file/main/mosdns/config/config_up.zip)

其中：

- `config_all.zip` 用于新部署或整套模板替换
- `config_up.zip` 用于现有部署的增量配置更新

完整配置包解压后的运行目录应为：

- `/cus/mosdns`

## 当前相对上游的改动

- 新增改动：
  - 专属分流组与专属上游联动
  - 在线规则、本地规则、日志排障等日常维护能力
  - 使用 Vue 对 UI 进行了重构，并持续替换原有前端工作流
- 当前 UI 路径关系如下：
  - 默认入口 `/` 为当前维护中的 Vue UI，后续功能演进以这套界面为主
  - main分支`/log` 保留原版 UI，主要用于兼容、对照和过渡期使用
- 重构 UI 内容包括：
  - 将概览、查询日志、规则管理、数据管理、上游设置、系统设置统一到同一套组件结构下
  - 统一弹窗编辑、详情查看、刷新行为和模块层级，减少旧 UI 中大量分散式脚本交互
  - 在保持原有功能覆盖面的前提下，让后续新增功能更容易继续扩展到 UI
- 同时，这个 fork 也明确收缩了与当前定位无关的能力面，当前不跟 `nft` / `eBPF` 这条线。



详细说明见：

- [相对上游的改动说明](docs/fork_diff_summary_zh.md)

## 发布状态

本项目作为持续维护的 WebUI 与 DNS 分流增强分支发布。当前版本请以仓库的 [Tags](https://github.com/jasonxtt/mosdns/tags) 和 [更新日志](CHANGELOG.md) 为准。

## 文档

- [项目简介草案](docs/github_project_intro_zh.md)
- [相对上游的改动说明](docs/fork_diff_summary_zh.md)
- [GitHub 发布前清单](docs/github_release_checklist_zh.md)

## 开源协议

本项目采用 **GNU General Public License v3.0（GPL-3.0）**。使用、修改和再发布本项目时，请遵守仓库中的 [LICENSE](LICENSE)，并保留原作者及相关版权声明。

## 致谢

本项目基于：

- [`yyysuo/mosdns`](https://github.com/yyysuo/mosdns)

感谢原项目作者及所有贡献者。

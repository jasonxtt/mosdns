# mosdns

这是一个基于 `yyysuo/mosdns` 的魔改版 mosdns，重构WebUI、新增专属上游、删除`nft` / `eBPF`支持及其他细节调整

## 适用场景

- 以DNS分流为方案的家庭网络部署
- 建议搭配sing-box / mosdns FakeIP模式
- 内置相对完善的国内外域名分流策略，一次判定后自生成直连域名、代理域名列表，后续分流优先采信，越用越快
- 可通过 WebUI 轻松维护白名单、灰名单、DDNS域名、DNS重定向，设定开关缓存、IPv4优先、指定客户端直连或代理

## WebUI预览
![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-1.png)
![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-2.png)
![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-3.png)
![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-4.png)
![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-5.png)
![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosui-6.png)

## 安装方法
**步骤 1：** 新建 Debian 或 Ubuntu 虚拟机，运行安装脚本

```bash
wget --quiet --show-progress -O /mnt/main_install.sh https://raw.githubusercontent.com/jasonxtt/LinuxScripts/main/AIO/Scripts/main_install.sh && chmod +x /mnt/main_install.sh && /mnt/main_install.sh
```


**步骤 2：** 输入 `6` ，再输入 `1` ，安装mosdns 

![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-13.png)

**步骤 3：** 按提示输入以下信息：
1. sing-box/mihomo 提供的socks代理  `IP:端口`（例如 10.0.0.2:7890）
2. 输入sing-box/mihomo监听的DNS端口，用于获取fakeip (例如 10.0.0.2:6666)

![](https://raw.githubusercontent.com/jasonxtt/images/main/images/mosdns-7.png)

**步骤 4：** 安装完成后，UI 地址为 `IP:9099`, 例如`http://10.0.0.3:9099`

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

当前正式发布版本为：

- `v0.4.14`

这个 fork 现在已经按持续维护的 WebUI 增强分支在发布，不再是早期预览版定位。

## 文档

- [项目简介草案](docs/github_project_intro_zh.md)
- [相对上游的改动说明](docs/fork_diff_summary_zh.md)
- [GitHub 发布前清单](docs/github_release_checklist_zh.md)

## 致谢

本项目基于：

- `yyysuo/mosdns`

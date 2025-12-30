# mosdns
fakeip分流大法总教程：https://drive.google.com/drive/u/1/folders/1ldD2XqIrREPgr_CKMSgvYomXgwknpApi  
原版mosdns知识库: https://irine-sistiana.gitbook.io/mosdns-wiki/  
下载: https://github.com/yyysuo/mosdns/releases  
魔改版本配置语法基本无差异，仅添加了一些插件，具体参见fakeip分流大法总教程中mosdns配置 下载预编译文件、更新日志。

### 在mosdns1225all配置的基础上保留域名总表（top_domians）/上游DNS配置（config_overrides.json）升级（不保留系统-高级设置-功能开关状态）

1. 备份整个mosdns文件夹
2. web上更新2进制
3. 在系统-配置管理部分
MosDNS 本地工作目录填入自己的mosdns配置所在目录，比如/cus/mosdns
远程配置下载 URL (ZIP)填入：https://raw.githubusercontent.com/yyysuo/firetv/refs/heads/master/mosdnsconfigupdate/mosdns20251225allup.zip
然后点击：应用远程配置，mosdns自动重启。

### 保存个性化配置

1. gen/top_domains.txt
2. rule文件夹
3. config_overrides.json
离线全新安装/在线初始化重置后，可以将上述文件（不要删除对应位置的文件夹，需要按文件覆盖）覆盖至对应位置，即可恢复个性化配置。

### 在线初始化重置

1. 备份整个mosdns文件夹
2. web上更新2进制
3. 在系统-配置管理部分  
   MosDNS 本地工作目录填入自己的mosdns配置所在目录，比如 `/cus/mosdns`  
   远程配置下载 URL (ZIP)填入：`https://raw.githubusercontent.com/yyysuo/firetv/refs/heads/master/mosdnsconfigupdate/mosdns1225all.zip`  
   然后点击：应用远程配置，mosdns自动重启。
4. 在web ui上按自己的需求调整功能开关部分。
5. 在web ui上按mosdns配置说明填写SOCKS5/ECS IP部分和上游DNS设置/其它设置部分。

### 离线全新安装

1. 这里下载对应自己架构的2进制，比如放入 `/cus/bin/`  
   https://github.com/yyysuo/mosdns/releases
2. 这里下载全量初始化配置，比如解析至 `/cus/mosdns/` 下  
   https://raw.githubusercontent.com/yyysuo/firetv/refs/heads/master/mosdnsconfigupdate/mosdns1225all.zip
3. 运行命令
   ```bash
   /cus/bin/mosdns start -c /cus/mosdns/config_custom.yaml -d /cus/mosdns
   ```
4. 在web ui上按自己的需求调整功能开关部分。
5. 在web ui上按mosdns配置说明填写SOCKS5/ECS IP部分和上游DNS设置/其它设置部分。

### mosdns配置说明：

远程下载的mosdns配置为通用模板，需要在此处设置上游dns等相关信息，此处信息存储于运行目录的 `config_overrides.json` 中。  
原值 (查找)为配置模板中预设的初始值（不要修改！），新值 (替换)将替换初始值在mosdns启动时加载，不会回写至配置模板，状态中的数字代表配置中有多少处原值被替换。

**下面是模板一些必要替换的原值的说明**

- `udp://127.0.0.1:7874`，替换为sing-box dns，用于对国外域名返回fakeip。
- `114.114.114.114`，替换为运营商dns，用于返回最优DNS结果。
- `127.0.0.1:7777`，替换为 `:7777`，取消仅监听127.0.0.1限制
- `127.0.0.1:8888`，替换为 `:8888`，取消仅监听127.0.0.1限制

**下面是模板一些可选替换的原值的说明**

- `udp://127.0.0.1:1053`，替换为mihomo dns，不使用CNToMihomo可不配置。
- `123.123.110.123`，填写ipw.cn显示的ipv4地址或者本城市任意ipv4地址，传递给阿里私有doh。
- `888888`，替换为阿里私有doh Account ID。
- `888888_88888`，替换为阿里私有doh AccessKey ID。
- `999999999`，替换为阿里私有doh AccessKey Secret。
- nft功能说明：https://raw.githubusercontent.com/yyysuo/firetv/refs/heads/master/mosdnsconfigupdate/mosdns%20nft%E8%A7%84%E5%88%99%E6%B7%BB%E5%8A%A0%E5%8A%9F%E8%83%BD%E4%BD%BF%E7%94%A8%E8%AF%B4%E6%98%8E

<img width="1590" height="885" alt="image" src="https://github.com/user-attachments/assets/62e34ea4-35ff-45cf-9d4b-ad4d05d9e9a2" />


### 下面是如何查找新值、ecs ip、socks5的补充说明

#### mosdns ui 系统-高级设置-上游DNS设置/其它设置部分：

- **添加第1条数据**：原值固定写 `114.114.114.114`，新值为你的运营商dns。
- **添加第2条数据**：原值固定写 `udp://127.0.0.1:7874`。
  如果 sing-box 和 mosdns 在一个虚拟机上，新值去你 sing-box 的配置文件中去找，找到 `inbounds` 部分类似这样的配置：
  ```json
  "inbounds": [
    {
      "type": "direct",
      "tag": "in-dns",
      "sniff": false,
      "listen": "::",
      "listen_port": 7800
    },
  ```
  那新值就填 `udp://127.0.0.1:7800`，注意你的端口和ip可能不一样。

#### mosdns ui 系统-高级设置-SOCKS5/ECS IP部分：

1. **第1条数据**：打开 https://ipw.cn/ ，如果有ipv4、ipv6的ip，就把ipv6 ip填入ECS IP框中；如果没有，就写ipv4地址。
2. **第2条数据**：打开sing-box配置文件，找到类似如下的配置：
   ```json
       {
         "type": "socks",
         "listen": "0.0.0.0",
         "listen_port": 7900,
         "tcp_multi_path": false,
         "udp_fragment": false,
         "sniff": false,
         "users": []
       },
   ```
   如果 sing-box 和 mosdns 在一个虚拟机上，那么 socks5 代理就填 `127.0.0.1:7900`，注意你的端口和ip可能不一样。

![mosdns解析流程](https://github.com/user-attachments/assets/c4b0c10c-7c99-4dbb-922c-64de1d566f98)

<img width="1559" height="896" alt="image" src="https://github.com/user-attachments/assets/3a5d7f92-ee34-4612-a0c0-f97fbc2f2b59" />

<img width="1545" height="852" alt="image" src="https://github.com/user-attachments/assets/4c8c72ae-636a-42e3-a645-059e4ca89f12" />

<img width="1558" height="902" alt="image" src="https://github.com/user-attachments/assets/e60b8530-e9fb-4792-a711-6d5e03b11f4f" />

<img width="1586" height="896" alt="image" src="https://github.com/user-attachments/assets/9236b9b5-6058-43fa-899a-a669c04372b5" />

<img width="1551" height="899" alt="image" src="https://github.com/user-attachments/assets/80a53a63-957b-4b04-abe5-d0976972da11" />

<img width="1558" height="904" alt="image" src="https://github.com/user-attachments/assets/2318f836-efe8-45aa-9dbd-b6faffaa11fa" />

<img width="1568" height="900" alt="image" src="https://github.com/user-attachments/assets/1ff7afe5-abeb-413d-88e3-98481a28c8a9" />

<img width="1564" height="906" alt="image" src="https://github.com/user-attachments/assets/83e7c8fe-7cbb-474a-8f71-a52fc1f8dba3" />

<img width="1561" height="893" alt="image" src="https://github.com/user-attachments/assets/e91b7a6e-53e3-4be9-ae54-e750843292c6" />

## 版本与下载

- 版本号规范（自 2025-11-06 起）
  - 统一为 `v5-ph-srs-YYYYMMDD-<shortsha>`，例如：`v5-ph-srs-20251104-4f2f1c9`。
  - 可执行文件内的 `./mosdns version` 输出与发布标签一致。

- 获取最新稳定版
  - 通过 GitHub `releases/latest` 固定入口下载：
    - `.../releases/latest/download/mosdns-linux-amd64.zip`
    - `.../releases/latest/download/mosdns-linux-amd64-v3.zip`
    - `.../releases/latest/download/mosdns-linux-arm64.zip`
    - `.../releases/latest/download/mosdns-darwin-amd64.zip`
    - `.../releases/latest/download/mosdns-darwin-arm64.zip`
    - `.../releases/latest/download/mosdns-windows-amd64.zip`
    - `.../releases/latest/download/mosdns-windows-amd64-v3.zip`
    - `.../releases/latest/download/mosdns-windows-arm64.zip`

- 获取指定版本
  - 将 `<TAG>` 替换为发布页中的完整标签，例如 `v5-ph-srs-20251104-4f2f1c9`：
    - `.../releases/download/<TAG>/mosdns-<os-arch>.zip`

## 内置在线更新

- 前端：在“系统 → 版本与更新”可手动强制检查、立即更新或强制更新；支持自动检查间隔设置。
- API：
  - `GET /api/v1/update/status` 查询状态
  - `POST /api/v1/update/check` 强制刷新状态
  - `POST /api/v1/update/apply` 执行更新（JSON: `{ "force": true|false }`）
- 行为：
  - 下载匹配当前平台的压缩包，解压并原地覆盖二进制（Unix 保留可执行位；Windows 写入 `.new` 并提示手动替换）。
  - 安装成功后自动调用 `POST http://127.0.0.1:9099/api/v1/system/restart` 自重启（Unix）；Windows 因文件锁定不执行自重启。

## 重要变更：移除固定 tag 回退

- 自 2025-11-06 起，客户端仅使用 `releases/latest` 获取更新信息；不再回退到固定 tag（例如 `v5-ph-srs`）。
- 发布侧无需再向固定 tag 上传资产；请使用版本化标签发布即可。
- 


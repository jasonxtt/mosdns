# mosdns
fakeip分流大法总教程：https://drive.google.com/drive/u/1/folders/1ldD2XqIrREPgr_CKMSgvYomXgwknpApi




功能概述、配置方式、教程等，详见: [wiki](https://irine-sistiana.gitbook.io/mosdns-wiki/)

下载预编译文件、更新日志，详见: [release](https://github.com/IrineSistiana/mosdns/releases)

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

docker 镜像: [docker hub](https://hub.docker.com/r/irinesistiana/mosdns)

![image](https://github.com/user-attachments/assets/302f74c7-3f22-4cb0-aa8b-dc902d98cd11)

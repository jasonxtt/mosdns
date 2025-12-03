# mosdns
fakeip分流大法总教程：https://drive.google.com/drive/u/1/folders/1ldD2XqIrREPgr_CKMSgvYomXgwknpApi
原版mosdns知识库: [wiki](https://irine-sistiana.gitbook.io/mosdns-wiki/)
魔改版本配置语法基本无差异，仅添加了一些插件，具体参见fakeip分流大法总教程中mosdns配置
下载预编译文件、更新日志，详见: [release](https://github.com/yyysuo/mosdns/releases)

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



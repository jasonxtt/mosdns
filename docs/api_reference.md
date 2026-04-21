# mosdns API 参考文档

> 本文档为 AI Agent 与 mosdns v0.3.0 交互的完整接口规范。

## 基础信息

| 项目 | 值 |
|------|-----|
| API Base URL | `http://{host}:9099` |
| Content-Type | `application/json` |
| 编码 | UTF-8 |

---

## 1. 系统管理 (System)

### 1.1 服务重启
```
POST /api/v1/system/restart
```

**请求体 (可选)**
```json
{
  "delay_ms": 300
}
```
- `delay_ms`: 延迟毫秒数，默认 300ms

**响应 (200)**
```json
{
  "status": "scheduled",
  "delay_ms": 300
}
```

**说明**
- 立即返回，重启异步执行
- 重启前会优雅关闭 cache 和 domain_output 插件（保存数据）
- Windows 不支持此接口

---

## 2. 上游管理 (Upstream)

### 2.1 获取所有上游配置
```
GET /api/v1/upstream/config
```

**响应 (200)**
```json
{
  "aliapi_upstream_1": [
    {
      "tag": "upstream_1",
      "enabled": true,
      "protocol": "aliapi",
      "account_id": "xxx",
      "access_key_id": "yyy",
      "access_key_secret": "***",
      "server_addr": "alidns.cn-shanghai.aliyuncs.com"
    }
  ],
  "special_upstream_50": [...]
}
```

### 2.2 获取特定上游的运行时状态
```
GET /api/v1/upstream/runtime/{tag}
```

**响应 (200)**
```json
{
  "tag": "aliapi_upstream_1",
  "override_config": [...],
  "runtime_targets": ["8.8.8.8:53", "1.1.1.1:53"]
}
```

### 2.3 获取已发现的 AliAPI 插件 Tag
```
GET /api/v1/upstream/tags
```

**响应 (200)**
```json
["aliapi_upstream_1", "special_upstream_50"]
```

### 2.4 保存上游配置 (热重载)
```
POST /api/v1/upstream/config
```

**请求体**
```json
{
  "plugin_tag": "aliapi_upstream_1",
  "upstreams": [
    {
      "tag": "upstream_1",
      "enabled": true,
      "protocol": "aliapi",
      "account_id": "xxx",
      "access_key_id": "yyy",
      "access_key_secret": "***",
      "server_addr": "alidns.cn-shanghai.aliyuncs.com",
      "ecs_client_ip": "11.22.33.44",
      "ecs_client_mask": 24
    },
    {
      "tag": "upstream_2",
      "enabled": true,
      "protocol": "udp",
      "addr": "8.8.8.8:53",
      "enable_pipeline": true,
      "so_mark": 255
    }
  ]
}
```

**配置字段说明**

| 字段 | 类型 | 说明 |
|------|------|------|
| `tag` | string | 上游名称（必填） |
| `enabled` | bool | 是否启用 |
| `protocol` | string | 类型：aliapi, udp, tcp, dot, doh, socks5 |
| `addr` | string | DNS 地址（aliapi 以外必填） |
| `dial_addr` | string | 拨号地址 |
| `idle_timeout` | int | 空闲超时（秒） |
| `upstream_query_timeout` | int | 查询超时（秒） |
| `enable_pipeline` | bool | 并行查询 |
| `enable_http3` | bool | 启用 HTTP/3 (DoH) |
| `insecure_skip_verify` | bool | 跳过证书验证 |
| `socks5` | string | SOCKS5 代理地址 |
| `so_mark` | int | SO_MARK 值 |
| `bind_to_device` | string | 绑定网卡 |
| `bootstrap` | string | Bootstrap DNS |
| `bootstrap_version` | int | Bootstrap 版本 |
| `account_id` | string | 阿里云账户 ID (AliAPI) |
| `access_key_id` | string | AccessKey ID (AliAPI) |
| `access_key_secret` | string | AccessKey Secret (AliAPI) |
| `server_addr` | string | AliDNS 服务地址 (AliAPI) |
| `ecs_client_ip` | string | ECS 客户端 IP (AliAPI) |
| `ecs_client_mask` | uint8 | ECS 掩码 (AliAPI) |

**响应 (200)**
```json
{"message": "Upstream configuration saved."}
```

**响应 (400/500)**
```json
{"error": "具体错误信息"}
```

**说明**
- AliAPI 类型需要 `account_id`, `access_key_id`, `access_key_secret` 三个字段
- 其他类型需要 `addr` 字段
- 保存后自动热重载对应插件
- 会刷新相关缓存

---

## 3. 全局覆盖 (Global Overrides)

### 3.1 获取全局设置
```
GET /api/v1/overrides
```

**响应 (200)**
```json
{
  "socks5": "127.0.0.1:1080",
  "ecs": "11.22.33.44/24",
  "replacements": [
    {
      "original": "*.cn",
      "new": "*.baidu.com",
      "comment": "替换测试",
      "result": "Success (42)"
    }
  ]
}
```

### 3.2 保存全局设置
```
POST /api/v1/overrides
```

**请求体**
```json
{
  "socks5": "127.0.0.1:1080",
  "ecs": "11.22.33.44/24",
  "replacements": [
    {
      "original": "*.cn",
      "new": "*.baidu.com",
      "comment": "替换测试"
    }
  ]
}
```

**响应 (200)**
```json
{"message": "Global overrides saved. Please restart mosdns to apply changes."}
```

---

## 4. 专用路由组 (Special Groups)

### 4.1 获取所有专用组
```
GET /api/v1/special-groups
```

**响应 (200)**
```json
[
  {
    "slot": 50,
    "name": "香港节点",
    "key": "special_50",
    "upstream_plugin_tag": "special_upstream_50",
    "diversion_plugin_tag": "special_route_50",
    "manual_plugin_tag": "special_manual_50",
    "local_config": "srs/special_50.json",
    "manual_rule_path": "rule/special_50.txt"
  }
]
```

### 4.2 创建/更新专用组
```
POST /api/v1/special-groups
```

**请求体**
```json
{
  "slot": 50,
  "name": "香港节点"
}
```
- `slot`: 50-59，不传则自动分配最小可用
- `name`: 组名称（必填）

**响应 (200)**
```json
{
  "slot": 50,
  "name": "香港节点",
  "key": "special_50",
  ...
}
```

### 4.3 删除专用组
```
DELETE /api/v1/special-groups/{slot}
```

**响应 (204)** 无内容

**说明**
- 删除会同时清理关联的上游配置和规则文件

---

## 5. 审计日志 (Audit Log)

### 5.1 获取审计日志
```
GET /api/v1/audit/logs
```

**响应 (200)**
```json
[
  {
    "time": "2024-04-13T12:00:00+08:00",
    "src_ip": "192.168.1.100",
    "query": "google.com",
    "qtype": "A",
    "result": "IP_LIST",
    "answer": ["142.250.xxx.xxx"],
    "upstream": "aliapi_upstream_1",
    "rule_tag": "bypass",
    "duration_ms": 15
  }
]
```

### 5.2 启动/停止审计
```
POST /api/v1/audit/start
POST /api/v1/audit/stop
```

**响应** `OK`

### 5.3 获取审计状态
```
GET /api/v1/audit/status
```

**响应 (200)**
```json
{
  "capturing": true
}
```

### 5.4 清理日志
```
POST /api/v1/audit/clear
```

**响应** `OK`

### 5.5 容量管理
```
GET /api/v1/audit/capacity
POST /api/v1/audit/capacity
```

**POST 请求体**
```json
{
  "capacity": 10000
}
```

---

## 6. 进程日志 (Capture)

### 6.1 启动日志捕获
```
POST /api/v1/capture/start
```

**请求体 (可选)**
```json
{
  "duration_seconds": 120
}
```
- 默认 120 秒，最大 600 秒

**响应** `OK`

### 6.2 获取捕获的日志
```
GET /api/v1/capture/logs
```

**响应 (200)**
```json
[
  {
    "time": "2024-04-13T12:00:00+08:00",
    "level": "INFO",
    "message": "plugin loaded",
    "fields": {}
  }
]
```

---

## 7. 规则更新 (Update Manager)

### 7.1 获取更新状态
```
GET /api/v1/update/status
```

**响应 (200)**
```json
{
  "current_version": "0.1.13",
  "latest_version": "0.1.14",
  "release_url": "https://github.com/...",
  "architecture": "linux/amd64",
  "asset_name": "mosdns-linux-amd64",
  "download_url": "https://github.com/...",
  "asset_signature": "...",
  "current_signature": "...",
  "published_at": "2024-04-10T00:00:00Z",
  "checked_at": "2024-04-13T12:00:00Z",
  "cache_expires_at": "2024-04-14T12:00:00Z",
  "update_available": true,
  "cached": true,
  "message": "",
  "pending_restart": false,
  "amd64_v3_capable": true,
  "current_is_v3": false
}
```

### 7.2 强制检查更新
```
POST /api/v1/update/check
```

**响应** 同上

### 7.3 执行更新
```
POST /api/v1/update/apply
```

**请求体 (可选)**
```json
{
  "force": false,
  "prefer_v3": false
}
```

**响应 (200)**
```json
{
  "status": {...},
  "installed": true,
  "restart_required": true,
  "notes": "Update installed successfully"
}
```

---

## 8. 配置管理 (Config Manager)

### 8.1 导出配置
```
POST /api/v1/config/export
```

**请求体**
```json
{
  "dir": "/cus/mosdns"
}
```

**响应** ZIP 文件下载

**Header**
```
Content-Type: application/zip
Content-Disposition: attachment; filename="mosdns_backup_{timestamp}.zip"
```

### 8.2 在线更新配置
```
POST /api/v1/config/update_from_url
```

**请求体**
```json
{
  "url": "https://raw.githubusercontent.com/.../config.zip",
  "dir": "/cus/mosdns"
}
```

**响应 (200)**
```json
{
  "message": "Update successful. 42 files updated. Restarting...",
  "status": "success"
}
```

**说明**
- 自动备份到 `backup/` 目录
- 支持 SOCKS5 代理下载
- 失败时自动回滚

---

## 9. WebUI 路由

| 路径 | 说明 |
|------|------|
| `/` | 主界面 (dashboard.html) |
| `/legacy` | 旧版界面 (mosdnsp.html) |
| `/log` | 日志页面 |

---

## 10. 监控端点

| 路径 | 说明 |
|------|------|
| `/metrics` | Prometheus 指标 |
| `/debug/pprof/*` | pprof 性能分析 |

---

## 使用示例

### Python

```python
import requests

BASE = "http://127.0.0.1:9099"

# 获取上游配置
r = requests.get(f"{BASE}/api/v1/upstream/config")
print(r.json())

# 保存上游配置
r = requests.post(f"{BASE}/api/v1/upstream/config", json={
    "plugin_tag": "aliapi_upstream_1",
    "upstreams": [
        {
            "tag": "upstream_1",
            "enabled": True,
            "protocol": "aliapi",
            "account_id": "xxx",
            "access_key_id": "yyy",
            "access_key_secret": "***",
            "server_addr": "alidns.cn-shanghai.aliyuncs.com"
        }
    ]
})
print(r.json())

# 创建专用组
r = requests.post(f"{BASE}/api/v1/special-groups", json={
    "name": "香港节点"
})
print(r.json())

# 重启服务
r = requests.post(f"{BASE}/api/v1/system/restart", json={
    "delay_ms": 500
})
print(r.json())
```

### curl

```bash
# 获取配置
curl http://127.0.0.1:9099/api/v1/upstream/config

# 保存配置
curl -X POST http://127.0.0.1:9099/api/v1/upstream/config \
  -H "Content-Type: application/json" \
  -d '{"plugin_tag":"aliapi_upstream_1","upstreams":[{"tag":"up1","enabled":true,"protocol":"udp","addr":"8.8.8.8:53"}]}'

# 创建专用组
curl -X POST http://127.0.0.1:9099/api/v1/special-groups \
  -H "Content-Type: application/json" \
  -d '{"name":"香港节点"}'

# 重启
curl -X POST http://127.0.0.1:9099/api/v1/system/restart \
  -H "Content-Type: application/json" \
  -d '{"delay_ms":500}'
```

---

## 错误码

| HTTP 状态码 | 说明 |
|-------------|------|
| 200 | 成功 |
| 204 | 删除成功（无响应体） |
| 400 | 请求格式错误 |
| 404 | 资源不存在 |
| 409 | 冲突（如名称已存在） |
| 500 | 服务器内部错误 |
| 502 | 更新失败 |

---

## 配置文件路径

API 操作涉及的配置文件（相对于配置目录）：

| 文件 | 对应 API |
|------|----------|
| `upstream_overrides.json` | /api/v1/upstream/* |
| `config_overrides.json` | /api/v1/overrides |
| `special_upstream_groups.json` | /api/v1/special-groups |


---

## 11. 插件 API (Plugin APIs)

插件通过 `bp.RegAPI()` 注册自己的子路由，前缀为 `/plugins/{plugin_tag}`。

### 11.1 adguard_rule 插件

**基础路径**: `/plugins/adguard_rule`

#### 获取所有规则
```
GET /plugins/adguard_rule/rules
```

**响应**
```json
[
  {
    "id": "uuid-xxx",
    "name": "AdGuard DNS Filter",
    "url": "https://xxx/filter.txt",
    "enabled": true,
    "auto_update": true,
    "update_interval_hours": 24,
    "rule_count": 50000,
    "last_updated": "2024-04-13T12:00:00+08:00"
  }
]
```

#### 添加规则
```
POST /plugins/adguard_rule/rules
```
```json
{
  "name": "规则名称",
  "url": "https://example.com/rules.txt",
  "enabled": true,
  "auto_update": true,
  "update_interval_hours": 24
}
```

#### 更新规则
```
PUT /plugins/adguard_rule/rules/{id}
```

#### 删除规则
```
DELETE /plugins/adguard_rule/rules/{id}
```

#### **手动更新所有已启用规则** (对应 WebUI 更新按钮)
```
POST /plugins/adguard_rule/update
```

**响应 (202 Accepted)**
```
Update process for enabled rules has been started in the background.
```

---

### 11.2 sd_set 插件 (分流规则)

**基础路径**: `/plugins/sd_set`

#### 获取所有分流规则
```
GET /plugins/sd_set/config
```

**响应**
```json
[
  {
    "name": "分流规则1",
    "type": "sd_set",
    "url": "https://example.com/rules.srs",
    "files": ["rule_set.yaml"],
    "enabled": true,
    "auto_update": true,
    "update_interval_hours": 24,
    "rule_count": 1000,
    "last_updated": "2024-04-13T12:00:00+08:00"
  }
]
```

#### 添加/更新分流规则
```
POST /plugins/sd_set/config
```
```json
{
  "name": "规则名称",
  "type": "sd_set",
  "url": "https://example.com/rules.srs",
  "files": ["rule_set.yaml"],
  "enabled": true,
  "auto_update": true,
  "update_interval_hours": 24
}
```

#### 删除分流规则
```
DELETE /plugins/sd_set/config/{name}
```

#### **手动更新指定分流规则** (对应 WebUI 更新按钮)
```
POST /plugins/sd_set/update/{name}
```

**响应 (202 Accepted)**
```json
{"message": "update process for '规则名称' started in the background"}
```

#### 更新所有已启用的分流规则
```
POST /plugins/sd_set/update
```

---

### 11.3 domain_set / ip_set / si_set / domain_set_light / sd_set_light 插件

这些插件也通过 `/plugins/{plugin_tag}` 提供类似的 API：
- `GET /plugins/{tag}/config` - 获取配置
- `POST /plugins/{tag}/config` - 添加/更新
- `DELETE /plugins/{tag}/config/{name}` - 删除
- `POST /plugins/{tag}/update/{name}` - 更新单个
- `POST /plugins/{tag}/update` - 更新全部

---

## 12. aliapi 插件 API

**基础路径**: `/plugins/{aliapi_tag}`

aliapi 插件的 API 端点（如果插件注册了 API）：

```
GET /plugins/{tag}/domains     # 获取域名列表
GET /plugins/{tag}/ipv4        # 获取 IPv4 列表
GET /plugins/{tag}/ipv6        # 获取 IPv6 列表
```

---

## 13. 通用错误响应

所有 API 失败时返回：

```json
{"error": "错误信息"}
```

---

## 14. 配置文件与 API 的对应关系

| 配置文件 | 操作方式 |
|----------|----------|
| `upstream_overrides.json` | `/api/v1/upstream/config` |
| `config_overrides.json` | `/api/v1/overrides` |
| `special_upstream_groups.json` | `/api/v1/special-groups` |
| `adguard_rule/config.json` | `/plugins/adguard_rule/rules` |
| `sd_set/config.json` | `/plugins/sd_set/config` |
| `aliapi_upstream_*/config.json` | `/api/v1/upstream/config` |

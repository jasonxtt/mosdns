# MosDNS 全新前端 UI 设计方案

日期: 2026-05-15

## 1. 设计概述

为 mosdns DNS 代理系统设计一个全新的 Web UI，替换现有前端。采用深蓝紫暗黑科技风格，Bento 网格布局，可折叠侧边栏导航。

### 设计决策记录

| 决策 | 选择 | 原因 |
|------|------|------|
| 视觉风格 | 深蓝紫暗黑科技风 | 沉稳专业，适合长时间使用 |
| 导航结构 | 可折叠侧边栏 | 展开时显示图标+文字，折叠时只显示图标 |
| 首页布局 | Bento 网格 | 不对称网格，大图表突出，更有设计感 |
| 开关位置 | 独立设置页 | 首页不放开关，在设置页用网格卡片展示 |
| 日志展示 | 表格列表 | 每行一条查询，支持搜索过滤分页 |
| 规则管理 | 左右分栏 | 左侧规则分类列表，右侧编辑区 |
| 上游管理 | 分组卡片 | 分组展示（国内/国外/特殊组），可启用/禁用 |
| 数据刷新 | 自动轮询 | 3秒间隔，无需手动操作 |
| 图表库 | ECharts | 复用现有 Vue UI 的动态趋势图方案 |

## 2. 技术栈

- **前端框架**: Vue 3 + TypeScript + Vite
- **图表库**: ECharts 5.6+
- **HTTP 客户端**: 原生 fetch API
- **样式**: 纯 CSS + CSS Variables（无 UI 框架依赖）
- **构建产物**: 嵌入 Go 二进制（go:embed）

## 3. 色彩系统

```css
:root {
  --bg-primary: #0a0e1a;
  --bg-secondary: #0d1117;
  --bg-card: rgba(59, 130, 246, 0.06);
  --bg-card-hover: rgba(59, 130, 246, 0.12);
  --border-primary: rgba(59, 130, 246, 0.15);
  --border-accent: rgba(59, 130, 246, 0.3);

  --text-primary: #e0e0e0;
  --text-secondary: #888;
  --text-muted: #555;

  --accent-blue: #3b82f6;
  --accent-blue-light: #60a5fa;
  --accent-purple: #8b5cf6;
  --accent-purple-light: #a78bfa;
  --accent-green: #22c55e;
  --accent-green-light: #4ade80;
  --accent-amber: #f59e0b;
  --accent-amber-light: #fbbf24;
  --accent-red: #ef4444;
  --accent-red-light: #f87171;
  --accent-cyan: #06b6d4;
  --accent-cyan-light: #22d3ee;
}
```

## 4. 页面结构

### 4.1 侧边栏导航

- 可折叠：展开时 200px（图标+文字），折叠时 56px（仅图标）
- 顶部：MosDNS Logo（渐变蓝紫）
- 导航项：仪表盘、日志审计、规则上游、系统缓存、设置
- 底部：折叠/展开按钮

### 4.2 仪表盘页面 (`/`)

**Bento 网格布局：**

```
┌─────────────────────────┬──────────┐
│                         │ 查询总量  │
│    查询趋势图 (ECharts)  │ 1,234,567│
│    双轴: 请求数 + 延迟    ├──────────┤
│    3秒轮询, 平滑动画      │ 缓存命中  │
│    时间窗口: 1H/6H/24H   │  87.3%   │
├─────────┬───────┬───────┴──────────┤
│ 平均延迟 │ 客户端 │ 广告拦截 │ 域名分类 │
│  8.2ms  │  23   │ 12,847 │ 48.2K  │
└─────────┴───────┴────────┴────────┘
```

- **查询趋势图**: ECharts 实现，复用现有 `RealtimeTrendChart.vue` 方案
  - 双 Y 轴：左侧请求数（蓝色），右侧延迟 ms（绿色）
  - 平滑曲线 (smooth: 0.26)，渐变填充
  - 3秒轮询，滑动窗口 40 个数据点
  - 动画过渡 700ms cubicOut
  - 时间窗口切换按钮：1H / 6H / 24H
- **数据卡片**: 4 个核心指标，带趋势箭头和百分比变化
- **底部统计行**: 延迟、客户端数、广告拦截数、域名分类数

### 4.3 日志与审计页面

**布局：**
- 顶部：统计概览卡片（总查询、平均延迟、Top 域名、Top 客户端）
- 主体：查询日志表格
  - 列：时间、客户端、域名、类型、响应、耗时、来源
  - 支持搜索框过滤
  - 分页控件（每页 50 条）
  - 自动轮询刷新（3秒）

**数据源：**
- `GET /api/v2/audit/logs?page=N&limit=50` — 分页日志
- `GET /api/v2/audit/stats` — 统计概览
- `GET /api/v2/audit/rank/domain` — Top 域名
- `GET /api/v2/audit/rank/client` — Top 客户端

### 4.4 规则与上游页面

**左右分栏布局：**

左侧规则分类列表：
- 黑名单 (blocklist)
- 白名单 (whitelist)
- 灰名单 (greylist)
- DDNS 列表
- 广告拦截规则 (AdGuard)
- 订阅规则（SRS）
- 特殊路由组 (50-59)

右侧编辑区：
- 选中规则后显示规则内容
- 支持添加/删除/搜索域名
- 保存按钮（触发热重载）

**上游 DNS 管理：**
- 分组卡片展示：国内 (domestic)、国外 (foreign)、国外+ECS (foreignecs)
- 每组显示服务器列表、协议、地址
- 启用/禁用开关
- 保存按钮（热重载）

**数据源：**
- `GET /api/v1/upstream/config` — 上游配置
- `GET /api/v1/upstream/tags` — 上游标签
- `POST /api/v1/upstream/config` — 保存上游配置
- Plugin API: `/plugins/{tag}/list`, `/plugins/{tag}/post` — 规则 CRUD

### 4.5 系统与缓存页面

**缓存管理：**
- 7 个缓存实例的状态卡片（cache_all, cache_cn, cache_google 等）
- 每个卡片显示：大小、TTL、上次转储时间
- 操作按钮：刷新、保存、清空

**系统控制：**
- 服务重启按钮
- 更新检查与安装
- WebUI 端口设置
- 配置导出/导入（ZIP）
- 审计日志容量设置

**数据源：**
- `POST /plugins/{cache_tag}/flush` — 清空缓存
- `POST /plugins/{cache_tag}/save` — 保存缓存
- `GET /api/v1/update/status` — 更新状态
- `POST /api/v1/system/restart` — 重启服务
- `GET/POST /api/v1/system/webui-port` — WebUI 端口
- `POST /api/v1/config/export` — 导出配置
- `POST /api/v1/config/update_from_url` — 导入配置

### 4.6 设置页面

**运行时开关面板：**
- 15 个开关，5 列 3 行网格布局
- 每个开关卡片：
  - 开关名称（较大字号，12-14px）
  - 副标题/描述（较小字号，灰色）
  - Toggle 按钮（右侧，尺寸适中 36x18px）
  - 卡片背景色区分开/关状态
- 点击开关即时切换，POST 到 `/plugins/switchN/post`

**开关列表：**

| 开关 | 名称 | 描述 |
|------|------|------|
| switch1 | 域名黑洞 | 拦截无解析域名 + 黑名单 |
| switch2 | 选择性代理 | 白名单客户端走代理 |
| switch3 | 泄漏模式 | 兼容模式/安全模式 |
| switch4 | 过期缓存 L1 | 一级过期缓存 |
| switch5 | SOA/PTR 拦截 | 阻止 SOA/PTR/HTTPS 查询 |
| switch6 | AAAA 拦截 | 阻止 IPv6 查询 |
| switch7 | 广告拦截 | AdGuard 规则过滤 |
| switch8 | IPv4 优先 | 优先返回 A 记录 |
| switch9 | CN FakeIP | 国内域名 FakeIP/真实IP |
| switch10 | IPv6 优先 | 优先返回 AAAA 记录 |
| switch11 | Ali 私有 DoH | 使用阿里私有 DoH |
| switch12 | 选择性直连 | 黑名单客户端直连 |
| switch13 | 过期缓存 L2 | 二级过期缓存 |
| switch14 | ISP DNS | 使用运营商 DNS |
| switch15 | 其他 DNS | 使用其他 DNS |

**外观设置：**
- 面板背景（URL/上传/纯色）
- 文字颜色（亮色/暗色模式）
- 按钮颜色

## 5. API 接口清单

### 系统 API
- `GET /api/v1/system/webui-port` — 获取 WebUI 端口
- `POST /api/v1/system/webui-port` — 修改 WebUI 端口
- `POST /api/v1/system/restart` — 重启服务

### 审计 API v2
- `GET /api/v2/audit/stats` — 统计概览
- `GET /api/v2/audit/stats/windows` — 时间窗口统计
- `GET /api/v2/audit/logs?page=N&limit=N` — 分页日志
- `GET /api/v2/audit/rank/domain` — Top 域名
- `GET /api/v2/audit/rank/client` — Top 客户端
- `GET /api/v2/audit/rank/effective` — 有效路由排名
- `GET /api/v2/audit/rank/slowest` — 最慢查询

### 审计 API v1
- `POST /api/v1/audit/start` — 开始审计
- `POST /api/v1/audit/stop` — 停止审计
- `GET /api/v1/audit/status` — 审计状态
- `GET /api/v1/audit/capacity` — 审计容量
- `POST /api/v1/audit/capacity` — 设置审计容量

### 上游 API
- `GET /api/v1/upstream/tags` — 上游标签
- `GET /api/v1/upstream/config` — 上游配置
- `GET /api/v1/upstream/runtime/{tag}` — 上游运行时状态
- `POST /api/v1/upstream/config` — 保存上游配置

### 开关 API
- `POST /plugins/switch{N}/post` — 切换开关（body: `{"value":"A"}` 或 `{"value":"B"}`）

### 缓存 API
- `POST /plugins/cache_{name}/flush` — 清空缓存
- `POST /plugins/cache_{name}/save` — 保存缓存

### 规则 API (Plugin)
- `GET /plugins/{tag}/list` — 获取规则列表
- `POST /plugins/{tag}/post` — 添加规则
- `DELETE /plugins/{tag}/delete` — 删除规则

### 更新 API
- `GET /api/v1/update/status` — 更新状态
- `POST /api/v1/update/check` — 检查更新
- `POST /api/v1/update/apply` — 应用更新

### 配置 API
- `POST /api/v1/config/export` — 导出配置
- `POST /api/v1/config/update_from_url` — 导入配置

### 外观 API
- `GET/POST /api/v1/appearance/panel-background` — 背景设置
- `GET/POST /api/v1/appearance/text-color` — 文字颜色
- `GET/POST /api/v1/appearance/button-color` — 按钮颜色

## 6. 项目结构

```
mosdns-claude/
├── main.go                    # Go 入口，引用 coremain
├── coremain/
│   ├── mosdns.go              # HTTP 路由、API 处理、插件加载
│   └── www/                   # go:embed 静态资源
│       ├── index.html         # 新 UI 入口 HTML
│       └── assets/
│           └── vue-claude/    # 构建产物
│               ├── app.js
│               └── app.css
├── webui-claude/              # 新 Vue 前端源码
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   └── src/
│       ├── App.vue            # 主壳（侧边栏 + 路由视图）
│       ├── main.ts            # Vue 入口
│       ├── router.ts          # 路由配置
│       ├── styles/
│       │   └── variables.css  # CSS 变量
│       ├── api/
│       │   └── http.ts        # API 客户端
│       ├── composables/
│       │   └── useRealtimeMetrics.ts  # 实时数据轮询
│       ├── views/
│       │   ├── Dashboard.vue      # 仪表盘
│       │   ├── Logs.vue           # 日志审计
│       │   ├── Rules.vue          # 规则上游
│       │   ├── System.vue         # 系统缓存
│       │   └── Settings.vue       # 设置（开关面板）
│       ├── components/
│       │   ├── layout/
│       │   │   ├── Sidebar.vue    # 侧边栏
│       │   │   └── Header.vue     # 页面头部
│       │   ├── dashboard/
│       │   │   ├── RealtimeTrendChart.vue  # ECharts 趋势图
│       │   │   └── StatCard.vue   # 数据卡片
│       │   ├── logs/
│       │   │   └── LogTable.vue   # 日志表格
│       │   ├── rules/
│       │   │   ├── RuleList.vue   # 规则列表
│       │   │   └── RuleEditor.vue # 规则编辑器
│       │   ├── upstream/
│       │   │   └── UpstreamCard.vue # 上游卡片
│       │   ├── cache/
│       │   │   └── CacheCard.vue  # 缓存卡片
│       │   └── settings/
│       │       ├── SwitchGrid.vue # 开关网格
│       │       └── Appearance.vue # 外观设置
│       └── types/
│           └── dashboard.ts   # 类型定义
└── plugin/                    # Go 插件（从 mosdns 复制）
```

## 7. 构建流程

1. `cd webui-claude && npm install && npm run build`
2. 复制构建产物到 `coremain/www/assets/vue-claude/`
3. 创建 `coremain/www/index.html` 作为入口
4. `cd mosdns-claude && CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o mosdns ./`

## 8. 部署

- 编译后的二进制替换 10.0.0.91 上运行的 mosdns
- 重启服务后访问 WebUI 端口（默认 9099）

## 9. 与现有配置的匹配

UI 设计完全基于用户的实际配置文件 (`config_all/config_custom.yaml`)：

- **15 个开关** 对应 `switch.yaml` 中的 switch1-switch15
- **缓存管理** 对应 `cache.yaml` 中的 7 个缓存实例
- **上游分组** 对应 `forward_local.yaml`、`forward_nocn.yaml`、`forward_nocn_ecs.yaml`
- **特殊路由组** 对应 `special_groups.yaml` 中的 10 个组 (50-59)
- **规则列表** 对应 `rule_set.yaml` 中的各类 domain_set、sd_set 等
- **审计日志** 对应 `audit_settings.json` 配置
- **域名输出** 对应 `domain_output.yaml` 中的动态域名列表

# MosDNS Frontend UI Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a complete new Vue 3 frontend UI for mosdns with dark blue-purple theme, Bento grid dashboard, ECharts trend chart, collapsible sidebar, and 5 pages (dashboard, logs, rules, system, settings).

**Architecture:** Vue 3 SPA with TypeScript, built via Vite into static assets embedded in the Go binary via `go:embed`. The UI consumes existing mosdns REST APIs. The Go backend is copied from the original mosdns repo with modifications to serve the new UI at `/`.

**Tech Stack:** Vue 3, TypeScript, Vite 7, ECharts 5.6+, CSS Variables, Go (chi router)

---

## File Structure

```
mosdns-claude/
├── main.go
├── go.mod / go.sum
├── coremain/
│   ├── mosdns.go              # Modified: serve index.html at /
│   ├── www/
│   │   ├── index.html         # New UI entry HTML
│   │   └── assets/vue-claude/ # Build output
│   └── ...                    # All other Go files from original
├── webui-claude/
│   ├── package.json
│   ├── vite.config.ts
│   ├── index.html
│   ├── tsconfig.json
│   └── src/
│       ├── main.ts
│       ├── App.vue
│       ├── router.ts
│       ├── styles/
│       │   └── main.css
│       ├── api/
│       │   └── http.ts
│       ├── types/
│       │   └── index.ts
│       ├── composables/
│       │   └── useRealtimeMetrics.ts
│       ├── views/
│       │   ├── Dashboard.vue
│       │   ├── Logs.vue
│       │   ├── Rules.vue
│       │   ├── System.vue
│       │   └── Settings.vue
│       └── components/
│           ├── layout/
│           │   ├── AppSidebar.vue
│           │   └── AppHeader.vue
│           ├── dashboard/
│           │   ├── TrendChart.vue
│           │   └── StatCard.vue
│           ├── logs/
│           │   └── LogTable.vue
│           ├── rules/
│           │   ├── RuleList.vue
│           │   └── UpstreamCards.vue
│           ├── cache/
│           │   └── CacheCard.vue
│           └── settings/
│               └── SwitchGrid.vue
```

---

## Task 1: Project Scaffolding

**Files:**
- Create: `mosdns-claude/` (entire directory)
- Create: `mosdns-claude/webui-claude/package.json`
- Create: `mosdns-claude/webui-claude/vite.config.ts`
- Create: `mosdns-claude/webui-claude/tsconfig.json`
- Create: `mosdns-claude/webui-claude/index.html`
- Create: `mosdns-claude/webui-claude/src/main.ts`
- Create: `mosdns-claude/coremain/www/index.html`

- [ ] **Step 1: Copy Go source from original mosdns**

```bash
cd /Users/tom/Documents/github
rm -rf mosdns-claude
cp -R mosdns mosdns-claude
cd mosdns-claude
# Remove old UI workspaces
rm -rf webui-log webui-blog
# Remove old build artifacts in www
rm -f coremain/www/log.html coremain/www/blog.html coremain/www/dashboard.html coremain/www/mosdnsp.html
rm -rf coremain/www/assets/vue-log coremain/www/assets/vue-blog
rm -rf coremain/www/assets/js coremain/www/assets/css
```

- [ ] **Step 2: Create Vue project package.json**

```json
{
  "name": "mosdns-claude-vue",
  "version": "1.0.0",
  "private": true,
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "vite build",
    "preview": "vite preview"
  },
  "dependencies": {
    "echarts": "^5.6.0",
    "vue": "^3.5.22",
    "vue-router": "^4.5.0"
  },
  "devDependencies": {
    "@vitejs/plugin-vue": "^6.0.1",
    "typescript": "^5.7.0",
    "vite": "^7.1.9",
    "vue-tsc": "^2.2.0"
  }
}
```

- [ ] **Step 3: Create vite.config.ts**

```typescript
import path from 'node:path'
import fs from 'node:fs'
import { fileURLToPath } from 'node:url'
import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const assetVersion = process.env.MOSDNS_ASSET_VERSION || new Date().toISOString().replace(/[-:TZ.]/g, '').slice(0, 14)
const outDir = path.resolve(__dirname, '../coremain/www/assets/vue-claude')
const rootHtmlPath = path.resolve(__dirname, '../coremain/www/index.html')

export default defineConfig({
  plugins: [
    vue(),
    {
      name: 'mosdns-asset-version-stamp',
      closeBundle() {
        const files = ['index.html']
        for (const file of files) {
          const filePath = path.join(outDir, file)
          if (!fs.existsSync(filePath)) continue
          const html = fs.readFileSync(filePath, 'utf8')
          const stamped = html
            .replace('/app.js', `/app.js?v=${assetVersion}`)
            .replace('/app.css', `/app.css?v=${assetVersion}`)
          if (stamped !== html) fs.writeFileSync(filePath, stamped, 'utf8')
        }
        if (!fs.existsSync(rootHtmlPath)) return
        const rootHtml = fs.readFileSync(rootHtmlPath, 'utf8')
        const stampedRootHtml = rootHtml
          .replace(/\/assets\/vue-claude\/app\.js\?v=[^"']+/g, `/assets/vue-claude/app.js?v=${assetVersion}`)
          .replace(/\/assets\/vue-claude\/app\.css\?v=[^"']+/g, `/assets/vue-claude/app.css?v=${assetVersion}`)
        if (stampedRootHtml !== rootHtml) fs.writeFileSync(rootHtmlPath, stampedRootHtml, 'utf8')
      }
    }
  ],
  publicDir: false,
  build: {
    outDir,
    emptyOutDir: true,
    sourcemap: false,
    cssCodeSplit: false,
    rollupOptions: {
      input: path.resolve(__dirname, 'index.html'),
      output: {
        entryFileNames: 'app.js',
        chunkFileNames: 'chunks/[name]-[hash].js',
        assetFileNames: (assetInfo) => {
          if (assetInfo?.names?.some((name) => name.endsWith('.css'))) return 'app.css'
          return 'assets/[name]-[hash][extname]'
        }
      }
    }
  },
  server: {
    proxy: {
      '/api': 'http://localhost:9099',
      '/plugins': 'http://localhost:9099'
    }
  }
})
```

- [ ] **Step 4: Create tsconfig.json**

```json
{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ESNext",
    "moduleResolution": "bundler",
    "strict": true,
    "jsx": "preserve",
    "resolveJsonModule": true,
    "isolatedModules": true,
    "esModuleInterop": true,
    "lib": ["ES2022", "DOM", "DOM.Iterable"],
    "skipLibCheck": true,
    "noEmit": true,
    "paths": {
      "@/*": ["./src/*"]
    },
    "baseUrl": "."
  },
  "include": ["src/**/*.ts", "src/**/*.vue", "env.d.ts"]
}
```

- [ ] **Step 5: Create env.d.ts**

```typescript
/// <reference types="vite/client" />
declare module '*.vue' {
  import type { DefineComponent } from 'vue'
  const component: DefineComponent<{}, {}, any>
  export default component
}
```

- [ ] **Step 6: Create index.html (Vue entry)**

```html
<!DOCTYPE html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>MosDNS</title>
  <link rel="icon" href="data:image/svg+xml,<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32'><rect width='32' height='32' rx='6' fill='%233b82f6'/><text x='50%25' y='55%25' dominant-baseline='middle' text-anchor='middle' fill='white' font-size='18' font-weight='800'>M</text></svg>" />
</head>
<body>
  <div id="app"></div>
  <script type="module" src="/src/main.ts"></script>
</body>
</html>
```

- [ ] **Step 7: Create coremain/www/index.html (Go embed entry)**

```html
<!doctype html>
<html lang="zh-CN">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>MosDNS</title>
  <link rel="stylesheet" href="/assets/vue-claude/app.css?v=placeholder" />
</head>
<body>
  <div id="app"></div>
  <script type="module" src="/assets/vue-claude/app.js?v=placeholder"></script>
</body>
</html>
```

- [ ] **Step 8: Install npm dependencies**

```bash
cd /Users/tom/Documents/github/mosdns-claude/webui-claude
npm install
```

---

## Task 2: CSS Theme System

**Files:**
- Create: `mosdns-claude/webui-claude/src/styles/main.css`

- [ ] **Step 1: Create main.css with full theme**

```css
@import url('https://fonts.googleapis.com/css2?family=IBM+Plex+Sans:wght@400;500;600;700&display=swap');

:root {
  --bg-primary: #0a0e1a;
  --bg-secondary: #0d1117;
  --bg-card: rgba(59, 130, 246, 0.06);
  --bg-card-hover: rgba(59, 130, 246, 0.12);
  --bg-card-active: rgba(59, 130, 246, 0.18);
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

  --sidebar-width: 200px;
  --sidebar-collapsed-width: 56px;
  --header-height: 52px;
  --radius-sm: 6px;
  --radius-md: 10px;
  --radius-lg: 14px;
  --transition-fast: 0.15s ease;
  --transition-normal: 0.25s ease;
}

* {
  margin: 0;
  padding: 0;
  box-sizing: border-box;
}

html, body, #app {
  height: 100%;
  width: 100%;
  overflow: hidden;
}

body {
  font-family: 'IBM Plex Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif;
  background: var(--bg-primary);
  color: var(--text-primary);
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}

::-webkit-scrollbar {
  width: 6px;
  height: 6px;
}
::-webkit-scrollbar-track {
  background: transparent;
}
::-webkit-scrollbar-thumb {
  background: rgba(255,255,255,0.1);
  border-radius: 3px;
}
::-webkit-scrollbar-thumb:hover {
  background: rgba(255,255,255,0.2);
}

.app-layout {
  display: flex;
  height: 100%;
  width: 100%;
}

.main-area {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  overflow: hidden;
}

.page-content {
  flex: 1;
  overflow-y: auto;
  padding: 20px 24px;
}

/* Card base */
.card {
  background: var(--bg-card);
  border: 1px solid var(--border-primary);
  border-radius: var(--radius-md);
  padding: 16px;
  transition: border-color var(--transition-fast), background var(--transition-fast);
}
.card:hover {
  border-color: var(--border-accent);
  background: var(--bg-card-hover);
}

/* Button base */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 16px;
  border: 1px solid var(--border-accent);
  border-radius: var(--radius-sm);
  background: var(--bg-card);
  color: var(--text-primary);
  font-size: 13px;
  font-family: inherit;
  cursor: pointer;
  transition: all var(--transition-fast);
}
.btn:hover {
  background: var(--bg-card-hover);
  border-color: var(--accent-blue);
}
.btn-primary {
  background: var(--accent-blue);
  border-color: var(--accent-blue);
  color: #fff;
}
.btn-primary:hover {
  background: #2563eb;
}
.btn-sm {
  padding: 4px 10px;
  font-size: 12px;
}

/* Toggle switch */
.toggle {
  position: relative;
  width: 36px;
  height: 20px;
  border-radius: 10px;
  background: #444;
  cursor: pointer;
  transition: background var(--transition-fast);
  flex-shrink: 0;
}
.toggle.active {
  background: var(--accent-blue);
}
.toggle::after {
  content: '';
  position: absolute;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: #fff;
  top: 2px;
  left: 2px;
  transition: transform var(--transition-fast);
}
.toggle.active::after {
  transform: translateX(16px);
}

/* Badge */
.badge {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 11px;
  font-weight: 500;
}
.badge-green { background: rgba(34,197,94,0.15); color: var(--accent-green-light); }
.badge-red { background: rgba(239,68,68,0.15); color: var(--accent-red-light); }
.badge-blue { background: rgba(59,130,246,0.15); color: var(--accent-blue-light); }
.badge-amber { background: rgba(245,158,11,0.15); color: var(--accent-amber-light); }

/* Input */
.input {
  background: rgba(255,255,255,0.05);
  border: 1px solid var(--border-primary);
  border-radius: var(--radius-sm);
  padding: 8px 12px;
  color: var(--text-primary);
  font-size: 13px;
  font-family: inherit;
  outline: none;
  transition: border-color var(--transition-fast);
}
.input:focus {
  border-color: var(--accent-blue);
}
.input::placeholder {
  color: var(--text-muted);
}

/* Table */
.table-wrap {
  overflow-x: auto;
}
table {
  width: 100%;
  border-collapse: collapse;
  font-size: 13px;
}
thead th {
  text-align: left;
  padding: 10px 12px;
  color: var(--text-secondary);
  font-weight: 500;
  font-size: 11px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  border-bottom: 1px solid var(--border-primary);
  position: sticky;
  top: 0;
  background: var(--bg-primary);
  z-index: 1;
}
tbody td {
  padding: 8px 12px;
  border-bottom: 1px solid rgba(255,255,255,0.04);
  color: var(--text-primary);
}
tbody tr:hover {
  background: rgba(59,130,246,0.06);
}

/* Pagination */
.pagination {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 12px 0;
}

/* Page title */
.page-title {
  font-size: 18px;
  font-weight: 600;
  color: var(--text-primary);
}
.page-subtitle {
  font-size: 12px;
  color: var(--text-muted);
  margin-top: 2px;
}

/* Animations */
@keyframes fadeIn {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}
.fade-in {
  animation: fadeIn 0.3s ease forwards;
}
```

---

## Task 3: API Client and Types

**Files:**
- Create: `mosdns-claude/webui-claude/src/api/http.ts`
- Create: `mosdns-claude/webui-claude/src/types/index.ts`

- [ ] **Step 1: Create http.ts**

```typescript
async function request(url: string, options: RequestInit = {}): Promise<Response> {
  const response = await fetch(url, options)
  if (!response.ok) {
    let message = `HTTP ${response.status} ${response.statusText}`
    try {
      const data = await response.json()
      if (data?.error) message = data.error
    } catch {
      try {
        const text = await response.text()
        if (text) message = text
      } catch {}
    }
    throw new Error(message)
  }
  return response
}

export async function getJSON<T = any>(url: string): Promise<T> {
  const res = await request(url)
  return res.json()
}

export async function postJSON<T = any>(url: string, body?: any): Promise<T> {
  const res = await request(url, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: body ? JSON.stringify(body) : undefined
  })
  const ct = res.headers.get('content-type') || ''
  if (ct.includes('application/json')) return res.json()
  return res.text() as any
}

export async function deleteJSON<T = any>(url: string): Promise<T | null> {
  const res = await request(url, { method: 'DELETE' })
  if (res.status === 204) return null
  const ct = res.headers.get('content-type') || ''
  if (ct.includes('application/json')) return res.json()
  return res.text() as any
}
```

- [ ] **Step 2: Create types/index.ts**

```typescript
export interface DashboardMetrics {
  timestamps: string[]
  requestCounts: number[]
  avgLatencyMs: number[]
  totalQueries: number
  averageLatency: number
  currentQueries: number
  currentLatency: number
}

export interface AuditStats {
  total_queries: number
  average_duration_ms: number
}

export interface AuditLog {
  trace_id?: string
  query_time: string
  client_ip: string
  query_name: string
  query_type: string
  response_code: string
  duration_ms: number
  upstream_tag?: string
  matched_domain_set?: string
}

export interface RankItem {
  name: string
  count: number
}

export interface UpstreamConfig {
  tag: string
  addr: string
  protocol?: string
  enabled?: boolean
}

export interface UpstreamGroup {
  name: string
  tag: string
  upstreams: UpstreamConfig[]
}

export interface CacheInfo {
  tag: string
  name: string
}

export interface SwitchState {
  tag: string
  name: string
  description: string
  value: 'A' | 'B'
  labels: { A: string; B: string }
}

export const SWITCH_DEFINITIONS: Omit<SwitchState, 'value'>[] = [
  { tag: 'switch1', name: '域名黑洞', description: '拦截无解析域名 + 黑名单', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch2', name: '选择性代理', description: '白名单客户端走代理', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch3', name: '泄漏模式', description: '兼容模式 / 安全模式', labels: { A: '泄漏', B: '安全' } },
  { tag: 'switch4', name: '缓存 L1', description: '一级过期缓存', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch5', name: 'SOA 拦截', description: '阻止 SOA/PTR/HTTPS 查询', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch6', name: 'AAAA 拦截', description: '阻止 IPv6 查询', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch7', name: '广告拦截', description: 'AdGuard 规则过滤', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch8', name: 'IPv4 优先', description: '优先返回 A 记录', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch9', name: 'CN FakeIP', description: '国内域名 FakeIP / 真实 IP', labels: { A: '真实 IP', B: 'FakeIP' } },
  { tag: 'switch10', name: 'IPv6 优先', description: '优先返回 AAAA 记录', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch11', name: 'Ali 私有 DoH', description: '使用阿里私有 DoH', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch12', name: '选择性直连', description: '黑名单客户端直连', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch13', name: '缓存 L2', description: '二级过期缓存', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch14', name: 'ISP DNS', description: '使用运营商 DNS', labels: { A: '启用', B: '禁用' } },
  { tag: 'switch15', name: '其他 DNS', description: '使用其他 DNS', labels: { A: '启用', B: '禁用' } },
]

export const CACHE_TAGS = [
  { tag: 'cache_all', name: '主缓存 (泄漏模式)' },
  { tag: 'cache_all_noleak', name: '主缓存 (安全模式)' },
  { tag: 'cache_cn', name: '国内域名缓存' },
  { tag: 'cache_google', name: '国外域名缓存' },
  { tag: 'cache_google_node', name: '国外 ECS 缓存' },
  { tag: 'cache_node', name: '节点域名缓存' },
  { tag: 'cache_cnmihomo', name: 'Mihomo 缓存' },
]
```

---

## Task 4: Router and Main Entry

**Files:**
- Create: `mosdns-claude/webui-claude/src/router.ts`
- Create: `mosdns-claude/webui-claude/src/main.ts`
- Create: `mosdns-claude/webui-claude/src/App.vue`

- [ ] **Step 1: Create router.ts**

```typescript
import { createRouter, createWebHashHistory } from 'vue-router'

const routes = [
  { path: '/', name: 'dashboard', component: () => import('./views/Dashboard.vue') },
  { path: '/logs', name: 'logs', component: () => import('./views/Logs.vue') },
  { path: '/rules', name: 'rules', component: () => import('./views/Rules.vue') },
  { path: '/system', name: 'system', component: () => import('./views/System.vue') },
  { path: '/settings', name: 'settings', component: () => import('./views/Settings.vue') },
]

export const router = createRouter({
  history: createWebHashHistory(),
  routes
})
```

- [ ] **Step 2: Create main.ts**

```typescript
import { createApp } from 'vue'
import { router } from './router'
import App from './App.vue'
import './styles/main.css'

createApp(App).use(router).mount('#app')
```

- [ ] **Step 3: Create App.vue shell**

```vue
<script setup lang="ts">
import AppSidebar from './components/layout/AppSidebar.vue'
</script>

<template>
  <div class="app-layout">
    <AppSidebar />
    <div class="main-area">
      <router-view v-slot="{ Component }">
        <transition name="page" mode="out-in">
          <component :is="Component" />
        </transition>
      </router-view>
    </div>
  </div>
</template>

<style>
.page-enter-active,
.page-leave-active {
  transition: opacity 0.15s ease, transform 0.15s ease;
}
.page-enter-from {
  opacity: 0;
  transform: translateY(6px);
}
.page-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}
</style>
```

---

## Task 5: Layout Components (Sidebar + Header)

**Files:**
- Create: `mosdns-claude/webui-claude/src/components/layout/AppSidebar.vue`
- Create: `mosdns-claude/webui-claude/src/components/layout/AppHeader.vue`

- [ ] **Step 1: Create AppSidebar.vue**

```vue
<script setup lang="ts">
import { ref } from 'vue'
import { useRouter, useRoute } from 'vue-router'

const collapsed = ref(false)
const router = useRouter()
const route = useRoute()

const navItems = [
  { path: '/', icon: '📊', label: '仪表盘' },
  { path: '/logs', icon: '📋', label: '日志审计' },
  { path: '/rules', icon: '🔀', label: '规则上游' },
  { path: '/system', icon: '💾', label: '系统缓存' },
  { path: '/settings', icon: '⚙️', label: '设置' },
]

function navigate(path: string) {
  router.push(path)
}
</script>

<template>
  <aside class="sidebar" :class="{ collapsed }">
    <div class="sidebar-logo" @click="navigate('/')">
      <div class="logo-icon">M</div>
      <transition name="fade">
        <span v-if="!collapsed" class="logo-text">MosDNS</span>
      </transition>
    </div>

    <nav class="sidebar-nav">
      <div
        v-for="item in navItems"
        :key="item.path"
        class="nav-item"
        :class="{ active: route.path === item.path }"
        @click="navigate(item.path)"
        :title="collapsed ? item.label : undefined"
      >
        <span class="nav-icon">{{ item.icon }}</span>
        <transition name="fade">
          <span v-if="!collapsed" class="nav-label">{{ item.label }}</span>
        </transition>
      </div>
    </nav>

    <div class="sidebar-footer">
      <div class="nav-item" @click="collapsed = !collapsed" :title="collapsed ? '展开' : '折叠'">
        <span class="nav-icon">{{ collapsed ? '→' : '←' }}</span>
        <transition name="fade">
          <span v-if="!collapsed" class="nav-label">折叠</span>
        </transition>
      </div>
    </div>
  </aside>
</template>

<style scoped>
.sidebar {
  width: var(--sidebar-width);
  height: 100%;
  background: var(--bg-secondary);
  border-right: 1px solid var(--border-primary);
  display: flex;
  flex-direction: column;
  transition: width var(--transition-normal);
  flex-shrink: 0;
  overflow: hidden;
}
.sidebar.collapsed {
  width: var(--sidebar-collapsed-width);
}

.sidebar-logo {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 16px;
  cursor: pointer;
  border-bottom: 1px solid var(--border-primary);
  min-height: 52px;
}
.logo-icon {
  width: 32px;
  height: 32px;
  background: linear-gradient(135deg, var(--accent-blue), var(--accent-purple));
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: #fff;
  font-size: 15px;
  font-weight: 800;
  flex-shrink: 0;
}
.logo-text {
  font-size: 15px;
  font-weight: 700;
  color: var(--text-primary);
  white-space: nowrap;
}

.sidebar-nav {
  flex: 1;
  padding: 8px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
  color: var(--text-secondary);
  white-space: nowrap;
}
.nav-item:hover {
  background: var(--bg-card-hover);
  color: var(--text-primary);
}
.nav-item.active {
  background: rgba(59, 130, 246, 0.15);
  color: var(--accent-blue-light);
}
.nav-icon {
  font-size: 16px;
  width: 20px;
  text-align: center;
  flex-shrink: 0;
}
.nav-label {
  font-size: 13px;
  font-weight: 500;
}

.sidebar-footer {
  padding: 8px;
  border-top: 1px solid var(--border-primary);
}

.fade-enter-active, .fade-leave-active {
  transition: opacity 0.15s ease;
}
.fade-enter-from, .fade-leave-to {
  opacity: 0;
}
</style>
```

- [ ] **Step 2: Create AppHeader.vue**

```vue
<script setup lang="ts">
defineProps<{
  title: string
  subtitle?: string
}>()
</script>

<template>
  <div class="page-header">
    <div>
      <h1 class="page-title">{{ title }}</h1>
      <p v-if="subtitle" class="page-subtitle">{{ subtitle }}</p>
    </div>
    <div class="header-actions">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 20px;
}
.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
```

---

## Task 6: Realtime Metrics Composable

**Files:**
- Create: `mosdns-claude/webui-claude/src/composables/useRealtimeMetrics.ts`

- [ ] **Step 1: Create useRealtimeMetrics.ts (adapted from existing)**

```typescript
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON } from '../api/http'
import type { DashboardMetrics, AuditStats, AuditLog } from '../types'

const DEFAULT_POLL_INTERVAL_MS = 3000
const DEFAULT_WINDOW_SIZE = 40
const DEFAULT_LOG_SAMPLE_LIMIT = 160

function formatTimelineLabel(date: Date): string {
  return date.toLocaleTimeString('zh-CN', { hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit' })
}

export function useRealtimeMetrics(options: { pollIntervalMs?: number; windowSize?: number } = {}) {
  const pollIntervalMs = Math.max(1000, options.pollIntervalMs || DEFAULT_POLL_INTERVAL_MS)
  const windowSize = Math.max(30, options.windowSize || DEFAULT_WINDOW_SIZE)

  const metrics = reactive<DashboardMetrics>({
    timestamps: [],
    requestCounts: [],
    avgLatencyMs: [],
    totalQueries: 0,
    averageLatency: 0,
    currentQueries: 0,
    currentLatency: 0
  })

  const isRunning = ref(true)
  const initialized = ref(false)
  const warningMessage = ref('')
  let pollTimerId = 0
  let previousTotalQueries: number | null = null
  let previousTopLogKey: string | null = null
  const inFlight = ref(false)

  function buildLogKey(log: AuditLog): string {
    const traceId = String(log.trace_id || '').trim()
    if (traceId) return traceId
    return [log.query_time, log.client_ip, log.query_name, log.query_type].join('|')
  }

  function appendPoint(timestamp: string, requestCount: number, avgLatencyMs: number) {
    metrics.timestamps.push(timestamp)
    metrics.requestCounts.push(requestCount)
    metrics.avgLatencyMs.push(avgLatencyMs)
    while (metrics.timestamps.length > windowSize) metrics.timestamps.shift()
    while (metrics.requestCounts.length > windowSize) metrics.requestCounts.shift()
    while (metrics.avgLatencyMs.length > windowSize) metrics.avgLatencyMs.shift()
  }

  async function refreshMetrics() {
    if (inFlight.value) return
    inFlight.value = true
    try {
      const [stats, logs, status] = await Promise.allSettled([
        getJSON<AuditStats>('/api/v2/audit/stats'),
        getJSON<{ data: AuditLog[] }>('/api/v2/audit/logs?page=1&limit=' + DEFAULT_LOG_SAMPLE_LIMIT),
        getJSON<{ running: boolean }>('/api/v1/audit/status')
      ])

      if (status.status === 'fulfilled') isRunning.value = Boolean((status.value as any).running ?? true)

      if (stats.status !== 'fulfilled') throw stats.reason
      const s = stats.value
      const totalQueries = Number(s.total_queries || 0)
      const averageLatency = Number(s.average_duration_ms || 0)
      const fallbackCurrentQueries = previousTotalQueries === null ? 0 : totalQueries >= previousTotalQueries ? totalQueries - previousTotalQueries : totalQueries

      let currentQueries = fallbackCurrentQueries
      let currentLatency = averageLatency

      if (logs.status === 'fulfilled') {
        const logData = (logs.value as any).data || logs.value || []
        if (Array.isArray(logData) && logData.length > 0) {
          const newest = logData[0]
          const newestKey = buildLogKey(newest)
          currentLatency = Number(newest?.duration_ms || averageLatency)
          if (previousTopLogKey !== null) {
            let deltaFromLogs = 0
            for (const log of logData) {
              if (buildLogKey(log) === previousTopLogKey) break
              deltaFromLogs++
            }
            currentQueries = deltaFromLogs
          }
          previousTopLogKey = newestKey
        }
      }

      if (currentQueries === 0 && fallbackCurrentQueries > 0) currentQueries = fallbackCurrentQueries
      previousTotalQueries = totalQueries

      metrics.totalQueries = totalQueries
      metrics.averageLatency = averageLatency
      metrics.currentQueries = currentQueries
      metrics.currentLatency = currentLatency
      appendPoint(formatTimelineLabel(new Date()), currentQueries, currentLatency)
      warningMessage.value = ''
      initialized.value = true
    } catch (error) {
      warningMessage.value = `刷新失败: ${error instanceof Error ? error.message : String(error)}`
    } finally {
      inFlight.value = false
    }
  }

  function stopPolling() {
    if (pollTimerId) { clearInterval(pollTimerId); pollTimerId = 0 }
  }
  function startPolling() {
    stopPolling()
    pollTimerId = window.setInterval(() => refreshMetrics(), pollIntervalMs)
  }

  onMounted(() => { refreshMetrics(); startPolling() })
  onBeforeUnmount(() => { stopPolling() })

  return { metrics, isRunning, initialized, warningMessage, refreshMetrics }
}
```

---

## Task 7: Dashboard Page (Trend Chart + Stat Cards)

**Files:**
- Create: `mosdns-claude/webui-claude/src/views/Dashboard.vue`
- Create: `mosdns-claude/webui-claude/src/components/dashboard/TrendChart.vue`
- Create: `mosdns-claude/webui-claude/src/components/dashboard/StatCard.vue`

- [ ] **Step 1: Create StatCard.vue**

```vue
<script setup lang="ts">
defineProps<{
  label: string
  value: string
  trend?: string
  trendUp?: boolean
  color: string
}>()
</script>

<template>
  <div class="stat-card" :style="{ '--card-accent': color }">
    <div class="stat-label">{{ label }}</div>
    <div class="stat-value">{{ value }}</div>
    <div v-if="trend" class="stat-trend" :class="trendUp !== false ? 'up' : 'down'">
      {{ trendUp !== false ? '↑' : '↓' }} {{ trend }}
    </div>
  </div>
</template>

<style scoped>
.stat-card {
  background: rgba(59, 130, 246, 0.06);
  border: 1px solid var(--border-primary);
  border-radius: var(--radius-md);
  padding: 14px 16px;
  transition: border-color var(--transition-fast);
}
.stat-card:hover {
  border-color: var(--card-accent, var(--border-accent));
}
.stat-label {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 1px;
  color: var(--card-accent, var(--accent-blue-light));
  font-weight: 600;
}
.stat-value {
  font-size: 26px;
  font-weight: 700;
  color: #fff;
  margin: 4px 0;
  font-variant-numeric: tabular-nums;
}
.stat-trend {
  font-size: 11px;
  display: flex;
  align-items: center;
  gap: 4px;
}
.stat-trend.up { color: var(--accent-green-light); }
.stat-trend.down { color: var(--accent-red-light); }
</style>
```

- [ ] **Step 2: Create TrendChart.vue (ECharts, adapted from existing)**

```vue
<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref, watch } from 'vue'
import * as echarts from 'echarts/core'
import type { ECharts, EChartsOption } from 'echarts/core'
import { LineChart } from 'echarts/charts'
import { GridComponent, LegendComponent, TooltipComponent } from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'

echarts.use([LineChart, GridComponent, LegendComponent, TooltipComponent, CanvasRenderer])

const props = defineProps<{
  timestamps: string[]
  requestCounts: number[]
  avgLatencyMs: number[]
}>()

const chartEl = ref<HTMLDivElement | null>(null)
let chart: ECharts | null = null
let resizeObserver: ResizeObserver | null = null

function formatCount(v: number): string {
  if (v >= 1000000) return (v / 1000000).toFixed(1) + 'M'
  if (v >= 1000) return (v / 1000).toFixed(1) + 'K'
  return String(v)
}

function createOption(): EChartsOption {
  return {
    animation: true,
    animationDuration: 520,
    animationEasing: 'cubicOut',
    animationDurationUpdate: 700,
    animationEasingUpdate: 'cubicOut',
    textStyle: { color: '#c8d6eb', fontFamily: 'IBM Plex Sans, sans-serif' },
    legend: { show: false },
    tooltip: {
      trigger: 'axis',
      borderWidth: 1,
      borderColor: 'rgba(155, 186, 230, 0.24)',
      backgroundColor: 'rgba(9, 16, 30, 0.94)',
      textStyle: { color: '#e8f0ff', fontSize: 12 },
      axisPointer: { type: 'line', lineStyle: { type: 'dashed', color: 'rgba(143, 174, 219, 0.55)', width: 1 } }
    },
    grid: { top: 14, left: 48, right: 58, bottom: 24 },
    xAxis: {
      type: 'category', boundaryGap: false, data: [],
      axisTick: { show: false },
      axisLine: { lineStyle: { color: 'rgba(130, 157, 197, 0.22)' } },
      axisLabel: { color: '#7f95b9', fontSize: 11 },
      splitLine: { show: false }
    },
    yAxis: [
      {
        type: 'value', name: '请求数',
        nameTextStyle: { color: '#7f95b9', fontSize: 11, align: 'left' },
        axisLabel: { color: '#7f95b9', fontSize: 11, formatter: (v: number) => formatCount(v) },
        axisLine: { show: false },
        splitLine: { lineStyle: { color: 'rgba(119, 147, 188, 0.12)' } }
      },
      {
        type: 'value', name: '延迟 (ms)',
        nameTextStyle: { color: '#7f95b9', fontSize: 11, align: 'right' },
        axisLabel: { color: '#7f95b9', fontSize: 11, formatter: (v: number) => v.toFixed(0) },
        axisLine: { show: false },
        splitLine: { show: false }
      }
    ],
    series: [
      {
        name: '请求数', type: 'line', yAxisIndex: 0, smooth: 0.26, showSymbol: false,
        lineStyle: { width: 2, color: '#5da8ff' },
        itemStyle: { color: '#5da8ff' },
        areaStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
          { offset: 0, color: 'rgba(93, 168, 255, 0.28)' },
          { offset: 1, color: 'rgba(93, 168, 255, 0.02)' }
        ])},
        emphasis: { focus: 'series', scale: false },
        data: []
      },
      {
        name: '延迟', type: 'line', yAxisIndex: 1, smooth: 0.26, showSymbol: false,
        lineStyle: { width: 2, color: '#40d889' },
        itemStyle: { color: '#40d889' },
        areaStyle: { color: new echarts.graphic.LinearGradient(0, 0, 0, 1, [
          { offset: 0, color: 'rgba(64, 216, 137, 0.22)' },
          { offset: 1, color: 'rgba(64, 216, 137, 0.02)' }
        ])},
        emphasis: { focus: 'series', scale: false },
        data: []
      }
    ]
  }
}

function syncData() {
  if (!chart) return
  chart.setOption({
    xAxis: { data: props.timestamps },
    series: [
      { data: props.requestCounts },
      { data: props.avgLatencyMs }
    ]
  }, { notMerge: false, lazyUpdate: true, silent: true })
}

onMounted(() => {
  if (!chartEl.value) return
  chart = echarts.init(chartEl.value, undefined, { renderer: 'canvas' })
  chart.setOption(createOption())
  syncData()
  resizeObserver = new ResizeObserver(() => chart?.resize())
  resizeObserver.observe(chartEl.value)
  window.addEventListener('resize', () => chart?.resize())
})

watch(() => [props.timestamps, props.requestCounts, props.avgLatencyMs], syncData, { deep: true })

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  chart?.dispose()
})
</script>

<template>
  <div ref="chartEl" class="trend-chart"></div>
</template>

<style scoped>
.trend-chart {
  width: 100%;
  height: 220px;
}
</style>
```

- [ ] **Step 3: Create Dashboard.vue**

```vue
<script setup lang="ts">
import AppHeader from '../components/layout/AppHeader.vue'
import TrendChart from '../components/dashboard/TrendChart.vue'
import StatCard from '../components/dashboard/StatCard.vue'
import { useRealtimeMetrics } from '../composables/useRealtimeMetrics'

const { metrics, isRunning, initialized, warningMessage } = useRealtimeMetrics()

function formatNumber(n: number): string {
  if (n >= 1000000) return (n / 1000000).toFixed(1) + 'M'
  if (n >= 1000) return (n / 1000).toFixed(1) + 'K'
  return n.toLocaleString()
}
</script>

<template>
  <div class="page-content">
    <AppHeader title="仪表盘" subtitle="MosDNS 运行概览">
      <div class="status-badge">
        <span class="dot" :class="isRunning ? 'running' : 'stopped'"></span>
        <span :style="{ color: isRunning ? 'var(--accent-green-light)' : 'var(--accent-red-light)', fontSize: '12px' }">
          {{ isRunning ? '运行中' : '已停止' }}
        </span>
      </div>
    </AppHeader>

    <div v-if="warningMessage" class="warning-banner">{{ warningMessage }}</div>

    <div class="bento-grid">
      <!-- Trend chart - spans 2 rows -->
      <div class="bento-chart card">
        <div class="chart-header">
          <span class="chart-title">查询趋势</span>
          <span class="chart-live" v-if="initialized">
            <span class="live-dot"></span> 实时
          </span>
        </div>
        <TrendChart
          :timestamps="metrics.timestamps"
          :request-counts="metrics.requestCounts"
          :avg-latency-ms="metrics.avgLatencyMs"
        />
      </div>

      <!-- Stat cards -->
      <StatCard
        label="查询总量"
        :value="formatNumber(metrics.totalQueries)"
        color="var(--accent-blue-light)"
      />
      <StatCard
        label="缓存命中率"
        :value="metrics.averageLatency > 0 ? (100 - metrics.averageLatency / 10).toFixed(1) + '%' : '--'"
        color="var(--accent-purple-light)"
      />

      <!-- Bottom stats row -->
      <div class="bento-stats">
        <StatCard label="当前延迟" :value="metrics.currentLatency.toFixed(1) + 'ms'" color="var(--accent-green-light)" />
        <StatCard label="当前 QPS" :value="String(metrics.currentQueries)" color="var(--accent-amber-light)" />
        <StatCard label="平均延迟" :value="metrics.averageLatency.toFixed(1) + 'ms'" color="var(--accent-cyan-light)" />
        <StatCard label="运行状态" :value="isRunning ? '正常' : '异常'" :color="isRunning ? 'var(--accent-green-light)' : 'var(--accent-red-light)'" />
      </div>
    </div>
  </div>
</template>

<style scoped>
.status-badge {
  display: flex;
  align-items: center;
  gap: 6px;
}
.dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}
.dot.running {
  background: var(--accent-green);
  box-shadow: 0 0 6px var(--accent-green);
}
.dot.stopped {
  background: var(--accent-red);
}

.warning-banner {
  background: rgba(239, 68, 68, 0.1);
  border: 1px solid rgba(239, 68, 68, 0.3);
  border-radius: var(--radius-sm);
  padding: 8px 12px;
  color: var(--accent-red-light);
  font-size: 12px;
  margin-bottom: 16px;
}

.bento-grid {
  display: grid;
  grid-template-columns: 2fr 1fr;
  gap: 12px;
}

.bento-chart {
  grid-row: span 2;
  padding: 16px;
}
.chart-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 8px;
}
.chart-title {
  color: var(--accent-blue-light);
  font-size: 12px;
  font-weight: 600;
}
.chart-live {
  display: flex;
  align-items: center;
  gap: 4px;
  color: var(--accent-green-light);
  font-size: 11px;
}
.live-dot {
  width: 6px;
  height: 6px;
  background: var(--accent-green);
  border-radius: 50%;
  animation: pulse 2s infinite;
}
@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

.bento-stats {
  grid-column: span 2;
  display: flex;
  gap: 12px;
}
.bento-stats > * {
  flex: 1;
}
</style>
```

---

## Task 8: Logs Page

**Files:**
- Create: `mosdns-claude/webui-claude/src/views/Logs.vue`
- Create: `mosdns-claude/webui-claude/src/components/logs/LogTable.vue`

- [ ] **Step 1: Create LogTable.vue**

```vue
<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from 'vue'
import { getJSON } from '../../api/http'
import type { AuditLog, RankItem } from '../../types'

const logs = ref<AuditLog[]>([])
const page = ref(1)
const total = ref(0)
const limit = 50
const search = ref('')
const loading = ref(false)
const topDomains = ref<RankItem[]>([])
const topClients = ref<RankItem[]>([])
let pollTimer = 0

async function fetchLogs() {
  loading.value = true
  try {
    const params = new URLSearchParams({ page: String(page.value), limit: String(limit) })
    if (search.value) params.set('query_name', search.value)
    const result = await getJSON<any>('/api/v2/audit/logs?' + params)
    logs.value = result.data || result || []
    total.value = result.total || logs.value.length
  } catch (e) {
    console.error('Failed to fetch logs', e)
  } finally {
    loading.value = false
  }
}

async function fetchRanks() {
  try {
    const [domains, clients] = await Promise.allSettled([
      getJSON<any>('/api/v2/audit/rank/domain?limit=10'),
      getJSON<any>('/api/v2/audit/rank/client?limit=10')
    ])
    if (domains.status === 'fulfilled') topDomains.value = (domains.value.data || domains.value || []).slice(0, 10)
    if (clients.status === 'fulfilled') topClients.value = (clients.value.data || clients.value || []).slice(0, 10)
  } catch {}
}

const totalPages = computed(() => Math.max(1, Math.ceil(total.value / limit)))

function goToPage(p: number) {
  if (p < 1 || p > totalPages.value) return
  page.value = p
  fetchLogs()
}

onMounted(() => {
  fetchLogs()
  fetchRanks()
  pollTimer = window.setInterval(() => {
    if (page.value === 1 && !search.value) fetchLogs()
  }, 5000)
})
onBeforeUnmount(() => clearInterval(pollTimer))
</script>

<template>
  <div class="logs-page">
    <!-- Rank cards -->
    <div class="rank-row">
      <div class="card rank-card">
        <div class="rank-title" style="color: var(--accent-blue-light)">Top 域名</div>
        <div class="rank-list">
          <div v-for="(item, i) in topDomains" :key="i" class="rank-item">
            <span class="rank-num">{{ i + 1 }}</span>
            <span class="rank-name">{{ item.name }}</span>
            <span class="rank-count">{{ item.count }}</span>
          </div>
          <div v-if="topDomains.length === 0" class="rank-empty">暂无数据</div>
        </div>
      </div>
      <div class="card rank-card">
        <div class="rank-title" style="color: var(--accent-purple-light)">Top 客户端</div>
        <div class="rank-list">
          <div v-for="(item, i) in topClients" :key="i" class="rank-item">
            <span class="rank-num">{{ i + 1 }}</span>
            <span class="rank-name">{{ item.name }}</span>
            <span class="rank-count">{{ item.count }}</span>
          </div>
          <div v-if="topClients.length === 0" class="rank-empty">暂无数据</div>
        </div>
      </div>
    </div>

    <!-- Log table -->
    <div class="card" style="margin-top: 12px;">
      <div class="table-toolbar">
        <input class="input" v-model="search" placeholder="搜索域名..." @keyup.enter="page = 1; fetchLogs()" style="width: 260px;" />
        <button class="btn btn-sm" @click="fetchLogs()">刷新</button>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>客户端</th>
              <th>域名</th>
              <th>类型</th>
              <th>响应</th>
              <th>耗时</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="(log, i) in logs" :key="i">
              <td style="color: var(--text-secondary); white-space: nowrap;">{{ log.query_time }}</td>
              <td>{{ log.client_ip }}</td>
              <td style="color: var(--accent-blue-light); max-width: 300px; overflow: hidden; text-overflow: ellipsis;">{{ log.query_name }}</td>
              <td><span class="badge badge-blue">{{ log.query_type }}</span></td>
              <td><span :class="['badge', log.response_code === 'NOERROR' ? 'badge-green' : 'badge-red']">{{ log.response_code }}</span></td>
              <td style="font-variant-numeric: tabular-nums;">{{ log.duration_ms?.toFixed(1) }}ms</td>
            </tr>
            <tr v-if="logs.length === 0">
              <td colspan="6" style="text-align: center; color: var(--text-muted); padding: 32px;">{{ loading ? '加载中...' : '暂无日志' }}</td>
            </tr>
          </tbody>
        </table>
      </div>
      <div class="pagination">
        <button class="btn btn-sm" :disabled="page <= 1" @click="goToPage(page - 1)">上一页</button>
        <span style="color: var(--text-secondary); font-size: 12px;">{{ page }} / {{ totalPages }}</span>
        <button class="btn btn-sm" :disabled="page >= totalPages" @click="goToPage(page + 1)">下一页</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.rank-row {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 12px;
}
.rank-card {
  padding: 14px;
}
.rank-title {
  font-size: 12px;
  font-weight: 600;
  text-transform: uppercase;
  letter-spacing: 1px;
  margin-bottom: 10px;
}
.rank-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.rank-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 0;
  font-size: 12px;
}
.rank-num {
  width: 18px;
  height: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(59,130,246,0.15);
  border-radius: 4px;
  color: var(--accent-blue-light);
  font-size: 10px;
  font-weight: 600;
  flex-shrink: 0;
}
.rank-name {
  flex: 1;
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.rank-count {
  color: var(--text-secondary);
  font-variant-numeric: tabular-nums;
  font-size: 11px;
}
.rank-empty {
  color: var(--text-muted);
  font-size: 12px;
  text-align: center;
  padding: 16px;
}
.table-toolbar {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 12px;
}
</style>
```

- [ ] **Step 2: Create Logs.vue**

```vue
<script setup lang="ts">
import AppHeader from '../components/layout/AppHeader.vue'
import LogTable from '../components/logs/LogTable.vue'
</script>

<template>
  <div class="page-content">
    <AppHeader title="日志审计" subtitle="DNS 查询记录与统计排名" />
    <LogTable />
  </div>
</template>
```

---

## Task 9: Rules & Upstream Page

**Files:**
- Create: `mosdns-claude/webui-claude/src/views/Rules.vue`
- Create: `mosdns-claude/webui-claude/src/components/rules/RuleList.vue`
- Create: `mosdns-claude/webui-claude/src/components/rules/UpstreamCards.vue`

- [ ] **Step 1: Create RuleList.vue**

```vue
<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { getJSON, postJSON } from '../../api/http'

interface RuleCategory {
  tag: string
  name: string
  description: string
}

const categories: RuleCategory[] = [
  { tag: 'blocklist', name: '黑名单', description: '被拦截的域名' },
  { tag: 'whitelist', name: '白名单', description: '始终直连的域名' },
  { tag: 'greylist', name: '灰名单', description: '强制代理的域名' },
  { tag: 'ddnslist', name: 'DDNS 列表', description: 'DDNS 域名' },
  { tag: 'adguard', name: '广告拦截', description: 'AdGuardHome 规则' },
]

const selectedCategory = ref<string>('blocklist')
const rules = ref<string[]>([])
const newRule = ref('')
const loading = ref(false)
const saving = ref(false)

async function loadRules() {
  loading.value = true
  try {
    const cat = categories.find(c => c.tag === selectedCategory.value)
    if (!cat) return
    const result = await getJSON<any>(`/plugins/${cat.tag}/list`)
    rules.value = Array.isArray(result) ? result : result?.data || []
  } catch (e) {
    rules.value = []
  } finally {
    loading.value = false
  }
}

async function addRule() {
  if (!newRule.value.trim()) return
  saving.value = true
  try {
    await postJSON(`/plugins/${selectedCategory.value}/post`, { data: newRule.value.trim() })
    newRule.value = ''
    await loadRules()
  } catch (e) {
    alert('添加失败: ' + (e instanceof Error ? e.message : String(e)))
  } finally {
    saving.value = false
  }
}

async function removeRule(rule: string) {
  if (!confirm(`确定删除 "${rule}" ?`)) return
  try {
    await postJSON(`/plugins/${selectedCategory.value}/delete`, { data: rule })
    await loadRules()
  } catch (e) {
    alert('删除失败: ' + (e instanceof Error ? e.message : String(e)))
  }
}

onMounted(loadRules)
</script>

<template>
  <div class="rules-layout">
    <!-- Left: category list -->
    <div class="rules-sidebar">
      <div
        v-for="cat in categories"
        :key="cat.tag"
        class="rule-cat"
        :class="{ active: selectedCategory === cat.tag }"
        @click="selectedCategory = cat.tag; loadRules()"
      >
        <div class="cat-name">{{ cat.name }}</div>
        <div class="cat-desc">{{ cat.description }}</div>
      </div>
    </div>

    <!-- Right: rule content -->
    <div class="rules-content card">
      <div class="rules-toolbar">
        <input class="input" v-model="newRule" placeholder="输入域名，如 example.com" @keyup.enter="addRule()" style="flex: 1;" />
        <button class="btn btn-primary btn-sm" @click="addRule()" :disabled="saving">添加</button>
      </div>
      <div class="rules-list">
        <div v-if="loading" class="rules-empty">加载中...</div>
        <div v-else-if="rules.length === 0" class="rules-empty">暂无规则</div>
        <div v-for="(rule, i) in rules" :key="i" class="rule-item">
          <span class="rule-text">{{ rule }}</span>
          <button class="btn btn-sm" style="color: var(--accent-red-light); border-color: rgba(239,68,68,0.3);" @click="removeRule(rule)">删除</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.rules-layout {
  display: grid;
  grid-template-columns: 220px 1fr;
  gap: 12px;
  height: calc(100vh - 120px);
}
.rules-sidebar {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.rule-cat {
  padding: 10px 12px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  transition: all var(--transition-fast);
  border-left: 3px solid transparent;
}
.rule-cat:hover {
  background: var(--bg-card-hover);
}
.rule-cat.active {
  background: rgba(59,130,246,0.12);
  border-left-color: var(--accent-blue);
}
.cat-name {
  font-size: 13px;
  font-weight: 500;
  color: var(--text-primary);
}
.cat-desc {
  font-size: 11px;
  color: var(--text-muted);
  margin-top: 2px;
}
.rules-content {
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.rules-toolbar {
  display: flex;
  gap: 8px;
  margin-bottom: 12px;
}
.rules-list {
  flex: 1;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.rule-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 6px 8px;
  border-radius: 4px;
  font-size: 13px;
  transition: background var(--transition-fast);
}
.rule-item:hover {
  background: rgba(255,255,255,0.04);
}
.rule-text {
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.rules-empty {
  color: var(--text-muted);
  text-align: center;
  padding: 32px;
  font-size: 13px;
}
</style>
```

- [ ] **Step 2: Create UpstreamCards.vue**

```vue
<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { getJSON, postJSON } from '../../api/http'

interface Upstream {
  tag: string
  addr: string
  protocol?: string
  enabled?: boolean
}

const groups = ref<{ name: string; tag: string; upstreams: Upstream[] }[]>([])
const loading = ref(false)

const GROUP_MAP: Record<string, string> = {
  domestic: '国内 DNS',
  foreign: '国外 DNS',
  foreignecs: '国外 DNS + ECS',
  nocnfake: '非国内 FakeIP',
  cnfake: '国内 FakeIP',
}

async function loadUpstreams() {
  loading.value = true
  try {
    const config = await getJSON<any>('/api/v1/upstream/config')
    const result: typeof groups.value = []
    if (config && typeof config === 'object') {
      for (const [tag, data] of Object.entries(config)) {
        const name = GROUP_MAP[tag] || tag
        const upstreams = Array.isArray(data) ? data as Upstream[] : []
        result.push({ name, tag, upstreams })
      }
    }
    groups.value = result
  } catch (e) {
    console.error('Failed to load upstreams', e)
  } finally {
    loading.value = false
  }
}

async function saveUpstream(group: string) {
  try {
    await postJSON('/api/v1/upstream/config', { [group]: groups.value.find(g => g.tag === group)?.upstreams })
    alert('保存成功')
  } catch (e) {
    alert('保存失败: ' + (e instanceof Error ? e.message : String(e)))
  }
}

onMounted(loadUpstreams)
</script>

<template>
  <div class="upstream-section">
    <div v-if="loading" style="color: var(--text-muted); text-align: center; padding: 32px;">加载中...</div>
    <div v-else class="upstream-grid">
      <div v-for="group in groups" :key="group.tag" class="card upstream-card">
        <div class="upstream-header">
          <span class="upstream-name">{{ group.name }}</span>
          <span class="badge badge-blue">{{ group.tag }}</span>
        </div>
        <div class="upstream-list">
          <div v-for="(up, i) in group.upstreams" :key="i" class="upstream-item">
            <span class="up-addr">{{ up.addr }}</span>
            <span v-if="up.protocol" class="badge badge-amber" style="font-size: 10px;">{{ up.protocol }}</span>
          </div>
          <div v-if="group.upstreams.length === 0" style="color: var(--text-muted); font-size: 12px; padding: 8px 0;">无上游配置</div>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.upstream-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(280px, 1fr));
  gap: 12px;
}
.upstream-card {
  padding: 14px;
}
.upstream-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 10px;
}
.upstream-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}
.upstream-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.upstream-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 4px 0;
  font-size: 13px;
}
.up-addr {
  color: var(--accent-blue-light);
  font-family: 'IBM Plex Mono', monospace;
  font-size: 12px;
}
</style>
```

- [ ] **Step 3: Create Rules.vue**

```vue
<script setup lang="ts">
import AppHeader from '../components/layout/AppHeader.vue'
import RuleList from '../components/rules/RuleList.vue'
import UpstreamCards from '../components/rules/UpstreamCards.vue'
</script>

<template>
  <div class="page-content">
    <AppHeader title="规则与上游" subtitle="域名规则管理与上游 DNS 配置" />

    <h3 style="color: var(--accent-blue-light); font-size: 13px; font-weight: 600; margin-bottom: 12px; text-transform: uppercase; letter-spacing: 1px;">上游 DNS</h3>
    <UpstreamCards />

    <h3 style="color: var(--accent-purple-light); font-size: 13px; font-weight: 600; margin: 20px 0 12px; text-transform: uppercase; letter-spacing: 1px;">域名规则</h3>
    <RuleList />
  </div>
</template>
```

---

## Task 10: System & Cache Page

**Files:**
- Create: `mosdns-claude/webui-claude/src/views/System.vue`
- Create: `mosdns-claude/webui-claude/src/components/cache/CacheCard.vue`

- [ ] **Step 1: Create CacheCard.vue**

```vue
<script setup lang="ts">
import { ref } from 'vue'
import { postJSON } from '../../api/http'

const props = defineProps<{
  tag: string
  name: string
}>()

const loading = ref(false)
const message = ref('')

async function flush() {
  if (!confirm(`确定清空 "${props.name}" 缓存?`)) return
  loading.value = true
  try {
    await postJSON(`/plugins/${props.tag}/flush`)
    message.value = '已清空'
  } catch (e) {
    message.value = '失败: ' + (e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
    setTimeout(() => message.value = '', 3000)
  }
}

async function save() {
  loading.value = true
  try {
    await postJSON(`/plugins/${props.tag}/save`)
    message.value = '已保存'
  } catch (e) {
    message.value = '失败: ' + (e instanceof Error ? e.message : String(e))
  } finally {
    loading.value = false
    setTimeout(() => message.value = '', 3000)
  }
}
</script>

<template>
  <div class="card cache-card">
    <div class="cache-name">{{ name }}</div>
    <div class="cache-tag">{{ tag }}</div>
    <div class="cache-actions">
      <button class="btn btn-sm" @click="save()" :disabled="loading">保存</button>
      <button class="btn btn-sm" style="color: var(--accent-red-light); border-color: rgba(239,68,68,0.3);" @click="flush()" :disabled="loading">清空</button>
    </div>
    <div v-if="message" class="cache-msg">{{ message }}</div>
  </div>
</template>

<style scoped>
.cache-card {
  padding: 14px;
}
.cache-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
}
.cache-tag {
  font-size: 11px;
  color: var(--text-muted);
  font-family: 'IBM Plex Mono', monospace;
  margin: 2px 0 10px;
}
.cache-actions {
  display: flex;
  gap: 6px;
}
.cache-msg {
  font-size: 11px;
  color: var(--accent-green-light);
  margin-top: 6px;
}
</style>
```

- [ ] **Step 2: Create System.vue**

```vue
<script setup lang="ts">
import { ref } from 'vue'
import AppHeader from '../components/layout/AppHeader.vue'
import CacheCard from '../components/cache/CacheCard.vue'
import { postJSON } from '../api/http'
import { CACHE_TAGS } from '../types'

const restarting = ref(false)

async function restart() {
  if (!confirm('确定要重启 mosdns 服务吗?')) return
  restarting.value = true
  try {
    await postJSON('/api/v1/system/restart')
    alert('重启指令已发送')
  } catch (e) {
    alert('重启失败: ' + (e instanceof Error ? e.message : String(e)))
  } finally {
    restarting.value = false
  }
}

async function checkUpdate() {
  try {
    const result = await postJSON<any>('/api/v1/update/check')
    alert(result?.message || JSON.stringify(result))
  } catch (e) {
    alert('检查更新失败: ' + (e instanceof Error ? e.message : String(e)))
  }
}

async function exportConfig() {
  try {
    const res = await fetch('/api/v1/config/export', { method: 'POST' })
    const blob = await res.blob()
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = 'mosdns-config.zip'
    a.click()
    URL.revokeObjectURL(url)
  } catch (e) {
    alert('导出失败: ' + (e instanceof Error ? e.message : String(e)))
  }
}
</script>

<template>
  <div class="page-content">
    <AppHeader title="系统与缓存" subtitle="缓存管理、系统控制与配置" />

    <!-- Cache section -->
    <h3 style="color: var(--accent-blue-light); font-size: 13px; font-weight: 600; margin-bottom: 12px; text-transform: uppercase; letter-spacing: 1px;">缓存管理</h3>
    <div class="cache-grid">
      <CacheCard v-for="c in CACHE_TAGS" :key="c.tag" :tag="c.tag" :name="c.name" />
    </div>

    <!-- System controls -->
    <h3 style="color: var(--accent-purple-light); font-size: 13px; font-weight: 600; margin: 24px 0 12px; text-transform: uppercase; letter-spacing: 1px;">系统控制</h3>
    <div class="system-grid">
      <div class="card sys-card">
        <div class="sys-title">服务控制</div>
        <div class="sys-desc">重启 mosdns 服务进程</div>
        <button class="btn btn-primary btn-sm" @click="restart()" :disabled="restarting">重启服务</button>
      </div>
      <div class="card sys-card">
        <div class="sys-title">更新检查</div>
        <div class="sys-desc">检查 GitHub 最新版本</div>
        <button class="btn btn-sm" @click="checkUpdate()">检查更新</button>
      </div>
      <div class="card sys-card">
        <div class="sys-title">配置导出</div>
        <div class="sys-desc">下载完整配置 ZIP 包</div>
        <button class="btn btn-sm" @click="exportConfig()">导出配置</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.cache-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 12px;
}
.system-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(240px, 1fr));
  gap: 12px;
}
.sys-card {
  padding: 16px;
}
.sys-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: 4px;
}
.sys-desc {
  font-size: 12px;
  color: var(--text-muted);
  margin-bottom: 12px;
}
</style>
```

---

## Task 11: Settings Page (Switch Grid)

**Files:**
- Create: `mosdns-claude/webui-claude/src/views/Settings.vue`
- Create: `mosdns-claude/webui-claude/src/components/settings/SwitchGrid.vue`

- [ ] **Step 1: Create SwitchGrid.vue**

```vue
<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { postJSON } from '../../api/http'
import { SWITCH_DEFINITIONS } from '../../types'

interface SwitchState {
  tag: string
  name: string
  description: string
  value: 'A' | 'B'
  labels: { A: string; B: string }
}

const switches = ref<SwitchState[]>([])
const loading = ref<Record<string, boolean>>({})

async function loadSwitches() {
  switches.value = SWITCH_DEFINITIONS.map(def => ({
    ...def,
    value: 'B' as const // default, will be updated
  }))
  // Load current values from rule files via plugin API
  for (const sw of switches.value) {
    try {
      const result = await fetch(`/plugins/${sw.tag}/list`)
      if (result.ok) {
        const data = await result.json()
        const val = Array.isArray(data) ? data[0] : data?.value || data?.data?.[0]
        if (val === 'A' || val === 'B') sw.value = val
      }
    } catch {}
  }
}

async function toggleSwitch(sw: SwitchState) {
  const newValue = sw.value === 'A' ? 'B' : 'A'
  loading.value[sw.tag] = true
  try {
    await postJSON(`/plugins/${sw.tag}/post`, { value: newValue })
    sw.value = newValue
  } catch (e) {
    alert(`切换失败: ${e instanceof Error ? e.message : String(e)}`)
  } finally {
    loading.value[sw.tag] = false
  }
}

onMounted(loadSwitches)
</script>

<template>
  <div class="switch-grid">
    <div
      v-for="sw in switches"
      :key="sw.tag"
      class="switch-card"
      :class="{ active: sw.value === 'A' }"
      @click="toggleSwitch(sw)"
    >
      <div class="switch-info">
        <div class="switch-name">{{ sw.name }}</div>
        <div class="switch-desc">{{ sw.description }}</div>
        <div class="switch-status">
          <span :class="sw.value === 'A' ? 'status-on' : 'status-off'">
            {{ sw.value === 'A' ? sw.labels.A : sw.labels.B }}
          </span>
        </div>
      </div>
      <div
        class="toggle"
        :class="{ active: sw.value === 'A', loading: loading[sw.tag] }"
      ></div>
    </div>
  </div>
</template>

<style scoped>
.switch-grid {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 10px;
}

@media (max-width: 900px) {
  .switch-grid {
    grid-template-columns: repeat(2, 1fr);
  }
}

.switch-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 14px 16px;
  background: var(--bg-card);
  border: 1px solid var(--border-primary);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
  gap: 12px;
}
.switch-card:hover {
  border-color: var(--border-accent);
  background: var(--bg-card-hover);
}
.switch-card.active {
  border-color: rgba(59, 130, 246, 0.4);
  background: rgba(59, 130, 246, 0.1);
}

.switch-info {
  flex: 1;
  min-width: 0;
}
.switch-name {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-primary);
  margin-bottom: 2px;
}
.switch-desc {
  font-size: 11px;
  color: var(--text-muted);
  margin-bottom: 4px;
}
.switch-status {
  font-size: 11px;
}
.status-on {
  color: var(--accent-blue-light);
  font-weight: 500;
}
.status-off {
  color: var(--text-muted);
}

.toggle {
  position: relative;
  width: 40px;
  height: 22px;
  border-radius: 11px;
  background: #444;
  transition: background var(--transition-fast);
  flex-shrink: 0;
}
.toggle.active {
  background: var(--accent-blue);
}
.toggle::after {
  content: '';
  position: absolute;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #fff;
  top: 2px;
  left: 2px;
  transition: transform var(--transition-fast);
}
.toggle.active::after {
  transform: translateX(18px);
}
.toggle.loading {
  opacity: 0.5;
}
</style>
```

- [ ] **Step 2: Create Settings.vue**

```vue
<script setup lang="ts">
import AppHeader from '../components/layout/AppHeader.vue'
import SwitchGrid from '../components/settings/SwitchGrid.vue'
</script>

<template>
  <div class="page-content">
    <AppHeader title="设置" subtitle="运行时开关与外观配置" />

    <h3 style="color: var(--accent-blue-light); font-size: 13px; font-weight: 600; margin-bottom: 12px; text-transform: uppercase; letter-spacing: 1px;">运行时开关</h3>
    <SwitchGrid />
  </div>
</template>
```

---

## Task 12: Go Backend Modification

**Files:**
- Modify: `mosdns-claude/coremain/mosdns.go` (line ~283: change rootHandler to serve index.html)

- [ ] **Step 1: Modify rootHandler to serve new UI**

In `coremain/mosdns.go`, change the `rootHandler` function (around line 283) from reading `www/log.html` to reading `www/index.html`:

```go
rootHandler := func(w http.ResponseWriter, r *http.Request) {
    data, err := content.ReadFile("www/index.html")
    if err != nil {
        m.logger.Error("Error reading embedded file", zap.String("file", "www/index.html"), zap.Error(err))
        http.Error(w, "Error reading the embedded file", http.StatusInternalServerError)
        return
    }
    w.Header().Set("Content-Type", "text/html; charset=utf-8")
    if _, err := w.Write(data); err != nil {
        m.logger.Error("Error writing response", zap.Error(err))
    }
}
```

Also change the static asset handler to also serve `vue-claude` assets (the existing handler already handles `/assets/*` which covers `assets/vue-claude/`).

---

## Task 13: Build and Deploy

**Files:**
- Build: `mosdns-claude/webui-claude/` → `mosdns-claude/coremain/www/assets/vue-claude/`
- Build: `mosdns-claude/` → `mosdns-claude/mosdns` binary

- [ ] **Step 1: Build Vue frontend**

```bash
cd /Users/tom/Documents/github/mosdns-claude/webui-claude
npm run build
```

Expected: Files `app.js` and `app.css` appear in `../coremain/www/assets/vue-claude/`

- [ ] **Step 2: Build Go binary**

```bash
cd /Users/tom/Documents/github/mosdns-claude
CGO_ENABLED=0 go build -trimpath -ldflags "-s -w" -o mosdns ./
```

Expected: Binary `mosdns` created in project root

- [ ] **Step 3: Deploy to 10.0.0.91**

```bash
# Stop running mosdns
ssh root@10.0.0.91 "pkill mosdns || true"
# Wait for process to stop
sleep 2
# Copy new binary
scp /Users/tom/Documents/github/mosdns-claude/mosdns root@10.0.0.91:/usr/local/bin/mosdns
# Start mosdns (background)
ssh root@10.0.0.91 "nohup mosdns -c /path/to/config > /dev/null 2>&1 &"
```

Note: The actual binary path and config path on the remote server need to be verified. The existing running mosdns process path should be checked before deployment.

---

## Task 14: Visual Verification

- [ ] **Step 1: Open browser and verify**

Open `http://10.0.0.91:9099` in browser and verify:
1. Dark blue-purple theme loads correctly
2. Sidebar navigation works (all 5 pages)
3. Dashboard shows ECharts trend chart with live data
4. Logs page shows query records in table
5. Rules page shows left-right split with categories
6. System page shows cache cards
7. Settings page shows 15 switch cards in 3-column grid
8. Toggle switches work and persist
9. All API calls succeed (check browser console for errors)

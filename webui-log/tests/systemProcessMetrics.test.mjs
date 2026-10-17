import test from 'node:test'
import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'

import { requestOperation, validateRuntimeCapabilities } from '../src/api/runtimeCapabilities.js'

const read = (path) => readFileSync(new URL(path, import.meta.url), 'utf8')

test('the four F-1 operations are gated by the paths the native host serves', () => {
  assert.equal(requestOperation('/api/v1/appearance/panel-background/upload', 'POST'), 'appearance.server')
  assert.equal(requestOperation('/api/v1/appearance/text-color', 'GET'), 'appearance.server')
  assert.equal(requestOperation('/api/v1/system/webui-port', 'POST'), 'system.webui_port')
  assert.equal(requestOperation('/api/v1/system/restart', 'POST'), 'system.restart')
  // The process figures travel on the metrics endpoint the cache panel already uses.
  assert.equal(requestOperation('/metrics', 'GET'), 'metrics.cache')
})

test('a native peer that reports the operations supported enables them without reasons', () => {
  const ids = ['appearance.server', 'system.restart', 'system.webui_port', 'metrics.process']
  const value = validateRuntimeCapabilities({
    schema_version: 1,
    runtime: 'rust',
    special_groups: { enabled: false },
    upstream_protocols: [],
    rule_formats: [],
    unsupported_features: [],
    endpoints: {},
    ui_operations: Object.fromEntries([
      ...['system.health', 'system.version', 'audit.read', 'audit.control', 'audit.capacity', 'query.rank', 'cache.inventory', 'cache.manage',
        'metrics.cache', 'rules.local.read', 'rules.local.manage', 'groups.read', 'groups.manage', 'groups.upstreams', 'upstreams.read',
        'upstreams.manage', 'rules.diversion', 'rules.adguard', 'capture.logs', 'client.aliases', 'switches.manage', 'cache.requery',
        'lists.remembered', 'system.config_management', 'system.update', 'system.domain_generation', 'system.global_overrides']
        .map((id) => [id, { supported: false, reason: 'n/a' }]),
      ...ids.map((id) => [id, { supported: true, reason: null }])
    ])
  })
  for (const id of ids) assert.deepEqual(value.ui_operations[id], { supported: true, reason: null })
})

test('the native overview and system panel show the process figures the Rust runtime has and mark the Go ones', () => {
  const overview = read('../src/components/OverviewManager.vue')
  const panel = read('../src/components/system/SystemInfoPanel.vue')
  const control = read('../src/components/SystemControlManager.vue')
  // Both metric parsers read the native thread gauge next to the Go one.
  assert.match(overview, /startsWith\('go_threads'\) \|\| line\.startsWith\('mosdns_process_threads'\)/)
  assert.match(control, /startsWith\("go_threads"\) \|\| line\.startsWith\("mosdns_process_threads"\)/)
  // Supported: real cards; not applicable: one honest card, no invented numbers.
  assert.match(overview, /supportsOperation\('metrics\.process'\)/)
  assert.match(overview, /不适用于 Rust 运行时/)
  assert.match(panel, /不适用于 Rust 运行时/)
  const native = overview.slice(overview.indexOf("capabilityState.value?.kind === 'native'"), overview.indexOf("title: '启动时间'"))
  assert.doesNotMatch(native, /gcCount|goVersion|goroutines/, 'no Go-runtime value is shown for the native runtime')
})

test('the WebUI port panel is not hidden on the native runtime (only the Go-specific mode panels are)', () => {
  const control = read('../src/components/SystemControlManager.vue')
  const stack = control.slice(control.indexOf('class="system-mini-stack"') - 40, control.indexOf('<SystemWebuiPortPanel'))
  assert.doesNotMatch(stack.slice(0, stack.indexOf('system-mini-stack') + 20), /v-if=/, 'the stack that holds the port panel renders on native')
  assert.match(control, /<CapabilityBoundary v-if="capabilityState\.value\?\.kind !== 'native'" operation="switches\.manage">\s*<SystemCoreModePanel/)
  assert.match(control, /<CapabilityBoundary operation="system\.webui_port">\s*<SystemWebuiPortPanel/)
})

test('the compatibility shell is untouched by the F-1 work', () => {
  const log1 = read('../src-log1/SystemControlManager.vue')
  assert.doesNotMatch(log1, /mosdns_process_threads/)
})

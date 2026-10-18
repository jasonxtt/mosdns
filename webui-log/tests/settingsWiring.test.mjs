import test from 'node:test'
import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'

import { isNativeUpstreamReadOnly } from '../src/api/nativeManagement.js'
import { requestOperation } from '../src/api/runtimeCapabilities.js'
import {
  isModified,
  nativeProxyFlag,
  protocolUsesProxy,
  settingsBackedProxy,
  settingsSources
} from '../src/utils/nativeSettings.js'

const settingsBacked = {
  kind: 'native',
  upstream_protocols: ['udp', 'tcp', 'dot', 'doh'],
  unsupported_features: ['signed_aliapi', 'quic', 'http3', 'socket_marks', 'bind_to_device', 'positive_idle_timeout', 'enable_pipeline']
}
const managed = { ...settingsBacked, unsupported_features: [...settingsBacked.unsupported_features, 'socks_proxy'] }

test('the proxy switch is carried only by a runtime that does not list socks_proxy as unsupported', () => {
  assert.equal(settingsBackedProxy(settingsBacked), true)
  assert.equal(settingsBackedProxy(managed), false)
  assert.equal(settingsBackedProxy({ kind: 'legacy' }), false)
  assert.equal(settingsBackedProxy(undefined), false)
  assert.equal(settingsBackedProxy({ kind: 'native' }), false, 'no feature list means no claim')
})

test('a proxied entry is editable on a settings-backed runtime, a literal socks5 is not', () => {
  const entry = { tag: 'google', protocol: 'https', enabled: true, addr: 'https://dns.google/dns-query', use_socks_proxy: true }
  assert.deepEqual(isNativeUpstreamReadOnly(entry, settingsBacked), { readOnly: false, reason: '' })
  assert.equal(isNativeUpstreamReadOnly(entry, managed).readOnly, true, 'the managed runtime keeps it read-only')
  assert.equal(isNativeUpstreamReadOnly({ ...entry, socks5: '10.0.0.1:1080' }, settingsBacked).readOnly, true)
  assert.equal(isNativeUpstreamReadOnly({ ...entry, use_socks_proxy: false, so_mark: 3 }, settingsBacked).readOnly, true)
})

test('only protocols that can use a proxy carry a checked switch', () => {
  for (const protocol of ['tcp', 'tls', 'dot', 'https', 'doh', 'HTTPS']) {
    assert.equal(protocolUsesProxy(protocol), true, protocol)
    assert.equal(nativeProxyFlag(protocol, true), true)
    assert.equal(nativeProxyFlag(protocol, false), false)
  }
  for (const protocol of ['udp', 'aliapi', '', undefined]) assert.equal(nativeProxyFlag(protocol, true), false, String(protocol))
})

test('effective items become the modified marks of the pages', () => {
  const marks = settingsSources({
    items: [
      { object: 'main_proxy', kind: 'socks5_proxy', field: 'addr', value: '10.0.0.1:1080', source: 'settings' },
      { object: 'client_ecs', kind: 'ecs_address', field: 'addr', value: '2408:8888::8', source: 'yaml' },
      { object: 'foreign', kind: 'aliapi', field: 'upstreams', value: 'a=udp://1.1.1.1', source: 'settings' },
      { object: 'domestic', kind: 'aliapi', field: 'upstreams', value: 'b=udp://2.2.2.2', source: 'yaml' },
      { object: 'my_realiplist', kind: 'domain_output', field: 'enabled', value: 'true', source: 'yaml' },
      null
    ]
  })
  assert.deepEqual(marks, { socks5: 'settings', ecs: 'yaml', upstreams: { foreign: 'settings', domestic: 'yaml' } })
  assert.equal(isModified(marks.socks5), true)
  assert.equal(isModified(marks.ecs), false)
  assert.deepEqual(settingsSources(undefined), { socks5: '', ecs: '', upstreams: {} })
  assert.deepEqual(settingsSources({ items: 'x' }), { socks5: '', ecs: '', upstreams: {} })
})

test('the effective-settings read is a plain, ungated request', () => {
  assert.equal(requestOperation('/api/v1/settings/effective', 'GET'), null)
  assert.equal(requestOperation('/api/v1/overrides', 'POST'), 'system.global_overrides')
  assert.equal(requestOperation('/api/v1/upstream/config', 'POST'), 'upstreams.manage')
})

const sources = [
  new URL('../src/components/SystemControlManager.vue', import.meta.url)
]

test('the shell hides the replacement rules, apply overrides at once and offer restore only on the native runtime', async () => {
  for (const path of sources) {
    const source = await readFile(path, 'utf8')
    // The replacement panel and its toggle exist only outside the native runtime.
    assert.match(source, /expandedAdvanced\.replacementRules && !nativeRuntime/, path.pathname)
    assert.match(source, /v-if="!nativeRuntime" class="system-advanced-toggle-row"/, path.pathname)
    assert.match(source, /data-native-replacements-note/, path.pathname)
    // Native saves send no replacements, never ask for a restart and can restore a field.
    const save = source.slice(source.indexOf('async function saveOverrides()'))
    const nativeBranch = save.slice(0, save.indexOf('const replacements = overrides.replacements'))
    assert.match(nativeBranch, /if \(nativeRuntime\.value\)/, path.pathname)
    assert.match(nativeBranch, /replacements: \[\]/, path.pathname)
    assert.doesNotMatch(nativeBranch, /system\/restart/, path.pathname)
    assert.match(source, /async function restoreOverride\(field\)/, path.pathname)
    assert.match(source, /@restore-override="restoreOverride"/, path.pathname)
    // The Go flow (restart prompt) is still there for the other runtime.
    assert.match(save, /覆盖配置已保存，是否立即重启 MosDNS 使其生效/, path.pathname)
  }
})

test('the upstream page offers the proxy switch for every group and never a literal address on a settings-backed runtime', async () => {
  const source = await readFile(new URL('../src/components/UpstreamManager.vue', import.meta.url), 'utf8')
  assert.match(source, /showSocksProxyToggle = computed\(\(\) => showSocks5\.value && \(settingsProxy\.value \|\|/)
  assert.match(source, /showSocks5Input = computed\(\(\) => !settingsProxy\.value &&/)
  assert.match(source, /nativeProxyFlag\(protocol, form\.use_socks_proxy\)/)
  assert.match(source, /if \(settingsProxy\.value\) form\.use_socks_proxy = false/, 'a new native entry starts without the proxy')
  assert.match(source, /plugin_tag: group, upstreams: \[\]/, 'restore default posts an empty list')
  // The Go path of the same page is unchanged: only foreign groups get the toggle, with a literal address.
  assert.match(source, /\['foreign', 'foreignecs'\]\.includes\(String\(form\.group \|\| ''\)\.trim\(\)\)/)
})

import test from 'node:test'
import assert from 'node:assert/strict'

import {
  createRuntimeCapabilityClient,
  validateRuntimeCapabilities
} from '../src/api/runtimeCapabilities.js'
import {
  buildNativeDiversionPayload,
  classifyDiversionSource,
  isNativeUpstreamReadOnly,
  loadLegacyDiversionCatalogs,
  validateDiversionCatalogChange
} from '../src/api/nativeManagement.js'

const nativeCapabilities = {
  schema_version: 1,
  runtime: 'rust',
  special_groups: { enabled: true, profile: 'local_text_dns_v1' },
  upstream_protocols: ['udp', 'tcp', 'dot', 'doh'],
  rule_formats: ['local_text'],
  unsupported_features: ['remote_download', 'auto_update', 'quic', 'http3', 'socks_proxy'],
  endpoints: {
    special_groups: { get: true, post: true, delete: true },
    upstream: { tags_get: true, config_get: true, config_post: true },
    diversion_sources: { list_get: true, put: true, delete: true },
    manual_rules: { show_get: true, save_get: true, post: true }
  }
}

test('runtime capability discovery caches native response for the page session', async () => {
  let calls = 0
  const client = createRuntimeCapabilityClient(async () => {
    calls += 1
    return nativeCapabilities
  })

  assert.deepEqual(await client(), { kind: 'native', ...validateRuntimeCapabilities(nativeCapabilities) })
  assert.deepEqual(await client(), { kind: 'native', ...validateRuntimeCapabilities(nativeCapabilities) })
  assert.equal(calls, 1)
})

test('only capability endpoint 404 selects the legacy Go workflow', async () => {
  const client = createRuntimeCapabilityClient(async () => {
    throw Object.assign(new Error('not found'), { status: 404 })
  })
  assert.deepEqual(await client(), { kind: 'legacy' })
})

test('legacy capability fallback retains successful diversion catalogs when an optional catalog is 404', async () => {
  const getCapabilities = createRuntimeCapabilityClient(async () => {
    throw Object.assign(new Error('not found'), { status: 404 })
  })
  const capabilities = await getCapabilities()
  assert.equal(capabilities.kind, 'legacy')

  const entries = [
    ['special_50', 'diversion_50'],
    ['geoipcn', 'geoip_cn']
  ]
  const result = await loadLegacyDiversionCatalogs(entries, async (path) => {
    if (path === '/plugins/diversion_50/config') {
      throw Object.assign(new Error('HTTP 404 Not Found'), { status: 404 })
    }
    return [{ name: 'kept-rule.txt', type: 'geoipcn', enabled: true }]
  })

  assert.deepEqual(result.catalogs, [{
    type: 'geoipcn',
    tag: 'geoip_cn',
    rules: [{ name: 'kept-rule.txt', type: 'geoipcn', enabled: true }]
  }])
  assert.deepEqual(result.failures, [{
    type: 'special_50',
    tag: 'diversion_50',
    status: 404,
    error: 'HTTP 404 Not Found'
  }])
})

test('network and server errors are surfaced and can be retried', async () => {
  let calls = 0
  const client = createRuntimeCapabilityClient(async () => {
    calls += 1
    if (calls === 1) throw Object.assign(new Error('gateway unavailable'), { status: 503 })
    return nativeCapabilities
  })

  await assert.rejects(client(), /gateway unavailable/)
  assert.equal((await client()).kind, 'native')
  assert.equal(calls, 2)
})

test('malformed capability responses fail closed', () => {
  assert.throws(() => validateRuntimeCapabilities({ runtime: 'rust' }), /capability response/i)
})

test('native source editor accepts only supported local text records', () => {
  const supported = {
    name: 'block.txt', type: 'special_50', enabled: true,
    files: 'rules/block.txt', url: '', auto_update: false, enable_regexp: false
  }
  assert.deepEqual(classifyDiversionSource(supported, 'special_50'), { readOnly: false, reason: '' })
  assert.match(classifyDiversionSource({ ...supported, files: 'rules/block.srs' }, 'special_50').reason, /\.txt/)
  assert.equal(classifyDiversionSource({ ...supported, auto_update: true }, 'special_50').readOnly, true)
  assert.equal(classifyDiversionSource({ ...supported, extra: { retained: true }, enabled: false }, 'special_50').readOnly, true)
})

test('native source payload excludes downloader and advanced-format state', () => {
  assert.deepEqual(buildNativeDiversionPayload({
    name: 'new-list', type: 'special_50', files: 'rules/new-list.txt', enabled: true
  }), {
    name: 'new-list', type: 'special_50', files: 'rules/new-list.txt', enabled: true,
    url: '', auto_update: false, enable_regexp: false
  })
})

test('native source edits cannot move ownership across catalogs', () => {
  assert.throws(() => validateDiversionCatalogChange('special_route_50', 'special_route_51'), /另一个组/)
  assert.doesNotThrow(() => validateDiversionCatalogChange('special_route_50', 'special_route_50'))
})

test('unsupported upstreams stay read-only and visible', () => {
  const capabilities = nativeCapabilities
  assert.equal(isNativeUpstreamReadOnly({ protocol: 'quic', enabled: false }, capabilities).readOnly, true)
  assert.equal(isNativeUpstreamReadOnly({ protocol: 'udp', enabled: false, so_mark: 42 }, capabilities).readOnly, true)
  assert.deepEqual(isNativeUpstreamReadOnly({
    tag: 'local', protocol: 'udp', enabled: true, addr: '127.0.0.1:5300'
  }, capabilities), { readOnly: false, reason: '' })
})

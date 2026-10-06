import { reactive } from 'vue'

export const OPERATION_IDS = Object.freeze([
  'system.health', 'system.version', 'audit.read', 'audit.control', 'audit.capacity',
  'query.rank', 'cache.inventory', 'cache.manage', 'metrics.cache', 'rules.local.read',
  'rules.local.manage', 'groups.read', 'groups.manage', 'upstreams.read', 'upstreams.manage',
  'rules.diversion', 'rules.adguard', 'capture.logs', 'client.aliases', 'switches.manage',
  'cache.requery', 'lists.remembered', 'appearance.server', 'system.restart', 'system.webui_port',
  'system.config_management', 'system.update', 'system.domain_generation', 'system.global_overrides', 'metrics.process'
])
function isObject(value) { return value !== null && typeof value === 'object' && !Array.isArray(value) }
const unimplemented = new Set(['rules.adguard','capture.logs','client.aliases','switches.manage','cache.requery','lists.remembered','appearance.server','system.restart','system.webui_port','system.config_management','system.update','system.domain_generation','system.global_overrides','metrics.process'])

function validateSwitches(value) {
  if (!isObject(value) || value.schema_version !== 1 || typeof value.config_generation !== 'string' || !/^(?:0|[1-9][0-9]*)$/.test(value.config_generation) || !Array.isArray(value.instances)) {
    throw new Error('invalid runtime switch capability response')
  }
  const types = new Set()
  const tags = new Set()
  for (const instance of value.instances) {
    if (!isObject(instance) || !/^switch(?:[1-9]|1[0-7])$/.test(instance.type) || typeof instance.tag !== 'string' || !instance.tag || types.has(instance.type) || tags.has(instance.tag) || typeof instance.readable !== 'boolean' || typeof instance.writable !== 'boolean') {
      throw new Error('invalid runtime switch capability response')
    }
    if (instance.writable && instance.reason !== null) throw new Error('invalid runtime switch capability response')
    if (!instance.writable && (typeof instance.reason !== 'string' || !instance.reason.trim())) throw new Error('invalid runtime switch capability response')
    types.add(instance.type)
    tags.add(instance.tag)
  }
  return value
}

export function switchInstances(capabilities = capabilityState.value) {
  if (capabilities?.kind !== 'native' || !isObject(capabilities.switches)) return []
  return Array.isArray(capabilities.switches.instances) ? capabilities.switches.instances : []
}

export function switchTagForType(type, capabilities = capabilityState.value) {
  const typeName = String(type).startsWith('switch') ? String(type) : `switch${type}`
  if (capabilities?.kind === 'legacy') return typeName
  return switchInstances(capabilities).find(instance => instance.type === typeName)?.tag || null
}

export function switchValueFromResponse(value, runtimeKind) {
  return runtimeKind === 'native' ? String(value ?? '') : String(value || '').trim()
}

function switchTagFromPath(path) {
  const match = path.match(/^\/plugins\/([^/]+)\/(show|post)$/)
  if (!match) return null
  try { return decodeURIComponent(match[1]) } catch { return null }
}

function withSwitchGeneration(url, options) {
  const tag = switchTagFromPath(new URL(url, 'http://capability.invalid').pathname)
  const generation = capabilityState.value?.switches?.config_generation
  if (!tag || typeof generation !== 'string') return options
  const headers = new Headers(options.headers || {})
  headers.set('X-Mosdns-Config-Generation', generation)
  return {...options, headers}
}
export function oldNativeOperations(value) {
  const e = value.endpoints, enabled = value.special_groups.enabled === true
  const flags = {
    'system.health': false, 'system.version': false,
    'audit.read': e.audit_v1 === true || e.audit_v2 === true,
    'audit.control': e.audit_v1 === true, 'audit.capacity': e.audit_v1 === true,
    'query.rank': e.audit_v2 === true,
    'cache.inventory': e.cache_inventory_get === true, 'cache.manage': e.cache_inventory_get === true,
    'metrics.cache': e.metrics_get === true,
    'rules.local.read': e.manual_rules?.show_get === true,
    'rules.local.manage': e.manual_rules?.post === true && e.manual_rules?.save_get === true,
    'groups.read': e.special_groups?.get === true,
    'groups.manage': enabled && e.special_groups?.post === true && e.special_groups?.delete === true,
    'upstreams.read': e.upstream?.tags_get === true && e.upstream?.config_get === true && e.upstream?.runtime_get === true,
    'upstreams.manage': enabled && e.upstream?.config_post === true,
    'rules.diversion': enabled && e.diversion_sources?.list_get === true && e.diversion_sources?.put === true && e.diversion_sources?.delete === true
  }
  return Object.fromEntries(OPERATION_IDS.map(id => [id, {supported: flags[id] === true, reason: flags[id] === true ? null : unimplemented.has(id) ? '当前 Rust 原生运行时尚未实现此操作' : '旧原生后端未声明此能力'}]))
}
export function validateRuntimeCapabilities(value) {
  if (!isObject(value) || value.schema_version !== 1 || value.runtime !== 'rust' || !isObject(value.special_groups) || typeof value.special_groups.enabled !== 'boolean' || !Array.isArray(value.upstream_protocols) || !value.upstream_protocols.every(item => typeof item === 'string') || !Array.isArray(value.rule_formats) || !value.rule_formats.every(item => typeof item === 'string') || !Array.isArray(value.unsupported_features) || !value.unsupported_features.every(item => typeof item === 'string') || !isObject(value.endpoints)) throw new Error('invalid runtime capability response')
  if (Object.hasOwn(value, 'ui_operations')) {
    if (!isObject(value.ui_operations) || OPERATION_IDS.some(id => {
      const op = value.ui_operations[id]
      return !isObject(op) || typeof op.supported !== 'boolean' || (op.supported ? op.reason !== null : typeof op.reason !== 'string' || !op.reason.trim())
    })) throw new Error('invalid runtime operation capability response')
  }
  if (Object.hasOwn(value, 'switches')) validateSwitches(value.switches)
  const uiOperations = Object.hasOwn(value, 'ui_operations') ? {...value.ui_operations} : oldNativeOperations(value)
  if (value.runtime === 'rust' && !Object.hasOwn(value, 'switches')) {
    uiOperations['switches.manage'] = { supported: false, reason: '原生后端未提供已配置开关清单' }
  }
  return {...value, ui_operations: uiOperations}
}
async function discover(url) {
  // Discovery alone bypasses operation admission, avoiding a dependency cycle.
  const response = await fetch(url)
  if (!response.ok) throw Object.assign(new Error(`HTTP ${response.status}`), {status:response.status})
  return response.json()
}
export function createRuntimeCapabilityClient(request = discover) {
  let pending
  const get = () => {
    if (!pending) pending = Promise.resolve().then(() => request('/api/v1/capabilities'))
      .then(response => ({...validateRuntimeCapabilities(response), kind:'native'}))
      .catch(error => { if (error?.status === 404) return {kind:'legacy'}; pending = null; throw error })
    return pending
  }
  get.invalidate = () => { pending = null }
  return get
}
const client = createRuntimeCapabilityClient()
export const capabilityState = reactive({status:'pending', value:null, error:'', health:null, cacheTags:new Set()})
let loading
export function getRuntimeCapabilities() {
  if (capabilityState.status === 'ready') return Promise.resolve(capabilityState.value)
  if (capabilityState.status === 'error') return Promise.reject(new Error(capabilityState.error))
  if (!loading) {
    capabilityState.status = 'pending'
    loading = client().then(value => {capabilityState.value = value; capabilityState.status = 'ready'; capabilityState.error = ''; return value})
      .catch(error => {capabilityState.status = 'error'; capabilityState.error = error.message; throw error})
      .finally(() => {loading = null})
  }
  return loading
}
export async function refreshRuntimeCapabilities() { client.invalidate(); capabilityState.status = 'pending'; return getRuntimeCapabilities() }
export function supportsOperation(id) {
  if (capabilityState.status !== 'ready') return false
  if (capabilityState.value?.kind === 'legacy') return true
  if (id === 'switches.manage' && !Object.hasOwn(capabilityState.value, 'switches')) return false
  return capabilityState.value?.ui_operations?.[id]?.supported === true
}
export function operationReason(id) {
  if (capabilityState.status === 'pending') return '正在读取运行时能力'
  if (capabilityState.status === 'error') return `运行时能力读取失败：${capabilityState.error}`
  if (id === 'switches.manage' && capabilityState.value?.kind === 'native' && !Object.hasOwn(capabilityState.value, 'switches')) return '原生后端未提供已配置开关清单'
  return supportsOperation(id) ? '' : capabilityState.value?.ui_operations?.[id]?.reason || '后端未声明此能力'
}
export async function retryRuntimeCapabilities() {
  await refreshRuntimeCapabilities()
  if (typeof window !== 'undefined') { window.dispatchEvent(new CustomEvent('mosdns-capabilities-ready')); window.dispatchEvent(new CustomEvent('mosdns-log-refresh')) }
}

// One endpoint inventory for shared HTTP, raw upload/export calls and services.
export function requestOperation(url, method = 'GET') {
  const path = new URL(url, 'http://capability.invalid').pathname
  if (path === '/api/v1/capabilities') return null
  if (path === '/api/v1/system/health') return 'system.health'
  if (path.startsWith('/api/v1/appearance/')) return 'appearance.server'
  if (path.startsWith('/api/v1/capture/')) return 'capture.logs'
  if (path.startsWith('/api/v1/update/')) return 'system.update'
  if (path.startsWith('/api/v1/config/')) return 'system.config_management'
  if (path === '/api/v1/system/restart') return 'system.restart'
  if (path === '/api/v1/system/webui-port') return 'system.webui_port'
  if (path.startsWith('/api/v1/domain-generation')) return 'system.domain_generation'
  if (path.startsWith('/api/v1/overrides')) return 'system.global_overrides'
  if (/^\/plugins\/clientname(?:\/|$)/.test(path)) return 'client.aliases'
  const switchTag = switchTagFromPath(path)
  if (switchTag && capabilityState.value?.kind === 'legacy') return 'switches.manage'
  if (switchTag && capabilityState.value?.kind === 'native') {
    // Schema-1 native peers retain the standard switchN gate. New peers
    // additionally expose configured custom tags in the inventory.
    if (switchInstances().some(instance => instance.tag === switchTag) || /^switch(?:[1-9]|1[0-7])$/.test(switchTag)) return 'switches.manage'
  }
  if (/^\/plugins\/adguard\/(rules|update)(?:\/|$)/.test(path)) return 'rules.adguard'
  if (/^\/plugins\/requery(?:\/|$)/.test(path)) return 'cache.requery'
  if (/^\/plugins\/(my_[^/]+|top_domains)(?:\/|$)/.test(path)) return 'lists.remembered'
  if (/^\/api\/v[12]\/audit\/(capacity|settings)$/.test(path)) return 'audit.capacity'
  if (/^\/api\/v[12]\/audit\/(start|stop|enable|disable|clear)$/.test(path)) return 'audit.control'
  if (/^\/api\/v2\/audit\/rank\//.test(path)) return 'query.rank'
  if (/^\/api\/v[12]\/audit\//.test(path)) return 'audit.read'
  if (path === '/api/v1/cache/inventory') return 'cache.inventory'
  if (path.startsWith('/api/v1/special-groups')) return method === 'GET' ? 'groups.read' : 'groups.manage'
  if (path.startsWith('/api/v1/upstream/')) return method === 'GET' ? 'upstreams.read' : 'upstreams.manage'
  if (path === '/metrics') return 'metrics.cache'
  if (/^\/plugins\/[^/]+\/(config|update)(?:\/|$)/.test(path)) return 'rules.diversion'
  if (/^\/plugins\/[^/]+\/(show|dump|save|flush|load_dump|search)$/.test(path)) {
    const tag = path.split('/')[2]
    return capabilityState.value?.kind === 'native' && !/^cache/.test(tag) && !capabilityState.cacheTags.has(decodeURIComponent(tag)) && /\/(show|save)$/.test(path) ? (path.endsWith('/show') ? 'rules.local.read' : 'rules.local.manage') : 'cache.manage'
  }
  if (/^\/plugins\/[^/]+\/post$/.test(path)) return 'rules.local.manage'
  return null
}
export async function capabilityFetch(url, options = {}) {
  const method = (options.method || 'GET').toUpperCase()
  let operation = requestOperation(url, method)
  if (operation) {
    await getRuntimeCapabilities()
    operation = requestOperation(url, method)
    if (!supportsOperation(operation)) throw Object.assign(new Error(operationReason(operation)), {operation, capabilityDisabled:true})
    // Old native audit.read may advertise only one protocol version. Never probe
    // a route from the unadvertised version merely because the family is true.
    if (capabilityState.value?.kind === 'native' && /^\/api\/v[12]\/audit\//.test(new URL(url,'http://capability.invalid').pathname)) {
      const version = new URL(url,'http://capability.invalid').pathname.startsWith('/api/v2/') ? 'audit_v2' : 'audit_v1'
      if (capabilityState.value.endpoints[version] !== true) throw Object.assign(new Error('后端未声明此审计版本'), {capabilityDisabled:true, operation})
    }
    if (operation === 'switches.manage' && capabilityState.value?.kind === 'native') options = withSwitchGeneration(url, options)
  }
  const response = await fetch(url, options)
  if (response.ok && operation === 'cache.inventory') { const inventory = await response.clone().json(); capabilityState.cacheTags = new Set((inventory.caches || []).map(cache => cache.tag)) }
  if (response.ok && operation === 'system.health') capabilityState.health = await response.clone().json()
  if (response.ok && method !== 'GET' && ['groups.manage','upstreams.manage','rules.diversion','rules.local.manage'].includes(operation)) {
    try { await refreshRuntimeCapabilities() } catch { /* Successful mutation ACK stays factual; shared discovery error disables subsequent optional work and exposes retry. */ }
  }
  return response
}

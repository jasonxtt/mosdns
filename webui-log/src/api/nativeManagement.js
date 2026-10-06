const SOURCE_FIELDS = new Set([
  'enabled', 'name', 'type', 'url', 'auto_update', 'enable_regexp', 'files'
])
const UPSTREAM_FIELDS = new Set([
  'tag', 'enabled', 'protocol', 'addr', 'dial_addr', 'bootstrap', 'bootstrap_version',
  'upstream_query_timeout', 'insecure_skip_verify', 'idle_timeout', 'enable_pipeline',
  'enable_http3', 'socks5', 'use_socks_proxy', 'so_mark', 'bind_to_device', 'max_conns',
  'account_id', 'access_key_id', 'access_key_secret', 'server_addr', 'ecs_client_ip',
  'ecs_client_mask'
])

function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function isHarmless(value) {
  return value == null || value === false || value === 0 || value === ''
}

function normalizeProtocol(value) {
  switch (String(value || '').trim().toLowerCase()) {
    case 'dot': return 'tls'
    case 'doh': return 'https'
    case 'doq': return 'quic'
    default: return String(value || '').trim().toLowerCase()
  }
}

function readOnly(reason) {
  return { readOnly: true, reason }
}

export function classifyDiversionSource(source, expectedType, expectedName) {
  if (!isObject(source)) return readOnly('规则记录格式无法识别')
  if (Object.keys(source).some((key) => !SOURCE_FIELDS.has(key))) {
    return readOnly('包含原生端不支持的字段，已保留为只读')
  }
  if (typeof source.enabled !== 'boolean') return readOnly('启用状态格式无法识别')
  if (
    typeof source.name !== 'string' ||
    !source.name.trim() ||
    (expectedName !== undefined && source.name !== expectedName)
  ) {
    return readOnly('规则名称与目录索引不一致')
  }
  if (source.type !== expectedType) return readOnly('规则归属与目录不一致')
  for (const key of ['url', 'auto_update', 'enable_regexp']) {
    if (Object.hasOwn(source, key) && !isHarmless(source[key])) {
      return readOnly(`包含不支持的 ${key} 配置，已保留为只读`)
    }
  }
  if (typeof source.files !== 'string' || !source.files.trim() || !source.files.endsWith('.txt')) {
    return readOnly('原生端仅支持本地 .txt 规则文件')
  }
  return { readOnly: false, reason: '' }
}

export function buildNativeDiversionPayload({ name, type, files, enabled }) {
  const normalizedName = String(name || '').trim()
  const normalizedFiles = String(files || '').trim()
  if (!normalizedName || normalizedName === '.' || normalizedName === '..' || /[\\/\u0000-\u001f\u007f]/.test(normalizedName)) {
    throw new Error('规则名称不能为空且不能包含路径字符')
  }
  if (!normalizedFiles || !normalizedFiles.endsWith('.txt')) {
    throw new Error('原生端仅支持本地 .txt 规则文件')
  }
  if (typeof type !== 'string' || !type.trim()) {
    throw new Error('规则类型不能为空')
  }
  return {
    name: normalizedName,
    type,
    files: normalizedFiles,
    enabled: Boolean(enabled),
    url: '',
    auto_update: false,
    enable_regexp: false
  }
}

export function validateDiversionCatalogChange(oldTag, newTag) {
  if (oldTag && oldTag !== newTag) {
    throw new Error('原生模式不支持将规则移动到另一个组；请先在目标组新建，再显式删除原规则')
  }
}

export function isNativeUpstreamReadOnly(upstream, capabilities) {
  if (!isObject(upstream)) return readOnly('上游记录格式无法识别')
  if (Object.keys(upstream).some((key) => !UPSTREAM_FIELDS.has(key))) {
    return readOnly('包含原生端不支持的字段，已保留为只读')
  }
  const supported = new Set((capabilities?.upstream_protocols || []).map(normalizeProtocol))
  const protocol = normalizeProtocol(upstream.protocol)
  if (!protocol || !supported.has(protocol)) {
    return readOnly(`协议 ${upstream.protocol || '(未指定)'} 当前不可用，已保留为只读`)
  }
  const unsupportedOptions = [
    'idle_timeout', 'enable_pipeline', 'enable_http3', 'socks5', 'use_socks_proxy',
    'so_mark', 'bind_to_device', 'max_conns', 'account_id', 'access_key_id',
    'access_key_secret', 'server_addr', 'ecs_client_ip', 'ecs_client_mask'
  ]
  for (const key of unsupportedOptions) {
    if (Object.hasOwn(upstream, key) && !isHarmless(upstream[key])) {
      return readOnly(`包含不支持的 ${key} 配置，已保留为只读`)
    }
  }
  return { readOnly: false, reason: '' }
}

export function supportsEndpoint(capabilities, area, operation) {
  return capabilities?.kind === 'native' &&
    capabilities?.endpoints?.[area]?.[operation] === true
}

export async function loadLegacyDiversionCatalogs(entries, getJSON) {
  const settled = await Promise.allSettled(entries.map(async ([type, tag]) => {
    const data = await getJSON(`/plugins/${tag}/config`)
    return { type, tag, rules: Array.isArray(data) ? data : [] }
  }))
  const catalogs = []
  const failures = []
  settled.forEach((result, index) => {
    if (result.status === 'fulfilled') {
      catalogs.push(result.value)
      return
    }
    const [type, tag] = entries[index]
    failures.push({
      type,
      tag,
      status: result.reason?.status,
      error: result.reason instanceof Error ? result.reason.message : String(result.reason)
    })
  })
  return { catalogs, failures }
}

// Existing standard local-rule names remain available on unmanaged native
// configurations only after their real canonical show route accepts them.
export async function loadNativeLocalProfiles(profiles, read) {
  const eligible = []
  for (const profile of profiles) {
    try {
      await read(`/plugins/${encodeURIComponent(profile.tag)}/show`)
      eligible.push(profile)
    } catch (error) {
      if (error?.status !== 404 && error?.status !== 400) throw error
    }
  }
  return eligible
}

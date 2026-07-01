const BUILTIN_GROUP_DEFS = [
  { key: 'domestic', title: '国内直连上游', tone: 'domestic' },
  { key: 'nocnfake', title: '国外 FakeIP 上游', tone: 'foreign-fake' },
  { key: 'foreign', title: '国外代理上游', tone: 'foreign-proxy' },
  { key: 'foreignecs', title: '国外 ECS 上游', tone: 'foreign-ecs' },
  { key: 'cnfake', title: '国内 FakeIP 上游', tone: 'domestic-fake' }
]

const PROTOCOL_LABELS = {
  udp: 'UDP',
  tcp: 'TCP',
  tls: 'DoT',
  dot: 'DoT',
  https: 'DoH',
  doh: 'DoH',
  quic: 'DoQ',
  doq: 'DoQ',
  aliapi: 'AliAPI'
}

function toNumber(value) {
  const number = Number(value)
  return Number.isFinite(number) ? number : 0
}

function clampNonNegative(value) {
  return Math.max(0, toNumber(value))
}

function decodeLabelValue(value) {
  return String(value || '').replace(/\\"/g, '"').replace(/\\\\/g, '\\')
}

function parseLabels(input = '') {
  const labels = {}
  const regex = /([a-zA-Z_][a-zA-Z0-9_]*)="((?:\\.|[^"])*)"/g
  let match = regex.exec(input)
  while (match !== null) {
    labels[match[1]] = decodeLabelValue(match[2])
    match = regex.exec(input)
  }
  return labels
}

function emptyMetricBucket() {
  return {
    queryTotal: 0,
    errorTotal: 0,
    winnerTotal: 0,
    latencySum: 0,
    latencyCount: 0
  }
}

function ensureBucket(target, key) {
  if (!target[key]) {
    target[key] = emptyMetricBucket()
  }
  return target[key]
}

function assignMetricValue(bucket, field, value) {
  bucket[field] = clampNonNegative(value)
}

function metricRule(metricName) {
  switch (metricName) {
    case 'mosdns_aliapi_query_total':
      return { field: 'queryTotal', groupLabel: 'metrics_tag', tagLabel: 'tag' }
    case 'mosdns_aliapi_error_total':
      return { field: 'errorTotal', groupLabel: 'metrics_tag', tagLabel: 'tag' }
    case 'mosdns_aliapi_upstream_winner_total':
      return { field: 'winnerTotal', groupLabel: 'metrics_tag', tagLabel: 'tag' }
    case 'mosdns_aliapi_response_latency_millisecond_sum':
      return { field: 'latencySum', groupLabel: 'metrics_tag', tagLabel: 'tag' }
    case 'mosdns_aliapi_response_latency_millisecond_count':
      return { field: 'latencyCount', groupLabel: 'metrics_tag', tagLabel: 'tag' }
    case 'mosdns_forward_query_total':
      return { field: 'queryTotal', groupLabel: 'tag', tagLabel: 'upstream' }
    case 'mosdns_forward_err_total':
      return { field: 'errorTotal', groupLabel: 'tag', tagLabel: 'upstream' }
    case 'mosdns_forward_response_latency_millisecond_sum':
      return { field: 'latencySum', groupLabel: 'tag', tagLabel: 'upstream' }
    case 'mosdns_forward_response_latency_millisecond_count':
      return { field: 'latencyCount', groupLabel: 'tag', tagLabel: 'upstream' }
    default:
      return null
  }
}

export function parseUpstreamStatsMetrics(rawText = '') {
  const buckets = {}
  const lines = String(rawText || '').split('\n')

  for (const line of lines) {
    const trimmed = line.trim()
    if (!trimmed || trimmed.startsWith('#')) {
      continue
    }

    const match = trimmed.match(/^([a-zA-Z_:][a-zA-Z0-9_:]*)(?:\{([^}]*)\})?\s+([0-9.eE+-]+)$/)
    if (!match) {
      continue
    }

    const rule = metricRule(match[1])
    if (!rule) {
      continue
    }

    const labels = parseLabels(match[2] || '')
    const group = String(labels[rule.groupLabel] || '').trim()
    const tag = String(labels[rule.tagLabel] || '').trim()
    if (!group || !tag) {
      continue
    }

    const bucket = ensureBucket(buckets, `${group}|${tag}`)
    assignMetricValue(bucket, rule.field, match[3])
  }

  return buckets
}

export function orderUpstreamGroups(groupKeys = [], specialGroups = []) {
  const keySet = new Set(groupKeys)
  const ordered = []

  BUILTIN_GROUP_DEFS.forEach((def) => {
    if (keySet.has(def.key)) {
      ordered.push(def.key)
      keySet.delete(def.key)
    }
  })

  const specialOrdered = [...(specialGroups || [])]
    .sort((a, b) => Number(a?.slot || 0) - Number(b?.slot || 0))
    .map((item) => String(item?.upstream_plugin_tag || '').trim())
    .filter(Boolean)

  specialOrdered.forEach((key) => {
    if (keySet.has(key)) {
      ordered.push(key)
      keySet.delete(key)
    }
  })

  return [
    ...ordered,
    ...Array.from(keySet).sort((a, b) => a.localeCompare(b, 'zh-CN'))
  ]
}

export function upstreamGroupDisplay(group, specialGroups = []) {
  const builtin = BUILTIN_GROUP_DEFS.find((item) => item.key === group)
  if (builtin) {
    return builtin
  }

  const special = (specialGroups || []).find((item) => item?.upstream_plugin_tag === group)
  if (special?.name) {
    return {
      key: group,
      title: special.name,
      tone: 'special'
    }
  }

  return {
    key: group,
    title: group || '-',
    tone: 'other'
  }
}

export function protocolDisplayLabel(protocol) {
  const normalized = String(protocol || '').trim().toLowerCase()
  return PROTOCOL_LABELS[normalized] || String(protocol || '-').toUpperCase()
}

export function upstreamAddressDisplay(item = {}) {
  const protocol = String(item.protocol || '').trim().toLowerCase()
  if (protocol === 'aliapi') {
    return String(item.server_addr || '-').trim() || '-'
  }
  return String(item.addr || '-').trim() || '-'
}

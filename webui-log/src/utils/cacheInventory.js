// null means an explicitly unsupported catalog and permits the Go fallback.
export async function loadCacheInventory(getJSON) {
  let inventory
  try {
    inventory = await getJSON('/api/v1/cache/inventory')
  } catch (error) {
    if (error?.status === 404) return null
    throw error
  }
  if (inventory?.supported === false) return null
  if (inventory?.schema_version !== 1 || !Array.isArray(inventory.caches)) {
    throw new Error('缓存列表格式或版本不受支持')
  }
  const tags = new Set()
  return inventory.caches.map((cache) => {
    if (typeof cache?.tag !== 'string' || !cache.tag.trim() || tags.has(cache.tag)) {
      throw new Error('缓存列表包含无效或重复标签')
    }
    tags.add(cache.tag)
    return { key: `native-cache:${cache.tag}`, name: cache.tag, tag: cache.tag }
  })
}

export function parseCacheMetrics(metricsText, tag) {
  const stats = { query_total: null, hit_total: null, lazy_hit_total: null, size_current: null }
  const escapedTag = String(tag).replaceAll('\\', '\\\\').replaceAll('"', '\\"').replaceAll('\n', '\\n')
  const prefix = `mosdns_cache_`
  for (const line of String(metricsText || '').split('\n')) {
    for (const key of Object.keys(stats)) {
      const name = `${prefix}${key}{tag="${escapedTag}"}`
      if (!line.startsWith(`${name} `)) continue
      const number = Number(line.slice(name.length).trim().split(/\s+/)[0])
      if (Number.isFinite(number) && number >= 0) stats[key] = number
    }
  }
  return stats
}

export function summarizeCacheFlush(caches, results) {
  const failed = caches.filter((_, index) => results[index]?.status !== 'fulfilled').map((cache) => cache.tag)
  return { succeeded: caches.length - failed.length, failed }
}

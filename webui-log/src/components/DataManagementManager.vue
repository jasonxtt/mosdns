<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getText } from '../api/http'
import { openConfirm } from '../utils/confirm'

const errorMessage = ref('')
const successMessage = ref('')
const specialGroups = ref([])
const cacheRows = ref([])
const cacheRefreshing = ref(false)
const cacheClearingAll = ref(false)
const cacheClearingByTag = reactive({})

const dataView = reactive({
  open: false,
  title: '',
  cacheTag: '',
  query: '',
  entries: [],
  totalCount: 0,
  currentOffset: 0,
  currentLimit: 100,
  hasMore: true,
  loading: false,
  loadingMore: false,
  error: ''
})

const baseCacheConfig = [
  { key: 'cache_all', name: '全部缓存 (兼容)', tag: 'cache_all' },
  { key: 'cache_cn', name: '国内缓存', tag: 'cache_cn' },
  { key: 'cache_node', name: '节点缓存', tag: 'cache_node' },
  { key: 'cache_google', name: '国外缓存 (兼容)', tag: 'cache_google' },
  { key: 'cache_all_noleak', name: '全部缓存 (安全)', tag: 'cache_all_noleak' },
  { key: 'cache_google_node', name: '国外缓存 (安全)', tag: 'cache_google_node' },
  { key: 'cache_cnmihomo', name: '国内域名fakeip', tag: 'cache_cnmihomo' }
]

let dataViewSearchTimerId = 0

const cacheConfig = computed(() => {
  const dynamic = [...specialGroups.value]
    .sort((a, b) => Number(a.slot) - Number(b.slot))
    .map((group) => ({
      key: `cache_special_${group.slot}`,
      name: `${group.name || `专属分流组 ${group.slot}`} 缓存`,
      tag: `cache_special_${group.slot}`
    }))
  return [...baseCacheConfig, ...dynamic]
})

function setError(message) {
  successMessage.value = ''
  errorMessage.value = message
}

function setSuccess(message) {
  errorMessage.value = ''
  successMessage.value = message
}

function parseMetrics(metricsText, cacheTag) {
  const lines = String(metricsText || '').split('\n')
  const stats = { query_total: 0, hit_total: 0, lazy_hit_total: 0, size_current: 0 }
  const queryKey = `mosdns_cache_query_total{tag="${cacheTag}"}`
  const hitKey = `mosdns_cache_hit_total{tag="${cacheTag}"}`
  const lazyKey = `mosdns_cache_lazy_hit_total{tag="${cacheTag}"}`
  const sizeKey = `mosdns_cache_size_current{tag="${cacheTag}"}`

  lines.forEach((line) => {
    if (line.startsWith(queryKey)) {
      stats.query_total = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith(hitKey)) {
      stats.hit_total = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith(lazyKey)) {
      stats.lazy_hit_total = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith(sizeKey)) {
      stats.size_current = Number.parseFloat(line.split(' ')[1] || '0') || 0
    }
  })

  return stats
}

function buildCacheRows(metricsText) {
  cacheRows.value = cacheConfig.value.map((cache) => {
    const stats = parseMetrics(metricsText, cache.tag)
    const hitRate = stats.query_total > 0 ? ((stats.hit_total / stats.query_total) * 100).toFixed(2) : '0.00'
    const lazyRate = stats.query_total > 0 ? ((stats.lazy_hit_total / stats.query_total) * 100).toFixed(2) : '0.00'
    return {
      ...cache,
      ...stats,
      hit_rate: `${hitRate}%`,
      lazy_hit_rate: `${lazyRate}%`
    }
  })
}

async function requestResponse(url, options = {}) {
  const response = await fetch(url, options)
  if (!response.ok) {
    let message = `HTTP ${response.status} ${response.statusText}`
    try {
      const data = await response.json()
      if (data?.error) {
        message = data.error
      }
    } catch {
      try {
        const text = await response.text()
        if (text) {
          message = text
        }
      } catch {
        // ignore
      }
    }
    throw new Error(message)
  }
  return response
}

async function refreshCacheStats(showMessage = false) {
  cacheRefreshing.value = true
  try {
    const [groupsRes, metricsText] = await Promise.all([
      getJSON('/api/v1/special-groups').catch(() => []),
      getText('/metrics')
    ])
    specialGroups.value = Array.isArray(groupsRes) ? groupsRes : []
    buildCacheRows(metricsText)
    if (showMessage) {
      setSuccess('缓存统计已刷新')
    }
  } catch (error) {
    setError(`刷新缓存统计失败: ${error.message}`)
  } finally {
    cacheRefreshing.value = false
  }
}

async function clearSingleCache(cacheTag, cacheName) {
  if (!(await openConfirm(`确定要清空缓存“${cacheName}”吗？`, { tone: 'danger' }))) {
    return
  }
  cacheClearingByTag[cacheTag] = true
  try {
    await requestResponse(`/plugins/${cacheTag}/flush`)
    await refreshCacheStats()
    setSuccess(`缓存“${cacheName}”已清空`)
  } catch (error) {
    setError(`清空缓存失败: ${error.message}`)
  } finally {
    cacheClearingByTag[cacheTag] = false
  }
}

async function clearAllCaches() {
  if (!(await openConfirm(`将依次清空 ${cacheConfig.value.length} 个缓存实例，此操作不可恢复。`, { tone: 'danger' }))) {
    return
  }
  cacheClearingAll.value = true
  try {
    const results = await Promise.allSettled(
      cacheConfig.value.map((cache) => requestResponse(`/plugins/${cache.tag}/flush`))
    )
    const failed = results.filter((item) => item.status === 'rejected').length
    await refreshCacheStats()
    if (failed > 0) {
      setError(`全部缓存已执行清空，失败 ${failed} 个`)
      return
    }
    setSuccess('全部缓存已清空')
  } catch (error) {
    setError(`清空所有缓存失败: ${error.message}`)
  } finally {
    cacheClearingAll.value = false
  }
}

function parseCacheEntries(text, startOffset = 0) {
  const chunks = String(text || '').trim()
    ? String(text || '').trim().split('----- Cache Entry -----').filter((entry) => entry.trim() !== '')
    : []
  return chunks.map((entryText, index) => {
    const questionMatch = entryText.match(/;; QUESTION SECTION:\s*;\s*([^\s]+)/)
    const domainSetMatch = entryText.match(/DomainSet:\s*(.+)/)
    let headerTitle = questionMatch ? questionMatch[1].replace(/\.$/, '') : `Entry #${startOffset + index + 1}`
    if (domainSetMatch) {
      headerTitle += ` [${domainSetMatch[1].trim()}]`
    }

    const dnsMessageIndex = entryText.indexOf('DNS Message:')
    const metadataText = dnsMessageIndex >= 0 ? entryText.slice(0, dnsMessageIndex) : entryText
    const dnsMessage = dnsMessageIndex >= 0 ? entryText.slice(dnsMessageIndex).trim() : 'DNS Message not found.'
    const metadataRows = metadataText
      .trim()
      .split('\n')
      .map((line) => {
        const parts = line.match(/^([^:]+):\s*(.*)$/)
        if (!parts) {
          return null
        }
        return { key: parts[1].trim(), value: parts[2].trim() }
      })
      .filter(Boolean)

    return {
      key: `${headerTitle}-${startOffset + index}`,
      headerTitle,
      metadataRows,
      dnsMessage
    }
  })
}

async function fetchDataView(append = false) {
  if (!dataView.cacheTag) {
    dataView.error = '无效的缓存标签'
    return
  }

  if (append) {
    dataView.loadingMore = true
  } else {
    dataView.loading = true
    dataView.error = ''
  }

  try {
    const params = new URLSearchParams({
      q: String(dataView.query || ''),
      offset: String(dataView.currentOffset || 0),
      limit: String(dataView.currentLimit || 100)
    })
    const response = await requestResponse(`/plugins/${dataView.cacheTag}/show?${params.toString()}`)
    const totalHeader = response.headers.get('X-Total-Count')
    const totalCount = totalHeader !== null && totalHeader !== ''
      ? Number.parseInt(totalHeader, 10) || 0
      : 0
    const text = await response.text()
    const nextEntries = parseCacheEntries(text, dataView.currentOffset)

    dataView.entries = append ? [...dataView.entries, ...nextEntries] : nextEntries
    dataView.totalCount = totalCount
    dataView.currentOffset += nextEntries.length
    if (totalCount > 0) {
      dataView.hasMore = dataView.entries.length < totalCount
    } else {
      dataView.hasMore = nextEntries.length >= dataView.currentLimit
    }
  } catch (error) {
    if (!append) {
      dataView.entries = []
    }
    dataView.error = `加载失败: ${error.message}`
    dataView.hasMore = false
  } finally {
    dataView.loading = false
    dataView.loadingMore = false
  }
}

function openDataViewForCache(cache) {
  if (!cache) {
    return
  }
  dataView.open = true
  dataView.title = cache.name
  dataView.cacheTag = cache.tag
  dataView.query = ''
  dataView.entries = []
  dataView.totalCount = 0
  dataView.currentOffset = 0
  dataView.hasMore = true
  dataView.error = ''
  fetchDataView(false)
}

function closeDataView() {
  dataView.open = false
}

function onDataViewSearchInput() {
  if (dataViewSearchTimerId) {
    window.clearTimeout(dataViewSearchTimerId)
  }
  dataViewSearchTimerId = window.setTimeout(() => {
    dataView.currentOffset = 0
    dataView.entries = []
    dataView.totalCount = 0
    dataView.hasMore = true
    dataView.error = ''
    fetchDataView(false)
  }, 350)
}

function loadMoreDataView() {
  if (dataView.loading || dataView.loadingMore || !dataView.hasMore) {
    return
  }
  fetchDataView(true)
}

function reloadAll() {
  errorMessage.value = ''
  successMessage.value = ''
  refreshCacheStats()
}

onMounted(() => {
  reloadAll()
  window.addEventListener('mosdns-log-refresh', reloadAll)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', reloadAll)
  if (dataViewSearchTimerId) {
    window.clearTimeout(dataViewSearchTimerId)
    dataViewSearchTimerId = 0
  }
})
</script>

<template>
  <section class="data-panel">
    <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
    <p v-if="successMessage && !errorMessage" class="msg success">{{ successMessage }}</p>

    <section class="panel sub-panel data-module cache-module">
      <header class="panel-header cache-module-head">
        <div>
          <h3>缓存管理</h3>
          <p class="muted">仅保留缓存统计、查看与清理。</p>
        </div>
        <div class="actions">
          <button class="btn secondary" :disabled="cacheRefreshing" @click="refreshCacheStats(true)">
            {{ cacheRefreshing ? '刷新中...' : '刷新统计' }}
          </button>
          <button class="btn danger cache-clear-btn" :disabled="cacheClearingAll" @click="clearAllCaches">
            {{ cacheClearingAll ? '清空中...' : '清空所有缓存' }}
          </button>
        </div>
      </header>

      <div class="table-wrap cache-table-wrap data-scroll-wrap">
        <table class="cache-adaptive-table">
          <thead>
            <tr>
              <th>缓存名称</th>
              <th>请求总数</th>
              <th>缓存命中</th>
              <th>过期命中</th>
              <th>命中率</th>
              <th>过期命中率</th>
              <th>条目数</th>
              <th>操作</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="cacheRows.length === 0">
              <td colspan="8" class="empty">暂无缓存数据</td>
            </tr>
            <tr v-for="cache in cacheRows" :key="cache.key">
              <td>{{ cache.name }}</td>
              <td>{{ Number(cache.query_total || 0).toLocaleString() }}</td>
              <td>{{ Number(cache.hit_total || 0).toLocaleString() }}</td>
              <td>{{ Number(cache.lazy_hit_total || 0).toLocaleString() }}</td>
              <td>{{ cache.hit_rate }}</td>
              <td>{{ cache.lazy_hit_rate }}</td>
              <td>
                <button class="btn-link" type="button" @click="openDataViewForCache(cache)">
                  {{ Number(cache.size_current || 0).toLocaleString() }}
                </button>
              </td>
              <td>
                <button
                  class="btn danger tiny"
                  :disabled="Boolean(cacheClearingByTag[cache.tag])"
                  @click="clearSingleCache(cache.tag, cache.name)"
                >
                  {{ cacheClearingByTag[cache.tag] ? '清空中...' : '清空' }}
                </button>
              </td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <div v-if="dataView.open" class="modal-mask" @click.self="closeDataView">
      <section class="panel data-view-modal">
        <header class="panel-header">
          <div>
            <h3>{{ dataView.title }}</h3>
            <p class="muted">
              <span v-if="dataView.totalCount > 0">当前显示 {{ dataView.entries.length.toLocaleString() }} / {{ dataView.totalCount.toLocaleString() }} 条</span>
              <span v-else>当前显示 {{ dataView.entries.length.toLocaleString() }} 条</span>
            </p>
          </div>
          <div class="actions">
            <button class="btn secondary" type="button" @click="closeDataView">关闭</button>
          </div>
        </header>

        <div class="data-view-search">
          <input
            v-model="dataView.query"
            placeholder="搜索..."
            @input="onDataViewSearchInput"
          />
        </div>

        <p v-if="dataView.error" class="msg error">{{ dataView.error }}</p>

        <div class="cache-entry-list">
          <p v-if="dataView.loading" class="empty">加载中...</p>
          <p v-else-if="dataView.entries.length === 0" class="empty">没有匹配的条目</p>
          <details
            v-for="item in dataView.entries"
            :key="item.key"
            class="cache-entry"
          >
            <summary>{{ item.headerTitle }}</summary>
            <div class="table-wrap cache-meta-wrap">
              <table>
                <tbody>
                  <tr v-for="(meta, index) in item.metadataRows" :key="`${item.key}-meta-${index}`">
                    <td>{{ meta.key }}</td>
                    <td class="mono">{{ meta.value }}</td>
                  </tr>
                </tbody>
              </table>
            </div>
            <pre class="mono cache-pre">{{ item.dnsMessage }}</pre>
          </details>
        </div>

        <div class="actions" style="margin-top: 10px;">
          <button
            class="btn primary"
            type="button"
            :disabled="dataView.loading || dataView.loadingMore || !dataView.hasMore"
            @click="loadMoreDataView"
          >
            {{ dataView.loadingMore ? '加载中...' : '加载更多' }}
          </button>
        </div>
      </section>
    </div>
  </section>
</template>

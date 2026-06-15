<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getText, postJSON } from '../api/http'
import DataCachePanel from './data/DataCachePanel.vue'
import DataListStatsPanel from './data/DataListStatsPanel.vue'
import DataRequeryPanel from './data/DataRequeryPanel.vue'
import DataViewModal from './data/DataViewModal.vue'
import { openConfirm } from '../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../utils/notice'
import { formatRelativeTime, isZeroTime } from '../utils/time'

const props = defineProps({
  mode: {
    type: String,
    default: 'all'
  }
})

const loading = ref(false)

const specialGroups = ref([])
const cacheRows = ref([])
const cacheRefreshing = ref(false)
const cacheClearingAll = ref(false)
const cacheClearingByTag = reactive({})
const coreMode = ref('')

const listStatsRefreshing = ref(false)
const listStats = ref([
  { key: 'fakeip', name: 'FakeIP 域名', endpoint: '/plugins/my_fakeiplist/show', count: null, error: '' },
  { key: 'realip', name: 'RealIP 域名', endpoint: '/plugins/my_realiplist/show', count: null, error: '' },
  { key: 'nov4', name: '无 V4 域名', endpoint: '/plugins/my_nov4list/show', count: null, error: '' },
  { key: 'nov6', name: '无 V6 域名', endpoint: '/plugins/my_nov6list/show', count: null, error: '' },
  { key: 'total', name: '域名排行', endpoint: '/plugins/top_domains/show', count: null, error: '' }
])

const dataView = reactive({
  open: false,
  title: '',
  mode: 'domain',
  listType: '',
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

const dataViewListEndpointMap = {
  fakeip: '/plugins/my_fakeiplist/show',
  realip: '/plugins/my_realiplist/show',
  nov4: '/plugins/my_nov4list/show',
  nov6: '/plugins/my_nov6list/show',
  total: '/plugins/top_domains/show'
}

const requeryAvailable = ref(true)
const requeryLoadError = ref('')
const requeryStatus = ref(null)
const requeryConfig = ref(null)
const requeryStatusRefreshing = ref(false)
const requeryAction = ref('')
const schedulerSaving = ref(false)

const sourceFileCountsRefreshing = ref(false)
const sourceFileCounts = ref([])
const sourceFileCountsError = ref('')

const schedulerForm = reactive({
  enabled: false,
  intervalMinutes: 0,
  dateRangeDays: 30,
  startDatetimeLocal: ''
})

const saveRulePaths = [
  'top_domains/save',
  'my_fakeiplist/save',
  'my_nodenov4list/save',
  'my_nodenov6list/save',
  'my_notinlist/save',
  'my_nov4list/save',
  'my_nov6list/save',
  'my_realiplist/save'
]

const flushRulePaths = [
  'top_domains/flush',
  'my_fakeiplist/flush',
  'my_nodenov4list/flush',
  'my_nodenov6list/flush',
  'my_notinlist/flush',
  'my_nov4list/flush',
  'my_nov6list/flush',
  'my_realiplist/flush'
]

const baseCacheConfig = [
  { key: 'cache_all', name: '全部缓存 (兼容)', tag: 'cache_all', coreModes: ['A'] },
  { key: 'cache_cn', name: '国内缓存', tag: 'cache_cn' },
  { key: 'cache_node', name: '节点缓存', tag: 'cache_node' },
  { key: 'cache_google', name: '国外缓存 (兼容)', tag: 'cache_google', coreModes: ['A'] },
  { key: 'cache_all_noleak', name: '全部缓存 (安全)', tag: 'cache_all_noleak', coreModes: ['B'] },
  { key: 'cache_google_node', name: '国外缓存 (安全)', tag: 'cache_google_node', coreModes: ['B'] },
  { key: 'cache_cnmihomo', name: '国内域名fakeip', tag: 'cache_cnmihomo' }
]

let schedulerTimerId = 0
let requeryPollTimerId = 0
let requeryPollIntervalMs = 0
let requeryPendingTimerId = 0
let dataViewSearchTimerId = 0
const requeryTriggerPending = ref(false)
const requeryTriggerRequestedAt = ref(0)

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

const visibleCacheConfig = computed(() => {
  const mode = String(coreMode.value || '').trim().toUpperCase()
  if (mode !== 'A' && mode !== 'B') {
    return cacheConfig.value
  }
  return cacheConfig.value.filter((cache) => {
    if (!Array.isArray(cache.coreModes) || cache.coreModes.length === 0) {
      return true
    }
    return cache.coreModes.includes(mode)
  })
})

const requeryStatusMeta = computed(() => {
  const state = String(requeryStatus.value?.task_state || 'idle')
  if (state === 'running') {
    return { text: '正在执行...', className: 'running' }
  }
  if (state === 'failed') {
    return { text: '上次执行失败', className: 'failed' }
  }
  if (state === 'cancelled') {
    return { text: '上次任务已取消', className: 'cancelled' }
  }
  return { text: '空闲', className: 'idle' }
})

const requeryProgress = computed(() => {
  const processed = Number(requeryStatus.value?.progress?.processed || 0)
  const total = Number(requeryStatus.value?.progress?.total || 0)
  const percent = total > 0 ? Math.max(0, Math.min(100, (processed / total) * 100)) : 0
  return { processed, total, percent }
})

const isRequeryRunning = computed(() => String(requeryStatus.value?.task_state || '') === 'running')

const lastRunText = computed(() => {
  const status = requeryStatus.value
  if (!status?.last_run_start_time || isZeroTime(status.last_run_start_time)) {
    return '从未执行'
  }
  if (status.last_run_end_time && !isZeroTime(status.last_run_end_time)) {
    const startTime = new Date(status.last_run_start_time).getTime()
    const endTime = new Date(status.last_run_end_time).getTime()
    const durationMs = Number.isFinite(startTime) && Number.isFinite(endTime) ? Math.max(0, endTime - startTime) : 0
    if (durationMs < 1000) {
      return `完成于 ${formatRelativeTime(status.last_run_end_time)} (耗时 <1秒)`
    }
    const durationSeconds = Math.max(1, Math.round(durationMs / 1000))
    return `完成于 ${formatRelativeTime(status.last_run_end_time)} (耗时 ${durationSeconds}秒)`
  }
  return `开始于 ${formatRelativeTime(status.last_run_start_time)}`
})

const lastRunErrorText = computed(() => {
  const status = requeryStatus.value
  if (!status || String(status.task_state || '') !== 'failed') {
    return ''
  }
  return String(status.last_error || '').trim()
})

const lastRunDomainCountText = computed(() => {
  const count = Number(requeryStatus.value?.last_run_domain_count)
  if (Number.isFinite(count) && count >= 0) {
    return `${count.toLocaleString()} 条`
  }
  return '--'
})

const showAllPanels = computed(() => {
  const mode = String(props.mode || '').trim()
  return mode === '' || mode === 'all'
})

const showCachePanel = computed(() => showAllPanels.value || props.mode === 'cache-management')
const showListStatsPanel = computed(() => showAllPanels.value || props.mode === 'domain-stats')
const showRequeryPanel = computed(() => showAllPanels.value || props.mode === 'requery-cache')

function formatDateForInputLocal(value) {
  if (!value || isZeroTime(value)) {
    return ''
  }
  const date = new Date(value)
  if (!Number.isFinite(date.getTime())) {
    return ''
  }
  const year = date.getFullYear()
  const month = String(date.getMonth() + 1).padStart(2, '0')
  const day = String(date.getDate()).padStart(2, '0')
  const hours = String(date.getHours()).padStart(2, '0')
  const minutes = String(date.getMinutes()).padStart(2, '0')
  return `${year}-${month}-${day}T${hours}:${minutes}`
}

function toUtcISOString(localDatetime) {
  if (!localDatetime) {
    return ''
  }
  const date = new Date(localDatetime)
  if (!Number.isFinite(date.getTime())) {
    throw new Error('输入的首次执行时间格式无效')
  }
  return date.toISOString()
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
  cacheRows.value = visibleCacheConfig.value.map((cache) => {
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

async function loadCoreMode() {
  try {
    coreMode.value = String(await getText('/plugins/switch3/show') || '').trim().toUpperCase()
  } catch {
    coreMode.value = ''
  }
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

async function postEmpty(url) {
  return requestResponse(url, { method: 'POST' })
}

async function refreshCacheStats(showMessage = false) {
  cacheRefreshing.value = true
  try {
    const [groupsRes, metricsText] = await Promise.all([
      getJSON('/api/v1/special-groups').catch(() => []),
      getText('/metrics')
    ])
    specialGroups.value = Array.isArray(groupsRes) ? groupsRes : []
    await loadCoreMode()
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
  if (!(await openConfirm(`将依次清空 ${visibleCacheConfig.value.length} 个缓存实例，此操作不可恢复。`, { tone: 'danger' }))) {
    return
  }
  cacheClearingAll.value = true
  try {
    const results = await Promise.allSettled(
      visibleCacheConfig.value.map((cache) => requestResponse(`/plugins/${cache.tag}/flush`))
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

function countLines(text) {
  const normalized = String(text || '').trim()
  if (!normalized) {
    return 0
  }
  return normalized.split('\n').filter(Boolean).length
}

async function fetchListCount(endpoint) {
  const response = await requestResponse(`${endpoint}?limit=1`)
  const totalCount = response.headers.get('X-Total-Count')
  if (totalCount !== null && totalCount !== '') {
    return Number.parseInt(totalCount, 10) || 0
  }
  const text = await response.text()
  return countLines(text)
}

async function refreshListStats(showMessage = false) {
  listStatsRefreshing.value = true
  try {
    const next = listStats.value.map((item) => ({ ...item, count: null, error: '' }))
    const results = await Promise.allSettled(
      next.map(async (item) => ({
        key: item.key,
        count: await fetchListCount(item.endpoint)
      }))
    )

    results.forEach((result, index) => {
      if (result.status === 'fulfilled') {
        next[index].count = result.value.count
      } else {
        next[index].error = result.reason?.message || '获取失败'
      }
    })

    listStats.value = next
    if (showMessage) {
      setSuccess('域名列表统计已刷新')
    }
  } catch (error) {
    setError(`刷新域名列表统计失败: ${error.message}`)
  } finally {
    listStatsRefreshing.value = false
  }
}

function parseDomainEntries(text) {
  const lines = String(text || '').trim() ? String(text || '').trim().split('\n') : []
  return lines.map((line) => {
    const trimmed = line.trim()
    const match3 = trimmed.match(/^(\S+)\s+(\S+)\s+(.*)$/)
    if (match3) {
      return { count: match3[1], date: match3[2], domain: match3[3] }
    }
    const match2 = trimmed.match(/^(\S+)\s+(.*)$/)
    if (match2) {
      return { count: match2[1], date: '-', domain: match2[2] }
    }
    return { count: '-', date: '-', domain: trimmed }
  })
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
  const endpoint = dataView.mode === 'domain'
    ? dataViewListEndpointMap[dataView.listType]
    : `/plugins/${dataView.cacheTag}/show`

  if (!endpoint) {
    dataView.error = '无效的数据来源'
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
    const response = await requestResponse(`${endpoint}?${params.toString()}`)
    const totalHeader = response.headers.get('X-Total-Count')
    const totalCount = totalHeader !== null && totalHeader !== ''
      ? Number.parseInt(totalHeader, 10) || 0
      : 0
    const text = await response.text()

    const nextEntries = dataView.mode === 'domain'
      ? parseDomainEntries(text)
      : parseCacheEntries(text, dataView.currentOffset)

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

function openDataViewForList(row) {
  if (!row || row.count === null || row.error) {
    return
  }
  dataView.open = true
  dataView.title = row.name
  dataView.mode = 'domain'
  dataView.listType = row.key
  dataView.cacheTag = ''
  dataView.query = ''
  dataView.entries = []
  dataView.totalCount = 0
  dataView.currentOffset = 0
  dataView.hasMore = true
  dataView.error = ''
  fetchDataView(false)
}

function openDataViewForCache(cache) {
  if (!cache) {
    return
  }
  dataView.open = true
  dataView.title = cache.name
  dataView.mode = 'cache'
  dataView.listType = ''
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

function clearCacheRow(cache) {
  if (!cache) {
    return
  }
  clearSingleCache(cache.tag, cache.name)
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

function applyRequeryConfigToForm() {
  const scheduler = requeryConfig.value?.scheduler || {}
  const executionSettings = requeryConfig.value?.execution_settings || {}
  schedulerForm.enabled = Boolean(scheduler.enabled)
  schedulerForm.intervalMinutes = Number(scheduler.interval_minutes || 0)
  schedulerForm.dateRangeDays = Number(executionSettings.date_range_days || 30)
  schedulerForm.startDatetimeLocal = formatDateForInputLocal(scheduler.start_datetime)
}

function setRequeryTriggerPending(active) {
  requeryTriggerPending.value = Boolean(active)
  if (!active) {
    requeryTriggerRequestedAt.value = 0
    if (requeryPendingTimerId) {
      window.clearTimeout(requeryPendingTimerId)
      requeryPendingTimerId = 0
    }
    ensureRequeryPolling()
    return
  }
  if (requeryPendingTimerId) {
    window.clearTimeout(requeryPendingTimerId)
  }
  requeryPendingTimerId = window.setTimeout(() => {
    requeryTriggerPending.value = false
    requeryPendingTimerId = 0
    ensureRequeryPolling()
  }, 15000)
  ensureRequeryPolling()
}

function ensureRequeryPolling() {
  const shouldPoll = isRequeryRunning.value || requeryTriggerPending.value
  const intervalMs = requeryTriggerPending.value ? 1000 : 5000
  if (shouldPoll) {
    if (requeryPollTimerId && requeryPollIntervalMs === intervalMs) {
      return
    }
    if (requeryPollTimerId) {
      window.clearInterval(requeryPollTimerId)
    }
    requeryPollIntervalMs = intervalMs
    requeryPollTimerId = window.setInterval(() => {
      refreshRequeryStatusAndConfig(false)
    }, intervalMs)
    return
  }
  if (requeryPollTimerId) {
    window.clearInterval(requeryPollTimerId)
    requeryPollTimerId = 0
  }
  requeryPollIntervalMs = 0
}

async function refreshRequeryStatusAndConfig(showMessage = false) {
  requeryStatusRefreshing.value = true
  requeryLoadError.value = ''
  try {
    const [status, config] = await Promise.all([
      getJSON('/plugins/requery/status'),
      getJSON('/plugins/requery')
    ])
    requeryAvailable.value = true
    requeryStatus.value = status
    requeryConfig.value = config
    const state = String(status?.task_state || '')
    const lastStartMs = new Date(status?.last_run_start_time || '').getTime()
    if (state === 'running') {
      setRequeryTriggerPending(false)
    } else if (
      requeryTriggerPending.value &&
      Number.isFinite(lastStartMs) &&
      lastStartMs >= requeryTriggerRequestedAt.value - 1000
    ) {
      setRequeryTriggerPending(false)
    }
    applyRequeryConfigToForm()
    ensureRequeryPolling()
    if (showMessage) {
      setSuccess('刷新分流缓存状态已刷新')
    }
  } catch (error) {
    const message = String(error?.message || '')
    if (message.includes('404')) {
      requeryAvailable.value = false
      requeryStatus.value = null
      requeryConfig.value = null
      sourceFileCounts.value = []
      sourceFileCountsError.value = ''
      requeryLoadError.value = '未检测到 requery 插件，已跳过该模块。'
      setRequeryTriggerPending(false)
      ensureRequeryPolling()
      return
    }
    requeryAvailable.value = true
    requeryLoadError.value = `加载刷新分流缓存状态失败: ${message}`
    setError(requeryLoadError.value)
  } finally {
    requeryStatusRefreshing.value = false
  }
}

async function refreshSourceFileCounts(showMessage = false) {
  if (!requeryAvailable.value) {
    return
  }
  sourceFileCountsRefreshing.value = true
  sourceFileCountsError.value = ''
  try {
    const response = await getJSON('/plugins/requery/stats/source_file_counts')
    if (response?.status === 'success' && Array.isArray(response?.data)) {
      sourceFileCounts.value = response.data
      if (showMessage) {
        setSuccess('来源文件统计已刷新')
      }
      return
    }
    throw new Error('返回数据格式不正确')
  } catch (error) {
    sourceFileCounts.value = []
    sourceFileCountsError.value = `获取来源文件统计失败: ${error.message}`
  } finally {
    sourceFileCountsRefreshing.value = false
  }
}

async function triggerRequery() {
  if (!(await openConfirm('将启动一次全新刷新任务，并完整执行所有步骤，可能需要一些时间。'))) {
    return
  }
  requeryAction.value = 'trigger'
  try {
    await postEmpty('/plugins/requery/trigger')
    requeryTriggerRequestedAt.value = Date.now()
    setRequeryTriggerPending(true)
    await refreshRequeryStatusAndConfig()
    setSuccess('刷新任务已开始')
  } catch (error) {
    setRequeryTriggerPending(false)
    setError(`开始任务失败: ${error.message}`)
  } finally {
    requeryAction.value = ''
  }
}

async function cancelRequery() {
  if (!(await openConfirm('确定要取消当前正在执行的刷新任务吗？', { tone: 'danger' }))) {
    return
  }
  requeryAction.value = 'cancel'
  try {
    await postEmpty('/plugins/requery/cancel')
    await refreshRequeryStatusAndConfig()
    setSuccess('已发送取消请求')
  } catch (error) {
    setError(`取消任务失败: ${error.message}`)
  } finally {
    requeryAction.value = ''
  }
}

async function updateSchedulerConfig() {
  if (!requeryAvailable.value) {
    return
  }
  const enabled = Boolean(schedulerForm.enabled)
  const interval = Number(schedulerForm.intervalMinutes || 0)
  const dateRangeDays = Number(schedulerForm.dateRangeDays || 0)
  if (enabled && interval <= 0) {
    setError('启用定时任务时，必须设置一个有效的间隔分钟数')
    return
  }
  if (dateRangeDays <= 0) {
    setError('域名刷新天数必须大于 0')
    return
  }

  schedulerSaving.value = true
  try {
    const payload = {
      enabled,
      interval_minutes: interval || 0,
      start_datetime: toUtcISOString(schedulerForm.startDatetimeLocal),
      date_range_days: dateRangeDays
    }
    await postJSON('/plugins/requery/scheduler/config', payload)
    await refreshRequeryStatusAndConfig()
    setSuccess('定时任务配置已更新')
  } catch (error) {
    setError(`更新定时任务失败: ${error.message}`)
  } finally {
    schedulerSaving.value = false
  }
}

function scheduleSchedulerConfigUpdate() {
  if (schedulerTimerId) {
    window.clearTimeout(schedulerTimerId)
  }
  schedulerTimerId = window.setTimeout(() => {
    updateSchedulerConfig()
  }, 1500)
}

async function saveAllShuntRules() {
  if (!(await openConfirm('确定要保存所有分流规则吗？'))) {
    return
  }
  requeryAction.value = 'save-rules'
  try {
    const results = await Promise.allSettled(
      saveRulePaths.map((path) => requestResponse(`/plugins/${path}`))
    )
    const failed = results.filter((result) => result.status === 'rejected').length
    if (failed > 0) {
      setError(`部分规则保存失败 (${failed}/${results.length})`)
      return
    }
    setSuccess('所有分流规则已成功保存')
  } catch (error) {
    setError(`保存分流规则失败: ${error.message}`)
  } finally {
    requeryAction.value = ''
  }
}

async function clearAllShuntRules() {
  if (!(await openConfirm('确定要清空所有动态生成的分流规则吗？此操作不可撤销。', { tone: 'danger' }))) {
    return
  }
  requeryAction.value = 'clear-rules'
  try {
    await Promise.allSettled(
      flushRulePaths.map((path) => requestResponse(`/plugins/${path}`))
    )
    await refreshListStats()
    setSuccess('所有分流规则已清空')
  } catch (error) {
    setError(`清空分流规则失败: ${error.message}`)
  } finally {
    requeryAction.value = ''
  }
}

async function reloadAll() {
  loading.value = true
  clearTopNotice()
  try {
    await Promise.all([
      refreshCacheStats(),
      refreshRequeryStatusAndConfig(),
      refreshListStats()
    ])
  } finally {
    loading.value = false
  }
}

onMounted(() => {
  reloadAll()
  window.addEventListener('mosdns-log-refresh', reloadAll)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', reloadAll)
  if (schedulerTimerId) {
    window.clearTimeout(schedulerTimerId)
    schedulerTimerId = 0
  }
  if (requeryPollTimerId) {
    window.clearInterval(requeryPollTimerId)
    requeryPollTimerId = 0
  }
  if (requeryPendingTimerId) {
    window.clearTimeout(requeryPendingTimerId)
    requeryPendingTimerId = 0
  }
  if (dataViewSearchTimerId) {
    window.clearTimeout(dataViewSearchTimerId)
    dataViewSearchTimerId = 0
  }
})
</script>

<template>
  <section class="data-panel">
    <DataCachePanel
      v-if="showCachePanel"
      :cache-clearing-all="cacheClearingAll"
      :cache-clearing-by-tag="cacheClearingByTag"
      :cache-rows="cacheRows"
      @clear-all="clearAllCaches"
      @open-cache="openDataViewForCache"
      @clear-cache="clearCacheRow"
    />

    <div
      v-if="showListStatsPanel || showRequeryPanel"
      :class="showAllPanels ? 'data-inline-row' : 'data-inline-row data-inline-row-single'"
    >
    <DataListStatsPanel
      v-if="showListStatsPanel"
      :last-run-domain-count-text="lastRunDomainCountText"
      :list-stats="listStats"
      @open-list="openDataViewForList"
    />

    <DataRequeryPanel
      v-if="showRequeryPanel"
      :is-requery-running="isRequeryRunning"
      :last-run-error-text="lastRunErrorText"
      :last-run-text="lastRunText"
      :requery-action="requeryAction"
      :requery-available="requeryAvailable"
      :requery-load-error="requeryLoadError"
      :requery-progress="requeryProgress"
      :requery-status-meta="requeryStatusMeta"
      :scheduler-form="schedulerForm"
      :scheduler-saving="schedulerSaving"
      @trigger="triggerRequery"
      @cancel="cancelRequery"
      @update-scheduler="updateSchedulerConfig"
      @schedule-scheduler-update="scheduleSchedulerConfigUpdate"
      @save-rules="saveAllShuntRules"
      @clear-rules="clearAllShuntRules"
    />
    </div>

    <DataViewModal
      v-if="dataView.open"
      :data-view="dataView"
      @close="closeDataView"
      @search-input="onDataViewSearchInput"
      @load-more="loadMoreDataView"
    />
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getText, postJSON } from '../api/http'
import { openConfirm } from '../utils/confirm'

const loading = ref(false)
const errorMessage = ref('')
const successMessage = ref('')

const specialGroups = ref([])
const cacheRows = ref([])
const cacheRefreshing = ref(false)
const cacheClearingAll = ref(false)
const cacheClearingByTag = reactive({})

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
  { key: 'cache_all', name: '全部缓存 (兼容)', tag: 'cache_all' },
  { key: 'cache_cn', name: '国内缓存', tag: 'cache_cn' },
  { key: 'cache_node', name: '节点缓存', tag: 'cache_node' },
  { key: 'cache_google', name: '国外缓存 (兼容)', tag: 'cache_google' },
  { key: 'cache_all_noleak', name: '全部缓存 (安全)', tag: 'cache_all_noleak' },
  { key: 'cache_google_node', name: '国外缓存 (安全)', tag: 'cache_google_node' },
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
    if (durationMs > 0 && durationMs < 1000) {
      return `完成于 ${formatRelativeTime(status.last_run_end_time)} (耗时 <1秒)`
    }
    const durationSeconds = Math.max(0, Math.round(durationMs / 1000))
    return `完成于 ${formatRelativeTime(status.last_run_end_time)} (耗时 ${durationSeconds}秒)`
  }
  return `开始于 ${formatRelativeTime(status.last_run_start_time)}`
})

const lastRunDomainCountText = computed(() => {
  const count = Number(requeryStatus.value?.last_run_domain_count)
  if (Number.isFinite(count) && count >= 0) {
    return `${count.toLocaleString()} 条`
  }
  return '--'
})

function setError(message) {
  successMessage.value = ''
  errorMessage.value = message
}

function setSuccess(message) {
  errorMessage.value = ''
  successMessage.value = message
}

function isZeroTime(value) {
  return String(value || '').startsWith('0001-01-01')
}

function formatRelativeTime(value) {
  if (!value) {
    return '-'
  }
  const ts = new Date(value).getTime()
  if (!Number.isFinite(ts)) {
    return String(value)
  }
  const diffSeconds = Math.max(0, Math.floor((Date.now() - ts) / 1000))
  if (diffSeconds < 5) {
    return '刚刚'
  }
  if (diffSeconds < 60) {
    return `${diffSeconds}秒前`
  }
  if (diffSeconds < 3600) {
    return `${Math.floor(diffSeconds / 60)}分钟前`
  }
  if (diffSeconds < 86400) {
    return `${Math.floor(diffSeconds / 3600)}小时前`
  }
  return new Date(value).toLocaleString('zh-CN', { hour12: false })
}

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
  errorMessage.value = ''
  successMessage.value = ''
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
  <section class="panel data-panel">
    <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
    <p v-if="successMessage && !errorMessage" class="msg success">{{ successMessage }}</p>

    <section class="panel sub-panel data-module">
      <header class="panel-header">
        <div>
          <h3>缓存管理</h3>
        </div>
        <div class="actions">
          <button class="btn danger" :disabled="cacheClearingAll" @click="clearAllCaches">
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

    <div class="data-inline-row">
    <section class="panel sub-panel data-module data-inline-module domain-stats-module">
      <header class="panel-header">
        <div>
          <h3>域名列表统计</h3>
          <p class="muted">展示动态分流列表当前条目数。</p>
        </div>
      </header>

      <div class="table-wrap domain-stats-table-wrap data-scroll-wrap">
        <table class="domain-stats-table">
          <thead>
            <tr>
              <th>列表名称</th>
              <th>条目数</th>
            </tr>
          </thead>
          <tbody>
            <tr v-for="row in listStats" :key="row.key">
              <td>{{ row.name }}</td>
              <td>
                <span v-if="row.error" class="mono">{{ row.error }}</span>
                <span v-else-if="row.count === null">--</span>
                <button v-else class="btn-link" type="button" @click="openDataViewForList(row)">
                  {{ Number(row.count).toLocaleString() }}
                </button>
              </td>
            </tr>
            <tr>
              <td><strong>刷新域名</strong></td>
              <td><strong class="mono">{{ lastRunDomainCountText }}</strong></td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <section class="panel sub-panel data-module data-inline-module requery-module">
      <header class="panel-header">
        <div>
          <h3>刷新分流缓存</h3>
          <p class="muted">任务状态、进度与定时配置。</p>
        </div>
      </header>

      <p v-if="requeryLoadError" class="msg error">{{ requeryLoadError }}</p>

      <template v-if="requeryAvailable">
        <div class="requery-status-head">
          <div class="requery-status-item">
            <strong>当前状态</strong>
            <span class="requery-status-chip" :class="requeryStatusMeta.className">{{ requeryStatusMeta.text }}</span>
          </div>
          <div class="requery-status-item">
            <strong>上次运行</strong>
            <span>{{ lastRunText }}</span>
          </div>
          <div class="actions">
            <button
              v-if="!isRequeryRunning"
              class="btn primary"
              :disabled="requeryAction === 'trigger'"
              @click="triggerRequery"
            >
              {{ requeryAction === 'trigger' ? '启动中...' : '开始全新任务' }}
            </button>
            <button
              v-else
              class="btn danger"
              :disabled="requeryAction === 'cancel'"
              @click="cancelRequery"
            >
              {{ requeryAction === 'cancel' ? '取消中...' : '取消任务' }}
            </button>
          </div>
        </div>

        <div v-if="isRequeryRunning" class="requery-progress-wrap">
          <div class="requery-progress-bar">
            <div class="requery-progress-bar-fill" :style="{ width: `${requeryProgress.percent}%` }"></div>
            <span class="requery-progress-text">
              {{ Math.floor(requeryProgress.percent) }}% ({{ requeryProgress.processed.toLocaleString() }} / {{ requeryProgress.total.toLocaleString() }})
            </span>
          </div>
        </div>

        <div class="requery-scheduler">
          <div class="scheduler-row">
            <span>启用定时任务</span>
            <label class="switch">
              <input v-model="schedulerForm.enabled" type="checkbox" @change="updateSchedulerConfig" />
              <span class="slider"></span>
            </label>
          </div>
          <div class="scheduler-grid">
            <label>首次执行时间</label>
            <input
              class="scheduler-field"
              v-model="schedulerForm.startDatetimeLocal"
              type="datetime-local"
              :disabled="!schedulerForm.enabled"
              @change="scheduleSchedulerConfigUpdate"
            />

            <label>间隔 (分钟)</label>
            <input
              class="scheduler-field"
              v-model.number="schedulerForm.intervalMinutes"
              type="number"
              min="1"
              :disabled="!schedulerForm.enabled"
              @change="scheduleSchedulerConfigUpdate"
            />

            <label>域名刷新天数</label>
            <input
              class="scheduler-field"
              v-model.number="schedulerForm.dateRangeDays"
              type="number"
              min="1"
              @change="scheduleSchedulerConfigUpdate"
            />
          </div>
          <p class="muted">{{ schedulerSaving ? '定时配置保存中...' : '修改后自动保存配置。' }}</p>
        </div>

        <div class="requery-important">
          <strong>重要操作</strong>
          <div class="actions">
            <button class="btn primary" :disabled="requeryAction === 'save-rules'" @click="saveAllShuntRules">
              {{ requeryAction === 'save-rules' ? '保存中...' : '保存分流规则' }}
            </button>
            <button class="btn danger" :disabled="requeryAction === 'clear-rules'" @click="clearAllShuntRules">
              {{ requeryAction === 'clear-rules' ? '清空中...' : '清空分流规则' }}
            </button>
          </div>
        </div>

      </template>
    </section>
    </div>

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

        <div v-if="dataView.mode === 'domain'" class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>次数</th>
                <th>最后日期</th>
                <th>域名</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="dataView.loading">
                <td colspan="3" class="empty">加载中...</td>
              </tr>
              <tr v-else-if="dataView.entries.length === 0">
                <td colspan="3" class="empty">没有匹配的条目</td>
              </tr>
              <tr v-for="(item, index) in dataView.entries" :key="`domain-entry-${index}`">
                <td>{{ item.count }}</td>
                <td>{{ item.date }}</td>
                <td>{{ item.domain }}</td>
              </tr>
            </tbody>
          </table>
        </div>

        <div v-else class="cache-entry-list">
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

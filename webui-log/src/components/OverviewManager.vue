<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getText } from '../api/http'
import DnsOverviewCard from './dashboard/DnsOverviewCard.vue'
import { clearTopNotice, setError, setSuccess } from '../utils/notice'
import { formatDateTime } from '../utils/time'
import {
  applyUpstreamStatsBaseline,
  loadUpstreamStatsBaseline,
  orderUpstreamGroups,
  parseUpstreamStatsMetrics,
  protocolDisplayLabel,
  saveUpstreamStatsBaseline,
  upstreamAddressDisplay,
  upstreamGroupDisplay
} from '../utils/upstreamStats'

const props = defineProps({
  showSystemSummary: {
    type: Boolean,
    default: false
  }
})

const HISTORY_KEY = 'mosdnsHistory'
const HISTORY_LENGTH = 60

const loading = ref(false)
const lastUpdatedText = ref('--')

const stats = reactive({
  totalQueries: 0,
  averageDurationMs: 0
})

const systemSummary = reactive({
  startTime: 0,
  cpuTime: 0,
  residentMemory: 0,
  networkReceiveBytes: 0,
  networkTransmitBytes: 0,
  gcCount: 0,
  gcDurationSeconds: 0,
  lastGcTime: 0,
  goVersion: 'N/A',
  threads: 0,
  openFds: 0,
  goroutines: 0
})

const history = reactive({
  totalQueries: [],
  avgDuration: [],
  timestamps: []
})

const aliases = ref({})
const topDomains = ref([])
const topClients = ref([])
const slowestQueries = ref([])
const domainSetRank = ref([])
const specialGroups = ref([])
const upstreamConfig = ref({})
const upstreamMetricsText = ref('')
const dnsRoutingMode = ref('')
const upstreamStatsBaseline = ref(loadUpstreamStatsBaseline())
const upstreamStatsResetting = ref(false)
const slowDetailOpen = ref(false)
const selectedSlowQuery = ref(null)
const domainSetRankSource = ref('effective_tag')
const rankingDetail = reactive({
  open: false,
  type: 'domain',
  filterField: 'q',
  key: '',
  title: '',
  headline: '',
  subline: '',
  count: 0,
  percent: 0,
  loading: false,
  logs: []
})
const overviewGridRef = ref(null)
const visibleOverviewRows = ref(7)

const OVERVIEW_MIN_ROWS = 5
const OVERVIEW_MAX_ROWS = 15
const OVERVIEW_CARD_CHROME = 44
const OVERVIEW_TABLE_HEAD = 42
const OVERVIEW_TABLE_ROW = 42
const OVERVIEW_VIEWPORT_BOTTOM_GAP = 28

const DONUT_COLORS = ['#6d9dff', '#f778ba', '#2dd4bf', '#fb923c', '#a78bfa', '#fde047', '#ff8c8c', '#ef4444', '#f97316', '#f59e0b', '#84cc16', '#10b981', '#06b6d4', '#3b82f6', '#6366f1', '#8b5cf6', '#d946ef', '#f43f5e', '#64748b']
const DONUT_RADIUS = 48
const DONUT_CIRCUMFERENCE = 2 * Math.PI * DONUT_RADIUS

const sparklineTotal = computed(() => generateSparklineSVG(history.totalQueries, false, 300, 60, 'spark-total'))
const sparklineAvg = computed(() => generateSparklineSVG(history.avgDuration, true, 300, 60, 'spark-avg'))
const mergedSparkline = computed(() => generateDualSparklineSVG(history.totalQueries, history.avgDuration, history.timestamps))
const domainSetRows = computed(() => {
  const total = domainSetRank.value.reduce((sum, item) => sum + Number(item?.count || 0), 0)
  return domainSetRank.value.map((item, index) => {
    const count = Number(item?.count || 0)
    const percent = total > 0 ? (count / total) * 100 : 0
    return {
      key: item?.key || '-',
      count,
      percent,
      color: DONUT_COLORS[index % DONUT_COLORS.length]
    }
  })
})
const domainSetTotal = computed(() => domainSetRows.value.reduce((sum, item) => sum + item.count, 0))
const rankingDetailSummaryCards = computed(() => {
  if (!rankingDetail.open) {
    return []
  }

  const logs = Array.isArray(rankingDetail.logs) ? rankingDetail.logs : []
  const latestTime = findLatestLogTime(logs)
  const averageLatency = computeAverageLatency(logs)
  const maxLatency = computeMaxLatency(logs)
  const uniqueDomainCount = countUniqueValues(logs, (item) => String(item?.query_name || '').trim())
  const uniqueClientCount = countUniqueValues(logs, (item) => normalizeIP(item?.client_ip))
  const topClient = topLogValue(logs, (item) => normalizeIP(item?.client_ip))
  const topRule = topLogValue(logs, (item) => getRuleDetailValue(item))
  const topDomain = topLogValue(logs, (item) => String(item?.query_name || '').trim())
  const topUpstream = topLogValue(logs, (item) => getUpstreamDetailValue(item))

  const cards = [
    {
      key: 'count',
      title: rankingDetail.type === 'rule' ? '命中次数' : '查询次数',
      value: Number(rankingDetail.count || 0).toLocaleString()
    },
    {
      key: 'percent',
      title: '占总查询比例',
      value: formatPercent(rankingDetail.percent)
    },
    {
      key: 'latest',
      title: rankingDetail.type === 'rule' ? '最近命中时间' : '最近查询时间',
      value: latestTime ? formatTime(latestTime) : '--'
    },
    {
      key: 'avg',
      title: '平均耗时',
      value: logs.length > 0 ? formatDuration(averageLatency) : '--'
    },
    {
      key: 'max',
      title: '最大耗时',
      value: logs.length > 0 ? formatDuration(maxLatency) : '--'
    }
  ]

  if (rankingDetail.type === 'domain') {
    cards.push(
      {
        key: 'client',
        title: '主要客户端',
        value: topClient ? getClientDisplay(topClient.key) : '--'
      },
      {
        key: 'rule',
        title: '主要分流规则',
        value: topRule ? getDetailRuleLabel(topRule.key) : '--'
      }
    )
  } else if (rankingDetail.type === 'client') {
    cards.push(
      {
        key: 'domains',
        title: '唯一域名数量',
        value: uniqueDomainCount.toLocaleString()
      },
      {
        key: 'rule',
        title: '主要分流规则',
        value: topRule ? getDetailRuleLabel(topRule.key) : '--'
      },
      {
        key: 'upstream',
        title: '主要上游',
        value: topUpstream ? topUpstream.key : '--'
      }
    )
  } else if (rankingDetail.type === 'rule') {
    cards.push(
      {
        key: 'domains',
        title: '唯一域名数量',
        value: uniqueDomainCount.toLocaleString()
      },
      {
        key: 'clients',
        title: '唯一客户端数量',
        value: uniqueClientCount.toLocaleString()
      },
      {
        key: 'domain',
        title: '主要域名',
        value: topDomain ? topDomain.key : '--'
      },
      {
        key: 'client',
        title: '主要客户端',
        value: topClient ? getClientDisplay(topClient.key) : '--'
      }
    )
  }

  return cards
})
const domainSetSegments = computed(() => {
  const total = domainSetTotal.value
  let offset = 0
  return domainSetRows.value
    .filter((item) => item.count > 0)
    .map((item) => {
      const ratio = total > 0 ? item.count / total : 0
      const segment = {
        key: item.key,
        color: item.color,
        dasharray: `${(ratio * DONUT_CIRCUMFERENCE).toFixed(2)} ${(DONUT_CIRCUMFERENCE - ratio * DONUT_CIRCUMFERENCE).toFixed(2)}`,
        dashoffset: (-offset * DONUT_CIRCUMFERENCE).toFixed(2)
      }
      offset += ratio
      return segment
    })
})
const overviewLayoutVars = computed(() => {
  const rows = visibleOverviewRows.value
  const listHeight = OVERVIEW_TABLE_HEAD + rows * OVERVIEW_TABLE_ROW
  const cardHeight = OVERVIEW_CARD_CHROME + listHeight
  return {
    '--overview-visible-rows': String(rows),
    '--overview-list-max-height': `${listHeight}px`,
    '--overview-card-min-height': `${cardHeight}px`
  }
})
const slowDetailActionFields = computed(() => {
  const log = selectedSlowQuery.value
  if (!log) {
    return {}
  }
  return {
    client_ip: {
      value: getClientDisplay(log.client_ip),
      copyValue: normalizeIP(log.client_ip),
      filterValue: normalizeIP(log.client_ip),
      exact: true,
      mono: false
    },
    query_name: {
      value: log.query_name || '-',
      copyValue: String(log.query_name || '').trim(),
      filterValue: String(log.query_name || '').trim(),
      exact: false,
      mono: false
    },
    domain_set: {
      value: getDetailRuleLabel(log.domain_set || log.effective_tag || '-'),
      copyValue: String(log.domain_set || '').trim(),
      filterValue: String(log.domain_set || '').trim(),
      exact: true,
      mono: false
    },
    trace_id: {
      value: log.trace_id || '-',
      copyValue: String(log.trace_id || '').trim(),
      filterValue: String(log.trace_id || '').trim(),
      exact: true,
      mono: true
    }
  }
})
const systemSummaryCards = computed(() => {
  if (!props.showSystemSummary) {
    return []
  }
  return [
    {
      key: 'start-time',
      title: '启动时间',
      value: systemSummary.startTime
        ? new Date(systemSummary.startTime * 1000).toLocaleString('zh-CN', { hour12: false })
        : '--',
      tone: 'ok'
    },
    {
      key: 'cpu-time',
      title: 'CPU 时间',
      value: formatSeconds(systemSummary.cpuTime, 2),
      tone: 'neutral'
    },
    {
      key: 'memory',
      title: '常驻内存 (RSS)',
      value: formatBytes(systemSummary.residentMemory),
      tone: 'neutral'
    },
    {
      key: 'network-receive',
      title: '系统累计接收流量',
      value: formatBytes(systemSummary.networkReceiveBytes),
      tone: 'neutral'
    },
    {
      key: 'network-transmit',
      title: '系统累计发送流量',
      value: formatBytes(systemSummary.networkTransmitBytes),
      tone: 'neutral'
    },
    {
      key: 'gc-count',
      title: 'GC 次数',
      value: Number(systemSummary.gcCount || 0).toLocaleString(),
      tone: 'neutral'
    },
    {
      key: 'gc-duration',
      title: 'GC 累计耗时',
      value: formatSeconds(systemSummary.gcDurationSeconds, 3),
      tone: 'neutral'
    },
    {
      key: 'last-gc',
      title: '上次 GC 时间',
      value: systemSummary.lastGcTime
        ? new Date(systemSummary.lastGcTime * 1000).toLocaleString('zh-CN', { hour12: false })
        : '--',
      tone: 'neutral'
    },
    {
      key: 'go-version',
      title: 'Go 版本',
      value: systemSummary.goVersion || '--',
      tone: 'ok'
    },
    {
      key: 'threads',
      title: '线程数',
      value: Number(systemSummary.threads || 0).toLocaleString(),
      tone: 'neutral'
    },
    {
      key: 'open-fds',
      title: '打开文件描述符',
      value: Number(systemSummary.openFds || 0).toLocaleString(),
      tone: 'neutral'
    },
    {
      key: 'goroutines',
      title: 'go_goroutines',
      value: Number(systemSummary.goroutines || 0).toLocaleString(),
      tone: 'neutral'
    }
  ]
})
const upstreamStatsMap = computed(() => applyUpstreamStatsBaseline(
  parseUpstreamStatsMetrics(upstreamMetricsText.value),
  upstreamStatsBaseline.value.snapshots,
  {
    currentProcessStartTime: parseSystemMetrics(upstreamMetricsText.value).startTime,
    baselineProcessStartTime: upstreamStatsBaseline.value.processStartTime
  }
))
const upstreamStatSections = computed(() => {
  const configuredGroups = Object.keys(upstreamConfig.value || {})
  const orderedGroups = orderUpstreamGroups(configuredGroups, specialGroups.value)
  const sections = []

  orderedGroups.forEach((group) => {
    if (dnsRoutingMode.value === 'B' && group === 'nocnfake') {
      return
    }
    const rows = Array.isArray(upstreamConfig.value?.[group]) ? upstreamConfig.value[group] : []
    const enabledRows = rows
      .filter((item) => Boolean(item?.enabled))
      .map((item, index) => {
        const key = `${group}|${item?.tag || ''}`
        const stats = upstreamStatsMap.value[key] || {}
        const queryNumber = Number(stats.queryTotal || 0)
        const winnerNumber = Number(stats.winnerTotal || 0)
        const errorNumber = Number(stats.errorTotal || 0)
        const latencySum = Number(stats.latencySum || 0)
        const latencyCount = Number(stats.latencyCount || 0)
        const avgLatencyNumber = latencyCount > 0 ? latencySum / latencyCount : 0
        const winnerRateNumber = queryNumber > 0 ? (winnerNumber / queryNumber) * 100 : 0
        const errorRateNumber = queryNumber > 0 ? (errorNumber / queryNumber) * 100 : 0

        return {
          key: `${group}-${item?.tag || index}`,
          name: String(item?.tag || '-'),
          typeLabel: protocolDisplayLabel(item?.protocol),
          address: upstreamAddressDisplay(item || {}),
          avgLatencyText: latencyCount > 0 ? avgLatencyNumber.toFixed(2) : '-',
          avgLatencyNumber,
          queryText: queryNumber.toLocaleString(),
          queryNumber,
          winnerRateText: `${winnerRateNumber.toFixed(2)}%`,
          winnerRateNumber,
          errorText: errorNumber.toLocaleString(),
          errorNumber,
          errorRateText: `${errorRateNumber.toFixed(2)}%`,
          errorRateNumber
        }
      })

    if (enabledRows.length === 0) {
      return
    }

    const groupMeta = upstreamGroupDisplay(group, specialGroups.value)
    sections.push({
      key: group,
      title: groupMeta.title,
      tone: groupMeta.tone,
      rows: enabledRows
    })
  })

  return sections
})

function clamp(value, min, max) {
  return Math.min(Math.max(value, min), max)
}

function updateOverviewRows() {
  const grid = overviewGridRef.value
  if (!grid || typeof window === 'undefined') {
    return
  }
  if (window.innerWidth <= 1100) {
    visibleOverviewRows.value = OVERVIEW_MIN_ROWS
    return
  }

  const gridRect = grid.getBoundingClientRect()
  const availableHeight = window.innerHeight - gridRect.top - OVERVIEW_VIEWPORT_BOTTOM_GAP
  const computedRows = Math.floor((availableHeight - OVERVIEW_CARD_CHROME - OVERVIEW_TABLE_HEAD) / OVERVIEW_TABLE_ROW)
  visibleOverviewRows.value = clamp(computedRows, OVERVIEW_MIN_ROWS, OVERVIEW_MAX_ROWS)
}

function clearMessages() {
  clearTopNotice()
}

function normalizeIP(ip) {
  return String(ip || '').replace(/^::ffff:/, '').trim()
}

function normalizeAliasMap(input) {
  const output = {}
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    return output
  }
  Object.entries(input).forEach(([key, value]) => {
    const ip = normalizeIP(key)
    const alias = String(value || '').trim()
    if (ip && alias) {
      output[ip] = alias
    }
  })
  return output
}

function getClientDisplay(ip) {
  const normalized = normalizeIP(ip)
  if (!normalized) {
    return '-'
  }
  const alias = aliases.value[normalized]
  return alias || normalized
}

function hasAlias(ip) {
  const normalized = normalizeIP(ip)
  return Boolean(normalized && aliases.value[normalized] && aliases.value[normalized] !== normalized)
}

function formatDuration(value) {
  const num = Number(value || 0)
  return `${num.toFixed(2)} ms`
}

function formatCompactDuration(value) {
  const num = Number(value || 0)
  if (num >= 100) {
    return String(Math.round(num))
  }
  if (num >= 10) {
    return num.toFixed(1)
  }
  return num.toFixed(2)
}

function formatBytes(bytes) {
  const value = Number(bytes || 0)
  if (!Number.isFinite(value) || value <= 0) {
    return '--'
  }
  if (value >= 1024 ** 3) {
    return `${(value / 1024 ** 3).toFixed(2)} GB`
  }
  if (value >= 1024 ** 2) {
    return `${(value / 1024 ** 2).toFixed(2)} MB`
  }
  if (value >= 1024) {
    return `${(value / 1024).toFixed(2)} KB`
  }
  return `${value.toFixed(0)} B`
}

function formatSeconds(value, fractionDigits = 2) {
  const num = Number(value || 0)
  if (!Number.isFinite(num) || num < 0) {
    return '--'
  }
  return `${num.toFixed(fractionDigits)} 秒`
}

function formatTime(value) {
  return formatDateTime(value)
}

function getRuleLabel(key) {
  if (!key) {
    return '-'
  }
  const match = String(key).match(/^特殊上游(\d+)$/)
  if (!match) {
    return key
  }
  const slot = Number(match[1])
  const group = specialGroups.value.find((item) => Number(item.slot) === slot)
  return group?.name || key
}

function formatPercent(value) {
  return `${Number(value || 0).toFixed(1)}%`
}

function resetUpstreamStatsBaseline() {
  upstreamStatsResetting.value = true
  try {
    const snapshots = parseUpstreamStatsMetrics(upstreamMetricsText.value)
    upstreamStatsBaseline.value = saveUpstreamStatsBaseline(snapshots, parseSystemMetrics(upstreamMetricsText.value).startTime)
    setSuccess('上游统计已重置')
  } catch (error) {
    setError(`重置上游统计失败: ${error.message}`)
  } finally {
    upstreamStatsResetting.value = false
  }
}

function formatResponseFlags(flags) {
  if (!flags || typeof flags !== 'object') {
    return '-'
  }
  const enabled = ['ra', 'aa', 'tc', 'ad', 'cd']
    .filter((key) => Boolean(flags[key]))
    .map((key) => key.toUpperCase())
  return enabled.length > 0 ? enabled.join(', ') : '-'
}

function getDetailRuleLabel(value) {
  if (!value) {
    return '-'
  }
  return getRuleLabel(value)
}

function getRuleDetailValue(log) {
  return String(log?.effective_tag || log?.domain_set || '').trim()
}

function getUpstreamDetailValue(log) {
  return String(log?.selected_upstream || log?.final_upstream || log?.upstream_group || '-').trim()
}

function findLatestLogTime(logs) {
  return logs.reduce((latest, item) => {
    const timestamp = new Date(item?.query_time || 0).getTime()
    if (!Number.isFinite(timestamp) || timestamp <= 0) {
      return latest
    }
    return Math.max(latest, timestamp)
  }, 0)
}

function computeAverageLatency(logs) {
  if (!Array.isArray(logs) || logs.length === 0) {
    return 0
  }
  const total = logs.reduce((sum, item) => sum + Number(item?.duration_ms || 0), 0)
  return total / logs.length
}

function computeMaxLatency(logs) {
  if (!Array.isArray(logs) || logs.length === 0) {
    return 0
  }
  return logs.reduce((max, item) => Math.max(max, Number(item?.duration_ms || 0)), 0)
}

function countUniqueValues(logs, selector) {
  const values = new Set()
  logs.forEach((item) => {
    const key = String(selector(item) || '').trim()
    if (key) {
      values.add(key)
    }
  })
  return values.size
}

function topLogValue(logs, selector) {
  const counts = new Map()
  logs.forEach((item) => {
    const key = String(selector(item) || '').trim()
    if (!key) {
      return
    }
    counts.set(key, (counts.get(key) || 0) + 1)
  })
  let bestKey = ''
  let bestCount = 0
  counts.forEach((count, key) => {
    if (count > bestCount) {
      bestKey = key
      bestCount = count
    }
  })
  return bestKey ? { key: bestKey, count: bestCount } : null
}

function calculateRankingPercent(count) {
  const total = Number(stats.totalQueries || 0)
  if (total <= 0) {
    return 0
  }
  return (Number(count || 0) / total) * 100
}

function openSlowDetail(item) {
  selectedSlowQuery.value = item || null
  slowDetailOpen.value = Boolean(item)
}

function closeSlowDetail() {
  slowDetailOpen.value = false
  selectedSlowQuery.value = null
}

async function copyText(text) {
  const value = String(text || '').trim()
  if (!value) {
    setError('没有可复制的内容')
    return
  }
  try {
    if (navigator.clipboard?.writeText && window.isSecureContext) {
      await navigator.clipboard.writeText(value)
    } else {
      const textArea = document.createElement('textarea')
      textArea.value = value
      textArea.setAttribute('readonly', 'readonly')
      textArea.style.position = 'fixed'
      textArea.style.left = '-9999px'
      document.body.appendChild(textArea)
      textArea.select()
      document.execCommand('copy')
      document.body.removeChild(textArea)
    }
    setSuccess('已复制到剪贴板')
  } catch (error) {
    setError(`复制失败: ${error.message}`)
  }
}

function getSlowDetailActionField(key) {
  return slowDetailActionFields.value?.[key] || null
}

function openLogFilterFromOverview(value, exact = false) {
  const text = String(value || '').trim()
  if (!text) {
    setError('没有可筛选的内容')
    return
  }
  closeSlowDetail()
  closeRankingDetail()
  window.dispatchEvent(new CustomEvent('mosdns-open-log-filter', {
    detail: {
      value: text,
      exact: Boolean(exact)
    }
  }))
}

async function loadRankingDetailLogs() {
  const detailKey = rankingDetail.key
  const filterField = rankingDetail.filterField
  rankingDetail.loading = true
  try {
    const params = new URLSearchParams({
      page: '1',
      limit: '50'
    })
    if (filterField === 'q') {
      params.set('q', detailKey)
      params.set('exact', 'true')
    } else {
      params.set(filterField, detailKey)
    }
    const data = await getJSON(`/api/v2/audit/logs?${params.toString()}`)
    if (!rankingDetail.open || rankingDetail.key !== detailKey || rankingDetail.filterField !== filterField) {
      return
    }
    rankingDetail.logs = Array.isArray(data?.logs) ? data.logs : []
  } catch (error) {
    rankingDetail.logs = []
    setError(`加载详情失败: ${error.message}`)
  } finally {
    if (rankingDetail.key === detailKey && rankingDetail.filterField === filterField) {
      rankingDetail.loading = false
    }
  }
}

function openRankingDetail(type, item) {
  const rawKey = String(item?.key || '').trim()
  const count = Number(item?.count || 0)
  if (!rawKey) {
    return
  }
  rankingDetail.type = type
  rankingDetail.key = rawKey
  rankingDetail.count = count
  rankingDetail.percent = calculateRankingPercent(count)
  rankingDetail.logs = []
  rankingDetail.loading = false

  if (type === 'domain') {
    rankingDetail.title = '域名详情'
    rankingDetail.headline = rawKey
    rankingDetail.subline = ''
    rankingDetail.filterField = 'q'
  } else if (type === 'client') {
    rankingDetail.title = '客户端详情'
    rankingDetail.headline = getClientDisplay(rawKey)
    rankingDetail.subline = hasAlias(rawKey) ? normalizeIP(rawKey) : ''
    rankingDetail.filterField = 'client_ip'
    rankingDetail.key = normalizeIP(rawKey)
  } else {
    const filterField = domainSetRankSource.value === 'domain_set' ? 'domain_set' : 'effective_tag'
    rankingDetail.title = '分流规则详情'
    rankingDetail.headline = getRuleLabel(rawKey)
    rankingDetail.subline = `${filterField}: ${rawKey}`
    rankingDetail.filterField = filterField
  }

  rankingDetail.open = true
  loadRankingDetailLogs()
}

function openTopDomainDetail(item) {
  openRankingDetail('domain', item)
}

function openTopClientDetail(item) {
  openRankingDetail('client', item)
}

function openDomainSetDetail(item) {
  openRankingDetail('rule', item)
}

function closeRankingDetail() {
  rankingDetail.open = false
  rankingDetail.key = ''
  rankingDetail.headline = ''
  rankingDetail.subline = ''
  rankingDetail.logs = []
  rankingDetail.loading = false
}

function loadHistory() {
  try {
    const saved = JSON.parse(localStorage.getItem(HISTORY_KEY) || 'null')
    history.totalQueries = Array.isArray(saved?.totalQueries) ? saved.totalQueries.map((item) => Number(item || 0)) : []
    history.avgDuration = Array.isArray(saved?.avgDuration) ? saved.avgDuration.map((item) => Number(item || 0)) : []
    history.timestamps = Array.isArray(saved?.timestamps) ? saved.timestamps.map((item) => Number(item || 0)) : []
  } catch {
    history.totalQueries = []
    history.avgDuration = []
    history.timestamps = []
  }
}

function saveHistory() {
  localStorage.setItem(HISTORY_KEY, JSON.stringify({
    totalQueries: history.totalQueries,
    avgDuration: history.avgDuration,
    timestamps: history.timestamps
  }))
}

function addHistoryPoint(totalQueriesValue, avgDurationValue) {
  history.totalQueries.push(Number(totalQueriesValue || 0))
  history.avgDuration.push(Number(avgDurationValue || 0))
  history.timestamps.push(Date.now())

  if (history.totalQueries.length > HISTORY_LENGTH) {
    history.totalQueries.shift()
  }
  if (history.avgDuration.length > HISTORY_LENGTH) {
    history.avgDuration.shift()
  }
  if (history.timestamps.length > HISTORY_LENGTH) {
    history.timestamps.shift()
  }
  saveHistory()
}

function parseSystemMetrics(metricsText) {
  const lines = String(metricsText || '').split('\n')
  const next = {
    startTime: 0,
    cpuTime: 0,
    residentMemory: 0,
    networkReceiveBytes: 0,
    networkTransmitBytes: 0,
    gcCount: 0,
    gcDurationSeconds: 0,
    lastGcTime: 0,
    goVersion: 'N/A',
    threads: 0,
    openFds: 0,
    goroutines: 0
  }
  lines.forEach((line) => {
    if (line.startsWith('process_start_time_seconds')) {
      next.startTime = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('process_cpu_seconds_total')) {
      next.cpuTime = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('process_resident_memory_bytes')) {
      next.residentMemory = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('process_network_receive_bytes_total')) {
      next.networkReceiveBytes = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('process_network_transmit_bytes_total')) {
      next.networkTransmitBytes = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_gc_duration_seconds_count')) {
      next.gcCount = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_gc_duration_seconds_sum')) {
      next.gcDurationSeconds = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_memstats_last_gc_time_seconds')) {
      next.lastGcTime = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_threads')) {
      next.threads = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('process_open_fds')) {
      next.openFds = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_goroutines')) {
      next.goroutines = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_info{version="')) {
      const match = line.match(/go_info\{version="([^"]+)"/)
      if (match?.[1]) {
        next.goVersion = match[1]
      }
    }
  })
  return next
}

function applySystemSummaryState({ metricsText = '' } = {}) {
  if (!props.showSystemSummary) {
    return
  }
  const metrics = parseSystemMetrics(metricsText)
  systemSummary.startTime = metrics.startTime
  systemSummary.cpuTime = metrics.cpuTime
  systemSummary.residentMemory = metrics.residentMemory
  systemSummary.networkReceiveBytes = metrics.networkReceiveBytes
  systemSummary.networkTransmitBytes = metrics.networkTransmitBytes
  systemSummary.gcCount = metrics.gcCount
  systemSummary.gcDurationSeconds = metrics.gcDurationSeconds
  systemSummary.lastGcTime = metrics.lastGcTime
  systemSummary.goVersion = metrics.goVersion
  systemSummary.threads = metrics.threads
  systemSummary.openFds = metrics.openFds
  systemSummary.goroutines = metrics.goroutines
}

function applyEWMA(values, alpha = 0.4) {
  if (!Array.isArray(values) || values.length < 2) {
    return values || []
  }
  const smoothed = [Number(values[0] || 0)]
  for (let index = 1; index < values.length; index += 1) {
    const curr = Number(values[index] || 0)
    smoothed[index] = alpha * curr + (1 - alpha) * smoothed[index - 1]
  }
  return smoothed
}

function generateSparklineSVG(values, isFloat = false, width = 300, height = 60, gradientId = 'sparkline-gradient') {
  if (!Array.isArray(values) || values.length < 2) {
    return ''
  }
  const data = applyEWMA(values.map((item) => Number(item || 0)), isFloat ? 0.3 : 0.4)
  const maxValue = Math.max(...data)
  const minValue = Math.min(...data)
  const range = maxValue - minValue === 0 ? 1 : maxValue - minValue

  const points = data.map((value, index) => {
    const x = (index / (data.length - 1)) * width
    const y = height - ((value - minValue) / range) * height
    return `${x.toFixed(2)},${y.toFixed(2)}`
  })
  const path = `M ${points.join(' L ')}`
  const areaPath = `${path} L ${width},${height} L 0,${height} Z`
  return `<svg viewBox="0 0 ${width} ${height}" preserveAspectRatio="none"><defs><linearGradient id="${gradientId}" x1="0%" y1="0%" x2="0%" y2="100%"><stop offset="0%" stop-color="var(--brand)" stop-opacity="0.45" /><stop offset="100%" stop-color="var(--brand)" stop-opacity="0" /></linearGradient></defs><path d="${areaPath}" fill="url(#${gradientId})" /><path d="${path}" fill="none" stroke="var(--brand)" stroke-width="2" /></svg>`
}

function generateDualSparklineSVG(totalValues, avgValues, timestamps) {
  if (!Array.isArray(totalValues) || !Array.isArray(avgValues) || totalValues.length < 2 || avgValues.length < 2) {
    return ''
  }

  const width = 1000
  const height = 250
  const padding = { top: 24, right: 54, bottom: 30, left: 54 }
  const chartWidth = width - padding.left - padding.right
  const chartHeight = height - padding.top - padding.bottom

  const totalData = applyEWMA(totalValues.map((item) => Number(item || 0)), 0.4)
  const avgData = applyEWMA(avgValues.map((item) => Number(item || 0)), 0.3)
  const totalMax = Math.max(...totalData, 1)
  const avgMax = Math.max(...avgData, 1)

  const points = (data, max) => data.map((value, index) => {
    const x = padding.left + (index / (data.length - 1)) * chartWidth
    const y = padding.top + chartHeight - (value / max) * chartHeight
    return `${x.toFixed(2)},${y.toFixed(2)}`
  })

  const totalPath = `M ${points(totalData, totalMax).join(' L ')}`
  const avgPath = `M ${points(avgData, avgMax).join(' L ')}`
  const totalArea = `${totalPath} L ${padding.left + chartWidth},${padding.top + chartHeight} L ${padding.left},${padding.top + chartHeight} Z`
  const avgArea = `${avgPath} L ${padding.left + chartWidth},${padding.top + chartHeight} L ${padding.left},${padding.top + chartHeight} Z`

  const startText = timestamps?.length ? new Date(Number(timestamps[0] || 0)).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''
  const endText = timestamps?.length ? new Date(Number(timestamps[timestamps.length - 1] || 0)).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' }) : ''

  return `<svg viewBox="0 0 ${width} ${height}" preserveAspectRatio="none">
      <defs>
        <linearGradient id="dual-total-grad" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="var(--brand)" stop-opacity="0.2"/><stop offset="1" stop-color="var(--brand)" stop-opacity="0"/></linearGradient>
        <linearGradient id="dual-avg-grad" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#f59e0b" stop-opacity="0.2"/><stop offset="1" stop-color="#f59e0b" stop-opacity="0"/></linearGradient>
      </defs>
      <g stroke="var(--line)" stroke-width="1" stroke-dasharray="4 4" opacity="0.5">
        <line x1="${padding.left}" y1="${padding.top}" x2="${width - padding.right}" y2="${padding.top}"/>
        <line x1="${padding.left}" y1="${padding.top + chartHeight / 2}" x2="${width - padding.right}" y2="${padding.top + chartHeight / 2}"/>
        <line x1="${padding.left}" y1="${padding.top + chartHeight}" x2="${width - padding.right}" y2="${padding.top + chartHeight}"/>
      </g>
      <path d="${totalArea}" fill="url(#dual-total-grad)" />
      <path d="${avgArea}" fill="url(#dual-avg-grad)" />
      <path d="${totalPath}" fill="none" stroke="var(--brand)" stroke-width="2.2" stroke-linecap="round" />
      <path d="${avgPath}" fill="none" stroke="#f59e0b" stroke-width="2.2" stroke-linecap="round" />
      <g fill="var(--ink-1)" font-size="11">
        <text x="${padding.left}" y="${height - 6}" text-anchor="start">${startText}</text>
        <text x="${width - padding.right}" y="${height - 6}" text-anchor="end">${endText}</text>
      </g>
    </svg>`
}

async function reloadOverview(showMessage = false) {
  loading.value = true
  clearTopNotice()
  if (showMessage) {
    clearTopNotice()
  }
  try {
    const domainSetRankPromise = getJSON('/api/v2/audit/rank/effective?limit=20')
      .then((data) => ({ data, source: 'effective_tag' }))
      .catch(async () => ({
        data: await getJSON('/api/v2/audit/rank/domain_set?limit=20'),
        source: 'domain_set'
      }))

    const [
      statsRes,
      topDomainsRes,
      topClientsRes,
      slowestRes,
      domainSetRes,
      specialGroupsRes,
      aliasesRes,
      upstreamConfigRes,
      metricsRes,
      dnsRoutingModeRes
    ] = await Promise.all([
      getJSON('/api/v2/audit/stats'),
      getJSON('/api/v2/audit/rank/domain?limit=20'),
      getJSON('/api/v2/audit/rank/client?limit=20'),
      getJSON('/api/v2/audit/rank/slowest?limit=20'),
      domainSetRankPromise,
      getJSON('/api/v1/special-groups'),
      getJSON('/plugins/clientname').catch(() => ({})),
      getJSON('/api/v1/upstream/config').catch(() => ({})),
      getText('/metrics').catch(() => ''),
      getText('/plugins/switch17/show').catch(() => '')
    ])

    stats.totalQueries = Number(statsRes?.total_queries || 0)
    stats.averageDurationMs = Number(statsRes?.average_duration_ms || 0)
    addHistoryPoint(stats.totalQueries, stats.averageDurationMs)

    topDomains.value = Array.isArray(topDomainsRes) ? topDomainsRes : []
    topClients.value = Array.isArray(topClientsRes) ? topClientsRes : []
    slowestQueries.value = Array.isArray(slowestRes) ? slowestRes : []
    domainSetRank.value = Array.isArray(domainSetRes?.data) ? domainSetRes.data : []
    domainSetRankSource.value = domainSetRes?.source || 'effective_tag'
    specialGroups.value = Array.isArray(specialGroupsRes) ? specialGroupsRes : []
    aliases.value = normalizeAliasMap(aliasesRes)
    upstreamConfig.value = upstreamConfigRes && typeof upstreamConfigRes === 'object' ? upstreamConfigRes : {}
    upstreamMetricsText.value = String(metricsRes || '')
    dnsRoutingMode.value = String(dnsRoutingModeRes || '').trim()
    applySystemSummaryState({ metricsText: metricsRes })
    lastUpdatedText.value = new Date().toLocaleString('zh-CN', { hour12: false })

    if (showMessage) {
      setSuccess('概览数据已刷新')
    }
  } catch (error) {
    setError(`加载概览失败: ${error.message}`)
  } finally {
    loading.value = false
    nextTick(() => {
      updateOverviewRows()
    })
  }
}

function handleGlobalRefresh() {
  reloadOverview(false)
}

onMounted(() => {
  loadHistory()
  reloadOverview(false)
  nextTick(() => {
    updateOverviewRows()
  })
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
  window.addEventListener('resize', updateOverviewRows)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
  window.removeEventListener('resize', updateOverviewRows)
})
</script>

<template>
  <section class="overview-page" :style="overviewLayoutVars">
    <DnsOverviewCard />

    <section class="panel sub-panel upstream-stats-panel">
      <header class="upstream-stats-head">
        <div class="upstream-stats-title-wrap">
          <h3>上游 DNS 统计</h3>
        </div>
        <button
          class="btn tiny secondary"
          type="button"
          :disabled="upstreamStatsResetting || loading"
          @click="resetUpstreamStatsBaseline"
        >
          {{ upstreamStatsResetting ? '重置中...' : '重置统计' }}
        </button>
      </header>
      <div class="table-wrap upstream-stats-table-wrap">
        <table class="upstream-stats-table">
          <thead>
            <tr>
              <th>类型</th>
              <th>名称</th>
              <th>地址</th>
              <th class="text-right">平均响应(ms)</th>
              <th class="text-right">请求数</th>
              <th class="text-right">采纳率</th>
              <th class="text-right">出错数</th>
              <th class="text-right">出错率</th>
            </tr>
          </thead>
          <tbody v-if="loading">
            <tr>
              <td colspan="8" class="empty">加载中...</td>
            </tr>
          </tbody>
          <tbody v-else-if="upstreamStatSections.length === 0">
            <tr>
              <td colspan="8" class="empty">暂无已启用上游统计</td>
            </tr>
          </tbody>
          <tbody v-for="section in upstreamStatSections" v-else :key="section.key">
            <tr class="upstream-stats-group-row" :class="`tone-${section.tone}`">
              <td colspan="8">
                <span class="upstream-stats-group-name">{{ section.title }}</span>
                <span class="upstream-stats-group-count">({{ section.rows.length }})</span>
              </td>
            </tr>
            <tr v-for="row in section.rows" :key="row.key">
              <td :title="row.typeLabel">
                <span class="upstream-stats-type-chip">{{ row.typeLabel }}</span>
              </td>
              <td class="upstream-stats-name-cell" :title="row.name">{{ row.name }}</td>
              <td class="mono upstream-stats-address-cell" :title="row.address">{{ row.address }}</td>
              <td class="text-right" :title="row.avgLatencyText === '-' ? '-' : `${row.avgLatencyText} ms`">
                {{ row.avgLatencyText }}
              </td>
              <td class="text-right">{{ row.queryText }}</td>
              <td class="text-right">{{ row.winnerRateText }}</td>
              <td class="text-right">{{ row.errorText }}</td>
              <td class="text-right">{{ row.errorRateText }}</td>
            </tr>
          </tbody>
        </table>
      </div>
    </section>

    <div ref="overviewGridRef" class="overview-grid">
      <section class="panel sub-panel overview-metric-module">
        <h3>Top 域名</h3>
        <div class="table-wrap overview-table-fit top-domains-fit module-scroll-list">
          <table>
            <thead>
              <tr>
                <th>域名</th>
                <th class="text-right">次数</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="topDomains.length === 0">
                <td colspan="2" class="empty">暂无数据</td>
              </tr>
              <tr
                v-for="item in topDomains"
                :key="`domain-${item.key}`"
                class="overview-click-row"
                @click="openTopDomainDetail(item)"
              >
                <td>
                  <div class="overview-row-main-cell">
                    <span class="overview-row-label">{{ item.key }}</span>
                    <span class="overview-row-hint">详情</span>
                  </div>
                </td>
                <td class="text-right">{{ Number(item.count || 0).toLocaleString() }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="panel sub-panel overview-metric-module">
        <h3>Top 客户端</h3>
        <div class="table-wrap overview-table-fit top-clients-fit module-scroll-list">
          <table>
            <thead>
              <tr>
                <th>客户端</th>
                <th class="text-right">次数</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="topClients.length === 0">
                <td colspan="2" class="empty">暂无数据</td>
              </tr>
              <tr
                v-for="item in topClients"
                :key="`client-${item.key}`"
                class="overview-click-row"
                @click="openTopClientDetail(item)"
              >
                <td>
                  <div class="overview-row-main-cell overview-row-main-cell-client">
                    <div class="overview-row-meta">
                      <div class="overview-row-label">{{ getClientDisplay(item.key) }}</div>
                      <div v-if="hasAlias(item.key)" class="overview-row-subline">
                        <small class="muted mono">{{ normalizeIP(item.key) }}</small>
                        <span class="overview-row-hint">详情</span>
                      </div>
                    </div>
                    <span v-if="!hasAlias(item.key)" class="overview-row-hint">详情</span>
                  </div>
                </td>
                <td class="text-right">{{ Number(item.count || 0).toLocaleString() }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="panel sub-panel overview-metric-module">
        <h3>最慢查询</h3>
        <div class="table-wrap overview-table-fit slowest-fit module-scroll-list">
          <table>
            <thead>
              <tr>
                <th>域名</th>
                <th class="text-right">耗时</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="slowestQueries.length === 0">
                <td colspan="2" class="empty">暂无数据</td>
              </tr>
              <tr
                v-for="(item, index) in slowestQueries"
                :key="`slow-${index}-${item.trace_id || item.query_time}`"
                class="overview-click-row"
                @click="openSlowDetail(item)"
              >
                <td>
                  <div class="overview-row-main-cell">
                    <span class="overview-row-label">{{ item.query_name }}</span>
                    <span class="overview-row-hint">详情</span>
                  </div>
                </td>
                <td class="text-right" :title="formatDuration(item.duration_ms)">
                  <span class="duration-compact">
                    <span class="duration-value">{{ formatCompactDuration(item.duration_ms) }}</span>
                    <span class="duration-unit">ms</span>
                  </span>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>

      <section class="panel sub-panel overview-metric-module">
        <h3>分流统计</h3>
        <div class="table-wrap overview-table-fit domain-set-fit module-scroll-list">
          <table>
            <thead>
              <tr>
                <th>规则</th>
                <th class="text-right">次数</th>
                <th class="text-right">占比</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="domainSetRows.length === 0">
                <td colspan="3" class="empty">暂无数据</td>
              </tr>
              <tr
                v-for="item in domainSetRows"
                :key="`set-${item.key}`"
                class="overview-click-row"
                @click="openDomainSetDetail(item)"
              >
                <td>
                  <div class="overview-row-main-cell">
                    <span class="domain-set-rule">
                      <span class="domain-set-name">{{ getRuleLabel(item.key) }}</span>
                    </span>
                    <span class="overview-row-hint">详情</span>
                  </div>
                </td>
                <td class="text-right">{{ item.count.toLocaleString() }}</td>
                <td class="text-right">{{ formatPercent(item.percent) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>

    <section v-if="showSystemSummary" class="panel sub-panel overview-system-summary-panel">
      <header class="overview-system-summary-head">
        <div>
          <h3>系统信息</h3>
        </div>
      </header>
      <div class="overview-system-summary-grid">
        <article
          v-for="item in systemSummaryCards"
          :key="item.key"
          class="overview-system-summary-card"
          :class="`tone-${item.tone}`"
        >
          <span class="overview-system-summary-label">{{ item.title }}</span>
          <strong>{{ item.value }}</strong>
          <small v-if="item.desc">{{ item.desc }}</small>
        </article>
      </div>
    </section>

    <Teleport to="body">
      <div v-if="slowDetailOpen && selectedSlowQuery" class="modal-mask modal-mask-top" @click.self="closeSlowDetail">
        <section class="panel data-view-modal overview-slow-detail-modal">
          <header class="panel-header">
            <div>
              <h3>最慢查询详情</h3>
            </div>
            <div class="actions">
              <button class="btn secondary" type="button" @click="closeSlowDetail">关闭</button>
            </div>
          </header>
          <div class="detail-grid">
            <div><strong>时间:</strong> {{ formatTime(selectedSlowQuery.query_time) }}</div>
            <div>
              <strong>客户端:</strong>
              <span :class="{ mono: getSlowDetailActionField('client_ip')?.mono }">{{ getSlowDetailActionField('client_ip')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('client_ip')?.copyValue" @click="copyText(getSlowDetailActionField('client_ip')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('client_ip')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('client_ip')?.filterValue, getSlowDetailActionField('client_ip')?.exact)">筛选</button>
              </span>
            </div>
            <div>
              <strong>域名:</strong>
              <span :class="{ mono: getSlowDetailActionField('query_name')?.mono }">{{ getSlowDetailActionField('query_name')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('query_name')?.copyValue" @click="copyText(getSlowDetailActionField('query_name')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('query_name')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('query_name')?.filterValue, getSlowDetailActionField('query_name')?.exact)">筛选</button>
              </span>
            </div>
            <div><strong>类型:</strong> {{ selectedSlowQuery.query_type || '-' }}</div>
            <div><strong>类别:</strong> {{ selectedSlowQuery.query_class || '-' }}</div>
            <div>
              <strong>Trace ID:</strong>
              <span class="mono">{{ getSlowDetailActionField('trace_id')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('trace_id')?.copyValue" @click="copyText(getSlowDetailActionField('trace_id')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('trace_id')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('trace_id')?.filterValue, getSlowDetailActionField('trace_id')?.exact)">筛选</button>
              </span>
            </div>
            <div><strong>生效标签:</strong> {{ getDetailRuleLabel(selectedSlowQuery.effective_tag || selectedSlowQuery.domain_set) }}</div>
            <div>
              <strong>分流规则:</strong>
              <span>{{ getSlowDetailActionField('domain_set')?.value || '-' }}</span>
              <span class="detail-inline-actions">
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('domain_set')?.copyValue" @click="copyText(getSlowDetailActionField('domain_set')?.copyValue)">复制</button>
                <button class="btn tiny secondary" type="button" :disabled="!getSlowDetailActionField('domain_set')?.filterValue" @click="openLogFilterFromOverview(getSlowDetailActionField('domain_set')?.filterValue, getSlowDetailActionField('domain_set')?.exact)">筛选</button>
              </span>
            </div>
            <div><strong>上游组:</strong> {{ selectedSlowQuery.final_upstream || selectedSlowQuery.upstream_group || '-' }}</div>
            <div><strong>最终上游:</strong> {{ selectedSlowQuery.selected_upstream || '-' }}</div>
            <div><strong>响应码:</strong> {{ selectedSlowQuery.response_code || '-' }}</div>
            <div><strong>响应标志:</strong> {{ formatResponseFlags(selectedSlowQuery.response_flags) }}</div>
            <div><strong>耗时:</strong> {{ formatDuration(selectedSlowQuery.duration_ms) }}</div>
          </div>
          <div class="table-wrap" style="margin-top: 10px;">
            <table>
              <thead>
                <tr>
                  <th>应答类型</th>
                  <th>数据</th>
                  <th class="text-right">TTL</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="!Array.isArray(selectedSlowQuery.answers) || selectedSlowQuery.answers.length === 0">
                  <td colspan="3" class="empty">(empty)</td>
                </tr>
                <tr v-for="(answer, index) in (selectedSlowQuery.answers || [])" :key="`slow-answer-${index}`">
                  <td>{{ answer.type || '-' }}</td>
                  <td class="mono">{{ answer.data || '-' }}</td>
                  <td class="text-right">{{ Number(answer.ttl || 0) }}s</td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>

      <div v-if="rankingDetail.open" class="modal-mask modal-mask-ranking" @click.self="closeRankingDetail">
        <section class="panel data-view-modal overview-ranking-detail-modal">
          <header class="panel-header">
            <div>
              <h3>{{ rankingDetail.title }}</h3>
              <p class="muted">{{ rankingDetail.headline || '-' }}</p>
              <p v-if="rankingDetail.subline" class="muted mono ranking-detail-subline">{{ rankingDetail.subline }}</p>
            </div>
            <div class="actions">
              <button class="btn secondary" type="button" @click="closeRankingDetail">关闭</button>
            </div>
          </header>
          <div class="overview-ranking-summary-grid">
            <article
              v-for="card in rankingDetailSummaryCards"
              :key="card.key"
              class="overview-ranking-summary-card"
            >
              <span class="overview-ranking-summary-label">{{ card.title }}</span>
              <strong>{{ card.value }}</strong>
            </article>
          </div>
          <div class="table-wrap ranking-detail-table-fit">
            <table>
              <thead>
                <tr>
                  <th>时间</th>
                  <th>域名</th>
                  <th>客户端</th>
                  <th>类型</th>
                  <th>分流规则</th>
                  <th>上游</th>
                  <th class="text-right">耗时</th>
                  <th>响应</th>
                  <th class="text-right">详情</th>
                </tr>
              </thead>
              <tbody>
              <tr v-if="rankingDetail.loading">
                <td colspan="9" class="empty">加载中...</td>
              </tr>
              <tr v-else-if="rankingDetail.logs.length === 0">
                <td colspan="9" class="empty">暂无详情数据</td>
              </tr>
              <tr v-for="(log, index) in rankingDetail.logs" :key="`ranking-log-${log.trace_id || log.query_time || index}`">
                <td data-label="时间">{{ formatTime(log.query_time) }}</td>
                <td data-label="域名" class="mono ranking-detail-domain-cell" :title="log.query_name || '-'">{{ log.query_name || '-' }}</td>
                <td data-label="客户端">{{ getClientDisplay(log.client_ip) }}</td>
                <td data-label="类型">{{ log.query_type || '-' }}</td>
                <td data-label="分流规则" :title="getDetailRuleLabel(getRuleDetailValue(log))">{{ getDetailRuleLabel(getRuleDetailValue(log)) }}</td>
                <td data-label="上游" :title="getUpstreamDetailValue(log)">{{ getUpstreamDetailValue(log) }}</td>
                <td data-label="耗时" class="text-right">{{ formatDuration(log.duration_ms) }}</td>
                <td data-label="响应">{{ log.response_code || '-' }}</td>
                <td data-label="详情" class="text-right">
                  <button class="btn tiny secondary" type="button" @click="openSlowDetail(log)">查看</button>
                </td>
              </tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>
    </Teleport>
  </section>
</template>

<style scoped>
.upstream-stats-panel {
  padding-top: 10px;
}

.upstream-stats-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  margin-bottom: 8px;
}

.upstream-stats-title-wrap h3 {
  margin: 0;
}

.upstream-stats-table-wrap {
  overflow-x: auto;
}

.upstream-stats-table {
  width: 100%;
  min-width: 980px;
  table-layout: fixed;
}

.upstream-stats-table th:nth-child(1),
.upstream-stats-table td:nth-child(1) {
  width: 92px;
}

.upstream-stats-table th:nth-child(2),
.upstream-stats-table td:nth-child(2) {
  width: 128px;
}

.upstream-stats-table th:nth-child(3),
.upstream-stats-table td:nth-child(3) {
  width: 240px;
}

.upstream-stats-group-row td {
  font-weight: 700;
  border-top: 1px solid var(--line);
}

.upstream-stats-group-row.tone-domestic td {
  background: rgba(59, 130, 246, 0.08);
  color: #1d4ed8;
}

.upstream-stats-group-row.tone-foreign-fake td {
  background: rgba(244, 114, 182, 0.08);
  color: #be185d;
}

.upstream-stats-group-row.tone-foreign-proxy td {
  background: rgba(249, 115, 22, 0.08);
  color: #c2410c;
}

.upstream-stats-group-row.tone-foreign-ecs td {
  background: rgba(139, 92, 246, 0.08);
  color: #7c3aed;
}

.upstream-stats-group-row.tone-domestic-fake td {
  background: rgba(16, 185, 129, 0.08);
  color: #047857;
}

.upstream-stats-group-row.tone-special td,
.upstream-stats-group-row.tone-other td {
  background: rgba(100, 116, 139, 0.08);
  color: var(--ink-0);
}

.upstream-stats-group-name {
  display: inline-block;
}

.upstream-stats-group-count {
  margin-left: 6px;
  opacity: 0.78;
}

.upstream-stats-type-chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 64px;
  padding: 4px 10px;
  border-radius: 999px;
  background: rgba(37, 99, 235, 0.1);
  color: #2563eb;
  font-size: 0.75rem;
  font-weight: 700;
  letter-spacing: 0.02em;
}

.upstream-stats-name-cell,
.upstream-stats-address-cell {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.overview-system-summary-panel {
  padding-bottom: 12px;
}

.overview-system-summary-head {
  margin-bottom: 10px;
}

.overview-system-summary-head h3 {
  margin: 0;
}

.overview-system-summary-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(156px, 1fr));
  gap: 10px;
}

.overview-system-summary-card {
  border: 1px solid var(--line);
  border-radius: 16px;
  padding: 10px 11px;
  background: rgba(var(--panel-glass-rgb), var(--panel-glass-opacity));
  backdrop-filter: blur(var(--panel-glass-blur));
  -webkit-backdrop-filter: blur(var(--panel-glass-blur));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.7);
  display: flex;
  flex-direction: column;
  gap: 4px;
  min-width: 0;
}

.overview-system-summary-card strong {
  font-size: 0.98rem;
  line-height: 1.25;
  word-break: break-word;
}

.overview-system-summary-card small {
  color: var(--ink-1);
  font-size: 0.76rem;
  line-height: 1.3;
}

.overview-system-summary-label {
  color: var(--ink-1);
  font-size: 0.76rem;
  font-weight: 700;
}

.overview-system-summary-card.tone-ok strong {
  color: var(--ok);
}

.overview-system-summary-card.tone-warn strong {
  color: var(--warn);
}

@media (max-width: 760px) {
  .upstream-stats-head {
    align-items: flex-start;
    flex-direction: column;
  }

  .overview-system-summary-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 540px) {
  .overview-system-summary-grid {
    grid-template-columns: 1fr;
  }
}
</style>

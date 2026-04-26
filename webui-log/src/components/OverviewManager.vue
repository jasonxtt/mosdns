<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON } from '../api/http'
import DnsOverviewCard from './dashboard/DnsOverviewCard.vue'

const HISTORY_KEY = 'mosdnsHistory'
const HISTORY_LENGTH = 60

const loading = ref(false)
const errorMessage = ref('')
const successMessage = ref('')
const lastUpdatedText = ref('--')

const stats = reactive({
  totalQueries: 0,
  averageDurationMs: 0
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
const slowDetailOpen = ref(false)
const selectedSlowQuery = ref(null)
const topDomainDetailOpen = ref(false)
const selectedTopDomain = ref('')
const topDomainDetailLoading = ref(false)
const topDomainDetailLogs = ref([])

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

function clearMessages() {
  errorMessage.value = ''
  successMessage.value = ''
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

function formatTime(value) {
  if (!value || String(value).startsWith('0001-01-01')) {
    return '-'
  }
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) {
    return String(value)
  }
  return date.toLocaleString('zh-CN', { hour12: false })
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

function openSlowDetail(item) {
  selectedSlowQuery.value = item || null
  slowDetailOpen.value = Boolean(item)
}

function closeSlowDetail() {
  slowDetailOpen.value = false
  selectedSlowQuery.value = null
}

async function loadTopDomainDetail(domain) {
  topDomainDetailLoading.value = true
  try {
    const params = new URLSearchParams({
      q: String(domain || ''),
      exact: 'true',
      page: '1',
      limit: '20'
    })
    const data = await getJSON(`/api/v2/audit/logs?${params.toString()}`)
    topDomainDetailLogs.value = Array.isArray(data?.logs) ? data.logs : []
  } catch (error) {
    topDomainDetailLogs.value = []
    errorMessage.value = `加载域名详情失败: ${error.message}`
  } finally {
    topDomainDetailLoading.value = false
  }
}

function openTopDomainDetail(item) {
  const domain = String(item?.key || '').trim()
  if (!domain) {
    return
  }
  selectedTopDomain.value = domain
  topDomainDetailOpen.value = true
  topDomainDetailLogs.value = []
  loadTopDomainDetail(domain)
}

function closeTopDomainDetail() {
  topDomainDetailOpen.value = false
  selectedTopDomain.value = ''
  topDomainDetailLogs.value = []
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
  errorMessage.value = ''
  if (showMessage) {
    successMessage.value = ''
  }
  try {
    const [
      statsRes,
      topDomainsRes,
      topClientsRes,
      slowestRes,
      domainSetRes,
      specialGroupsRes,
      aliasesRes
    ] = await Promise.all([
      getJSON('/api/v2/audit/stats'),
      getJSON('/api/v2/audit/rank/domain?limit=20'),
      getJSON('/api/v2/audit/rank/client?limit=20'),
      getJSON('/api/v2/audit/rank/slowest?limit=20'),
      getJSON('/api/v2/audit/rank/effective?limit=20').catch(() => getJSON('/api/v2/audit/rank/domain_set?limit=20')),
      getJSON('/api/v1/special-groups'),
      getJSON('/plugins/clientname').catch(() => ({}))
    ])

    stats.totalQueries = Number(statsRes?.total_queries || 0)
    stats.averageDurationMs = Number(statsRes?.average_duration_ms || 0)
    addHistoryPoint(stats.totalQueries, stats.averageDurationMs)

    topDomains.value = Array.isArray(topDomainsRes) ? topDomainsRes : []
    topClients.value = Array.isArray(topClientsRes) ? topClientsRes : []
    slowestQueries.value = Array.isArray(slowestRes) ? slowestRes : []
    domainSetRank.value = Array.isArray(domainSetRes) ? domainSetRes : []
    specialGroups.value = Array.isArray(specialGroupsRes) ? specialGroupsRes : []
    aliases.value = normalizeAliasMap(aliasesRes)
    lastUpdatedText.value = new Date().toLocaleString('zh-CN', { hour12: false })

    if (showMessage) {
      successMessage.value = '概览数据已刷新'
    }
  } catch (error) {
    errorMessage.value = `加载概览失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

function handleGlobalRefresh() {
  reloadOverview(false)
}

onMounted(() => {
  loadHistory()
  reloadOverview(false)
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
})
</script>

<template>
  <section class="overview-page">
    <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
    <p v-if="successMessage && !errorMessage" class="msg success">{{ successMessage }}</p>

    <DnsOverviewCard />

    <div class="overview-grid">
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
                <td>{{ item.key }}</td>
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
              <tr v-for="item in topClients" :key="`client-${item.key}`">
                <td>
                  <div>{{ getClientDisplay(item.key) }}</div>
                  <small v-if="hasAlias(item.key)" class="muted mono">{{ normalizeIP(item.key) }}</small>
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
                <td>{{ item.query_name }}</td>
                <td class="text-right">{{ formatDuration(item.duration_ms) }}</td>
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
              <tr v-for="item in domainSetRows" :key="`set-${item.key}`">
                <td>
                  <span class="domain-set-rule">
                    <span class="domain-set-name">{{ getRuleLabel(item.key) }}</span>
                  </span>
                </td>
                <td class="text-right">{{ item.count.toLocaleString() }}</td>
                <td class="text-right">{{ formatPercent(item.percent) }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>

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
          <div><strong>客户端:</strong> {{ getClientDisplay(selectedSlowQuery.client_ip) }}</div>
          <div><strong>域名:</strong> {{ selectedSlowQuery.query_name || '-' }}</div>
          <div><strong>类型:</strong> {{ selectedSlowQuery.query_type || '-' }}</div>
          <div><strong>类别:</strong> {{ selectedSlowQuery.query_class || '-' }}</div>
          <div><strong>Trace ID:</strong> <span class="mono">{{ selectedSlowQuery.trace_id || '-' }}</span></div>
          <div><strong>生效标签:</strong> {{ getDetailRuleLabel(selectedSlowQuery.effective_tag || selectedSlowQuery.domain_set) }}</div>
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

    <div v-if="topDomainDetailOpen" class="modal-mask" @click.self="closeTopDomainDetail">
      <section class="panel data-view-modal overview-slow-detail-modal">
        <header class="panel-header">
          <div>
            <h3>域名详情</h3>
            <p class="muted">{{ selectedTopDomain || '-' }}</p>
          </div>
          <div class="actions">
            <button class="btn secondary" type="button" @click="closeTopDomainDetail">关闭</button>
          </div>
        </header>
        <div class="table-wrap overview-table-fit top-domain-detail-fit">
          <table>
            <thead>
              <tr>
                <th>时间</th>
                <th>客户端</th>
                <th>类型</th>
                <th class="text-right">耗时</th>
                <th>响应</th>
                <th class="text-right">详情</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="topDomainDetailLoading">
                <td colspan="6" class="empty">加载中...</td>
              </tr>
              <tr v-else-if="topDomainDetailLogs.length === 0">
                <td colspan="6" class="empty">暂无详情数据</td>
              </tr>
              <tr v-for="(log, index) in topDomainDetailLogs" :key="`top-domain-log-${log.trace_id || log.query_time || index}`">
                <td>{{ formatTime(log.query_time) }}</td>
                <td>{{ getClientDisplay(log.client_ip) }}</td>
                <td>{{ log.query_type || '-' }}</td>
                <td class="text-right">{{ formatDuration(log.duration_ms) }}</td>
                <td>{{ log.response_code || '-' }}</td>
                <td class="text-right">
                  <button class="btn tiny secondary" type="button" @click="openSlowDetail(log)">查看</button>
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>
  </section>
</template>

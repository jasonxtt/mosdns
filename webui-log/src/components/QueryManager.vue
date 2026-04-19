<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getJSON, postJSON, putJSON } from '../api/http'

const props = defineProps({
  mode: {
    type: String,
    default: 'live'
  }
})

const loading = ref(false)
const errorMessage = ref('')
const successMessage = ref('')

const searchInput = ref('')
const logs = ref([])
const pagination = ref({
  total_items: 0,
  total_pages: 0,
  current_page: 1,
  items_per_page: 50
})
const selectedLog = ref(null)
const detailModalOpen = ref(false)

const clientAliases = ref({})
const specialGroups = ref([])
const aliasModalOpen = ref(false)
const aliasLoading = ref(false)
const aliasSaving = ref(false)
const aliasRows = ref([])
const manualAliasIp = ref('')
const manualAliasName = ref('')
const importAliasInput = ref(null)

const captureDuration = ref(15)
const captureLogs = ref([])
const diagnosticRequests = ref([])
const selectedDiagnosticKey = ref('__raw__')
const showFalseLogs = ref(false)

const isLiveMode = computed(() => props.mode === 'live')

const activeDiagnosticRequest = computed(() => {
  if (selectedDiagnosticKey.value === '__raw__') {
    return null
  }
  return diagnosticRequests.value.find((item) => item.key === selectedDiagnosticKey.value) || null
})

const structuredItems = computed(() => {
  const request = activeDiagnosticRequest.value
  if (!request || !Array.isArray(request.logs) || request.logs.length === 0) {
    return []
  }
  const startAt = new Date(request.logs[0].time).getTime()
  let lastLogger = ''
  const items = []

  request.logs.forEach((log, index) => {
    const logger = String(log.logger_name || '')
    const msg = String(log.msg || '')
    if (logger && logger !== lastLogger && !msg.includes('lazy cache')) {
      items.push({
        kind: 'section',
        key: `section-${logger}-${index}`,
        text: `进入序列: ${logger}`
      })
      lastLogger = logger
    }

    let icon = ''
    let label = ''
    let resultText = ''
    let isFalse = false
    if (String(log.matcher_name || '')) {
      icon = '?'
      label = String(log.matcher_name)
      const matched = parseMatchResult(log.match_result)
      if (matched === true) {
        resultText = 'TRUE'
      } else if (matched === false) {
        resultText = 'FALSE'
        isFalse = true
      } else {
        resultText = '—'
      }
    } else if (String(log.plugin_name || '')) {
      icon = 'P'
      label = String(log.plugin_name)
    } else if (msg) {
      icon = 'L'
      label = msg
    }

    if (!label) {
      return
    }

    const currentAt = new Date(log.time).getTime()
    const diffMs = Number.isFinite(currentAt) && Number.isFinite(startAt) ? Math.max(0, currentAt - startAt) : 0
    items.push({
      kind: 'item',
      key: `item-${index}`,
      icon,
      label,
      resultText,
      isFalse,
      timeDiffMs: diffMs
    })
  })

  return items
})

const flowDurationMs = computed(() => {
  const request = activeDiagnosticRequest.value
  if (!request || !Array.isArray(request.logs) || request.logs.length === 0) {
    return 0
  }
  const startAt = new Date(request.logs[0].time).getTime()
  const endAt = new Date(request.logs[request.logs.length - 1].time).getTime()
  if (!Number.isFinite(startAt) || !Number.isFinite(endAt)) {
    return 0
  }
  return Math.max(0, endAt - startAt)
})

const responseFlagText = computed(() => {
  const flags = selectedLog.value?.response_flags || {}
  const items = []
  if (flags.ra) {
    items.push('RA')
  }
  if (flags.aa) {
    items.push('AA')
  }
  if (flags.tc) {
    items.push('TC')
  }
  return items.length ? items.join(', ') : '-'
})

function normalizeIP(ip) {
  return String(ip || '').replace(/^::ffff:/, '').trim()
}

function normalizeAliasMap(input) {
  const normalized = {}
  if (!input || typeof input !== 'object' || Array.isArray(input)) {
    return normalized
  }
  Object.entries(input).forEach(([ip, alias]) => {
    const key = normalizeIP(ip)
    const value = String(alias || '').trim()
    if (key && value) {
      normalized[key] = value
    }
  })
  return normalized
}

function getDisplayName(ip) {
  const normalizedIp = normalizeIP(ip)
  return clientAliases.value[normalizedIp] || normalizedIp || '-'
}

function hasAlias(ip) {
  const normalizedIp = normalizeIP(ip)
  return Boolean(normalizedIp && clientAliases.value[normalizedIp] && clientAliases.value[normalizedIp] !== normalizedIp)
}

function getClientLabel(ip) {
  const normalizedIp = normalizeIP(ip)
  if (!normalizedIp) {
    return '-'
  }
  const alias = clientAliases.value[normalizedIp]
  return alias ? `${alias} (${normalizedIp})` : normalizedIp
}

function getIpByAlias(alias) {
  const searchTerm = String(alias || '').trim().toLowerCase()
  if (!searchTerm) {
    return ''
  }
  const entries = Object.entries(clientAliases.value)
  for (let index = 0; index < entries.length; index += 1) {
    const [ip, name] = entries[index]
    if (String(name || '').trim().toLowerCase() === searchTerm) {
      return ip
    }
  }
  return ''
}

function getMatchedGroupDisplay(value) {
  const key = String(value || '').trim()
  if (!key) {
    return '-'
  }
  const hit = specialGroups.value.find((item) => String(item?.key || '') === key)
  return hit?.name || key
}

function resetMessages() {
  errorMessage.value = ''
  successMessage.value = ''
}

function parseSearchKeyword(raw) {
  const term = String(raw || '').trim()
  if (!term) {
    return { query: '', exact: false }
  }
  if (term.startsWith('"') && term.endsWith('"') && term.length >= 2) {
    return { query: term.slice(1, -1), exact: true }
  }
  return { query: term, exact: false }
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

function formatRelativeTime(value) {
  if (!value) {
    return '-'
  }
  const now = Date.now()
  const ts = new Date(value).getTime()
  if (Number.isNaN(ts)) {
    return String(value)
  }
  const diffSeconds = Math.max(0, Math.floor((now - ts) / 1000))
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
  return new Date(value).toLocaleDateString('zh-CN', { month: '2-digit', day: '2-digit' })
}

function responseSummary(log) {
  if (!log) {
    return '-'
  }
  if (log.response_code && log.response_code !== 'NOERROR') {
    return log.response_code
  }
  const answers = Array.isArray(log.answers) ? log.answers : []
  if (answers.length === 0) {
    return '(empty)'
  }
  const first = answers.find((item) => item.type === 'A' || item.type === 'AAAA' || item.type === 'CNAME') || answers[0]
  let text = String(first?.data || '')
  if (text.length > 28) {
    text = `${text.slice(0, 25)}...`
  }
  if (answers.length > 1) {
    text += ` (+${answers.length - 1})`
  }
  return text
}

function logRowKey(item, index) {
  return `${item.trace_id || 'trace'}-${item.query_time || 'time'}-${item.client_ip || 'client'}-${index}`
}

function openLogDetail(item) {
  selectedLog.value = item || null
  detailModalOpen.value = Boolean(item)
}

function closeLogDetail() {
  detailModalOpen.value = false
}

function parseMatchResult(value) {
  if (typeof value === 'boolean') {
    return value
  }
  if (typeof value === 'number') {
    if (value === 1) {
      return true
    }
    if (value === 0) {
      return false
    }
  }
  if (typeof value === 'string') {
    const normalized = value.trim().toLowerCase()
    if (normalized === 'true') {
      return true
    }
    if (normalized === 'false') {
      return false
    }
  }
  return null
}

function normalizeCaptureLog(item) {
  if (!item || typeof item !== 'object') {
    return null
  }
  return {
    time: item.time || item.timestamp || '',
    msg: item.msg || item.message || '',
    logger_name: item.logger_name || item.logger || '',
    trace_id: item.trace_id || item.traceId || '',
    domain: item.domain || item.query_name || item.queryName || '',
    matcher_name: item.matcher_name || item.matcher || '',
    plugin_name: item.plugin_name || item.plugin || '',
    match_result: item.match_result ?? item.matched ?? item.result,
    level: item.level || ''
  }
}

function sortByTimeAsc(list) {
  return [...list].sort((a, b) => new Date(a.time).getTime() - new Date(b.time).getTime())
}

function processDiagnosticLogs(allLogs) {
  const normalized = allLogs.map(normalizeCaptureLog).filter(Boolean)
  captureLogs.value = sortByTimeAsc(normalized)

  const requestMap = new Map()
  captureLogs.value.forEach((log) => {
    if (!log.trace_id) {
      return
    }
    if (!requestMap.has(log.trace_id)) {
      requestMap.set(log.trace_id, {
        key: log.trace_id,
        traceId: log.trace_id,
        domain: log.domain || 'N/A',
        startTime: log.time,
        logs: []
      })
    }
    const request = requestMap.get(log.trace_id)
    if (log.domain && request.domain === 'N/A') {
      request.domain = log.domain
    }
    request.logs.push(log)
  })

  const requests = [...requestMap.values()].map((request) => {
    request.logs = sortByTimeAsc(request.logs)
    request.startTime = request.logs[0]?.time || request.startTime
    return request
  })
  requests.sort((a, b) => new Date(b.startTime).getTime() - new Date(a.startTime).getTime())

  diagnosticRequests.value = requests
  selectedDiagnosticKey.value = '__raw__'
  showFalseLogs.value = false
}

async function loadAliases() {
  try {
    const data = await getJSON('/plugins/clientname')
    clientAliases.value = normalizeAliasMap(data)
  } catch {
    clientAliases.value = {}
  }
}

async function loadSpecialGroups() {
  try {
    const data = await getJSON('/api/v1/special-groups')
    specialGroups.value = Array.isArray(data) ? data : []
  } catch {
    specialGroups.value = []
  }
}

function syncAliasRowsFromMap(baseMap = clientAliases.value, knownIPs = []) {
  const unique = new Set([
    ...Object.keys(baseMap || {}),
    ...knownIPs.map((ip) => normalizeIP(ip)).filter(Boolean)
  ])
  const rows = [...unique]
    .sort((a, b) => a.localeCompare(b, 'zh-CN', { numeric: true, sensitivity: 'base' }))
    .map((ip) => ({
      ip,
      alias: String(baseMap?.[ip] || ''),
      originalAlias: String(baseMap?.[ip] || '')
    }))
  aliasRows.value = rows
}

async function openAliasManager() {
  aliasModalOpen.value = true
  aliasLoading.value = true
  manualAliasIp.value = ''
  manualAliasName.value = ''
  resetMessages()
  try {
    const [aliasData, topClients] = await Promise.all([
      getJSON('/plugins/clientname').catch(() => ({})),
      getJSON('/api/v2/audit/rank/client?limit=200').catch(() => [])
    ])
    const normalizedMap = normalizeAliasMap(aliasData)
    clientAliases.value = normalizedMap
    const ips = Array.isArray(topClients) ? topClients.map((item) => item?.key) : []
    syncAliasRowsFromMap(normalizedMap, ips)
  } finally {
    aliasLoading.value = false
  }
}

function closeAliasManager() {
  aliasModalOpen.value = false
}

async function saveAliasesFromRows(showMessage = true) {
  aliasSaving.value = true
  try {
    const nextMap = {}
    aliasRows.value.forEach((row) => {
      const ip = normalizeIP(row.ip)
      const alias = String(row.alias || '').trim()
      if (ip && alias) {
        nextMap[ip] = alias
      }
    })
    await putJSON('/plugins/clientname', nextMap)
    clientAliases.value = normalizeAliasMap(nextMap)
    syncAliasRowsFromMap(clientAliases.value, aliasRows.value.map((row) => row.ip))
    if (showMessage) {
      successMessage.value = '客户端别名已保存'
    }
  } catch (error) {
    errorMessage.value = `保存别名失败: ${error.message}`
  } finally {
    aliasSaving.value = false
  }
}

function addManualAlias() {
  const ip = normalizeIP(manualAliasIp.value)
  const alias = String(manualAliasName.value || '').trim()
  if (!ip || !alias) {
    errorMessage.value = '请填写完整的 IP 与别名'
    return
  }
  const existing = aliasRows.value.find((row) => normalizeIP(row.ip) === ip)
  if (existing) {
    existing.alias = alias
  } else {
    aliasRows.value.push({ ip, alias, originalAlias: '' })
    aliasRows.value.sort((a, b) => normalizeIP(a.ip).localeCompare(normalizeIP(b.ip), 'zh-CN', { numeric: true, sensitivity: 'base' }))
  }
  manualAliasIp.value = ''
  manualAliasName.value = ''
  successMessage.value = `已加入待保存项: ${ip}`
}

function triggerImportAliases() {
  importAliasInput.value?.click()
}

function parseJsonFile(file) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      try {
        const parsed = JSON.parse(String(reader.result || '{}'))
        resolve(parsed)
      } catch (error) {
        reject(error)
      }
    }
    reader.onerror = () => reject(new Error('读取文件失败'))
    reader.readAsText(file)
  })
}

async function onImportAliases(event) {
  const file = event?.target?.files?.[0]
  if (!file) {
    return
  }
  try {
    const parsed = await parseJsonFile(file)
    const imported = normalizeAliasMap(parsed)
    if (Object.keys(imported).length === 0) {
      throw new Error('文件中没有可用的别名数据')
    }
    const merged = {
      ...clientAliases.value,
      ...imported
    }
    await putJSON('/plugins/clientname', merged)
    clientAliases.value = normalizeAliasMap(merged)
    syncAliasRowsFromMap(clientAliases.value, aliasRows.value.map((row) => row.ip))
    successMessage.value = `已导入 ${Object.keys(imported).length} 条别名`
  } catch (error) {
    errorMessage.value = `导入别名失败: ${error.message}`
  } finally {
    event.target.value = ''
  }
}

async function exportAliases() {
  try {
    const latest = await getJSON('/plugins/clientname')
    const payload = normalizeAliasMap(latest)
    const blob = new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' })
    const url = window.URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `mosdns-aliases-${new Date().toISOString().slice(0, 10)}.json`
    document.body.appendChild(anchor)
    anchor.click()
    document.body.removeChild(anchor)
    window.URL.revokeObjectURL(url)
  } catch (error) {
    errorMessage.value = `导出别名失败: ${error.message}`
  }
}

async function loadLogs(page = 1, append = false) {
  loading.value = true
  resetMessages()
  const { query: rawQuery, exact } = parseSearchKeyword(searchInput.value)
  let query = rawQuery
  if (query && !exact) {
    const aliasIP = getIpByAlias(query)
    if (aliasIP) {
      query = aliasIP
    }
  }
  try {
    const params = new URLSearchParams({
      page: String(page),
      limit: '50'
    })
    if (query) {
      params.set('q', query)
      params.set('exact', String(exact))
    }
    const data = await getJSON(`/api/v2/audit/logs?${params.toString()}`)
    const nextLogs = Array.isArray(data?.logs) ? data.logs : []
    pagination.value = data?.pagination || pagination.value
    logs.value = append ? [...logs.value, ...nextLogs] : nextLogs
    if (!append) {
      selectedLog.value = null
      detailModalOpen.value = false
    }
  } catch (error) {
    errorMessage.value = `加载查询日志失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

function refreshLogs() {
  return loadLogs(1, false)
}

function loadMoreLogs() {
  const page = Number(pagination.value.current_page || 1)
  const totalPages = Number(pagination.value.total_pages || 1)
  if (loading.value || page >= totalPages) {
    return
  }
  return loadLogs(page + 1, true)
}

async function startCapture() {
  const seconds = Math.max(1, Math.min(600, Number(captureDuration.value || 15)))
  loading.value = true
  resetMessages()
  try {
    await postJSON('/api/v1/capture/start', { duration_seconds: seconds })
    successMessage.value = `已开始抓取，时长 ${seconds} 秒`
  } catch (error) {
    errorMessage.value = `启动抓取失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

async function fetchCaptureLogs() {
  loading.value = true
  resetMessages()
  try {
    const data = await getJSON('/api/v1/capture/logs')
    const list = Array.isArray(data) ? data : Array.isArray(data?.logs) ? data.logs : Array.isArray(data?.items) ? data.items : []
    processDiagnosticLogs(list)
    if (captureLogs.value.length === 0) {
      errorMessage.value = '没有采集到任何 Debug 日志'
      return
    }
    successMessage.value = `成功获取 ${captureLogs.value.length} 条日志`
  } catch (error) {
    errorMessage.value = `获取抓取日志失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

function renderLogType(log) {
  if (log.matcher_name) {
    return 'Matcher'
  }
  if (log.plugin_name) {
    return 'Plugin'
  }
  return 'Log'
}

function renderLogName(log) {
  return log.matcher_name || log.plugin_name || log.logger_name || log.msg || ''
}

function handleGlobalRefresh() {
  if (isLiveMode.value) {
    refreshLogs()
    return
  }
  fetchCaptureLogs()
}

onMounted(async () => {
  await Promise.all([
    loadAliases(),
    loadSpecialGroups()
  ])
  if (isLiveMode.value) {
    refreshLogs()
  }
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
})
</script>

<template>
  <section class="sub-panel">
    <div v-if="isLiveMode" class="panel">
      <header class="panel-header">
        <div>
          <h3>实时查询</h3>
          <p class="muted">支持分页与关键字过滤。输入带双引号表示精确匹配；支持按客户端别名搜索。</p>
        </div>
        <div class="actions">
          <button class="btn secondary" @click="openAliasManager">客户端别名</button>
        </div>
      </header>

      <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
      <p v-if="successMessage" class="msg success">{{ successMessage }}</p>

      <form class="query-search" @submit.prevent="refreshLogs">
        <input v-model="searchInput" placeholder="全局搜索，使用 &quot;引号&quot; 精确匹配" />
        <button class="btn primary" type="submit" :disabled="loading">搜索</button>
        <button class="btn secondary" type="button" :disabled="loading" @click="searchInput = ''; refreshLogs()">清空</button>
      </form>

      <p class="muted">
        当前第 {{ pagination.current_page || 1 }} / {{ pagination.total_pages || 1 }} 页，
        共 {{ Number(pagination.total_items || 0).toLocaleString() }} 条
      </p>

      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>时间</th>
              <th>域名 / 响应</th>
              <th>类型</th>
              <th>耗时(ms)</th>
              <th>客户端</th>
            </tr>
          </thead>
          <tbody>
            <tr v-if="loading && logs.length === 0">
              <td colspan="5" class="empty">加载中...</td>
            </tr>
            <tr v-else-if="logs.length === 0">
              <td colspan="5" class="empty">暂无日志</td>
            </tr>
            <tr
              v-for="(item, index) in logs"
              :key="logRowKey(item, index)"
              class="log-row"
              :class="{ selected: selectedLog === item }"
              @click="openLogDetail(item)"
            >
              <td>{{ formatRelativeTime(item.query_time) }}</td>
              <td>
                <div><strong>{{ item.query_name }}</strong></div>
                <div class="muted">{{ responseSummary(item) }}</div>
              </td>
              <td>{{ item.query_type || '-' }}</td>
              <td>{{ Number(item.duration_ms || 0).toFixed(2) }}</td>
              <td>
                <div><strong>{{ getDisplayName(item.client_ip) }}</strong></div>
                <div v-if="hasAlias(item.client_ip)" class="muted mono">{{ normalizeIP(item.client_ip) }}</div>
              </td>
            </tr>
          </tbody>
        </table>
      </div>

      <div class="actions" style="margin-top: 10px;">
        <button
          class="btn secondary"
          :disabled="loading || Number(pagination.current_page || 1) >= Number(pagination.total_pages || 1)"
          @click="loadMoreLogs"
        >
          加载更多
        </button>
      </div>
    </div>

    <div v-else class="panel">
      <header class="panel-header">
        <div>
          <h3>诊断抓取</h3>
        </div>
      </header>

      <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
      <p v-if="successMessage" class="msg success">{{ successMessage }}</p>

      <div class="diagnostic-toolbar">
        <label>抓取时长(秒)</label>
        <input v-model.number="captureDuration" type="number" min="1" max="600" />
        <button class="btn secondary" :disabled="loading" @click="startCapture">{{ loading ? '处理中...' : '日志抓取' }}</button>
        <button class="btn primary" :disabled="loading" @click="fetchCaptureLogs">{{ loading ? '处理中...' : '获取日志' }}</button>
      </div>

      <div class="diagnostic-layout">
        <section class="panel sub-panel">
          <h4>请求列表</h4>
          <div class="diagnostic-request-list">
            <button
              class="diagnostic-request-item"
              :class="{ active: selectedDiagnosticKey === '__raw__' }"
              @click="selectedDiagnosticKey = '__raw__'"
            >
              <span class="domain">原始日志</span>
              <span class="details">全部 {{ captureLogs.length }} 条</span>
            </button>

            <button
              v-for="item in diagnosticRequests"
              :key="item.key"
              class="diagnostic-request-item"
              :class="{ active: selectedDiagnosticKey === item.key }"
              @click="selectedDiagnosticKey = item.key"
            >
              <span class="domain">{{ item.domain || 'N/A' }}</span>
              <span class="details">{{ formatTime(item.startTime) }} · {{ item.traceId }}</span>
            </button>
          </div>
        </section>

        <section class="panel sub-panel">
          <h4>分析结果</h4>

          <div v-if="selectedDiagnosticKey === '__raw__'" class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>时间</th>
                  <th>Trace ID</th>
                  <th>类型</th>
                  <th>名称/信息</th>
                  <th>结果</th>
                  <th>域名</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="captureLogs.length === 0">
                  <td colspan="6" class="empty">暂无抓取数据</td>
                </tr>
                <tr v-for="(log, index) in captureLogs" :key="`raw-${index}`">
                  <td>{{ formatTime(log.time) }}</td>
                  <td class="mono">{{ log.trace_id || '-' }}</td>
                  <td>{{ renderLogType(log) }}</td>
                  <td>{{ renderLogName(log) }}</td>
                  <td>
                    <span v-if="parseMatchResult(log.match_result) === true" class="result-badge ok">TRUE</span>
                    <span v-else-if="parseMatchResult(log.match_result) === false" class="result-badge fail">FALSE</span>
                    <span v-else>-</span>
                  </td>
                  <td>{{ log.domain || '-' }}</td>
                </tr>
              </tbody>
            </table>
          </div>

          <div v-else-if="activeDiagnosticRequest" class="structured-flow">
            <header class="timeline-header">
              <h5>
                DNS 查询流程: <strong>{{ activeDiagnosticRequest.domain || 'N/A' }}</strong>
                (总耗时: {{ flowDurationMs }} ms)
              </h5>
              <button class="btn tiny secondary" @click="showFalseLogs = !showFalseLogs">
                {{ showFalseLogs ? '隐藏 FALSE' : '显示 FALSE' }}
              </button>
            </header>

            <div class="structured-log-list">
              <template v-for="item in structuredItems" :key="item.key">
                <div v-if="item.kind === 'section'" class="log-section-header">{{ item.text }}</div>
                <div
                  v-else-if="showFalseLogs || !item.isFalse"
                  class="log-item"
                  :class="{ 'result-fail': item.isFalse }"
                >
                  <div class="log-item-main">
                    <span class="icon">{{ item.icon }}</span>
                    <span class="content">{{ item.label }}</span>
                  </div>
                  <div class="log-item-meta">
                    <span v-if="item.resultText && item.resultText !== '—'" class="result-badge" :class="{ ok: item.resultText === 'TRUE', fail: item.resultText === 'FALSE' }">
                      {{ item.resultText }}
                    </span>
                    <span class="time">+{{ item.timeDiffMs }}ms</span>
                  </div>
                </div>
              </template>
            </div>
          </div>

          <p v-else class="muted">点击左侧请求查看分流流程。</p>
        </section>
      </div>
    </div>

    <div v-if="detailModalOpen && selectedLog" class="modal-mask" @click.self="closeLogDetail">
      <section class="panel data-view-modal">
        <header class="panel-header">
          <div>
            <h3>查询详情</h3>
          </div>
          <div class="actions">
            <button class="btn secondary" type="button" @click="closeLogDetail">关闭</button>
          </div>
        </header>
        <div class="detail-grid">
          <div><strong>时间:</strong> {{ formatTime(selectedLog.query_time) }}</div>
          <div><strong>客户端:</strong> {{ getClientLabel(selectedLog.client_ip) }}</div>
          <div><strong>域名:</strong> {{ selectedLog.query_name || '-' }}</div>
          <div><strong>类型:</strong> {{ selectedLog.query_type || '-' }}</div>
          <div><strong>类别:</strong> {{ selectedLog.query_class || '-' }}</div>
          <div><strong>Trace ID:</strong> <span class="mono">{{ selectedLog.trace_id || '-' }}</span></div>
          <div><strong>分流规则:</strong> {{ selectedLog.domain_set || '-' }}</div>
          <div><strong>匹配来源:</strong> {{ selectedLog.matched_rule_source || '-' }}</div>
          <div><strong>专属分流组:</strong> {{ getMatchedGroupDisplay(selectedLog.matched_group) }}</div>
          <div><strong>最终序列:</strong> {{ selectedLog.final_sequence || '-' }}</div>
          <div><strong>最终上游组:</strong> {{ selectedLog.final_upstream || '-' }}</div>
          <div><strong>上游目标:</strong> {{ selectedLog.upstream_targets || '-' }}</div>
          <div><strong>最终上游:</strong> {{ selectedLog.selected_upstream || '-' }}</div>
          <div>
            <strong>响应码:</strong>
            {{ selectedLog.response_code || '-' }}<span v-if="selectedLog.is_blocked"> (已拦截)</span>
          </div>
          <div><strong>响应标志:</strong> {{ responseFlagText }}</div>
          <div><strong>耗时:</strong> {{ Number(selectedLog.duration_ms || 0).toFixed(2) }} ms</div>
        </div>

        <div class="table-wrap" style="margin-top: 10px;">
          <table>
            <thead>
              <tr>
                <th>应答类型</th>
                <th>数据</th>
                <th>TTL</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="!Array.isArray(selectedLog.answers) || selectedLog.answers.length === 0">
                <td colspan="3" class="empty">(empty)</td>
              </tr>
              <tr v-for="(answer, index) in (selectedLog.answers || [])" :key="`answer-${index}`">
                <td>{{ answer.type || '-' }}</td>
                <td class="mono">{{ answer.data || '-' }}</td>
                <td>{{ Number(answer.ttl || 0) }}s</td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>

    <div v-if="aliasModalOpen" class="modal-mask" @click.self="closeAliasManager">
      <section class="panel data-view-modal alias-modal">
        <header class="panel-header">
          <div>
            <h3>客户端别名管理</h3>
            <p class="muted">保存后立即生效，搜索框可直接输入别名。</p>
          </div>
          <div class="actions">
            <button class="btn secondary" @click="closeAliasManager">关闭</button>
          </div>
        </header>

        <div class="form-grid alias-manual-grid">
          <label>手动新增</label>
          <div class="alias-manual-row">
            <input v-model="manualAliasIp" placeholder="客户端 IP，例如 192.168.1.10" />
            <input v-model="manualAliasName" placeholder="别名，例如 iPhone-15" />
            <button class="btn secondary" type="button" @click="addManualAlias">加入列表</button>
          </div>
        </div>

        <div class="actions" style="margin: 10px 0;">
          <button class="btn primary" :disabled="aliasLoading || aliasSaving" @click="saveAliasesFromRows(true)">
            {{ aliasSaving ? '保存中...' : '保存全部' }}
          </button>
          <button class="btn secondary" :disabled="aliasLoading || aliasSaving" @click="exportAliases">导出 JSON</button>
          <button class="btn secondary" :disabled="aliasLoading || aliasSaving" @click="triggerImportAliases">导入 JSON</button>
          <input ref="importAliasInput" type="file" accept=".json,application/json" style="display:none" @change="onImportAliases" />
        </div>

        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th style="width: 40%;">客户端 IP</th>
                <th>别名</th>
              </tr>
            </thead>
            <tbody>
              <tr v-if="aliasLoading">
                <td colspan="2" class="empty">加载中...</td>
              </tr>
              <tr v-else-if="aliasRows.length === 0">
                <td colspan="2" class="empty">暂无客户端记录</td>
              </tr>
              <tr v-for="row in aliasRows" :key="`alias-${row.ip}`">
                <td class="mono">{{ row.ip }}</td>
                <td>
                  <input v-model="row.alias" placeholder="为空表示删除该别名" />
                </td>
              </tr>
            </tbody>
          </table>
        </div>
      </section>
    </div>
  </section>
</template>

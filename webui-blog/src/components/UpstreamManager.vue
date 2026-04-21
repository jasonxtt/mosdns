<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { deleteRequest, getJSON, getText, postJSON } from '../api/http'
import { openConfirm } from '../utils/confirm'

const HIDE_DISABLED_KEY = 'mosdnsHideDisabledUpstreams'

const loading = ref(false)
const saving = ref(false)
const filterGroup = ref('all')
const showEditor = ref(false)
const hideDisabled = ref(false)
const errorMessage = ref('')
const successMessage = ref('')

const sortState = reactive({
  key: '',
  order: 'desc'
})

const upstreamTags = ref([])
const upstreamConfig = ref({})
const specialGroups = ref([])
const specialModalOpen = ref(false)
const specialSaving = ref(false)
const specialEditor = reactive({
  slot: 0,
  name: ''
})
const editingCtx = ref({ group: '', index: -1 })
const metrics = ref({
  latSum: {},
  latCount: {},
  queryTotal: {},
  errorTotal: {},
  winnerTotal: {}
})

const form = reactive({
  group: '',
  tag: '',
  protocol: 'aliapi',
  addr: '',
  dial_addr: '',
  socks5: '',
  bootstrap: '',
  bootstrap_version: 0,
  enable_pipeline: false,
  enable_http3: false,
  insecure_skip_verify: false,
  idle_timeout: 0,
  upstream_query_timeout: 0,
  bind_to_device: '',
  so_mark: 0,
  account_id: '',
  access_key_id: '',
  access_key_secret: '',
  server_addr: '223.5.5.5',
  ecs_client_ip: '',
  ecs_client_mask: 0
})

const protocolOptions = [
  { value: 'udp', label: 'UDP' },
  { value: 'tcp', label: 'TCP' },
  { value: 'tls', label: 'DoT (TLS)' },
  { value: 'dot', label: 'DoT (dot)' },
  { value: 'https', label: 'DoH (HTTPS)' },
  { value: 'doh', label: 'DoH (doh)' },
  { value: 'quic', label: 'DoQ (QUIC)' },
  { value: 'doq', label: 'DoQ (doq)' },
  { value: 'aliapi', label: '阿里 API (AliAPI)' }
]

function isSpecialUpstreamTag(tag) {
  return /^special_upstream_\d+$/.test(String(tag || ''))
}

const protocolValue = computed(() => String(form.protocol || '').trim().toLowerCase())
const isAliapi = computed(() => protocolValue.value === 'aliapi')
const showPipeline = computed(() => ['tcp', 'dot', 'tls'].includes(protocolValue.value))
const showHttp3 = computed(() => ['https', 'doh', 'quic', 'doq'].includes(protocolValue.value))
const showSocks5 = computed(() => ['dot', 'tls', 'tcp', 'doh', 'https', 'quic', 'doq'].includes(protocolValue.value))
const showTlsVerify = computed(() => ['dot', 'tls', 'tcp', 'doh', 'https', 'quic', 'doq'].includes(protocolValue.value))

const groupOptions = computed(() => {
  const options = new Set()
  ;(upstreamTags.value || []).forEach((tag) => {
    if (typeof tag === 'string' && tag.trim() && !isSpecialUpstreamTag(tag)) {
      options.add(tag.trim())
    }
  })
  Object.keys(upstreamConfig.value || {}).forEach((group) => {
    if (group && group.trim() && !isSpecialUpstreamTag(group)) {
      options.add(group.trim())
    }
  })
  ;(specialGroups.value || []).forEach((group) => {
    if (group?.upstream_plugin_tag) {
      options.add(String(group.upstream_plugin_tag))
    }
  })
  return [...options].sort((a, b) => a.localeCompare(b, 'zh-CN', { numeric: true, sensitivity: 'base' }))
})

const hideDisabledLabel = computed(() => (hideDisabled.value ? '显示全部上游' : '隐藏未启用上游'))

function groupDisplayName(group) {
  const special = (specialGroups.value || []).find((item) => item?.upstream_plugin_tag === group)
  if (special?.name) {
    return `${special.name} (${group})`
  }
  return group
}

function parseMetricMap(rawText, metricName) {
  const map = {}
  const regex = new RegExp(`${metricName}\\{[^}]*metrics_tag="([^"]+)"[^}]*tag="([^"]+)"[^}]*\\} ([0-9.eE+-]+)`, 'g')
  let match = regex.exec(rawText)
  while (match !== null) {
    const metricsTag = match[1]
    const tag = match[2]
    const value = Number.parseFloat(match[3] || '0') || 0
    map[`${metricsTag}|${tag}`] = value
    match = regex.exec(rawText)
  }
  return map
}

function parseAllMetrics(rawText) {
  const text = String(rawText || '')
  metrics.value = {
    latSum: parseMetricMap(text, 'mosdns_aliapi_response_latency_millisecond_sum'),
    latCount: parseMetricMap(text, 'mosdns_aliapi_response_latency_millisecond_count'),
    queryTotal: parseMetricMap(text, 'mosdns_aliapi_query_total'),
    errorTotal: parseMetricMap(text, 'mosdns_aliapi_error_total'),
    winnerTotal: parseMetricMap(text, 'mosdns_aliapi_upstream_winner_total')
  }
}

function getRowStats(group, item) {
  const enabled = Boolean(item?.enabled)
  if (!enabled) {
    return {
      avgLatency: '-',
      query: '-',
      winner: '-',
      winRate: '-',
      error: '-',
      errorRate: '-',
      avgLatencyNumber: 0,
      queryNumber: 0,
      winnerNumber: 0,
      winRateNumber: 0,
      errorNumber: 0,
      errorRateNumber: 0
    }
  }

  const key = `${group}|${item?.tag || ''}`
  const q = Number(metrics.value.queryTotal[key] || 0)
  const e = Number(metrics.value.errorTotal[key] || 0)
  const w = Number(metrics.value.winnerTotal[key] || 0)
  const lSum = Number(metrics.value.latSum[key] || 0)
  const lCount = Number(metrics.value.latCount[key] || 0)
  const avgLatencyNumber = lCount > 0 ? (lSum / lCount) : 0
  const errorRateNumber = q > 0 ? ((e / q) * 100) : 0
  const winRateNumber = q > 0 ? ((w / q) * 100) : 0

  return {
    avgLatency: `${avgLatencyNumber.toFixed(2)} ms`,
    query: q.toLocaleString(),
    winner: w.toLocaleString(),
    winRate: `${winRateNumber.toFixed(2)}%`,
    error: e.toLocaleString(),
    errorRate: `${errorRateNumber.toFixed(2)}%`,
    avgLatencyNumber,
    queryNumber: q,
    winnerNumber: w,
    winRateNumber,
    errorNumber: e,
    errorRateNumber
  }
}

function getSortValue(row) {
  switch (sortState.key) {
    case 'enabled':
      return row.data?.enabled ? 1 : 0
    case 'group':
      return groupDisplayName(row.group)
    case 'tag':
      return String(row.data?.tag || '')
    case 'protocol':
      return String(row.data?.protocol || '')
    case 'avg_latency':
      return row.stats.avgLatencyNumber
    case 'query':
      return row.stats.queryNumber
    case 'winner':
      return row.stats.winnerNumber
    case 'win_rate':
      return row.stats.winRateNumber
    case 'error':
      return row.stats.errorNumber
    case 'error_rate':
      return row.stats.errorRateNumber
    default:
      return ''
  }
}

const rows = computed(() => {
  const all = []
  let originalOrder = 0
  Object.entries(upstreamConfig.value || {}).forEach(([group, upstreams]) => {
    if (filterGroup.value !== 'all' && group !== filterGroup.value) {
      return
    }
    if (!Array.isArray(upstreams)) {
      return
    }
    upstreams.forEach((item, index) => {
      if (hideDisabled.value && !Boolean(item?.enabled)) {
        return
      }
      all.push({
        group,
        index,
        originalOrder,
        data: item || {},
        stats: getRowStats(group, item || {})
      })
      originalOrder += 1
    })
  })

  if (!sortState.key) {
    return [...all].reverse()
  }

  const collator = new Intl.Collator('zh-CN', { numeric: true, sensitivity: 'base' })
  return [...all].sort((a, b) => {
    const valueA = getSortValue(a)
    const valueB = getSortValue(b)
    let result = 0
    if (typeof valueA === 'string' || typeof valueB === 'string') {
      result = collator.compare(String(valueA || ''), String(valueB || ''))
    } else if (valueA < valueB) {
      result = -1
    } else if (valueA > valueB) {
      result = 1
    }
    if (result === 0) {
      result = a.originalOrder - b.originalOrder
    }
    return sortState.order === 'asc' ? result : -result
  })
})

function rowAddress(item) {
  if (!item) {
    return '-'
  }
  if (String(item.protocol || '').toLowerCase() === 'aliapi') {
    return item.server_addr || '-'
  }
  return item.addr || '-'
}

function resetMessage() {
  errorMessage.value = ''
  successMessage.value = ''
}

function resetForm() {
  form.group = ''
  form.tag = ''
  form.protocol = 'aliapi'
  form.addr = ''
  form.dial_addr = ''
  form.socks5 = ''
  form.bootstrap = ''
  form.bootstrap_version = 0
  form.enable_pipeline = false
  form.enable_http3 = false
  form.insecure_skip_verify = false
  form.idle_timeout = 0
  form.upstream_query_timeout = 0
  form.bind_to_device = ''
  form.so_mark = 0
  form.account_id = ''
  form.access_key_id = ''
  form.access_key_secret = ''
  form.server_addr = '223.5.5.5'
  form.ecs_client_ip = ''
  form.ecs_client_mask = 0
}

function toInt(value, fallback = 0) {
  const n = Number(value)
  return Number.isFinite(n) ? Math.trunc(n) : fallback
}

function onSort(key) {
  if (sortState.key === key) {
    sortState.order = sortState.order === 'asc' ? 'desc' : 'asc'
    return
  }
  sortState.key = key
  sortState.order = 'asc'
}

function sortIndicator(key) {
  if (sortState.key !== key) {
    return ' '
  }
  return sortState.order === 'asc' ? '▲' : '▼'
}

function toggleHideDisabled() {
  hideDisabled.value = !hideDisabled.value
  localStorage.setItem(HIDE_DISABLED_KEY, hideDisabled.value ? '1' : '0')
}

async function loadData() {
  loading.value = true
  resetMessage()
  try {
    const [tagsRes, configRes, groupsRes, metricsRes] = await Promise.allSettled([
      getJSON('/api/v1/upstream/tags'),
      getJSON('/api/v1/upstream/config'),
      getJSON('/api/v1/special-groups'),
      getText('/metrics')
    ])
    upstreamTags.value = tagsRes.status === 'fulfilled' && Array.isArray(tagsRes.value) ? tagsRes.value : []
    upstreamConfig.value = configRes.status === 'fulfilled' && configRes.value ? configRes.value : {}
    specialGroups.value = groupsRes.status === 'fulfilled' && Array.isArray(groupsRes.value) ? groupsRes.value : []
    parseAllMetrics(metricsRes.status === 'fulfilled' ? metricsRes.value : '')

    if (tagsRes.status === 'rejected' || configRes.status === 'rejected' || groupsRes.status === 'rejected' || metricsRes.status === 'rejected') {
      errorMessage.value = '部分数据加载失败，已使用可用数据渲染页面。'
    }
  } catch (error) {
    errorMessage.value = `加载上游配置失败: ${error.message}`
  } finally {
    loading.value = false
  }
}

function beginAdd() {
  resetMessage()
  editingCtx.value = { group: '', index: -1 }
  resetForm()
  form.group = groupOptions.value[0] || ''
  showEditor.value = true
}

function beginEdit(row) {
  resetMessage()
  const item = row.data || {}
  editingCtx.value = { group: row.group, index: row.index }
  resetForm()

  form.group = row.group
  form.tag = String(item.tag || '')
  form.protocol = String(item.protocol || 'udp')
  form.addr = String(item.addr || '')
  form.dial_addr = String(item.dial_addr || '')
  form.socks5 = String(item.socks5 || '')
  form.bootstrap = String(item.bootstrap || '')
  form.bootstrap_version = toInt(item.bootstrap_version, 0)
  form.enable_pipeline = Boolean(item.enable_pipeline)
  form.enable_http3 = Boolean(item.enable_http3)
  form.insecure_skip_verify = Boolean(item.insecure_skip_verify)
  form.idle_timeout = toInt(item.idle_timeout, 0)
  form.upstream_query_timeout = toInt(item.upstream_query_timeout, 0)
  form.bind_to_device = String(item.bind_to_device || '')
  form.so_mark = toInt(item.so_mark, 0)
  form.account_id = String(item.account_id || '')
  form.access_key_id = String(item.access_key_id || '')
  form.access_key_secret = String(item.access_key_secret || '')
  form.server_addr = String(item.server_addr || '223.5.5.5')
  form.ecs_client_ip = String(item.ecs_client_ip || '')
  form.ecs_client_mask = toInt(item.ecs_client_mask, 0)
  showEditor.value = true
}

function closeEditor() {
  showEditor.value = false
}

function openCreateSpecialGroup() {
  resetMessage()
  specialEditor.slot = 0
  specialEditor.name = ''
  specialModalOpen.value = true
}

function openEditSpecialGroup(group) {
  resetMessage()
  specialEditor.slot = Number(group?.slot) || 0
  specialEditor.name = String(group?.name || '')
  specialModalOpen.value = true
}

function closeSpecialGroupModal() {
  specialModalOpen.value = false
}

async function saveSpecialGroup() {
  const name = String(specialEditor.name || '').trim()
  if (!name) {
    errorMessage.value = '专属分流组名称不能为空'
    return
  }

  specialSaving.value = true
  resetMessage()
  try {
    await postJSON('/api/v1/special-groups', {
      slot: Number(specialEditor.slot) || 0,
      name
    })
    successMessage.value = '专属分流组已保存'
    closeSpecialGroupModal()
    await loadData()
  } catch (error) {
    errorMessage.value = `保存专属分流组失败: ${error.message}`
  } finally {
    specialSaving.value = false
  }
}

async function deleteSpecialGroup(group) {
  const ok = await openConfirm(`确定删除专属分流组“${group?.name || ''}”吗？删除后会清空该组绑定的上游配置与在线分流配置。`, { tone: 'danger' })
  if (!ok) {
    return
  }

  resetMessage()
  try {
    await deleteRequest(`/api/v1/special-groups/${group.slot}`)
    successMessage.value = '专属分流组已删除'
    await loadData()
  } catch (error) {
    errorMessage.value = `删除专属分流组失败: ${error.message}`
  }
}

function buildUpstreamObject(enabledWhenSave = true) {
  const protocol = protocolValue.value
  return {
    tag: String(form.tag || '').trim(),
    protocol,
    addr: protocol !== 'aliapi' ? String(form.addr || '').trim() : '',
    dial_addr: protocol !== 'aliapi' ? String(form.dial_addr || '').trim() : '',
    idle_timeout: protocol !== 'aliapi' ? toInt(form.idle_timeout, 0) : 0,
    upstream_query_timeout: protocol !== 'aliapi' ? toInt(form.upstream_query_timeout, 0) : 0,
    bind_to_device: protocol !== 'aliapi' ? String(form.bind_to_device || '').trim() : '',
    so_mark: protocol !== 'aliapi' ? toInt(form.so_mark, 0) : 0,
    enable_pipeline: protocol !== 'aliapi' ? Boolean(form.enable_pipeline) : false,
    enable_http3: protocol !== 'aliapi' ? Boolean(form.enable_http3) : false,
    insecure_skip_verify: protocol !== 'aliapi' ? Boolean(form.insecure_skip_verify) : false,
    socks5: protocol !== 'aliapi' ? String(form.socks5 || '').trim() : '',
    bootstrap: protocol !== 'aliapi' ? String(form.bootstrap || '').trim() : '',
    bootstrap_version: protocol !== 'aliapi' ? toInt(form.bootstrap_version, 0) : 0,
    account_id: protocol === 'aliapi' ? String(form.account_id || '').trim() : '',
    access_key_id: protocol === 'aliapi' ? String(form.access_key_id || '').trim() : '',
    access_key_secret: protocol === 'aliapi' ? String(form.access_key_secret || '').trim() : '',
    server_addr: protocol === 'aliapi' ? String(form.server_addr || '').trim() : '',
    ecs_client_ip: protocol === 'aliapi' ? String(form.ecs_client_ip || '').trim() : '',
    ecs_client_mask: protocol === 'aliapi' ? toInt(form.ecs_client_mask, 0) : 0,
    enabled: Boolean(enabledWhenSave)
  }
}

async function saveUpstream() {
  const group = String(form.group || '').trim()
  const tag = String(form.tag || '').trim()
  const protocol = protocolValue.value

  if (!group) {
    errorMessage.value = '请选择所属组'
    return
  }
  if (!tag) {
    errorMessage.value = '上游标识不能为空'
    return
  }
  if (!protocol) {
    errorMessage.value = '协议不能为空'
    return
  }

  saving.value = true
  resetMessage()
  try {
    const list = Array.isArray(upstreamConfig.value[group]) ? [...upstreamConfig.value[group]] : []
    if (editingCtx.value.index >= 0 && editingCtx.value.group === group) {
      const current = list[editingCtx.value.index] || {}
      const enabled = Boolean(current.enabled)
      list[editingCtx.value.index] = buildUpstreamObject(enabled)
    } else {
      list.push(buildUpstreamObject(true))
    }

    await postJSON('/api/v1/upstream/config', {
      plugin_tag: group,
      upstreams: list
    })
    successMessage.value = '上游配置已保存'
    showEditor.value = false
    await loadData()
  } catch (error) {
    errorMessage.value = `保存失败: ${error.message}`
  } finally {
    saving.value = false
  }
}

async function removeRow(row) {
  const ok = await openConfirm(`确定删除上游 "${row.data?.tag || 'unnamed'}" 吗？`, { tone: 'danger' })
  if (!ok) {
    return
  }
  resetMessage()
  try {
    const list = Array.isArray(upstreamConfig.value[row.group]) ? [...upstreamConfig.value[row.group]] : []
    list.splice(row.index, 1)
    await postJSON('/api/v1/upstream/config', {
      plugin_tag: row.group,
      upstreams: list
    })
    successMessage.value = '上游已删除'
    await loadData()
  } catch (error) {
    errorMessage.value = `删除失败: ${error.message}`
  }
}

async function toggleEnable(row) {
  resetMessage()
  try {
    const list = Array.isArray(upstreamConfig.value[row.group]) ? [...upstreamConfig.value[row.group]] : []
    if (!list[row.index]) {
      return
    }
    list[row.index] = {
      ...list[row.index],
      enabled: !Boolean(list[row.index].enabled)
    }
    await postJSON('/api/v1/upstream/config', {
      plugin_tag: row.group,
      upstreams: list
    })
    await loadData()
  } catch (error) {
    errorMessage.value = `切换失败: ${error.message}`
  }
}

function handleGlobalRefresh() {
  loadData()
}

onMounted(() => {
  hideDisabled.value = localStorage.getItem(HIDE_DISABLED_KEY) === '1'
  loadData()
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
})
</script>

<template>
  <section class="panel upstream-page">
    <div class="upstream-toolbar upstream-toolbar-card">
      <div class="upstream-toolbar-left">
        <button class="btn primary" type="button" @click="beginAdd">添加上游DNS</button>
        <button class="btn secondary" type="button" @click="openCreateSpecialGroup">新增专属分流组</button>
      </div>
      <div class="upstream-toolbar-right">
        <div class="upstream-special-groups-strip">
          <span class="special-groups-title-inline">专属分流组</span>
          <div class="special-groups-list">
            <span v-if="specialGroups.length === 0" class="special-groups-empty">
              还没有专属分流组。点击左侧“新增专属分流组”后即可在上游设置和在线分流里使用。
            </span>
            <div v-for="group in specialGroups" v-else :key="group.slot" class="special-group-row">
              <span class="special-group-name">{{ group.name }}</span>
              <div class="special-group-actions">
                <button class="btn tiny secondary" type="button" @click="openEditSpecialGroup(group)">改名</button>
                <button class="btn tiny danger" type="button" @click="deleteSpecialGroup(group)">删除</button>
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>

    <div class="toolbar upstream-filter-card">
      <label for="group-filter">过滤分组</label>
      <select id="group-filter" v-model="filterGroup">
        <option value="all">全部</option>
        <option v-for="group in groupOptions" :key="group" :value="group">{{ groupDisplayName(group) }}</option>
      </select>
      <button class="btn secondary" type="button" @click="toggleHideDisabled">{{ hideDisabledLabel }}</button>
    </div>

    <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
    <p v-if="successMessage" class="msg success">{{ successMessage }}</p>

    <div class="table-wrap upstream-table-card">
      <table>
        <thead>
          <tr>
            <th class="sortable" @click="onSort('enabled')">启用 <span class="sort-indicator">{{ sortIndicator('enabled') }}</span></th>
            <th class="sortable" @click="onSort('group')">所属组 <span class="sort-indicator">{{ sortIndicator('group') }}</span></th>
            <th class="sortable" @click="onSort('tag')">标识 <span class="sort-indicator">{{ sortIndicator('tag') }}</span></th>
            <th class="sortable" @click="onSort('protocol')">协议 <span class="sort-indicator">{{ sortIndicator('protocol') }}</span></th>
            <th class="sortable text-center" @click="onSort('avg_latency')">平均响应 <span class="sort-indicator">{{ sortIndicator('avg_latency') }}</span></th>
            <th class="sortable text-center" @click="onSort('query')">请求数 <span class="sort-indicator">{{ sortIndicator('query') }}</span></th>
            <th class="sortable text-center" @click="onSort('winner')">采纳数 <span class="sort-indicator">{{ sortIndicator('winner') }}</span></th>
            <th class="sortable text-center" @click="onSort('win_rate')">采纳率 <span class="sort-indicator">{{ sortIndicator('win_rate') }}</span></th>
            <th class="sortable text-center" @click="onSort('error')">错误数 <span class="sort-indicator">{{ sortIndicator('error') }}</span></th>
            <th class="sortable text-center" @click="onSort('error_rate')">出错率 <span class="sort-indicator">{{ sortIndicator('error_rate') }}</span></th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="loading">
            <td colspan="11" class="empty">加载中...</td>
          </tr>
          <tr v-else-if="rows.length === 0">
            <td colspan="11" class="empty">{{ hideDisabled ? '当前没有已启用的上游配置' : '暂无上游配置' }}</td>
          </tr>
          <tr v-for="row in rows" :key="`${row.group}-${row.index}-${row.data?.tag || 'x'}`" :class="{ disabled: !row.data?.enabled }">
            <td>
              <button class="btn tiny status-toggle-btn" :class="row.data?.enabled ? 'status-on' : 'status-off'" @click="toggleEnable(row)">
                {{ row.data?.enabled ? 'ON' : 'OFF' }}
              </button>
            </td>
            <td>{{ groupDisplayName(row.group) }}</td>
            <td>{{ row.data?.tag || '-' }}</td>
            <td>{{ row.data?.protocol || '-' }}</td>
            <td class="text-center">{{ row.stats.avgLatency }}</td>
            <td class="text-center">{{ row.stats.query }}</td>
            <td class="text-center">{{ row.stats.winner }}</td>
            <td class="text-center">{{ row.stats.winRate }}</td>
            <td class="text-center">{{ row.stats.error }}</td>
            <td class="text-center">{{ row.stats.errorRate }}</td>
            <td class="row-actions">
              <button class="btn tiny secondary" @click="beginEdit(row)">编辑</button>
              <button class="btn tiny danger" @click="removeRow(row)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-if="showEditor" class="modal-mask" @click.self="closeEditor">
      <section class="panel form-modal-card upstream-editor-modal-card">
        <header class="panel-header">
          <h3>{{ editingCtx.index >= 0 ? '编辑上游' : '新增上游' }}</h3>
          <button class="btn tiny secondary" type="button" @click="closeEditor">✕</button>
        </header>
        <div class="form-grid">
          <label>所属组</label>
          <input v-if="editingCtx.index >= 0" v-model="form.group" disabled />
          <select v-else v-model="form.group">
            <option value="" disabled>请选择所属组</option>
            <option v-for="group in groupOptions" :key="group" :value="group">
              {{ groupDisplayName(group) }}
            </option>
          </select>

          <label>上游标识</label>
          <input v-model="form.tag" placeholder="例如 cmcc_dns_1" />

          <label>协议</label>
          <select v-model="form.protocol">
            <option v-for="item in protocolOptions" :key="item.value" :value="item.value">{{ item.label }}</option>
          </select>

          <template v-if="!isAliapi">
            <label>服务器地址 (Addr)</label>
            <input v-model="form.addr" placeholder="例如 https://dns.google/dns-query 或 223.5.5.5" />

            <label>拨号地址 (Dial Addr)</label>
            <input v-model="form.dial_addr" placeholder="可选，填 IP 可免域名解析" />

            <label v-if="showSocks5">Socks5 代理</label>
            <input v-if="showSocks5" v-model="form.socks5" placeholder="host:port" />

            <label v-if="showPipeline">Enable Pipeline</label>
            <label v-if="showPipeline" class="switch-inline">
              <input v-model="form.enable_pipeline" type="checkbox" />
              <span>{{ form.enable_pipeline ? '开启' : '关闭' }}</span>
            </label>

            <label v-if="showHttp3">Enable HTTP/3</label>
            <label v-if="showHttp3" class="switch-inline">
              <input v-model="form.enable_http3" type="checkbox" />
              <span>{{ form.enable_http3 ? '开启' : '关闭' }}</span>
            </label>

            <label v-if="showTlsVerify">Insecure Skip Verify</label>
            <label v-if="showTlsVerify" class="switch-inline">
              <input v-model="form.insecure_skip_verify" type="checkbox" />
              <span>{{ form.insecure_skip_verify ? '开启' : '关闭' }}</span>
            </label>

            <label>Bootstrap Server</label>
            <input v-model="form.bootstrap" placeholder="可选，解析服务器域名用的 DNS" />

            <label>Bootstrap Version</label>
            <select v-model.number="form.bootstrap_version">
              <option :value="0">0 (自动/默认)</option>
              <option :value="4">4 (IPv4)</option>
              <option :value="6">6 (IPv6)</option>
            </select>

            <label>Idle Timeout (秒)</label>
            <input v-model.number="form.idle_timeout" type="number" min="0" placeholder="空闲超时" />

            <label>Query Timeout (毫秒)</label>
            <input v-model.number="form.upstream_query_timeout" type="number" min="0" placeholder="查询超时" />

            <label>Bind Device (网卡)</label>
            <input v-model="form.bind_to_device" placeholder="例如: eth0" />

            <label>SoMark (标记)</label>
            <input v-model.number="form.so_mark" type="number" min="0" placeholder="例如: 100" />
          </template>

          <template v-else>
            <label>Account ID</label>
            <input v-model="form.account_id" />

            <label>Access Key ID</label>
            <input v-model="form.access_key_id" />

            <label>Access Key Secret</label>
            <input v-model="form.access_key_secret" />

            <label>Server Addr</label>
            <input v-model="form.server_addr" />

            <label>ECS Client IP</label>
            <input v-model="form.ecs_client_ip" />

            <label>ECS Client Mask</label>
            <input v-model.number="form.ecs_client_mask" type="number" min="0" max="128" />
          </template>
        </div>

        <div class="actions">
          <button class="btn secondary" @click="closeEditor">取消</button>
          <button class="btn primary" :disabled="saving" @click="saveUpstream">
            {{ saving ? '保存中...' : '保存' }}
          </button>
        </div>
      </section>
    </div>

    <div v-if="specialModalOpen" class="modal-mask" @click.self="closeSpecialGroupModal">
      <section class="panel special-group-modal-card">
        <header class="panel-header special-group-modal-header">
          <h3>{{ specialEditor.slot ? '修改专属分流组' : '新增专属分流组' }}</h3>
          <button class="btn tiny secondary" type="button" @click="closeSpecialGroupModal" aria-label="Close">✕</button>
        </header>
        <div class="form-grid special-group-form-grid">
          <label for="special-group-name-vue">组名</label>
          <input
            id="special-group-name-vue"
            v-model="specialEditor.name"
            type="text"
            placeholder="例如：移动上游 / CMCC"
            @keyup.enter="saveSpecialGroup"
          />
        </div>
        <p class="muted">保存后可在上游设置中维护该组上游，并在在线分流中直接选择该组。</p>
        <div class="actions">
          <button class="btn secondary" type="button" @click="closeSpecialGroupModal">取消</button>
          <button class="btn primary" type="button" :disabled="specialSaving" @click="saveSpecialGroup">
            {{ specialSaving ? '保存中...' : '保存' }}
          </button>
        </div>
      </section>
    </div>
  </section>
</template>

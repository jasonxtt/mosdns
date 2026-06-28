<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { deleteRequest, getJSON, getText, postJSON } from '../api/http'
import { openConfirm } from '../utils/confirm'
import { clearTopNotice, setError, setSuccess } from '../utils/notice'
import { orderUpstreamGroups, upstreamAddressDisplay, upstreamGroupDisplay } from '../utils/upstreamStats'

defineProps({
  mode: {
    type: String,
    default: 'upstream-settings'
  }
})

const HIDE_DISABLED_KEY = 'mosdnsHideDisabledUpstreams'
const DNS_ROUTING_SWITCH_TAG = 'switch17'

const loading = ref(false)
const saving = ref(false)
const filterGroup = ref('all')
const showEditor = ref(false)
const hideDisabled = ref(false)
const dnsRoutingMode = ref('')

const sortState = reactive({
  key: '',
  order: 'desc'
})

const upstreamTags = ref([])
const upstreamConfig = ref({})
const specialGroups = ref([])
const globalSocks5 = ref('')
const specialGroupsManagerOpen = ref(false)
const specialModalOpen = ref(false)
const specialSaving = ref(false)
const specialEditor = reactive({
  slot: 0,
  name: '',
  listenPort: '',
  customPortOnly: false
})
const editingCtx = ref({ group: '', index: -1 })

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
  { value: 'https', label: 'DoH (HTTPS)' },
  { value: 'quic', label: 'DoQ (QUIC)' },
  { value: 'aliapi', label: '阿里 API (AliAPI)' }
]

function isSpecialUpstreamTag(tag) {
  return /^special_upstream_\d+$/.test(String(tag || ''))
}

function normalizeProtocolAlias(protocol) {
  const value = String(protocol || '').trim().toLowerCase()
  switch (value) {
    case 'dot':
      return 'tls'
    case 'doh':
      return 'https'
    case 'doq':
      return 'quic'
    default:
      return value
  }
}

const protocolValue = computed(() => normalizeProtocolAlias(form.protocol))
const isAliapi = computed(() => protocolValue.value === 'aliapi')
const showPipeline = computed(() => ['tcp', 'dot', 'tls'].includes(protocolValue.value))
const showHttp3 = computed(() => ['https', 'doh', 'quic', 'doq'].includes(protocolValue.value))
const showSocks5 = computed(() => ['dot', 'tls', 'tcp', 'doh', 'https', 'quic', 'doq'].includes(protocolValue.value))
const showTlsVerify = computed(() => ['dot', 'tls', 'tcp', 'doh', 'https', 'quic', 'doq'].includes(protocolValue.value))
const showForeignSocksFallbackHint = computed(() => {
  if (!showSocks5.value) {
    return false
  }
  if (String(form.group || '').trim() !== 'foreign') {
    return false
  }
  if (String(form.socks5 || '').trim()) {
    return false
  }
  return Boolean(String(globalSocks5.value || '').trim())
})

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
  return orderUpstreamGroups(Array.from(options), specialGroups.value)
})

const hideDisabledLabel = computed(() => (hideDisabled.value ? '显示全部上游' : '隐藏未启用上游'))
const isRealIpRoutingMode = computed(() => dnsRoutingMode.value === 'B')

function isModeDisabledGroup(group) {
  return isRealIpRoutingMode.value && String(group || '').trim() === 'nocnfake'
}

function isRowModeDisabled(row) {
  return isModeDisabledGroup(row?.group)
}

function isRowEffectiveEnabled(row) {
  return Boolean(row?.data?.enabled) && !isRowModeDisabled(row)
}

function groupDisplayName(group) {
  return upstreamGroupDisplay(group, specialGroups.value).title
}

const specialGroupCards = computed(() => {
  return (specialGroups.value || []).map((group) => {
    const upstreamCount = Array.isArray(upstreamConfig.value?.[group?.upstream_plugin_tag])
      ? upstreamConfig.value[group.upstream_plugin_tag].length
      : 0

    return {
      ...group,
      portLabel: group?.listen_port ? `监听端口 ${group.listen_port}` : '未设置专属端口',
      routeLabel: group?.listen_port
        ? (group?.custom_port_only ? '仅自定义端口生效' : '53端口 + 自定义端口')
        : '53端口生效',
      upstreamCountLabel: `已绑定 ${upstreamCount} 个上游`
    }
  })
})

const summarySpecialGroups = computed(() => specialGroupCards.value.slice(0, 2))
const summarySpecialGroupsOverflow = computed(() => Math.max(0, specialGroupCards.value.length - summarySpecialGroups.value.length))

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
    case 'address':
      return rowAddress(row.data || {})
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
      const row = {
        group,
        index,
        originalOrder,
        data: item || {}
      }
      if (hideDisabled.value && !isRowEffectiveEnabled(row)) {
        return
      }
      all.push(row)
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
  return upstreamAddressDisplay(item)
}

function rowStatusLabel(row) {
  if (isRowModeDisabled(row)) {
    return '当前模式未启用'
  }
  return Boolean(row?.data?.enabled) ? '已启用' : '已关闭'
}

function blockModeDisabledGroup(group) {
  if (!isModeDisabledGroup(group)) {
    return false
  }
  setError('RealIP 分流模式下，国外 FakeIP 上游当前未启用。请切换到 FakeIP 分流后再编辑。')
  return true
}

function resetMessage() {
  clearTopNotice()
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
    const [tagsRes, configRes, groupsRes, overridesRes, dnsModeRes] = await Promise.allSettled([
      getJSON('/api/v1/upstream/tags'),
      getJSON('/api/v1/upstream/config'),
      getJSON('/api/v1/special-groups'),
      getJSON('/api/v1/overrides'),
      getText(`/plugins/${DNS_ROUTING_SWITCH_TAG}/show`)
    ])
    upstreamTags.value = tagsRes.status === 'fulfilled' && Array.isArray(tagsRes.value) ? tagsRes.value : []
    upstreamConfig.value = configRes.status === 'fulfilled' && configRes.value ? configRes.value : {}
    specialGroups.value = groupsRes.status === 'fulfilled' && Array.isArray(groupsRes.value) ? groupsRes.value : []
    globalSocks5.value = overridesRes.status === 'fulfilled'
      ? String(overridesRes.value?.socks5 || '').trim()
      : ''
    dnsRoutingMode.value = dnsModeRes.status === 'fulfilled' ? String(dnsModeRes.value || '').trim() : ''

    if (tagsRes.status === 'rejected' || configRes.status === 'rejected' || groupsRes.status === 'rejected' || overridesRes.status === 'rejected' || dnsModeRes.status === 'rejected') {
      setError('部分数据加载失败，已使用可用数据渲染页面。')
    }
  } catch (error) {
    setError(`加载上游配置失败: ${error.message}`)
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
  if (blockModeDisabledGroup(row?.group)) {
    return
  }
  const item = row.data || {}
  editingCtx.value = { group: row.group, index: row.index }
  resetForm()

  form.group = row.group
  form.tag = String(item.tag || '')
  form.protocol = normalizeProtocolAlias(item.protocol || 'udp')
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
  specialEditor.listenPort = ''
  specialEditor.customPortOnly = false
  specialModalOpen.value = true
}

function openSpecialGroupsManager() {
  resetMessage()
  specialGroupsManagerOpen.value = true
}

function openEditSpecialGroup(group) {
  resetMessage()
  specialEditor.slot = Number(group?.slot) || 0
  specialEditor.name = String(group?.name || '')
  specialEditor.listenPort = group?.listen_port ? String(group.listen_port) : ''
  specialEditor.customPortOnly = Boolean(group?.custom_port_only && group?.listen_port)
  specialModalOpen.value = true
}

function closeSpecialGroupModal() {
  specialModalOpen.value = false
}

function closeSpecialGroupsManager() {
  specialGroupsManagerOpen.value = false
}

async function saveSpecialGroup() {
  const name = String(specialEditor.name || '').trim()
  if (!name) {
    setError('专属分流组名称不能为空')
    return
  }
  const listenPortText = String(specialEditor.listenPort || '').trim()
  let listenPort = 0
  if (listenPortText) {
    const parsed = Number(listenPortText)
    if (!Number.isInteger(parsed) || parsed < 1 || parsed > 65535) {
      setError('监听端口必须在 1-65535 之间')
      return
    }
    if (parsed === 53) {
      setError('监听端口不能使用 53')
      return
    }
    listenPort = parsed
  }
  const customPortOnly = listenPort !== 0 && Boolean(specialEditor.customPortOnly)

  specialSaving.value = true
  resetMessage()
  try {
    await postJSON('/api/v1/special-groups', {
      slot: Number(specialEditor.slot) || 0,
      name,
      listen_port: listenPort,
      custom_port_only: customPortOnly
    })
    setSuccess('专属分流组已保存')
    closeSpecialGroupModal()
    await loadData()
  } catch (error) {
    setError(`保存专属分流组失败: ${error.message}`)
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
    setSuccess('专属分流组已删除')
    await loadData()
  } catch (error) {
    setError(`删除专属分流组失败: ${error.message}`)
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
    setError('请选择所属组')
    return
  }
  if (blockModeDisabledGroup(group)) {
    return
  }
  if (!tag) {
    setError('上游标识不能为空')
    return
  }
  if (!protocol) {
    setError('协议不能为空')
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
    setSuccess('上游配置已保存')
    showEditor.value = false
    await loadData()
  } catch (error) {
    setError(`保存失败: ${error.message}`)
  } finally {
    saving.value = false
  }
}

async function removeRow(row) {
  resetMessage()
  if (blockModeDisabledGroup(row?.group)) {
    return
  }
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
    setSuccess('上游已删除')
    await loadData()
  } catch (error) {
    setError(`删除失败: ${error.message}`)
  }
}

async function toggleEnable(row) {
  resetMessage()
  if (blockModeDisabledGroup(row?.group)) {
    return
  }
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
    setError(`切换失败: ${error.message}`)
  }
}

function handleGlobalRefresh() {
  loadData()
}

watch(() => specialEditor.listenPort, (value) => {
  if (!String(value || '').trim()) {
    specialEditor.customPortOnly = false
  }
})

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
    <div class="upstream-toolbar">
      <div class="upstream-toolbar-left">
        <button class="btn primary entry-action-btn" type="button" @click="beginAdd">添加上游DNS</button>
        <section class="special-groups-summary" aria-label="专属分流组摘要">
          <div class="special-groups-summary-copy">
            <span class="special-groups-summary-title">专属分流组</span>
            <div v-if="summarySpecialGroups.length > 0" class="special-groups-summary-list">
              <span v-for="group in summarySpecialGroups" :key="group.slot" class="special-groups-summary-chip" :title="group.portLabel">
                {{ group.listen_port ? `${group.name} · ${group.listen_port}` : group.name }}
              </span>
              <span v-if="summarySpecialGroupsOverflow > 0" class="special-groups-summary-chip summary-overflow-chip">
                +{{ summarySpecialGroupsOverflow }}
              </span>
            </div>
            <span v-else class="special-groups-summary-empty">暂未配置</span>
          </div>
          <button class="btn secondary" type="button" @click="openSpecialGroupsManager">管理</button>
        </section>
      </div>
    </div>

    <div v-if="specialGroupsManagerOpen" class="modal-mask" @click.self="closeSpecialGroupsManager">
      <section class="panel special-groups-manager-modal">
        <header class="panel-header special-groups-manager-header">
          <div class="special-groups-panel-copy">
            <h3>专属分流组管理</h3>
            <p class="muted">管理组名、监听端口和删除操作</p>
          </div>
          <button class="btn tiny secondary" type="button" @click="closeSpecialGroupsManager" aria-label="Close">✕</button>
        </header>

        <div class="special-groups-manager-actions">
          <button class="btn secondary entry-action-btn" type="button" @click="openCreateSpecialGroup">新增专属分流组</button>
        </div>

        <div v-if="specialGroupCards.length === 0" class="special-group-empty">
          <strong>还没有专属分流组</strong>
          <p>新增后即可在上游设置和在线分流里使用，也可以为该组单独设置监听端口。</p>
        </div>

        <div v-else class="special-groups-grid">
          <article v-for="group in specialGroupCards" :key="group.slot" class="special-group-card">
            <div class="special-group-card-top">
              <div class="special-group-heading">
                <h4 :title="group.name">{{ group.name }}</h4>
                <span class="special-group-port-chip" :class="{ unset: !group.listen_port }">
                  {{ group.portLabel }}
                </span>
              </div>
              <p class="special-group-meta">{{ group.routeLabel }} · {{ group.upstreamCountLabel }}</p>
            </div>
            <div class="special-group-actions special-group-card-actions">
              <button class="btn tiny secondary" type="button" @click="openEditSpecialGroup(group)">编辑</button>
              <button class="btn tiny danger" type="button" @click="deleteSpecialGroup(group)">删除</button>
            </div>
          </article>
        </div>
      </section>
    </div>

    <div v-if="showEditor" class="modal-mask">
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
            <div v-if="showSocks5">
              <input v-model="form.socks5" placeholder="host:port" />
              <small v-if="showForeignSocksFallbackHint" class="muted">
                当前为空时会自动继承系统设置中的 SOCKS5：{{ globalSocks5 }}
              </small>
            </div>

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

    <div class="toolbar upstream-filter-toolbar">
      <label for="group-filter">过滤分组</label>
      <select id="group-filter" v-model="filterGroup">
        <option value="all">全部</option>
        <option v-for="group in groupOptions" :key="group" :value="group">{{ groupDisplayName(group) }}</option>
      </select>
      <button class="btn secondary" type="button" @click="toggleHideDisabled">{{ hideDisabledLabel }}</button>
    </div>

    <div class="table-wrap adaptive-table-wrap upstream-adaptive-wrap">
      <table class="upstream-adaptive-table">
        <thead>
          <tr>
            <th class="sortable" @click="onSort('enabled')">启用 <span class="sort-indicator">{{ sortIndicator('enabled') }}</span></th>
            <th class="sortable" @click="onSort('group')">所属组 <span class="sort-indicator">{{ sortIndicator('group') }}</span></th>
            <th class="sortable" @click="onSort('tag')">标识 <span class="sort-indicator">{{ sortIndicator('tag') }}</span></th>
            <th class="sortable" @click="onSort('protocol')">协议 <span class="sort-indicator">{{ sortIndicator('protocol') }}</span></th>
            <th class="sortable" @click="onSort('address')">地址 <span class="sort-indicator">{{ sortIndicator('address') }}</span></th>
            <th>操作</th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="loading">
            <td colspan="6" class="empty">加载中...</td>
          </tr>
          <tr v-else-if="rows.length === 0">
            <td colspan="6" class="empty">{{ hideDisabled ? '当前没有已启用的上游配置' : '暂无上游配置' }}</td>
          </tr>
          <tr
            v-for="row in rows"
            :key="`${row.group}-${row.index}-${row.data?.tag || 'x'}`"
            :class="{ disabled: !isRowEffectiveEnabled(row), 'upstream-row-mode-disabled': isRowModeDisabled(row) }"
          >
            <td>
              <label class="switch switch-table">
                <input
                  type="checkbox"
                  :checked="Boolean(row.data?.enabled)"
                  :disabled="isRowModeDisabled(row)"
                  @change="toggleEnable(row)"
                />
                <span class="slider"></span>
              </label>
              <span v-if="isRowModeDisabled(row)" class="upstream-mode-disabled-chip">当前模式未启用</span>
            </td>
            <td :title="groupDisplayName(row.group)">{{ groupDisplayName(row.group) }}</td>
            <td :title="row.data?.tag || '-'">{{ row.data?.tag || '-' }}</td>
            <td :title="row.data?.protocol || '-'">{{ row.data?.protocol || '-' }}</td>
            <td :title="rowAddress(row.data || {})" class="mono">{{ rowAddress(row.data || {}) }}</td>
            <td class="row-actions">
              <button class="btn tiny secondary" :disabled="isRowModeDisabled(row)" @click="beginEdit(row)">编辑</button>
              <button class="btn tiny danger" :disabled="isRowModeDisabled(row)" @click="removeRow(row)">删除</button>
            </td>
          </tr>
        </tbody>
      </table>
    </div>

    <div v-if="specialModalOpen" class="modal-mask">
      <section class="panel special-group-modal-card">
        <header class="panel-header special-group-modal-header">
          <h3>{{ specialEditor.slot ? '编辑专属分流组' : '新增专属分流组' }}</h3>
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
          <label for="special-group-port-vue">监听端口</label>
          <input
            id="special-group-port-vue"
            v-model="specialEditor.listenPort"
            type="number"
            min="1"
            max="65535"
            placeholder="留空则沿用原逻辑"
            @keyup.enter="saveSpecialGroup"
          />
          <label for="special-group-port-only-vue">仅自定义端口生效</label>
          <label class="switch-inline">
            <input
              id="special-group-port-only-vue"
              v-model="specialEditor.customPortOnly"
              type="checkbox"
              :disabled="!String(specialEditor.listenPort || '').trim()"
            />
          </label>
        </div>
        <p class="muted">1.未勾选则53端口及自定义端口均生效</p>
        <p class="muted">2.保存后可在上游设置中维护该组上游，并在在线分流中直接选择该组。</p>
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

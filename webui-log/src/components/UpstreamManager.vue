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
const upstreamSources = ref([])
const specialGroups = ref([])
const globalSocks5 = ref('')
const specialGroupsManagerOpen = ref(false)
const specialModalOpen = ref(false)
const specialSaving = ref(false)
const specialEditor = reactive({
  slot: 0,
  name: '',
  listenPort: '',
  customPortOnly: false,
  upstreamSources: [],
  ownedUpstreams: []
})
const specialSourceDraft = reactive({
  pluginTag: '',
  kind: 'group',
  upstreamTag: ''
})
const editingCtx = ref({ group: '', index: -1, specialOwned: false })

const form = reactive({
  group: '',
  tag: '',
  protocol: 'aliapi',
  addr: '',
  dial_addr: '',
  socks5: '',
  use_socks_proxy: true,
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
const showSocksProxyToggle = computed(() => showSocks5.value && ['foreign', 'foreignecs'].includes(String(form.group || '').trim()))
const showSocks5Input = computed(() => showSocks5.value && (!showSocksProxyToggle.value || form.use_socks_proxy))
const showForeignSocksFallbackHint = computed(() => {
  if (!showSocks5Input.value) {
    return false
  }
  if (!['foreign', 'foreignecs'].includes(String(form.group || '').trim())) {
    return false
  }
  if (!form.use_socks_proxy || String(form.socks5 || '').trim()) {
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
  return orderUpstreamGroups(Array.from(options), specialGroups.value)
})

const sourceDraftGroup = computed(() => {
  return (upstreamSources.value || []).find((group) => group?.plugin_tag === specialSourceDraft.pluginTag) || null
})

const sourceDraftUpstreams = computed(() => {
  return Array.isArray(sourceDraftGroup.value?.upstreams) ? sourceDraftGroup.value.upstreams : []
})

const specialGroupSourceRows = computed(() => {
  return Array.isArray(specialEditor.upstreamSources) ? specialEditor.upstreamSources : []
})

const specialGroupOwnedRows = computed(() => {
  return Array.isArray(specialEditor.ownedUpstreams) ? specialEditor.ownedUpstreams : []
})

const isSpecialOwnedEditing = computed(() => Boolean(editingCtx.value.specialOwned))

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
      routeLabel: group?.upstream_active
        ? (group?.listen_port
            ? (group?.custom_port_only ? '仅自定义端口生效' : '53端口 + 自定义端口')
            : '53端口生效')
        : '当前不参与分流',
      upstreamCountLabel: group?.upstream_active
        ? `当前生效 ${group.effective_upstream_count || upstreamCount} 个上游`
        : '当前无生效上游'
    }
  })
})

const activeSpecialEditorGroup = computed(() => {
  return (specialGroups.value || []).find((group) => Number(group?.slot) === Number(specialEditor.slot)) || null
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
    if (isSpecialUpstreamTag(group)) {
      return
    }
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
  form.use_socks_proxy = true
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
    const [tagsRes, configRes, sourcesRes, groupsRes, overridesRes, dnsModeRes] = await Promise.allSettled([
      getJSON('/api/v1/upstream/tags'),
      getJSON('/api/v1/upstream/config'),
      getJSON('/api/v1/upstream/sources'),
      getJSON('/api/v1/special-groups'),
      getJSON('/api/v1/overrides'),
      getText(`/plugins/${DNS_ROUTING_SWITCH_TAG}/show`)
    ])
    upstreamTags.value = tagsRes.status === 'fulfilled' && Array.isArray(tagsRes.value) ? tagsRes.value : []
    upstreamConfig.value = configRes.status === 'fulfilled' && configRes.value ? configRes.value : {}
    upstreamSources.value = sourcesRes.status === 'fulfilled' && Array.isArray(sourcesRes.value) ? sourcesRes.value : []
    specialGroups.value = groupsRes.status === 'fulfilled' && Array.isArray(groupsRes.value) ? groupsRes.value : []
    globalSocks5.value = overridesRes.status === 'fulfilled'
      ? String(overridesRes.value?.socks5 || '').trim()
      : ''
    dnsRoutingMode.value = dnsModeRes.status === 'fulfilled' ? String(dnsModeRes.value || '').trim() : ''

    if (tagsRes.status === 'rejected' || configRes.status === 'rejected' || sourcesRes.status === 'rejected' || groupsRes.status === 'rejected' || overridesRes.status === 'rejected' || dnsModeRes.status === 'rejected') {
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
  editingCtx.value = { group: '', index: -1, specialOwned: false }
  resetForm()
  form.group = groupOptions.value[0] || ''
  showEditor.value = true
}

function fillUpstreamForm(item, group) {
  const data = item || {}
  resetForm()
  form.group = group
  form.tag = String(data.tag || '')
  form.protocol = normalizeProtocolAlias(data.protocol || 'udp')
  form.addr = String(data.addr || '')
  form.dial_addr = String(data.dial_addr || '')
  form.socks5 = String(data.socks5 || '')
  form.use_socks_proxy = Boolean(data.use_socks_proxy)
  form.bootstrap = String(data.bootstrap || '')
  form.bootstrap_version = toInt(data.bootstrap_version, 0)
  form.enable_pipeline = Boolean(data.enable_pipeline)
  form.enable_http3 = Boolean(data.enable_http3)
  form.insecure_skip_verify = Boolean(data.insecure_skip_verify)
  form.idle_timeout = toInt(data.idle_timeout, 0)
  form.upstream_query_timeout = toInt(data.upstream_query_timeout, 0)
  form.bind_to_device = String(data.bind_to_device || '')
  form.so_mark = toInt(data.so_mark, 0)
  form.account_id = String(data.account_id || '')
  form.access_key_id = String(data.access_key_id || '')
  form.access_key_secret = String(data.access_key_secret || '')
  form.server_addr = String(data.server_addr || '223.5.5.5')
  form.ecs_client_ip = String(data.ecs_client_ip || '')
  form.ecs_client_mask = toInt(data.ecs_client_mask, 0)
}

function beginEdit(row) {
  resetMessage()
  if (blockModeDisabledGroup(row?.group)) {
    return
  }
  const item = row.data || {}
  editingCtx.value = { group: row.group, index: row.index, specialOwned: false }
  fillUpstreamForm(item, row.group)
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
  specialEditor.upstreamSources = []
  specialEditor.ownedUpstreams = []
  resetSpecialSourceDraft()
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
  specialEditor.upstreamSources = Array.isArray(group?.upstream_sources)
    ? group.upstream_sources.map((source) => ({ ...source }))
    : []
  specialEditor.ownedUpstreams = Array.isArray(group?.owned_upstreams)
    ? group.owned_upstreams.map((item) => ({ ...item }))
    : []
  resetSpecialSourceDraft()
  specialModalOpen.value = true
}

function closeSpecialGroupModal() {
  specialModalOpen.value = false
}

function closeSpecialGroupsManager() {
  specialGroupsManagerOpen.value = false
}

function resetSpecialSourceDraft() {
  specialSourceDraft.pluginTag = ''
  specialSourceDraft.kind = 'group'
  specialSourceDraft.upstreamTag = ''
}

function specialSourceLabel(source) {
  if (source?.kind === 'group') {
    const group = (upstreamSources.value || []).find((item) => item?.plugin_tag === source.plugin_tag)
    const count = Array.isArray(group?.upstreams) ? group.upstreams.length : 0
    return `${source.plugin_tag}（整个组，当前 ${count} 个上游）`
  }
  return `${source?.plugin_tag || '-'} / ${source?.upstream_tag || '-'}`
}

function specialSourceStatus(source) {
  const group = (upstreamSources.value || []).find((item) => item?.plugin_tag === source?.plugin_tag)
  if (!group) {
    return '源上游组不存在'
  }
  if (source?.kind === 'group') {
    const enabled = (group.upstreams || []).filter((item) => item?.enabled).length
    return enabled > 0 ? `${enabled} 个上游当前启用` : '源组没有启用中的上游'
  }
  const entry = (group.upstreams || []).find((item) => item?.tag === source?.upstream_tag)
  if (!entry) {
    return '源上游不存在'
  }
  return entry.enabled ? '当前启用' : '源上游已关闭'
}

function addSpecialSource() {
  const pluginTag = String(specialSourceDraft.pluginTag || '').trim()
  const kind = specialSourceDraft.kind === 'upstream' ? 'upstream' : 'group'
  const upstreamTag = String(specialSourceDraft.upstreamTag || '').trim()
  if (!pluginTag) {
    setError('请选择要引用的上游组')
    return
  }
  if (kind === 'upstream' && !upstreamTag) {
    setError('请选择要引用的上游标识')
    return
  }
  const duplicate = specialGroupSourceRows.value.some((source) => (
    source.kind === kind && source.plugin_tag === pluginTag && source.upstream_tag === upstreamTag
  ))
  if (duplicate) {
    setError('该上游引用已经添加')
    return
  }
  specialEditor.upstreamSources.push({
    kind,
    plugin_tag: pluginTag,
    ...(kind === 'upstream' ? { upstream_tag: upstreamTag } : {})
  })
  specialSourceDraft.upstreamTag = ''
  setSuccess('上游引用已加入，点击专属组保存后生效')
}

function removeSpecialSource(index) {
  specialEditor.upstreamSources.splice(index, 1)
}

function beginAddSpecialOwned(group) {
  resetMessage()
  editingCtx.value = {
    group: String(group?.upstream_plugin_tag || ''),
    index: -1,
    specialOwned: true
  }
  resetForm()
  form.group = editingCtx.value.group
  showEditor.value = true
}

function beginEditSpecialOwned(group, index) {
  resetMessage()
  const item = specialGroupOwnedRows.value[index] || {}
  editingCtx.value = {
    group: String(group?.upstream_plugin_tag || ''),
    index,
    specialOwned: true
  }
  fillUpstreamForm(item, editingCtx.value.group)
  showEditor.value = true
}

function removeSpecialOwned(index) {
  specialEditor.ownedUpstreams.splice(index, 1)
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
    const saved = await postJSON('/api/v1/special-groups', {
      slot: Number(specialEditor.slot) || 0,
      name,
      listen_port: listenPort,
      custom_port_only: customPortOnly,
      upstream_sources: specialGroupSourceRows.value.map((source) => ({ ...source })),
      upstreams: specialGroupOwnedRows.value.map((item) => ({ ...item }))
    })
    closeSpecialGroupModal()
    await loadData()
    const followup = String(saved?.message || '').trim()
    setSuccess(followup ? `专属分流组已保存。${followup}` : '专属分流组已保存')
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
  const targetGroupUsesSocksToggle = ['foreign', 'foreignecs'].includes(String(form.group || '').trim())
  const useSocksProxy = protocol !== 'aliapi' && targetGroupUsesSocksToggle ? Boolean(form.use_socks_proxy) : undefined
  const socks5Value = protocol !== 'aliapi' && (!targetGroupUsesSocksToggle || useSocksProxy)
    ? String(form.socks5 || '').trim()
    : ''
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
    socks5: socks5Value,
    use_socks_proxy: useSocksProxy,
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

  if (isSpecialOwnedEditing.value) {
    if (!tag) {
      setError('上游标识不能为空')
      return
    }
    if (!protocol) {
      setError('协议不能为空')
      return
    }
    const list = Array.isArray(specialEditor.ownedUpstreams) ? [...specialEditor.ownedUpstreams] : []
    const currentIndex = editingCtx.value.index
    const current = currentIndex >= 0 ? list[currentIndex] || {} : {}
    const next = buildUpstreamObject(currentIndex < 0 ? true : Boolean(current.enabled))
    const duplicate = list.some((item, index) => index !== currentIndex && String(item?.tag || '').trim() === tag)
    if (duplicate) {
      setError(`专属自有上游标识重复：${tag}`)
      return
    }
    if (currentIndex >= 0) {
      list[currentIndex] = next
    } else {
      list.push(next)
    }
    specialEditor.ownedUpstreams = list
    showEditor.value = false
    setSuccess('自有上游已加入，点击专属组保存后生效')
    return
  }

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

watch(() => specialSourceDraft.pluginTag, () => {
  specialSourceDraft.upstreamTag = ''
})

watch(() => specialSourceDraft.kind, () => {
  specialSourceDraft.upstreamTag = ''
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
            <p class="muted">管理组名、上游来源、监听端口和删除操作</p>
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
              <p v-if="group.upstream_warnings?.length" class="special-group-warning">
                {{ group.upstream_warnings[0] }}
              </p>
            </div>
            <div class="special-group-actions special-group-card-actions">
              <button class="btn tiny secondary" type="button" @click="openEditSpecialGroup(group)">编辑</button>
              <button class="btn tiny danger" type="button" @click="deleteSpecialGroup(group)">删除</button>
            </div>
          </article>
        </div>
      </section>
    </div>

    <Teleport to="body">
    <div v-if="showEditor" class="modal-mask upstream-editor-modal-mask">
      <section class="panel form-modal-card upstream-editor-modal-card">
        <header class="panel-header upstream-editor-modal-header">
          <h3>{{ editingCtx.index >= 0 ? '编辑上游' : '新增上游' }}</h3>
          <button class="btn tiny secondary upstream-editor-close" type="button" aria-label="关闭" @click="closeEditor">✕</button>
        </header>
        <div class="upstream-editor-modal-body">
        <div class="form-grid">
          <label>所属组</label>
          <input v-if="editingCtx.index >= 0 || isSpecialOwnedEditing" :value="isSpecialOwnedEditing ? (specialEditor.name || '当前专属分流组') : form.group" disabled />
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

            <label v-if="showSocksProxyToggle">使用 socks 代理</label>
            <label v-if="showSocksProxyToggle" class="switch-inline">
              <input v-model="form.use_socks_proxy" type="checkbox" />
              <span>{{ form.use_socks_proxy ? '开启' : '关闭' }}</span>
            </label>

            <label v-if="showSocks5Input">Socks5 代理</label>
            <div v-if="showSocks5Input">
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
        </div>
      </section>
    </div>
    </Teleport>

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
        </div>

        <section class="special-upstream-settings">
          <div class="special-upstream-settings-header">
            <div>
              <h4>上游设置</h4>
              <p class="muted">可引用现有上游，也可以添加仅属于本组的上游。</p>
            </div>
            <span class="special-upstream-count">
              生效 {{ activeSpecialEditorGroup?.effective_upstream_count || 0 }} 个
            </span>
          </div>

          <div class="special-source-section">
            <div class="special-source-section-header">
              <h5>
                引用已有上游
                <span
                  class="special-source-help"
                  role="img"
                  aria-label="引用关系说明"
                  title="引用关系会实时跟随来源上游组的修改、禁用和删除。"
                >ⓘ</span>
              </h5>
            </div>

            <div
              class="special-source-add-row"
              :class="{ 'has-upstream-tag': specialSourceDraft.kind === 'upstream' }"
            >
              <label class="special-source-field">
                <span>来源上游组</span>
                <select v-model="specialSourceDraft.pluginTag">
                  <option value="" disabled>选择上游组</option>
                  <option v-for="group in upstreamSources" :key="group.plugin_tag" :value="group.plugin_tag">
                    {{ groupDisplayName(group.plugin_tag) }}（{{ group.plugin_tag }}）
                  </option>
                </select>
              </label>
              <label class="special-source-field">
                <span>引用范围</span>
                <select v-model="specialSourceDraft.kind">
                  <option value="group">整个上游组</option>
                  <option value="upstream">组内单个上游</option>
                </select>
              </label>
              <label v-if="specialSourceDraft.kind === 'upstream'" class="special-source-field">
                <span>上游标识</span>
                <select v-model="specialSourceDraft.upstreamTag">
                  <option value="" disabled>选择上游标识</option>
                  <option v-for="item in sourceDraftUpstreams" :key="item.tag" :value="item.tag">
                    {{ item.tag || '未设置标识' }}{{ item.enabled ? '' : '（已关闭）' }}
                  </option>
                </select>
              </label>
              <button class="btn tiny secondary" type="button" @click="addSpecialSource">＋ 添加</button>
            </div>

            <div v-if="specialGroupSourceRows.length > 0" class="special-source-list">
              <div v-for="(source, index) in specialGroupSourceRows" :key="`${source.kind}-${source.plugin_tag}-${source.upstream_tag || 'group'}`" class="special-source-row">
                <div class="special-source-row-copy">
                  <strong>{{ specialSourceLabel(source) }}</strong>
                  <span class="muted">{{ specialSourceStatus(source) }}</span>
                </div>
                <button class="btn tiny secondary" type="button" @click="removeSpecialSource(index)">解绑</button>
              </div>
            </div>
            <p v-else class="special-source-empty">暂无引用</p>
          </div>

          <div class="special-owned-section">
            <div class="special-owned-header">
              <div>
                <h4>本组专属上游</h4>
                <p class="muted">仅供当前分流组使用，删除分流组时同步删除。</p>
              </div>
              <button class="btn tiny secondary" type="button" @click="beginAddSpecialOwned({ upstream_plugin_tag: specialEditor.slot ? `special_upstream_${specialEditor.slot}` : '' })">
                ＋ 新增上游
              </button>
            </div>

            <div v-if="specialGroupOwnedRows.length > 0" class="special-owned-list">
              <div v-for="(item, index) in specialGroupOwnedRows" :key="`${item.tag || 'owned'}-${index}`" class="special-owned-row">
                <div class="special-source-row-copy">
                  <strong>{{ item.tag || '未设置标识' }}</strong>
                  <span class="muted">{{ item.protocol || '-' }} · {{ rowAddress(item) }} · {{ item.enabled ? '已启用' : '已关闭' }}</span>
                </div>
                <div class="special-group-actions">
                  <button class="btn tiny secondary" type="button" @click="beginEditSpecialOwned({ upstream_plugin_tag: specialEditor.slot ? `special_upstream_${specialEditor.slot}` : '' }, index)">编辑</button>
                  <button class="btn tiny danger" type="button" @click="removeSpecialOwned(index)">删除</button>
                </div>
              </div>
            </div>
            <p v-else class="special-source-empty">暂无专属上游</p>
          </div>

          <div v-if="activeSpecialEditorGroup?.upstream_warnings?.length" class="special-upstream-warning-box">
            <strong>当前存在告警</strong>
            <p v-for="warning in activeSpecialEditorGroup.upstream_warnings" :key="warning">{{ warning }}</p>
          </div>
        </section>

        <section class="special-port-settings" aria-label="专属组端口设置">
          <div class="form-grid special-group-form-grid special-port-field">
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
          </div>
          <label class="special-port-toggle-row">
            <span class="special-port-toggle-copy">
              <span class="special-port-toggle-title">仅自定义端口生效</span>
              <span id="special-group-port-only-hint-vue" class="special-port-toggle-hint">关闭时，53 端口和自定义端口均生效</span>
            </span>
            <span class="switch special-port-switch">
              <input
                id="special-group-port-only-vue"
                v-model="specialEditor.customPortOnly"
                type="checkbox"
                aria-describedby="special-group-port-only-hint-vue"
                :disabled="!String(specialEditor.listenPort || '').trim()"
              />
              <span class="slider"></span>
            </span>
          </label>
        </section>

        <div class="actions special-group-modal-actions">
          <button class="btn no-frame-btn" type="button" @click="closeSpecialGroupModal">取消</button>
          <button class="btn primary" type="button" :disabled="specialSaving" @click="saveSpecialGroup">
            {{ specialSaving ? '保存中...' : '保存' }}
          </button>
        </div>
      </section>
    </div>
  </section>
</template>

<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { deleteRequest, getJSON, postJSON, putJSON } from '../api/http'
import { openConfirm } from '../utils/confirm'
import RulesAdguardPanel from './rules/RulesAdguardPanel.vue'
import RulesDiversionPanel from './rules/RulesDiversionPanel.vue'
import RulesAdguardEditorModal from './rules/RulesAdguardEditorModal.vue'
import RulesSpecialGroupsPanel from './rules/RulesSpecialGroupsPanel.vue'
import RulesSpecialEditorModal from './rules/RulesSpecialEditorModal.vue'
import RulesDiversionEditorModal from './rules/RulesDiversionEditorModal.vue'
import { clearTopNotice, setError, setSuccess } from '../utils/notice'
import { formatDateTime } from '../utils/time'

const props = defineProps({
  mode: {
    type: String,
    default: 'all'
  }
})

const mode = computed(() => {
  const allowed = ['all', 'special', 'adguard', 'diversion']
  return allowed.includes(props.mode) ? props.mode : 'all'
})

const activeTab = ref(mode.value === 'all' ? 'special' : mode.value)
const showInnerTabs = computed(() => mode.value === 'all')
const loading = reactive({
  special: false,
  adguard: false,
  diversion: false
})

const specialGroups = ref([])
const adguardRules = ref([])
const diversionRules = ref([])

const builtInDiversionTypes = [
  { value: 'geositecn', label: '中国域名', pluginTag: 'geosite_cn' },
  { value: 'geositenocn', label: '非中国域名', pluginTag: 'geosite_no_cn' },
  { value: 'geoipcn', label: '中国 IP', pluginTag: 'geoip_cn' },
  { value: 'cuscn', label: '!cn@cn', pluginTag: 'cuscn' },
  { value: 'cusnocn', label: 'cn@!cn', pluginTag: 'cusnocn' }
]

const diversionTypeOptions = computed(() => {
  const base = builtInDiversionTypes.map((item) => ({ value: item.value, label: item.label }))
  const special = specialGroups.value.map((group) => ({ value: group.key, label: group.name }))
  return [...base, ...special]
})

const diversionPluginMap = computed(() => {
  const map = {}
  builtInDiversionTypes.forEach((item) => {
    map[item.value] = item.pluginTag
  })
  specialGroups.value.forEach((group) => {
    map[group.key] = group.diversion_plugin_tag
  })
  return map
})

const typeLabelMap = computed(() => {
  const map = {}
  diversionTypeOptions.value.forEach((item) => {
    map[item.value] = item.label
  })
  return map
})

const specialEditor = reactive({
  open: false,
  slot: 0,
  name: '',
  listenPort: '',
  customPortOnly: false
})

const adguardEditor = reactive({
  open: false,
  id: '',
  name: '',
  url: '',
  enabled: true,
  auto_update: true,
  update_interval_hours: 24
})
const adguardRaw = ref(null)

const diversionEditor = reactive({
  open: false,
  oldName: '',
  oldType: '',
  name: '',
  type: 'geositecn',
  files: '',
  url: '',
  enabled: true,
  auto_update: true,
  enable_regexp: false,
  update_interval_hours: 24
})
const diversionRaw = ref(null)
const diversionAutofill = reactive({
  isApplying: false,
  nameDirty: false,
  filesDirty: false
})

const panelTitle = computed(() => {
  if (mode.value === 'special') {
    return '专属分流组'
  }
  if (mode.value === 'adguard') {
    return '广告拦截'
  }
  if (mode.value === 'diversion') {
    return '订阅规则'
  }
  return '规则管理'
})

const panelDesc = computed(() => {
  if (mode.value === 'special') {
    return '管理专属分流组，自动联动上游组和分流插件。'
  }
  if (mode.value === 'adguard') {
    return '管理 AdGuard 在线拦截规则。'
  }
  if (mode.value === 'diversion') {
    return '管理在线分流规则，支持系统类型与专属分流组类型。'
  }
  return '覆盖旧版规则管理核心能力：专属分流组、AdGuard 在线规则、在线分流规则。'
})

function clearMessage() {
  clearTopNotice()
}

function formatTime(value) {
  return formatDateTime(value)
}

function sanitizeRulePayload(rule) {
  const copy = { ...rule }
  delete copy.__type
  delete copy.__pluginTag
  delete copy.__typeLabel
  return copy
}

async function loadSpecialGroups() {
  loading.special = true
  try {
    const data = await getJSON('/api/v1/special-groups')
    specialGroups.value = Array.isArray(data) ? data : []
  } catch (error) {
    setError(`加载专属分流组失败: ${error.message}`)
  } finally {
    loading.special = false
  }
}

async function loadAdguardRules() {
  loading.adguard = true
  try {
    const data = await getJSON('/plugins/adguard/rules')
    adguardRules.value = Array.isArray(data) ? data : []
    adguardRules.value.sort((a, b) => String(a.name || '').localeCompare(String(b.name || '')))
  } catch (error) {
    setError(`加载 AdGuard 规则失败: ${error.message}`)
  } finally {
    loading.adguard = false
  }
}

async function loadDiversionRules() {
  loading.diversion = true
  try {
    const entries = Object.entries(diversionPluginMap.value)
    const tasks = entries.map(async ([type, tag]) => {
      const data = await getJSON(`/plugins/${tag}/config`)
      return { type, tag, rules: Array.isArray(data) ? data : [] }
    })
    const settled = await Promise.allSettled(tasks)
    const merged = []
    settled.forEach((item) => {
      if (item.status !== 'fulfilled') {
        return
      }
      const { type, tag, rules } = item.value
      rules.forEach((rule) => {
        merged.push({
          ...rule,
          type: rule.type || type,
          __pluginTag: tag,
          __type: type,
          __typeLabel: typeLabelMap.value[rule.type || type] || (rule.type || type)
        })
      })
    })
    merged.sort((a, b) => {
      const enabledDiff = Number(Boolean(b.enabled)) - Number(Boolean(a.enabled))
      if (enabledDiff !== 0) {
        return enabledDiff
      }
      const typeDiff = String(a.__typeLabel || '').localeCompare(String(b.__typeLabel || ''))
      if (typeDiff !== 0) {
        return typeDiff
      }
      return String(a.name || '').localeCompare(String(b.name || ''))
    })
    diversionRules.value = merged
  } catch (error) {
    setError(`加载分流规则失败: ${error.message}`)
  } finally {
    loading.diversion = false
  }
}

function shouldShowTab(tab) {
  return showInnerTabs.value ? activeTab.value === tab : mode.value === tab
}

async function reloadCurrentView() {
  clearMessage()
  if (mode.value === 'all') {
    await loadSpecialGroups()
    await Promise.all([loadAdguardRules(), loadDiversionRules()])
    return
  }
  if (mode.value === 'special') {
    await loadSpecialGroups()
    return
  }
  if (mode.value === 'adguard') {
    await loadAdguardRules()
    return
  }
  await loadSpecialGroups()
  await loadDiversionRules()
}

function openCreateSpecial() {
  clearMessage()
  specialEditor.open = true
  specialEditor.slot = 0
  specialEditor.name = ''
  specialEditor.listenPort = ''
  specialEditor.customPortOnly = false
}

function openEditSpecial(group) {
  clearMessage()
  specialEditor.open = true
  specialEditor.slot = Number(group.slot) || 0
  specialEditor.name = String(group.name || '')
  specialEditor.listenPort = group?.listen_port ? String(group.listen_port) : ''
  specialEditor.customPortOnly = Boolean(group?.custom_port_only && group?.listen_port)
}

function closeSpecialEditor() {
  specialEditor.open = false
}

async function saveSpecial() {
  const name = specialEditor.name.trim()
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
  try {
    await postJSON('/api/v1/special-groups', {
      slot: Number(specialEditor.slot) || 0,
      name,
      listen_port: listenPort,
      custom_port_only: customPortOnly
    })
    setSuccess('专属分流组已保存')
    closeSpecialEditor()
    await loadSpecialGroups()
    if (mode.value === 'all' || mode.value === 'diversion') {
      await loadDiversionRules()
    }
  } catch (error) {
    setError(`保存专属分流组失败: ${error.message}`)
  }
}

async function deleteSpecial(group) {
  if (!(await openConfirm(`确定删除专属分流组“${group.name}”吗？`, { tone: 'danger' }))) {
    return
  }
  try {
    await deleteRequest(`/api/v1/special-groups/${group.slot}`)
    setSuccess('专属分流组已删除')
    await loadSpecialGroups()
    if (mode.value === 'all' || mode.value === 'diversion') {
      await loadDiversionRules()
    }
  } catch (error) {
    setError(`删除专属分流组失败: ${error.message}`)
  }
}

function openCreateAdguard() {
  clearMessage()
  adguardRaw.value = null
  adguardEditor.open = true
  adguardEditor.id = ''
  adguardEditor.name = ''
  adguardEditor.url = ''
  adguardEditor.enabled = true
  adguardEditor.auto_update = true
  adguardEditor.update_interval_hours = 24
}

function openEditAdguard(rule) {
  clearMessage()
  adguardRaw.value = { ...rule }
  adguardEditor.open = true
  adguardEditor.id = String(rule.id || '')
  adguardEditor.name = String(rule.name || '')
  adguardEditor.url = String(rule.url || '')
  adguardEditor.enabled = Boolean(rule.enabled)
  adguardEditor.auto_update = Boolean(rule.auto_update)
  adguardEditor.update_interval_hours = Number(rule.update_interval_hours || 24)
}

function closeAdguardEditor() {
  adguardEditor.open = false
}

function adguardPayload() {
  const name = adguardEditor.name.trim()
  const url = adguardEditor.url.trim()
  if (!name || !url) {
    throw new Error('名称和 URL 不能为空')
  }
  return {
    name,
    url,
    enabled: Boolean(adguardEditor.enabled),
    auto_update: Boolean(adguardEditor.auto_update),
    update_interval_hours: Number(adguardEditor.update_interval_hours || 24)
  }
}

async function saveAdguard() {
  try {
    const payload = adguardPayload()
    if (adguardEditor.id) {
      const merged = {
        ...(adguardRaw.value || {}),
        ...payload
      }
      await putJSON(`/plugins/adguard/rules/${adguardEditor.id}`, merged)
      setSuccess('AdGuard 规则已更新')
    } else {
      await postJSON('/plugins/adguard/rules', payload)
      setSuccess('AdGuard 规则已新增')
    }
    closeAdguardEditor()
    await loadAdguardRules()
  } catch (error) {
    setError(`保存 AdGuard 规则失败: ${error.message}`)
  }
}

async function toggleAdguard(rule) {
  try {
    const payload = {
      ...rule,
      enabled: !Boolean(rule.enabled)
    }
    await putJSON(`/plugins/adguard/rules/${rule.id}`, payload)
    await loadAdguardRules()
  } catch (error) {
    setError(`切换 AdGuard 规则失败: ${error.message}`)
  }
}

async function deleteAdguard(rule) {
  if (!(await openConfirm(`确定删除 AdGuard 规则“${rule.name}”吗？`, { tone: 'danger' }))) {
    return
  }
  try {
    await deleteRequest(`/plugins/adguard/rules/${rule.id}`)
    setSuccess('AdGuard 规则已删除')
    await loadAdguardRules()
  } catch (error) {
    setError(`删除 AdGuard 规则失败: ${error.message}`)
  }
}

async function updateAdguardAll() {
  try {
    await postJSON('/plugins/adguard/update', {})
    setSuccess('已触发 AdGuard 全量更新，5 秒后自动刷新列表')
    setTimeout(() => {
      loadAdguardRules()
    }, 5000)
  } catch (error) {
    setError(`触发 AdGuard 更新失败: ${error.message}`)
  }
}

async function updateAdguard(rule) {
  const id = String(rule?.id || '')
  if (!id) {
    setError('无法定位 AdGuard 规则 ID')
    return
  }
  try {
    await postJSON(`/plugins/adguard/update/${id}`, {})
    setSuccess(`已触发拦截规则“${rule.name}”更新，5 秒后自动刷新列表`)
    setTimeout(() => {
      loadAdguardRules()
    }, 5000)
  } catch (error) {
    setError(`触发拦截规则更新失败: ${error.message}`)
  }
}

function openCreateDiversion() {
  clearMessage()
  diversionRaw.value = null
  diversionEditor.open = true
  diversionEditor.oldName = ''
  diversionEditor.oldType = ''
  diversionEditor.name = ''
  diversionEditor.type = diversionTypeOptions.value[0]?.value || 'geositecn'
  diversionEditor.files = ''
  diversionEditor.url = ''
  diversionEditor.enabled = true
  diversionEditor.auto_update = true
  diversionEditor.enable_regexp = false
  diversionEditor.update_interval_hours = 24
  diversionAutofill.isApplying = false
  diversionAutofill.nameDirty = false
  diversionAutofill.filesDirty = false
  syncDiversionAutofill()
}

function openEditDiversion(rule) {
  clearMessage()
  diversionRaw.value = { ...rule }
  diversionEditor.open = true
  diversionEditor.oldName = String(rule.name || '')
  diversionEditor.oldType = String(rule.type || '')
  diversionEditor.name = String(rule.name || '')
  diversionEditor.type = String(rule.type || diversionTypeOptions.value[0]?.value || 'geositecn')
  diversionEditor.files = String(rule.files || '')
  diversionEditor.url = String(rule.url || '')
  diversionEditor.enabled = Boolean(rule.enabled)
  diversionEditor.auto_update = Boolean(rule.auto_update)
  diversionEditor.enable_regexp = Boolean(rule.enable_regexp)
  diversionEditor.update_interval_hours = Number(rule.update_interval_hours || 24)
  diversionAutofill.isApplying = false
  diversionAutofill.nameDirty = false
  diversionAutofill.filesDirty = false
}

function closeDiversionEditor() {
  diversionEditor.open = false
}

function inferDiversionFromUrl(url) {
  const raw = String(url || '').trim()
  if (!raw) {
    return null
  }
  let fileName = ''
  try {
    const parsed = new URL(raw)
    fileName = (parsed.pathname || '').split('/').pop() || ''
  } catch {
    fileName = raw.split('#')[0].split('?')[0].split('/').pop() || ''
  }
  if (!fileName) {
    return null
  }
  try {
    fileName = decodeURIComponent(fileName)
  } catch {
    // ignore decode error and use raw filename
  }
  fileName = fileName.trim()
  if (!fileName) {
    return null
  }
  const baseName = fileName.replace(/\.[^.]+$/, '') || fileName
  return {
    name: baseName,
    filePath: `srs/${fileName}`
  }
}

function syncDiversionAutofill({ force = false } = {}) {
  if (!diversionEditor.open || diversionEditor.oldName) {
    return false
  }
  const inferred = inferDiversionFromUrl(diversionEditor.url)
  if (!inferred) {
    return false
  }
  diversionAutofill.isApplying = true
  try {
    if (force || !diversionAutofill.nameDirty || !diversionEditor.name.trim()) {
      diversionEditor.name = inferred.name
      if (force) {
        diversionAutofill.nameDirty = false
      }
    }
    if (force || !diversionAutofill.filesDirty || !diversionEditor.files.trim()) {
      diversionEditor.files = inferred.filePath
      if (force) {
        diversionAutofill.filesDirty = false
      }
    }
  } finally {
    diversionAutofill.isApplying = false
  }
  return true
}

function onDiversionNameInput() {
  if (!diversionAutofill.isApplying) {
    diversionAutofill.nameDirty = true
  }
}

function onDiversionFilesInput() {
  if (!diversionAutofill.isApplying) {
    diversionAutofill.filesDirty = true
  }
}

function onDiversionUrlInput() {
  syncDiversionAutofill()
}

function onDiversionTypeChange() {
  syncDiversionAutofill()
}

function applyDiversionAutofill() {
  clearMessage()
  const applied = syncDiversionAutofill({ force: true })
  if (!applied) {
    setError('无法从当前 URL 识别名称和本地文件路径')
    return
  }
  setSuccess('已按当前 URL 自动识别')
}

function diversionPayload() {
  const name = diversionEditor.name.trim()
  const type = diversionEditor.type.trim()
  const files = diversionEditor.files.trim()
  const url = diversionEditor.url.trim()
  if (!name) {
    throw new Error('规则名称不能为空')
  }
  if (!type) {
    throw new Error('规则类型不能为空')
  }
  if (!files || !url) {
    throw new Error('本地文件路径和 URL 都不能为空')
  }
  return {
    name,
    type,
    files,
    url,
    enabled: Boolean(diversionEditor.enabled),
    auto_update: Boolean(diversionEditor.auto_update),
    enable_regexp: Boolean(diversionEditor.enable_regexp),
    update_interval_hours: Number(diversionEditor.update_interval_hours || 24)
  }
}

function wait(ms) {
  return new Promise((resolve) => {
    setTimeout(resolve, ms)
  })
}

async function waitForDiversionRuleReady(ruleName, attempts = 6, intervalMs = 3000) {
  for (let i = 0; i < attempts; i += 1) {
    await loadDiversionRules()
    const rule = diversionRules.value.find((item) => item.name === ruleName)
    if (rule) {
      const hasCount = Number(rule.rule_count || 0) > 0
      const hasUpdated = rule.last_updated && !String(rule.last_updated).startsWith('0001-01-01')
      if (hasCount || hasUpdated) {
        return true
      }
    }
    if (i < attempts - 1) {
      await wait(intervalMs)
    }
  }
  return false
}

async function saveDiversion() {
  try {
    const payload = diversionPayload()
    const currentMap = diversionPluginMap.value
    const newPluginTag = currentMap[payload.type]
    if (!newPluginTag) {
      throw new Error(`无效的分流类型: ${payload.type}`)
    }

    const merged = {
      ...sanitizeRulePayload(diversionRaw.value || {}),
      ...payload
    }

    let shouldAutoUpdate = false
    if (diversionEditor.oldName) {
      const oldPluginTag = currentMap[diversionEditor.oldType] || diversionRaw.value?.__pluginTag
      if (!oldPluginTag) {
        throw new Error('无法定位旧规则插件')
      }
      const nameChanged = diversionEditor.oldName !== payload.name
      const typeChanged = diversionEditor.oldType !== payload.type
      if (nameChanged || typeChanged) {
        await deleteRequest(`/plugins/${oldPluginTag}/config/${diversionEditor.oldName}`)
        shouldAutoUpdate = Boolean(payload.url)
      }
      const endpointName = nameChanged || typeChanged ? payload.name : diversionEditor.oldName
      await putJSON(`/plugins/${newPluginTag}/config/${endpointName}`, merged)
      setSuccess('分流规则已更新')
    } else {
      await putJSON(`/plugins/${newPluginTag}/config/${payload.name}`, merged)
      shouldAutoUpdate = Boolean(payload.url)
      setSuccess('分流规则已新增')
    }

    closeDiversionEditor()
    await loadDiversionRules()

    if (shouldAutoUpdate) {
      setSuccess(`正在后台自动下载规则“${payload.name}”...`)
      try {
        await postJSON(`/plugins/${newPluginTag}/update/${payload.name}`, {})
        const ready = await waitForDiversionRuleReady(payload.name)
        if (!ready) {
          setSuccess('规则已开始下载，详情可能稍后刷新。')
        } else {
          setSuccess('规则已自动下载并刷新。')
        }
      } catch (updateError) {
        setError(`触发自动下载失败: ${updateError.message}`)
      }
    }
  } catch (error) {
    setError(`保存分流规则失败: ${error.message}`)
  }
}

async function toggleDiversion(rule) {
  const pluginTag = diversionPluginMap.value[rule.type] || rule.__pluginTag
  if (!pluginTag) {
    setError('无法定位分流规则插件')
    return
  }
  try {
    const payload = {
      ...sanitizeRulePayload(rule),
      enabled: !Boolean(rule.enabled)
    }
    await putJSON(`/plugins/${pluginTag}/config/${rule.name}`, payload)
    await loadDiversionRules()
  } catch (error) {
    setError(`切换分流规则失败: ${error.message}`)
  }
}

async function deleteDiversion(rule) {
  if (!(await openConfirm(`确定删除分流规则“${rule.name}”吗？`, { tone: 'danger' }))) {
    return
  }
  const pluginTag = diversionPluginMap.value[rule.type] || rule.__pluginTag
  if (!pluginTag) {
    setError('无法定位分流规则插件')
    return
  }
  try {
    await deleteRequest(`/plugins/${pluginTag}/config/${rule.name}`)
    setSuccess('分流规则已删除')
    await loadDiversionRules()
  } catch (error) {
    setError(`删除分流规则失败: ${error.message}`)
  }
}

async function updateDiversion(rule) {
  const pluginTag = diversionPluginMap.value[rule.type] || rule.__pluginTag
  if (!pluginTag) {
    setError('无法定位分流规则插件')
    return
  }
  try {
    await postJSON(`/plugins/${pluginTag}/update/${rule.name}`, {})
    setSuccess(`已触发规则“${rule.name}”更新，5 秒后自动刷新列表`)
    setTimeout(() => {
      loadDiversionRules()
    }, 5000)
  } catch (error) {
    setError(`触发分流规则更新失败: ${error.message}`)
  }
}

async function updateDiversionAll() {
  const targetsMap = new Map()
  diversionRules.value.forEach((rule) => {
    const pluginTag = diversionPluginMap.value[rule.type] || rule.__pluginTag
    if (!pluginTag || !rule?.name) {
      return
    }
    const key = `${pluginTag}:${rule.name}`
    targetsMap.set(key, { pluginTag, name: rule.name })
  })

  const targets = Array.from(targetsMap.values())
  if (targets.length === 0) {
    setError('暂无可更新的分流规则')
    return
  }

  try {
    const settled = await Promise.allSettled(
      targets.map((target) => postJSON(`/plugins/${target.pluginTag}/update/${target.name}`, {}))
    )
    const okCount = settled.filter((item) => item.status === 'fulfilled').length
    const failCount = settled.length - okCount
    if (okCount === 0) {
      setError(`触发分流全量更新失败，共 ${failCount} 条`)
      return
    }
    if (failCount > 0) {
      setError(`已触发 ${okCount} 条分流规则更新，${failCount} 条触发失败`)
    } else {
      setSuccess(`已触发全部 ${okCount} 条分流规则更新，5 秒后自动刷新列表`)
    }
    setTimeout(() => {
      loadDiversionRules()
    }, 5000)
  } catch (error) {
    setError(`触发分流全量更新失败: ${error.message}`)
  }
}

function handleGlobalRefresh() {
  reloadCurrentView()
}

watch(() => specialEditor.listenPort, (value) => {
  if (!String(value || '').trim()) {
    specialEditor.customPortOnly = false
  }
})

onMounted(() => {
  reloadCurrentView()
  window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
})
</script>

<template>
  <section class="panel">
    <nav v-if="showInnerTabs" class="tab-bar inner">
      <button class="tab-btn" :class="{ active: activeTab === 'special' }" @click="activeTab = 'special'">专属分流组</button>
      <button class="tab-btn" :class="{ active: activeTab === 'adguard' }" @click="activeTab = 'adguard'">
        AdGuard<span v-if="adguardRules.length > 0" class="mini-badge">{{ adguardRules.length }}</span>
      </button>
      <button class="tab-btn" :class="{ active: activeTab === 'diversion' }" @click="activeTab = 'diversion'">
        在线分流<span v-if="diversionRules.length > 0" class="mini-badge">{{ diversionRules.length }}</span>
      </button>
    </nav>

    <RulesSpecialGroupsPanel
      v-if="shouldShowTab('special')"
      :loading="loading.special"
      :special-groups="specialGroups"
      @create="openCreateSpecial"
      @edit="openEditSpecial"
      @delete="deleteSpecial"
    />

    <RulesAdguardPanel
      v-if="shouldShowTab('adguard')"
      :loading="loading.adguard"
      :adguard-rules="adguardRules"
      :format-time="formatTime"
      @create="openCreateAdguard"
      @update-all="updateAdguardAll"
      @toggle="toggleAdguard"
      @update="updateAdguard"
      @edit="openEditAdguard"
      @delete="deleteAdguard"
    />

    <RulesDiversionPanel
      v-if="shouldShowTab('diversion')"
      :loading="loading.diversion"
      :diversion-rules="diversionRules"
      :format-time="formatTime"
      @create="openCreateDiversion"
      @update-all="updateDiversionAll"
      @toggle="toggleDiversion"
      @update="updateDiversion"
      @edit="openEditDiversion"
      @delete="deleteDiversion"
    />

    <RulesSpecialEditorModal
      v-if="specialEditor.open"
      :editor="specialEditor"
      @close="closeSpecialEditor"
      @save="saveSpecial"
    />

    <RulesAdguardEditorModal
      v-if="adguardEditor.open"
      :editor="adguardEditor"
      @close="closeAdguardEditor"
      @save="saveAdguard"
    />

    <RulesDiversionEditorModal
      v-if="diversionEditor.open"
      :editor="diversionEditor"
      :diversion-type-options="diversionTypeOptions"
      :is-editing="Boolean(diversionEditor.oldName)"
      @close="closeDiversionEditor"
      @save="saveDiversion"
      @apply-autofill="applyDiversionAutofill"
      @type-change="onDiversionTypeChange"
      @url-input="onDiversionUrlInput"
      @name-input="onDiversionNameInput"
      @files-input="onDiversionFilesInput"
    />
  </section>
</template>

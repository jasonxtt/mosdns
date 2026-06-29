<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getText, postJSON } from '../src/api/http'
import UpstreamManager from '../src/components/UpstreamManager.vue'
import { openConfirm } from '../src/utils/confirm'
import SystemAuditCapacityPanel from '../src/components/system/SystemAuditCapacityPanel.vue'
import SystemAppearancePanel from '../src/components/system/SystemAppearancePanel.vue'
import SystemConfigManagePanel from '../src/components/system/SystemConfigManagePanel.vue'
import SystemDomainGenerationPanel from '../src/components/system/SystemDomainGenerationPanel.vue'
import SystemOverridesPanel from '../src/components/system/SystemOverridesPanel.vue'
import SystemReplacementRulesPanel from '../src/components/system/SystemReplacementRulesPanel.vue'
import SystemUpdatePanel from '../src/components/system/SystemUpdatePanel.vue'
import SystemWebuiPortPanel from '../src/components/system/SystemWebuiPortPanel.vue'
import { clearTopNotice, setError, setSuccess } from '../src/utils/notice'
import { formatRelativeTime } from '../src/utils/time'
import {
  getDefaultPanelBackgroundSettings,
  normalizePanelBackgroundSettings,
  previewPanelBackground,
  transparencyToOpacity,
} from '../src/utils/panelBackground'
import {
  applyTextColorForTheme,
  getDefaultTextColorSettings,
  getEffectiveTextColor,
  loadTextColorSettingsFromStorage,
  normalizeTextColorSettings,
  normalizeUserHexColor,
  saveTextColorSettingsToStorage,
} from '../src/utils/appearanceTextColor'
import {
  applyButtonColorForTheme,
  getDefaultButtonColorSettings,
  getEffectiveButtonColor,
  loadButtonColorSettingsFromStorage,
  normalizeButtonColorSettings,
  normalizeUserHexButtonColor,
  saveButtonColorSettingsToStorage,
} from '../src/utils/appearanceButtonColor'

const props = defineProps({
  mode: {
    type: String,
    default: 'system-upstream',
  },
})

const audit = reactive({
  capturing: null,
  capacity: null,
  newCapacity: '',
})

const update = reactive({
  loading: false,
  status: null,
})

const applyingOverrides = ref(false)
const configManaging = reactive({
  localDir: '',
  remoteUrl: '',
  backingUp: false,
  updating: false,
})

const webuiPort = reactive({
  loading: false,
  saving: false,
  input: '',
  activePort: 0,
  activeAddr: '',
})

const overrides = reactive({
  socks5: '',
  ecs: '',
  replacements: [],
})

const switchLoading = reactive({})
const switchStates = reactive({})
const domainGenerationLoading = ref(false)
const domainGenerationSettings = reactive({
  enabled: true,
  remember_direct: true,
  remember_proxy: true,
  no_v4: true,
  no_v6: true,
})

const autoRefresh = reactive({
  enabled: false,
  intervalSeconds: 15,
})

const expandedAdvanced = reactive({
  replacementRules: false,
  behaviorAdvanced: false,
})

const appearance = reactive({
  theme: 'dark',
})

const defaultTextColorSettings = getDefaultTextColorSettings()
const textColorSettings = reactive(getDefaultTextColorSettings())
const textColorDraft = ref(defaultTextColorSettings.light.color)
const textColorSaving = ref(false)
const defaultButtonColorSettings = getDefaultButtonColorSettings()
const buttonColorSettings = reactive(getDefaultButtonColorSettings())
const buttonColorDraft = ref(defaultButtonColorSettings.light.color)
const buttonColorSaving = ref(false)
const eyeDropperSupported = ref(false)

const panelBackgroundDefaults = getDefaultPanelBackgroundSettings()
const panelBackgroundMaxUpload = 20 * 1024 * 1024
const panelBackground = reactive({
  mode: panelBackgroundDefaults.mode,
  url: '',
  lightColor: panelBackgroundDefaults.lightColor,
  darkColor: panelBackgroundDefaults.darkColor,
  imageUrl: '',
  uploadId: '',
  transparency: panelBackgroundDefaults.transparency,
  opacity: panelBackgroundDefaults.opacity,
  blur: panelBackgroundDefaults.blur,
  applying: false,
  uploading: false,
})
const panelBackgroundHistory = ref([])
const panelBackgroundHistoryOpen = ref(false)
const panelBackgroundHistoryLoading = ref(false)
const panelBackgroundHistoryBusy = ref('')

let restartProbeTimerId = 0
let textColorSaveTimerId = 0
let textColorSaveQueued = false
let buttonColorSaveTimerId = 0
let buttonColorSaveQueued = false

const themeOptions = [
  { value: 'light', label: '明亮' },
  { value: 'dark', label: '黑暗' },
]

const switchProfiles = [
  {
    tag: 'switch3',
    name: '核心运行模式',
    tip: '切换后会按新模式重建分流相关缓存和数据。',
    modes: { A: { name: '兼容模式' }, B: { name: '安全模式' } },
  },
  {
    tag: 'switch17',
    name: 'DNS 分流模式',
    tip: '切换后会清空相关缓存并重建分流数据。',
    modes: { A: { name: 'FakeIP 分流' }, B: { name: 'RealIP 分流' } },
  },
  {
    tag: 'switch1',
    name: '请求屏蔽',
    desc: '对无解析结果的请求进行屏蔽',
    tip: '建议开启。',
    valueForOn: 'A',
  },
  {
    tag: 'switch5',
    name: '类型屏蔽',
    desc: '屏蔽 SOA / PTR / HTTPS 等请求',
    tip: '建议开启。',
    valueForOn: 'A',
  },
  {
    tag: 'switch4',
    name: '过期缓存1',
    desc: '启用多组过期缓存',
    tip: '建议开启。',
    valueForOn: 'A',
  },
  {
    tag: 'switch13',
    name: '过期缓存2',
    desc: '启用全量缓存与 fakeip 缓存',
    tip: '建议开启。',
    valueForOn: 'A',
  },
  {
    tag: 'switch7',
    name: '广告屏蔽',
    desc: '启用 AdGuard 在线规则支持',
    tip: '按需开启。',
    valueForOn: 'A',
  },
  {
    tag: 'switch9',
    name: 'CNFakeIP',
    desc: '国内域名返回 fakeip',
    tip: '切换后会自动清空核心缓存。',
    valueForOn: 'B',
  },
  {
    tag: 'switch16',
    name: '国外解析DDNS',
    desc: '优先使用国外上游解析 DDNS 域名',
    tip: '按需开启。开启后无缓存，国外失败回退国内。',
    valueForOn: 'A',
  },
  {
    tag: 'switch2',
    name: '指定 Client fakeip',
    desc: '仅指定客户端可科学',
    tip: '需配合 client_ip 名单。',
    valueForOn: 'A',
  },
  {
    tag: 'switch12',
    name: '指定 Client realip',
    desc: '指定客户端不科学',
    tip: '需配合 client_ip 名单。',
    valueForOn: 'A',
  },
  {
    tag: 'switch8',
    name: 'IPv4优先',
    desc: '优先采信A记录',
    tip: '适合希望优先走 IPv4，但仍保留纯 IPv6 域名可解析的场景。',
    valueForOn: 'A',
  },
  {
    tag: 'switch6',
    name: 'IPV6屏蔽',
    desc: '屏蔽AAAA记录',
    tip: '无 IPV6 场景建议开启。',
    valueForOn: 'A',
  },
]

const switchGroupDefinitions = [
  {
    key: 'blocking',
    title: '解析屏蔽',
    tags: ['switch7', 'switch1', 'switch5'],
  },
  {
    key: 'cache',
    title: '缓存设置',
    tags: ['switch4', 'switch13'],
  },
  {
    key: 'behavior',
    title: '行为开关',
    tags: ['switch9', 'switch16', 'switch2', 'switch12'],
  },
]

const domainGenerationProfiles = [
  { key: 'enabled', name: '总开关', desc: '统一控制域名表生成' },
  { key: 'remember_direct', name: '记忆直连', desc: '生成直连域名表' },
  { key: 'remember_proxy', name: '记忆代理', desc: '生成代理域名表' },
  { key: 'no_v4', name: '记忆无v4', desc: '生成无 IPv4 域名表' },
  { key: 'no_v6', name: '记忆无v6', desc: '生成无 IPv6 域名表' },
]

const coreMode = computed(() => String(switchStates.switch3 || ''))
const dnsRoutingMode = computed(() => String(switchStates.switch17 || ''))
const currentMode = computed(() => String(props.mode || 'system-upstream'))

const switchGroups = computed(() =>
  switchGroupDefinitions.map((group) => ({
    ...group,
    profiles: group.tags
      .map((tag) => switchProfiles.find((profile) => profile.tag === tag))
      .filter(Boolean),
  })),
)

const hasUpdate = computed(() => {
  const status = update.status
  if (!status) {
    return false
  }
  if (status.pending_restart) {
    return false
  }
  const cur = normalizeVersion(status.current_version)
  const latest = normalizeVersion(status.latest_version)
  if (cur && latest && cur === latest) {
    return false
  }
  return Boolean(status.update_available && status.download_url)
})

const showV3Callout = computed(() => {
  const status = update.status
  if (!status) {
    return false
  }
  const arch = String(status.architecture || '')
  return (
    (arch === 'linux/amd64' || arch === 'windows/amd64') &&
    Boolean(status.amd64_v3_capable) &&
    !Boolean(status.current_is_v3)
  )
})

const updateBannerText = computed(() => {
  const status = update.status
  if (!status) {
    return '等待检查'
  }
  if (status.pending_restart) {
    const isWindows = String(status.architecture || '').startsWith('windows/')
    return isWindows ? '更新已安装，等待手动重启。' : '更新已安装，正在自重启…'
  }
  if (status.message) {
    return String(status.message)
  }
  return hasUpdate.value ? '发现新版本，可立即更新。' : '当前已是最新版本'
})

const updateLastCheckedText = computed(() => {
  const checkedAt = update.status?.checked_at
  if (!checkedAt) {
    return '--'
  }
  const date = new Date(checkedAt)
  if (!Number.isFinite(date.getTime())) {
    return String(checkedAt)
  }
  return date.toLocaleString('zh-CN', { hour12: false })
})

const updateLatestBadge = computed(() => {
  const status = update.status
  if (!status) {
    return false
  }
  const cur = normalizeVersion(status.current_version)
  const latest = normalizeVersion(status.latest_version)
  return Boolean(cur && latest && cur === latest)
})

const configVersionDisplayMap = {
  1: 'v1',
  2: 'v2',
}

const configVersionInfo = computed(() => {
  const applied = Number(update.status?.config_schema_applied || 0)
  const required = Number(update.status?.config_schema_required || 0)
  if (!applied && !required) {
    return {
      versionText: '--',
      statusText: '',
    }
  }
  return {
    versionText: formatConfigVersionDisplay(applied || required || 0),
    statusText: applied >= required ? '已是最新' : '需要更新',
  }
})

const ipStrategyMode = computed(() => {
  const preferV4Profile = findSwitchProfile('switch8')
  const blockV6Profile = findSwitchProfile('switch6')
  if (blockV6Profile && isSwitchChecked(blockV6Profile)) {
    return 'block-v6'
  }
  if (preferV4Profile && isSwitchChecked(preferV4Profile)) {
    return 'prefer-v4'
  }
  return 'auto'
})

function clearMessage() {
  clearTopNotice()
}

function toggleAdvancedSection(key) {
  if (!(key in expandedAdvanced)) {
    return
  }
  expandedAdvanced[key] = !expandedAdvanced[key]
}

function normalizeVersion(value) {
  if (!value) {
    return ''
  }
  return String(value).trim().toLowerCase().replace(/^v/, '')
}

function formatConfigVersionDisplay(schema) {
  const value = Number(schema || 0)
  if (!value) {
    return '--'
  }
  return configVersionDisplayMap[value] || `v${value}`
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

function isHttpConflictError(error) {
  return /(^|\b)409(\b|$)|conflict/i.test(String(error?.message || error || ''))
}

async function readSwitchValue(tag) {
  const value = String(await getText(`/plugins/${tag}/show`) || '').trim()
  switchStates[tag] = value
  return value
}

async function loadAuditStatusAndCapacity() {
  const [statusRes, capacityRes] = await Promise.all([
    getJSON('/api/v1/audit/status'),
    getJSON('/api/v1/audit/capacity'),
  ])
  audit.capturing = Boolean(statusRes?.capturing)
  audit.capacity = Number(capacityRes?.capacity || 0)
}

async function toggleAuditCapture() {
  clearMessage()
  try {
    if (audit.capturing) {
      await postEmpty('/api/v1/audit/stop')
      setSuccess('审计已停止')
    } else {
      await postEmpty('/api/v1/audit/start')
      setSuccess('审计已启动')
    }
    await loadAuditStatusAndCapacity()
  } catch (error) {
    setError(`切换审计状态失败: ${error.message}`)
  }
}

async function clearAuditLogs() {
  if (
    !(await openConfirm('将删除当前所有内存审计日志，此操作不可恢复。', {
      tone: 'danger',
    }))
  ) {
    return
  }
  clearMessage()
  try {
    await postEmpty('/api/v1/audit/clear')
    setSuccess('日志已清空')
    await loadAuditStatusAndCapacity()
  } catch (error) {
    setError(`清空日志失败: ${error.message}`)
  }
}

async function submitCapacity() {
  const capacity = Number(audit.newCapacity || 0)
  if (!Number.isFinite(capacity) || capacity <= 0 || capacity > 400000) {
    setError('请输入 1 到 400000 之间的有效热日志上限')
    return
  }
  if (
    !(await openConfirm(
      `将详细日志热数据上限设置为 ${capacity.toLocaleString()}，并清空当前所有日志。`,
      { tone: 'danger' },
    ))
  ) {
    return
  }
  clearMessage()
  try {
    await postJSON('/api/v1/audit/capacity', { capacity })
    audit.newCapacity = ''
    setSuccess(`详细日志热数据上限已设置为 ${capacity.toLocaleString()}`)
    await loadAuditStatusAndCapacity()
  } catch (error) {
    setError(`设置详细日志热数据上限失败: ${error.message}`)
  }
}

async function loadFeatureSwitches() {
  const settled = await Promise.allSettled(
    switchProfiles.map((profile) => getText(`/plugins/${profile.tag}/show`)),
  )
  settled.forEach((item, index) => {
    const profile = switchProfiles[index]
    if (item.status === 'fulfilled') {
      switchStates[profile.tag] = String(item.value || '').trim()
      return
    }
    switchStates[profile.tag] = 'error'
  })
}

async function setSwitchValue(tag, value, successHint = '') {
  switchLoading[tag] = true
  try {
    await postJSON(`/plugins/${tag}/post`, { value })
    switchStates[tag] = value
    if (successHint) {
      setSuccess(successHint)
    }
  } finally {
    switchLoading[tag] = false
  }
}

function findSwitchProfile(tag) {
  return switchProfiles.find((profile) => profile.tag === tag) || null
}

function getSwitchOffValue(profile) {
  if (!profile?.valueForOn) {
    return 'B'
  }
  return profile.valueForOn === 'A' ? 'B' : 'A'
}

function isSwitchChecked(profile) {
  if (!profile?.tag) {
    return false
  }
  return switchStates[profile.tag] === profile.valueForOn
}

function getMutuallyExclusiveProfile(tag) {
  if (tag === 'switch6') {
    return findSwitchProfile('switch8')
  }
  if (tag === 'switch8') {
    return findSwitchProfile('switch6')
  }
  return null
}

async function setCoreMode(modeValue) {
  if (!['A', 'B'].includes(String(modeValue)) || coreMode.value === modeValue) {
    return
  }
  const fromName = coreMode.value === 'B' ? '安全模式' : '兼容模式'
  const toName = modeValue === 'B' ? '安全模式' : '兼容模式'
  if (!(await openConfirm(`确认从“${fromName}”切换到“${toName}”？`))) {
    return
  }
  clearMessage()
  try {
    await setSwitchValue('switch3', modeValue, '核心模式已切换')
    try {
      await postEmpty('/plugins/requery/trigger')
    } catch {
      // ignore
    }
  } catch (error) {
    setError(`切换核心模式失败: ${error.message}`)
  }
}

async function setDnsRoutingMode(modeValue) {
  if (
    switchLoading.switch17 ||
    !['A', 'B'].includes(String(modeValue)) ||
    dnsRoutingMode.value === modeValue
  ) {
    return
  }

  const fromName = dnsRoutingMode.value === 'B' ? 'RealIP 分流' : 'FakeIP 分流'
  const toName = modeValue === 'B' ? 'RealIP 分流' : 'FakeIP 分流'
  const confirmText =
    modeValue === 'B'
      ? `确认从“${fromName}”切换到“${toName}”？切换后代理域名会直接返回国外真实 IP，并清空相关缓存后重建分流数据。`
      : `确认从“${fromName}”切换到“${toName}”？切换后代理域名会恢复返回 FakeIP，并清空相关缓存后重建分流数据。`
  if (!(await openConfirm(confirmText))) {
    return
  }

  clearMessage()
  switchLoading.switch17 = true
  try {
    try {
      await postJSON('/plugins/switch17/post', { value: modeValue })
      switchStates.switch17 = modeValue
    } catch (error) {
      const applied = isHttpConflictError(error)
        ? (await readSwitchValue('switch17')) === modeValue
        : false
      if (!applied) {
        throw error
      }
    }

    const flushResults = await Promise.allSettled([
      requestResponse('/plugins/cache_all/flush'),
      requestResponse('/plugins/cache_all_noleak/flush'),
      requestResponse('/plugins/cache_google/flush'),
      requestResponse('/plugins/cache_google_node/flush'),
      requestResponse('/plugins/cache_cnmihomo/flush'),
    ])
    const requeryResults = await Promise.allSettled([
      postEmpty('/plugins/requery/trigger'),
    ])
    const backgroundErrors = [...flushResults, ...requeryResults]
      .filter((item) => item.status === 'rejected')
      .map((item) => item.reason)
    if (!backgroundErrors.length) {
      setSuccess('DNS 分流模式已切换')
    } else if (backgroundErrors.every(isHttpConflictError)) {
      setSuccess('DNS 分流模式已切换，后台缓存/重查任务正在处理中')
    } else {
      setSuccess('DNS 分流模式已切换；部分后台重建任务未完成，可稍后刷新重试')
    }
  } catch (error) {
    setError(`切换 DNS 分流模式失败: ${error.message}`)
    await loadFeatureSwitches()
  } finally {
    switchLoading.switch17 = false
  }
}

async function toggleSecondarySwitch(profile, checked) {
  if (!profile?.tag || !profile.valueForOn) {
    return
  }
  clearMessage()
  const next = checked ? profile.valueForOn : getSwitchOffValue(profile)
  try {
    let autoDisabledProfile = null
    if (checked) {
      const conflictProfile = getMutuallyExclusiveProfile(profile.tag)
      if (conflictProfile && isSwitchChecked(conflictProfile)) {
        await setSwitchValue(
          conflictProfile.tag,
          getSwitchOffValue(conflictProfile),
        )
        autoDisabledProfile = conflictProfile
      }
    }
    await setSwitchValue(profile.tag, next)
    if (profile.tag === 'switch9') {
      await Promise.allSettled([
        requestResponse('/plugins/cache_all/flush'),
        requestResponse('/plugins/cache_all_noleak/flush'),
      ])
    }
    if (autoDisabledProfile) {
      setSuccess(
        `已自动关闭“${autoDisabledProfile.name}”，因为它与“${profile.name}”互斥`,
      )
      return
    }
    setSuccess(`“${profile.name}” 已${checked ? '启用' : '禁用'}`)
  } catch (error) {
    setError(`切换“${profile.name}”失败: ${error.message}`)
    await loadFeatureSwitches()
  }
}

async function setIpStrategy(mode) {
  const preferV4Profile = findSwitchProfile('switch8')
  const blockV6Profile = findSwitchProfile('switch6')
  if (!preferV4Profile || !blockV6Profile) {
    return
  }
  if (mode === ipStrategyMode.value) {
    return
  }
  clearMessage()
  try {
    if (mode === 'auto') {
      if (isSwitchChecked(preferV4Profile)) {
        await setSwitchValue(
          preferV4Profile.tag,
          getSwitchOffValue(preferV4Profile),
        )
      }
      if (isSwitchChecked(blockV6Profile)) {
        await setSwitchValue(blockV6Profile.tag, getSwitchOffValue(blockV6Profile))
      }
      setSuccess('IP 策略已切换为自动')
      return
    }

    if (mode === 'prefer-v4') {
      if (isSwitchChecked(blockV6Profile)) {
        await setSwitchValue(blockV6Profile.tag, getSwitchOffValue(blockV6Profile))
      }
      await setSwitchValue(preferV4Profile.tag, preferV4Profile.valueForOn)
      setSuccess('IP 策略已切换为 IPv4 优先')
      return
    }

    if (mode === 'block-v6') {
      if (isSwitchChecked(preferV4Profile)) {
        await setSwitchValue(
          preferV4Profile.tag,
          getSwitchOffValue(preferV4Profile),
        )
      }
      await setSwitchValue(blockV6Profile.tag, blockV6Profile.valueForOn)
      setSuccess('IP 策略已切换为屏蔽 IPv6')
    }
  } catch (error) {
    setError(`切换 IP 策略失败: ${error.message}`)
    await loadFeatureSwitches()
  }
}

function applyDomainGenerationSettings(next) {
  domainGenerationSettings.enabled = Boolean(next?.enabled)
  domainGenerationSettings.remember_direct = Boolean(next?.remember_direct)
  domainGenerationSettings.remember_proxy = Boolean(next?.remember_proxy)
  domainGenerationSettings.no_v4 = Boolean(next?.no_v4)
  domainGenerationSettings.no_v6 = Boolean(next?.no_v6)
}

function buildDomainGenerationPayload(key, checked) {
  if (key === 'enabled') {
    return {
      enabled: checked,
      remember_direct: checked,
      remember_proxy: checked,
      no_v4: checked,
      no_v6: checked,
    }
  }

  const payload = { [key]: checked }
  if (checked && !domainGenerationSettings.enabled) {
    payload.enabled = true
  }
  return payload
}

async function toggleDomainGeneration(profile, checked) {
  if (!profile?.key) {
    return
  }
  clearMessage()
  try {
    if (!checked) {
      const confirmed = await openConfirm(
        `关闭“${profile.name}”后，会停止新增并清空当前已生成数据，确认继续？`,
        {
          tone: 'danger',
          confirmText: '关闭并清空',
        },
      )
      if (!confirmed) {
        return
      }
    }
    domainGenerationLoading.value = true
    const payload = buildDomainGenerationPayload(profile.key, checked)
    const response = await postJSON('/api/v1/domain-generation', payload)
    applyDomainGenerationSettings(response?.settings || response)
    setSuccess(`“${profile.name}” 已${checked ? '启用' : '关闭'}`)
  } catch (error) {
    setError(`切换“${profile.name}”失败: ${error.message}`)
  } finally {
    domainGenerationLoading.value = false
  }
}

async function loadDomainGenerationSettings() {
  const payload = await getJSON('/api/v1/domain-generation')
  applyDomainGenerationSettings(payload)
}

async function loadOverrides() {
  const data = await getJSON('/api/v1/overrides')
  overrides.socks5 = String(data?.socks5 || '')
  overrides.ecs = String(data?.ecs || '')
  const source = Array.isArray(data?.replacements) ? data.replacements : []
  overrides.replacements = source.map((item) => ({
    original: String(item?.original || ''),
    new: String(item?.new || ''),
    comment: String(item?.comment || ''),
    result: String(item?.result || ''),
  }))
}

function addReplacement() {
  overrides.replacements.push({
    original: '',
    new: '',
    comment: '',
    result: '',
  })
}

function removeReplacement(index) {
  overrides.replacements.splice(index, 1)
}

async function saveOverrides() {
  clearMessage()
  applyingOverrides.value = true
  try {
    const replacements = overrides.replacements
      .map((rule) => ({
        original: String(rule.original || '').trim(),
        new: String(rule.new || '').trim(),
        comment: String(rule.comment || '').trim(),
      }))
      .filter((rule) => rule.original)

    await postJSON('/api/v1/overrides', {
      socks5: String(overrides.socks5 || '').trim(),
      ecs: String(overrides.ecs || '').trim(),
      replacements,
    })

    if (await openConfirm('覆盖配置已保存，是否立即重启 MosDNS 使其生效？')) {
      await postJSON('/api/v1/system/restart', { delay_ms: 500 })
      setSuccess('已发送重启请求，页面将自动刷新。')
      setTimeout(() => window.location.reload(), 4000)
      return
    }

    setSuccess('覆盖配置已保存')
    await loadOverrides()
  } catch (error) {
    setError(`保存覆盖配置失败: ${error.message}`)
  } finally {
    applyingOverrides.value = false
  }
}

function buildWebUIRootUrl(port) {
  try {
    const url = new URL(window.location.href)
    url.port = String(port)
    url.pathname = '/log'
    url.search = ''
    url.hash = ''
    return url.toString()
  } catch {
    return `${window.location.protocol}//${window.location.hostname}:${port}/log`
  }
}

async function loadWebUIPortSettings() {
  webuiPort.loading = true
  try {
    const payload = await getJSON('/api/v1/system/webui-port')
    const port = Number(payload?.port || 0)
    webuiPort.activePort = Number(payload?.active_port || 0)
    webuiPort.activeAddr = String(payload?.active_addr || '')
    webuiPort.input = port > 0 ? String(port) : ''
  } finally {
    webuiPort.loading = false
  }
}

async function applyWebUIPortAndRestart() {
  const port = Number.parseInt(String(webuiPort.input || '').trim(), 10)
  if (!Number.isFinite(port) || port < 1 || port > 65535) {
    setError('请输入 1-65535 之间的端口')
    return
  }
  if (
    !(await openConfirm(`将 WebUI 端口改为 ${port} 并重启 MosDNS，确认继续？`, {
      tone: 'danger',
    }))
  ) {
    return
  }

  clearMessage()
  webuiPort.saving = true
  try {
    await postJSON('/api/v1/system/webui-port', { port })
    await postJSON('/api/v1/system/restart', { delay_ms: 800 })
    const target = buildWebUIRootUrl(port)
    setSuccess(`端口已更新为 ${port}，MosDNS 正在重启，稍后将跳转到新地址。`)
    setTimeout(() => {
      window.location.href = target
    }, 4500)
  } catch (error) {
    setError(`设置 WebUI 端口失败: ${error.message}`)
    await loadWebUIPortSettings()
  } finally {
    webuiPort.saving = false
  }
}

async function loadUpdateStatus() {
  const status = await getJSON('/api/v1/update/status')
  update.status = status
  if (status?.config_auto_updated > 0) {
    setSuccess(`配置已自动更新（${status.config_auto_updated} 个文件）`)
  } else if (status?.config_update_status === 'failed') {
    setError(
      `配置自动升级失败: ${status.config_update_error || status.config_update_message || '未知错误'}`,
    )
  }
}

async function checkUpdate() {
  clearMessage()
  update.loading = true
  try {
    const status = await postJSON('/api/v1/update/check', {})
    update.status = status
    setSuccess('已刷新最新版本信息')
  } catch (error) {
    setError(`检查更新失败: ${error.message}`)
  } finally {
    update.loading = false
  }
}

function startRestartWatch() {
  stopRestartWatch()
  const deadline = Date.now() + 90_000
  restartProbeTimerId = window.setInterval(async () => {
    if (Date.now() > deadline) {
      stopRestartWatch()
      return
    }
    try {
      const status = await getJSON('/api/v1/update/status')
      update.status = status
      if (!status?.pending_restart) {
        stopRestartWatch()
        setSuccess('重启完成')
        setTimeout(() => window.location.reload(), 800)
      }
    } catch {
      // ignore polling errors
    }
  }, 1000)
}

function stopRestartWatch() {
  if (restartProbeTimerId) {
    window.clearInterval(restartProbeTimerId)
    restartProbeTimerId = 0
  }
}

async function applyUpdate(force = false, preferV3 = false) {
  if (update.loading) {
    return
  }
  if (!force && !hasUpdate.value) {
    return
  }
  clearMessage()
  update.loading = true
  try {
    const response = await postJSON('/api/v1/update/apply', {
      force,
      prefer_v3: preferV3,
    })
    const status = response?.status || response
    update.status = status
    setSuccess(String(response?.notes || status?.message || '更新请求已提交'))
    if (
      status?.pending_restart &&
      !String(status?.architecture || '').startsWith('windows/')
    ) {
      startRestartWatch()
    }
  } catch (error) {
    setError(`执行更新失败: ${error.message}`)
  } finally {
    update.loading = false
  }
}

function applyTheme(theme, save = true) {
  const nextTheme = ['light', 'dark'].includes(String(theme))
    ? String(theme)
    : 'light'
  appearance.theme = nextTheme
  document.documentElement.setAttribute('data-theme', nextTheme)
  applyTextColorForTheme(nextTheme, textColorSettings)
  applyButtonColorForTheme(nextTheme, buttonColorSettings)
  syncTextColorDraft(nextTheme)
  syncButtonColorDraft(nextTheme)
  void syncPanelBackgroundPreview(false)
  if (save) {
    localStorage.setItem('mosdns-theme', nextTheme)
  }
}

function overwriteTextColorSettings(nextSettings) {
  const normalized = normalizeTextColorSettings(nextSettings || {})
  textColorSettings.light.mode = normalized.light.mode
  textColorSettings.light.color = normalized.light.color
  textColorSettings.dark.mode = normalized.dark.mode
  textColorSettings.dark.color = normalized.dark.color
}

function activeThemeKey() {
  return appearance.theme === 'dark' ? 'dark' : 'light'
}

function syncTextColorDraft(theme = activeThemeKey()) {
  const effective = getEffectiveTextColor(theme, textColorSettings)
  textColorDraft.value = effective
}

function overwriteButtonColorSettings(nextSettings) {
  const normalized = normalizeButtonColorSettings(nextSettings || {})
  buttonColorSettings.light.mode = normalized.light.mode
  buttonColorSettings.light.color = normalized.light.color
  buttonColorSettings.dark.mode = normalized.dark.mode
  buttonColorSettings.dark.color = normalized.dark.color
}

function syncButtonColorDraft(theme = activeThemeKey()) {
  const effective = getEffectiveButtonColor(theme, buttonColorSettings)
  buttonColorDraft.value = effective
}

function setCustomTextColorForActiveTheme(rawColor) {
  const theme = activeThemeKey()
  const fallback = defaultTextColorSettings[theme].color
  const normalized = normalizeUserHexColor(rawColor, fallback)
  textColorDraft.value = normalized
  textColorSettings[theme].mode = 'custom'
  textColorSettings[theme].color = normalized
  applyTextColorForTheme(theme, textColorSettings)
}

function initializeAppearance() {
  applyTheme(localStorage.getItem('mosdns-theme') || 'light', false)
  const cached = loadTextColorSettingsFromStorage()
  const cachedButtonColors = loadButtonColorSettingsFromStorage()
  overwriteTextColorSettings(cached)
  overwriteButtonColorSettings(cachedButtonColors)
  applyTextColorForTheme(activeThemeKey(), textColorSettings)
  applyButtonColorForTheme(activeThemeKey(), buttonColorSettings)
  syncTextColorDraft(activeThemeKey())
  syncButtonColorDraft(activeThemeKey())
}

function textColorPayload() {
  return normalizeTextColorSettings(textColorSettings)
}

async function loadTextColorSettings() {
  try {
    const settings = await getJSON('/api/v1/appearance/text-color')
    const normalized = normalizeTextColorSettings(settings || {})
    overwriteTextColorSettings(normalized)
    saveTextColorSettingsToStorage(normalized)
  } catch {
    // fallback to cached settings without interrupting UI
  }
  applyTextColorForTheme(activeThemeKey(), textColorSettings)
  syncTextColorDraft(activeThemeKey())
}

function buttonColorPayload() {
  return normalizeButtonColorSettings(buttonColorSettings)
}

async function loadButtonColorSettings() {
  try {
    const settings = await getJSON('/api/v1/appearance/button-color')
    const normalized = normalizeButtonColorSettings(settings || {})
    overwriteButtonColorSettings(normalized)
    saveButtonColorSettingsToStorage(normalized)
  } catch {
    // fallback to cached settings without interrupting UI
  }
  applyButtonColorForTheme(activeThemeKey(), buttonColorSettings)
  syncButtonColorDraft(activeThemeKey())
}

async function saveTextColorSettings(showMessage = true) {
  if (textColorSaving.value) {
    textColorSaveQueued = true
    return
  }
  textColorSaving.value = true
  try {
    const saved = await postJSON(
      '/api/v1/appearance/text-color',
      textColorPayload(),
    )
    const normalized = normalizeTextColorSettings(saved || {})
    overwriteTextColorSettings(normalized)
    saveTextColorSettingsToStorage(normalized)
    applyTextColorForTheme(activeThemeKey(), textColorSettings)
    syncTextColorDraft(activeThemeKey())
    if (showMessage) {
      setSuccess('字体颜色已保存')
    }
  } catch (error) {
    setError(`保存字体颜色失败: ${error.message}`)
  } finally {
    textColorSaving.value = false
    if (textColorSaveQueued) {
      textColorSaveQueued = false
      void saveTextColorSettings(false)
    }
  }
}

async function saveButtonColorSettings(showMessage = true) {
  if (buttonColorSaving.value) {
    buttonColorSaveQueued = true
    return
  }
  buttonColorSaving.value = true
  try {
    const saved = await postJSON(
      '/api/v1/appearance/button-color',
      buttonColorPayload(),
    )
    const normalized = normalizeButtonColorSettings(saved || {})
    overwriteButtonColorSettings(normalized)
    saveButtonColorSettingsToStorage(normalized)
    applyButtonColorForTheme(activeThemeKey(), buttonColorSettings)
    syncButtonColorDraft(activeThemeKey())
    if (showMessage) {
      setSuccess('按钮颜色已保存')
    }
  } catch (error) {
    setError(`保存按钮颜色失败: ${error.message}`)
  } finally {
    buttonColorSaving.value = false
    if (buttonColorSaveQueued) {
      buttonColorSaveQueued = false
      void saveButtonColorSettings(false)
    }
  }
}

function queueTextColorSave() {
  if (textColorSaveTimerId) {
    window.clearTimeout(textColorSaveTimerId)
    textColorSaveTimerId = 0
  }
  textColorSaveTimerId = window.setTimeout(() => {
    textColorSaveTimerId = 0
    void saveTextColorSettings(false)
  }, 180)
}

function queueButtonColorSave() {
  if (buttonColorSaveTimerId) {
    window.clearTimeout(buttonColorSaveTimerId)
    buttonColorSaveTimerId = 0
  }
  buttonColorSaveTimerId = window.setTimeout(() => {
    buttonColorSaveTimerId = 0
    void saveButtonColorSettings(false)
  }, 180)
}

function onTextColorPickerInput(event) {
  setCustomTextColorForActiveTheme(
    event?.target?.value || textColorDraft.value,
  )
  queueTextColorSave()
}

async function onTextColorPickerChange(event) {
  setCustomTextColorForActiveTheme(
    event?.target?.value || textColorDraft.value,
  )
  await saveTextColorSettings(false)
}

async function pickTextColorFromScreen() {
  if (!eyeDropperSupported.value) {
    return
  }
  try {
    const dropper = new window.EyeDropper()
    const result = await dropper.open()
    if (!result?.sRGBHex) {
      return
    }
    setCustomTextColorForActiveTheme(result.sRGBHex)
    await saveTextColorSettings(false)
  } catch {
    // user cancelled or unsupported runtime state
  }
}

function setCustomButtonColorForActiveTheme(rawColor) {
  const theme = activeThemeKey()
  const fallback = defaultButtonColorSettings[theme].color
  const normalized = normalizeUserHexButtonColor(rawColor, fallback)
  buttonColorDraft.value = normalized
  buttonColorSettings[theme].mode = 'custom'
  buttonColorSettings[theme].color = normalized
  applyButtonColorForTheme(theme, buttonColorSettings)
}

function onButtonColorPickerInput(event) {
  setCustomButtonColorForActiveTheme(
    event?.target?.value || buttonColorDraft.value,
  )
  queueButtonColorSave()
}

async function onButtonColorPickerChange(event) {
  setCustomButtonColorForActiveTheme(
    event?.target?.value || buttonColorDraft.value,
  )
  await saveButtonColorSettings(false)
}

async function pickButtonColorFromScreen() {
  if (!eyeDropperSupported.value) {
    return
  }
  try {
    const dropper = new window.EyeDropper()
    const result = await dropper.open()
    if (!result?.sRGBHex) {
      return
    }
    setCustomButtonColorForActiveTheme(result.sRGBHex)
    await saveButtonColorSettings(false)
  } catch {
    // user cancelled or unsupported runtime state
  }
}

async function resetThemeTextColor() {
  const theme = activeThemeKey()
  textColorSettings[theme].mode = 'default'
  textColorSettings[theme].color = defaultTextColorSettings[theme].color
  textColorDraft.value = defaultTextColorSettings[theme].color
  applyTextColorForTheme(theme, textColorSettings)
  await saveTextColorSettings()
}

async function resetThemeButtonColor() {
  const theme = activeThemeKey()
  buttonColorSettings[theme].mode = 'default'
  buttonColorSettings[theme].color = defaultButtonColorSettings[theme].color
  buttonColorDraft.value = defaultButtonColorSettings[theme].color
  applyButtonColorForTheme(theme, buttonColorSettings)
  await saveButtonColorSettings()
}

function applyPanelBackgroundDraft(raw) {
  const normalized = normalizePanelBackgroundSettings({
    ...(raw || {}),
    theme_key: activeThemeKey(),
  })
  panelBackground.mode = normalized.mode
  panelBackground.url = normalized.url
  panelBackground.lightColor = normalized.lightColor
  panelBackground.darkColor = normalized.darkColor
  panelBackground.imageUrl = normalized.imageUrl
  panelBackground.uploadId = normalized.uploadId
  panelBackground.transparency = normalized.transparency
  panelBackground.opacity = normalized.opacity
  panelBackground.blur = normalized.blur
}

function getPanelBackgroundDraftForPreview() {
  return {
    mode: panelBackground.mode,
    url: panelBackground.url,
    color:
      activeThemeKey() === 'dark'
        ? panelBackground.darkColor
        : panelBackground.lightColor,
    light_color: panelBackground.lightColor,
    dark_color: panelBackground.darkColor,
    theme_key: activeThemeKey(),
    image_url: panelBackground.imageUrl,
    upload_id: panelBackground.uploadId,
    opacity: transparencyToOpacity(panelBackground.transparency),
    blur: panelBackground.blur,
  }
}

async function syncPanelBackgroundPreview(showError = false) {
  const result = await previewPanelBackground(
    getPanelBackgroundDraftForPreview(),
    {
      onError: (error) => {
        if (showError) {
          setError(`背景加载失败，已回退默认背景: ${error.message}`)
        }
      },
    },
  )
  return result.ok
}

function buildPanelBackgroundPayload() {
  const normalized = normalizePanelBackgroundSettings(
    getPanelBackgroundDraftForPreview(),
  )
  return {
    mode: normalized.mode,
    url: normalized.mode === 'url' ? normalized.url : '',
    color: normalized.mode === 'color' ? normalized.color : '',
    light_color: normalized.lightColor,
    dark_color: normalized.darkColor,
    upload_id: normalized.mode === 'upload' ? normalized.uploadId : '',
    opacity: normalized.opacity,
    blur: normalized.blur,
  }
}

async function loadPanelBackgroundSettings() {
  if (panelBackground.applying || panelBackground.uploading) {
    return
  }
  try {
    const settings = await getJSON('/api/v1/appearance/panel-background')
    applyPanelBackgroundDraft(settings)
    await syncPanelBackgroundPreview(false)
  } catch (error) {
    setError(`加载面板背景设置失败: ${error.message}`)
  }
}

async function loadPanelBackgroundHistory() {
  panelBackgroundHistoryLoading.value = true
  try {
    const payload = await getJSON(
      '/api/v1/appearance/panel-background/history',
    )
    const items = Array.isArray(payload?.items)
      ? payload.items
      : Array.isArray(payload)
        ? payload
        : []
    panelBackgroundHistory.value = items
  } catch (error) {
    setError(`加载背景历史失败: ${error.message}`)
  } finally {
    panelBackgroundHistoryLoading.value = false
  }
}

async function togglePanelBackgroundHistory() {
  panelBackgroundHistoryOpen.value = !panelBackgroundHistoryOpen.value
  if (panelBackgroundHistoryOpen.value) {
    await loadPanelBackgroundHistory()
  }
}

async function usePanelBackgroundHistory(item) {
  const uploadId = String(item?.id || '').trim()
  const imageUrl = String(item?.image_url || '').trim()
  if (!uploadId || !imageUrl) {
    return
  }
  panelBackground.mode = 'upload'
  panelBackground.uploadId = uploadId
  panelBackground.imageUrl = imageUrl
  if (!(await syncPanelBackgroundPreview(true))) {
    return
  }
  await applyPanelBackgroundSettings()
}

async function deletePanelBackgroundHistory(item) {
  const uploadId = String(item?.id || '').trim()
  if (!uploadId) {
    return
  }
  if (
    !(await openConfirm('确认删除这张历史背景图片吗？', { tone: 'danger' }))
  ) {
    return
  }
  panelBackgroundHistoryBusy.value = uploadId
  try {
    const response = await fetch(
      `/api/v1/appearance/panel-background/history/${encodeURIComponent(uploadId)}`,
      {
        method: 'DELETE',
      },
    )
    if (!response.ok) {
      const text = await response.text().catch(() => '')
      throw new Error(text || `HTTP ${response.status} ${response.statusText}`)
    }
    await Promise.all([
      loadPanelBackgroundHistory(),
      loadPanelBackgroundSettings(),
    ])
    setSuccess('历史背景已删除')
  } catch (error) {
    setError(`删除历史背景失败: ${error.message}`)
  } finally {
    panelBackgroundHistoryBusy.value = ''
  }
}

async function clearPanelBackgroundHistory() {
  if (
    !(await openConfirm('确认清空所有历史背景图片吗？', { tone: 'danger' }))
  ) {
    return
  }
  panelBackgroundHistoryBusy.value = 'clear-all'
  try {
    const response = await fetch(
      '/api/v1/appearance/panel-background/history',
      {
        method: 'DELETE',
      },
    )
    if (!response.ok) {
      const text = await response.text().catch(() => '')
      throw new Error(text || `HTTP ${response.status} ${response.statusText}`)
    }
    await Promise.all([
      loadPanelBackgroundHistory(),
      loadPanelBackgroundSettings(),
    ])
    await syncPanelBackgroundPreview(false)
    setSuccess('历史背景已清空')
  } catch (error) {
    setError(`清空历史背景失败: ${error.message}`)
  } finally {
    panelBackgroundHistoryBusy.value = ''
  }
}

async function applyPanelBackgroundSettings() {
  clearMessage()
  panelBackground.applying = true
  try {
    if (panelBackground.mode !== 'upload') {
      if (panelBackground.url.trim()) {
        panelBackground.mode = 'url'
      } else if (panelBackground.mode !== 'color') {
        panelBackground.mode = 'none'
      }
    }
    const payload = buildPanelBackgroundPayload()
    if (
      (payload.mode === 'url' || payload.mode === 'upload') &&
      !(await syncPanelBackgroundPreview(true))
    ) {
      return
    }
    const saved = await postJSON(
      '/api/v1/appearance/panel-background',
      payload,
    )
    applyPanelBackgroundDraft(saved)
    await syncPanelBackgroundPreview(false)
  } catch (error) {
    setError(`应用面板背景失败: ${error.message}`)
  } finally {
    panelBackground.applying = false
  }
}

async function resetAppearanceSettings() {
  const confirmed = await openConfirm(
    '所有主题相关设置将重置为初始值：主题改为明亮、背景清空、透明度 100%、毛玻璃 0px、字体和按钮颜色恢复默认。确认继续吗？',
    { tone: 'danger' },
  )
  if (!confirmed) {
    return
  }

  clearMessage()

  overwriteTextColorSettings(defaultTextColorSettings)
  saveTextColorSettingsToStorage(defaultTextColorSettings)
  overwriteButtonColorSettings(defaultButtonColorSettings)
  saveButtonColorSettingsToStorage(defaultButtonColorSettings)
  applyTheme('light')
  applyTextColorForTheme('light', textColorSettings)
  applyButtonColorForTheme('light', buttonColorSettings)
  syncTextColorDraft('light')
  syncButtonColorDraft('light')

  panelBackground.mode = 'none'
  panelBackground.url = ''
  panelBackground.lightColor = panelBackgroundDefaults.lightColor
  panelBackground.darkColor = panelBackgroundDefaults.darkColor
  panelBackground.imageUrl = ''
  panelBackground.uploadId = ''
  panelBackground.transparency = 100
  panelBackground.opacity = 1
  panelBackground.blur = 0
  panelBackgroundHistoryOpen.value = false
  await syncPanelBackgroundPreview(false)

  try {
    await Promise.all([
      postJSON(
        '/api/v1/appearance/text-color',
        normalizeTextColorSettings(defaultTextColorSettings),
      ),
      postJSON(
        '/api/v1/appearance/button-color',
        normalizeButtonColorSettings(defaultButtonColorSettings),
      ),
      postJSON('/api/v1/appearance/panel-background', {
        mode: 'none',
        url: '',
        opacity: 1,
        blur: 0,
      }),
    ])
    setSuccess('主题与外观已重置为初始值')
  } catch (error) {
    setError(`重置主题与外观失败: ${error.message}`)
  }
}

async function onPanelBackgroundUrlEnter() {
  panelBackground.mode = 'url'
  await applyPanelBackgroundSettings()
}

function onPanelBackgroundColorInput(event) {
  const next = event?.target?.value || ''
  if (activeThemeKey() === 'dark') {
    panelBackground.darkColor = next
  } else {
    panelBackground.lightColor = next
  }
  panelBackground.mode = 'color'
  panelBackground.url = ''
  void syncPanelBackgroundPreview(false)
}

function onPanelBackgroundSliderInput() {
  panelBackground.opacity = transparencyToOpacity(panelBackground.transparency)
  void syncPanelBackgroundPreview(false)
}

async function onPanelBackgroundFileChange(event) {
  const input = event?.target
  const file = input?.files?.[0]
  if (input) {
    input.value = ''
  }
  if (!file) {
    return
  }
  if (Number(file.size || 0) > panelBackgroundMaxUpload) {
    setError('图片大小不能超过 20MB')
    return
  }

  clearMessage()
  panelBackground.uploading = true
  try {
    const formData = new FormData()
    formData.append('file', file)
    const response = await fetch('/api/v1/appearance/panel-background/upload', {
      method: 'POST',
      body: formData,
    })
    if (!response.ok) {
      const text = await response.text().catch(() => '')
      throw new Error(text || `HTTP ${response.status} ${response.statusText}`)
    }
    const data = await response.json()
    panelBackground.mode = 'upload'
    panelBackground.uploadId = String(data?.upload_id || '')
    panelBackground.imageUrl = String(data?.image_url || '')
    if (!(await syncPanelBackgroundPreview(true))) {
      return
    }
    await Promise.all([
      applyPanelBackgroundSettings(),
      loadPanelBackgroundHistory(),
    ])
  } catch (error) {
    setError(`上传背景图片失败: ${error.message}`)
  } finally {
    panelBackground.uploading = false
  }
}

function loadAutoRefreshSettings() {
  let saved = null
  try {
    saved = JSON.parse(localStorage.getItem('mosdnsAutoRefresh') || 'null')
  } catch {
    saved = null
  }
  autoRefresh.enabled = Boolean(saved?.enabled)
  autoRefresh.intervalSeconds = Math.max(
    5,
    Number(saved?.intervalSeconds || 15),
  )
}

function loadConfigManagerSettings() {
  configManaging.localDir = localStorage.getItem('mosdns-config-dir') || ''
  configManaging.remoteUrl = localStorage.getItem('mosdns-config-url') || ''
}

function saveConfigManagerSettings() {
  localStorage.setItem(
    'mosdns-config-dir',
    String(configManaging.localDir || '').trim(),
  )
  localStorage.setItem(
    'mosdns-config-url',
    String(configManaging.remoteUrl || '').trim(),
  )
}

async function backupConfig() {
  const dir = String(configManaging.localDir || '').trim()
  if (!dir) {
    setError('请先输入 MosDNS 本地工作目录')
    return
  }
  clearMessage()
  saveConfigManagerSettings()
  configManaging.backingUp = true
  try {
    const response = await fetch('/api/v1/config/export', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ dir }),
    })
    if (!response.ok) {
      const text = await response.text().catch(() => '')
      throw new Error(text || `HTTP ${response.status} ${response.statusText}`)
    }
    const blob = await response.blob()
    const downloadUrl = window.URL.createObjectURL(blob)
    const link = document.createElement('a')
    const disposition =
      response.headers.get('Content-Disposition') ||
      response.headers.get('content-disposition') ||
      ''
    let filename = 'mosdns_backup.zip'
    const matches = /filename[^;=\n]*=((['"]).*?\2|[^;\n]*)/.exec(disposition)
    if (matches?.[1]) {
      filename = matches[1].replace(/['"]/g, '')
    }
    link.style.display = 'none'
    link.href = downloadUrl
    link.download = filename
    document.body.appendChild(link)
    link.click()
    document.body.removeChild(link)
    window.URL.revokeObjectURL(downloadUrl)
    setSuccess('备份文件下载已开始')
  } catch (error) {
    setError(`备份失败: ${error.message}`)
  } finally {
    configManaging.backingUp = false
  }
}

async function applyRemoteConfig() {
  const dir = String(configManaging.localDir || '').trim()
  const url = String(configManaging.remoteUrl || '').trim()
  if (!dir || !url) {
    setError('请完整填写本地目录和远程 URL')
    return
  }
  if (
    !(await openConfirm(
      '当前配置会先备份到 backup 子目录，新配置将覆盖现有文件，并在完成后自动重启 MosDNS。',
      { tone: 'danger' },
    ))
  ) {
    return
  }
  clearMessage()
  saveConfigManagerSettings()
  configManaging.updating = true
  try {
    const response = await postJSON('/api/v1/config/update_from_url', {
      url,
      dir,
    })
    setSuccess(String(response?.message || '更新成功，4秒后自动刷新...'))
    setTimeout(() => {
      window.location.reload()
    }, 4000)
  } catch (error) {
    setError(`远程更新失败: ${error.message}`)
  } finally {
    configManaging.updating = false
  }
}

function emitAutoRefreshSettings(showToast = false) {
  autoRefresh.intervalSeconds = Math.max(
    5,
    Number(autoRefresh.intervalSeconds || 15),
  )
  const payload = {
    enabled: Boolean(autoRefresh.enabled),
    intervalSeconds: autoRefresh.intervalSeconds,
  }
  localStorage.setItem('mosdnsAutoRefresh', JSON.stringify(payload))
  window.dispatchEvent(
    new CustomEvent('mosdns-auto-refresh-update', { detail: payload }),
  )
  if (showToast) {
    setSuccess(
      `自动刷新已${payload.enabled ? `开启（${payload.intervalSeconds}秒）` : '关闭'}`,
    )
  }
}

function onAutoRefreshToggle(event) {
  autoRefresh.enabled = Boolean(event?.target?.checked)
  emitAutoRefreshSettings(true)
}

function onAutoRefreshIntervalChange() {
  emitAutoRefreshSettings(true)
}

async function reloadAll() {
  clearMessage()
  try {
    await Promise.all([
      loadAuditStatusAndCapacity(),
      loadFeatureSwitches(),
      loadDomainGenerationSettings(),
      loadOverrides(),
      loadUpdateStatus(),
      loadWebUIPortSettings(),
    ])
  } catch (error) {
    setError(`加载系统设置失败: ${error.message}`)
  }
}

async function refreshOnGlobalEvent() {
  await reloadAll()
}

onMounted(() => {
  eyeDropperSupported.value =
    typeof window !== 'undefined' && 'EyeDropper' in window
  initializeAppearance()
  loadTextColorSettings()
  loadButtonColorSettings()
  loadAutoRefreshSettings()
  loadConfigManagerSettings()
  emitAutoRefreshSettings(false)
  loadPanelBackgroundSettings()
  loadPanelBackgroundHistory()
  reloadAll()
  window.addEventListener('mosdns-log-refresh', refreshOnGlobalEvent)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', refreshOnGlobalEvent)
  stopRestartWatch()
  if (textColorSaveTimerId) {
    window.clearTimeout(textColorSaveTimerId)
    textColorSaveTimerId = 0
  }
  if (buttonColorSaveTimerId) {
    window.clearTimeout(buttonColorSaveTimerId)
    buttonColorSaveTimerId = 0
  }
})
</script>

<template>
  <section class="system-panel log1-system-page">
    <section v-if="currentMode === 'system-upstream'" class="log1-system-mode">
      <UpstreamManager mode="upstream-settings" />
    </section>

    <section v-else-if="currentMode === 'system-maintenance'" class="log1-system-mode">
      <div class="control-panel-grid log1-system-grid-four">
        <SystemUpdatePanel
          :has-update="hasUpdate"
          :show-v3-callout="showV3Callout"
          :update="update"
          :update-banner-text="updateBannerText"
          :update-last-checked-text="updateLastCheckedText"
          :update-latest-badge="updateLatestBadge"
          @check-update="checkUpdate"
          @apply-update="applyUpdate(false, false)"
          @apply-force-update="applyUpdate(true, false)"
          @apply-v3-update="applyUpdate(true, true)"
        />

        <SystemConfigManagePanel
          :config-managing="configManaging"
          :config-version="configVersionInfo"
          @save-settings="saveConfigManagerSettings"
          @backup-config="backupConfig"
          @apply-remote-config="applyRemoteConfig"
        />

        <SystemOverridesPanel
          :applying-overrides="applyingOverrides"
          :overrides="overrides"
          @load-overrides="loadOverrides"
          @save-overrides="saveOverrides"
        />

        <SystemWebuiPortPanel
          :webui-port="webuiPort"
          @apply-port="applyWebUIPortAndRestart"
        />
      </div>

      <div class="log1-module-toggle-row log1-module-toggle-row-center">
        <button
          class="log1-module-toggle log1-module-toggle-compact"
          :class="{ active: expandedAdvanced.replacementRules }"
          type="button"
          @click="toggleAdvancedSection('replacementRules')"
        >
          <span>{{ expandedAdvanced.replacementRules ? '收起高级设置' : '高级设置' }}</span>
          <span class="log1-module-toggle-arrow" aria-hidden="true">{{ expandedAdvanced.replacementRules ? '▴' : '▾' }}</span>
        </button>
      </div>

      <SystemReplacementRulesPanel
        v-if="expandedAdvanced.replacementRules"
        :applying-overrides="applyingOverrides"
        :overrides="overrides"
        @load-overrides="loadOverrides"
        @add-replacement="addReplacement"
        @save-overrides="saveOverrides"
        @remove-replacement="removeReplacement"
      />
    </section>

    <section v-else-if="currentMode === 'system-behavior'" class="log1-system-mode">
      <div class="control-panel-grid log1-system-grid-three">
        <section class="panel control-module log1-switch-group-card">
          <header class="module-head">
            <div>
              <h3>核心运行模式</h3>
            </div>
          </header>
          <div class="log1-core-mode-list">
            <button
              class="log1-ip-option"
              :class="{ active: coreMode === 'A' }"
              :disabled="switchLoading.switch3"
              @click="setCoreMode('A')"
            >
              <strong>兼容模式</strong>
              <span>表外域名国内解析，保证速度</span>
            </button>
            <button
              class="log1-ip-option"
              :class="{ active: coreMode === 'B' }"
              :disabled="switchLoading.switch3"
              @click="setCoreMode('B')"
            >
              <strong>安全模式</strong>
              <span>表外域名国外解析，阻止泄漏</span>
            </button>
          </div>
        </section>

        <section class="panel control-module log1-switch-group-card">
          <header class="module-head">
            <div>
              <h3>DNS 分流模式</h3>
            </div>
          </header>
          <div class="log1-dns-routing-list">
            <button
              class="log1-ip-option"
              :class="{ active: dnsRoutingMode === 'A' }"
              :disabled="switchLoading.switch17"
              @click="setDnsRoutingMode('A')"
            >
              <strong>FakeIP</strong>
              <span>国外域名返回 FakeIP</span>
            </button>
            <button
              class="log1-ip-option"
              :class="{ active: dnsRoutingMode === 'B' }"
              :disabled="switchLoading.switch17"
              @click="setDnsRoutingMode('B')"
            >
              <strong>Redir-Host</strong>
              <span>国外域名返回真实 IP</span>
            </button>
          </div>
        </section>

        <section class="panel control-module log1-switch-group-card">
          <header class="module-head">
            <div>
              <h3>解析策略</h3>
            </div>
          </header>
          <div class="log1-ip-strategy-list">
            <button
              class="log1-ip-option"
              :class="{ active: ipStrategyMode === 'auto' }"
              @click="setIpStrategy('auto')"
            >
              <strong>自动</strong>
              <span>默认解析行为</span>
            </button>
            <button
              class="log1-ip-option"
              :class="{ active: ipStrategyMode === 'prefer-v4' }"
              @click="setIpStrategy('prefer-v4')"
            >
              <strong>IPv4 优先</strong>
              <span>优先采信A记录</span>
            </button>
            <button
              class="log1-ip-option"
              :class="{ active: ipStrategyMode === 'block-v6' }"
              @click="setIpStrategy('block-v6')"
            >
              <strong>IPv6 屏蔽</strong>
              <span>屏蔽AAAA记录</span>
            </button>
          </div>
        </section>
      </div>

      <div class="log1-module-toggle-row log1-module-toggle-row-center">
        <button
          class="log1-module-toggle log1-module-toggle-compact"
          :class="{ active: expandedAdvanced.behaviorAdvanced }"
          type="button"
          @click="toggleAdvancedSection('behaviorAdvanced')"
        >
          <span>{{ expandedAdvanced.behaviorAdvanced ? '收起高级设置' : '高级设置' }}</span>
          <span class="log1-module-toggle-arrow" aria-hidden="true">{{ expandedAdvanced.behaviorAdvanced ? '▴' : '▾' }}</span>
        </button>
      </div>

      <div v-if="expandedAdvanced.behaviorAdvanced" class="log1-advanced-grid">
        <section
          v-for="group in switchGroups"
          :key="`${group.key}-panel`"
          class="panel control-module log1-advanced-module-panel"
        >
          <header class="module-head">
            <div>
              <h3>{{ group.title }}</h3>
              <p v-if="group.desc" class="muted">{{ group.desc }}</p>
            </div>
          </header>
          <div class="log1-switch-grid">
            <label
              v-for="profile in group.profiles"
              :key="profile.tag"
              class="switch-row"
            >
              <div class="switch-meta">
                <strong>{{ profile.name }}</strong>
                <span class="muted">{{ profile.desc }}</span>
              </div>
              <span class="switch switch-compact">
                <input
                  type="checkbox"
                  :checked="isSwitchChecked(profile)"
                  :disabled="switchLoading[profile.tag] || switchStates[profile.tag] === 'error'"
                  @change="toggleSecondarySwitch(profile, $event.target.checked)"
                />
                <span class="slider"></span>
              </span>
            </label>
          </div>
        </section>

        <SystemDomainGenerationPanel
          class="log1-advanced-module-panel"
          :domain-generation-profiles="domainGenerationProfiles"
          :domain-generation-loading="domainGenerationLoading"
          :domain-generation-settings="domainGenerationSettings"
          @toggle-domain-generation="toggleDomainGeneration"
        />
      </div>
    </section>

    <section v-else-if="currentMode === 'system-preferences'" class="log1-system-mode log1-preferences-page">
      <SystemAppearancePanel
        :appearance="appearance"
        :button-color-draft="buttonColorDraft"
        :button-color-saving="buttonColorSaving"
        :eye-dropper-supported="eyeDropperSupported"
        :format-relative-time="formatRelativeTime"
        :panel-background="panelBackground"
        :panel-background-history="panelBackgroundHistory"
        :panel-background-history-busy="panelBackgroundHistoryBusy"
        :panel-background-history-loading="panelBackgroundHistoryLoading"
        :panel-background-history-open="panelBackgroundHistoryOpen"
        :text-color-draft="textColorDraft"
        :text-color-saving="textColorSaving"
        :theme-options="themeOptions"
        @apply-theme="applyTheme"
        @text-color-input="onTextColorPickerInput"
        @text-color-change="onTextColorPickerChange"
        @pick-text-color="pickTextColorFromScreen"
        @reset-text-color="resetThemeTextColor"
        @button-color-input="onButtonColorPickerInput"
        @button-color-change="onButtonColorPickerChange"
        @pick-button-color="pickButtonColorFromScreen"
        @reset-button-color="resetThemeButtonColor"
        @panel-bg-color-input="onPanelBackgroundColorInput"
        @panel-bg-url-enter="onPanelBackgroundUrlEnter"
        @panel-bg-file-change="onPanelBackgroundFileChange"
        @toggle-panel-bg-history="togglePanelBackgroundHistory"
        @clear-panel-bg-history="clearPanelBackgroundHistory"
        @use-panel-bg-history="usePanelBackgroundHistory"
        @delete-panel-bg-history="deletePanelBackgroundHistory"
        @panel-bg-slider-input="onPanelBackgroundSliderInput"
        @apply-panel-bg-settings="applyPanelBackgroundSettings"
        @reset-appearance-settings="resetAppearanceSettings"
      />
    </section>

    <section v-else class="log1-system-mode">
      <div class="log1-logs-grid">
        <section class="panel control-module control-module--mini log1-logs-card log1-audit-card">
          <div class="log1-refresh-head">
            <h3>审计控制</h3>
            <label class="switch switch-table">
              <input
                type="checkbox"
                :checked="Boolean(audit.capturing)"
                :disabled="audit.capturing === null"
                @change="toggleAuditCapture"
              />
              <span class="slider"></span>
            </label>
          </div>
          <p class="muted">关闭后将停止新增查询审计记录，已有热数据不会立即删除。</p>
          <div class="actions log1-audit-actions">
            <button class="btn tiny secondary" type="button" @click="clearAuditLogs">清空日志</button>
          </div>
        </section>

        <section class="panel control-module control-module--mini log1-logs-card log1-refresh-panel">
          <div class="log1-refresh-head">
            <h3>自动刷新</h3>
            <label class="switch switch-table">
              <input type="checkbox" :checked="autoRefresh.enabled" @change="onAutoRefreshToggle($event)" />
              <span class="slider"></span>
            </label>
          </div>
          <div class="control-line">
            <strong>刷新间隔</strong>
            <div class="capacity-form log1-refresh-form">
              <input v-model.number="autoRefresh.intervalSeconds" type="number" min="5" @change="onAutoRefreshIntervalChange" />
              <span class="muted">秒</span>
            </div>
          </div>
        </section>

        <SystemAuditCapacityPanel
          class="log1-logs-card"
          :audit="audit"
          @submit-capacity="submitCapacity"
        />
      </div>
    </section>
  </section>
</template>

<style scoped>
.log1-system-page {
  display: flex;
  flex-direction: column;
  gap: 14px;
}

.log1-system-mode {
  display: flex;
  flex-direction: column;
  gap: 14px;
  min-width: 0;
}

.log1-system-grid-four {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

.log1-system-grid-two {
  grid-template-columns: repeat(2, minmax(0, 1fr));
}

.log1-system-grid-three {
  grid-template-columns: repeat(3, minmax(0, 1fr));
}

.log1-advanced-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 10px;
  margin-top: 14px;
}

.log1-switch-group-card {
  min-width: 0;
}

.log1-switch-grid {
  display: grid;
  grid-template-columns: 1fr;
  gap: 8px;
}

.log1-ip-strategy-list {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 8px;
}

.log1-core-mode-list,
.log1-dns-routing-list {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 8px;
}

.log1-ip-option {
  text-align: left;
  border: 1px solid var(--line);
  border-radius: 14px;
  background: rgba(var(--panel-glass-rgb), var(--panel-glass-opacity));
  padding: 12px;
  cursor: pointer;
  color: inherit;
  backdrop-filter: blur(var(--panel-glass-blur));
  -webkit-backdrop-filter: blur(var(--panel-glass-blur));
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.24);
  position: relative;
  overflow: hidden;
  isolation: isolate;
}

.log1-ip-option strong {
  display: flex;
  align-items: center;
  margin-bottom: 4px;
  font-size: 0.95rem;
  min-height: 22px;
}

.log1-ip-option span {
  color: var(--ink-1);
  font-size: 0.82rem;
  line-height: 1.45;
}

.log1-ip-option.active {
  border-color: var(--log1-menu-btn-active-border);
  background: var(--log1-menu-btn-active-bg);
  color: var(--log1-menu-btn-active-text);
  box-shadow: var(--log1-menu-btn-active-shadow);
}

.log1-ip-option::before {
  content: "";
  position: absolute;
  left: 0;
  top: 12%;
  bottom: 12%;
  width: 4px;
  border-radius: 0 999px 999px 0;
  background: var(--log1-menu-btn-active-line);
  opacity: 0;
  transform: scaleY(0.7);
  transition: transform 0.18s ease, opacity 0.18s ease;
}

.log1-ip-option.active::before {
  opacity: 1;
}

.log1-ip-option.active::before {
  transform: scaleY(1);
}

.log1-ip-option.active span {
  color: var(--log1-menu-btn-active-text);
}

:deep(.restart-mosdns-btn),
:deep(.core-mode-btn) {
  background: rgba(var(--panel-glass-rgb), var(--panel-glass-opacity)) !important;
  border-color: rgba(var(--panel-glass-border-rgb), 0.46) !important;
  color: var(--ink-0) !important;
  backdrop-filter: blur(var(--panel-glass-blur));
  -webkit-backdrop-filter: blur(var(--panel-glass-blur));
}

:deep(.restart-mosdns-btn:hover:not(:disabled)),
:deep(.core-mode-btn:hover:not(:disabled)) {
  background: rgba(var(--panel-glass-rgb), var(--panel-primary-hover-alpha)) !important;
  border-color: rgba(var(--panel-glass-border-rgb), 0.46) !important;
  color: var(--ink-0) !important;
}

:deep(.core-mode-btn.is-active) {
  background: var(--log1-menu-btn-active-bg) !important;
  border-color: var(--log1-menu-btn-active-border) !important;
  color: var(--log1-menu-btn-active-text) !important;
  box-shadow: var(--log1-menu-btn-active-shadow) !important;
}

:deep(.system-core-mode-option) {
  position: relative;
  overflow: hidden;
  isolation: isolate;
  border-radius: 14px;
}

:deep(.system-core-mode-option::before) {
  content: "";
  position: absolute;
  left: 0;
  top: 12%;
  bottom: 12%;
  width: 4px;
  border-radius: 0 999px 999px 0;
  background: var(--log1-menu-btn-active-line);
  opacity: 0;
  transform: scaleY(0.7);
  transition: transform 0.18s ease, opacity 0.18s ease;
}

:deep(.system-core-mode-option.active) {
  border-color: var(--log1-menu-btn-active-border) !important;
  background: var(--log1-menu-btn-active-bg) !important;
  color: var(--log1-menu-btn-active-text) !important;
  box-shadow: var(--log1-menu-btn-active-shadow) !important;
}

:deep(.system-core-mode-option.active::before) {
  opacity: 1;
  transform: scaleY(1);
}

:deep(.system-core-mode-option.active span) {
  color: var(--log1-menu-btn-active-text) !important;
}

.log1-module-toggle-row {
  display: grid;
  gap: 10px;
  margin-top: 14px;
}

.log1-module-toggle-row-single {
  grid-template-columns: 1fr;
}

.log1-module-toggle-row-center {
  display: flex;
  justify-content: center;
  align-items: center;
  margin-top: 12px;
}

.log1-module-toggle-row-quad {
  grid-template-columns: repeat(4, minmax(0, 1fr));
}

.log1-module-toggle {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  cursor: pointer;
  padding: 14px 16px;
  border: 1px solid var(--line);
  border-radius: 16px;
  background: linear-gradient(180deg, var(--surface-input), rgba(var(--panel-glass-rgb), 0.72));
  color: inherit;
  font: inherit;
  font-weight: 700;
  user-select: none;
  transition: 0.18s ease;
}

.log1-module-toggle-arrow {
  color: var(--brand);
  font-size: 1rem;
  line-height: 1;
  flex: 0 0 auto;
}

.log1-module-toggle.active {
  border-color: var(--log1-menu-btn-active-border);
  background: var(--log1-menu-btn-active-bg);
  color: var(--log1-menu-btn-active-text);
  box-shadow: var(--log1-menu-btn-active-shadow);
}

.log1-module-toggle-compact {
  min-width: 0;
  width: auto;
  padding: 4px 14px;
  border-radius: 999px;
  gap: 4px;
  font-size: 0.78rem;
  font-weight: 500;
  justify-content: center;
  background: transparent;
  border-style: dashed;
  border-color: rgba(var(--panel-glass-border-rgb), 0.6);
  color: var(--ink-1);
  box-shadow: none;
}

.log1-module-toggle-compact:hover {
  border-color: rgba(var(--brand-rgb), 0.5);
  color: var(--ink-0);
  background: transparent;
}

.log1-module-toggle-compact.active {
  border-color: var(--log1-menu-btn-active-border);
  color: var(--log1-menu-btn-active-text);
  background: var(--log1-menu-btn-active-bg);
  box-shadow: var(--log1-menu-btn-active-shadow);
}

.log1-advanced-module-panel {
  margin-top: 0;
}

.log1-logs-grid {
  display: grid;
  grid-template-columns: repeat(3, minmax(0, 1fr));
  gap: 12px;
}

.log1-refresh-form {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  max-width: 170px;
}

.log1-refresh-form input {
  min-width: 0;
}

.log1-refresh-panel {
  justify-content: flex-start;
}

.log1-logs-card {
  min-height: 0;
  height: 100%;
}

.log1-refresh-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
}

.log1-refresh-head h3 {
  margin: 0;
}

.log1-audit-card {
  justify-content: space-between;
}

.log1-audit-actions {
  margin-top: auto;
}

@media (max-width: 1280px) {
  .log1-advanced-grid,
  .log1-system-grid-four,
  .log1-system-grid-three,
  .log1-logs-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
}

@media (max-width: 900px) {
  .log1-advanced-grid,
  .log1-logs-grid,
  .log1-system-grid-four,
  .log1-system-grid-three,
  .log1-system-grid-two,
  .log1-module-toggle-row-quad,
  .log1-core-mode-list,
  .log1-dns-routing-list,
  .log1-ip-strategy-list {
    grid-template-columns: 1fr;
  }
}

:deep(.appearance-compact-module) {
  max-width: none;
  width: 100%;
}

:deep(.appearance-compact-module .appearance-compact-control-end select) {
  max-width: none;
}

:deep(.appearance-compact-module .appearance-compact-stack) {
  display: grid;
  grid-template-columns: minmax(220px, 0.9fr) minmax(0, 1.4fr);
  gap: 14px 16px;
  align-items: start;
}

:deep(.appearance-compact-module .appearance-compact-row-theme) {
  grid-column: 1;
}

:deep(.appearance-compact-module .appearance-color-pair-row) {
  grid-column: 2;
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 12px;
}

:deep(.appearance-compact-module .appearance-compact-row-bg) {
  grid-column: 1 / -1;
}

:deep(.appearance-compact-module .appearance-compact-row) {
  min-width: 0;
}

:deep(.appearance-compact-module .appearance-color-pair) {
  min-width: 0;
}

:deep(.appearance-compact-module .appearance-compact-bg-layout) {
  grid-template-columns: 48px minmax(0, 1fr) auto;
}

@media (max-width: 1100px) {
  :deep(.appearance-compact-module .appearance-compact-stack) {
    grid-template-columns: 1fr;
  }

  :deep(.appearance-compact-module .appearance-compact-row-theme),
  :deep(.appearance-compact-module .appearance-color-pair-row),
  :deep(.appearance-compact-module .appearance-compact-row-bg) {
    grid-column: 1;
  }
}

@media (max-width: 760px) {
  :deep(.appearance-compact-module .appearance-color-pair-row) {
    grid-template-columns: 1fr;
  }

  :deep(.appearance-compact-module .appearance-compact-bg-layout) {
    grid-template-columns: 1fr;
  }
}
</style>

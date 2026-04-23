<script setup>
import { computed, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, getText, postJSON } from '../api/http'
import { openConfirm } from '../utils/confirm'
import { getDefaultPanelBackgroundSettings, normalizePanelBackgroundSettings, previewPanelBackground, transparencyToOpacity } from '../utils/panelBackground'

const loading = ref(false)
const errorMessage = ref('')
const successMessage = ref('')

const audit = reactive({
  capturing: null,
  capacity: null,
  newCapacity: ''
})

const update = reactive({
  loading: false,
  status: null
})

const systemInfo = reactive({
  startTime: 0,
  cpuTime: 0,
  residentMemory: 0,
  heapIdleMemory: 0,
  threads: 0,
  openFds: 0,
  grs: 0,
  goVersion: 'N/A'
})

const restarting = ref(false)
const applyingOverrides = ref(false)
const configManaging = reactive({
  localDir: '',
  remoteUrl: '',
  backingUp: false,
  updating: false
})

const overrides = reactive({
  socks5: '',
  ecs: '',
  replacements: []
})

const switchLoading = reactive({})
const switchStates = reactive({})

const autoRefresh = reactive({
  enabled: false,
  intervalSeconds: 15
})

const appearance = reactive({
  theme: 'dark',
  color: 'indigo'
})

const panelBackgroundDefaults = getDefaultPanelBackgroundSettings()
const panelBackgroundMaxUpload = 20 * 1024 * 1024
const panelBackgroundPicker = ref(null)
const panelBackground = reactive({
  mode: panelBackgroundDefaults.mode,
  url: '',
  imageUrl: '',
  transparency: panelBackgroundDefaults.transparency,
  opacity: panelBackgroundDefaults.opacity,
  blur: panelBackgroundDefaults.blur,
  applying: false,
  uploading: false
})

let restartProbeTimerId = 0

const themeOptions = [
  { value: 'light', label: '明亮' },
  { value: 'dark', label: '黑暗' }
]

const colorOptions = [
  { value: 'classic', label: '经典绿', color: '#0f766e' },
  { value: 'indigo', label: '靛蓝', color: '#4f46e5' },
  { value: 'pink', label: '粉色', color: '#ec4899' },
  { value: 'teal', label: '青色', color: '#14b8a6' },
  { value: 'orange', label: '橙色', color: '#f97316' },
  { value: 'green', label: '绿色', color: '#22c55e' },
  { value: 'violet', label: '紫色', color: '#8b5cf6' }
]

const switchProfiles = [
  {
    tag: 'switch3',
    name: '核心运行模式',
    tip: '切换后会按新模式重建分流相关缓存和数据。',
    modes: { A: { name: '兼容模式' }, B: { name: '安全模式' } }
  },
  { tag: 'switch1', name: '请求屏蔽', desc: '对无解析结果的请求进行屏蔽', tip: '建议开启。', valueForOn: 'A' },
  { tag: 'switch5', name: '类型屏蔽', desc: '屏蔽 SOA / PTR / HTTPS 等请求', tip: '建议开启。', valueForOn: 'A' },
  { tag: 'switch4', name: '过期缓存1', desc: '启用多组过期缓存', tip: '建议开启。', valueForOn: 'A' },
  { tag: 'switch13', name: '过期缓存2', desc: '启用全量缓存与 fakeip 缓存', tip: '建议开启。', valueForOn: 'A' },
  { tag: 'switch7', name: '广告屏蔽', desc: '启用 AdGuard 在线规则支持', tip: '按需开启。', valueForOn: 'A' },
  { tag: 'switch9', name: 'CNFakeIP', desc: '国内域名返回 fakeip', tip: '切换后会自动清空核心缓存。', valueForOn: 'B' },
  { tag: 'switch15', name: '极限加速', desc: '启用极限缓存与加速路径', tip: '按需开启。', valueForOn: 'A' },
  { tag: 'switch2', name: '指定 Client fakeip', desc: '仅指定客户端可科学', tip: '需配合 client_ip 名单。', valueForOn: 'A' },
  { tag: 'switch12', name: '指定 Client realip', desc: '指定客户端不科学', tip: '需配合 client_ip 名单。', valueForOn: 'A' },
  { tag: 'switch6', name: 'IPV6屏蔽', desc: '屏蔽 AAAA 请求', tip: '无 IPV6 场景建议开启。', valueForOn: 'A' }
]

const coreMode = computed(() => String(switchStates.switch3 || ''))
const secondarySwitches = computed(() => switchProfiles.filter((profile) => !profile.modes))

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
  return (arch === 'linux/amd64' || arch === 'windows/amd64') && Boolean(status.amd64_v3_capable) && !Boolean(status.current_is_v3)
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

const updateTargetInfo = computed(() => {
  const status = update.status
  if (!status) {
    return '--'
  }
  if (status.asset_name) {
    return `${status.asset_name} (${status.architecture || '未知'})`
  }
  return status.architecture || '--'
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

function setError(message) {
  successMessage.value = ''
  errorMessage.value = message
}

function setSuccess(message) {
  errorMessage.value = ''
  successMessage.value = message
}

function clearMessage() {
  errorMessage.value = ''
  successMessage.value = ''
}

function normalizeVersion(value) {
  if (!value) {
    return ''
  }
  return String(value).trim().toLowerCase().replace(/^v/, '')
}

function formatRelativeTime(value) {
  if (!value) {
    return '-'
  }
  const ts = new Date(value).getTime()
  if (!Number.isFinite(ts)) {
    return String(value)
  }
  const diff = Math.max(0, Math.floor((Date.now() - ts) / 1000))
  if (diff < 5) {
    return '刚刚'
  }
  if (diff < 60) {
    return `${diff}秒前`
  }
  if (diff < 3600) {
    return `${Math.floor(diff / 60)}分钟前`
  }
  if (diff < 86400) {
    return `${Math.floor(diff / 3600)}小时前`
  }
  return new Date(value).toLocaleString('zh-CN', { hour12: false })
}

function parseSystemMetrics(metricsText) {
  const lines = String(metricsText || '').split('\n')
  const out = {
    startTime: 0,
    cpuTime: 0,
    residentMemory: 0,
    heapIdleMemory: 0,
    threads: 0,
    openFds: 0,
    grs: 0,
    goVersion: 'N/A'
  }
  lines.forEach((line) => {
    if (line.startsWith('process_start_time_seconds')) {
      out.startTime = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('process_cpu_seconds_total')) {
      out.cpuTime = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('process_resident_memory_bytes')) {
      out.residentMemory = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_memstats_heap_idle_bytes')) {
      out.heapIdleMemory = Number.parseFloat(line.split(' ')[1] || '0') || 0
    } else if (line.startsWith('go_threads')) {
      out.threads = Number.parseInt(line.split(' ')[1] || '0', 10) || 0
    } else if (line.startsWith('process_open_fds')) {
      out.openFds = Number.parseInt(line.split(' ')[1] || '0', 10) || 0
    } else if (line.startsWith('go_goroutines')) {
      out.grs = Number.parseInt(line.split(' ')[1] || '0', 10) || 0
    } else if (line.startsWith('go_info{version="')) {
      const match = line.match(/go_info\{version="([^"]+)"/)
      if (match?.[1]) {
        out.goVersion = match[1]
      }
    }
  })
  return out
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

async function loadAuditStatusAndCapacity() {
  const [statusRes, capacityRes] = await Promise.all([
    getJSON('/api/v1/audit/status'),
    getJSON('/api/v1/audit/capacity')
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
  if (!(await openConfirm('将删除当前所有内存审计日志，此操作不可恢复。', { tone: 'danger' }))) {
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
    setError('请输入 1 到 400000 之间的有效容量')
    return
  }
  if (!(await openConfirm(`将容量设置为 ${capacity.toLocaleString()}，并清空当前所有日志。`, { tone: 'danger' }))) {
    return
  }
  clearMessage()
  try {
    await postJSON('/api/v1/audit/capacity', { capacity })
    audit.newCapacity = ''
    setSuccess(`容量已设置为 ${capacity.toLocaleString()}`)
    await loadAuditStatusAndCapacity()
  } catch (error) {
    setError(`设置容量失败: ${error.message}`)
  }
}

async function loadFeatureSwitches() {
  const settled = await Promise.allSettled(
    switchProfiles.map((profile) => getText(`/plugins/${profile.tag}/show`))
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

async function toggleSecondarySwitch(profile, checked) {
  if (!profile?.tag || !profile.valueForOn) {
    return
  }
  clearMessage()
  const next = checked ? profile.valueForOn : (profile.valueForOn === 'A' ? 'B' : 'A')
  try {
    await setSwitchValue(profile.tag, next, `“${profile.name}” 已${checked ? '启用' : '禁用'}`)
    if (profile.tag === 'switch9') {
      await Promise.allSettled([
        requestResponse('/plugins/cache_all/flush'),
        requestResponse('/plugins/cache_all_noleak/flush')
      ])
    }
  } catch (error) {
    setError(`切换“${profile.name}”失败: ${error.message}`)
    await loadFeatureSwitches()
  }
}

function isSwitchChecked(profile) {
  if (!profile?.tag) {
    return false
  }
  return switchStates[profile.tag] === profile.valueForOn
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
    result: String(item?.result || '')
  }))
}

function addReplacement() {
  overrides.replacements.push({
    original: '',
    new: '',
    comment: '',
    result: ''
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
        comment: String(rule.comment || '').trim()
      }))
      .filter((rule) => rule.original)

    await postJSON('/api/v1/overrides', {
      socks5: String(overrides.socks5 || '').trim(),
      ecs: String(overrides.ecs || '').trim(),
      replacements
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

async function loadSystemInfo() {
  const metricsText = await getText('/metrics')
  const next = parseSystemMetrics(metricsText)
  Object.assign(systemInfo, next)
}

async function restartMosdns() {
  if (!(await openConfirm('确认重启 MosDNS？', { tone: 'danger' }))) {
    return
  }
  clearMessage()
  restarting.value = true
  try {
    await postJSON('/api/v1/system/restart', { delay_ms: 500 })
    setSuccess('已发送重启请求，页面将自动刷新。')
    setTimeout(() => window.location.reload(), 4000)
  } catch (error) {
    setError(`重启失败: ${error.message}`)
  } finally {
    restarting.value = false
  }
}

async function loadUpdateStatus() {
  const status = await getJSON('/api/v1/update/status')
  update.status = status
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
    const response = await postJSON('/api/v1/update/apply', { force, prefer_v3: preferV3 })
    const status = response?.status || response
    update.status = status
    setSuccess(String(response?.notes || status?.message || '更新请求已提交'))
    if (status?.pending_restart && !String(status?.architecture || '').startsWith('windows/')) {
      startRestartWatch()
    }
  } catch (error) {
    setError(`执行更新失败: ${error.message}`)
  } finally {
    update.loading = false
  }
}

function applyTheme(theme, save = true) {
  const nextTheme = ['light', 'dark'].includes(String(theme)) ? String(theme) : 'light'
  appearance.theme = nextTheme
  document.documentElement.setAttribute('data-theme', nextTheme)
  if (save) {
    localStorage.setItem('mosdns-theme', nextTheme)
  }
}

function applyColor(color, save = true) {
  appearance.color = color
  document.documentElement.setAttribute('data-color-scheme', color)
  if (save) {
    localStorage.setItem('mosdns-color', color)
  }
}

function initializeAppearance() {
  applyTheme(localStorage.getItem('mosdns-theme') || 'light', false)
  applyColor(localStorage.getItem('mosdns-color') || 'classic', false)
}

function applyPanelBackgroundDraft(raw) {
  const normalized = normalizePanelBackgroundSettings(raw || {})
  panelBackground.mode = normalized.mode
  panelBackground.url = normalized.url
  panelBackground.imageUrl = normalized.imageUrl
  panelBackground.transparency = normalized.transparency
  panelBackground.opacity = normalized.opacity
  panelBackground.blur = normalized.blur
}

function getPanelBackgroundDraftForPreview() {
  return {
    mode: panelBackground.mode,
    url: panelBackground.url,
    image_url: panelBackground.imageUrl,
    opacity: transparencyToOpacity(panelBackground.transparency),
    blur: panelBackground.blur
  }
}

async function syncPanelBackgroundPreview(showError = false) {
  const result = await previewPanelBackground(getPanelBackgroundDraftForPreview(), {
    onError: (error) => {
      if (showError) {
        setError(`背景加载失败，已回退默认背景: ${error.message}`)
      }
    }
  })
  return result.ok
}

function buildPanelBackgroundPayload() {
  const normalized = normalizePanelBackgroundSettings(getPanelBackgroundDraftForPreview())
  return {
    mode: normalized.mode,
    url: normalized.mode === 'url' ? normalized.url : '',
    opacity: normalized.opacity,
    blur: normalized.blur
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

async function applyPanelBackgroundSettings() {
  clearMessage()
  panelBackground.applying = true
  try {
    if (panelBackground.mode !== 'upload') {
      panelBackground.mode = panelBackground.url.trim() ? 'url' : 'none'
    }
    const payload = buildPanelBackgroundPayload()
    if ((payload.mode === 'url' || payload.mode === 'upload') && !(await syncPanelBackgroundPreview(true))) {
      return
    }
    const saved = await postJSON('/api/v1/appearance/panel-background', payload)
    applyPanelBackgroundDraft(saved)
    await syncPanelBackgroundPreview(false)
    setSuccess('面板背景已应用')
  } catch (error) {
    setError(`应用面板背景失败: ${error.message}`)
  } finally {
    panelBackground.applying = false
  }
}

async function resetPanelBackgroundSettings() {
  clearMessage()
  panelBackground.mode = 'none'
  panelBackground.url = ''
  panelBackground.imageUrl = ''
  panelBackground.transparency = panelBackgroundDefaults.transparency
  panelBackground.opacity = panelBackgroundDefaults.opacity
  panelBackground.blur = panelBackgroundDefaults.blur
  await applyPanelBackgroundSettings()
  if (!errorMessage.value) {
    setSuccess('面板背景已重置')
  }
}

async function onPanelBackgroundUrlEnter() {
  panelBackground.mode = 'url'
  await applyPanelBackgroundSettings()
}

function onPanelBackgroundSliderInput() {
  panelBackground.opacity = transparencyToOpacity(panelBackground.transparency)
  void syncPanelBackgroundPreview(false)
}

function openPanelBackgroundPicker() {
  panelBackgroundPicker.value?.click()
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
      body: formData
    })
    if (!response.ok) {
      const text = await response.text().catch(() => '')
      throw new Error(text || `HTTP ${response.status} ${response.statusText}`)
    }
    const data = await response.json()
    panelBackground.mode = 'upload'
    panelBackground.imageUrl = String(data?.image_url || '')
    if (!(await syncPanelBackgroundPreview(true))) {
      return
    }
    await applyPanelBackgroundSettings()
    if (!errorMessage.value) {
      setSuccess('图片已上传并应用')
    }
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
  autoRefresh.intervalSeconds = Math.max(5, Number(saved?.intervalSeconds || 15))
}

function loadConfigManagerSettings() {
  configManaging.localDir = localStorage.getItem('mosdns-config-dir') || ''
  configManaging.remoteUrl = localStorage.getItem('mosdns-config-url') || ''
}

function saveConfigManagerSettings() {
  localStorage.setItem('mosdns-config-dir', String(configManaging.localDir || '').trim())
  localStorage.setItem('mosdns-config-url', String(configManaging.remoteUrl || '').trim())
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
      body: JSON.stringify({ dir })
    })
    if (!response.ok) {
      const text = await response.text().catch(() => '')
      throw new Error(text || `HTTP ${response.status} ${response.statusText}`)
    }
    const blob = await response.blob()
    const downloadUrl = window.URL.createObjectURL(blob)
    const link = document.createElement('a')
    const disposition = response.headers.get('Content-Disposition') || response.headers.get('content-disposition') || ''
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
  if (!(await openConfirm('当前配置会先备份到 backup 子目录，新配置将覆盖现有文件，并在完成后自动重启 MosDNS。', { tone: 'danger' }))) {
    return
  }
  clearMessage()
  saveConfigManagerSettings()
  configManaging.updating = true
  try {
    const response = await postJSON('/api/v1/config/update_from_url', { url, dir })
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
  autoRefresh.intervalSeconds = Math.max(5, Number(autoRefresh.intervalSeconds || 15))
  const payload = {
    enabled: Boolean(autoRefresh.enabled),
    intervalSeconds: autoRefresh.intervalSeconds
  }
  localStorage.setItem('mosdnsAutoRefresh', JSON.stringify(payload))
  window.dispatchEvent(new CustomEvent('mosdns-auto-refresh-update', { detail: payload }))
  if (showToast) {
    setSuccess(`自动刷新已${payload.enabled ? `开启（${payload.intervalSeconds}秒）` : '关闭'}`)
  }
}

function setAutoRefreshEnabled(enabled) {
  if (autoRefresh.enabled === enabled) {
    return
  }
  autoRefresh.enabled = enabled
  emitAutoRefreshSettings(true)
}

function onAutoRefreshToggle(event) {
  setAutoRefreshEnabled(Boolean(event?.target?.checked))
}

function onAutoRefreshIntervalChange() {
  emitAutoRefreshSettings(true)
}

async function reloadAll() {
  loading.value = true
  clearMessage()
  try {
    await Promise.all([
      loadAuditStatusAndCapacity(),
      loadFeatureSwitches(),
      loadOverrides(),
      loadSystemInfo(),
      loadUpdateStatus()
    ])
  } catch (error) {
    setError(`加载系统设置失败: ${error.message}`)
  } finally {
    loading.value = false
  }
}

async function refreshOnGlobalEvent() {
  await reloadAll()
}

onMounted(() => {
  initializeAppearance()
  loadAutoRefreshSettings()
  loadConfigManagerSettings()
  emitAutoRefreshSettings(false)
  loadPanelBackgroundSettings()
  reloadAll()
  window.addEventListener('mosdns-log-refresh', refreshOnGlobalEvent)
})

onBeforeUnmount(() => {
  window.removeEventListener('mosdns-log-refresh', refreshOnGlobalEvent)
  stopRestartWatch()
})
</script>

<template>
  <section class="panel system-panel">
    <header class="panel-header">
      <div class="actions">
        <button class="btn tiny secondary restart-mosdns-btn" :disabled="restarting" @click="restartMosdns">{{ restarting ? '处理中...' : '重启 MosDNS' }}</button>
      </div>
    </header>

    <p v-if="errorMessage" class="msg error">{{ errorMessage }}</p>
    <p v-if="successMessage && !errorMessage" class="msg success">{{ successMessage }}</p>

    <div class="system-layout-stack">
      <div class="control-panel-grid system-grid-quad">
        <section class="panel control-module control-module--mini">
          <h3>系统信息</h3>
          <div class="module-kv-list">
            <div class="control-line"><strong>启动时间</strong><span>{{ systemInfo.startTime ? new Date(systemInfo.startTime * 1000).toLocaleString('zh-CN', { hour12: false }) : 'N/A' }}</span></div>
            <div class="control-line"><strong>CPU 时间</strong><span>{{ Number(systemInfo.cpuTime || 0).toFixed(2) }} 秒</span></div>
            <div class="control-line"><strong>常驻内存 (RSS)</strong><span>{{ (Number(systemInfo.residentMemory || 0) / 1024 / 1024).toFixed(2) }} MB</span></div>
            <div class="control-line"><strong>待用堆内存 (Idle)</strong><span>{{ (Number(systemInfo.heapIdleMemory || 0) / 1024 / 1024).toFixed(2) }} MB</span></div>
            <div class="control-line"><strong>Go 版本</strong><span>{{ systemInfo.goVersion }}</span></div>
          </div>
        </section>

        <section class="panel control-module control-module--mini">
          <h3>版本与更新</h3>
          <div class="module-kv-list">
            <div class="control-line"><strong>当前版本</strong><span>{{ update.status?.current_version || '未知' }}</span></div>
            <div class="control-line"><strong>最新版本</strong><span>{{ update.status?.latest_version || '--' }} <span v-if="updateLatestBadge" class="mini-badge">已是最新</span></span></div>
            <div class="control-line"><strong>上次检查</strong><span>{{ updateLastCheckedText }}</span></div>
          </div>
          <p class="update-banner">{{ updateBannerText }}</p>
          <div class="actions">
            <button class="btn tiny secondary" :disabled="update.loading" @click="checkUpdate">{{ update.loading ? '处理中...' : '检查更新' }}</button>
            <button class="btn tiny primary" :disabled="update.loading || !hasUpdate" @click="applyUpdate(false, false)">立即更新</button>
            <button class="btn tiny danger" :disabled="update.loading || !update.status?.download_url" @click="applyUpdate(true, false)">强制更新</button>
            <button v-if="showV3Callout" class="btn tiny warning" :disabled="update.loading" @click="applyUpdate(true, true)">切换 v3</button>
          </div>
        </section>

        <section class="panel control-module control-module--mini">
          <h3>配置管理</h3>
          <div class="module-form-stack">
            <label class="mini-field">
              <span>本地目录</span>
              <input v-model="configManaging.localDir" placeholder="/cus/mosdns" @change="saveConfigManagerSettings" />
            </label>
            <label class="mini-field">
              <span>远程 ZIP URL</span>
              <input v-model="configManaging.remoteUrl" placeholder="https://example.com/mosdns.zip" @change="saveConfigManagerSettings" />
            </label>
          </div>
          <div class="actions">
            <button class="btn tiny secondary" :disabled="configManaging.backingUp" @click="backupConfig">
              {{ configManaging.backingUp ? '备份中...' : '备份配置' }}
            </button>
            <button class="btn tiny primary" :disabled="configManaging.updating" @click="applyRemoteConfig">
              {{ configManaging.updating ? '更新中...' : '应用远程配置' }}
            </button>
          </div>
        </section>

        <section class="panel control-module control-module--mini">
          <h3>SOCKS5 / ECS 覆盖</h3>
          <div class="module-form-stack">
            <label class="mini-field">
              <span>socks5 代理</span>
              <input v-model="overrides.socks5" placeholder="host:port" />
            </label>
            <label class="mini-field">
              <span>ECS IP</span>
              <input v-model="overrides.ecs" placeholder="IPv4 / IPv6" />
            </label>
          </div>
          <div class="actions">
            <button class="btn tiny secondary" @click="loadOverrides">读取当前</button>
            <button class="btn tiny primary" :disabled="applyingOverrides" @click="saveOverrides">
              {{ applyingOverrides ? '保存中...' : '保存并应用' }}
            </button>
          </div>
        </section>
      </div>

      <div class="control-panel-grid">
        <section class="panel control-module control-module-wide">
          <header class="module-head">
            <div>
              <h3>核心运行模式</h3>
            </div>
            <div class="actions">
              <button class="btn tiny core-mode-btn" :class="coreMode === 'A' ? 'primary is-active' : 'secondary'" :disabled="switchLoading.switch3" @click="setCoreMode('A')">兼容模式</button>
              <button class="btn tiny core-mode-btn" :class="coreMode === 'B' ? 'primary is-active' : 'secondary'" :disabled="switchLoading.switch3" @click="setCoreMode('B')">安全模式</button>
            </div>
          </header>
          <div class="core-mode-hints">
            <p class="muted">兼容模式：表外域名优先国内dns解析，保证速度。</p>
            <p class="muted">安全模式：表外域名仅用国外dns解析，阻止dns泄漏。</p>
          </div>
        </section>
      </div>

      <div class="control-panel-grid">
        <section class="panel control-module control-module-wide">
          <header class="module-head">
            <div>
              <h3>功能开关</h3>
            </div>
          </header>

          <div class="switch-list">
            <label v-for="profile in secondarySwitches" :key="profile.tag" class="switch-row">
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
      </div>

      <div class="control-panel-grid">
        <section class="panel control-module control-module-wide">
          <header class="module-head">
            <div>
              <h3>高级替换规则</h3>
              <p class="muted">可配置 DNS 覆盖映射。修改后点击保存并应用。</p>
            </div>
            <div class="actions">
              <button class="btn tiny secondary" @click="loadOverrides">读取当前</button>
              <button class="btn tiny secondary" @click="addReplacement">添加规则</button>
              <button class="btn tiny primary" :disabled="applyingOverrides" @click="saveOverrides">
                {{ applyingOverrides ? '保存中...' : '保存并应用' }}
              </button>
            </div>
          </header>

          <div class="table-wrap replacements-table-wrap">
            <table class="replacements-table">
              <thead>
                <tr>
                  <th>状态</th>
                  <th>原值</th>
                  <th>新值</th>
                  <th>备注</th>
                  <th>操作</th>
                </tr>
              </thead>
              <tbody>
                <tr v-if="overrides.replacements.length === 0">
                  <td colspan="5" class="empty">暂无规则</td>
                </tr>
                <tr v-for="(item, index) in overrides.replacements" :key="`rep-${index}`">
                  <td>{{ item.result || '未保存' }}</td>
                  <td><input v-model="item.original" placeholder="例如: 1.1.1.1" /></td>
                  <td><input v-model="item.new" placeholder="例如: 127.0.0.1" /></td>
                  <td><input v-model="item.comment" placeholder="可选备注" /></td>
                  <td><button class="btn tiny danger" @click="removeReplacement(index)">删除</button></td>
                </tr>
              </tbody>
            </table>
          </div>
        </section>
      </div>

      <div class="control-panel-grid system-grid-quad">
        <section class="panel control-module control-module--mini">
          <h3>审计控制</h3>
          <div class="control-line">
            <strong>运行状态</strong>
            <span>{{ audit.capturing === null ? '读取中...' : (audit.capturing ? '运行中' : '已停止') }}</span>
          </div>
          <div class="button-group-vue">
            <button class="btn tiny primary" @click="toggleAuditCapture">{{ audit.capturing ? '停止审计' : '启动审计' }}</button>
            <button class="btn tiny danger" @click="clearAuditLogs">清空日志</button>
          </div>
        </section>

        <section class="panel control-module control-module--mini">
          <h3>日志容量</h3>
          <div class="control-line">
            <strong>当前容量</strong>
            <span>{{ audit.capacity === null ? '读取中...' : Number(audit.capacity).toLocaleString() }}</span>
          </div>
          <form class="capacity-form" @submit.prevent="submitCapacity">
            <input v-model="audit.newCapacity" type="number" min="1" max="400000" placeholder="输入新容量" />
            <button class="btn tiny primary" type="submit">设置</button>
          </form>
          <p class="muted">设置新容量将清空日志。</p>
        </section>

        <section class="panel control-module control-module--mini">
          <h3>自动刷新</h3>
          <div class="control-line">
            <strong>启用状态</strong>
            <label class="switch switch-table">
              <input type="checkbox" :checked="autoRefresh.enabled" @change="onAutoRefreshToggle" />
              <span class="slider"></span>
            </label>
          </div>
          <div class="control-line">
            <strong>刷新间隔</strong>
            <div class="capacity-form" style="max-width: 170px;">
              <input v-model.number="autoRefresh.intervalSeconds" type="number" min="5" @change="onAutoRefreshIntervalChange" />
              <span class="muted">秒</span>
            </div>
          </div>
        </section>

        <section class="panel control-module control-module--mini">
          <h3>主题与外观</h3>
          <div class="control-line"><strong>界面风格</strong><select v-model="appearance.theme" @change="applyTheme(appearance.theme)"><option v-for="opt in themeOptions" :key="opt.value" :value="opt.value">{{ opt.label }}</option></select></div>
          <div class="control-line panel-bg-line">
            <strong>面板背景</strong>
            <div class="panel-bg-input-wrap">
              <input
                v-model="panelBackground.url"
                placeholder="输入图片 URL，回车直接应用"
                @keydown.enter.prevent="onPanelBackgroundUrlEnter"
              />
              <button class="btn tiny secondary" type="button" :disabled="panelBackground.uploading || panelBackground.applying" @click="openPanelBackgroundPicker">
                {{ panelBackground.uploading ? '上传中...' : '上传' }}
              </button>
              <input
                ref="panelBackgroundPicker"
                class="panel-bg-file-input"
                type="file"
                accept="image/*"
                @change="onPanelBackgroundFileChange"
              />
            </div>
          </div>
          <div class="control-line">
            <strong>透明度</strong>
            <div class="panel-bg-range-wrap">
              <input v-model.number="panelBackground.transparency" type="range" min="0" max="100" step="1" @input="onPanelBackgroundSliderInput" />
              <span>{{ Number(panelBackground.transparency || 0) }}%</span>
            </div>
          </div>
          <div class="control-line">
            <strong>毛玻璃强度</strong>
            <div class="panel-bg-range-wrap">
              <input v-model.number="panelBackground.blur" type="range" min="0" max="40" step="1" @input="onPanelBackgroundSliderInput" />
              <span>{{ Number(panelBackground.blur || 0) }}px</span>
            </div>
          </div>
          <div class="actions">
            <button class="btn tiny primary" type="button" :disabled="panelBackground.applying || panelBackground.uploading" @click="applyPanelBackgroundSettings">
              {{ panelBackground.applying ? '应用中...' : '应用' }}
            </button>
            <button class="btn tiny secondary" type="button" :disabled="panelBackground.applying || panelBackground.uploading" @click="resetPanelBackgroundSettings">重置</button>
          </div>
          <div class="color-palette-vue">
            <button
              v-for="opt in colorOptions"
              :key="opt.value"
              class="color-swatch-vue"
              :class="{ active: appearance.color === opt.value }"
              :style="{ backgroundColor: opt.color }"
              :title="opt.label"
              @click="applyColor(opt.value)"
            />
          </div>
        </section>
      </div>
    </div>
  </section>
</template>

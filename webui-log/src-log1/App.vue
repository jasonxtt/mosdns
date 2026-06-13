<script setup>
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from '../src/api/http'
import ConfirmBubbleHost from '../src/components/ConfirmBubbleHost.vue'
import DataManagementManager from '../src/components/DataManagementManager.vue'
import ListManager from '../src/components/ListManager.vue'
import OverviewManager from '../src/components/OverviewManager.vue'
import QueryManager from '../src/components/QueryManager.vue'
import RulesManager from '../src/components/RulesManager.vue'
import SystemControlManager from './SystemControlManager.vue'
import { openConfirm } from '../src/utils/confirm'
import { previewPanelBackground } from '../src/utils/panelBackground'
import { applyTextColorForTheme, loadTextColorSettingsFromStorage, normalizeTextColorSettings, saveTextColorSettingsToStorage } from '../src/utils/appearanceTextColor'
import { applyButtonColorForTheme, loadButtonColorSettingsFromStorage, normalizeButtonColorSettings, saveButtonColorSettingsToStorage } from '../src/utils/appearanceButtonColor'

const DEFAULT_AUTO_REFRESH_STATE = { enabled: false, intervalSeconds: 15 }

const activeMainTab = ref('overview')
const activeQuerySubTab = ref('live')
const activeRulesSubTab = ref('list-mgmt')
const activeDataSubTab = ref('cache-management')

const mainTabs = [
  { id: 'overview', label: '概览', icon: '◫' },
  { id: 'log-query', label: '查询日志', icon: '⌕' },
  { id: 'rules', label: '规则管理', icon: '☰' },
  { id: 'data-management', label: '数据管理', icon: '◌' },
  { id: 'system-control', label: '系统设置', icon: '⚙' }
]

const querySubTabs = [
  { id: 'live', label: '实时查询', icon: '⌕' },
  { id: 'diagnostic', label: '诊断抓取', icon: '◎' }
]

const rulesSubTabs = [
  { id: 'list-mgmt', label: '本地规则', icon: '☰' },
  { id: 'diversion', label: '订阅规则', icon: '↗' },
  { id: 'adguard', label: '广告拦截', icon: '⛶' }
]

const dataSubTabs = [
  { id: 'cache-management', label: '缓存管理', icon: '◫' },
  { id: 'domain-stats', label: '域名统计', icon: '◎' },
  { id: 'requery-cache', label: '刷新分流', icon: '↻' }
]

const systemSecondaryTabs = [
  { id: 'system-upstream', label: '上游设置', icon: '⇄' },
  { id: 'system-maintenance', label: '运行维护', icon: '↻' },
  { id: 'system-behavior', label: '解析行为', icon: '⌁' },
  { id: 'system-preferences', label: '界面偏好', icon: '◐' },
  { id: 'system-logs', label: '日志刷新', icon: '◎' }
]

const activeSystemSection = ref('system-upstream')
const restartLoading = ref(false)

const autoRefreshState = ref({
  enabled: false,
  intervalSeconds: 15
})

const topNotice = reactive({
  open: false,
  tone: 'success',
  message: ''
})

let autoRefreshTimerId = 0
let topNoticeTimerId = 0

const currentSecondaryTabs = computed(() => {
  switch (activeMainTab.value) {
    case 'log-query':
      return querySubTabs
    case 'rules':
      return rulesSubTabs
    case 'data-management':
      return dataSubTabs
    case 'system-control':
      return systemSecondaryTabs
    default:
      return []
  }
})

const hasSecondaryTabs = computed(() => currentSecondaryTabs.value.length > 0)
const showTopRail = computed(() => activeMainTab.value !== 'overview')

function formatPrimaryLabel(label = '') {
  return String(label || '')
}

function initializeAppearance() {
  const root = document.documentElement
  const theme = ['light', 'dark'].includes(String(localStorage.getItem('mosdns-theme'))) ? String(localStorage.getItem('mosdns-theme')) : 'light'
  root.setAttribute('data-theme', theme)
  const cachedTextColors = loadTextColorSettingsFromStorage()
  const cachedButtonColors = loadButtonColorSettingsFromStorage()
  applyTextColorForTheme(theme, cachedTextColors)
  applyButtonColorForTheme(theme, cachedButtonColors)
}

function stopAutoRefresh() {
  if (autoRefreshTimerId) {
    window.clearInterval(autoRefreshTimerId)
    autoRefreshTimerId = 0
  }
}

function startAutoRefresh() {
  stopAutoRefresh()
  if (!autoRefreshState.value.enabled || document.hidden) {
    return
  }
  autoRefreshTimerId = window.setInterval(() => {
    window.dispatchEvent(new CustomEvent('mosdns-log-refresh'))
  }, Math.max(5, autoRefreshState.value.intervalSeconds) * 1000)
}

function applyAutoRefreshState(state = {}) {
  const enabled = Boolean(state.enabled)
  const intervalSeconds = Math.max(5, Number(state.intervalSeconds || 15))
  autoRefreshState.value = { enabled, intervalSeconds }
  localStorage.setItem('mosdnsAutoRefresh', JSON.stringify(autoRefreshState.value))
  startAutoRefresh()
}

function loadAutoRefreshState() {
  let saved = null
  try {
    saved = JSON.parse(localStorage.getItem('mosdnsAutoRefresh') || 'null')
  } catch {
    saved = null
  }
  applyAutoRefreshState(saved || DEFAULT_AUTO_REFRESH_STATE)
}

function handleAutoRefreshUpdate(event) {
  applyAutoRefreshState(event?.detail || {})
}

function handleVisibilityChange() {
  startAutoRefresh()
}

function triggerGlobalRefresh() {
  window.dispatchEvent(new CustomEvent('mosdns-log-refresh'))
}

function handleOpenLogFilter(event) {
  const detail = event?.detail || {}
  const text = String(detail.value || '').trim()
  if (!text) {
    return
  }
  activeMainTab.value = 'log-query'
  activeQuerySubTab.value = 'live'
  nextTick(() => {
    window.dispatchEvent(new CustomEvent('mosdns-open-log-filter-ready', {
      detail: {
        value: text,
        exact: Boolean(detail.exact)
      }
    }))
  })
}

function clearTopNotice() {
  if (topNoticeTimerId) {
    window.clearTimeout(topNoticeTimerId)
    topNoticeTimerId = 0
  }
  topNotice.open = false
  topNotice.message = ''
}

function showTopNotice(payload = {}) {
  const message = String(payload?.message || '').trim()
  if (!message) {
    clearTopNotice()
    return
  }
  const tone = String(payload?.tone || '').toLowerCase() === 'error' ? 'error' : 'success'
  const durationMsRaw = Number(payload?.durationMs || 2400)
  const durationMs = Number.isFinite(durationMsRaw) ? Math.min(8000, Math.max(1200, durationMsRaw)) : 2400
  topNotice.tone = tone
  topNotice.message = message
  topNotice.open = true
  if (topNoticeTimerId) {
    window.clearTimeout(topNoticeTimerId)
  }
  topNoticeTimerId = window.setTimeout(() => {
    topNoticeTimerId = 0
    topNotice.open = false
  }, durationMs)
}

function handleTopNoticeEvent(event) {
  showTopNotice(event?.detail || {})
}

async function initializePanelBackground() {
  try {
    const settings = await getJSON('/api/v1/appearance/panel-background')
    await previewPanelBackground(settings)
  } catch {
    // ignore non-critical appearance errors
  }
}

async function initializeTextColors() {
  try {
    const settings = await getJSON('/api/v1/appearance/text-color')
    const normalized = normalizeTextColorSettings(settings || {})
    saveTextColorSettingsToStorage(normalized)
    const theme = document.documentElement.getAttribute('data-theme') || 'light'
    applyTextColorForTheme(theme, normalized)
  } catch {
    // ignore non-critical appearance errors
  }
}

async function initializeButtonColors() {
  try {
    const settings = await getJSON('/api/v1/appearance/button-color')
    const normalized = normalizeButtonColorSettings(settings || {})
    saveButtonColorSettingsToStorage(normalized)
    const theme = document.documentElement.getAttribute('data-theme') || 'light'
    applyButtonColorForTheme(theme, normalized)
  } catch {
    // ignore non-critical appearance errors
  }
}

function onMainTabClick(tabId) {
  activeMainTab.value = tabId
  if (tabId === 'system-control') {
    activeSystemSection.value = 'system-upstream'
  }
  if (tabId === 'data-management') {
    activeDataSubTab.value = 'cache-management'
  }
}

function isSecondaryTabActive(tabId) {
  switch (activeMainTab.value) {
    case 'log-query':
      return activeQuerySubTab.value === tabId
    case 'rules':
      return activeRulesSubTab.value === tabId
    case 'data-management':
      return activeDataSubTab.value === tabId
    case 'system-control':
      return activeSystemSection.value === tabId
    default:
      return false
  }
}

function activateSecondaryTab(tab) {
  switch (activeMainTab.value) {
    case 'log-query':
      activeQuerySubTab.value = tab.id
      break
    case 'rules':
      activeRulesSubTab.value = tab.id
      break
    case 'data-management':
      activeDataSubTab.value = tab.id
      break
    case 'system-control':
      activeSystemSection.value = tab.id
      break
    default:
      break
  }
}

async function restartMosdns() {
  if (restartLoading.value) {
    return
  }
  if (!(await openConfirm('确认重启 MosDNS？', { tone: 'danger' }))) {
    return
  }
  restartLoading.value = true
  try {
    await postJSON('/api/v1/system/restart', { delay_ms: 500 })
    showTopNotice({ message: '已发送重启请求，页面将自动刷新。', tone: 'success', durationMs: 3200 })
    window.setTimeout(() => {
      window.location.reload()
    }, 4000)
  } catch (error) {
    showTopNotice({ message: `重启失败: ${error.message}`, tone: 'error', durationMs: 4200 })
  } finally {
    restartLoading.value = false
  }
}

onMounted(() => {
  initializeAppearance()
  initializePanelBackground()
  initializeTextColors()
  initializeButtonColors()
  loadAutoRefreshState()
  window.addEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  window.addEventListener('mosdns-top-notice', handleTopNoticeEvent)
  window.addEventListener('mosdns-open-log-filter', handleOpenLogFilter)
  document.addEventListener('visibilitychange', handleVisibilityChange)
})

onBeforeUnmount(() => {
  stopAutoRefresh()
  clearTopNotice()
  window.removeEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  window.removeEventListener('mosdns-top-notice', handleTopNoticeEvent)
  window.removeEventListener('mosdns-open-log-filter', handleOpenLogFilter)
  document.removeEventListener('visibilitychange', handleVisibilityChange)
})
</script>

<template>
  <div class="log1-shell">
    <aside class="log1-sidebar">
      <div class="log1-brand">
        <div class="log1-brand-mark">M</div>
        <div class="log1-brand-copy">
          <strong>MosDNS</strong>
        </div>
      </div>

      <nav class="log1-primary-nav">
        <button
          v-for="tab in mainTabs"
          :key="tab.id"
          class="log1-primary-btn"
          :class="{ active: activeMainTab === tab.id }"
          @click="onMainTabClick(tab.id)"
        >
          <span class="log1-primary-icon" aria-hidden="true">{{ tab.icon }}</span>
          <span class="log1-primary-label">{{ formatPrimaryLabel(tab.label) }}</span>
        </button>
        <button
          class="log1-primary-btn log1-primary-btn-refresh-mobile"
          type="button"
          title="刷新当前页面数据"
          aria-label="刷新当前页面数据"
          @click="triggerGlobalRefresh"
        >
          <span class="log1-primary-icon" aria-hidden="true">⟳</span>
          <span class="log1-primary-label">{{ formatPrimaryLabel('刷新') }}</span>
        </button>
        <button
          class="log1-primary-btn log1-primary-btn-refresh-mobile"
          type="button"
          title="重启"
          aria-label="重启"
          :disabled="restartLoading"
          @click="restartMosdns"
        >
          <span class="log1-primary-icon" aria-hidden="true">↻</span>
          <span class="log1-primary-label">{{ restartLoading ? '重启中' : formatPrimaryLabel('重启') }}</span>
        </button>
      </nav>

      <div class="log1-sidebar-tools">
        <button class="log1-primary-btn log1-primary-btn-refresh-desktop" type="button" title="刷新当前页面数据" @click="triggerGlobalRefresh">
          <span class="log1-primary-icon" aria-hidden="true">⟳</span>
          <span class="log1-primary-label">{{ formatPrimaryLabel('刷新') }}</span>
        </button>
        <button class="log1-primary-btn log1-primary-btn-refresh-desktop" type="button" title="重启" :disabled="restartLoading" @click="restartMosdns">
          <span class="log1-primary-icon" aria-hidden="true">↻</span>
          <span class="log1-primary-label">{{ restartLoading ? '重启中' : formatPrimaryLabel('重启') }}</span>
        </button>
      </div>
    </aside>

    <section class="log1-main">
      <transition name="top-inline-notice-fade">
        <div
          v-if="topNotice.open"
          class="top-inline-notice log1-top-inline-notice"
          :class="topNotice.tone === 'error' ? 'error' : 'success'"
          role="status"
          aria-live="polite"
        >
          {{ topNotice.message }}
        </div>
      </transition>

      <div v-if="showTopRail" class="log1-main-head">
        <div v-if="hasSecondaryTabs" class="log1-secondary-bar">
          <nav class="log1-secondary-nav">
            <button
              v-for="tab in currentSecondaryTabs"
              :key="tab.id"
              class="log1-secondary-btn"
              :class="{ active: isSecondaryTabActive(tab.id) }"
              type="button"
              @click="activateSecondaryTab(tab)"
            >
              {{ tab.label }}
            </button>
          </nav>
        </div>
      </div>

      <main class="log1-content" :class="{ 'has-secondary-bar': showTopRail }">
        <section v-if="activeMainTab === 'overview'" class="page-shell">
          <OverviewManager show-system-summary />
        </section>

        <section v-else-if="activeMainTab === 'log-query'" class="page-shell">
          <QueryManager v-if="activeQuerySubTab === 'live'" mode="live" />
          <QueryManager v-else mode="diagnostic" />
        </section>

        <section v-else-if="activeMainTab === 'rules'" class="page-shell">
          <ListManager v-if="activeRulesSubTab === 'list-mgmt'" />
          <RulesManager v-else-if="activeRulesSubTab === 'diversion'" mode="diversion" />
          <RulesManager v-else mode="adguard" />
        </section>

        <section v-else-if="activeMainTab === 'data-management'" class="page-shell">
          <DataManagementManager :mode="activeDataSubTab" />
        </section>

        <section v-else class="page-shell">
          <SystemControlManager :mode="activeSystemSection" />
        </section>
      </main>
    </section>

    <ConfirmBubbleHost />
  </div>
</template>

<script setup>
import { nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON, postJSON } from './api/http'
import ConfirmBubbleHost from './components/ConfirmBubbleHost.vue'
import DataManagementManager from './components/DataManagementManager.vue'
import ListManager from './components/ListManager.vue'
import OverviewManager from './components/OverviewManager.vue'
import QueryManager from './components/QueryManager.vue'
import RulesManager from './components/RulesManager.vue'
import SystemControlManager from './components/SystemControlManager.vue'
import UpstreamManager from './components/UpstreamManager.vue'
import { openConfirm } from './utils/confirm'
import { setError, setSuccess } from './utils/notice'
import { previewPanelBackground } from './utils/panelBackground'
import { applyTextColorForTheme, loadTextColorSettingsFromStorage, normalizeTextColorSettings, saveTextColorSettingsToStorage } from './utils/appearanceTextColor'
import { applyButtonColorForTheme, loadButtonColorSettingsFromStorage, normalizeButtonColorSettings, saveButtonColorSettingsToStorage } from './utils/appearanceButtonColor'

const DEFAULT_AUTO_REFRESH_STATE = { enabled: false, intervalSeconds: 15 }
const OVERVIEW_HISTORY_KEY = 'mosdnsHistory'
const UPSTREAM_STATS_RESET_KEY = 'mosdnsUpstreamStatsResetBaselineV1'

const activeMainTab = ref('overview')
const activeQuerySubTab = ref('live')
const activeRulesSubTab = ref('list-mgmt')

const mainTabs = [
  { id: 'overview', label: '概览' },
  { id: 'log-query', label: '查询日志' },
  { id: 'rules', label: '规则管理' },
  { id: 'data-management', label: '数据管理' },
  { id: 'upstream', label: '上游设置' },
  { id: 'system-control', label: '系统设置' }
]

const querySubTabs = [
  { id: 'live', label: '实时查询' },
  { id: 'diagnostic', label: '诊断抓取' }
]

const rulesSubTabs = [
  { id: 'list-mgmt', label: '本地规则' },
  { id: 'diversion', label: '订阅规则' },
  { id: 'adguard', label: '广告拦截' }
]

const autoRefreshState = ref({
  enabled: false,
  intervalSeconds: 15
})
const topNotice = reactive({
  open: false,
  tone: 'success',
  message: ''
})
const overviewResetting = ref(false)

let autoRefreshTimerId = 0
let topNoticeTimerId = 0

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

async function triggerOverviewReset() {
  if (overviewResetting.value) {
    return
  }
  const confirmed = await openConfirm('将清空概览页全部统计并重启 mosdns，是否继续？', {
    title: '重置统计',
    confirmText: '继续',
    tone: 'danger'
  })
  if (!confirmed) {
    return
  }

  overviewResetting.value = true
  try {
    await postJSON('/api/v1/system/restart', { delay_ms: 500 })
    localStorage.removeItem(OVERVIEW_HISTORY_KEY)
    localStorage.removeItem(UPSTREAM_STATS_RESET_KEY)
    setSuccess('已发送重启请求，页面将自动刷新。')
    window.setTimeout(() => window.location.reload(), 4000)
  } catch (error) {
    setError(`重置统计失败: ${error.message}`)
  } finally {
    overviewResetting.value = false
  }
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
  <div class="app-shell">
    <div class="top-strip">
      <div class="top-strip-head">
        <header class="hero compact">
          <h1>MosDNS 仪表盘</h1>
        </header>
        <div class="top-strip-actions">
          <button
            v-if="activeMainTab === 'overview'"
            class="legacy-main-btn reset-inline-btn"
            type="button"
            :disabled="overviewResetting"
            title="清空概览页全部统计并重启 mosdns"
            @click="triggerOverviewReset"
          >
            {{ overviewResetting ? '重置中...' : '重置统计' }}
          </button>
          <button
            class="legacy-main-btn refresh-inline-btn"
            type="button"
            title="刷新当前页面数据"
            @click="triggerGlobalRefresh"
          >
            ⟳
          </button>
        </div>
      </div>
      <transition name="top-inline-notice-fade">
        <div v-if="topNotice.open" class="top-inline-notice" :class="topNotice.tone === 'error' ? 'error' : 'success'" role="status" aria-live="polite">
          {{ topNotice.message }}
        </div>
      </transition>

      <nav class="legacy-main-nav compact">
        <button
          v-for="tab in mainTabs"
          :key="tab.id"
          class="legacy-main-btn"
          :class="{ active: activeMainTab === tab.id }"
          @click="activeMainTab = tab.id"
        >
          {{ tab.label }}
        </button>
        <div class="nav-inline-actions desktop-only-action">
          <button
            v-if="activeMainTab === 'overview'"
            class="legacy-main-btn reset-inline-btn"
            type="button"
            :disabled="overviewResetting"
            title="清空概览页全部统计并重启 mosdns"
            @click="triggerOverviewReset"
          >
            {{ overviewResetting ? '重置中...' : '重置统计' }}
          </button>
          <button
            class="legacy-main-btn refresh-inline-btn"
            type="button"
            title="刷新当前页面数据"
            @click="triggerGlobalRefresh"
          >
            ⟳
          </button>
        </div>
      </nav>
    </div>

    <main class="main-body">
      <section v-if="activeMainTab === 'overview'" class="page-shell">
        <OverviewManager />
      </section>

      <section v-else-if="activeMainTab === 'log-query'" class="page-shell">
        <div class="page-subnav-strip">
          <nav class="legacy-sub-nav page-subnav-nav">
            <button
              v-for="tab in querySubTabs"
              :key="tab.id"
              class="legacy-sub-btn"
              :class="{ active: activeQuerySubTab === tab.id }"
              @click="activeQuerySubTab = tab.id"
            >
              {{ tab.label }}
            </button>
          </nav>
        </div>
        <QueryManager v-if="activeQuerySubTab === 'live'" mode="live" />
        <QueryManager v-else mode="diagnostic" />
      </section>

      <section v-else-if="activeMainTab === 'rules'" class="page-shell">
        <div class="page-subnav-strip">
          <nav class="legacy-sub-nav page-subnav-nav">
            <button
              v-for="tab in rulesSubTabs"
              :key="tab.id"
              class="legacy-sub-btn"
              :class="{ active: activeRulesSubTab === tab.id }"
              @click="activeRulesSubTab = tab.id"
            >
              {{ tab.label }}
            </button>
          </nav>
        </div>
        <ListManager v-if="activeRulesSubTab === 'list-mgmt'" />
        <RulesManager v-else-if="activeRulesSubTab === 'diversion'" mode="diversion" />
        <RulesManager v-else mode="adguard" />
      </section>

      <section v-else-if="activeMainTab === 'data-management'" class="page-shell">
        <DataManagementManager />
      </section>

      <section v-else-if="activeMainTab === 'upstream'" class="page-shell">
        <UpstreamManager />
      </section>

      <section v-else class="page-shell">
        <SystemControlManager />
      </section>
    </main>

    <ConfirmBubbleHost />
  </div>
</template>

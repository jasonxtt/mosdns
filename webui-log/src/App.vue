<script setup>
import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { getJSON } from './api/http'
import ConfirmBubbleHost from './components/ConfirmBubbleHost.vue'
import DataManagementManager from './components/DataManagementManager.vue'
import ListManager from './components/ListManager.vue'
import OverviewManager from './components/OverviewManager.vue'
import QueryManager from './components/QueryManager.vue'
import RulesManager from './components/RulesManager.vue'
import SystemControlManager from './components/SystemControlManager.vue'
import UpstreamManager from './components/UpstreamManager.vue'
import { previewPanelBackground } from './utils/panelBackground'
import { applyTextColorForTheme, loadTextColorSettingsFromStorage, normalizeTextColorSettings, saveTextColorSettingsToStorage } from './utils/appearanceTextColor'
import { applyButtonColorForTheme, loadButtonColorSettingsFromStorage, normalizeButtonColorSettings, saveButtonColorSettingsToStorage } from './utils/appearanceButtonColor'

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
  applyAutoRefreshState(saved || { enabled: false, intervalSeconds: 15 })
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
  document.addEventListener('visibilitychange', handleVisibilityChange)
})

onBeforeUnmount(() => {
  stopAutoRefresh()
  clearTopNotice()
  window.removeEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  window.removeEventListener('mosdns-top-notice', handleTopNoticeEvent)
  document.removeEventListener('visibilitychange', handleVisibilityChange)
})
</script>

<template>
  <div class="app-shell">
    <div class="top-strip">
      <header class="hero compact">
        <h1>MosDNS 仪表盘</h1>
      </header>
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
        <button
          class="legacy-main-btn refresh-inline-btn"
          type="button"
          title="刷新当前页面数据"
          @click="triggerGlobalRefresh"
        >
          ⟳
        </button>
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

<script setup>
import { onBeforeUnmount, onMounted, ref } from 'vue'
import ConfirmBubbleHost from './components/ConfirmBubbleHost.vue'
import DataManagementManager from './components/DataManagementManager.vue'
import ListManager from './components/ListManager.vue'
import OverviewManager from './components/OverviewManager.vue'
import QueryManager from './components/QueryManager.vue'
import RulesManager from './components/RulesManager.vue'
import SystemControlManager from './components/SystemControlManager.vue'
import UpstreamManager from './components/UpstreamManager.vue'

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

let autoRefreshTimerId = 0

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

onMounted(() => {
  loadAutoRefreshState()
  window.addEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  document.addEventListener('visibilitychange', handleVisibilityChange)
})

onBeforeUnmount(() => {
  stopAutoRefresh()
  window.removeEventListener('mosdns-auto-refresh-update', handleAutoRefreshUpdate)
  document.removeEventListener('visibilitychange', handleVisibilityChange)
})
</script>

<template>
  <div class="app-shell">
    <div class="top-strip">
      <header class="hero compact">
        <h1>MosDNS 仪表盘</h1>
      </header>

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
      </nav>
      <button class="main-refresh-btn" type="button" title="刷新当前页面数据" @click="triggerGlobalRefresh">
        ⟳
      </button>
    </div>

    <main class="main-body">
      <OverviewManager v-if="activeMainTab === 'overview'" />

      <section v-else-if="activeMainTab === 'log-query'" class="panel">
        <nav class="legacy-sub-nav">
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
        <QueryManager v-if="activeQuerySubTab === 'live'" mode="live" />
        <QueryManager v-else mode="diagnostic" />
      </section>

      <section v-else-if="activeMainTab === 'rules'" class="rules-shell">
        <section class="panel">
          <nav class="legacy-sub-nav">
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
        </section>

        <ListManager v-if="activeRulesSubTab === 'list-mgmt'" />
        <RulesManager v-else-if="activeRulesSubTab === 'diversion'" mode="diversion" />
        <RulesManager v-else mode="adguard" />
      </section>

      <DataManagementManager v-else-if="activeMainTab === 'data-management'" />

      <section v-else-if="activeMainTab === 'upstream'" class="upstream-shell">
        <UpstreamManager />
      </section>

      <SystemControlManager v-else />
    </main>

    <ConfirmBubbleHost />
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { useRealtimeMetrics } from '../../composables/useRealtimeMetrics'
import { fetchDashboardWindowStats, type DashboardWindowStat } from '../../services/dashboard'
import { formatCount, formatLatencyMs } from '../../utils/dashboardFormat'
import RealtimeTrendChart from './RealtimeTrendChart.vue'

const {
  metrics,
  initialized,
  warningMessage
} = useRealtimeMetrics({
  pollIntervalMs: 3000,
  windowSize: 40,
  listenGlobalRefresh: true
})

type SeriesKey = 'request' | 'latency'

const seriesState = reactive<Record<SeriesKey, boolean>>({
  request: true,
  latency: true
})

function toggleSeries(key: SeriesKey) {
  seriesState[key] = !seriesState[key]
}

const totalQueriesText = computed(() => formatCount(metrics.totalQueries))
const averageLatencyText = computed(() => formatLatencyMs(metrics.averageLatency))
const currentQueriesText = computed(() => formatCount(metrics.currentQueries))
const currentLatencyText = computed(() => formatLatencyMs(metrics.currentLatency))
const currentTheme = ref<'light' | 'dark'>('light')

const trendCardRef = ref<HTMLElement | null>(null)
const trendTitleTriggerRef = ref<HTMLElement | null>(null)
const totalQueriesLabelRef = ref<HTMLElement | null>(null)
const averageLatencyValueRef = ref<HTMLElement | null>(null)
const popoverOpen = ref(false)
const popoverLoading = ref(false)
const popoverError = ref('')
const popoverTriggerRef = ref<HTMLElement | null>(null)
const popoverPanelRef = ref<HTMLElement | null>(null)
const windowStats = ref<DashboardWindowStat[]>([])
const lastWindowStatsLoadedAt = ref(0)
let closeTimerId = 0

const popoverPosition = reactive({
  top: 0,
  left: 0,
  height: 66,
  scale: 1,
  arrowTop: 30,
  placement: 'right' as 'right' | 'left'
})
let themeObserver: MutationObserver | null = null

const popoverThemeClass = computed(() => currentTheme.value === 'light' ? 'theme-light' : 'theme-dark')

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max)
}

function syncThemeFromDocument() {
  if (typeof document === 'undefined') {
    return
  }
  currentTheme.value = document.documentElement.getAttribute('data-theme') === 'light' ? 'light' : 'dark'
}

function closePopover() {
  if (closeTimerId) {
    window.clearTimeout(closeTimerId)
    closeTimerId = 0
  }
  popoverOpen.value = false
}

async function loadWindowStats() {
  popoverLoading.value = true
  popoverError.value = ''
  try {
    const payload = await fetchDashboardWindowStats()
    windowStats.value = payload.items
    lastWindowStatsLoadedAt.value = Date.now()
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    popoverError.value = `加载时间段统计失败: ${message}`
  } finally {
    popoverLoading.value = false
    if (popoverOpen.value) {
      await nextTick()
      updatePopoverPosition()
    }
  }
}

function updatePopoverPosition() {
  const card = trendCardRef.value
  const trigger = popoverTriggerRef.value
  const panel = popoverPanelRef.value
  const titleTrigger = trendTitleTriggerRef.value
  const totalLabel = totalQueriesLabelRef.value
  const averageLatencyValue = averageLatencyValueRef.value
  if (!trigger || !panel || !titleTrigger || !totalLabel || !averageLatencyValue) {
    return
  }

  const cardRect = card?.getBoundingClientRect()
  const triggerRect = trigger.getBoundingClientRect()
  const titleRect = titleTrigger.getBoundingClientRect()
  const totalLabelRect = totalLabel.getBoundingClientRect()
  const averageLatencyRect = averageLatencyValue.getBoundingClientRect()
  const panelRect = panel.getBoundingClientRect()
  const viewportWidth = window.innerWidth
  const viewportHeight = window.innerHeight
  const mobilePopover = viewportWidth <= 760
  const viewportPadding = 12
  const gap = 5
  const panelWidth = panelRect.width || panel.offsetWidth || 0
  const canPlaceRight = averageLatencyRect.right + gap + panelWidth <= viewportWidth - viewportPadding
  const canPlaceLeft = averageLatencyRect.left - gap - panelWidth >= viewportPadding
  const placement = canPlaceRight || !canPlaceLeft ? 'right' : 'left'
  const rawLeft = placement === 'right'
    ? averageLatencyRect.right + gap
    : averageLatencyRect.left - gap - panelWidth

  const minLeft = cardRect ? Math.max(viewportPadding, cardRect.left + 10) : viewportPadding
  const maxLeft = cardRect
    ? Math.min(viewportWidth - viewportPadding - panelWidth, cardRect.right - panelWidth - 10)
    : viewportWidth - viewportPadding - panelWidth
  const left = clamp(rawLeft, minLeft, Math.max(minLeft, maxLeft))

  const desktopHeight = Math.max(62, totalLabelRect.bottom - titleRect.top)
  const mobileHeight = Math.max(136, panel.scrollHeight || panelRect.height || desktopHeight)
  const maxHeight = Math.max(62, viewportHeight - viewportPadding * 2)
  const targetHeight = clamp(mobilePopover ? mobileHeight : desktopHeight, 62, maxHeight)
  const top = clamp(titleRect.top, viewportPadding, Math.max(viewportPadding, viewportHeight - viewportPadding - targetHeight))
  const arrowAnchor = averageLatencyRect.top + averageLatencyRect.height * 0.5
  const arrowTop = clamp(arrowAnchor - top, 14, targetHeight - 14)
  const scale = mobilePopover ? 1 : clamp(targetHeight / 60, 1, 1.18)

  popoverPosition.top = Math.round(top)
  popoverPosition.left = Math.round(left)
  popoverPosition.height = Math.round(targetHeight)
  popoverPosition.scale = Number(scale.toFixed(3))
  popoverPosition.arrowTop = Math.round(arrowTop)
  popoverPosition.placement = placement
}

function cancelScheduledClose() {
  if (closeTimerId) {
    window.clearTimeout(closeTimerId)
    closeTimerId = 0
  }
}

function scheduleClosePopover() {
  cancelScheduledClose()
  closeTimerId = window.setTimeout(() => {
    closePopover()
  }, 120)
}

async function openPopover(forceReload = false) {
  cancelScheduledClose()
  popoverOpen.value = true
  await nextTick()
  updatePopoverPosition()
  if (forceReload || Date.now() - lastWindowStatsLoadedAt.value > 15000 || windowStats.value.length === 0) {
    void loadWindowStats()
  }
}

function togglePopover() {
  if (popoverOpen.value) {
    closePopover()
    return
  }
  void openPopover(true)
}

function handleTriggerEnter() {
  void openPopover(false)
}

function handleTriggerLeave() {
  scheduleClosePopover()
}

function handlePanelEnter() {
  cancelScheduledClose()
}

function handlePanelLeave() {
  scheduleClosePopover()
}

function handleDocumentPointerDown(event: PointerEvent) {
  if (!popoverOpen.value) {
    return
  }
  const target = event.target as Node | null
  if (!target) {
    return
  }
  if (popoverTriggerRef.value?.contains(target) || popoverPanelRef.value?.contains(target)) {
    return
  }
  closePopover()
}

function handleWindowChange() {
  if (!popoverOpen.value) {
    return
  }
  updatePopoverPosition()
}

function handleDocumentKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && popoverOpen.value) {
    closePopover()
  }
}

onMounted(() => {
  syncThemeFromDocument()
  themeObserver = new MutationObserver(() => {
    syncThemeFromDocument()
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme']
  })
  document.addEventListener('pointerdown', handleDocumentPointerDown)
  document.addEventListener('keydown', handleDocumentKeydown)
  window.addEventListener('resize', handleWindowChange)
  window.addEventListener('scroll', handleWindowChange, true)
})

onBeforeUnmount(() => {
  themeObserver?.disconnect()
  themeObserver = null
  document.removeEventListener('pointerdown', handleDocumentPointerDown)
  document.removeEventListener('keydown', handleDocumentKeydown)
  window.removeEventListener('resize', handleWindowChange)
  window.removeEventListener('scroll', handleWindowChange, true)
})
</script>

<template>
  <section class="dns-overview-shell">
    <article ref="trendCardRef" class="trend-card">
      <section class="trend-metrics">
        <section
          ref="popoverTriggerRef"
          class="trend-hover-zone"
          :class="{ open: popoverOpen }"
          @mouseenter="handleTriggerEnter"
          @mouseleave="handleTriggerLeave"
        >
          <header class="trend-card-header">
          <button
            ref="trendTitleTriggerRef"
            class="trend-title trend-title-trigger"
            :class="{ open: popoverOpen }"
            type="button"
            title="查看 1 小时到 7 天的时间段统计"
            @focus="handleTriggerEnter"
            @blur="handleTriggerLeave"
            @click="togglePopover"
          >
            <span class="trend-icon">▤</span>
            <h3>查询趋势</h3>
            <span class="trend-popover-caret" aria-hidden="true">⌄</span>
          </button>
          </header>
          <div class="kpi-main">
            <article class="kpi-item total-queries">
              <p class="kpi-value">{{ totalQueriesText }}</p>
              <p ref="totalQueriesLabelRef" class="kpi-label">总查询数</p>
            </article>
            <article class="kpi-item average-latency">
              <p ref="averageLatencyValueRef" class="kpi-value accent">{{ averageLatencyText }}</p>
              <p class="kpi-label">平均处理时间</p>
            </article>
          </div>
        </section>

        <aside class="kpi-side">
          <div class="side-item">
            <span class="side-label">当前请求数：</span>
            <span class="side-value">{{ currentQueriesText }}</span>
          </div>
          <div class="side-item">
            <span class="side-label">当前处理时间：</span>
            <span class="side-value accent">{{ currentLatencyText }}</span>
          </div>
        </aside>
      </section>

      <RealtimeTrendChart
        :timestamps="metrics.timestamps"
        :request-counts="metrics.requestCounts"
        :avg-latency-ms="metrics.avgLatencyMs"
        :show-request-series="seriesState.request"
        :show-latency-series="seriesState.latency"
      />

      <div class="series-toggle-row">
        <button
          class="series-toggle-btn request"
          :class="{ selected: seriesState.request }"
          type="button"
          @click="toggleSeries('request')"
        >
          <span class="series-dot"></span>
          请求数
        </button>
        <button
          class="series-toggle-btn latency"
          :class="{ selected: seriesState.latency }"
          type="button"
          @click="toggleSeries('latency')"
        >
          <span class="series-dot"></span>
          平均处理时间
        </button>
      </div>

      <footer class="trend-foot">
        <span v-if="!initialized" class="muted-text">正在加载实时指标...</span>
        <span v-if="warningMessage" class="warn-text">{{ warningMessage }}</span>
      </footer>
    </article>

    <Teleport to="body">
      <Transition name="trend-popover">
        <section
          v-if="popoverOpen"
          ref="popoverPanelRef"
          class="trend-popover-panel"
          :class="[popoverThemeClass, `placement-${popoverPosition.placement}`]"
          @mouseenter="handlePanelEnter"
          @mouseleave="handlePanelLeave"
          :style="{
            top: `${popoverPosition.top}px`,
            left: `${popoverPosition.left}px`,
            height: `${popoverPosition.height}px`,
            '--trend-popover-arrow-top': `${popoverPosition.arrowTop}px`,
            '--trend-popover-scale': `${popoverPosition.scale}`
          }"
        >
          <div v-if="popoverError" class="trend-popover-state error">
            {{ popoverError }}
          </div>
          <div v-else-if="popoverLoading" class="trend-popover-state">
            正在加载时间段统计...
          </div>
          <div v-else-if="windowStats.length === 0" class="trend-popover-state">
            当前还没有足够的审计数据可供统计。
          </div>
          <div v-else class="trend-popover-grid">
            <article
              v-for="item in windowStats"
              :key="item.key"
              class="trend-window-card"
            >
              <h5>{{ item.label }}</h5>
              <div class="trend-window-row">
                <span class="trend-window-meta">请求</span>
                <strong class="trend-window-number">{{ formatCount(item.requestCount) }}</strong>
              </div>
              <div class="trend-window-row">
                <span class="trend-window-meta">平均</span>
                <strong class="trend-window-latency">{{ formatLatencyMs(item.averageLatency) }}</strong>
              </div>
            </article>
          </div>
        </section>
      </Transition>
    </Teleport>
  </section>
</template>

<style scoped>
.dns-overview-shell {
  display: flex;
  flex-direction: column;
  gap: 0;
  width: 100%;
  margin: 0;
}

.trend-card {
  width: 100%;
  border-radius: var(--radius-lg);
  border: 1px solid var(--line);
  background:
    radial-gradient(circle at 7% 0, var(--surface-hover), transparent 46%),
    linear-gradient(165deg, var(--surface-soft-2) 0%, var(--panel) 72%);
  box-shadow: 0 10px 18px rgba(18, 28, 40, 0.09);
  padding: 10px 12px 9px;
  color: var(--ink-0);
}

.trend-hover-zone {
  display: flex;
  flex-direction: column;
  gap: 1px;
  width: fit-content;
  max-width: 100%;
  border-radius: 12px;
}

.trend-hover-zone.open .trend-title-trigger,
.trend-hover-zone:hover .trend-title-trigger {
  color: var(--brand);
}

.trend-card-header {
  display: flex;
  align-items: center;
  justify-content: flex-start;
  margin-bottom: 2px;
}

.trend-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.trend-title-trigger {
  appearance: none;
  border: none;
  background: transparent;
  padding: 0;
  color: var(--ink-0);
  cursor: pointer;
  transition: color 0.16s ease, transform 0.16s ease;
}

.trend-title-trigger:hover,
.trend-title-trigger.open {
  color: var(--brand);
}

.trend-title-trigger:hover .trend-icon,
.trend-title-trigger.open .trend-icon {
  color: var(--brand);
}

.trend-title h3 {
  margin: 0;
  font-size: 0.94rem;
  color: inherit;
}

.trend-popover-caret {
  display: inline-block;
  margin-left: -2px;
  font-size: 0.82rem;
  transform-origin: center;
  transition: transform 0.18s ease;
  opacity: 0.78;
}

.trend-title-trigger.open .trend-popover-caret {
  transform: rotate(180deg);
}

.trend-icon {
  color: var(--ink-0);
  font-size: 0.86rem;
}

.trend-metrics {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
  gap: 12px;
  margin-bottom: 6px;
}

.kpi-main {
  display: inline-flex;
  align-items: flex-start;
  gap: clamp(10px, 1.4vw, 22px);
  padding-left: 34px;
  min-width: 0;
  flex-wrap: nowrap;
}

.kpi-item {
  min-width: 0;
  flex: 0 1 auto;
}

.kpi-item.total-queries {
  min-width: fit-content;
}

.kpi-item.average-latency {
  min-width: fit-content;
}

.kpi-value {
  margin: 0;
  font-size: clamp(1.2rem, 2.2vw, 1.72rem);
  line-height: 1.1;
  font-weight: 700;
  color: var(--ink-0);
}

.kpi-value.accent {
  color: var(--ink-0);
}

.kpi-label {
  margin: 3px 0 0;
  color: var(--ink-1);
  font-size: 0.76rem;
  letter-spacing: 0.02em;
}

.kpi-side {
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: max-content;
  justify-self: end;
}

.side-item {
  display: inline-flex;
  align-items: baseline;
  gap: 2px;
}

.side-label {
  margin: 0;
  color: var(--ink-1);
  font-size: 0.75rem;
  white-space: nowrap;
}

.side-value {
  margin: 0;
  color: var(--ink-0);
  font-size: 0.9rem;
  font-weight: 600;
  white-space: nowrap;
}

.side-value.accent {
  color: var(--ink-0);
}

.series-toggle-row {
  margin-top: 4px;
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
  padding-left: 22px;
}

.series-toggle-btn {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  border-radius: 999px;
  border: 1px solid var(--line);
  background: var(--surface-soft);
  color: var(--ink-1);
  font-size: 0.74rem;
  font-weight: 600;
  line-height: 1;
  padding: 5px 9px;
  cursor: pointer;
  transition: all 0.16s ease;
}

.series-toggle-btn .series-dot {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  display: inline-block;
  opacity: 0.65;
}

.series-toggle-btn.request .series-dot {
  background: #5da8ff;
}

.series-toggle-btn.latency .series-dot {
  background: #40d889;
}

.series-toggle-btn:hover {
  border-color: var(--brand);
}

.series-toggle-btn.selected {
  background: var(--surface-active);
  color: var(--ink-0);
  border-color: var(--brand);
}

.series-toggle-btn.selected .series-dot {
  opacity: 1;
}

.trend-foot {
  margin-top: 4px;
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  align-items: center;
  min-height: 16px;
}

.muted-text {
  color: var(--ink-1);
  font-size: 0.74rem;
}

.warn-text {
  color: var(--warn);
  font-size: 0.74rem;
}

.trend-popover-panel {
  position: fixed;
  z-index: 1200;
  width: min(402px, calc(100vw - 24px));
  max-height: calc(100vh - 24px);
  border-radius: 11px;
  border: 1px solid var(--trend-popover-border);
  background: var(--trend-popover-bg);
  color: var(--trend-popover-ink);
  box-shadow: var(--trend-popover-shadow);
  backdrop-filter: var(--trend-popover-backdrop-filter, blur(13px) saturate(145%));
  -webkit-backdrop-filter: var(--trend-popover-backdrop-filter, blur(13px) saturate(145%));
  overflow-x: hidden;
  overflow-y: auto;
  overscroll-behavior: contain;
  padding: calc(4px * var(--trend-popover-scale, 1)) calc(5px * var(--trend-popover-scale, 1));
}

.trend-popover-panel.theme-light {
  --trend-popover-bg: #ffffff;
  --trend-popover-border: rgba(148, 163, 184, 0.34);
  --trend-popover-ink: #1f2937;
  --trend-popover-muted: #5f6b7a;
  --trend-popover-accent: #0f5f55;
  --trend-popover-shadow: 0 18px 34px rgba(15, 23, 42, 0.12), 0 8px 18px rgba(15, 23, 42, 0.08), inset 0 1px 0 rgba(255, 255, 255, 0.92);
  --trend-popover-state-bg: rgba(248, 250, 252, 0.98);
  --trend-popover-window-bg: linear-gradient(180deg, rgba(250, 252, 251, 0.98) 0%, rgba(244, 247, 245, 0.98) 100%);
  --trend-popover-divider: rgba(148, 163, 184, 0.18);
  --trend-popover-backdrop-filter: none;
}

.trend-popover-panel.theme-dark {
  --trend-popover-bg: linear-gradient(180deg, rgba(48, 62, 82, 0.99) 0%, rgba(39, 52, 70, 0.99) 100%);
  --trend-popover-border: rgba(96, 165, 250, 0.24);
  --trend-popover-ink: #eef4ff;
  --trend-popover-muted: #aebcd0;
  --trend-popover-accent: #68e0a8;
  --trend-popover-shadow: 0 18px 34px rgba(0, 0, 0, 0.3), inset 0 1px 0 rgba(255, 255, 255, 0.05);
  --trend-popover-state-bg: rgba(19, 31, 45, 0.5);
  --trend-popover-window-bg: linear-gradient(180deg, rgba(255, 255, 255, 0.04) 0%, rgba(255, 255, 255, 0.015) 100%);
  --trend-popover-divider: rgba(96, 165, 250, 0.16);
  --trend-popover-backdrop-filter: blur(13px) saturate(145%);
}

.trend-popover-panel::before {
  content: "";
  position: absolute;
  top: calc(var(--trend-popover-arrow-top) - 7px);
  width: 14px;
  height: 14px;
  border-top: 1px solid var(--trend-popover-border);
  border-left: 1px solid var(--trend-popover-border);
  background: var(--trend-popover-bg);
  transform: rotate(45deg);
}

.trend-popover-panel.placement-right::before {
  left: -8px;
}

.trend-popover-panel.placement-left::before {
  right: -8px;
  transform: rotate(225deg);
}

.trend-popover-state {
  border-radius: 8px;
  background: var(--trend-popover-state-bg);
  color: var(--trend-popover-muted);
  font-size: 0.7rem;
  padding: 7px 8px;
  box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.45);
}

.trend-popover-state.error {
  background: rgba(248, 113, 113, 0.12);
  color: #b42318;
}

.trend-popover-grid {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 0;
  height: 100%;
  align-items: stretch;
}

.trend-window-card {
  display: flex;
  flex-direction: column;
  justify-content: center;
  min-width: 0;
  padding: calc(3px * var(--trend-popover-scale, 1)) calc(5px * var(--trend-popover-scale, 1)) calc(4px * var(--trend-popover-scale, 1));
  border-right: 1px solid var(--trend-popover-divider);
  background: var(--trend-popover-window-bg);
}

.trend-window-card:last-child {
  border-right: none;
}

.trend-window-card h5 {
  margin: 0;
  color: var(--trend-popover-muted);
  font-size: calc(0.59rem * var(--trend-popover-scale, 1));
  font-weight: 600;
}

.trend-window-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 4px;
  margin-top: calc(2px * var(--trend-popover-scale, 1));
}

.trend-window-meta {
  color: var(--trend-popover-muted);
  font-size: calc(0.55rem * var(--trend-popover-scale, 1));
  line-height: 1.1;
  white-space: nowrap;
}

.trend-window-number {
  color: var(--trend-popover-ink);
  font-size: calc(0.68rem * var(--trend-popover-scale, 1));
  line-height: 1.08;
  font-weight: 700;
  letter-spacing: -0.01em;
  text-align: right;
  white-space: nowrap;
}

.trend-window-latency {
  color: var(--trend-popover-accent);
  font-size: calc(0.62rem * var(--trend-popover-scale, 1));
  line-height: 1.08;
  font-weight: 700;
  letter-spacing: -0.01em;
  text-align: right;
  white-space: nowrap;
}

.trend-popover-enter-active,
.trend-popover-leave-active {
  transition: opacity 0.18s ease, transform 0.22s ease;
}

.trend-popover-enter-from,
.trend-popover-leave-to {
  opacity: 0;
  transform: translateY(-4px) scale(0.98);
}

.trend-popover-panel.placement-right.trend-popover-enter-from,
.trend-popover-panel.placement-right.trend-popover-leave-to {
  transform: translateX(-8px) scale(0.98);
}

.trend-popover-panel.placement-left.trend-popover-enter-from,
.trend-popover-panel.placement-left.trend-popover-leave-to {
  transform: translateX(8px) scale(0.98);
}

@media (max-width: 1100px) {
  .kpi-main {
    display: flex;
    flex-wrap: nowrap;
    justify-content: flex-start;
    gap: clamp(8px, 1.8vw, 16px);
    padding-left: 20px;
  }

  .kpi-item {
    flex: 0 0 auto;
  }

  .kpi-item.total-queries {
    min-width: fit-content;
  }

  .kpi-item.average-latency {
    min-width: fit-content;
  }

  .kpi-side {
    justify-self: end;
  }

  .series-toggle-row {
    padding-left: 18px;
  }
}

@media (max-width: 760px) {
  .trend-hover-zone {
    width: 100%;
  }

  .trend-card-header {
    gap: 8px;
  }

  .dns-overview-header {
    flex-wrap: wrap;
  }

  .trend-metrics {
    grid-template-columns: minmax(0, 1fr) auto;
    gap: 10px;
  }

  .kpi-value {
    font-size: clamp(1.08rem, 4.4vw, 1.44rem);
  }

  .kpi-label {
    font-size: 0.72rem;
  }

  .side-label,
  .side-value {
    font-size: 0.76rem;
  }

  .trend-popover-panel {
    width: min(304px, calc(100vw - 18px));
    padding: 6px;
  }

  .series-toggle-row {
    padding-left: 12px;
  }

  .trend-popover-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 4px;
    height: auto;
  }

  .trend-window-card {
    padding: 7px 9px 8px;
    border: 1px solid var(--trend-popover-divider);
    border-radius: 8px;
    background: var(--trend-popover-window-bg);
  }
}

@media (max-width: 420px) {
  .trend-popover-panel {
    width: min(276px, calc(100vw - 16px));
  }

  .trend-popover-grid {
    grid-template-columns: 1fr;
  }
}
</style>

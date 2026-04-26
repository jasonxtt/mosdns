<script setup lang="ts">
import { computed, reactive } from 'vue'
import { useRealtimeMetrics } from '../../composables/useRealtimeMetrics'
import { formatCount, formatLatencyMs } from '../../utils/dashboardFormat'
import RealtimeTrendChart from './RealtimeTrendChart.vue'

const {
  metrics,
  initialized,
  warningMessage,
  lastUpdatedText
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
</script>

<template>
  <section class="dns-overview-shell">
    <article class="trend-card">
      <header class="trend-card-header">
        <div class="trend-title">
          <span class="trend-icon">▤</span>
          <h3>查询趋势</h3>
        </div>
      </header>

      <section class="trend-metrics">
        <div class="kpi-main">
          <article class="kpi-item">
            <p class="kpi-value">{{ totalQueriesText }}</p>
            <p class="kpi-label">总查询数</p>
          </article>
          <article class="kpi-item">
            <p class="kpi-value accent">{{ averageLatencyText }}</p>
            <p class="kpi-label">平均处理时间</p>
          </article>
        </div>

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
        <span v-else class="muted-text">最近更新：{{ lastUpdatedText }}</span>
        <span v-if="warningMessage" class="warn-text">{{ warningMessage }}</span>
      </footer>
    </article>
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
  border-radius: 14px;
  border: 1px solid var(--line);
  background:
    radial-gradient(circle at 7% 0, var(--surface-hover), transparent 46%),
    linear-gradient(165deg, var(--surface-soft-2) 0%, var(--panel) 72%);
  box-shadow: 0 10px 18px rgba(18, 28, 40, 0.09);
  padding: 10px 12px 9px;
  color: var(--ink-0);
}

.trend-card-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 6px;
}

.trend-title {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}

.trend-title h3 {
  margin: 0;
  font-size: 0.94rem;
  color: var(--ink-0);
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
  margin-bottom: 3px;
}

.kpi-main {
  display: flex;
  align-items: flex-start;
  gap: 20px;
}

.kpi-item {
  min-width: 150px;
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
  justify-content: space-between;
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

@media (max-width: 1100px) {
  .kpi-main {
    display: flex;
    flex-wrap: nowrap;
    justify-content: flex-start;
    gap: 6px;
  }

  .kpi-item {
    min-width: 118px;
    flex: 0 0 auto;
  }

  .kpi-side {
    justify-self: end;
  }
}

@media (max-width: 760px) {
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
}
</style>

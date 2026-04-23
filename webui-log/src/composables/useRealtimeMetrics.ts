import { onBeforeUnmount, onMounted, reactive, ref } from 'vue'
import { fetchAuditCapacity, fetchAuditStatus, fetchDashboardStats, fetchRecentAuditLogs } from '../services/dashboard'
import type { DashboardAuditLog, DashboardMetrics } from '../types/dashboard'

interface UseRealtimeMetricsOptions {
  pollIntervalMs?: number
  windowSize?: number
  listenGlobalRefresh?: boolean
}

const DEFAULT_POLL_INTERVAL_MS = 3000
const DEFAULT_WINDOW_SIZE = 40
const DEFAULT_LOG_SAMPLE_LIMIT = 160

function formatTimelineLabel(date: Date): string {
  return date.toLocaleTimeString('zh-CN', {
    hour12: false,
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit'
  })
}

export function useRealtimeMetrics(options: UseRealtimeMetricsOptions = {}) {
  const pollIntervalMs = Math.max(1000, Number(options.pollIntervalMs || DEFAULT_POLL_INTERVAL_MS))
  const windowSize = Math.max(30, Number(options.windowSize || DEFAULT_WINDOW_SIZE))
  const listenGlobalRefresh = options.listenGlobalRefresh !== false

  const metrics = reactive<DashboardMetrics>({
    timestamps: [],
    requestCounts: [],
    avgLatencyMs: [],
    totalQueries: 0,
    averageLatency: 0,
    currentQueries: 0,
    currentLatency: 0
  })

  const isRunning = ref(true)
  const initialized = ref(false)
  const warningMessage = ref('')
  const lastUpdatedText = ref('--')

  const inFlight = ref(false)
  const auditCapacity = ref(0)
  let pollTimerId = 0
  let previousTotalQueries: number | null = null
  let previousTopLogKey: string | null = null

  function buildLogKey(log: DashboardAuditLog): string {
    const traceId = String(log.trace_id || '').trim()
    if (traceId) {
      return traceId
    }
    return [
      String(log.query_time || ''),
      String(log.client_ip || ''),
      String(log.query_name || ''),
      String(log.query_type || '')
    ].join('|')
  }

  function appendPoint(timestamp: string, requestCount: number, avgLatencyMs: number) {
    metrics.timestamps.push(timestamp)
    metrics.requestCounts.push(requestCount)
    metrics.avgLatencyMs.push(avgLatencyMs)

    while (metrics.timestamps.length > windowSize) {
      metrics.timestamps.shift()
    }
    while (metrics.requestCounts.length > windowSize) {
      metrics.requestCounts.shift()
    }
    while (metrics.avgLatencyMs.length > windowSize) {
      metrics.avgLatencyMs.shift()
    }
  }

  async function refreshMetrics() {
    if (inFlight.value) {
      return
    }

    inFlight.value = true
    try {
      const [statsResult, statusResult, capacityResult, logsResult] = await Promise.allSettled([
        fetchDashboardStats(),
        fetchAuditStatus(),
        auditCapacity.value > 0 ? Promise.resolve(auditCapacity.value) : fetchAuditCapacity(),
        fetchRecentAuditLogs(DEFAULT_LOG_SAMPLE_LIMIT)
      ])

      if (statusResult.status === 'fulfilled') {
        isRunning.value = statusResult.value
      }

      if (capacityResult.status === 'fulfilled') {
        auditCapacity.value = Number(capacityResult.value || 0)
      }

      if (statsResult.status !== 'fulfilled') {
        throw statsResult.reason
      }

      const stats = statsResult.value
      const totalQueries = Number(stats.totalQueries || 0)
      const averageLatency = Number(stats.averageLatency || 0)
      const fallbackCurrentQueries = previousTotalQueries === null
        ? 0
        : totalQueries >= previousTotalQueries
          ? totalQueries - previousTotalQueries
          : totalQueries

      let currentQueries = fallbackCurrentQueries
      let currentLatency = averageLatency

      if (logsResult.status === 'fulfilled') {
        const logs = logsResult.value
        if (logs.length > 0) {
          const newest = logs[0]
          const newestKey = buildLogKey(newest)
          currentLatency = Number(newest?.duration_ms || averageLatency)

          if (previousTopLogKey !== null) {
            let deltaFromLogs = 0
            for (const log of logs) {
              if (buildLogKey(log) === previousTopLogKey) {
                break
              }
              deltaFromLogs += 1
            }
            currentQueries = deltaFromLogs
          }

          previousTopLogKey = newestKey
        }
      }

      if (currentQueries === 0 && fallbackCurrentQueries > 0) {
        currentQueries = fallbackCurrentQueries
      }

      previousTotalQueries = totalQueries

      metrics.totalQueries = totalQueries
      metrics.averageLatency = averageLatency
      metrics.currentQueries = currentQueries
      metrics.currentLatency = currentLatency

      appendPoint(formatTimelineLabel(new Date()), currentQueries, currentLatency)
      warningMessage.value = ''
      lastUpdatedText.value = new Date().toLocaleString('zh-CN', { hour12: false })
      initialized.value = true
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error)
      warningMessage.value = `实时数据刷新失败: ${message}`
      console.error('[DnsOverviewCard] polling failed', error)
    } finally {
      inFlight.value = false
    }
  }

  function stopPolling() {
    if (pollTimerId) {
      window.clearInterval(pollTimerId)
      pollTimerId = 0
    }
  }

  function startPolling() {
    stopPolling()
    pollTimerId = window.setInterval(() => {
      void refreshMetrics()
    }, pollIntervalMs)
  }

  function handleGlobalRefresh() {
    void refreshMetrics()
  }

  onMounted(() => {
    void refreshMetrics()
    startPolling()
    if (listenGlobalRefresh) {
      window.addEventListener('mosdns-log-refresh', handleGlobalRefresh)
    }
  })

  onBeforeUnmount(() => {
    stopPolling()
    if (listenGlobalRefresh) {
      window.removeEventListener('mosdns-log-refresh', handleGlobalRefresh)
    }
  })

  return {
    metrics,
    isRunning,
    initialized,
    warningMessage,
    lastUpdatedText,
    refreshMetrics
  }
}

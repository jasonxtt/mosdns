import type {
  AuditCapacityResponse,
  AuditStatusResponse,
  DashboardAuditLog,
  DashboardAuditLogsResponse,
  DashboardStatsResponse
} from '../types/dashboard'

export interface DashboardStats {
  totalQueries: number
  averageLatency: number
}

async function requestJSON<T>(url: string): Promise<T> {
  const response = await fetch(url)
  if (!response.ok) {
    let message = `HTTP ${response.status} ${response.statusText}`
    try {
      const payload = await response.json()
      if (payload?.error) {
        message = String(payload.error)
      }
    } catch {
      try {
        const text = await response.text()
        if (text) {
          message = text
        }
      } catch {
        // ignore secondary parse errors
      }
    }
    throw new Error(message)
  }
  return response.json() as Promise<T>
}

export async function fetchDashboardStats(): Promise<DashboardStats> {
  const payload = await requestJSON<DashboardStatsResponse>('/api/v2/audit/stats')
  return {
    totalQueries: Number(payload?.total_queries || 0),
    averageLatency: Number(payload?.average_duration_ms || 0)
  }
}

export async function fetchAuditStatus(): Promise<boolean> {
  const payload = await requestJSON<AuditStatusResponse>('/api/v1/audit/status')
  return Boolean(payload?.capturing)
}

export async function fetchAuditCapacity(): Promise<number> {
  const payload = await requestJSON<AuditCapacityResponse>('/api/v1/audit/capacity')
  return Number(payload?.capacity || 0)
}

export async function fetchRecentAuditLogs(limit = 120): Promise<DashboardAuditLog[]> {
  const safeLimit = Math.max(1, Math.min(500, Number(limit || 120)))
  const payload = await requestJSON<DashboardAuditLogsResponse>(`/api/v2/audit/logs?page=1&limit=${safeLimit}`)
  return Array.isArray(payload?.logs) ? payload.logs : []
}

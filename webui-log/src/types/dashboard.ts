export interface DashboardMetrics {
  timestamps: string[]
  requestCounts: number[]
  avgLatencyMs: number[]
  totalQueries: number
  averageLatency: number
  currentQueries: number
  currentLatency: number
}

export interface DashboardStatsResponse {
  total_queries?: number
  average_duration_ms?: number
}

export interface DashboardWindowStatItemResponse {
  key?: string
  label?: string
  window_seconds?: number
  request_count?: number
  average_duration_ms?: number
  complete?: boolean
  coverage_start?: string
}

export interface DashboardWindowStatsResponse {
  generated_at?: string
  items?: DashboardWindowStatItemResponse[]
}

export interface AuditStatusResponse {
  capturing?: boolean
}

export interface AuditCapacityResponse {
  capacity?: number
}

export interface DashboardAuditLog {
  trace_id?: string
  query_time?: string
  query_name?: string
  query_type?: string
  client_ip?: string
  duration_ms?: number
}

export interface DashboardAuditLogsResponse {
  logs?: DashboardAuditLog[]
}

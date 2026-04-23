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

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
  query_class?: string
  client_ip?: string
  duration_ms?: number
  response_code?: string
  response_flags?: {
    AA?: boolean
    TC?: boolean
    RA?: boolean
    aa?: boolean
    tc?: boolean
    ra?: boolean
  }
  answers?: Array<{
    type?: string
    ttl?: number
    data?: string
  }>
  answer_details_status?: 'complete' | 'raw_rdata' | 'decode_error' | string
  answer_decode_error?: string
  domain_set?: string
  effective_tag?: string
  matched_group?: string
  final_sequence?: string
  final_upstream?: string
  upstream_targets?: string
  selected_upstream?: string
  matched_rule_source?: string
}

export interface DashboardAuditLogsResponse {
  logs?: DashboardAuditLog[]
}

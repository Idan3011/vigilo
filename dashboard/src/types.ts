export interface SummaryResponse {
  sessions: number
  total_calls: number
  errors: number
  reads: number
  writes: number
  execs: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cost_usd: number
  total_duration_us: number
  projects: string[]
}

export interface SessionItem {
  id: string
  server: string
  date: string
  project: string | null
  branch: string | null
  call_count: number
  duration_us: number
  cost_usd: number
  error_count: number
  session_ids: string[]
}

export interface StatsResponse {
  counts: Counts
  models: ModelStats[]
  tools: ToolCount[]
  files: FileCount[]
  projects: ProjectStats[]
  timeline: TimelineDay[]
}

export interface Counts {
  total: number
  reads: number
  writes: number
  execs: number
  errors: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cost_usd: number
  total_duration_us: number
}

export interface ModelStats {
  model: string
  calls: number
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cost_usd: number
}

export interface ToolCount {
  tool: string
  count: number
  error_count: number
}

export interface FileCount {
  file: string
  count: number
}

export interface ProjectStats {
  name: string
  count: number
  reads: number
  writes: number
  execs: number
}

export interface TimelineDay {
  date: string
  cost_usd: number
  input_tokens: number
  output_tokens: number
  reads: number
  writes: number
  execs: number
  errors: number
}

export interface EventItem {
  id: string
  timestamp: string
  session_id: string
  server: string
  tool: string
  risk: 'read' | 'write' | 'exec' | 'unknown'
  duration_us: number
  is_error: boolean
  project: string | null
  branch: string | null
  arg_display: string
  input_tokens: number | null
  output_tokens: number | null
  cache_read_tokens: number | null
  cache_write_tokens: number | null
  model: string | null
  error_message: string | null
}

export interface ErrorsResponse {
  total_calls: number
  error_count: number
  by_tool: ToolErrorCount[]
  recent_errors: EventItem[]
  truncated: boolean
}

export interface ToolErrorCount {
  tool: string
  count: number
}

export type DateRange = '24h' | '7d' | '30d' | 'all'

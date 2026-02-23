import type {
  SummaryResponse,
  SessionItem,
  StatsResponse,
  EventItem,
  ErrorsResponse,
} from '@/types'

const BASE = window.location.origin

type Params = Record<string, string | number | undefined>

async function get<T>(path: string, params?: Params): Promise<T> {
  const url = new URL(path, BASE)
  if (params) {
    for (const [k, v] of Object.entries(params)) {
      if (v !== undefined) url.searchParams.set(k, String(v))
    }
  }
  const res = await fetch(url.toString())
  if (!res.ok) throw new Error(`${res.status} ${res.statusText}`)
  return res.json()
}

export const api = {
  summary: () => get<SummaryResponse>('/api/summary'),
  sessions: (params?: Params) => get<SessionItem[]>('/api/sessions', params),
  stats: (params?: Params) => get<StatsResponse>('/api/stats', params),
  events: (params?: Params) => get<EventItem[]>('/api/events', params),
  errors: (params?: Params) => get<ErrorsResponse>('/api/errors', params),
}

function localDateString(d: Date): string {
  const y = d.getFullYear()
  const m = String(d.getMonth() + 1).padStart(2, '0')
  const day = String(d.getDate()).padStart(2, '0')
  return `${y}-${m}-${day}`
}

export function dateRangeParams(range: string): Params {
  if (range === 'all') return {}
  const days = range === '24h' ? 1 : range === '7d' ? 7 : 30
  const since = new Date()
  since.setDate(since.getDate() - days)
  return { since: localDateString(since) }
}

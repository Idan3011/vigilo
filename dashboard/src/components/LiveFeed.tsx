import { useRef, useEffect, useState, useMemo } from 'react'
import type { EventItem } from '@/types'

const RISK_PILL: Record<string, string> = {
  read: 'bg-cyan-900/50 text-cyan-400',
  write: 'bg-yellow-900/50 text-yellow-400',
  exec: 'bg-red-900/50 text-red-400',
}

const SERVER_COLORS: Record<string, string> = {
  cursor: 'text-purple-400',
  'claude-code': 'text-blue-400',
  vigilo: 'text-emerald-400',
}

function fmtTime(ts: string): string {
  return ts.slice(11, 19) || ts
}

function fmtDuration(us: number): string {
  if (us < 1_000) return `${us}µs`
  if (us < 1_000_000) return `${(us / 1_000).toFixed(0)}ms`
  return `${(us / 1_000_000).toFixed(1)}s`
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${Math.floor(n / 1_000)}K`
  return String(n)
}

function serverLabel(s: string): string {
  if (s === 'claude-code') return 'claude'
  if (s === 'vigilo') return 'mcp'
  return s
}

interface Props {
  events: EventItem[]
  height: number
}

type ServerFilter = 'all' | 'cursor' | 'claude-code' | 'vigilo'
type SortKey = 'time' | 'tokens' | 'duration'
type SortDir = 'asc' | 'desc'

const RISK_CYCLE: (string | null)[] = [null, 'read', 'write', 'exec']

const FILTER_BUTTONS: { key: ServerFilter; label: string; color: string; activeColor: string }[] = [
  { key: 'all', label: 'All', color: 'text-gray-500', activeColor: 'bg-gray-700 text-gray-200' },
  { key: 'claude-code', label: 'Claude', color: 'text-blue-400/60', activeColor: 'bg-blue-500/20 text-blue-400' },
  { key: 'cursor', label: 'Cursor', color: 'text-purple-400/60', activeColor: 'bg-purple-500/20 text-purple-400' },
  { key: 'vigilo', label: 'MCP', color: 'text-emerald-400/60', activeColor: 'bg-emerald-500/20 text-emerald-400' },
]

function compareEvents(a: EventItem, b: EventItem, key: SortKey): number {
  switch (key) {
    case 'time': return a.timestamp.localeCompare(b.timestamp)
    case 'tokens': {
      const ta = (a.input_tokens ?? 0) + (a.output_tokens ?? 0)
      const tb = (b.input_tokens ?? 0) + (b.output_tokens ?? 0)
      return ta - tb
    }
    case 'duration': return a.duration_us - b.duration_us
  }
}

export default function LiveFeed({ events, height }: Props) {
  const [collapsed, setCollapsed] = useState(false)
  const [serverFilter, setServerFilter] = useState<ServerFilter>('all')
  const [riskFilter, setRiskFilter] = useState<string | null>(null)
  const [toolFilter, setToolFilter] = useState<string | null>(null)
  const [sortKey, setSortKey] = useState<SortKey | null>(null)
  const [sortDir, setSortDir] = useState<SortDir>('desc')
  const scrollRef = useRef<HTMLDivElement>(null)

  // Unique tool names from all events for cycling
  const toolNames = useMemo(() => {
    const set = new Set(events.map(e => e.tool))
    return Array.from(set).sort()
  }, [events])

  const cycleRisk = () => {
    const idx = RISK_CYCLE.indexOf(riskFilter)
    setRiskFilter(RISK_CYCLE[(idx + 1) % RISK_CYCLE.length])
  }

  const cycleTool = () => {
    if (toolNames.length === 0) return
    if (toolFilter === null) {
      setToolFilter(toolNames[0])
    } else {
      const idx = toolNames.indexOf(toolFilter)
      if (idx === -1 || idx === toolNames.length - 1) {
        setToolFilter(null)
      } else {
        setToolFilter(toolNames[idx + 1])
      }
    }
  }

  // Pipeline: server → risk → tool → sort
  const filtered = useMemo(() => {
    let out = events
    if (serverFilter !== 'all') out = out.filter(e => e.server === serverFilter)
    if (riskFilter) out = out.filter(e => e.risk === riskFilter)
    if (toolFilter) out = out.filter(e => e.tool === toolFilter)
    return out
  }, [events, serverFilter, riskFilter, toolFilter])

  const sorted = useMemo(() => {
    if (!sortKey) return filtered
    const out = filtered.slice().sort((a, b) => compareEvents(a, b, sortKey))
    return sortDir === 'desc' ? out.reverse() : out
  }, [filtered, sortKey, sortDir])

  const toggleSort = (key: SortKey) => {
    if (sortKey === key) {
      if (sortDir === 'desc') setSortDir('asc')
      else { setSortKey(null); setSortDir('desc') }
    } else {
      setSortKey(key)
      setSortDir('desc')
    }
  }

  useEffect(() => {
    if (scrollRef.current && !sortKey) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight
    }
  }, [filtered, sortKey])

  const anyFilter = riskFilter || toolFilter
  const filterCount = filtered.length

  return (
    <div className="border-t border-gray-800 bg-gray-900/50">
      <div className="flex items-center justify-between px-4 py-1.5">
        <button
          onClick={() => setCollapsed(!collapsed)}
          className="flex items-center gap-2 text-xs text-gray-400 hover:text-gray-200 transition-colors"
        >
          <span className="flex items-center gap-2 font-semibold uppercase tracking-wider">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-emerald-400 animate-pulse" />
            Live Feed
            {filtered.length > 0 && (
              <span className="bg-gray-700 text-gray-300 text-xs px-1.5 py-0.5 rounded-full font-normal">
                {filterCount}
              </span>
            )}
          </span>
          <span className="text-gray-500">{collapsed ? '▲' : '▼'}</span>
        </button>
        {!collapsed && (
          <div className="flex items-center gap-2">
            {anyFilter && (
              <button
                onClick={() => { setRiskFilter(null); setToolFilter(null) }}
                className="text-[10px] text-gray-500 hover:text-gray-300 transition-colors"
              >
                clear filters
              </button>
            )}
            <div className="flex items-center gap-0.5 bg-gray-800/50 rounded p-0.5">
              {FILTER_BUTTONS.map(f => (
                <button
                  key={f.key}
                  onClick={() => setServerFilter(f.key)}
                  className={`px-2 py-0.5 text-[10px] font-semibold rounded transition-colors ${
                    serverFilter === f.key ? f.activeColor : `${f.color} hover:text-gray-300`
                  }`}
                >
                  {f.label}
                </button>
              ))}
            </div>
          </div>
        )}
      </div>

      {!collapsed && (
        <>
        <div className="flex items-center gap-2 px-4 py-1 text-[9px] font-mono uppercase tracking-wider border-b border-gray-800/50 select-none">
          <SortHeader k="time" label="Time" className="w-16 shrink-0" sortKey={sortKey} sortDir={sortDir} onSort={toggleSort} />
          <span className="w-12 shrink-0 text-gray-600">Server</span>
          <FilterHeader label="Risk" value={riskFilter?.toUpperCase() ?? null} className="w-16 shrink-0" onClick={cycleRisk} />
          <FilterHeader label="Tool" value={toolFilter} className="w-24 shrink-0" onClick={cycleTool} />
          <span className="flex-1 min-w-0 text-gray-600">Argument</span>
          <span className="w-28 shrink-0 text-right text-gray-600">Model</span>
          <SortHeader k="tokens" label="Tokens" className="w-20 shrink-0 text-right" sortKey={sortKey} sortDir={sortDir} onSort={toggleSort} />
          <SortHeader k="duration" label="Dur" className="w-16 shrink-0 text-right" sortKey={sortKey} sortDir={sortDir} onSort={toggleSort} />
        </div>
        <div ref={scrollRef} style={{ height }} className="overflow-y-auto px-4 pb-2">
          {sorted.length === 0 ? (
            <div className="flex items-center justify-center h-full text-gray-600 text-xs">
              {events.length === 0 ? 'Waiting for events...' : 'No events match filter'}
            </div>
          ) : (
            sorted.map((e) => (
              <div
                key={e.id}
                className="flex items-center gap-2 py-0.5 text-[11px] font-mono w-full"
              >
                <span className="text-gray-600 w-16 shrink-0">{fmtTime(e.timestamp)}</span>
                <span className={`w-12 shrink-0 font-bold ${SERVER_COLORS[e.server] ?? 'text-gray-500'}`}>
                  {serverLabel(e.server)}
                </span>
                <span className="w-16 shrink-0">
                  <span className={`font-bold px-2 py-0.5 rounded text-[10px] font-mono ${RISK_PILL[e.risk] ?? 'text-gray-500'}`}>
                    {e.risk.toUpperCase()}
                  </span>
                </span>
                <span className="text-gray-300 font-semibold w-24 shrink-0 truncate">
                  {e.is_error && <span className="text-red-500 mr-1">ERR</span>}
                  {e.tool}
                </span>
                <span className="text-gray-500 truncate flex-1 min-w-0">{e.arg_display}</span>
                <span className="w-28 shrink-0 text-gray-600 truncate text-right">
                  {e.model ?? <span className="text-gray-700">—</span>}
                </span>
                <span className="w-20 shrink-0 text-right">
                  {e.input_tokens != null || e.output_tokens != null ? (
                    <>
                      <span className="text-cyan-400/70">{e.input_tokens != null ? fmtTokens(e.input_tokens) : '—'}</span>
                      <span className="text-gray-600">↑</span>
                      <span className="text-purple-400/70">{e.output_tokens != null ? fmtTokens(e.output_tokens) : '—'}</span>
                      <span className="text-gray-600">↓</span>
                    </>
                  ) : (
                    <span className="text-gray-700">—</span>
                  )}
                </span>
                <span className="w-16 shrink-0 text-right">
                  {e.duration_us > 0 ? (
                    <span className="text-gray-600">{fmtDuration(e.duration_us)}</span>
                  ) : (
                    <span className="text-gray-700">—</span>
                  )}
                </span>
              </div>
            ))
          )}
        </div>
        </>
      )}
    </div>
  )
}

function SortHeader({ k, label, className, sortKey, sortDir, onSort }: {
  k: SortKey
  label: string
  className: string
  sortKey: SortKey | null
  sortDir: SortDir
  onSort: (key: SortKey) => void
}) {
  const active = sortKey === k
  const arrow = active ? (sortDir === 'desc' ? ' ▾' : ' ▴') : ''
  return (
    <button
      onClick={() => onSort(k)}
      className={`${className} text-left hover:text-gray-300 transition-colors ${active ? 'text-cyan-400' : 'text-gray-600'}`}
    >
      {label}{arrow}
    </button>
  )
}

function FilterHeader({ label, value, className, onClick }: {
  label: string
  value: string | null
  className: string
  onClick: () => void
}) {
  return (
    <button
      onClick={onClick}
      className={`${className} text-left hover:text-gray-300 transition-colors ${value ? 'text-amber-400' : 'text-gray-600'}`}
    >
      {value ?? label}
    </button>
  )
}

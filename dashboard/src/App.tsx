import { useState, useEffect, useMemo, useRef, useCallback } from 'react'
import { api, dateRangeParams } from '@/api/client'
import { useApi } from '@/hooks/useApi'
import { useSSE } from '@/hooks/useSSE'
import { useResize } from '@/hooks/useResize'
import TopBar from '@/components/TopBar'
import type { StatAction } from '@/components/TopBar'
import SessionSidebar from '@/components/SessionSidebar'
import TimeSeriesPanel from '@/components/TimeSeriesPanel'
import type { TimeSeriesTab } from '@/components/TimeSeriesPanel'
import CrossDimensionalPanel from '@/components/CrossDimensionalPanel'
import TokenBreakdown from '@/components/TokenBreakdown'
import LiveFeed from '@/components/LiveFeed'
import type { DateRange, StatsResponse } from '@/types'

export default function App() {
  const [range, setRange] = useState<DateRange>('24h')
  const [selectedSession, setSelectedSession] = useState<string | null>(null)
  const [sidebarCollapsed, setSidebarCollapsed] = useState(false)

  const params = dateRangeParams(range)

  const { data: sessions, refetch: refetchSessions } = useApi(() => api.sessions(params), [range])

  // Find the merged session_ids for the selected session
  const selectedSessionIds = useMemo(() => {
    if (!selectedSession || !sessions) return null
    const found = sessions.find(s => s.id === selectedSession)
    return found?.session_ids ?? null
  }, [selectedSession, sessions])

  const sessionFilter = selectedSessionIds ? selectedSessionIds.join(',') : undefined
  const statsParams = sessionFilter ? { ...params, session: sessionFilter } : params
  const { data: stats } = useApi(() => api.stats(statsParams), [range, sessionFilter])

  const liveEvents = useSSE()

  const sidebar = useResize({ initial: 256, min: 180, max: 480, direction: 'horizontal' })
  const feed = useResize({ initial: 192, min: 60, max: 500, direction: 'vertical', invert: true })

  const [chartTab, setChartTab] = useState<TimeSeriesTab>('cost')
  const chartRef = useRef<HTMLDivElement>(null)
  const sidebarRef = useRef<HTMLDivElement>(null)

  const handleStatClick = useCallback((action: StatAction) => {
    if (action === 'sessions') {
      // Expand sidebar if collapsed, then flash it
      if (sidebarCollapsed) setSidebarCollapsed(false)
      if (sidebarRef.current) {
        sidebarRef.current.classList.add('ring-1', 'ring-cyan-400/60')
        setTimeout(() => {
          sidebarRef.current?.classList.remove('ring-1', 'ring-cyan-400/60')
        }, 1200)
      }
    } else {
      const tabMap: Record<string, TimeSeriesTab> = { calls: 'calls', cost: 'cost', errors: 'errors' }
      setChartTab(tabMap[action])
      chartRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    }
  }, [sidebarCollapsed])

  // Auto-refresh sessions every 15s to keep costs up to date
  useEffect(() => {
    const interval = setInterval(() => {
      refetchSessions()
    }, 15_000)
    return () => clearInterval(interval)
  }, [refetchSessions])

  // Also refetch when new SSE events arrive
  const lastEventId = liveEvents.length > 0 ? liveEvents[liveEvents.length - 1].id : null
  useEffect(() => {
    if (lastEventId) {
      refetchSessions()
    }
  }, [lastEventId, refetchSessions])

  return (
    <div className="h-screen flex flex-col bg-gray-950">
      <TopBar stats={stats} sessionCount={sessions?.length ?? 0} range={range} onRangeChange={setRange} onStatClick={handleStatClick} />

      <div className="flex flex-1 overflow-hidden">
        <SessionSidebar
          ref={sidebarRef}
          sessions={sessions}
          selectedSession={selectedSession}
          onSelect={setSelectedSession}
          collapsed={sidebarCollapsed}
          onToggle={() => setSidebarCollapsed(!sidebarCollapsed)}
          width={sidebar.size}
        />
        {!sidebarCollapsed && (
          <div
            onMouseDown={sidebar.onMouseDown}
            className="w-1 hover:bg-cyan-500/40 cursor-col-resize shrink-0 transition-colors"
          />
        )}

        <main className="flex-1 overflow-y-auto p-5 space-y-5">
          {selectedSession && selectedSessionIds && (
            <SessionDetail sessionIds={selectedSessionIds} stats={stats} />
          )}
          <div ref={chartRef}>
            <TimeSeriesPanel timeline={stats?.timeline ?? null} activeTab={chartTab} onTabChange={setChartTab} />
          </div>
          <TokenBreakdown stats={stats} />
          <CrossDimensionalPanel stats={stats} />
        </main>
      </div>

      {/* Vertical resize row: spacer + corner handle + horizontal line */}
      <div className="flex shrink-0">
        {!sidebarCollapsed && (
          <>
            <div
              onMouseDown={feed.onMouseDown}
              style={{ width: sidebar.size }}
              className="h-1.5 cursor-row-resize shrink-0 hover:bg-cyan-500/40 transition-colors"
            />
            <div
              onMouseDown={(e) => { sidebar.onMouseDown(e); feed.onMouseDown(e) }}
              className="w-2 h-1.5 cursor-move shrink-0 hover:bg-cyan-500/60 bg-gray-700/50 transition-colors rounded-sm"
            />
          </>
        )}
        <div
          onMouseDown={feed.onMouseDown}
          className="flex-1 h-1.5 hover:bg-cyan-500/40 cursor-row-resize shrink-0 transition-colors"
        />
      </div>

      <LiveFeed events={liveEvents} height={feed.size} />
    </div>
  )
}

function SessionSummary({ stats }: { stats: StatsResponse | null }) {
  if (!stats) return null
  const c = stats.counts
  const model = stats.models.length > 0 ? stats.models[0].model : null

  return (
    <div className="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-7 gap-2">
      <StatCard label="Calls" value={String(c.total)} />
      <StatCard label="Cost" value={c.cost_usd > 0 ? `$${c.cost_usd.toFixed(2)}` : '$0.00'} color="text-emerald-400" accent="border-t-2 border-emerald-400" />
      <StatCard label="Input" value={fmtTokens(c.input_tokens)} />
      <StatCard label="Output" value={fmtTokens(c.output_tokens)} />
      <StatCard label="Cache" value={fmtTokens(c.cache_read_tokens)} />
      <StatCard label="Errors" value={String(c.errors)} color={c.errors > 0 ? 'text-red-400' : 'text-gray-500'} accent={c.errors > 0 ? 'border-t-2 border-red-400' : ''} />
      {model && <StatCard label="Model" value={model} accent="border-t-2 border-blue-400" />}
    </div>
  )
}

function StatCard({ label, value, color, accent }: { label: string; value: string; color?: string; accent?: string }) {
  return (
    <div className={`bg-gray-800/50 rounded px-3 py-2 ${accent ?? ''}`}>
      <div className="text-[10px] text-gray-500 uppercase tracking-wider">{label}</div>
      <div className={`text-sm font-semibold truncate ${color ?? 'text-gray-200'}`} title={value}>{value}</div>
    </div>
  )
}

function SessionDetail({ sessionIds, stats }: { sessionIds: string[]; stats: StatsResponse | null }) {
  const sessionFilter = sessionIds.join(',')
  const { data: events, loading } = useApi(
    () => api.events({ session: sessionFilter }),
    [sessionFilter],
  )

  if (loading) {
    return (
      <div className="flex items-center justify-center h-32 text-gray-500 text-sm">
        Loading session...
      </div>
    )
  }

  if (!events || events.length === 0) {
    return (
      <div className="flex items-center justify-center h-32 text-gray-600 text-sm">
        No events for this session
      </div>
    )
  }

  const hasTokenData = events.some(e => e.input_tokens != null || e.model != null)

  return (
    <div className="space-y-3">
      <div className="flex items-center gap-2">
        <h2 className="text-xs font-semibold text-gray-400">
          Session — {events.length} events
        </h2>
        {sessionIds.length > 1 && (
          <span className="text-[10px] bg-gray-700/50 text-gray-400 px-1.5 py-0.5 rounded" title={`Merged from ${sessionIds.length} segments: ${sessionIds.join(', ')}`}>
            {sessionIds.length} segments merged
          </span>
        )}
      </div>

      <SessionSummary stats={stats} />

      <div className="bg-gray-900/80 rounded shadow-lg shadow-black/25 overflow-hidden">
        <div className="overflow-y-auto max-h-[400px]">
          <table className="w-full text-xs">
            <thead className="sticky top-0 bg-gray-900 z-10">
              <tr className="text-gray-500 border-b border-gray-800">
                <th className="text-left py-1.5 px-3">Time</th>
                <th className="text-left py-1.5 px-2">Server</th>
                <th className="text-left py-1.5 px-2">Risk</th>
                <th className="text-left py-1.5 px-2">Tool</th>
                <th className="text-left py-1.5 px-2">Argument</th>
                {hasTokenData && <>
                  <th className="text-left py-1.5 px-2">Model</th>
                  <th className="text-right py-1.5 px-2">In</th>
                  <th className="text-right py-1.5 px-2">Out</th>
                  <th className="text-right py-1.5 px-2">Cache</th>
                </>}
                <th className="text-right py-1.5 px-2">Duration</th>
                <th className="text-left py-1.5 px-3">Status</th>
              </tr>
            </thead>
            <tbody>
              {events.map((e, i) => (
                <tr
                  key={e.id}
                  className={`border-b border-gray-800/50 ${e.is_error ? 'bg-red-950/20' : i % 2 === 1 ? 'bg-gray-900/30' : 'hover:bg-gray-800/30'}`}
                >
                  <td className="py-1 px-3 text-gray-500 font-mono whitespace-nowrap">
                    {e.timestamp.slice(11, 19)}
                  </td>
                  <td className="py-1 px-2">
                    <ServerBadge server={e.server} />
                  </td>
                  <td className="py-1 px-2">
                    <RiskBadge risk={e.risk} />
                  </td>
                  <td className="py-1 px-2 text-gray-300 font-semibold whitespace-nowrap">{e.tool}</td>
                  <td className="py-1 px-2 text-gray-400 truncate max-w-[250px]" title={e.arg_display}>
                    {e.arg_display}
                  </td>
                  {hasTokenData && <>
                    <td className="py-1 px-2 text-gray-500 truncate max-w-[100px]" title={e.model ?? undefined}>
                      {e.model ?? ''}
                    </td>
                    <td className="py-1 px-2 text-right text-cyan-400/70 whitespace-nowrap">
                      {e.input_tokens != null ? fmtTokens(e.input_tokens) : ''}
                    </td>
                    <td className="py-1 px-2 text-right text-purple-400/70 whitespace-nowrap">
                      {e.output_tokens != null ? fmtTokens(e.output_tokens) : ''}
                    </td>
                    <td className="py-1 px-2 text-right text-gray-500 whitespace-nowrap">
                      {e.cache_read_tokens != null ? fmtTokens(e.cache_read_tokens) : ''}
                    </td>
                  </>}
                  <td className="py-1 px-2 text-right text-gray-500 whitespace-nowrap">
                    {e.duration_us > 0 ? fmtDuration(e.duration_us) : '—'}
                  </td>
                  <td className="py-1 px-3">
                    {e.is_error ? (
                      <span className="text-red-400" title={e.error_message ?? undefined}>ERR</span>
                    ) : (
                      <span className="text-green-500">OK</span>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  )
}

function ServerBadge({ server }: { server: string }) {
  const colors: Record<string, string> = {
    cursor: 'bg-purple-500/20 text-purple-400',
    'claude-code': 'bg-blue-500/20 text-blue-400',
    vigilo: 'bg-emerald-500/20 text-emerald-400',
  }
  const labels: Record<string, string> = {
    'claude-code': 'claude',
    vigilo: 'mcp',
  }
  const label = labels[server] ?? server
  return (
    <span className={`text-[10px] font-bold px-1.5 py-0.5 rounded ${colors[server] ?? 'bg-gray-500/20 text-gray-400'}`}>
      {label}
    </span>
  )
}

function RiskBadge({ risk }: { risk: string }) {
  const colors: Record<string, string> = {
    read: 'bg-cyan-900/50 text-cyan-400',
    write: 'bg-yellow-900/50 text-yellow-400',
    exec: 'bg-red-900/50 text-red-400',
  }
  return (
    <span className={`text-[10px] font-bold font-mono px-2 py-0.5 rounded ${colors[risk] ?? 'text-gray-500'}`}>
      {risk.toUpperCase()}
    </span>
  )
}

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${Math.floor(n / 1_000)}K`
  return String(n)
}

function fmtDuration(us: number): string {
  if (us < 1_000) return `${us}µs`
  if (us < 1_000_000) return `${(us / 1_000).toFixed(1)}ms`
  return `${(us / 1_000_000).toFixed(1)}s`
}

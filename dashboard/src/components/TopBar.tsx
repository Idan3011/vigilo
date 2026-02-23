import type { StatsResponse, DateRange } from '@/types'

const ranges: DateRange[] = ['24h', '7d', '30d', 'all']

export type StatAction = 'sessions' | 'calls' | 'cost' | 'errors'

interface Props {
  stats: StatsResponse | null
  sessionCount: number
  range: DateRange
  onRangeChange: (r: DateRange) => void
  onStatClick?: (action: StatAction) => void
}

export default function TopBar({ stats, sessionCount, range, onRangeChange, onStatClick }: Props) {
  const c = stats?.counts

  return (
    <header className="border-b border-gray-800 bg-gray-900/50 px-4 py-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <svg width="24" height="24" viewBox="0 0 64 64" className="shrink-0">
            <polygon points="8,10 56,10 32,54" fill="none" stroke="#e8e8f4" strokeWidth="2.5" strokeLinejoin="round"/>
            <line x1="16" y1="29" x2="50" y2="29" stroke="#f59e0b" strokeWidth="2"/>
          </svg>
          <h1 className="text-lg font-bold text-gray-100">vigilo</h1>
          <span className="text-xs text-gray-500">dashboard</span>
        </div>

        {c && (
          <div className="flex items-center gap-2">
            <StatCard label="Sessions" value={String(sessionCount)} onClick={() => onStatClick?.('sessions')} />
            <StatCard label="Calls" value={c.total.toLocaleString()} onClick={() => onStatClick?.('calls')} />
            <StatCard label="Cost" value={c.cost_usd > 0 ? `$${c.cost_usd.toFixed(2)}` : '$0.00'} color="text-emerald-400" onClick={() => onStatClick?.('cost')} />
            <StatCard label="Errors" value={String(c.errors)} color={c.errors > 0 ? 'text-red-400' : 'text-gray-500'} onClick={() => onStatClick?.('errors')} />
          </div>
        )}

        <div className="flex items-center gap-1 bg-gray-800 rounded p-0.5">
          {ranges.map((r) => (
            <button
              key={r}
              onClick={() => onRangeChange(r)}
              className={`px-2.5 py-1 text-xs rounded transition-colors ${
                range === r
                  ? 'bg-gray-700 text-gray-100'
                  : 'text-gray-400 hover:text-gray-200'
              }`}
            >
              {r}
            </button>
          ))}
        </div>
      </div>
    </header>
  )
}

function StatCard({ label, value, color, onClick }: { label: string; value: string; color?: string; onClick?: () => void }) {
  return (
    <button onClick={onClick} className="bg-gray-800/50 px-4 py-2 rounded hover:bg-gray-700/50 transition-colors text-left cursor-pointer">
      <div className="text-[10px] text-gray-500 uppercase tracking-wider">{label}</div>
      <div className={`text-lg font-semibold ${color ?? 'text-white'}`}>{value}</div>
    </button>
  )
}

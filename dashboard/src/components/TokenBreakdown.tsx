import type { StatsResponse } from '@/types'

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${Math.floor(n / 1_000)}K`
  return String(n)
}

interface Props {
  stats: StatsResponse | null
}

export default function TokenBreakdown({ stats }: Props) {
  if (!stats || stats.counts.input_tokens === 0) {
    return (
      <div className="bg-gray-900/80 rounded shadow-lg shadow-black/25 p-5 flex items-center justify-center h-24">
        <span className="text-gray-600 text-sm">No token data yet</span>
      </div>
    )
  }

  const { input_tokens, output_tokens, cache_read_tokens, cost_usd } = stats.counts
  const total = input_tokens + output_tokens + cache_read_tokens
  const inPct = total > 0 ? (input_tokens / total) * 100 : 0
  const outPct = total > 0 ? (output_tokens / total) * 100 : 0
  const cachePct = total > 0 ? (cache_read_tokens / total) * 100 : 0

  return (
    <div className="bg-gray-900/80 rounded shadow-lg shadow-black/25 p-5">
      <div className="flex items-center justify-between mb-3">
        <h3 className="text-sm font-medium text-gray-400 uppercase tracking-wider">Token Breakdown</h3>
        <span className="text-sm text-emerald-400 font-semibold">${cost_usd.toFixed(2)}</span>
      </div>

      {/* Stacked bar */}
      <div className="flex h-12 rounded overflow-hidden mb-3">
        {inPct > 0 && (
          <div
            className="bg-blue-500 flex items-center justify-center text-xs text-white font-bold"
            style={{ width: `${Math.max(inPct, 1.5)}%` }}
            title={`Input: ${fmtTokens(input_tokens)} (${inPct.toFixed(1)}%)`}
          >
            {inPct > 6 ? 'IN' : ''}
          </div>
        )}
        {outPct > 0 && (
          <div
            className="bg-purple-500 flex items-center justify-center text-xs text-white font-bold"
            style={{ width: `${Math.max(outPct, 1.5)}%` }}
            title={`Output: ${fmtTokens(output_tokens)} (${outPct.toFixed(1)}%)`}
          >
            {outPct > 6 ? 'OUT' : ''}
          </div>
        )}
        {cachePct > 0 && (
          <div
            className="bg-gray-700 flex items-center justify-center text-xs text-gray-400 font-bold"
            style={{ width: `${cachePct}%` }}
            title={`Cache: ${fmtTokens(cache_read_tokens)} (${cachePct.toFixed(1)}%)`}
          >
            {cachePct > 10 ? 'CACHE' : ''}
          </div>
        )}
      </div>

      {/* Insight line */}
      {cachePct > 50 && (
        <p className="text-sm text-gray-400 italic mb-3">
          {cachePct.toFixed(1)}% of your tokens are cache — you're paying mostly for context, not your prompts
        </p>
      )}

      {/* Legend */}
      <div className="flex gap-4 text-xs">
        <LegendItem color="bg-blue-500" label="Input" value={fmtTokens(input_tokens)} />
        <LegendItem color="bg-purple-500" label="Output" value={fmtTokens(output_tokens)} />
        {cache_read_tokens > 0 && (
          <LegendItem color="bg-gray-700" label="Cache" value={fmtTokens(cache_read_tokens)} />
        )}
      </div>
    </div>
  )
}

function LegendItem({ color, label, value }: { color: string; label: string; value: string }) {
  return (
    <div className="flex items-center gap-1.5">
      <div className={`w-2 h-2 rounded-sm ${color}`} />
      <span className="text-gray-500">{label}</span>
      <span className="text-gray-300 font-semibold">{value}</span>
    </div>
  )
}

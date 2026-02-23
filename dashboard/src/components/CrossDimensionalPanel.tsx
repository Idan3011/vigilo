import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  CartesianGrid,
} from 'recharts'
import type { StatsResponse } from '@/types'

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${Math.floor(n / 1_000)}K`
  return String(n)
}

const CARD = 'bg-gray-900/80 rounded shadow-lg shadow-black/25 p-5'
const HEADER = 'text-sm font-medium text-gray-400 uppercase tracking-wider mb-3'
const tooltipStyle = {
  contentStyle: { backgroundColor: '#111827', border: '1px solid #374151', borderRadius: 4, fontSize: 11 },
}
const gridProps = { strokeDasharray: '3 3', stroke: '#1f2937', vertical: false }

interface Props {
  stats: StatsResponse | null
}

export default function CrossDimensionalPanel({ stats }: Props) {
  if (!stats) {
    return (
      <div className={`${CARD} flex items-center justify-center h-48`}>
        <span className="text-gray-600 text-sm">No data yet</span>
      </div>
    )
  }

  return (
    <div className="grid grid-cols-2 gap-4">
      <FileChart files={stats.files} />
      <ModelTable models={stats.models} />
      <ToolChart tools={stats.tools} />
      <ProjectChart projects={stats.projects} />
    </div>
  )
}

function FileChart({ files }: { files: StatsResponse['files'] }) {
  const data = files.slice(0, 8)
  if (data.length === 0) {
    return <PanelEmpty label="Most Edited Files" />
  }
  return (
    <div className={CARD}>
      <h3 className={HEADER}>Most Edited Files</h3>
      <ResponsiveContainer width="100%" height={220}>
        <BarChart data={data} layout="vertical" margin={{ left: 80, right: 30, top: 0, bottom: 0 }}>
          <CartesianGrid {...gridProps} />
          <XAxis type="number" tick={{ fontSize: 10, fill: '#6b7280' }} axisLine={false} tickLine={false} />
          <YAxis
            type="category"
            dataKey="file"
            tick={{ fontSize: 10, fill: '#9ca3af' }}
            axisLine={false}
            tickLine={false}
            width={78}
          />
          <Tooltip {...tooltipStyle} />
          <Bar dataKey="count" name="Touches" fill="#06b6d4" radius={[0, 3, 3, 0]} label={{ position: 'right', fontSize: 10, fill: '#9ca3af' }} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  )
}

function ModelTable({ models }: { models: StatsResponse['models'] }) {
  if (models.length === 0) {
    return <PanelEmpty label="Model Efficiency" />
  }
  const topCost = Math.max(...models.map((m) => m.cost_usd))
  return (
    <div className={CARD}>
      <h3 className={HEADER}>Model Efficiency</h3>
      <div className="overflow-x-auto">
        <table className="w-full text-xs">
          <thead>
            <tr className="text-gray-500 border-b border-gray-800">
              <th className="text-left py-1 pr-2">Model</th>
              <th className="text-right py-1 px-2">Calls</th>
              <th className="text-right py-1 px-2">In</th>
              <th className="text-right py-1 px-2">Out</th>
              <th className="text-right py-1 px-2">Cache</th>
              <th className="text-right py-1 pl-2">Cost</th>
            </tr>
          </thead>
          <tbody>
            {models.map((m) => {
              const cacheHit =
                m.input_tokens > 0
                  ? ((m.cache_read_tokens / (m.input_tokens + m.cache_read_tokens)) * 100).toFixed(0)
                  : '—'
              const isTop = m.cost_usd === topCost && m.cost_usd > 0
              const isZero = m.cost_usd === 0
              return (
                <tr
                  key={m.model}
                  className={`border-b border-gray-800/50 ${isTop ? 'border-l-2 border-l-emerald-400' : ''} ${isZero ? 'text-gray-600' : 'text-gray-300'}`}
                >
                  <td className="py-1 pr-2 truncate max-w-[120px]">{m.model}</td>
                  <td className="text-right py-1 px-2 font-semibold">{m.calls}</td>
                  <td className="text-right py-1 px-2 text-gray-400">{fmtTokens(m.input_tokens)}</td>
                  <td className="text-right py-1 px-2 text-gray-400">{fmtTokens(m.output_tokens)}</td>
                  <td className={`text-right py-1 px-2 ${cacheHit === '100' ? 'text-emerald-400' : cacheHit === '—' ? 'text-gray-600' : 'text-gray-500'}`}>
                    {cacheHit}%
                  </td>
                  <td className={`text-right py-1 pl-2 ${isTop ? 'text-emerald-400' : isZero ? 'text-gray-600' : 'text-gray-400'}`}>
                    ${m.cost_usd.toFixed(2)}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
    </div>
  )
}

function ToolChart({ tools }: { tools: StatsResponse['tools'] }) {
  const data = tools.slice(0, 10).map((t) => ({
    ...t,
    ok_count: t.count - t.error_count,
  }))
  if (data.length === 0) {
    return <PanelEmpty label="Tool Breakdown" />
  }
  return (
    <div className={CARD}>
      <h3 className={HEADER}>Tool Breakdown</h3>
      <ResponsiveContainer width="100%" height={180}>
        <BarChart data={data} layout="vertical" margin={{ left: 70, right: 8, top: 0, bottom: 0 }}>
          <CartesianGrid {...gridProps} />
          <XAxis type="number" tick={{ fontSize: 10, fill: '#6b7280' }} axisLine={false} tickLine={false} />
          <YAxis
            type="category"
            dataKey="tool"
            tick={{ fontSize: 10, fill: '#9ca3af' }}
            axisLine={false}
            tickLine={false}
            width={68}
          />
          <Tooltip {...tooltipStyle} />
          <Bar dataKey="ok_count" name="OK" fill="#3B82F6" stackId="s" />
          <Bar dataKey="error_count" name="Errors" fill="#ef4444" stackId="s" radius={[0, 3, 3, 0]} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  )
}

function ProjectChart({ projects }: { projects: StatsResponse['projects'] }) {
  const data = projects.slice(0, 8)
  if (data.length === 0) {
    return <PanelEmpty label="Risk by Project" />
  }
  return (
    <div className={CARD}>
      <h3 className={HEADER}>Risk by Project</h3>
      <ResponsiveContainer width="100%" height={180}>
        <BarChart data={data} layout="vertical" margin={{ left: 70, right: 8, top: 0, bottom: 0 }}>
          <CartesianGrid {...gridProps} />
          <XAxis type="number" tick={{ fontSize: 10, fill: '#6b7280' }} axisLine={false} tickLine={false} />
          <YAxis
            type="category"
            dataKey="name"
            tick={{ fontSize: 10, fill: '#9ca3af' }}
            axisLine={false}
            tickLine={false}
            width={68}
          />
          <Tooltip {...tooltipStyle} />
          <Bar dataKey="reads" name="Read" fill="#06b6d4" stackId="risk" />
          <Bar dataKey="writes" name="Write" fill="#3B82F6" stackId="risk" />
          <Bar dataKey="execs" name="Exec" fill="#ef4444" stackId="risk" radius={[0, 3, 3, 0]} />
        </BarChart>
      </ResponsiveContainer>
    </div>
  )
}

function PanelEmpty({ label }: { label: string }) {
  return (
    <div className={CARD}>
      <h3 className={HEADER}>{label}</h3>
      <div className="flex items-center justify-center h-[180px]">
        <span className="text-gray-600 text-sm">No data yet</span>
      </div>
    </div>
  )
}

import { useState } from 'react'
import {
  BarChart,
  Bar,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  CartesianGrid,
} from 'recharts'
import type { TimelineDay } from '@/types'

type Tab = 'cost' | 'tokens' | 'calls' | 'errors'

const tabs: { key: Tab; label: string }[] = [
  { key: 'cost', label: 'Cost' },
  { key: 'tokens', label: 'Tokens' },
  { key: 'calls', label: 'Calls' },
  { key: 'errors', label: 'Errors' },
]

export type { Tab as TimeSeriesTab }

interface Props {
  timeline: TimelineDay[] | null
  activeTab?: Tab
  onTabChange?: (tab: Tab) => void
}

function shortDate(d: string): string {
  return d.slice(5).replace('-', '/')
}

const tooltipStyle = {
  contentStyle: {
    backgroundColor: '#111827',
    border: '1px solid #374151',
    borderRadius: 4,
    fontSize: 11,
  },
  labelStyle: { color: '#9ca3af' },
}

const xAxisProps = {
  dataKey: 'date' as const,
  tick: { fontSize: 10, fill: '#6b7280' },
  axisLine: { stroke: '#374151' },
  tickLine: false,
}

const yAxisProps = {
  tick: { fontSize: 10, fill: '#6b7280' },
  axisLine: false as const,
  tickLine: false,
  width: 50,
}

const chartMargin = { top: 4, right: 4, bottom: 0, left: 0 }
const gridProps = { strokeDasharray: '3 3', stroke: '#1f2937', vertical: false }

export default function TimeSeriesPanel({ timeline, activeTab, onTabChange }: Props) {
  const [internalTab, setInternalTab] = useState<Tab>('cost')
  const tab = activeTab ?? internalTab
  const setTab = (t: Tab) => { setInternalTab(t); onTabChange?.(t) }

  if (!timeline || timeline.length === 0) {
    return <EmptyState />
  }

  const data = timeline.map((d) => ({
    ...d,
    date: shortDate(d.date),
    total_calls: d.reads + d.writes + d.execs,
  }))

  return (
    <div className="bg-gray-900/80 rounded shadow-lg shadow-black/25 p-5">
      <div className="flex items-center gap-1 mb-4">
        {tabs.map((t) => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            className={`px-2.5 py-1 text-xs rounded transition-colors ${
              tab === t.key
                ? 'bg-gray-700 text-gray-100'
                : 'text-gray-500 hover:text-gray-300'
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>

      {tab === 'cost' && <CostChart data={data} />}
      {tab === 'tokens' && <TokensChart data={data} />}
      {tab === 'calls' && <CallsChart data={data} />}
      {tab === 'errors' && <ErrorsChart data={data} />}
    </div>
  )
}

function CostChart({ data }: { data: Record<string, unknown>[] }) {
  return (
    <ResponsiveContainer width="100%" height={220}>
      <BarChart data={data} margin={chartMargin}>
        <CartesianGrid {...gridProps} />
        <XAxis {...xAxisProps} />
        <YAxis {...yAxisProps} tickFormatter={(v: number) => `$${v}`} />
        <Tooltip {...tooltipStyle} />
        <Bar dataKey="cost_usd" name="Cost ($)" fill="#3B82F6" radius={[3, 3, 0, 0]} />
      </BarChart>
    </ResponsiveContainer>
  )
}

function TokensChart({ data }: { data: Record<string, unknown>[] }) {
  return (
    <ResponsiveContainer width="100%" height={220}>
      <BarChart data={data} margin={chartMargin}>
        <CartesianGrid {...gridProps} />
        <XAxis {...xAxisProps} />
        <YAxis {...yAxisProps} />
        <Tooltip {...tooltipStyle} />
        <Bar dataKey="input_tokens" name="Input" fill="#3B82F6" stackId="tok" radius={[0, 0, 0, 0]} />
        <Bar dataKey="output_tokens" name="Output" fill="#8B5CF6" stackId="tok" radius={[3, 3, 0, 0]} />
      </BarChart>
    </ResponsiveContainer>
  )
}

function CallsChart({ data }: { data: Record<string, unknown>[] }) {
  return (
    <ResponsiveContainer width="100%" height={220}>
      <BarChart data={data} margin={chartMargin}>
        <CartesianGrid {...gridProps} />
        <XAxis {...xAxisProps} />
        <YAxis {...yAxisProps} />
        <Tooltip {...tooltipStyle} />
        <Bar dataKey="reads" name="Read" fill="#06b6d4" stackId="risk" />
        <Bar dataKey="writes" name="Write" fill="#3B82F6" stackId="risk" />
        <Bar dataKey="execs" name="Exec" fill="#ef4444" stackId="risk" radius={[3, 3, 0, 0]} />
      </BarChart>
    </ResponsiveContainer>
  )
}

function ErrorsChart({ data }: { data: Record<string, unknown>[] }) {
  return (
    <ResponsiveContainer width="100%" height={220}>
      <BarChart data={data} margin={chartMargin}>
        <CartesianGrid {...gridProps} />
        <XAxis {...xAxisProps} />
        <YAxis {...yAxisProps} />
        <Tooltip {...tooltipStyle} />
        <Bar dataKey="errors" name="Errors" fill="#ef4444" radius={[3, 3, 0, 0]} />
      </BarChart>
    </ResponsiveContainer>
  )
}

function EmptyState() {
  return (
    <div className="bg-gray-900/80 rounded shadow-lg shadow-black/25 p-5 flex items-center justify-center h-[280px]">
      <span className="text-gray-600 text-sm">No timeline data yet</span>
    </div>
  )
}

import { forwardRef } from 'react'
import type { SessionItem } from '@/types'

function fmtCost(usd: number): string {
  if (usd === 0) return ''
  if (usd < 0.01) return `$${usd.toFixed(4)}`
  return `$${usd.toFixed(2)}`
}

interface Props {
  sessions: SessionItem[] | null
  selectedSession: string | null
  onSelect: (id: string | null) => void
  collapsed: boolean
  onToggle: () => void
  width: number
}

export default forwardRef<HTMLDivElement, Props>(function SessionSidebar({
  sessions,
  selectedSession,
  onSelect,
  collapsed,
  onToggle,
  width,
}, ref) {
  if (collapsed) {
    return (
      <div className="w-8 border-r border-gray-800 bg-gray-900/30 flex flex-col items-center pt-3">
        <button
          onClick={onToggle}
          className="text-gray-500 hover:text-gray-300 text-xs"
          title="Expand sessions"
        >
          &raquo;
        </button>
      </div>
    )
  }

  return (
    <aside ref={ref} style={{ width }} className="border-r border-gray-800 bg-gray-900/30 flex flex-col overflow-hidden shrink-0 transition-shadow duration-500">
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-800">
        <span className="text-xs font-semibold text-gray-400 uppercase tracking-wider">
          Sessions {sessions ? `(${sessions.length})` : ''}
        </span>
        <div className="flex gap-2">
          {selectedSession && (
            <button
              onClick={() => onSelect(null)}
              className="text-xs text-gray-500 hover:text-gray-300"
            >
              clear
            </button>
          )}
          <button
            onClick={onToggle}
            className="text-gray-500 hover:text-gray-300 text-xs"
            title="Collapse"
          >
            &laquo;
          </button>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {/* All Sessions option */}
        <button
          onClick={() => onSelect(null)}
          className={`w-full text-left px-3 py-2 border-b border-gray-800/30 border-l-2 transition-colors text-xs ${
            !selectedSession
              ? 'border-l-cyan-400 bg-gray-800/60 text-gray-200'
              : 'border-l-transparent hover:bg-gray-800/30 text-gray-400'
          }`}
        >
          All Sessions
        </button>

        {!sessions || sessions.length === 0 ? (
          <div className="p-3 text-xs text-gray-600">No sessions yet</div>
        ) : (
          sessions
            .slice()
            .reverse()
            .map((s) => {
              const isSelected = selectedSession === s.id
              const isMerged = s.session_ids && s.session_ids.length > 1
              const hasProject = s.project && s.project !== '—'
              const tooltipLines = [
                `Session: ${s.id.slice(0, 8)}${isMerged ? ` (+${s.session_ids.length - 1} merged)` : ''}`,
                hasProject ? `Project: ${s.project}` : null,
                s.branch ? `Branch: ${s.branch}` : null,
                `Calls: ${s.call_count}`,
                s.cost_usd > 0 ? `Cost: $${s.cost_usd.toFixed(4)}` : null,
                s.error_count > 0 ? `Errors: ${s.error_count}` : null,
              ].filter(Boolean).join('\n')

              return (
                <button
                  key={s.id}
                  onClick={() => onSelect(isSelected ? null : s.id)}
                  title={tooltipLines}
                  className={`w-full text-left px-3 py-2 border-b border-gray-800/30 border-l-2 transition-colors ${
                    isSelected
                      ? 'border-l-cyan-400 bg-gray-800/60'
                      : 'border-l-transparent hover:bg-gray-800/30'
                  }`}
                >
                  <div className="flex items-center gap-1.5">
                    <ServerBadge server={s.server} />
                    {hasProject && (
                      <span className="text-[11px] text-gray-300 truncate flex-1 font-medium" title={s.project!}>
                        {s.project}
                      </span>
                    )}
                    {isMerged && (
                      <span className="text-[9px] bg-gray-700 text-gray-300 px-1 rounded" title={`Merged ${s.session_ids.length} conversation segments`}>
                        {s.session_ids.length}
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-2 mt-0.5">
                    <span className="text-[10px] text-gray-500">{s.date}</span>
                    {s.branch && (
                      <span className="text-[10px] text-gray-600 truncate" title={s.branch}>
                        {s.branch}
                      </span>
                    )}
                  </div>
                  <div className="flex items-center gap-2 mt-0.5 text-[10px]">
                    <span className="text-gray-400">{s.call_count} calls</span>
                    {s.cost_usd > 0 && (
                      <span className="text-emerald-400/80">{fmtCost(s.cost_usd)}</span>
                    )}
                    {s.error_count > 0 && (
                      <span className="text-red-400">{s.error_count} err</span>
                    )}
                  </div>
                </button>
              )
            })
        )}
      </div>
    </aside>
  )
})

function ServerBadge({ server }: { server: string }) {
  const styles: Record<string, { cls: string; label: string; title: string }> = {
    cursor: { cls: 'bg-purple-900/60 text-purple-400', label: 'CURSOR', title: 'Cursor' },
    'claude-code': { cls: 'bg-blue-900/60 text-blue-400', label: 'CLAUDE', title: 'Claude Code' },
    vigilo: { cls: 'bg-emerald-900/60 text-emerald-400', label: 'MCP', title: 'Vigilo MCP Server' },
  }
  const s = styles[server] ?? { cls: 'bg-gray-700/60 text-gray-400', label: server.toUpperCase(), title: server }
  return (
    <span className={`text-[9px] font-bold px-1 py-0.5 rounded ${s.cls}`} title={s.title}>
      {s.label}
    </span>
  )
}

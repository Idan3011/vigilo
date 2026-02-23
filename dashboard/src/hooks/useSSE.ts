import { useState, useEffect, useRef, useCallback } from 'react'
import type { EventItem } from '@/types'

const MAX_EVENTS = 200

export function useSSE(): EventItem[] {
  const [events, setEvents] = useState<EventItem[]>([])
  const activeRef = useRef(true)
  const esRef = useRef<EventSource | null>(null)
  const reconnectRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const connect = useCallback(() => {
    if (!activeRef.current) return

    const url = `${window.location.origin}/api/events/stream`
    const es = new EventSource(url)
    esRef.current = es

    es.onmessage = (msg) => {
      try {
        const item: EventItem = JSON.parse(msg.data)
        setEvents((prev) => {
          const next = [...prev, item]
          return next.length > MAX_EVENTS ? next.slice(-MAX_EVENTS) : next
        })
      } catch {
        // ignore parse errors
      }
    }

    es.onerror = () => {
      es.close()
      esRef.current = null
      if (activeRef.current) {
        reconnectRef.current = setTimeout(connect, 3000)
      }
    }
  }, [])

  useEffect(() => {
    activeRef.current = true
    connect()

    return () => {
      activeRef.current = false
      if (reconnectRef.current) {
        clearTimeout(reconnectRef.current)
        reconnectRef.current = null
      }
      if (esRef.current) {
        esRef.current.close()
        esRef.current = null
      }
    }
  }, [connect])

  return events
}

import { useState, useCallback, useEffect, useRef } from 'react'

interface UseResizeOptions {
  initial: number
  min: number
  max: number
  direction: 'horizontal' | 'vertical'
  /** For vertical: 'up' means dragging up increases size (bottom panel) */
  invert?: boolean
}

// Track how many resize hooks are actively dragging so cursor cleanup is correct
let activeDrags = 0

export function useResize({ initial, min, max, direction, invert }: UseResizeOptions) {
  const [size, setSize] = useState(initial)
  const dragging = useRef(false)
  const startPos = useRef(0)
  const startSize = useRef(0)

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    dragging.current = true
    activeDrags++
    startPos.current = direction === 'horizontal' ? e.clientX : e.clientY
    startSize.current = size
    document.body.style.userSelect = 'none'
    // Cursor is set after all mousedown handlers fire (via microtask)
    // so the corner handle can override it
    queueMicrotask(() => {
      if (activeDrags >= 2) {
        document.body.style.cursor = 'move'
      } else if (activeDrags === 1) {
        document.body.style.cursor = direction === 'horizontal' ? 'col-resize' : 'row-resize'
      }
    })
  }, [direction, size])

  useEffect(() => {
    const onMouseMove = (e: MouseEvent) => {
      if (!dragging.current) return
      const pos = direction === 'horizontal' ? e.clientX : e.clientY
      const delta = invert ? startPos.current - pos : pos - startPos.current
      setSize(Math.min(max, Math.max(min, startSize.current + delta)))
    }

    const onMouseUp = () => {
      if (dragging.current) {
        dragging.current = false
        activeDrags = Math.max(0, activeDrags - 1)
        if (activeDrags === 0) {
          document.body.style.cursor = ''
          document.body.style.userSelect = ''
        }
      }
    }

    document.addEventListener('mousemove', onMouseMove)
    document.addEventListener('mouseup', onMouseUp)
    return () => {
      document.removeEventListener('mousemove', onMouseMove)
      document.removeEventListener('mouseup', onMouseUp)
    }
  }, [direction, min, max, invert])

  return { size, onMouseDown }
}

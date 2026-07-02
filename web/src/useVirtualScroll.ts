import { useState, useRef, useCallback, useEffect, useMemo } from 'react'

const LINE_HEIGHT = 26
const OVERSCAN = 20

export const ROW_LOG = 26
export const ROW_RECORD = 34
export const ROW_SEP = 24

export interface VirtualScrollState {
  scrollRef: (node: HTMLDivElement | null) => void
  scrollEl: React.RefObject<HTMLDivElement | null>
  handleScroll: () => void
  isAtBottomRef: React.MutableRefObject<boolean>
  resetScroll: () => void
  stickToBottom: () => void
  startIdx: (totalItems: number) => number
  endIdx: (totalItems: number) => number
  offsetOf: (i: number) => number
  totalHeight: (totalItems: number) => number
}

function indexAt(prefix: number[], y: number): number {
  let lo = 0
  let hi = prefix.length - 1
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1
    if (prefix[mid] <= y) lo = mid
    else hi = mid - 1
  }
  return lo
}

/**
 * @fn useVirtualScroll(rows: number | number[] = LINE_HEIGHT): VirtualScrollState
 * @brief keep large row sets within the visible scroll window.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-ruler: coordinates
 *
 * row heights and offsets are CSS pixels. exported row heights must match rendered CSS dimensions.
 * `startIdx` is inclusive and `endIdx` is exclusive. `offsetOf` is relative to the full row set.
 *
 * ### :material-refresh: prefix sums
 *
 * pass one height for uniform rows or one height per row.
 * pass a memoized array to reuse prefix sums.
 */
export function useVirtualScroll(rows: number | number[] = LINE_HEIGHT): VirtualScrollState {
  const elRef = useRef<HTMLDivElement | null>(null)
  const obsRef = useRef<ResizeObserver | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [containerHeight, setContainerHeight] = useState(600)
  const isAtBottomRef = useRef(true)
  const rafRef = useRef(0)

  const uniform = typeof rows === 'number' ? rows : null

  // `prefix[i]` is the row offset. the final entry is the total height.
  const prefix = useMemo(() => {
    if (typeof rows === 'number') return null
    const acc = new Array<number>(rows.length + 1)
    acc[0] = 0
    for (let i = 0; i < rows.length; i++) {
      acc[i + 1] = acc[i] + rows[i]
    }
    return acc
  }, [rows])

  // reconnect the observer when a conditional scroll element changes.
  const scrollRef = useCallback((node: HTMLDivElement | null) => {
    if (elRef.current === node) return
    if (obsRef.current) { obsRef.current.disconnect(); obsRef.current = null }
    elRef.current = node
    if (!node) return
    setContainerHeight(node.clientHeight)
    const obs = new ResizeObserver(([e]) => { setContainerHeight(e.contentRect.height); })
    obs.observe(node)
    obsRef.current = obs
  }, [])

  useEffect(() => () => { obsRef.current?.disconnect() }, [])

  useEffect(() => () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current)
  }, [])

  const nearBottom = uniform ?? ROW_RECORD

  const handleScroll = useCallback(() => {
    const el = elRef.current
    if (!el) return
    if (rafRef.current) cancelAnimationFrame(rafRef.current)
    rafRef.current = requestAnimationFrame(() => {
      setScrollTop(el.scrollTop)
      isAtBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < nearBottom
    })
  }, [nearBottom])

  const resetScroll = useCallback(() => {
    isAtBottomRef.current = true
    setScrollTop(0)
  }, [])

  // keep scroll mutation inside the hook that owns the element.
  const stickToBottom = useCallback(() => {
    if (!isAtBottomRef.current) return
    const el = elRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [])

  const getStartIdx = useCallback((_totalItems: number) => {
    if (uniform != null) return Math.max(0, Math.floor(scrollTop / uniform) - OVERSCAN)
    if (!prefix) return 0
    return Math.max(0, indexAt(prefix, scrollTop) - OVERSCAN)
  }, [scrollTop, uniform, prefix])

  const getEndIdx = useCallback((totalItems: number) => {
    if (uniform != null) {
      return Math.min(
        totalItems,
        Math.floor(scrollTop / uniform) + Math.ceil(containerHeight / uniform) + OVERSCAN,
      )
    }
    if (!prefix) return totalItems
    return Math.min(totalItems, indexAt(prefix, scrollTop + containerHeight) + 1 + OVERSCAN)
  }, [scrollTop, containerHeight, uniform, prefix])

  const offsetOf = useCallback((i: number) => {
    if (uniform != null) return i * uniform
    if (!prefix) return 0
    return prefix[Math.min(i, prefix.length - 1)]
  }, [uniform, prefix])

  const totalHeight = useCallback((totalItems: number) => {
    if (uniform != null) return totalItems * uniform
    if (!prefix) return 0
    return prefix[Math.min(totalItems, prefix.length - 1)]
  }, [uniform, prefix])

  return {
    scrollRef,
    scrollEl: elRef,
    handleScroll,
    isAtBottomRef,
    resetScroll,
    stickToBottom,
    startIdx: getStartIdx,
    endIdx: getEndIdx,
    offsetOf,
    totalHeight,
  }
}

export { LINE_HEIGHT }

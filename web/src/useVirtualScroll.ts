import { useState, useRef, useCallback, useEffect, useMemo } from 'react'

/** Default row height in CSS pixels when the caller passes none. Same value as ROW_LOG. */
const LINE_HEIGHT = 26
/** Extra rows rendered above and below the window, so a fast scroll shows no gap. */
const OVERSCAN = 20

/**
 * Fixed row heights in CSS pixels, and the only source of truth for them.
 *
 * The scroll arithmetic below multiplies these, so it cannot read a custom
 * property. index.css declares similarly named --h-logrow, --h-record and
 * --h-sep tokens, but nothing reads those and their values differ (24, 32, 22).
 * The two sets are unrelated; do not change either to match the other. Render a
 * row at exactly the constant used here, or the spacers drift and the list
 * slips as it scrolls.
 */
export const ROW_LOG = 26
export const ROW_RECORD = 34
export const ROW_SEP = 24

export interface VirtualScrollState {
  scrollRef: (node: HTMLDivElement | null) => void
  scrollEl: React.RefObject<HTMLDivElement | null>
  handleScroll: () => void
  isAtBottomRef: React.MutableRefObject<boolean>
  resetScroll: () => void
  /** Pin the viewport to the tail, unless the user has scrolled away from it. */
  stickToBottom: () => void
  startIdx: (totalItems: number) => number
  endIdx: (totalItems: number) => number
  /** Pixel offset of row `i` from the top of the content. */
  offsetOf: (i: number) => number
  /** Total content height for `totalItems` rows. */
  totalHeight: (totalItems: number) => number
}

/** Index of the last offset <= y. Offsets are ascending, so binary search. */
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
 * Windowed scrolling for long lists.
 *
 * `rows` is either a single row height (the common case) or a per-row height
 * array. The array form exists because the records list interleaves 34px mod
 * rows with 24px separator bands: a single fixed height would make every
 * spacer and every index-to-offset mapping wrong, and the list would drift as
 * you scroll. Pass a memoized array - a fresh one each render rebuilds the
 * prefix sums.
 */
export function useVirtualScroll(rows: number | number[] = LINE_HEIGHT): VirtualScrollState {
  const elRef = useRef<HTMLDivElement | null>(null)
  const obsRef = useRef<ResizeObserver | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [containerHeight, setContainerHeight] = useState(600)
  const isAtBottomRef = useRef(true)
  const rafRef = useRef(0)

  const uniform = typeof rows === 'number' ? rows : null

  // prefix[i] is the offset of row i; prefix[n] is the total height.
  const prefix = useMemo(() => {
    if (typeof rows === 'number') return null
    const acc = new Array<number>(rows.length + 1)
    acc[0] = 0
    for (let i = 0; i < rows.length; i++) {
      acc[i + 1] = acc[i] + rows[i]
    }
    return acc
  }, [rows])

  // A callback ref fires on every attach and detach, so the ResizeObserver
  // stays connected even when the element is conditionally rendered, such as
  // while a loading skeleton stands in for it.
  const scrollRef = useCallback((node: HTMLDivElement | null) => {
    if (elRef.current === node) return
    // Tear down the observer on the previous node.
    if (obsRef.current) { obsRef.current.disconnect(); obsRef.current = null }
    elRef.current = node
    if (!node) return
    setContainerHeight(node.clientHeight)
    const obs = new ResizeObserver(([e]) => { setContainerHeight(e.contentRect.height); })
    obs.observe(node)
    obsRef.current = obs
  }, [])

  // Disconnect on unmount.
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

  // Owned here rather than at the call site: the scroll element belongs to this
  // hook, and writing through a ref a hook handed you is what
  // react-hooks/immutability forbids. Callers only say "stick to the tail".
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

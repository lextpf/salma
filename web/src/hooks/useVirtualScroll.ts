import { useLayoutEffect, useState, useRef, useCallback, useEffect } from 'react'

const LINE_HEIGHT = 26
const OVERSCAN = 20

export interface VirtualScrollState {
  scrollRef: React.RefObject<HTMLDivElement>
  handleScroll: () => void
  isAtBottomRef: React.MutableRefObject<boolean>
  resetScroll: () => void
  startIdx: (totalItems: number) => number
  endIdx: (totalItems: number) => number
}

export function useVirtualScroll(): VirtualScrollState {
  const scrollRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [containerHeight, setContainerHeight] = useState(600)
  const isAtBottomRef = useRef(true)
  const rafRef = useRef(0)

  useLayoutEffect(() => {
    const el = scrollRef.current
    if (!el) return
    setContainerHeight(el.clientHeight)
    const obs = new ResizeObserver(([e]) => setContainerHeight(e.contentRect.height))
    obs.observe(el)
    return () => obs.disconnect()
  }, [])

  useEffect(() => () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current)
  }, [])

  const handleScroll = useCallback(() => {
    const el = scrollRef.current
    if (!el) return
    if (rafRef.current) cancelAnimationFrame(rafRef.current)
    rafRef.current = requestAnimationFrame(() => {
      setScrollTop(el.scrollTop)
      isAtBottomRef.current = el.scrollHeight - el.scrollTop - el.clientHeight < LINE_HEIGHT
    })
  }, [])

  const resetScroll = useCallback(() => {
    isAtBottomRef.current = true
    setScrollTop(0)
  }, [])

  const getStartIdx = useCallback((_totalItems: number) => {
    return Math.max(0, Math.floor(scrollTop / LINE_HEIGHT) - OVERSCAN)
  }, [scrollTop])

  const getEndIdx = useCallback((totalItems: number) => {
    return Math.min(totalItems, Math.floor(scrollTop / LINE_HEIGHT) + Math.ceil(containerHeight / LINE_HEIGHT) + OVERSCAN)
  }, [scrollTop, containerHeight])

  return {
    scrollRef,
    handleScroll,
    isAtBottomRef,
    resetScroll,
    startIdx: getStartIdx,
    endIdx: getEndIdx,
  }
}

export { LINE_HEIGHT }

import { useState, useRef, useCallback, useEffect } from 'react'

const LINE_HEIGHT = 26
const OVERSCAN = 20

export interface VirtualScrollState {
  scrollRef: (node: HTMLDivElement | null) => void
  scrollEl: React.RefObject<HTMLDivElement | null>
  handleScroll: () => void
  isAtBottomRef: React.MutableRefObject<boolean>
  resetScroll: () => void
  startIdx: (totalItems: number) => number
  endIdx: (totalItems: number) => number
}

export function useVirtualScroll(): VirtualScrollState {
  const elRef = useRef<HTMLDivElement | null>(null)
  const obsRef = useRef<ResizeObserver | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [containerHeight, setContainerHeight] = useState(600)
  const isAtBottomRef = useRef(true)
  const rafRef = useRef(0)

  // Callback ref: fires whenever the DOM node is attached or detached,
  // so the ResizeObserver is always connected - even when the element is
  // conditionally rendered (e.g. hidden behind a loading skeleton).
  const scrollRef = useCallback((node: HTMLDivElement | null) => {
    if (elRef.current === node) return
    // Tear down old observer
    if (obsRef.current) { obsRef.current.disconnect(); obsRef.current = null }
    elRef.current = node
    if (!node) return
    setContainerHeight(node.clientHeight)
    const obs = new ResizeObserver(([e]) => setContainerHeight(e.contentRect.height))
    obs.observe(node)
    obsRef.current = obs
  }, [])

  // Clean up observer on unmount
  useEffect(() => () => { obsRef.current?.disconnect() }, [])

  useEffect(() => () => {
    if (rafRef.current) cancelAnimationFrame(rafRef.current)
  }, [])

  const handleScroll = useCallback(() => {
    const el = elRef.current
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
    scrollEl: elRef,
    handleScroll,
    isAtBottomRef,
    resetScroll,
    startIdx: getStartIdx,
    endIdx: getEndIdx,
  }
}

export { LINE_HEIGHT }

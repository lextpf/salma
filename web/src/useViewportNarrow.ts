import { useEffect, useState } from 'react'

/** Expanded module rail width in CSS pixels. Keep in step with ModuleRail.tsx. */
export const RAIL_WIDTH = 216

/** Collapsed rail width in CSS pixels. Keep in step with ModuleRail.tsx. */
export const RAIL_WIDTH_ICON = 62

/**
 * Returns true when the current viewport width is below `thresholdPx`.
 *
 * Width sibling of useViewportShort. Prefer useContentBreakpoints for new
 * work: what matters is the space beside the rail, not the window.
 */
export function useViewportNarrow(thresholdPx = 1100): boolean {
  const [narrow, setNarrow] = useState(
    typeof window !== 'undefined' ? window.innerWidth < thresholdPx : false,
  )
  useEffect(() => {
    const onResize = () => { setNarrow(window.innerWidth < thresholdPx); }
    window.addEventListener('resize', onResize)
    return () => { window.removeEventListener('resize', onResize); }
  }, [thresholdPx])
  return narrow
}

export interface ContentBreakpoints {
  /** Width available to the module content, i.e. the window minus the rail. */
  content: number
  /** Below 950: the Library VFS tree column is not rendered at all. */
  hideTree: boolean
  /** Below 780: toolbars collapse to icon-only, log level tabs to one button. */
  compactToolbar: boolean
  /** Below 700: the inspector confidence dial degrades to an inline chip. */
  chipDial: boolean
}

function measure(railWidth: number): ContentBreakpoints {
  const content = typeof window === 'undefined' ? 1440 : window.innerWidth - railWidth
  return {
    content,
    hideTree: content < 950,
    compactToolbar: content < 780,
    chipDial: content < 700,
  }
}

/**
 * The three responsive breakpoints, measured against the content column
 * rather than the window.
 *
 * Users dock the dashboard narrow beside MO2, so these are a requirement, not
 * a nicety. A 936px window leaves only 720px next to the 216px rail, which is
 * where the Library triptych starts to fail: pinned column widths there starve
 * the inspector and push its dial past the viewport edge.
 */
export function useContentBreakpoints(railWidth = RAIL_WIDTH): ContentBreakpoints {
  const [bp, setBp] = useState(() => measure(railWidth))
  useEffect(() => {
    const onResize = () => setBp(measure(railWidth))
    onResize()
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [railWidth])
  return bp
}

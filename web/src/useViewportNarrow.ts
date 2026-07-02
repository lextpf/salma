/**
 * @brief measure dashboard breakpoints against the available content area.
 * @author Alex (https://github.com/lextpf)
 *
 * keep rail widths synchronized with `ModuleRail.tsx`.
 */
import { useEffect, useState } from 'react'

export const RAIL_WIDTH = 216

export const RAIL_WIDTH_ICON = 62

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
  content: number
  hideTree: boolean
  compactToolbar: boolean
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

import { useEffect, useState } from 'react'

/**
 * Returns true when the current viewport width is below `thresholdPx`.
 *
 * Width sibling of useViewportShort. Used by the install-page header to
 * collapse its toolbar to icon-only buttons and hide the LED status text
 * when there isn't enough horizontal room for full labels (e.g. phone
 * landscape widths around 930px, where the content column next to the
 * 192px module rail drops to ~740px).
 */
export function useViewportNarrow(thresholdPx = 1100): boolean {
  const [narrow, setNarrow] = useState(
    typeof window !== 'undefined' ? window.innerWidth < thresholdPx : false,
  )
  useEffect(() => {
    const onResize = () => setNarrow(window.innerWidth < thresholdPx)
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [thresholdPx])
  return narrow
}

import { useEffect, useState } from 'react'

/**
 * Returns true when the current viewport height is below `thresholdPx`.
 *
 * The Install queue uses it to start collapsed when there is not enough
 * vertical room for the expanded view, such as on a 1398x645 screen. The user
 * can still expand it by hand and scroll inside the space available.
 */
export function useViewportShort(thresholdPx = 760): boolean {
  const [short, setShort] = useState(
    typeof window !== 'undefined' ? window.innerHeight < thresholdPx : false,
  )
  useEffect(() => {
    const onResize = () => { setShort(window.innerHeight < thresholdPx); }
    window.addEventListener('resize', onResize)
    return () => { window.removeEventListener('resize', onResize); }
  }, [thresholdPx])
  return short
}

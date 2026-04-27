import { useEffect, useState } from 'react'

/**
 * Returns true when the current viewport height is below `thresholdPx`.
 *
 * Used by the install-page queue to default-collapse when there isn't
 * enough vertical room for a comfortable expanded view (e.g. on 1398x645
 * screens). The user can still manually expand and scroll within the
 * available space.
 */
export function useViewportShort(thresholdPx = 760): boolean {
  const [short, setShort] = useState(
    typeof window !== 'undefined' ? window.innerHeight < thresholdPx : false,
  )
  useEffect(() => {
    const onResize = () => setShort(window.innerHeight < thresholdPx)
    window.addEventListener('resize', onResize)
    return () => window.removeEventListener('resize', onResize)
  }, [thresholdPx])
  return short
}

import { useEffect, useState } from 'react'

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

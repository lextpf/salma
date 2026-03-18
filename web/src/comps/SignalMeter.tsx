import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface SignalMeterProps {
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
}

const HEIGHTS = [5, 7, 9, 11]

// Four rising bars filled to the confidence tier's rank in the tier color; the
// rest sit empty. With no confidence data every bar is neutral (no false LOW).
export default function SignalMeter({ confidence, exactMatch }: SignalMeterProps) {
  const info = tierFor({ confidence, exactMatch })
  return (
    <span
      aria-hidden="true"
      style={{ display: 'inline-flex', alignItems: 'flex-end', gap: 1.5, height: 11 }}
    >
      {HEIGHTS.map((h, i) => (
        <span
          key={h}
          style={{
            width: 3,
            height: h,
            borderRadius: 1,
            background: info.hasData && i < info.rank ? info.color : 'var(--meter-empty)',
          }}
        />
      ))}
    </span>
  )
}

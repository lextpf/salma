import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface SignalMeterProps {
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
  lit?: boolean
}

const HEIGHTS = [5, 7, 9, 11, 13]

export default function SignalMeter({ confidence, exactMatch, lit = false }: SignalMeterProps) {
  // omit the grade when confidence data is absent.
  const info = tierFor({ confidence, exactMatch })
  const filled = info.hasData ? info.bars : 0
  const letter = info.hasData ? info.grade : ''

  return (
    <span
      style={{
        position: 'relative',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'flex-end',
        gap: 7,
      }}
    >
      <span
        aria-hidden="true"
        style={{ display: 'inline-flex', alignItems: 'flex-end', gap: 2, height: 13 }}
      >
        {HEIGHTS.map((h, i) => {
          const on = i < filled
          return (
            <span
              key={h}
              style={{
                width: 3,
                height: h,
                background: on
                  ? info.color
                  : lit ? 'var(--tick)' : 'var(--meter-empty)',
              }}
            />
          )
        })}
      </span>
      <span
        aria-label={info.hasData ? info.label : undefined}
        style={{
          width: 22,
          textAlign: 'right',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          fontWeight: 700,
          letterSpacing: '0.06em',
          color: info.hasData ? info.color : 'var(--ink-faint)',
        }}
      >
        {letter || '-'}
      </span>
    </span>
  )
}

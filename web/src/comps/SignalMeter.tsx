import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface SignalMeterProps {
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
  /** Selected rows step their unfilled bars up so the meter survives --sel-bg. */
  lit?: boolean
}

const HEIGHTS = [5, 7, 9, 11, 13]

/**
 * Five rising bars filled to the confidence tier, followed by the tier letter.
 *
 * Square bars and flat fills, with no bloom on any of them. A selected row
 * carries a --sel-bg wash that swallows the faint unfilled bars, so `lit`
 * raises those to --tick and the meter keeps its full five-bar shape. With no
 * confidence data every bar is neutral and the letter is withheld, so the
 * meter never shows a false `LOW`.
 */
export default function SignalMeter({ confidence, exactMatch, lit = false }: SignalMeterProps) {
  const info = tierFor({ confidence, exactMatch })
  const filled = info.hasData ? info.bars : 0
  // A, B, C, D reads as a grade. An initial letter would only repeat the tier
  // name the pill already carries.
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

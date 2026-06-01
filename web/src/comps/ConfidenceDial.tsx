import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface ConfidenceDialProps {
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
  /** Below ~700px of content width the dial degrades to an inline chip. */
  compact?: boolean
}

const SIZE = 108
const RING = 9

/**
 * The inspector's hero instrument: a plain ring on the card plane with a mono
 * numeral at its centre.
 *
 * The ring is a conic-gradient with hard stops, so it draws two flat arcs, tier
 * colour against --meter-empty, rather than a fade. That is the same mark an
 * SVG arc would make without the extra element. The disc behind it is --card,
 * which is what makes the shape read as a ring. With no confidence data the
 * ring is neutral and the numeral is "--".
 */
export default function ConfidenceDial({ confidence, exactMatch, compact = false }: ConfidenceDialProps) {
  const info = tierFor({ confidence, exactMatch })
  const turn = info.hasData ? info.pct / 100 : 0

  if (compact) {
    return (
      <span
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 8,
          flexShrink: 0,
          padding: '5px 11px',
          // A degraded readout, not a status tag: it takes the control radius
          // rather than the pill radius so it reads as part of the instrument.
          borderRadius: 'var(--radius-ctrl)',
          border: `1px solid ${info.hasData ? info.pill.bd : 'var(--rule)'}`,
          background: info.hasData ? info.pill.bg : 'transparent',
        }}
      >
        <span
          className="tabular-nums"
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-lg)',
            fontWeight: 600,
            letterSpacing: 'var(--tr-hero)',
            color: info.hasData ? 'var(--ink)' : 'var(--ink-5)',
          }}
        >
          {info.hasData ? info.pct : '--'}
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-nano)',
            textTransform: 'uppercase',
            letterSpacing: 'var(--tr-kicker)',
            color: info.hasData ? info.color : 'var(--ink-5)',
          }}
        >
          {info.hasData ? info.label : 'NO DATA'}
        </span>
      </span>
    )
  }

  return (
    <div
      style={{
        position: 'relative',
        width: SIZE,
        height: SIZE,
        flexShrink: 0,
        borderRadius: 'var(--radius-full)',
        background: info.hasData
          ? `conic-gradient(from -90deg, ${info.color} 0turn ${turn}turn, var(--meter-empty) ${turn}turn 1turn)`
          : 'var(--meter-empty)',
      }}
    >
      <div
        style={{
          position: 'absolute',
          inset: RING,
          borderRadius: 'var(--radius-full)',
          background: 'var(--card)',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
          gap: 1,
        }}
      >
        <span
          className="tabular-nums"
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-hero-sm)',
            fontWeight: 600,
            letterSpacing: 'var(--tr-hero)',
            lineHeight: 1,
            color: info.hasData ? 'var(--ink)' : 'var(--ink-5)',
          }}
        >
          {info.hasData ? info.pct : '--'}
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-nano)',
            textTransform: 'uppercase',
            letterSpacing: 'var(--tr-kicker)',
            color: info.hasData ? info.color : 'var(--ink-5)',
          }}
        >
          {info.hasData ? info.label : 'NO DATA'}
        </span>
      </div>
    </div>
  )
}

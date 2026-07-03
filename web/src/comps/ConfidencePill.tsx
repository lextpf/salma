import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface ConfidencePillProps {
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
  size?: 'sm' | 'md'
}

export default function ConfidencePill({ confidence, exactMatch, size = 'md' }: ConfidencePillProps) {
  // render nothing when confidence data is absent.
  const info = tierFor({ confidence, exactMatch })
  if (!info.hasData) {
    return null
  }

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        flexShrink: 0,
        padding: size === 'sm' ? '2px 8px' : '3px 9px',
        borderRadius: 'var(--radius-chip)',
        border: `1px solid ${info.pill.bd}`,
        background: info.pill.bg,
        color: info.pill.fg,
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
        fontWeight: 700,
        textTransform: 'uppercase',
        letterSpacing: 'var(--tr-chip)',
        whiteSpace: 'nowrap',
      }}
    >
      <span
        aria-hidden="true"
        style={{ width: 4, height: 4, borderRadius: 'var(--radius-full)', background: info.pill.fg }}
      />
      {info.label}
    </span>
  )
}

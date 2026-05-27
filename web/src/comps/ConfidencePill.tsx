import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface ConfidencePillProps {
  // Accepts a full ConfidenceScore, a bare composite number, or nothing.
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
  size?: 'sm' | 'md'
}

/**
 * The four-tier confidence chip.
 *
 * Every tier is a tinted outline, never a solid fill and never lit: the tier
 * hue is the text, the dot and the border, over a flat wash of itself at about
 * 10%. A filled `EXACT` chip would need its own contrast ink and would read
 * louder than the dial beside it. The chip takes --radius-chip like every other
 * micro-tag; only the status dot is round. With no confidence data it renders
 * nothing, rather than a misleading `LOW`.
 */
export default function ConfidencePill({ confidence, exactMatch, size = 'md' }: ConfidencePillProps) {
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

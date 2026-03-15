import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface ConfidencePillProps {
  // Accepts a full ConfidenceScore, a bare composite number, or nothing.
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
  size?: 'sm' | 'md'
}

// The v5 four-tier confidence chip: EXACT filled ink, HIGH ink outline, PARTIAL
// ochre, LOW red. Renders nothing when there is no confidence data (rather than
// a misleading LOW). Tier + colors come from the shared confidence helper.
export default function ConfidencePill({ confidence, exactMatch, size = 'md' }: ConfidencePillProps) {
  const info = tierFor({ confidence, exactMatch })
  if (!info.hasData) {
    return null
  }

  const fontSize = 'var(--fs-micro)'
  const padding = size === 'sm' ? '2px 7px' : '3px 9px'

  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        padding,
        borderRadius: 5,
        border: `1px solid ${info.pill.bd}`,
        background: info.pill.bg,
        color: info.pill.fg,
        fontFamily: 'var(--font-mono)',
        fontSize,
        fontWeight: 600,
        letterSpacing: '0.05em',
        whiteSpace: 'nowrap',
      }}
    >
      {info.label}
    </span>
  )
}

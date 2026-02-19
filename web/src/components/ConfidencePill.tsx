import type { ConfidenceBand, ConfidenceScore } from '../types'

const BAND_COLOR: Record<ConfidenceBand, string> = {
  high: 'var(--moss)',
  medium: 'var(--ochre)',
  low: 'var(--danger)',
}

const BAND_LABEL: Record<ConfidenceBand, string> = {
  high: 'HIGH',
  medium: 'MED',
  low: 'LOW',
}

interface ConfidencePillProps {
  confidence?: ConfidenceScore
  size?: 'sm' | 'md' | 'lg'
  /** When true, hides the percentage and only renders the band dot + label. */
  compact?: boolean
}

export default function ConfidencePill({
  confidence,
  size = 'md',
  compact = false,
}: ConfidencePillProps) {
  if (!confidence) {
    return null
  }
  const band = confidence.band
  const color = BAND_COLOR[band] ?? 'var(--ink-3)'
  const label = BAND_LABEL[band] ?? band.toUpperCase()
  const pct = Math.round(confidence.composite * 100)

  const fontSize = size === 'lg' ? 13 : size === 'sm' ? 10 : 11
  const dot = size === 'lg' ? 8 : 6
  const gap = size === 'lg' ? 8 : 6

  return (
    <span
      className="flex items-center"
      style={{
        gap,
        fontFamily: 'var(--font-mono)',
        fontSize,
        letterSpacing: '0.12em',
        textTransform: 'uppercase',
        color: 'var(--ink-3)',
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: dot,
          height: dot,
          borderRadius: '50%',
          background: color,
          boxShadow: band === 'high' ? `0 0 6px ${color}55` : 'none',
        }}
      />
      <span style={{ color: 'var(--ink-2)' }}>{label}</span>
      {!compact && (
        <span
          className="tabular-nums"
          style={{ color: 'var(--ink-3)', letterSpacing: '0.04em' }}
        >
          {pct}%
        </span>
      )}
    </span>
  )
}

import { tierFor } from '../confidence'
import type { ConfidenceScore } from '../types'

interface ConfidenceDialProps {
  confidence?: ConfidenceScore | number | null
  exactMatch?: boolean
}

const R = 44
const CIRC = 2 * Math.PI * R // ~276.46

// 112px SVG ring whose filled arc and center percentage track the confidence
// tier. Tuned for the 112px spec-rail slot. Shows a neutral ring + "--" when no
// confidence data is available.
export default function ConfidenceDial({ confidence, exactMatch }: ConfidenceDialProps) {
  const info = tierFor({ confidence, exactMatch })
  const offset = CIRC * (1 - (info.hasData ? info.pct / 100 : 0))

  return (
    <div style={{ position: 'relative', width: 112, height: 112 }}>
      <svg width={112} height={112} viewBox="0 0 112 112">
        <circle cx="56" cy="56" r={R} fill="none" stroke="var(--meter-empty)" strokeWidth="8" />
        {info.hasData && (
          <circle
            cx="56"
            cy="56"
            r={R}
            fill="none"
            stroke={info.color}
            strokeWidth="8"
            strokeLinecap="round"
            strokeDasharray={CIRC}
            strokeDashoffset={offset}
            transform="rotate(-90 56 56)"
            style={{ transition: 'stroke-dashoffset 500ms cubic-bezier(0.21, 0.9, 0.3, 1)' }}
          />
        )}
      </svg>
      <div
        style={{
          position: 'absolute',
          inset: 0,
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          justifyContent: 'center',
        }}
      >
        <span
          className="tabular-nums"
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-display)',
            fontWeight: 600,
            letterSpacing: '-0.03em',
            color: info.hasData ? info.color : 'var(--ink-5)',
          }}
        >
          {info.hasData ? info.pct : '--'}
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            letterSpacing: '0.14em',
            color: 'var(--ink-6)',
            marginTop: 1,
          }}
        >
          PERCENT
        </span>
      </div>
    </div>
  )
}

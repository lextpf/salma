import type { ConfidenceComponents } from '../types'

interface ConfidenceBreakdownProps {
  components: ConfidenceComponents
}

interface BarRowProps {
  label: string
  value: number
  hint: string
}

// component scores use exact color at 0.98 without claiming an exact run.
function BarRow({ label, value, hint }: BarRowProps) {
  const pct = Math.round(value * 100)
  const fill =
    value >= 0.98 ? 'var(--tier-exact)'
      : value >= 0.85 ? 'var(--tier-high)'
        : value >= 0.6 ? 'var(--tier-partial)'
          : 'var(--tier-low)'
  return (
    <div className="flex items-center" style={{ gap: 12 }}>
      <span
        style={{
          minWidth: 90,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-kicker)',
          color: 'var(--ink-5)',
        }}
        title={hint}
      >
        {label}
      </span>
      <div
        style={{
          flex: 1,
          height: 6,
          background: 'var(--meter-empty)',
          borderRadius: 'var(--radius-chip)',
          overflow: 'hidden',
        }}
      >
        <div
          style={{
            width: `${pct}%`,
            height: '100%',
            background: fill,
            transition: 'width 220ms ease',
          }}
        />
      </div>
      <span
        className="tabular-nums"
        style={{
          minWidth: 36,
          textAlign: 'right',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-body)',
          color: 'var(--ink-3)',
          letterSpacing: '0.02em',
        }}
      >
        {pct}%
      </span>
    </div>
  )
}

export default function ConfidenceBreakdown({ components }: ConfidenceBreakdownProps) {
  // keep hints synchronized with `inference_diagnostics.rs` calculations.
  return (
    <div className="flex flex-col" style={{ gap: 6, width: '100%' }}>
      <BarRow
        label="Evidence"
        value={components.evidence}
        hint="Graded by the strongest evidence reason: unique file match 1.0, no unique match 0.5, extra file produced 0.3."
      />
      <BarRow
        label="Propagation"
        value={components.propagation}
        hint="1.0 when forced by FOMOD spec or unique evidence; 0.0 when CSP-decided."
      />
      <BarRow
        label="Repro"
        value={components.repro}
        hint="How much of the installed tree the simulated selection reproduced. 1.0 on an exact match."
      />
      <BarRow
        label="Ambiguity"
        value={components.ambiguity}
        hint="Graded by how many close alternatives the group had: none 1.0, one 0.6, two 0.4, three or more 0.2."
      />
    </div>
  )
}

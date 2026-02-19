import type { ConfidenceComponents } from '../types'

interface ConfidenceBreakdownProps {
  components: ConfidenceComponents
}

interface BarRowProps {
  label: string
  value: number
  hint: string
}

function BarRow({ label, value, hint }: BarRowProps) {
  const pct = Math.round(value * 100)
  const fill =
    value >= 0.85 ? 'var(--moss)' : value >= 0.5 ? 'var(--ochre)' : 'var(--danger)'
  return (
    <div className="flex items-center" style={{ gap: 12 }}>
      <span
        className="ui-label"
        style={{ minWidth: 90, fontSize: 10, letterSpacing: '0.18em' }}
        title={hint}
      >
        {label}
      </span>
      <div
        style={{
          flex: 1,
          height: 6,
          background: 'var(--paper-3)',
          borderRadius: 3,
          overflow: 'hidden',
          border: '1px solid var(--rule-soft)',
        }}
      >
        <div
          style={{
            width: `${pct}%`,
            height: '100%',
            background: fill,
            transition: 'width 200ms ease',
          }}
        />
      </div>
      <span
        className="tabular-nums"
        style={{
          minWidth: 36,
          textAlign: 'right',
          fontFamily: 'var(--font-mono)',
          fontSize: 12,
          color: 'var(--ink-3)',
          letterSpacing: '0.04em',
        }}
      >
        {pct}%
      </span>
    </div>
  )
}

export default function ConfidenceBreakdown({ components }: ConfidenceBreakdownProps) {
  return (
    <div className="flex flex-col" style={{ gap: 6, width: '100%' }}>
      <BarRow
        label="Evidence"
        value={components.evidence}
        hint="Fraction of plugin files uniquely matching the target tree."
      />
      <BarRow
        label="Propagation"
        value={components.propagation}
        hint="1.0 when forced by FOMOD spec or unique evidence; 0.0 when CSP-decided."
      />
      <BarRow
        label="Repro"
        value={components.repro}
        hint="1 - (mismatched destinations / total). High = simulation reproduces target."
      />
      <BarRow
        label="Ambiguity"
        value={components.ambiguity}
        hint="1 - (close-evidence alternatives / total). High = unambiguous pick."
      />
    </div>
  )
}

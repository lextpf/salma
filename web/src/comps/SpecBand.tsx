import type { RunDiagnostics } from '../types'

interface SpecBandProps {
  diagnostics?: RunDiagnostics
}

/** One figure and its label. The figure leads; the label explains it. */
function Figure({ label, value, tone }: { label: string; value: string; tone?: string }) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'baseline', gap: 6, minWidth: 0 }}>
      <span
        className="tabular-nums"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-body)',
          fontWeight: 600,
          color: tone ?? 'var(--ink)',
          whiteSpace: 'nowrap',
        }}
      >
        {value}
      </span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-chip)',
          color: 'var(--ink-5)',
          whiteSpace: 'nowrap',
        }}
      >
        {label}
      </span>
    </span>
  )
}

/**
 * The record's inference result, as one line.
 *
 * Only the figures that decide whether the Diagnostics tab is worth opening:
 * how much reproduced, how many groups the solver had to resolve, and how long
 * it took. The full report belongs in that tab, so resist growing this back
 * into a block of rows.
 *
 * Faults are the exception. `missing` and `extra` print only when non-zero: a
 * row of zeroes trains a reader to stop looking, while a red figure appearing
 * where there was nothing is what gets noticed.
 */
export default function SpecBand({ diagnostics }: SpecBandProps) {
  if (!diagnostics) {
    return (
      <div
        style={{
          flexShrink: 0,
          padding: '10px 22px 14px',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-label)',
          color: 'var(--ink-5)',
        }}
      >
        // no diagnostics recorded for this record
      </div>
    )
  }

  const { repro, groups, timings_ms: timings } = diagnostics
  const faults = repro.missing + repro.extra + repro.size_mismatch + repro.hash_mismatch

  return (
    <div
      style={{
        flexShrink: 0,
        padding: '10px 22px 14px',
        display: 'flex',
        alignItems: 'baseline',
        gap: '8px 26px',
        flexWrap: 'wrap',
      }}
    >
      <Figure
        label="reproduced"
        value={repro.reproduced.toLocaleString()}
        tone={faults === 0 ? 'var(--moss)' : undefined}
      />
      {repro.missing > 0 && (
        <Figure label="missing" value={repro.missing.toLocaleString()} tone="var(--danger)" />
      )}
      {repro.extra > 0 && (
        <Figure label="extra" value={repro.extra.toLocaleString()} tone="var(--brass)" />
      )}
      {repro.size_mismatch + repro.hash_mismatch > 0 && (
        <Figure
          label="mismatched"
          value={(repro.size_mismatch + repro.hash_mismatch).toLocaleString()}
          tone="var(--brass)"
        />
      )}
      <Figure label={groups.total === 1 ? 'group' : 'groups'} value={groups.total.toLocaleString()} />
      <Figure label="solve" value={`${timings.solve.toLocaleString()} ms`} />
      <span style={{ flex: 1 }} />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          color: 'var(--ink-faint)',
          whiteSpace: 'nowrap',
        }}
      >
        {diagnostics.phase_reached || 'n/a'}
      </span>
    </div>
  )
}

import ConfidenceBreakdown from '../../ConfidenceBreakdown'
import type { RunDiagnostics } from '../../../types'

interface DiagnosticsTabProps {
  diagnostics?: RunDiagnostics
}

interface KvEntry {
  k: string
  v: string
  color?: string
}

function KvRow({ entry }: { entry: KvEntry }) {
  return (
    <div
      style={{
        display: 'flex',
        justifyContent: 'space-between',
        gap: 10,
        padding: '3px 0',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-label)',
      }}
    >
      <span style={{ color: 'var(--ink-5)' }}>{entry.k}</span>
      <span className="tabular-nums" style={{ color: entry.color ?? 'var(--ink)' }}>
        {entry.v}
      </span>
    </div>
  )
}

function KvCard({ title, entries }: { title: string; entries: KvEntry[] }) {
  return (
    <div
      style={{
        border: '1px solid var(--rule-soft)',
        borderRadius: 9,
        padding: '13px 14px',
        boxShadow: 'var(--shadow-elevation-1)',
      }}
    >
      <div
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: '0.12em',
          textTransform: 'uppercase',
          color: 'var(--ink-5)',
          marginBottom: 11,
        }}
      >
        {title}
      </div>
      {entries.map(entry => (
        <KvRow key={entry.k} entry={entry} />
      ))}
    </div>
  )
}

function Divider({ label }: { label: string }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 9, marginBottom: 16 }}>
      <span aria-hidden="true" style={{ width: 5, height: 5, background: 'var(--ink)' }} />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: '0.14em',
          textTransform: 'uppercase',
          color: 'var(--ink-5)',
        }}
      >
        {label}
      </span>
      <span style={{ flex: 1, height: 1, background: 'var(--rule-faint)' }} />
    </div>
  )
}

// Diagnostics tab: the full inference breakdown. The component-confidence bars
// (reused ConfidenceBreakdown) sit above three mono k/v cards for reproduction,
// group resolution, and stage timings. Degrades when the record has no
// diagnostics block.
export default function DiagnosticsTab({ diagnostics }: DiagnosticsTabProps) {
  if (!diagnostics) {
    return (
      <p style={{ margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-label)', color: 'var(--ink-5)' }}>
        // no diagnostics recorded for this record
      </p>
    )
  }

  const { repro, groups, timings_ms: timings, confidence } = diagnostics

  const reproEntries: KvEntry[] = [
    { k: 'reproduced', v: String(repro.reproduced) },
    { k: 'missing', v: String(repro.missing), color: repro.missing > 0 ? 'var(--danger)' : undefined },
    { k: 'extra', v: String(repro.extra), color: repro.extra > 0 ? 'var(--ochre)' : undefined },
    { k: 'size mm', v: String(repro.size_mismatch), color: repro.size_mismatch > 0 ? 'var(--ochre)' : undefined },
    { k: 'hash mm', v: String(repro.hash_mismatch), color: repro.hash_mismatch > 0 ? 'var(--ochre)' : undefined },
  ]

  const groupEntries: KvEntry[] = [
    { k: 'total', v: String(groups.total) },
    { k: 'propagation', v: String(groups.resolved_by_propagation) },
    { k: 'csp', v: String(groups.resolved_by_csp) },
  ]

  const timingEntries: KvEntry[] = [
    { k: 'list', v: String(timings.list) },
    { k: 'scan', v: String(timings.scan) },
    { k: 'solve', v: String(timings.solve) },
    { k: 'total', v: String(timings.total) },
  ]

  return (
    <div>
      <Divider label="Full breakdown" />

      <div
        style={{
          border: '1px solid var(--rule-soft)',
          borderRadius: 9,
          padding: '14px 16px',
          marginBottom: 14,
          boxShadow: 'var(--shadow-elevation-1)',
        }}
      >
        <div
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            letterSpacing: '0.12em',
            textTransform: 'uppercase',
            color: 'var(--ink-5)',
            marginBottom: 12,
          }}
        >
          Confidence components
        </div>
        <ConfidenceBreakdown components={confidence.components} />
      </div>

      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))', gap: 14 }}>
        <KvCard title="Reproduction" entries={reproEntries} />
        <KvCard title="Groups" entries={groupEntries} />
        <KvCard title="Timings - ms" entries={timingEntries} />
      </div>
    </div>
  )
}

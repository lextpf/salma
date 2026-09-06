import ConfidenceBreakdown from './ConfidenceBreakdown'
import type { RunDiagnostics } from '../types'

interface DiagnosticsTabProps {
  diagnostics?: RunDiagnostics
}

interface KvEntry {
  k: string
  v: string
  color?: string
  /**
   * @brief normalized bar share.
   *
   * values are in [0, 1].
   */
  share?: number
  bar?: string
}

const LABEL: React.CSSProperties = {
  fontFamily: 'var(--font-mono)',
  fontSize: 'var(--fs-micro)',
  letterSpacing: 'var(--tr-chip)',
  textTransform: 'uppercase',
  color: 'var(--ink-5)',
}

function KvRow({ entry }: { entry: KvEntry }) {
  return (
    <div style={{ padding: '4px 0' }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          gap: 10,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-label)',
        }}
      >
        <span style={{ color: 'var(--ink-5)' }}>{entry.k}</span>
        <span className="tabular-nums" style={{ color: entry.color ?? 'var(--ink)' }}>
          {entry.v}
        </span>
      </div>
      {entry.share != null && (
        <div style={{ marginTop: 4, height: 2, background: 'var(--track)' }}>
          <div
            style={{
              height: '100%',
              width: `${Math.max(0, Math.min(100, entry.share * 100))}%`,
              background: entry.bar ?? 'var(--signal)',
            }}
          />
        </div>
      )}
    </div>
  )
}

function KvBlock({ title, entries }: { title: string; entries: KvEntry[] }) {
  return (
    <div style={{ minWidth: 0 }}>
      <div style={{ ...LABEL, paddingBottom: 5 }}>{title}</div>
      {entries.map(entry => (
        <KvRow key={entry.k} entry={entry} />
      ))}
    </div>
  )
}

function SectionBand({ label, note }: { label: string; note?: string }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '5px 10px' }}>
      <span aria-hidden="true" style={{ width: 4, height: 4, flexShrink: 0, background: 'var(--signal)' }} />
      <span style={LABEL}>{label}</span>
      {note && (
        <>
          <span style={{ flex: 1 }} />
          <span className="tabular-nums" style={{ ...LABEL, color: 'var(--ink-faint)' }}>
            {note}
          </span>
        </>
      )}
    </div>
  )
}

export default function DiagnosticsTab({ diagnostics }: DiagnosticsTabProps) {
  if (!diagnostics) {
    return (
      <p style={{ margin: 0, fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-label)', color: 'var(--ink-5)' }}>
        // no diagnostics recorded for this record
      </p>
    )
  }

  const { repro, groups, timings_ms: timings, confidence } = diagnostics

  // include all compared files in the reproduction-rate denominator.
  const accounted =
    repro.reproduced + repro.missing + repro.extra + repro.size_mismatch + repro.hash_mismatch
  const rate = accounted > 0 ? repro.reproduced / accounted : 0
  const faults = accounted - repro.reproduced

  const reproEntries: KvEntry[] = [
    {
      k: 'reproduced',
      v: String(repro.reproduced),
      color: faults === 0 ? 'var(--moss)' : undefined,
      share: rate,
      bar: faults === 0 ? 'var(--moss)' : 'var(--signal)',
    },
    { k: 'missing', v: String(repro.missing), color: repro.missing > 0 ? 'var(--danger)' : undefined },
    { k: 'extra', v: String(repro.extra), color: repro.extra > 0 ? 'var(--brass)' : undefined },
    { k: 'size mm', v: String(repro.size_mismatch), color: repro.size_mismatch > 0 ? 'var(--brass)' : undefined },
    { k: 'hash mm', v: String(repro.hash_mismatch), color: repro.hash_mismatch > 0 ? 'var(--brass)' : undefined },
  ]

  const groupEntries: KvEntry[] = [
    { k: 'total', v: String(groups.total) },
    {
      k: 'propagation',
      v: String(groups.resolved_by_propagation),
      share: groups.total > 0 ? groups.resolved_by_propagation / groups.total : 0,
    },
    {
      k: 'csp',
      v: String(groups.resolved_by_csp),
      share: groups.total > 0 ? groups.resolved_by_csp / groups.total : 0,
      bar: 'var(--brass)',
    },
  ]

  // report time outside measured stages as `other`.
  const measured = timings.list + timings.scan + timings.solve
  const other = Math.max(0, timings.total - measured)
  const denom = Math.max(1, timings.total)
  const timingEntries: KvEntry[] = [
    { k: 'list', v: String(timings.list), share: timings.list / denom },
    { k: 'scan', v: String(timings.scan), share: timings.scan / denom },
    { k: 'solve', v: String(timings.solve), share: timings.solve / denom, bar: 'var(--brass)' },
    { k: 'other', v: String(other), share: other / denom, bar: 'var(--ink-4)' },
    { k: 'total', v: String(timings.total) },
  ]

  const runEntries: KvEntry[] = [
    { k: 'phase', v: diagnostics.phase_reached || 'n/a' },
    {
      k: 'match',
      v: diagnostics.exact_match ? 'exact' : 'partial',
      color: diagnostics.exact_match ? 'var(--moss)' : 'var(--brass)',
    },
    { k: 'nodes', v: diagnostics.nodes_explored.toLocaleString() },
    {
      k: 'cache',
      v: diagnostics.cache.hit ? diagnostics.cache.source || 'hit' : 'miss',
      color: diagnostics.cache.hit ? 'var(--moss)' : 'var(--ink-5)',
    },
  ]

  return (
    <div>
      <SectionBand
        label="Reproduction"
        note={`${Math.round(rate * 100)}% of ${accounted.toLocaleString()} accounted`}
      />

      <div
        style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))',
          gap: '16px 22px',
          padding: '6px 10px 18px',
        }}
      >
        <KvBlock title="Files" entries={reproEntries} />
        <KvBlock title="Groups" entries={groupEntries} />
        <KvBlock title="Timings - ms" entries={timingEntries} />
        <KvBlock title="Run" entries={runEntries} />
      </div>

      {/* TODO: scale composite to a percentage before display. */}
      <SectionBand label="Confidence components" note={`${Math.round(confidence.composite)} composite`} />

      <div style={{ padding: '8px 10px 16px' }}>
        <ConfidenceBreakdown components={confidence.components} />
      </div>
    </div>
  )
}

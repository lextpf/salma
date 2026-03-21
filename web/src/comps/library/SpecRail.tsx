import ConfidenceDial from '../ConfidenceDial'
import ConfidencePill from '../ConfidencePill'
import Chip from '../Chip'
import { tierFromDiagnostics } from '../../confidence'
import type { RunDiagnostics } from '../../types'

interface SpecRailProps {
  diagnostics?: RunDiagnostics
}

function KvRow({ k, value, color }: { k: string; value: string; color?: string }) {
  return (
    <div
      style={{
        display: 'flex',
        justifyContent: 'space-between',
        gap: 10,
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-label)',
      }}
    >
      <span style={{ color: 'var(--ink-5)' }}>{k}</span>
      <span
        className="tabular-nums"
        style={{
          color: color ?? 'var(--ink)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {value}
      </span>
    </div>
  )
}

function RailLabel({ text }: { text: string }) {
  return (
    <div
      style={{
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
        letterSpacing: '0.12em',
        textTransform: 'uppercase',
        color: 'var(--ink-5)',
      }}
    >
      {text}
    </div>
  )
}

// The inspector's left spec-rail: the confidence dial + pill, an inference meta
// block, the reproduction tally, and badge chips. Everything degrades to quiet
// placeholders when the record predates the diagnostics schema.
export default function SpecRail({ diagnostics }: SpecRailProps) {
  const confidence = diagnostics?.confidence
  const exact = diagnostics?.exact_match === true
  const repro = diagnostics?.repro
  const tier = tierFromDiagnostics(diagnostics)

  const badges: { label: string; color: string; title?: string }[] = []
  if (diagnostics) {
    badges.push({ label: tier.label, color: tier.color })
    if (diagnostics.cache?.hit) {
      badges.push({
        label: 'CACHE',
        color: 'var(--moss)',
        title: `Cache hit${diagnostics.cache.source ? ` - ${diagnostics.cache.source}` : ''}`,
      })
    }
  }

  return (
    <div
      style={{
        width: 148,
        flexShrink: 0,
        borderRight: '1px solid var(--rule-soft)',
        background: 'var(--card)',
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        overflowY: 'auto',
        overflowX: 'hidden',
        padding: '20px 14px 14px',
        boxShadow: 'var(--shadow-elevation-1)',
      }}
    >
      <div style={{ margin: '0 auto' }}>
        <ConfidenceDial confidence={confidence} exactMatch={exact} />
      </div>
      <div style={{ textAlign: 'center', margin: '13px 0 16px', minHeight: 18 }}>
        {diagnostics ? (
          <ConfidencePill confidence={confidence} exactMatch={exact} />
        ) : (
          <span
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-micro)',
              letterSpacing: '0.08em',
              color: 'var(--ink-6)',
            }}
          >
            NO DIAGNOSTICS
          </span>
        )}
      </div>

      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 9,
          paddingTop: 15,
          borderTop: '1px solid var(--rule-soft)',
        }}
      >
        <KvRow k="phase" value={diagnostics?.phase_reached || 'n/a'} />
        <KvRow
          k="match"
          value={diagnostics ? (exact ? 'exact' : 'partial') : '--'}
          color={diagnostics ? (exact ? 'var(--ink)' : 'var(--ochre)') : 'var(--ink-5)'}
        />
        <KvRow
          k="nodes"
          value={diagnostics ? diagnostics.nodes_explored.toLocaleString() : '--'}
        />
      </div>

      <div
        style={{
          marginTop: 15,
          paddingTop: 15,
          borderTop: '1px solid var(--rule-soft)',
          display: 'flex',
          flexDirection: 'column',
          gap: 9,
        }}
      >
        <RailLabel text="Reproduction" />
        <KvRow k="reproduced" value={repro ? String(repro.reproduced) : '--'} />
        <KvRow
          k="missing"
          value={repro ? String(repro.missing) : '--'}
          color={repro && repro.missing > 0 ? 'var(--danger)' : 'var(--ink-5)'}
        />
        <KvRow
          k="extra"
          value={repro ? String(repro.extra) : '--'}
          color={repro && repro.extra > 0 ? 'var(--ochre)' : 'var(--ink-5)'}
        />
        <KvRow
          k="hash mm"
          value={repro ? String(repro.hash_mismatch) : '--'}
          color={repro && repro.hash_mismatch > 0 ? 'var(--ochre)' : 'var(--ink-5)'}
        />
      </div>

      <div style={{ flex: 1, minHeight: 16 }} />

      {badges.length > 0 && (
        <div style={{ display: 'flex', flexWrap: 'wrap', gap: 5 }}>
          {badges.map(b => (
            <Chip key={b.label} label={b.label} color={b.color} title={b.title} />
          ))}
        </div>
      )}
    </div>
  )
}

// The six-stage dot tracker in the active job card. The current op (derived from
// the install console) selects the active stage; earlier stages read as done and
// later ones as pending.

import MIcon from '../MIcon'

const STAGES = ['VALIDATE', 'EXTRACT', 'SCAN', 'PARSE', 'INFER', 'WRITE'] as const

// Map a console op token to its stage index (0..5). Unknown, generic INSTALL,
// and the pre-extract upload phase all sit at VALIDATE.
function stageIndexForOp(op: string | null): number {
  switch (op) {
    case 'EXTRACT':
      return 1
    case 'DETECT':
      return 2
    case 'PARSE':
      return 3
    case 'CACHE':
    case 'PROPAGATE':
    case 'CSP':
    case 'SIMULATE':
    case 'INFER':
      return 4
    case 'COPY':
    case 'WRITE':
    case 'DEPLOY':
    case 'CLEANUP':
    case 'DONE':
      return 5
    default:
      return 0
  }
}

interface StageTrackerProps {
  activeOp: string | null
  errored?: boolean
  elapsedLabel?: string
}

export default function StageTracker({ activeOp, errored = false, elapsedLabel }: StageTrackerProps) {
  const active = stageIndexForOp(activeOp)
  const accent = errored ? 'var(--danger)' : 'var(--ink)'

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        padding: '8px 16px',
        borderBottom: '1px solid var(--rule-soft)',
        background: 'var(--sheet)',
      }}
    >
      {STAGES.map((stage, i) => {
        const done = i < active
        const isActive = i === active
        return (
          <span key={stage} style={{ display: 'inline-flex', alignItems: 'center' }}>
            {i > 0 && (
              <span aria-hidden="true" style={{ width: 22, height: 1, background: 'var(--rule)', margin: '0 7px' }} />
            )}
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
              <span
                aria-hidden="true"
                style={{
                  width: 15,
                  height: 15,
                  display: 'inline-flex',
                  alignItems: 'center',
                  justifyContent: 'center',
                  borderRadius: '50%',
                  background: done ? 'var(--ink)' : 'var(--sheet)',
                  border: done
                    ? '1px solid var(--ink)'
                    : isActive
                      ? `1.5px solid ${accent}`
                      : '1px solid var(--rule)',
                  animation: isActive ? 'salma-blink 1.4s infinite' : undefined,
                }}
              >
                {done ? (
                  <MIcon name="check" size={12} weight={600} style={{ color: 'var(--sheet)' }} />
                ) : isActive ? (
                  <span style={{ width: 5, height: 5, borderRadius: '50%', background: accent }} />
                ) : (
                  <span style={{ width: 4, height: 4, borderRadius: '50%', background: 'var(--ink-faint)' }} />
                )}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  letterSpacing: '0.1em',
                  fontWeight: isActive ? 600 : 400,
                  color: isActive ? accent : done ? 'var(--ink-4)' : 'var(--ink-6)',
                }}
              >
                {stage}
              </span>
            </span>
          </span>
        )
      })}
      <div style={{ flex: 1 }} />
      {elapsedLabel && (
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-5)' }}>{elapsedLabel}</span>
      )}
    </div>
  )
}

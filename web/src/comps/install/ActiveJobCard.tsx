import { useEffect, useRef } from 'react'
import MIcon from '../MIcon'
import ProgressFooter from './ProgressFooter'
import StageTracker from './StageTracker'
import { formatForFile } from './formats'
import { FormatTile } from './FormatTile'
import type { ConsoleLine } from '../../useInstallConsole'
import type { InstallationJob } from '../../types'

interface ActiveJobCardProps {
  job: InstallationJob
  // 1-based position of this job in the session (the # column) and the total.
  index: number
  total: number
  lines: ConsoleLine[]
  rawLines: string[]
  onCancel: () => void
}

const PRE: Record<ConsoleLine['state'], string> = {
  done: 'check',
  active: 'play_arrow',
  pending: 'fiber_manual_record',
  error: 'close',
}
const PRE_COLOR: Record<ConsoleLine['state'], string> = {
  done: 'var(--ink-faint)',
  active: 'var(--ink)',
  pending: 'var(--ink-faint)',
  error: 'var(--danger)',
}
const OP_COLOR: Record<ConsoleLine['state'], string> = {
  done: 'var(--ink-4)',
  active: 'var(--ink)',
  pending: 'var(--ink-faint)',
  error: 'var(--danger)',
}
const MSG_COLOR: Record<ConsoleLine['state'], string> = {
  done: 'var(--ink-3)',
  active: 'var(--ink)',
  pending: 'var(--ink-6)',
  error: 'var(--danger)',
}

const STREAM_WINDOW = 8

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

function fmtTime(ms: number): string {
  const d = new Date(ms)
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
}

function fmtElapsed(job: InstallationJob): string {
  const s = Math.max(0, Math.floor((Date.now() - job.createdAt) / 1000))
  return `elapsed ${pad2(Math.floor(s / 60))}:${pad2(s % 60)}`
}

function fmtSize(bytes?: number): string | null {
  if (bytes == null) return null
  const mib = bytes / (1024 * 1024)
  return mib >= 10 ? `${Math.round(mib)} MiB` : `${mib.toFixed(1)} MiB`
}

// The inline card for the job currently installing: header, five-stage tracker,
// a live op-stream from the [install] log tail, and the striped progress footer.
// Console data is passed in (lifted to InstallPage) so it polls once.
export default function ActiveJobCard({ job, index, total, lines, rawLines, onCancel }: ActiveJobCardProps) {
  const streamRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    const el = streamRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [lines.length])

  const errored = job.status === 'error'
  const spec = formatForFile(job.fileName)
  const size = fmtSize(job.sizeBytes)

  // The active/error op drives the stage tracker; scan from the tail.
  let activeOp: string | null = null
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i].state === 'active' || lines[i].state === 'error') {
      activeOp = lines[i].op
      break
    }
  }

  const visible = lines.slice(Math.max(0, lines.length - STREAM_WINDOW))

  return (
    <div
      style={{
        marginTop: 16,
        border: `1px solid ${errored ? 'var(--tier-low-bd)' : 'var(--rule)'}`,
        borderRadius: 10,
        background: 'var(--card)',
        boxShadow: 'var(--shadow-elevation-2)',
        overflow: 'hidden',
      }}
    >
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 11, padding: '8px 16px', borderBottom: '1px solid var(--rule-soft)' }}>
        <span style={{ color: 'var(--ink-faint)', fontSize: 'var(--fs-micro)', flexShrink: 0 }}>{fmtTime(job.createdAt)}</span>
        <span style={{ color: 'var(--ink-3)', flexShrink: 0 }}>#{pad2(index)}</span>
        <FormatTile spec={spec} size={22} />
        <span
          title={job.fileName}
          style={{ color: 'var(--ink)', fontWeight: 600, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis', minWidth: 0 }}
        >
          {job.fileName}
        </span>
        {size && <span style={{ color: 'var(--ink-5)', fontSize: 'var(--fs-micro)', flexShrink: 0 }}>{size}</span>}
        <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, color: errored ? 'var(--danger)' : 'var(--ink-4)', fontSize: 'var(--fs-micro)', flexShrink: 0 }}>
          <span
            aria-hidden="true"
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              background: errored ? 'var(--danger)' : 'var(--ink)',
              animation: errored ? undefined : 'salma-blink 1.4s infinite',
            }}
          />
          {errored ? 'failed' : 'installing'}
        </span>
        <div style={{ flex: 1 }} />
        <span style={{ color: 'var(--ink-6)', fontSize: 'var(--fs-micro)', flexShrink: 0 }}>
          job {index} / {total}
        </span>
        <button
          type="button"
          onClick={onCancel}
          aria-label="Cancel install"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            padding: '4px 10px',
            border: '1px solid color-mix(in srgb, var(--danger) 35%, transparent)',
            borderRadius: 6,
            background: 'var(--sheet)',
            color: 'var(--danger)',
            fontSize: 'var(--fs-micro)',
            cursor: 'pointer',
            fontFamily: 'inherit',
            flexShrink: 0,
          }}
        >
          <MIcon name="stop" size={12} fill />
          Cancel
        </button>
      </div>

      {/* Stage tracker */}
      <StageTracker activeOp={activeOp} errored={errored} elapsedLabel={fmtElapsed(job)} />

      {/* Op stream */}
      <div
        ref={streamRef}
        style={{ maxHeight: 132, overflowY: 'auto', overflowX: 'hidden', padding: '8px 16px', fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-label)', lineHeight: 1.85 }}
      >
        {visible.length === 0 ? (
          <div style={{ color: 'var(--ink-6)' }}>
            <MIcon name="fiber_manual_record" size={8} style={{ marginRight: 8 }} />waiting for output...
          </div>
        ) : (
          visible.map(c => (
            <div key={c.id} style={{ display: 'flex', gap: 12, alignItems: 'center', padding: '1px 0' }}>
              <MIcon name={PRE[c.state]} size={11} style={{ color: PRE_COLOR[c.state] }} />
              <span style={{ color: 'var(--ink-faint)', flexShrink: 0 }}>{c.time}</span>
              <span style={{ color: OP_COLOR[c.state], flexShrink: 0, fontWeight: 600, letterSpacing: '0.04em', width: 70 }}>{c.op}</span>
              <span style={{ color: MSG_COLOR[c.state], flex: 1, minWidth: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{c.msg}</span>
            </div>
          ))
        )}
      </div>

      {/* Progress footer */}
      <ProgressFooter job={job} rawLines={rawLines} />
    </div>
  )
}

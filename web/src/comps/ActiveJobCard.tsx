import { useEffect, useRef } from 'react'
import MIcon from './MIcon'
import StageMeter from './StageMeter'
import { formatSize } from '../libraryFormat'
import { formatForFile } from './formats'
import { activeOpOf } from '../useInstallConsole'
import { fmtClock, modLeaf, pad2 } from './jobFormat'
import { FormatTile } from './FormatTile'
import type { ConsoleLine } from '../useInstallConsole'
import type { InstallationJob } from '../types'

interface ActiveJobCardProps {
  job: InstallationJob
  // 1-based position of this job in the session (the # column) and the total.
  index: number
  total: number
  lines: ConsoleLine[]
  rawLines: string[]
  onCancel: () => void
}

// Console colour is keyed on the op, not on the line's position in the stream:
// a result is the same result wherever it lands. Ops that report a clean result
// read moss; everything else stays quiet.
//
// Two tokens here are unreachable with the current op vocabulary. They stay so
// that adding the rule that produces them needs no change in this file:
//   - 'WARN' is never derived. OP_RULES in useInstallConsole.ts has no warn
//     rule and deriveOp falls back to 'INSTALL', so the brass branches in
//     opColor and msgColor never run. A warning line takes the colour of
//     whichever other rule its text matches.
//   - 'VALIDATE' is a stage name (stages.ts), not an op, so nothing reaches
//     OK_OPS through it either.
const OK_OPS = new Set(['VALIDATE', 'CSP', 'SIMULATE', 'DONE', 'PROPAGATE'])

function opColor(op: string, live: boolean): string {
  if (op === 'WARN') return 'var(--brass)'
  if (op === 'ERROR') return 'var(--danger)'
  if (live) return 'var(--signal)'
  if (OK_OPS.has(op)) return 'var(--moss)'
  return 'var(--ink-5)'
}

function msgColor(op: string, live: boolean): string {
  if (op === 'WARN') return 'var(--warn-text)'
  if (op === 'ERROR') return 'var(--danger)'
  return live ? 'var(--ink-2)' : 'var(--ink-4)'
}

const STREAM_WINDOW = 8

/**
 * The panel for the job currently installing, and the focal point of the
 * Install screen: a header band, the progress meter, and a live op-stream from
 * the [install] log tail.
 *
 * Three stacked groups with no surface and no hairlines of their own. What
 * marks it as the live row is the signal left edge and the meter, never a
 * shadow or a bloom.
 */
export default function ActiveJobCard({ job, index, total, lines, rawLines, onCancel }: ActiveJobCardProps) {
  const streamRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    const el = streamRef.current
    if (el) el.scrollTop = el.scrollHeight
  }, [lines.length])

  const errored = job.status === 'error'
  const spec = formatForFile(job.fileName)
  const size = job.sizeBytes != null ? formatSize(job.sizeBytes) : null
  const dest = modLeaf(job)
  const activeOp = activeOpOf(lines)

  const visible = lines.slice(Math.max(0, lines.length - STREAM_WINDOW))

  const meta = [
    size,
    `job ${pad2(index)} / ${pad2(total)}`,
    fmtClock(job.createdAt),
    dest ? `mods/${dest}` : null,
  ].filter(Boolean).join('  |  ')

  return (
    <div
      className="rise"
      style={{
        marginTop: 16,
        borderLeft: `2px solid ${errored ? 'var(--danger)' : 'var(--signal)'}`,
        borderRadius: 'var(--radius-card)',
        overflow: 'hidden',
      }}
    >
      {/* Band 1 - header */}
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '12px 16px',
          background: errored ? 'var(--danger-wash)' : undefined,
        }}
      >
        <FormatTile spec={spec} size={28} />
        <div style={{ display: 'flex', flexDirection: 'column', gap: 3, minWidth: 0, flex: 1 }}>
          <div
            title={job.fileName}
            style={{
              fontFamily: 'var(--font-body)',
              fontSize: 'var(--fs-head)',
              fontWeight: 700,
              letterSpacing: '-0.02em',
              color: 'var(--ink)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {job.fileName}
          </div>
          <div
            className="tabular-nums"
            style={{
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-meta)',
              color: 'var(--ink-5)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {meta}
          </div>
        </div>
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            flexShrink: 0,
            padding: '3px 9px',
            borderRadius: 'var(--radius-chip)',
            border: `1px solid ${errored ? 'var(--danger-bd)' : 'var(--signal-bd)'}`,
            background: errored ? 'var(--tier-low-bg)' : 'var(--signal-wash-chip)',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            fontWeight: 600,
            letterSpacing: 'var(--tr-chip)',
            textTransform: 'uppercase',
            color: errored ? 'var(--danger)' : 'var(--signal-2)',
          }}
        >
          <span
            aria-hidden="true"
            className={errored ? undefined : 'blink'}
            style={{
              width: 5,
              height: 5,
              background: errored ? 'var(--danger)' : 'var(--signal)',
            }}
          />
          {errored ? 'failed' : 'installing'}
        </span>
        <button
          type="button"
          className="btn"
          onClick={onCancel}
          aria-label="Cancel install"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
            height: 28,
            padding: '0 11px',
            border: '1px solid var(--rule-ctrl)',
            borderRadius: 'var(--radius-ctrl)',
            background: 'var(--btn-bg)',
            color: 'var(--ink-3)',
            fontFamily: 'inherit',
            fontSize: 'var(--fs-sm)',
            cursor: 'pointer',
            flexShrink: 0,
          }}
        >
          <MIcon name="close" size={14} />
          Cancel
        </button>
      </div>

      {/* Band 2 - the progress instrument: stages, segments, percent, metrics */}
      <StageMeter job={job} rawLines={rawLines} activeOp={activeOp} errored={errored} />

      {/* Band 3 - console */}
      <div
        ref={streamRef}
        style={{
          maxHeight: 150,
          overflowY: 'auto',
          overflowX: 'hidden',
          padding: '11px 16px 12px',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-mono)',
          lineHeight: 1.8,
        }}
      >
        {visible.length === 0 ? (
          <div style={{ color: 'var(--ink-5)' }}>waiting for output...</div>
        ) : (
          visible.map((c, i) => {
            const live = i === visible.length - 1 && !errored
            return (
              <div key={c.id} style={{ display: 'flex', alignItems: 'center', gap: 11, whiteSpace: 'nowrap' }}>
                <span className="tabular-nums" style={{ color: 'var(--ink-faint)', flexShrink: 0 }}>{c.time}</span>
                <span style={{ color: opColor(c.op, live), flexShrink: 0, fontWeight: 600, width: 78 }}>
                  [{c.op}]
                </span>
                <span
                  style={{
                    color: msgColor(c.op, live),
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {c.msg}
                </span>
                {live && (
                  <span
                    aria-hidden="true"
                    className="caret"
                    style={{
                      display: 'inline-block',
                      width: 7,
                      height: 13,
                      flexShrink: 0,
                      background: 'var(--signal)',
                    }}
                  />
                )}
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}

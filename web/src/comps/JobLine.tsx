import { useState } from 'react'
import { Link } from 'react-router-dom'
import MIcon from './MIcon'
import type { InstallationJob } from '../types'
import { formatSize } from '../libraryFormat'
import { formatForFile } from './formats'
import { FormatTile, QuietTile } from './FormatTile'
import { fmtClock, modLeaf, pad2 } from './jobFormat'

interface JobLineProps {
  job: InstallationJob
  index: number
}

const STATE: Record<string, { label: string; fg: string; bg: string }> = {
  completed: { label: 'Done', fg: 'var(--moss)', bg: 'var(--ok-bg)' },
  error: { label: 'Failed', fg: 'var(--danger)', bg: 'var(--tier-low-bg)' },
  pending: { label: 'Queued', fg: 'var(--ink-5)', bg: 'var(--chip-bg)' },
}

function fmtDuration(job: InstallationJob): string | null {
  if (job.completedAt == null) return null
  const s = (job.completedAt - job.createdAt) / 1000
  return s >= 100 ? `${Math.round(s)}s` : `${s.toFixed(1)}s`
}

export default function JobLine({ job, index }: JobLineProps) {
  const [expanded, setExpanded] = useState(false)
  const spec = formatForFile(job.fileName)
  const isError = job.status === 'error'
  const isQueued = job.status === 'pending'
  const canExpand = job.status === 'completed' || isError

  const size = job.sizeBytes != null ? formatSize(job.sizeBytes) : null
  const duration = fmtDuration(job)
  const dest = modLeaf(job)
  const state = STATE[job.status] ?? {
    label: 'Running',
    fg: 'var(--signal)',
    bg: 'var(--signal-wash-chip)',
  }

  const meta = isError
    ? job.error || 'failed'
    : isQueued
      ? ['queued', size].filter(Boolean).join(' | ')
      : [dest ? `mods/${dest}` : null, duration, size].filter(Boolean).join(' | ')

  const toggle = () => {
    if (canExpand) setExpanded(e => !e)
  }

  return (
    <div onClick={e => { e.stopPropagation(); }} style={{ margin: '0 -10px' }}>
      <div
        role={canExpand ? 'button' : undefined}
        tabIndex={canExpand ? 0 : undefined}
        aria-expanded={canExpand ? expanded : undefined}
        onClick={toggle}
        onKeyDown={
          canExpand
            ? e => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault()
                  toggle()
                }
              }
            : undefined
        }
        className="fm-row"
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 11,
          padding: '6px 10px',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-mono)',
          cursor: canExpand ? 'pointer' : 'default',
        }}
      >
        <span className="tabular-nums" style={{ color: 'var(--ink-5)', flexShrink: 0 }}>
          {fmtClock(job.createdAt)}
        </span>
        <span className="tabular-nums" style={{ color: 'var(--ink-5)', flexShrink: 0 }}>
          #{pad2(index)}
        </span>
        {isQueued ? <QuietTile size={18} /> : <FormatTile spec={spec} size={18} />}
        <span
          title={job.fileName}
          style={{
            flex: '1 1 auto',
            minWidth: 0,
            color: isError ? 'var(--danger-2)' : isQueued ? 'var(--ink-5)' : 'var(--ink-2)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          {job.fileName}
        </span>
        <span
          style={{
            width: 62,
            flexShrink: 0,
            textAlign: 'center',
            padding: '2px 0',
            borderRadius: 'var(--radius-chip)',
            background: state.bg,
            color: state.fg,
            fontSize: 'var(--fs-micro)',
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: 'var(--tr-chip)',
          }}
        >
          {state.label}
        </span>
        <span
          style={{
            width: 250,
            flexShrink: 0,
            textAlign: 'right',
            fontSize: 'var(--fs-label)',
            color: isError ? 'var(--danger)' : 'var(--ink-5)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          {meta}
        </span>
        <MIcon
          name={expanded ? 'expand_less' : 'expand_more'}
          size={15}
          style={{ color: 'var(--ink-5)', flexShrink: 0, visibility: canExpand ? 'visible' : 'hidden' }}
        />
      </div>
      {expanded && canExpand && (
        <div
          style={{
            padding: '9px 10px 11px 58px',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            color: 'var(--ink-5)',
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'flex-start',
            gap: 6,
          }}
        >
          {job.modPath && (
            <div>
              target - <span style={{ color: 'var(--ink-3)' }}>{job.modPath}</span>
            </div>
          )}
          {isError && job.error && <div style={{ color: 'var(--danger)' }}>error - {job.error}</div>}
          <Link
            className="btn"
            to="/logs"
            style={{
              display: 'inline-flex',
              alignItems: 'center',
              gap: 6,
              height: 26,
              padding: '0 10px',
              color: 'var(--ink-3)',
              textDecoration: 'none',
              border: '1px solid var(--rule-ctrl)',
              borderRadius: 'var(--radius-ctrl)',
              background: 'var(--btn-bg)',
            }}
          >
            <MIcon name="receipt_long" size={13} />
            View log
          </Link>
        </div>
      )}
    </div>
  )
}

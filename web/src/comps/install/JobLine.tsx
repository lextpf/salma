import { useState } from 'react'
import { Link } from 'react-router-dom'
import MIcon from '../MIcon'
import type { InstallationJob } from '../../types'
import { formatForFile } from './formats'
import { FormatTile } from './FormatTile'

interface JobLineProps {
  job: InstallationJob
  // 1-based position of this job in the session (the # column).
  index: number
}

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

function fmtTime(ms: number): string {
  const d = new Date(ms)
  return `${pad2(d.getHours())}:${pad2(d.getMinutes())}:${pad2(d.getSeconds())}`
}

function fmtSize(bytes?: number): string | null {
  if (bytes == null) return null
  const mib = bytes / (1024 * 1024)
  return mib >= 10 ? `${Math.round(mib)} MiB` : `${mib.toFixed(1)} MiB`
}

function fmtDuration(job: InstallationJob): string | null {
  if (job.completedAt == null) return null
  const s = (job.completedAt - job.createdAt) / 1000
  return s >= 100 ? `${Math.round(s)}s` : `${s.toFixed(1)}s`
}

// One collapsed feed row for a terminal (completed / error) or queued job.
// Completed and error rows expand on click to reveal the target and a link to
// the log; queued rows are read-only (removal is not wired this pass).
export default function JobLine({ job, index }: JobLineProps) {
  const [expanded, setExpanded] = useState(false)
  const spec = formatForFile(job.fileName)
  const isError = job.status === 'error'
  const isQueued = job.status === 'pending'
  const canExpand = job.status === 'completed' || isError

  const size = fmtSize(job.sizeBytes)
  const duration = fmtDuration(job)

  const meta = (() => {
    if (isError) return job.error || 'failed'
    if (isQueued) return `queued${size ? ` - ${size}` : ''}`
    const rest: string[] = []
    if (duration) rest.push(duration)
    if (size) rest.push(size)
    const restText = rest.length > 0 ? ` - ${rest.join(' - ')}` : ''
    if (job.modPath) {
      return (
        <>
          <MIcon name="arrow_forward" size={11} /> {job.modPath}
          {restText}
        </>
      )
    }
    return rest.join(' - ')
  })()

  const toggle = () => {
    if (canExpand) setExpanded(e => !e)
  }

  return (
    <div onClick={e => { e.stopPropagation(); }} style={{ margin: '2px -10px 0' }}>
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
        style={{
          display: 'flex',
          alignItems: 'center',
          gap: 11,
          padding: '6px 10px',
          borderRadius: 6,
          cursor: canExpand ? 'pointer' : 'default',
        }}
      >
        <span style={{ color: 'var(--ink-faint)', fontSize: 'var(--fs-micro)', flexShrink: 0 }}>{fmtTime(job.createdAt)}</span>
        <span style={{ color: 'var(--ink-6)', flexShrink: 0 }}>#{pad2(index)}</span>
        <FormatTile spec={spec} size={18} />
        <span
          title={job.fileName}
          style={{
            color: isQueued ? 'var(--ink-4)' : 'var(--ink)',
            fontWeight: 600,
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            maxWidth: 320,
            flexShrink: 0,
          }}
        >
          {job.fileName}
        </span>
        {isError && (
          <span
            style={{
              flexShrink: 0,
              fontSize: 'var(--fs-micro)',
              fontWeight: 600,
              letterSpacing: '0.08em',
              textTransform: 'uppercase',
              padding: '2px 7px',
              borderRadius: 4,
              background: 'var(--tier-low-bg)',
              border: '1px solid var(--tier-low-bd)',
              color: 'var(--tier-low-fg)',
            }}
          >
            Error
          </span>
        )}
        <span
          style={{
            color: isError ? 'var(--danger)' : 'var(--ink-5)',
            fontSize: 'var(--fs-micro)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            minWidth: 0,
          }}
        >
          {meta}
        </span>
        <span style={{ flex: 1 }} />
        {canExpand && (
          <span style={{ display: 'inline-flex', alignItems: 'center', gap: 5, color: 'var(--ink-faint)', fontSize: 'var(--fs-micro)', flexShrink: 0 }}>
            {expanded ? 'collapse' : 'expand'}
            <MIcon name={expanded ? 'expand_less' : 'expand_more'} size={12} />
          </span>
        )}
      </div>
      {expanded && canExpand && (
        <div
          style={{
            margin: '2px 10px 6px',
            padding: '12px 16px',
            border: '1px solid var(--rule-soft)',
            borderRadius: 6,
            background: 'var(--card)',
            boxShadow: 'var(--shadow-elevation-1)',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            color: 'var(--ink-4)',
            display: 'flex',
            flexDirection: 'column',
            gap: 6,
          }}
        >
          {job.modPath && (
            <div>
              target - <span style={{ color: 'var(--ink-2)' }}>{job.modPath}</span>
            </div>
          )}
          {isError && job.error && <div style={{ color: 'var(--danger)' }}>error - {job.error}</div>}
          <div>
            <Link
              to="/logs"
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                gap: 6,
                color: 'var(--ink-2)',
                textDecoration: 'none',
                border: '1px solid var(--rule)',
                borderRadius: 6,
                padding: '4px 10px',
              }}
            >
              <MIcon name="receipt_long" size={12} />
              View log
            </Link>
          </div>
        </div>
      )}
    </div>
  )
}

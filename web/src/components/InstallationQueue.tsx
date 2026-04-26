import type { InstallationJob } from '../types'
import { Link } from 'react-router-dom'

interface InstallationQueueProps {
  jobs: InstallationJob[]
}

const statusLabel: Record<InstallationJob['status'], string> = {
  pending:    'queued',
  uploading:  'uploading',
  processing: 'installing',
  completed:  'complete',
  error:      'error',
}

function statusColor(status: InstallationJob['status']): string {
  switch (status) {
    case 'completed':  return 'var(--moss)'
    case 'pending':    return 'var(--ink-5)'
    case 'uploading':  return 'var(--accent)'
    case 'processing': return 'var(--ochre)'
    case 'error':      return 'var(--accent)'
    default:           return 'var(--ink-5)'
  }
}

function formatSize(bytes?: number): string {
  if (!bytes || bytes <= 0) return ''
  if (bytes < 1024) return `${bytes} b`
  if (bytes < 1048576) return `${(bytes / 1024).toFixed(1)} kb`
  return `${(bytes / 1048576).toFixed(1)} mb`
}

function fileNameSize(job: InstallationJob): string {
  const size = formatSize((job as InstallationJob & { size?: number }).size)
  return size ? `${size} - archive` : 'archive'
}

export default function InstallationQueue({ jobs }: InstallationQueueProps) {
  if (jobs.length === 0) {
    return (
      <div style={{ padding: '32px 28px' }}>
        <div className="empty-state-card" style={{ borderStyle: 'dashed' }}>
          <div
            style={{
              width: 36,
              height: 36,
              borderRadius: '50%',
              border: '1px solid var(--rule-strong)',
              background: 'var(--paper-3)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: 'var(--accent)',
              flexShrink: 0,
            }}
          >
            <i className="fa-duotone fa-solid fa-cloud-moon" style={{ fontSize: 14 }} />
          </div>
          <div style={{ flex: 1 }}>
            <p
              className="display-serif-italic"
              style={{ fontSize: 18, color: 'var(--ink)', lineHeight: 1.2 }}
            >
              Nothing in flight<span className="display-period">.</span>
            </p>
            <p className="timestamp-print" style={{ marginTop: 4 }}>
              // drop archive files above to stage an install
              <span style={{ margin: '0 6px', color: 'var(--ink-5)' }}>-</span>
              <Link to="/logs" style={{ color: 'var(--ink-blue)', textDecoration: 'none' }}>
                tail log -&gt;
              </Link>
            </p>
          </div>
        </div>
      </div>
    )
  }

  return (
    <div>
      {jobs.map((job, i) => {
        const color = statusColor(job.status)
        const isActive = job.status === 'uploading' || job.status === 'processing'
        const progress = job.uploadProgress ?? (job.status === 'completed' ? 100 : 0)
        const stepText = job.processingStatus || statusLabel[job.status]

        return (
          <div
            key={job.id}
            style={{
              display: 'grid',
              gridTemplateColumns: '24px 1fr 130px 180px 70px 20px',
              alignItems: 'center',
              gap: 18,
              padding: '16px 28px',
              borderBottom: i === jobs.length - 1 ? 'none' : '1px solid var(--rule-soft)',
              transition: 'background-color 150ms ease',
            }}
            onMouseEnter={e => { e.currentTarget.style.background = 'var(--card-2)' }}
            onMouseLeave={e => { e.currentTarget.style.background = 'transparent' }}
          >
            {/* Status dot */}
            <div className="flex items-center">
              <span
                className={isActive ? 'dot-status-pulse' : ''}
                style={{
                  display: 'inline-block',
                  width: 8,
                  height: 8,
                  borderRadius: '50%',
                  background: color,
                  boxShadow: isActive
                    ? `0 0 0 4px ${color === 'var(--accent)' ? 'rgba(138,42,31,0.10)' : 'rgba(166,122,42,0.10)'}`
                    : 'none',
                }}
              />
            </div>

            {/* Name + meta */}
            <div style={{ minWidth: 0 }}>
              <div
                style={{
                  fontSize: 14,
                  color: 'var(--ink)',
                  marginBottom: 2,
                  letterSpacing: '-0.005em',
                  overflow: 'hidden',
                  textOverflow: 'ellipsis',
                  whiteSpace: 'nowrap',
                }}
              >
                {job.fileName}
              </div>
              <div
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 10.5,
                  color: 'var(--ink-4)',
                  letterSpacing: '0.04em',
                }}
              >
                {fileNameSize(job)}
              </div>
            </div>

            {/* Step label */}
            <div
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 10.5,
                letterSpacing: '0.1em',
                textTransform: 'uppercase',
                color: job.status === 'completed' ? 'var(--ink-3)' : 'var(--ink-2)',
                fontStyle: job.status === 'pending' ? 'italic' : 'normal',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
              }}
            >
              {stepText}
            </div>

            {/* Progress bar */}
            <div>
              {job.status === 'error' ? (
                <div
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 10.5,
                    color: 'var(--accent)',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                  }}
                  title={job.error}
                >
                  {job.error}
                </div>
              ) : (
                <div className="atelier-progress">
                  <div
                    className={`atelier-progress-fill ${isActive ? 'atelier-progress-sweep' : ''}`}
                    style={{
                      width: `${progress}%`,
                      background: color,
                    }}
                  />
                </div>
              )}
            </div>

            {/* Percentage / check */}
            <div
              style={{
                fontFamily: 'var(--font-display)',
                fontSize: 14,
                color: job.status === 'completed' ? 'var(--ink-3)' : 'var(--ink)',
                textAlign: 'right',
                letterSpacing: '-0.01em',
                fontVariantNumeric: 'tabular-nums',
              }}
            >
              {job.status === 'completed' ? 'OK' : job.status === 'error' ? '!' : `${Math.round(progress)}%`}
            </div>

            {/* Chevron */}
            <div style={{ color: 'var(--ink-4)' }}>
              {job.modPath ? (
                <span title={job.modPath}>
                  <i className="fa-duotone fa-solid fa-chevron-right" style={{ fontSize: 13 }} />
                </span>
              ) : (
                <i className="fa-duotone fa-solid fa-chevron-right" style={{ fontSize: 13, opacity: 0.5 }} />
              )}
            </div>
          </div>
        )
      })}
    </div>
  )
}

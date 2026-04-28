import type { InstallationJob } from '../types'
import Section from './Section'
import InstallationQueue from './InstallationQueue'
import { useCollapsedQueue } from '../hooks/useCollapsedQueue'
import { useViewportShort } from '../hooks/useViewportShort'

interface InstallationQueueShellProps {
  jobs: InstallationJob[]
  systemUnavailable: boolean
}

const MAX_VISIBLE_DOTS = 8

function statusColor(status: InstallationJob['status']): string {
  switch (status) {
    case 'completed':  return 'var(--moss)'
    case 'pending':    return 'var(--ink-5)'
    case 'uploading':  return 'var(--accent)'
    case 'processing': return 'var(--ochre)'
    case 'error':      return 'var(--danger)'
    default:           return 'var(--ink-5)'
  }
}

export default function InstallationQueueShell({
  jobs,
  systemUnavailable,
}: InstallationQueueShellProps) {
  const tooShort = useViewportShort(760)
  const [collapsed, setCollapsed] = useCollapsedQueue(tooShort)

  const activeCount    = jobs.filter(j => j.status === 'uploading' || j.status === 'processing').length
  const queuedCount    = jobs.filter(j => j.status === 'pending').length
  const completedCount = jobs.filter(j => j.status === 'completed').length
  const hasActive      = activeCount > 0

  const visibleDots    = jobs.slice(0, MAX_VISIBLE_DOTS)
  const hiddenDotCount = Math.max(0, jobs.length - MAX_VISIBLE_DOTS)
  const meanProgress = jobs.length === 0
    ? 0
    : Math.round(
        jobs.reduce(
          (sum, j) => sum + (j.uploadProgress ?? (j.status === 'completed' ? 100 : 0)),
          0,
        ) / jobs.length,
      )
  const primaryJob =
    jobs.find(j => j.status === 'uploading' || j.status === 'processing') ??
    jobs.find(j => j.status === 'pending') ??
    jobs[0]

  const meta = collapsed ? (
    <CollapsedLedger
      jobs={jobs}
      visibleDots={visibleDots}
      hiddenDotCount={hiddenDotCount}
      hasActive={hasActive}
      meanProgress={meanProgress}
      primaryJobName={primaryJob?.fileName}
      onExpand={() => setCollapsed(false)}
    />
  ) : (
    <ExpandedMeta
      activeCount={activeCount}
      queuedCount={queuedCount}
      completedCount={completedCount}
      onCollapse={() => setCollapsed(true)}
    />
  )

  return (
    <Section
      n="02"
      label="Activity"
      title="Installation queue"
      corner="02"
      bodyPadding="none"
      className="reveal reveal-delay-5"
      meta={meta}
    >
      <div className="queue-collapse-wrap" data-collapsed={collapsed ? 'true' : 'false'}>
        <div className="queue-collapse-inner">
          <div className={systemUnavailable ? undefined : 'queue-scroll'}>
            {systemUnavailable ? (
              <div>
                {[0, 1].map(i => (
                  <div
                    key={i}
                    className="grid"
                    style={{
                      gridTemplateColumns: '38px 1fr 130px 180px 70px 20px',
                      gap: 18,
                      padding: '12px 24px',
                      borderBottom: i === 1 ? 'none' : '1px solid var(--rule-soft)',
                      alignItems: 'center',
                    }}
                  >
                    <div
                      className="skeleton-line"
                      style={{ width: 38, height: 38, borderRadius: '50%' }}
                    />
                    <div>
                      <div
                        className="skeleton-line"
                        style={{ height: 14, width: ['72%', '58%'][i], marginBottom: 6 }}
                      />
                      <div className="skeleton-line" style={{ height: 14, width: 110 }} />
                    </div>
                    <div className="skeleton-line" style={{ height: 14, width: 100 }} />
                    <div className="skeleton-line" style={{ height: 14, width: '100%' }} />
                    <div className="skeleton-line" style={{ height: 14, width: 40 }} />
                    <div />
                  </div>
                ))}
              </div>
            ) : (
              <InstallationQueue jobs={jobs} />
            )}
          </div>
        </div>
      </div>
    </Section>
  )
}

// ---------------------------------------------------------------------------
// Collapsed ledger - editorial single line shown in Section 02's meta slot
// when the queue body is collapsed. Filename in EB Garamond italic, percent
// paired with it, oxblood rule before "expand", thin progress sweep below.
// ---------------------------------------------------------------------------

interface CollapsedLedgerProps {
  jobs: InstallationJob[]
  visibleDots: InstallationJob[]
  hiddenDotCount: number
  hasActive: boolean
  meanProgress: number
  primaryJobName: string | undefined
  onExpand: () => void
}

function CollapsedLedger({
  jobs,
  visibleDots,
  hiddenDotCount,
  hasActive,
  meanProgress,
  primaryJobName,
  onExpand,
}: CollapsedLedgerProps) {
  const empty = jobs.length === 0
  return (
    <button
      type="button"
      onClick={onExpand}
      className="queue-ledger"
      data-active={hasActive ? 'true' : 'false'}
    >
      {/* Bottom progress sweep - only when at least one job is in flight */}
      {jobs.length > 0 && (
        <span
          className="queue-ledger-progress"
          style={{ width: `calc(${meanProgress}% - 20px)` }}
          aria-hidden="true"
        />
      )}

      {/* Left: italic accent marker + filename / empty phrase */}
      <span className="queue-ledger-left">
        <span
          className="display-serif-italic"
          style={{ color: 'var(--accent)', fontSize: 14, lineHeight: 1, flexShrink: 0 }}
          aria-hidden="true"
        >
          i.
        </span>
        <span
          className="display-serif-italic queue-ledger-filename"
          style={{ fontSize: 17, color: empty ? 'var(--ink-3)' : 'var(--ink)' }}
        >
          {empty ? 'Nothing in flight.' : primaryJobName ?? ''}
        </span>
      </span>

      {/* Center: status dots, one per job up to MAX_VISIBLE_DOTS */}
      {jobs.length > 0 && (
        <span className="queue-ledger-dots">
          {visibleDots.map(job => {
            const isActive = job.status === 'uploading' || job.status === 'processing'
            const color = statusColor(job.status)
            return (
              <span
                key={job.id}
                className={isActive ? 'dot-status-pulse' : ''}
                style={{
                  display: 'inline-block',
                  width: 6,
                  height: 6,
                  borderRadius: '50%',
                  background: color,
                  boxShadow: isActive
                    ? `0 0 0 3px ${
                        color === 'var(--accent)'
                          ? 'var(--glow-info)'
                          : color === 'var(--danger)'
                            ? 'var(--glow-error)'
                            : 'var(--glow-warning)'
                      }`
                    : 'none',
                }}
              />
            )
          })}
          {hiddenDotCount > 0 && (
            <span
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 12,
                color: 'var(--ink-4)',
                letterSpacing: '0.04em',
                marginLeft: 2,
              }}
            >
              +{hiddenDotCount}
            </span>
          )}
        </span>
      )}

      {/* Right: percent - rule - expand */}
      <span className="queue-ledger-right">
        {jobs.length > 0 && (
          <span
            className="display-serif-italic tabular-nums"
            style={{ fontSize: 16, color: 'var(--ink-2)' }}
          >
            {meanProgress}%
          </span>
        )}
        {jobs.length > 0 && <span className="queue-ledger-rule" aria-hidden="true" />}
        <span
          className="display-serif-italic serif-link-arrow"
          style={{
            fontSize: 16,
            color: empty ? 'var(--ink-4)' : undefined,
            letterSpacing: '-0.01em',
          }}
        >
          expand <span className="arrow" aria-hidden="true">&rarr;</span>
        </span>
      </span>
    </button>
  )
}

// ---------------------------------------------------------------------------
// Expanded meta - the three count badges plus an italic collapse link.
// ---------------------------------------------------------------------------

interface ExpandedMetaProps {
  activeCount: number
  queuedCount: number
  completedCount: number
  onCollapse: () => void
}

function ExpandedMeta({ activeCount, queuedCount, completedCount, onCollapse }: ExpandedMetaProps) {
  return (
    <div className="flex items-center" style={{ gap: 16, flexWrap: 'wrap', flex: 1 }}>
      <Badge color="var(--moss)"  label={`${activeCount} active`} />
      <Badge color="var(--ink-4)" label={`${queuedCount} queued`} />
      <Badge color="var(--ink-5)" label={`${completedCount} complete`} dim />
      <button
        type="button"
        className="display-serif-italic serif-link-arrow serif-link-arrow-back"
        onClick={onCollapse}
        style={{
          fontSize: 16,
          background: 'none',
          border: 'none',
          cursor: 'pointer',
          padding: 0,
          marginLeft: 'auto',
          letterSpacing: '-0.01em',
        }}
      >
        <span className="arrow" aria-hidden="true">&larr;</span> collapse
      </button>
    </div>
  )
}

function Badge({ color, label, dim }: { color: string; label: string; dim?: boolean }) {
  return (
    <span className="flex items-center" style={{ gap: 6 }}>
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: color,
          opacity: dim ? 0.5 : 1,
        }}
      />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 13,
          letterSpacing: '0.1em',
          textTransform: 'uppercase',
          color: dim ? 'var(--ink-4)' : 'var(--ink-3)',
        }}
      >
        {label}
      </span>
    </span>
  )
}

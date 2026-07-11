import MIcon from '../MIcon'
import { computeInstallProgress } from '../../useInstallConsole'
import type { InstallationJob } from '../../types'

interface ProgressRibbonProps {
  job: InstallationJob
  // Raw salma log tail for the active job; parsed for the processing percent.
  rawLines: string[]
  index: number
  total: number
  queuedCount: number
}

// The 36px dock pinned at the bottom of the Install page region (above the
// chrome status bar) so live progress survives feed scrolling.
export default function ProgressRibbon({ job, rawLines, index, total, queuedCount }: ProgressRibbonProps) {
  const { pct, label, indeterminate } = computeInstallProgress(job, rawLines)
  const width = indeterminate ? 100 : pct ?? 0

  return (
    <div
      style={{
        flexShrink: 0,
        height: 36,
        display: 'flex',
        alignItems: 'center',
        gap: 12,
        padding: '0 18px',
        borderTop: '1px solid var(--rule)',
        background: 'var(--card)',
        fontFamily: 'var(--font-mono)',
      }}
    >
      <MIcon name="chevron_right" size={12} weight={600} style={{ color: 'var(--ink)' }} />
      <span
        title={job.fileName}
        style={{
          fontSize: 'var(--fs-micro)',
          fontWeight: 600,
          color: 'var(--ink)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          maxWidth: 240,
        }}
      >
        {job.fileName}
      </span>
      <span style={{ fontSize: 'var(--fs-micro)', color: 'var(--ink-5)', whiteSpace: 'nowrap' }}>{label}</span>
      <span
        style={{
          flex: 1,
          minWidth: 60,
          height: 4,
          borderRadius: 2,
          background: 'var(--meter-empty)',
          overflow: 'hidden',
        }}
      >
        <span className="fm-stripe" style={{ display: 'block', height: '100%', width: `${width}%`, transition: 'width 220ms ease' }} />
      </span>
      <span style={{ fontSize: 'var(--fs-label)', fontWeight: 600, color: 'var(--ink)' }}>{pct != null ? `${pct}%` : '--'}</span>
      <span aria-hidden="true" style={{ width: 1, height: 14, background: 'var(--rule-soft)' }} />
      <span style={{ fontSize: 'var(--fs-micro)', color: 'var(--ink-5)', whiteSpace: 'nowrap' }}>
        job {index} / {total}
        {queuedCount > 0 ? ` - ${queuedCount} queued` : ''}
      </span>
    </div>
  )
}

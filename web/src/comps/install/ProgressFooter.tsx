import { computeInstallProgress } from '../../useInstallConsole'
import type { InstallationJob } from '../../types'

interface ProgressFooterProps {
  job: InstallationJob
  // Raw salma log tail; parsed for processing progress when no upload % applies.
  rawLines: string[]
}

// Thin striped track + stage label + percent. The fill animates via .fm-stripe
// while work is in flight; it goes solid for the done/error terminal states.
export default function ProgressFooter({ job, rawLines }: ProgressFooterProps) {
  const { pct, label, tone, indeterminate } = computeInstallProgress(job, rawLines)

  const width = tone === 'done' || tone === 'error' || indeterminate ? 100 : (pct ?? 0)
  const animated = tone === 'normal'
  const fillBackground =
    tone === 'done' ? 'var(--ink)' : tone === 'error' ? 'var(--danger)' : undefined

  const pctText = pct != null ? `${pct}%` : tone === 'error' ? '!' : tone === 'done' ? '100%' : '--'

  return (
    <div
      style={{
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '12px 16px',
        borderTop: '1px solid var(--rule-soft)',
      }}
    >
      <div
        style={{
          flex: 1,
          height: 6,
          borderRadius: 3,
          background: 'var(--meter-empty)',
          overflow: 'hidden',
        }}
      >
        <div
          className={animated ? 'fm-stripe' : undefined}
          style={{
            height: '100%',
            width: `${width}%`,
            background: fillBackground,
            transition: 'width 220ms ease',
          }}
        />
      </div>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          color: tone === 'error' ? 'var(--danger)' : 'var(--ink-4)',
          whiteSpace: 'nowrap',
          maxWidth: 260,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-title)',
          fontWeight: 600,
          color: tone === 'error' ? 'var(--danger)' : 'var(--ink)',
          flexShrink: 0,
        }}
      >
        {pctText}
      </span>
    </div>
  )
}

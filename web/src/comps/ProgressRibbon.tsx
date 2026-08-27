import { computeInstallProgress, deriveActiveOp } from '../useInstallConsole'
import { StageSegments } from './StageMeter'
import { STAGES, segmentFills, stageIndexForOp } from './stages'
import { pad2 } from './jobFormat'
import type { InstallationJob } from '../types'

interface ProgressRibbonProps {
  job: InstallationJob
  // Raw salma log tail for the active job; parsed for the percent and the stage.
  rawLines: string[]
  index: number
  total: number
  queuedCount: number
}

/**
 * The dock pinned below the Install feed (above the chrome status bar) so live
 * progress survives feed scrolling.
 *
 * It runs the same six-segment meter as the active card at a smaller size, so
 * the two read as one instrument rather than two different progress bars. It
 * sits on the page plane: no rule above it, no surface of its own, nothing
 * raised.
 */
export default function ProgressRibbon({ job, rawLines, index, total, queuedCount }: ProgressRibbonProps) {
  const { pct, label, tone, indeterminate } = computeInstallProgress(job, rawLines)
  const stage = stageIndexForOp(deriveActiveOp(rawLines))
  const done = tone === 'done'
  const fills = segmentFills(stage, pct, tone, indeterminate)

  return (
    <div
      style={{
        flexShrink: 0,
        height: 44,
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '0 18px',
        position: 'relative',
        zIndex: 20,
        fontFamily: 'var(--font-mono)',
      }}
    >
      <span
        aria-hidden="true"
        className={done || tone === 'error' ? undefined : 'blink'}
        style={{
          width: 6,
          height: 6,
          flexShrink: 0,
          background: tone === 'error' ? 'var(--danger)' : done ? 'var(--moss)' : 'var(--signal)',
        }}
      />
      <span
        title={job.fileName}
        style={{
          fontSize: 'var(--fs-label)',
          fontWeight: 600,
          color: 'var(--ink-2)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          maxWidth: 240,
          flexShrink: 0,
        }}
      >
        {job.fileName}
      </span>
      <span
        style={{
          fontSize: 'var(--fs-micro)',
          fontWeight: 700,
          letterSpacing: 'var(--tr-stage)',
          color: tone === 'error' ? 'var(--danger)' : 'var(--signal)',
          flexShrink: 0,
        }}
      >
        {done ? 'DONE' : STAGES[stage]}
      </span>
      <span
        style={{
          fontSize: 'var(--fs-meta)',
          color: 'var(--ink-5)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          minWidth: 0,
          flex: 1,
        }}
      >
        {label}
      </span>

      <span style={{ display: 'block', flex: 1, minWidth: 90, maxWidth: 260 }}>
        <StageSegments
          stage={stage}
          fills={fills}
          errored={tone === 'error'}
          indeterminate={indeterminate}
          done={done}
          height={5}
          gap={2}
        />
      </span>

      <span
        className="tabular-nums"
        style={{ fontSize: 'var(--fs-body)', fontWeight: 700, color: 'var(--ink)', flexShrink: 0, width: 38, textAlign: 'right' }}
      >
        {pct != null ? `${pct}%` : '--'}
      </span>
      <span aria-hidden="true" style={{ width: 1, height: 16, flexShrink: 0, background: 'var(--rule-strong)' }} />
      <span
        className="tabular-nums"
        style={{ fontSize: 'var(--fs-meta)', color: 'var(--ink-faint)', whiteSpace: 'nowrap', flexShrink: 0 }}
      >
        job {pad2(index)} / {pad2(total)}
        {queuedCount > 0 ? ` | ${queuedCount} queued` : ''}
      </span>
    </div>
  )
}

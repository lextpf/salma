import { computeInstallProgress } from '../useInstallConsole'
import { fmtDur, fmtRate, parseProgressBars } from '../progressBarParsing'
import { STAGES, segmentFills, stageIndexForOp } from './stages'
import type { InstallationJob } from '../types'

interface SegmentsProps {
  stage: number
  fills: number[]
  errored: boolean
  indeterminate: boolean
  done: boolean
  height?: number
  gap?: number
}

export function StageSegments({ stage, fills, errored, indeterminate, done, height = 7, gap = 3 }: SegmentsProps) {
  return (
    <div style={{ display: 'flex', gap, minWidth: 0 }}>
      {fills.map((fill, i) => {
        const live = i === stage && !done
        return (
          <span
            key={STAGES[i]}
            style={{
              position: 'relative',
              display: 'block',
              flex: 1,
              minWidth: 0,
              height,
              background: 'var(--track)',
              overflow: 'hidden',
            }}
          >
            <span
              className={live && indeterminate && !errored ? 'fm-stripe' : undefined}
              style={{
                position: 'absolute',
                inset: '0 auto 0 0',
                width: `${fill}%`,
                background: errored && live
                  ? 'var(--danger)'
                  : done
                    ? 'var(--moss)'
                    : live && indeterminate
                      ? undefined
                      : live
                        ? 'var(--stage-live)'
                        : 'var(--stage-done)',
                transition: 'width 220ms var(--ease)',
              }}
            />
          </span>
        )
      })}
    </div>
  )
}

interface StageMeterProps {
  job: InstallationJob
  rawLines: string[]
  activeOp: string | null
  errored?: boolean
}

export default function StageMeter({ job, rawLines, activeOp, errored = false }: StageMeterProps) {
  const { pct, label, tone, indeterminate } = computeInstallProgress(job, rawLines)
  const stage = stageIndexForOp(activeOp)
  const done = tone === 'done'
  const fills = segmentFills(stage, pct, tone, indeterminate)

  const numeral = pct != null ? String(pct) : tone === 'error' ? '!' : done ? '100' : '--'

  // omit rate and ETA until the same tqdm record provides enough data.
  const bar = parseProgressBars(rawLines, 'salma')[0]
  const counted = bar?.current != null && bar.total != null
  const timed = counted && bar!.elapsedS != null && bar!.elapsedS > 0 && bar!.current! > 0
  const rate = timed ? bar!.current! / bar!.elapsedS! : 0
  const remain = timed && bar!.current! < bar!.total! ? (bar!.total! - bar!.current!) / rate : null

  // discard stale rate and ETA after failure.
  const failed = tone === 'error'
  const metrics = failed
    ? (job.error ?? label)
    : [
      label,
      counted ? `${bar!.current!.toLocaleString()} / ${bar!.total!.toLocaleString()}` : null,
      timed ? (rate >= 1 ? `${fmtRate(rate)}/s` : `${fmtRate(1 / rate)}s each`) : null,
      remain != null ? `eta ${fmtDur(remain)}` : null,
    ].filter(Boolean).join('  |  ')

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'flex-end',
        gap: 16,
        padding: '13px 16px 14px',
      }}
    >
      <span
        className="tabular-nums"
        style={{
          flexShrink: 0,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-hero)',
          fontWeight: 600,
          lineHeight: 0.82,
          letterSpacing: 'var(--tr-hero)',
          color: errored ? 'var(--danger)' : 'var(--ink)',
        }}
      >
        {numeral}
        {/* omit the percent unit for failures. */}
        {!failed && (
          <span style={{ fontSize: 'var(--fs-label)', fontWeight: 500, letterSpacing: 0, color: 'var(--ink-5)' }}>
            %
          </span>
        )}
      </span>

      <div style={{ flex: 1, minWidth: 0, display: 'flex', flexDirection: 'column', gap: 6 }}>
        <div style={{ display: 'flex', gap: 3, minWidth: 0 }}>
          {STAGES.map((s, i) => {
            const past = done || i < stage
            const live = !done && i === stage
            return (
              <span
                key={s}
                style={{
                  flex: 1,
                  minWidth: 0,
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  fontWeight: live ? 700 : past ? 500 : 400,
                  letterSpacing: 'var(--tr-stage)',
                  color: live
                    ? (errored ? 'var(--danger)' : 'var(--signal)')
                    : past ? 'var(--ink-4)' : 'var(--ink-faint)',
                  whiteSpace: 'nowrap',
                  overflow: 'hidden',
                  textOverflow: 'clip',
                }}
              >
                {s}
              </span>
            )
          })}
        </div>

        <StageSegments
          stage={stage}
          fills={fills}
          errored={errored}
          indeterminate={indeterminate}
          done={done}
        />

        <div
          style={{
            display: 'flex',
            alignItems: 'baseline',
            justifyContent: 'space-between',
            gap: 12,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-meta)',
          }}
        >
          <span
            className="tabular-nums"
            style={{
              minWidth: 0,
              color: errored ? 'var(--danger)' : 'var(--ink-5)',
              whiteSpace: 'nowrap',
              overflow: 'hidden',
              textOverflow: 'ellipsis',
            }}
          >
            {metrics}
          </span>
          <span
            className="tabular-nums"
            style={{ flexShrink: 0, color: 'var(--ink-faint)', whiteSpace: 'nowrap' }}
          >
            stage {String(Math.min(stage + 1, STAGES.length)).padStart(2, '0')} / {String(STAGES.length).padStart(2, '0')}
          </span>
        </div>
      </div>
    </div>
  )
}

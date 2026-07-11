import type { InstallationJob } from '../../types'
import type { ConsoleLine } from '../../useInstallConsole'
import MIcon from '../MIcon'
import ActiveJobCard from './ActiveJobCard'
import JobLine from './JobLine'
import { FORMATS, formatForFile } from './formats'
import { FormatChip, FormatTile } from './FormatTile'

interface SessionFeedProps {
  jobs: InstallationJob[]
  active: { job: InstallationJob; index: number } | null
  isInstalling: boolean
  // Intake disabled (purged / unavailable / installing). Drives click-to-browse.
  locked: boolean
  purged: boolean
  consoleLines: ConsoleLine[]
  rawLines: string[]
  total: number
  onCancel: () => void
  onBrowse: () => void
  isDragging: boolean
  onDragOver: (e: React.DragEvent) => void
  onDragLeave: (e: React.DragEvent) => void
  onDrop: (e: React.DragEvent) => void
  // Best-effort names of the dragged files (usually empty: browsers hide the
  // drag payload during dragover).
  dragFileNames: string[]
}

function pad2(n: number): string {
  return String(n).padStart(2, '0')
}

function Corner({ pos, color, thick }: { pos: 'tl' | 'tr' | 'bl' | 'br'; color: string; thick: number }) {
  const b = `${thick}px solid ${color}`
  const vert = pos[0] === 't' ? { top: 14 } : { bottom: 14 }
  const horiz = pos[1] === 'l' ? { left: 14 } : { right: 14 }
  const borders: React.CSSProperties =
    pos === 'tl'
      ? { borderTop: b, borderLeft: b }
      : pos === 'tr'
        ? { borderTop: b, borderRight: b }
        : pos === 'bl'
          ? { borderBottom: b, borderLeft: b }
          : { borderBottom: b, borderRight: b }
  return <span aria-hidden="true" style={{ position: 'absolute', width: 11, height: 11, zIndex: 2, ...vert, ...horiz, ...borders }} />
}

export default function SessionFeed(props: SessionFeedProps) {
  const {
    jobs,
    active,
    isInstalling,
    locked,
    purged,
    consoleLines,
    rawLines,
    total,
    onCancel,
    onBrowse,
    isDragging,
    onDragOver,
    onDragLeave,
    onDrop,
    dragFileNames,
  } = props

  const now = new Date()
  const sessionDate = `${now.getFullYear()}-${pad2(now.getMonth() + 1)}-${pad2(now.getDate())}`

  const done = jobs.filter(j => j.status === 'completed').length
  const failed = jobs.filter(j => j.status === 'error').length
  const running = jobs.filter(j => j.status === 'uploading' || j.status === 'processing').length
  const queued = jobs.filter(j => j.status === 'pending').length

  const isEmpty = jobs.length === 0
  const showLegend = !active && !isInstalling
  const tickColor = purged ? 'var(--ink-faint)' : isDragging ? 'var(--ink)' : 'var(--ink)'
  const tickThick = isDragging ? 2.5 : 1.5

  const tally: { text: string; danger?: boolean }[] = [{ text: `${jobs.length} job${jobs.length === 1 ? '' : 's'}` }]
  if (done) tally.push({ text: `${done} done` })
  if (running) tally.push({ text: `${running} running` })
  if (queued) tally.push({ text: `${queued} queued` })
  if (failed) tally.push({ text: `${failed} failed`, danger: true })

  const browse = (e: React.MouseEvent | React.KeyboardEvent) => {
    e.stopPropagation()
    if (!locked) onBrowse()
  }

  const ruleLine = (
    <div style={{ display: 'flex', alignItems: 'center', gap: 10, color: 'var(--ink-5)', fontSize: 'var(--fs-micro)' }}>
      <span style={{ flexShrink: 0 }}>- session {sessionDate}</span>
      <span style={{ flex: 1, height: 1, background: 'var(--rule-faint)' }} />
      <span style={{ color: 'var(--ink-4)', flexShrink: 0 }}>
        {tally.map((t, i) => (
          <span key={i}>
            {i > 0 && <span style={{ color: 'var(--ink-6)' }}> - </span>}
            <span style={{ color: t.danger ? 'var(--danger)' : undefined }}>{t.text}</span>
          </span>
        ))}
      </span>
    </div>
  )

  const legend = (
    <div style={{ marginTop: 10, display: 'flex', alignItems: 'center', gap: 7, flexWrap: 'wrap' }}>
      <span style={{ color: 'var(--ink-4)', fontSize: 'var(--fs-micro)', marginRight: 3 }}>accepts</span>
      {FORMATS.map(spec => (
        <FormatChip key={spec.extension} spec={spec} />
      ))}
      <span style={{ color: 'var(--ink-6)', fontSize: 'var(--fs-micro)' }}>- &lt;= 512 MiB - multi-file queue</span>
    </div>
  )

  const caret = purged ? (
    <div style={{ marginTop: 22, display: 'flex', alignItems: 'center', gap: 11 }}>
      <MIcon name="chevron_right" size={12} weight={600} style={{ color: 'var(--ink-6)' }} />
      <MIcon name="lock" size={12} style={{ color: 'var(--ink-6)' }} />
      <span style={{ color: 'var(--ink-6)' }}>locked - deploy the plugin to continue</span>
    </div>
  ) : isInstalling ? null : (
    <div
      role={locked ? undefined : 'button'}
      tabIndex={locked ? undefined : 0}
      onClick={locked ? undefined : browse}
      onKeyDown={
        locked
          ? undefined
          : e => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              browse(e)
            }
          }
      }
      style={{ marginTop: 24, display: 'flex', alignItems: 'center', gap: 11, cursor: locked ? 'default' : 'pointer' }}
    >
      <MIcon name="chevron_right" size={12} weight={600} style={{ color: 'var(--ink)' }} />
      <span aria-hidden="true" style={{ display: 'inline-block', width: 8, height: 15, background: 'var(--ink)', animation: 'salma-blink 1.1s steps(1) infinite' }} />
      <span style={{ color: 'var(--ink-4)' }}>{isEmpty ? 'awaiting first archive' : 'awaiting archive - drop anywhere, or click to browse'}</span>
    </div>
  )

  const emptyInvitation = (
    <div
      role={locked ? undefined : 'button'}
      tabIndex={locked ? undefined : 0}
      onClick={locked ? undefined : browse}
      onKeyDown={
        locked
          ? undefined
          : e => {
            if (e.key === 'Enter' || e.key === ' ') {
              e.preventDefault()
              browse(e)
            }
          }
      }
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        alignItems: 'center',
        justifyContent: 'center',
        gap: 0,
        cursor: locked ? 'default' : 'pointer',
      }}
    >
      <div
        style={{
          width: 52,
          height: 52,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          borderRadius: 12,
          color: 'var(--ink)',
          background: 'color-mix(in srgb, var(--ink) 6%, transparent)',
        }}
      >
        <MIcon name="upload" size={26} />
      </div>
      <div style={{ marginTop: 16, fontFamily: 'var(--font-body)', fontSize: 'var(--fs-head)', fontWeight: 600, letterSpacing: '-0.02em', color: 'var(--ink)' }}>
        No installs this session
      </div>
      <div style={{ marginTop: 6, fontSize: 'var(--fs-micro)', color: 'var(--ink-4)' }}>drop an archive anywhere on this surface - or click to browse</div>
      <div style={{ marginTop: 18, display: 'flex', alignItems: 'center', gap: 7, flexWrap: 'wrap', justifyContent: 'center' }}>
        {FORMATS.map(spec => (
          <FormatChip key={spec.extension} spec={spec} />
        ))}
      </div>
      <div style={{ marginTop: 10, fontSize: 'var(--fs-micro)', color: 'var(--ink-6)' }}>&lt;= 512 MiB each - multi-file queue supported</div>
    </div>
  )

  const history = (
    <>
      {showLegend && legend}
      {showLegend && (
        <div style={{ marginTop: 22, marginBottom: 4, fontSize: 'var(--fs-micro)', letterSpacing: '0.12em', textTransform: 'uppercase', color: 'var(--ink-5)' }}>
          Earlier this session
        </div>
      )}
      <div style={{ marginTop: showLegend ? 0 : 12 }}>
        {jobs.map((job, i) =>
          active && job.id === active.job.id ? (
            <ActiveJobCard key={job.id} job={job} index={i + 1} total={total} lines={consoleLines} rawLines={rawLines} onCancel={onCancel} />
          ) : (
            <JobLine key={job.id} job={job} index={i + 1} />
          ),
        )}
      </div>
    </>
  )

  return (
    <div
      role="region"
      aria-label="Install session feed"
      onDragOver={onDragOver}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
      style={{ flex: 1, minHeight: 0, position: 'relative', overflow: 'hidden', background: isDragging ? 'var(--card)' : 'var(--sheet)' }}
    >
      <Corner pos="tl" color={tickColor} thick={tickThick} />
      <Corner pos="tr" color={tickColor} thick={tickThick} />
      <Corner pos="bl" color={tickColor} thick={tickThick} />
      <Corner pos="br" color={tickColor} thick={tickThick} />

      {!isEmpty && !isDragging && (
        <div
          aria-hidden="true"
          style={{
            position: 'absolute',
            right: 10,
            bottom: -38,
            fontFamily: 'var(--font-body)',
            fontWeight: 600,
            fontSize: '150px',
            lineHeight: 1,
            letterSpacing: '-0.08em',
            color: 'color-mix(in srgb, var(--ink) 3.5%, transparent)',
            userSelect: 'none',
            pointerEvents: 'none',
            zIndex: 0,
          }}
        >
          01
        </div>
      )}

      <div
        style={{
          position: 'absolute',
          inset: 0,
          overflowY: 'auto',
          overflowX: 'hidden',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-label)',
          lineHeight: 1.9,
          opacity: isDragging ? 0.28 : 1,
          filter: isDragging ? 'grayscale(1)' : undefined,
          transition: 'opacity 120ms ease',
        }}
      >
        <div style={{ minHeight: '100%', display: 'flex', flexDirection: 'column', padding: '20px 28px', position: 'relative', zIndex: 1 }}>
          {ruleLine}
          {isEmpty ? emptyInvitation : history}
          {caret}
        </div>
      </div>

      {isDragging && (
        <div style={{ position: 'absolute', inset: 0, display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 3 }}>
          <div
            style={{
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              gap: 12,
              padding: '28px 44px',
              border: '1.5px dashed var(--ink)',
              borderRadius: 14,
              background: 'var(--sheet)',
              boxShadow: 'var(--shadow-elevation-2)',
            }}
          >
            <span style={{ width: 44, height: 44, display: 'inline-flex', alignItems: 'center', justifyContent: 'center', borderRadius: 10, color: 'var(--sheet)', background: 'var(--ink)' }}>
              <MIcon name="arrow_downward" size={22} />
            </span>
            <div style={{ fontFamily: 'var(--font-body)', fontSize: 'var(--fs-head)', fontWeight: 600, letterSpacing: '-0.02em', color: 'var(--ink)' }}>
              {dragFileNames.length > 0 ? `Release to queue ${dragFileNames.length} archive${dragFileNames.length === 1 ? '' : 's'}` : 'Release to queue'}
            </div>
            {dragFileNames.length > 0 && (
              <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap', justifyContent: 'center', fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-4)' }}>
                {dragFileNames.slice(0, 4).map((name, i) => (
                  <span key={i} style={{ display: 'inline-flex', alignItems: 'center', gap: 5 }}>
                    <FormatTile spec={formatForFile(name)} size={16} />
                    {name}
                  </span>
                ))}
              </div>
            )}
            <div style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-6)' }}>they run in order - the current job is never interrupted</div>
          </div>
        </div>
      )}
    </div>
  )
}

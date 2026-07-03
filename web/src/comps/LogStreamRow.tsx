import { memo } from 'react'
import { highlightLog, type HighlightSegment } from '../logHighlight'
import { ROW_LOG } from '../useVirtualScroll'
import type { LogLevel, LogRecord } from '../logParse'

interface LevelStyle {
  fg: string
  bg: string
  bd: string
  msg: string
  label: string
}

function levelStyle(level: LogLevel): LevelStyle {
  switch (level) {
    case 'WARN':
      return { fg: 'var(--log-warn)', bg: 'var(--lvl-warn-bg)', bd: 'var(--lvl-warn-bd)', msg: 'var(--warn-text)', label: 'WARN' }
    case 'ERROR':
      return { fg: 'var(--log-error)', bg: 'var(--lvl-error-bg)', bd: 'var(--lvl-error-bd)', msg: 'var(--danger-2)', label: 'ERROR' }
    case 'DEBUG':
      return { fg: 'var(--log-debug)', bg: 'var(--lvl-debug-bg)', bd: 'var(--lvl-debug-bd)', msg: 'var(--ink-4)', label: 'DEBUG' }
    case 'INFO':
      return { fg: 'var(--log-info)', bg: 'var(--lvl-info-bg)', bd: 'var(--lvl-info-bd)', msg: 'var(--ink-3)', label: 'INFO' }
    default:
      return { fg: 'var(--ink-5)', bg: 'var(--lvl-debug-bg)', bd: 'var(--lvl-debug-bd)', msg: 'var(--ink-4)', label: level || 'LOG' }
  }
}

function renderParts(parts: HighlightSegment[]) {
  return parts.map((p, j) =>
    p.cls ? (
      <span key={j} className={p.cls}>
        {p.text}
      </span>
    ) : (
      <span key={j}>{p.text}</span>
    )
  )
}

export default memo(function LogStreamRow({ record }: { record: LogRecord }) {
  // row height must match `ROW_LOG` in `useVirtualScroll.ts`.
  const lvl = levelStyle(record.level)
  const subsystem = record.subsystem || 'general'
  return (
    <div
      className="fm-row log-stream-row"
      data-level={record.level || 'LOG'}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 9,
        height: ROW_LOG,
        padding: '0 18px 0 16px',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-mono)',
        whiteSpace: 'nowrap',
        minWidth: 0,
      }}
    >
      <span className="log-stream-time" style={{ color: 'var(--ink-5)' }}>
        {record.ts ?? ''}
      </span>
      <span
        className="log-stream-level"
        style={{
          padding: '2.5px 0',
          borderRadius: 'var(--radius-chip)',
          background: lvl.bg,
          color: lvl.fg,
          fontSize: 'var(--fs-micro)',
          fontWeight: 700,
          letterSpacing: 'var(--tr-chip)',
        }}
      >
        {lvl.label}
      </span>
      <span
        className="log-stream-subsystem"
        style={{ overflow: 'hidden', textOverflow: 'ellipsis' }}
      >
        {subsystem}
      </span>
      <span
        style={{
          color: lvl.msg,
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {renderParts(highlightLog(record.message))}
      </span>
    </div>
  )
})

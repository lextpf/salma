import { memo } from 'react'
import { highlightLog, type HighlightSegment } from '../logHighlight'
import { ROW_LOG } from '../useVirtualScroll'
import type { LogLevel, LogRecord } from '../logParse'

interface LevelStyle {
  fg: string
  bg: string
  bd: string
  /** Base colour for the message text on this severity. */
  msg: string
  label: string
}

// Levels are chips rather than coloured words: a fixed-width block in a fixed
// column scans down the page in a way coloured text does not. The message tone
// carries the same severity one step quieter.
//
// The chip is a wash and nothing else. An outline here would draw a box on
// every visible row, thirty down a full screen of log, to mark a boundary the
// wash already marks.
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

/**
 * Fixed-height mono row matching the virtual-scroll line height: timestamp,
 * level chip, subsystem, highlighted message.
 *
 * A 'WARN' or 'ERROR' row also gets a flat 2px left severity edge and a flat
 * wash, applied from CSS through the data-level attribute so hover composes
 * with them. The inset severity bar is the only box-shadow on the row, and it
 * is a drawn edge rather than a lift.
 */
export default memo(function LogStreamRow({ record }: { record: LogRecord }) {
  const lvl = levelStyle(record.level)
  const subsystem = record.subsystem || 'general'
  return (
    <div
      className="fm-row log-stream-row"
      data-level={record.level || 'LOG'}
      style={{
        display: 'flex',
        alignItems: 'center',
        // Tight on purpose. The four leading columns are fixed and read as
        // columns, so the space between them is separation, not rhythm. What it
        // saves goes to the message, the only part that can run out of room.
        gap: 9,
        height: ROW_LOG,
        padding: '0 18px 0 16px',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-mono)',
        whiteSpace: 'nowrap',
        minWidth: 0,
      }}
    >
      {/* A timestamp is data, not a mark: it sits on a text-grade ink level
          rather than the meta level the stylesheet defaults it to. */}
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

import { memo } from 'react'
import { highlightLog, type HighlightSegment } from '../../logHighlight'
import { LINE_HEIGHT } from '../../useVirtualScroll'
import type { LogLevel, LogRecord } from '../../logParse'

// Fixed-height mono row matching the virtual-scroll line height: timestamp /
// level / subsystem tag / highlighted message.
function levelStyle(level: LogLevel): { color: string; fontWeight: number; label: string } {
  switch (level) {
    case 'WARN':
      return { color: 'var(--log-warn)', fontWeight: 700, label: 'WARN' }
    case 'ERROR':
      return { color: 'var(--log-error)', fontWeight: 700, label: 'ERROR' }
    case 'DEBUG':
      return { color: 'var(--log-debug)', fontWeight: 600, label: 'DEBUG' }
    case 'INFO':
      return { color: 'var(--log-info)', fontWeight: 600, label: 'INFO' }
    default:
      return { color: 'var(--ink-4)', fontWeight: 500, label: recordLabel(level) }
  }
}

function recordLabel(level: LogLevel): string {
  return level || 'LOG'
}

const SUBSYSTEM_TONES = ['var(--log-info)', 'var(--log-debug)', 'var(--moss)', 'var(--ochre)']

function subsystemColor(value: string): string {
  let hash = 0
  for (let i = 0; i < value.length; i++) hash = (hash * 31 + value.charCodeAt(i)) >>> 0
  return SUBSYSTEM_TONES[hash % SUBSYSTEM_TONES.length]
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
  const lvl = levelStyle(record.level)
  const subsystem = record.subsystem || 'general'
  const subsystemTone = subsystemColor(subsystem)
  return (
    <div
      className="fm-row log-stream-row"
      data-level={record.level || 'LOG'}
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        height: LINE_HEIGHT,
        padding: '2px 18px',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-body)',
        minWidth: 0,
      }}
    >
      <span className="log-stream-time" style={{ flexShrink: 0 }}>{record.ts ?? ''}</span>
      <span
        className="log-stream-level"
        style={{ color: lvl.color, fontWeight: lvl.fontWeight, flexShrink: 0 }}
      >
        {lvl.label}
      </span>
      <span
        className="log-stream-subsystem"
        style={{
          color: subsystemTone,
          flexShrink: 0,
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {subsystem}
      </span>
      <span
        style={{
          color: 'var(--ink-2)',
          minWidth: 0,
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {renderParts(highlightLog(record.message))}
      </span>
    </div>
  )
})

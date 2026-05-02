import { memo } from 'react'
import { highlightLog, type HighlightSegment } from '../logHighlight'

// Tab-leader pattern: a run of 4+ dots between whitespace boundaries. The
// backend (test_all.py / mo2-salma.py) pads label-to-status with literal dots so
// fixed-width terminal output aligns SKIP/PASS columns. In the HTML viewer
// the container is narrower, so the literal dots overflow and wrap. We pull
// the dots out of the text and render a flex-grow CSS leader instead so the
// line always fits the container.
const DOT_LEADER_RE = /\s\.{4,}\s/

function renderParts(parts: HighlightSegment[]) {
  return parts.map((p, j) => (
    p.cls
      ? <span key={j} className={p.cls}>{p.text}</span>
      : <span key={j}>{p.text}</span>
  ))
}

export const LogLine = memo(function LogLine({ line }: { line: string }) {
  const isError = /\bERROR\b|\bCRITICAL\b|\bFATAL\b|\bFAIL\b/i.test(line)
  const isWarning = /\bWARNING\b|\bWARN\b/i.test(line)
  const isPass = /\bPASS\b|\bINFERRED\b/.test(line)
  const bgClass = isError ? 'bg-error/5'
    : isWarning ? 'bg-warning/5'
    : isPass ? 'bg-success/5'
    : ''

  const leaderMatch = line.match(DOT_LEADER_RE)
  if (leaderMatch && leaderMatch.index !== undefined) {
    const dotStart = leaderMatch.index
    const dotEnd = dotStart + leaderMatch[0].length
    const prefixParts = highlightLog(line.slice(0, dotStart))
    const suffixParts = highlightLog(line.slice(dotEnd))
    return (
      <div
        className={`flex items-center py-0.5 px-2 rounded ${bgClass}`}
        style={{ gap: 8, minWidth: 0 }}
      >
        <span
          style={{
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            minWidth: 0,
          }}
        >
          {renderParts(prefixParts)}
        </span>
        <span
          aria-hidden="true"
          style={{
            flex: 1,
            minWidth: 16,
            borderBottom: '1px dotted var(--ink-4)',
            opacity: 0.55,
            transform: 'translateY(-3px)',
          }}
        />
        <span style={{ whiteSpace: 'nowrap', flexShrink: 0 }}>
          {renderParts(suffixParts)}
        </span>
      </div>
    )
  }

  return (
    <div className={`py-0.5 px-2 rounded ${bgClass}`}>
      {renderParts(highlightLog(line))}
    </div>
  )
})

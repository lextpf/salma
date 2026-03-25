import { memo } from 'react'
import { highlightLog } from '../utils/logHighlight'

export const LogLine = memo(function LogLine({ line }: { line: string }) {
  const isError = /\bERROR\b|\bCRITICAL\b|\bFATAL\b|\bFAIL\b/i.test(line)
  const isWarning = /\bWARNING\b|\bWARN\b/i.test(line)
  const isPass = /\bPASS\b|\bINFERRED\b/.test(line)
  const parts = highlightLog(line)
  return (
    <div
      className={`py-0.5 px-2 rounded ${
        isError ? 'bg-error/5' :
        isWarning ? 'bg-warning/5' :
        isPass ? 'bg-success/5' :
        ''
      }`}
    >
      {parts.map((p, j) => (
        p.cls
          ? <span key={j} className={p.cls}>{p.text}</span>
          : <span key={j}>{p.text}</span>
      ))}
    </div>
  )
})

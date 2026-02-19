import { useState } from 'react'

import type { ConfidenceScore, FomodReason } from '../types'
import ConfidenceBreakdown from './ConfidenceBreakdown'

interface WhyPanelProps {
  confidence?: ConfidenceScore
  reasons?: FomodReason[]
  /** Optional context label rendered as the panel header (e.g. "Why CBBE?"). */
  title?: string
}

const REASON_COLOR: Record<string, string> = {
  FORCED_REQUIRED: 'var(--moss)',
  FORCED_NOT_USABLE: 'var(--danger)',
  FORCED_SELECT_ALL: 'var(--moss)',
  FORCED_AT_LEAST_ONE: 'var(--moss)',
  FORCED_EXACTLY_ONE: 'var(--moss)',
  UNIQUE_FILE_EVIDENCE: 'var(--accent)',
  NO_FILE_EVIDENCE: 'var(--ochre)',
  NO_UNIQUE_EVIDENCE: 'var(--ink-3)',
  CARDINALITY_FORCED: 'var(--accent)',
  CSP_PHASE_GREEDY: 'var(--accent)',
  CSP_PHASE_LOCAL_SEARCH: 'var(--accent)',
  CSP_PHASE_BACKTRACK: 'var(--ochre)',
  CSP_PHASE_REPAIR: 'var(--ochre)',
  CSP_PHASE_FOCUSED: 'var(--ochre)',
  CSP_PHASE_FALLBACK: 'var(--danger)',
  CONDITION_FORCED_TRUE: 'var(--moss)',
  CONDITION_FORCED_FALSE: 'var(--moss)',
  CONDITION_UNKNOWN: 'var(--ochre)',
  STEP_VISIBILITY_FORCED: 'var(--moss)',
  STEP_VISIBILITY_UNKNOWN: 'var(--ochre)',
  STEP_NOT_VISIBLE: 'var(--ink-3)',
  EXTRA_FILE_PRODUCED: 'var(--danger)',
  FOMOD_PLUS_CACHE: 'var(--ink-blue)',
  IMPLICIT_DEFAULT: 'var(--ink-4)',
}

function CodeChip({ code }: { code: string }) {
  const color = REASON_COLOR[code] ?? 'var(--ink-3)'
  return (
    <span
      className="flex items-center"
      style={{
        gap: 6,
        fontFamily: 'var(--font-mono)',
        fontSize: 10,
        letterSpacing: '0.14em',
        textTransform: 'uppercase',
        color: 'var(--ink-2)',
        padding: '2px 8px',
        borderRadius: 3,
        background: 'var(--paper-3)',
        border: '1px solid var(--rule-soft)',
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: color,
        }}
      />
      <span>{code}</span>
    </span>
  )
}

function DetailBlock({ detail }: { detail: Record<string, unknown> }) {
  const text = JSON.stringify(detail, null, 2)
  return (
    <pre
      style={{
        margin: 0,
        marginTop: 6,
        padding: '8px 10px',
        background: 'var(--paper-2)',
        border: '1px solid var(--rule-soft)',
        borderRadius: 3,
        fontFamily: 'var(--font-mono)',
        fontSize: 11,
        color: 'var(--ink-3)',
        whiteSpace: 'pre-wrap',
        wordBreak: 'break-word',
      }}
    >
      {text}
    </pre>
  )
}

export default function WhyPanel({ confidence, reasons, title }: WhyPanelProps) {
  const [open, setOpen] = useState(false)
  const hasContent = (reasons && reasons.length > 0) || !!confidence
  if (!hasContent) {
    return null
  }
  const reasonRows = reasons ?? []
  return (
    <div style={{ marginTop: 8 }}>
      <button
        type="button"
        onClick={() => setOpen(v => !v)}
        className="tool-btn"
        style={{ padding: '4px 10px', fontSize: 10, gap: 6 }}
      >
        <i
          className={`fa-duotone fa-solid ${open ? 'fa-caret-down' : 'fa-caret-right'}`}
          style={{ fontSize: 10 }}
        />
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            letterSpacing: '0.14em',
            textTransform: 'uppercase',
            color: 'var(--ink-3)',
          }}
        >
          {open ? 'Hide why' : title ?? 'Why?'}
        </span>
      </button>
      {open && (
        <div
          style={{
            marginTop: 8,
            padding: '12px 14px',
            background: 'var(--paper-2)',
            border: '1px solid var(--rule-soft)',
            borderRadius: 'var(--radius-sm)',
          }}
        >
          {reasonRows.length === 0 && (
            <p
              className="timestamp-print"
              style={{ margin: 0, marginBottom: 8 }}
            >
              // no reasons recorded
            </p>
          )}
          {reasonRows.map((r, i) => (
            <div
              key={i}
              style={{
                marginBottom: i === reasonRows.length - 1 ? 0 : 12,
                paddingBottom: i === reasonRows.length - 1 ? 0 : 12,
                borderBottom:
                  i === reasonRows.length - 1 ? 'none' : '1px dashed var(--rule-soft)',
              }}
            >
              <div className="flex items-center" style={{ gap: 10, flexWrap: 'wrap' }}>
                <CodeChip code={r.code} />
                <span
                  style={{ fontSize: 13.5, color: 'var(--ink-2)', flex: 1 }}
                >
                  {r.message}
                </span>
              </div>
              {r.detail && Object.keys(r.detail).length > 0 && (
                <DetailBlock detail={r.detail} />
              )}
            </div>
          ))}
          {confidence && (
            <div
              style={{
                marginTop: 12,
                paddingTop: 12,
                borderTop: '1px solid var(--rule-soft)',
              }}
            >
              <p
                className="ui-label"
                style={{
                  marginBottom: 8,
                  fontSize: 10,
                  letterSpacing: '0.18em',
                }}
              >
                Confidence breakdown
              </p>
              <ConfidenceBreakdown components={confidence.components} />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

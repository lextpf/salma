import { useState } from 'react'

import type { ConfidenceScore, FomodReason } from '../types'
import ConfidenceBreakdown from './ConfidenceBreakdown'
import MIcon from './MIcon'
import { HRule } from './Rule'

interface WhyPanelProps {
  confidence?: ConfidenceScore
  reasons?: FomodReason[]
  title?: string
}

const REASON_COLOR: Record<string, string> = {
  FORCED_REQUIRED: 'var(--moss)',
  FORCED_NOT_USABLE: 'var(--danger)',
  FORCED_SELECT_ALL: 'var(--moss)',
  FORCED_AT_LEAST_ONE: 'var(--moss)',
  FORCED_EXACTLY_ONE: 'var(--moss)',
  UNIQUE_FILE_EVIDENCE: 'var(--signal-2)',
  NO_FILE_EVIDENCE: 'var(--brass)',
  NO_UNIQUE_EVIDENCE: 'var(--ink-3)',
  CARDINALITY_FORCED: 'var(--signal-2)',
  CSP_PHASE_GREEDY: 'var(--signal-2)',
  CSP_PHASE_LOCAL_SEARCH: 'var(--signal-2)',
  CSP_PHASE_BACKTRACK: 'var(--brass)',
  CSP_PHASE_REPAIR: 'var(--brass)',
  CSP_PHASE_FOCUSED: 'var(--brass)',
  CSP_PHASE_FALLBACK: 'var(--danger)',
  CONDITION_FORCED_TRUE: 'var(--moss)',
  CONDITION_FORCED_FALSE: 'var(--moss)',
  CONDITION_UNKNOWN: 'var(--brass)',
  STEP_VISIBILITY_FORCED: 'var(--moss)',
  STEP_VISIBILITY_UNKNOWN: 'var(--brass)',
  STEP_NOT_VISIBLE: 'var(--ink-3)',
  EXTRA_FILE_PRODUCED: 'var(--danger)',
  FOMOD_PLUS_CACHE: 'var(--log-info)',
  IMPLICIT_DEFAULT: 'var(--ink-4)',
}

function CodeChip({ code }: { code: string }) {
  const color = REASON_COLOR[code] ?? 'var(--ink-3)'
  return (
    <span
      className="flex items-center"
      style={{
        gap: 6,
        flexShrink: 0,
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
        fontWeight: 600,
        letterSpacing: 'var(--tr-chip)',
        textTransform: 'uppercase',
        color: 'var(--ink-3)',
      }}
    >
      <span
        aria-hidden="true"
        style={{
          width: 6,
          height: 6,
          borderRadius: 'var(--radius-full)',
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
        padding: '7px 9px',
        borderRadius: 'var(--radius-chip)',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-label)',
        lineHeight: 1.55,
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
        onClick={() => { setOpen(v => !v); }}
        className="btn btn-ghost"
        aria-expanded={open}
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          height: 24,
          padding: '0 9px',
          fontSize: 'var(--fs-micro)',
          border: '1px solid var(--rule-ctrl)',
          borderRadius: 'var(--radius-ctrl)',
          background: 'var(--btn-bg)',
          color: 'var(--ink-4)',
        }}
      >
        <MIcon name={open ? 'arrow_drop_down' : 'arrow_right'} size={14} />
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontWeight: 600,
            letterSpacing: 'var(--tr-chip)',
            textTransform: 'uppercase',
            color: 'var(--ink-3)',
          }}
        >
          {open ? 'Hide why' : title ?? 'Why?'}
        </span>
      </button>
      {open && (
        <div style={{ marginTop: 10, paddingTop: 9 }}>
          <div
            className="flex items-center"
            style={{
              gap: 10,
              marginBottom: 9,
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-micro)',
              fontWeight: 600,
              textTransform: 'uppercase',
              letterSpacing: 'var(--tr-kicker)',
              color: 'var(--ink-5)',
            }}
          >
            <span>Decision log</span>
            <HRule />
            <span className="tabular-nums" style={{ letterSpacing: 'var(--tr-chip)' }}>
              {String(reasonRows.length).padStart(2, '0')}
              {reasonRows.length === 1 ? ' reason' : ' reasons'}
            </span>
          </div>
          {reasonRows.length === 0 && (
            <p
              style={{
                margin: 0,
                marginBottom: 8,
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-mono)',
                color: 'var(--ink-5)',
              }}
            >
              // no reasons recorded
            </p>
          )}
          {reasonRows.map((r, i) => (
            <div
              key={i}
              style={{
                marginBottom: i === reasonRows.length - 1 ? 0 : 9,
                paddingBottom: i === reasonRows.length - 1 ? 0 : 9,
              }}
            >
              <div className="flex items-baseline" style={{ gap: 10, flexWrap: 'wrap' }}>
                <CodeChip code={r.code} />
                <span
                  style={{
                    fontSize: 'var(--fs-body)',
                    lineHeight: 'var(--lh-body)',
                    color: 'var(--ink-2)',
                    flex: 1,
                    minWidth: 180,
                  }}
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
                marginTop: 10,
                paddingTop: 9,
              }}
            >
              <p
                className="flex items-center"
                style={{
                  margin: 0,
                  marginBottom: 8,
                  gap: 10,
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  fontWeight: 600,
                  textTransform: 'uppercase',
                  letterSpacing: 'var(--tr-kicker)',
                  color: 'var(--ink-5)',
                }}
              >
                <span>Confidence breakdown</span>
              </p>
              <ConfidenceBreakdown components={confidence.components} />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

import { useMemo } from 'react'
import { highlightJson } from '../../../jsonHighlight'
import type { FomodDetail } from '../../../types'

interface JsonTabProps {
  detail: FomodDetail
  name: string
}

// JSON tab: the raw schema-v2 record, syntax-highlighted with the shared
// tokenizer (extracted to jsonHighlight.ts). The .json-* span classes resolve to
// per-theme colors in index.css.
export default function JsonTab({ detail, name }: JsonTabProps) {
  const json = useMemo(() => JSON.stringify(detail, null, 2), [detail])
  const tokens = useMemo(() => highlightJson(json), [json])

  return (
    <div style={{ border: '1px solid var(--rule-soft)', borderRadius: 9, overflow: 'hidden' }}>
      <div
        style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'space-between',
          gap: 10,
          padding: '9px 14px',
          background: 'var(--card)',
          borderBottom: '1px solid var(--rule-soft)',
        }}
      >
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            letterSpacing: '0.1em',
            textTransform: 'uppercase',
            color: 'var(--ink-4)',
            whiteSpace: 'nowrap',
            overflow: 'hidden',
            textOverflow: 'ellipsis',
          }}
        >
          {name}
        </span>
        <span
          className="tabular-nums"
          style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-6)', flexShrink: 0 }}
        >
          {json.length.toLocaleString()} bytes
        </span>
      </div>
      <pre
        className="log-viewer"
        style={{
          margin: 0,
          padding: '14px 16px',
          overflowX: 'auto',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-label)',
          lineHeight: 1.7,
          color: 'var(--ink-2)',
          whiteSpace: 'pre',
        }}
      >
        {tokens.map((token, i) =>
          token.cls ? (
            <span key={i} className={token.cls}>
              {token.text}
            </span>
          ) : (
            <span key={i}>{token.text}</span>
          ),
        )}
      </pre>
    </div>
  )
}

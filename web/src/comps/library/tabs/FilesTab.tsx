import MIcon from '../../MIcon'
import { filesToTree, flattenTree } from '../../../filesToTree'
import { formatSize } from '../../../libraryFormat'
import type { FomodDetail } from '../../../types'

interface FilesTabProps {
  detail: FomodDetail
}

const NO_COLLAPSE: ReadonlySet<string> = new Set<string>()

// Files tab: the inferred virtual output tree, fully expanded, with a formatted
// byte size on every file row. Shows the truncation note when the backend capped
// the cached tree, and a quiet placeholder for records that predate the
// embedded-tree field.
export default function FilesTab({ detail }: FilesTabProps) {
  const entries = detail.outputTree ?? []

  if (entries.length === 0) {
    return (
      <div style={{ border: '1px solid var(--rule-soft)', borderRadius: 9, overflow: 'hidden' }}>
        <div
          style={{
            padding: '16px 14px',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-label)',
            color: 'var(--ink-5)',
          }}
        >
          // file tree pending - rescan to populate
        </div>
      </div>
    )
  }

  const rows = flattenTree(filesToTree(entries), NO_COLLAPSE)
  const truncated = detail.outputTreeTruncated === true
  const total = detail.outputTreeTotal ?? entries.length

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
          }}
        >
          Virtual output - {entries.length} entries
        </span>
        {truncated && (
          <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-6)' }}>
            showing first {entries.length} of {total}
          </span>
        )}
      </div>
      <div style={{ padding: '7px 0' }}>
        {rows.map(({ node, depth }) => (
          <div
            key={node.path}
            className="fm-row"
            title={node.source ?? node.path}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              padding: `6px 14px 6px ${14 + depth * 16}px`,
            }}
          >
            <span
              aria-hidden="true"
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                color: 'var(--ink-6)',
                width: 10,
                flexShrink: 0,
              }}
            >
              {node.isDir ? (
                <MIcon name="expand_more" size={12} />
              ) : (
                <MIcon name="fiber_manual_record" size={6} />
              )}
            </span>
            <MIcon
              name={node.isDir ? 'folder' : 'draft'}
              size={12}
              style={{ color: node.isDir ? 'var(--ink-4)' : 'var(--ink-6)', flexShrink: 0 }}
            />
            <span
              style={{
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-label)',
                color: node.isDir ? 'var(--ink-2)' : 'var(--ink-4)',
                whiteSpace: 'nowrap',
                overflow: 'hidden',
                textOverflow: 'ellipsis',
                flex: 1,
              }}
            >
              {node.name}
            </span>
            {!node.isDir && (
              <span
                className="tabular-nums"
                style={{
                  marginLeft: 'auto',
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-micro)',
                  color: 'var(--ink-6)',
                  flexShrink: 0,
                }}
              >
                {formatSize(node.size)}
              </span>
            )}
          </div>
        ))}
      </div>
    </div>
  )
}

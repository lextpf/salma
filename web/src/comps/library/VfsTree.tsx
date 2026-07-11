import { useState } from 'react'
import MIcon from '../MIcon'
import Chip from '../Chip'
import { filesToTree, flattenTree } from '../../filesToTree'
import type { FomodFileEntry } from '../../types'

interface VfsTreeProps {
  outputTree?: FomodFileEntry[]
  hasSelection: boolean
}

function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1) : ''
}

function QuietEmpty({ text }: { text: string }) {
  return (
    <div
      style={{
        padding: '18px 14px',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
        lineHeight: 1.7,
        color: 'var(--ink-6)',
      }}
    >
      {text}
    </div>
  )
}

// Column 1 of the Library triptych: the inferred virtual output tree of the
// selected record, with collapsible folders held in local state. Folder rows
// toggle; file rows carry a faint extension chip. The component is remounted by
// the parent (key={record}) so the collapsed set resets on record change.
export default function VfsTree({ outputTree, hasSelection }: VfsTreeProps) {
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set<string>())

  const toggle = (path: string) => {
    setCollapsed(prev => {
      const next = new Set(prev)
      if (next.has(path)) {
        next.delete(path)
      } else {
        next.add(path)
      }
      return next
    })
  }

  const roots = filesToTree(outputTree)
  const rows = flattenTree(roots, collapsed)

  return (
    <div
      style={{
        flex: '1 1 210px',
        minWidth: 180,
        maxWidth: 320,
        borderRight: '1px solid var(--rule-soft)',
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      <div
        style={{
          padding: '9px 14px',
          borderBottom: '1px solid var(--rule-soft)',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: '0.12em',
          textTransform: 'uppercase',
          color: 'var(--ink-5)',
          flexShrink: 0,
        }}
      >
        Virtual tree
      </div>
      <div
        className="scroll-pane"
        style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden', padding: '8px 6px' }}
      >
        {!hasSelection ? (
          <QuietEmpty text="// select a record to preview its file tree" />
        ) : rows.length === 0 ? (
          <QuietEmpty text="// no file tree - rescan to populate" />
        ) : (
          rows.map(({ node, depth }) => {
            const isOpen = node.isDir && !collapsed.has(node.path)
            const ext = node.isDir ? '' : extensionOf(node.name)
            return (
              <div
                key={node.path}
                className="fm-row"
                onClick={node.isDir ? () => toggle(node.path) : undefined}
                title={node.source ?? node.name}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 7,
                  padding: `6px 8px 6px ${8 + depth * 13}px`,
                  borderRadius: 6,
                  cursor: node.isDir ? 'pointer' : 'default',
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    color: 'var(--ink-6)',
                    width: 8,
                    flexShrink: 0,
                  }}
                >
                  {node.isDir ? (
                    <MIcon name={isOpen ? 'expand_more' : 'chevron_right'} size={12} />
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
                {ext && (
                  <span style={{ marginLeft: 'auto', flexShrink: 0 }}>
                    <Chip label={ext} color="var(--ink-4)" />
                  </span>
                )}
              </div>
            )
          })
        )}
      </div>
    </div>
  )
}

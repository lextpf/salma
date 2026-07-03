import { useMemo, useState } from 'react'
import MIcon from './MIcon'
import { filesToTree, flattenTree, type TreeNode } from '../filesToTree'
import { formatCount, formatSize } from '../libraryFormat'
import type { FaultKind, FomodFileEntry, ReproDetail } from '../types'

interface VfsTreeProps {
  outputTree?: FomodFileEntry[]
  hasSelection: boolean
  repro?: { missing: number; extra: number; size_mismatch: number; hash_mismatch: number; reproduced: number }
  /**
   * @brief fault paths, including installed-only files.
   */
  reproDetail?: ReproDetail
}

const FAULT_TONE: Record<FaultKind, string> = {
  missing: 'var(--danger)',
  hash_mismatch: 'var(--brass)',
  size_mismatch: 'var(--brass)',
  extra: 'var(--brass)',
}

const FAULT_MARK: Record<FaultKind, { glyph: string; label: string }> = {
  missing: { glyph: 'error', label: 'missing - the selection never writes this file' },
  hash_mismatch: { glyph: 'difference', label: 'hash mismatch - same size, different contents' },
  size_mismatch: { glyph: 'straighten', label: 'size mismatch' },
  extra: { glyph: 'add_circle', label: 'extra - written but not in the installed mod' },
}

function faultMap(detail?: ReproDetail): Map<string, FaultKind> | undefined {
  if (!detail) return undefined
  const m = new Map<string, FaultKind>()
  const put = (paths: string[] | undefined, kind: FaultKind) => {
    for (const p of paths ?? []) m.set(p, kind)
  }
  // visit weaker faults last so they cannot replace a stronger duplicate.
  put(detail.extra, 'extra')
  put(detail.size_mismatch, 'size_mismatch')
  put(detail.hash_mismatch, 'hash_mismatch')
  put(detail.missing, 'missing')
  return m
}

interface Fault {
  label: string
  count: number
  tone: string
}

function faultsOf(repro: VfsTreeProps['repro']): Fault[] {
  if (!repro) return []
  return [
    { label: 'missing', count: repro.missing, tone: 'var(--danger)' },
    { label: 'hash mm', count: repro.hash_mismatch, tone: 'var(--brass)' },
    { label: 'size mm', count: repro.size_mismatch, tone: 'var(--brass)' },
    { label: 'extra', count: repro.extra, tone: 'var(--brass)' },
  ].filter(f => f.count > 0)
}

// keep this CSS-pixel height synchronized with `RecordsList` and `Inspector`.
const HEAD_H = 34

function extensionOf(name: string): string {
  const dot = name.lastIndexOf('.')
  return dot > 0 && dot < name.length - 1 ? name.slice(dot + 1).toLowerCase() : ''
}

const KIND: Record<string, { tone: string; glyph: string }> = {
  esm: { tone: 'var(--moss)', glyph: 'description' },
  esp: { tone: 'var(--format-zip)', glyph: 'description' },
  esl: { tone: 'var(--format-zip)', glyph: 'description' },
  bsa: { tone: 'var(--format-7z)', glyph: 'inventory_2' },
  ba2: { tone: 'var(--format-7z)', glyph: 'inventory_2' },
  xml: { tone: 'var(--brass)', glyph: 'code' },
  json: { tone: 'var(--brass)', glyph: 'data_object' },
  ini: { tone: 'var(--brass)', glyph: 'code' },
}

function kindFor(name: string): { tone: string; glyph: string } {
  return KIND[extensionOf(name)] ?? { tone: 'var(--ink-5)', glyph: 'description' }
}

function countDirs(nodes: TreeNode[]): number {
  let n = 0
  for (const node of nodes) {
    if (node.isDir) {
      n += 1 + countDirs(node.children)
    }
  }
  return n
}

function QuietEmpty({ label, text }: { label: string; text: string }) {
  return (
    <div style={{ padding: '15px 15px', display: 'flex', flexDirection: 'column', gap: 7 }}>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: 'var(--tr-chip)',
          textTransform: 'uppercase',
          color: 'var(--ink-faint)',
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-mono)',
          lineHeight: 1.7,
          color: 'var(--ink-5)',
        }}
      >
        {text}
      </span>
    </div>
  )
}

export default function VfsTree({ outputTree, hasSelection, repro, reproDetail }: VfsTreeProps) {
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

  const marks = useMemo(() => faultMap(reproDetail), [reproDetail])
  const roots = useMemo(() => filesToTree(outputTree, marks), [outputTree, marks])
  const rows = flattenTree(roots, collapsed)
  const faults = faultsOf(repro)

  // count source entries so collapse state and grafted missing rows do not affect totals.
  const fileTotal = outputTree?.length ?? 0
  const dirTotal = useMemo(() => countDirs(roots), [roots])
  const byteTotal = useMemo(
    () => (outputTree ?? []).reduce((sum, f) => sum + (f.size ?? 0), 0),
    [outputTree],
  )

  return (
    <div
      style={{
        flex: '1 1 330px',
        minWidth: 232,
        maxWidth: 520,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      <div
        style={{
          height: HEAD_H,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          gap: 9,
          padding: '0 12px 0 15px',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: 'var(--tr-kicker)',
          textTransform: 'uppercase',
          color: 'var(--ink-faint)',
        }}
      >
        <span>Virtual output</span>
        <span style={{ flex: 1 }} />
        {fileTotal > 0 && (
          <span
            className="tabular-nums"
            style={{ letterSpacing: 'var(--tr-chip)', color: 'var(--ink-5)' }}
          >
            {formatCount(fileTotal)}
          </span>
        )}
      </div>

      {hasSelection && repro && (
        <div
          style={{
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            gap: 12,
            flexWrap: 'wrap',
            padding: '0 12px 8px 15px',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
          }}
        >
          {faults.length === 0 ? (
            <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6, color: 'var(--moss)' }}>
              <span aria-hidden="true" style={{ width: 4, height: 4, flexShrink: 0, background: 'var(--moss)' }} />
              all {formatCount(repro.reproduced)} reproduced
            </span>
          ) : (
            faults.map(f => (
              <span
                key={f.label}
                className="tabular-nums"
                title={`${f.count} ${f.label} across this record - marked in the rows below`}
                style={{ display: 'inline-flex', alignItems: 'center', gap: 6, color: f.tone }}
              >
                <span aria-hidden="true" style={{ width: 4, height: 4, flexShrink: 0, background: f.tone }} />
                {f.count} {f.label}
              </span>
            ))
          )}
        </div>
      )}
      <div
        className="scroll-pane"
        style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden', padding: '6px 0' }}
      >
        {!hasSelection ? (
          <QuietEmpty label="No record" text="// select a record to preview its file tree" />
        ) : rows.length === 0 ? (
          <QuietEmpty label="No tree" text="// rescan to populate the output tree" />
        ) : (
          rows.map(({ node, depth }) => {
            const isOpen = node.isDir && !collapsed.has(node.path)
            const kind = node.isDir ? null : kindFor(node.name)
            const root = node.isDir && depth === 0
            // collapsed directories retain their worst descendant fault.
            const fault = node.isDir ? (isOpen ? undefined : node.worstFault) : node.fault
            const faultTone = fault ? FAULT_TONE[fault] : undefined
            const tone = faultTone ?? (node.isDir ? (root ? 'var(--ink-4)' : 'var(--ink-5)') : kind!.tone)
            const textTone = node.isDir ? (root ? 'var(--ink)' : 'var(--ink-4)') : 'var(--ink-3)'
            const glyph = node.isDir ? (isOpen ? 'folder_open' : 'folder') : kind!.glyph
            const trailing = node.isDir
              ? (node.children.length > 0 ? node.children.length.toLocaleString() : '')
              : node.absent
                ? 'not written'
                : (node.size != null ? formatSize(node.size) : '')
            const mark = !node.isDir && node.fault ? FAULT_MARK[node.fault] : null
            return (
              <div
                key={node.path}
                className="fm-row"
                // CSS state keeps fault highlighting composable with hover.
                data-fault={fault}
                onClick={node.isDir ? () => toggle(node.path) : undefined}
                title={mark ? `${node.path} - ${mark.label}` : (node.source ?? node.name)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  height: 25,
                  padding: `0 12px 0 ${15 + depth * 15}px`,
                  fontFamily: 'var(--font-mono)',
                  fontSize: 'var(--fs-mono)',
                  whiteSpace: 'nowrap',
                  cursor: node.isDir ? 'pointer' : 'default',
                }}
              >
                <span
                  aria-hidden="true"
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'center',
                    color: 'var(--ink-4)',
                    width: 8,
                    flexShrink: 0,
                  }}
                >
                  {node.isDir && <MIcon name={isOpen ? 'expand_more' : 'chevron_right'} size={12} />}
                </span>
                <MIcon name={glyph} size={14} style={{ color: tone, flexShrink: 0 }} />
                <span
                  style={{
                    color: textTone,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    minWidth: 0,
                    textDecoration: node.absent ? 'line-through' : undefined,
                  }}
                >
                  {node.name}
                </span>
                {mark && (
                  <MIcon
                    name={mark.glyph}
                    size={13}
                    label={mark.label}
                    style={{ color: FAULT_TONE[node.fault!], flexShrink: 0 }}
                  />
                )}
                <span style={{ flex: 1 }} />
                {trailing && (
                  <span
                    className="tabular-nums"
                    style={{
                      flexShrink: 0,
                      fontSize: 'var(--fs-micro)',
                      color: 'var(--ink-5)',
                    }}
                  >
                    {trailing}
                  </span>
                )}
              </div>
            )
          })
        )}
      </div>
      {fileTotal > 0 && (
        <div
          className="tabular-nums"
          style={{
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            gap: 9,
            height: 26,
            padding: '0 12px 0 15px',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            color: 'var(--ink-5)',
          }}
        >
          <span>{formatCount(dirTotal)} dirs</span>
          <span aria-hidden="true" style={{ flex: 1, height: 1, background: 'var(--rule)' }} />
          {byteTotal > 0 && <span>{formatSize(byteTotal)}</span>}
        </div>
      )}
    </div>
  )
}

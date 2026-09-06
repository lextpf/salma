import { useMemo } from 'react'
import MIcon from './MIcon'
import SignalMeter from './SignalMeter'
import { tierFromEntry, type Tier } from '../confidence'
import { useVirtualScroll, ROW_RECORD, ROW_SEP } from '../useVirtualScroll'
import type { FomodEntry } from '../types'

interface RecordsListProps {
  fomods: FomodEntry[]
  selectedName: string | null
  onSelect: (name: string) => void
  loading: boolean
  totalCount: number
  tight?: boolean
  collapsed?: boolean
  onExpand?: () => void
}

const GRID = '34px minmax(0, 1fr) 78px'

// keep this CSS-pixel height synchronized with `VfsTree` and `Inspector`.
const HEAD_H = 34

// reserve the selection edge so state changes do not shift row content.
const EDGE = '2px solid'

type Row =
  | { kind: 'sep'; key: string; label: string; count: number }
  | { kind: 'rec'; key: string; entry: FomodEntry; pri: number }

// group strongest confidence first and sort names within each band.
const BANDS: { tier: Tier | 'NONE'; label: string }[] = [
  { tier: 'EXACT', label: 'Exact' },
  { tier: 'HIGH', label: 'High confidence' },
  { tier: 'PARTIAL', label: 'Partial - review' },
  { tier: 'LOW', label: 'Low - review' },
  { tier: 'NONE', label: 'Not scored' },
]

function buildRows(fomods: FomodEntry[]): Row[] {
  const buckets = new Map<string, FomodEntry[]>()
  for (const e of fomods) {
    const info = tierFromEntry(e)
    const key = info.hasData ? info.tier : 'NONE'
    const list = buckets.get(key)
    if (list) list.push(e)
    else buckets.set(key, [e])
  }

  // omit the only band header when no record has confidence data.
  const populated = BANDS.filter(b => (buckets.get(b.tier)?.length ?? 0) > 0)
  const flat = populated.length < 2

  const rows: Row[] = []
  let pri = 0
  for (const band of populated) {
    const list = (buckets.get(band.tier) ?? []).sort((a, b) => a.name.localeCompare(b.name))
    if (!flat) {
      rows.push({ kind: 'sep', key: `sep-${band.tier}`, label: band.label, count: list.length })
    }
    for (const entry of list) {
      pri += 1
      rows.push({ kind: 'rec', key: entry.name, entry, pri })
    }
  }
  return rows
}

function SeparatorRow({ label, count }: { label: string; count: number }) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 9,
        height: ROW_SEP,
        padding: '0 15px 0 9px',
        background: 'var(--sep-bg)',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
      }}
    >
      <span aria-hidden="true" style={{ width: 8, height: 1, flexShrink: 0, background: 'var(--rule-strong)' }} />
      <span
        style={{
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-lockup)',
          color: 'var(--ink-5)',
          whiteSpace: 'nowrap',
        }}
      >
        {label}
      </span>
      <span aria-hidden="true" style={{ flex: 1, height: 1, background: 'var(--rule)' }} />
      <span className="tabular-nums" style={{ flexShrink: 0, color: 'var(--ink-6)' }}>
        {count}
      </span>
    </div>
  )
}

export default function RecordsList({
  fomods,
  selectedName,
  onSelect,
  loading,
  totalCount,
  tight = false,
  collapsed = false,
  onExpand,
}: RecordsListProps) {
  const rows = useMemo(() => buildRows(fomods), [fomods])
  // separator and record rows require separate virtual heights.
  const heights = useMemo(
    () => rows.map(r => (r.kind === 'sep' ? ROW_SEP : ROW_RECORD)),
    [rows],
  )
  const { scrollRef, handleScroll, startIdx: getStartIdx, endIdx: getEndIdx, offsetOf, totalHeight } =
    useVirtualScroll(heights)

  const total = rows.length
  const startIdx = getStartIdx(total)
  const endIdx = getEndIdx(total)

  if (collapsed) {
    return (
      <button
        type="button"
        onClick={onExpand}
        title="Show the record list"
        aria-label="Show the record list"
        aria-expanded={false}
        style={{
          flex: '0 0 34px',
          width: 34,
          minWidth: 34,
          border: 'none',
          background: 'transparent',
          display: 'flex',
          flexDirection: 'column',
          alignItems: 'center',
          minHeight: 0,
          padding: 0,
          cursor: 'pointer',
        }}
      >
        <span
          style={{
            height: HEAD_H,
            flexShrink: 0,
            width: '100%',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            color: 'var(--ink-4)',
          }}
        >
          <MIcon name="chevron_right" size={15} />
        </span>
        <span
          className="tabular-nums"
          style={{
            marginTop: 13,
            writingMode: 'vertical-rl',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            letterSpacing: 'var(--tr-kicker)',
            textTransform: 'uppercase',
            color: 'var(--ink-faint)',
            whiteSpace: 'nowrap',
          }}
        >
          Mods {totalCount}
        </span>
      </button>
    )
  }

  return (
    <div
      style={{
        flex: '1 1 340px',
        minWidth: tight ? 210 : 238,
        maxWidth: 400,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: GRID,
          alignItems: 'stretch',
          gap: 10,
          height: HEAD_H,
          padding: '0 15px 0 0',
          borderLeft: `${EDGE} transparent`,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: 'var(--tr-kicker)',
          textTransform: 'uppercase',
          color: 'var(--ink-faint)',
          flexShrink: 0,
        }}
      >
        <span
          style={{
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'flex-end',
            paddingRight: 9,
          }}
        >
          Pri
        </span>
        <span style={{ display: 'flex', alignItems: 'center' }}>Mod</span>
        <span style={{ display: 'flex', alignItems: 'center', justifyContent: 'flex-end' }}>
          Confidence
        </span>
      </div>

      {loading && fomods.length === 0 ? (
        <div style={{ padding: '8px 0' }}>
          {[0, 1, 2, 3, 4].map(i => (
            <div
              key={i}
              style={{
                display: 'grid',
                gridTemplateColumns: GRID,
                alignItems: 'center',
                gap: 10,
                height: ROW_RECORD,
                padding: '0 15px 0 0',
                borderLeft: `${EDGE} transparent`,
              }}
            >
              <span />
              <span className="skeleton-line" style={{ height: 11, width: ['80%', '62%', '74%', '55%', '68%'][i] }} />
              <span />
            </div>
          ))}
        </div>
      ) : total === 0 ? (
        <div
          style={{
            padding: '15px 15px',
            display: 'flex',
            flexDirection: 'column',
            gap: 7,
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-mono)',
            lineHeight: 1.7,
          }}
        >
          <span
            style={{
              fontSize: 'var(--fs-micro)',
              letterSpacing: 'var(--tr-chip)',
              textTransform: 'uppercase',
              color: 'var(--ink-faint)',
            }}
          >
            {totalCount === 0 ? 'Library empty' : 'No match'}
          </span>
          <span style={{ color: 'var(--ink-5)' }}>
            {totalCount === 0 ? '// no records - run a scan' : '// no records match filter'}
          </span>
          {totalCount > 0 && (
            <span className="tabular-nums" style={{ fontSize: 'var(--fs-micro)', color: 'var(--ink-6)' }}>
              {totalCount} indexed
            </span>
          )}
        </div>
      ) : (
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="scroll-pane"
          style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}
        >
          <div style={{ height: offsetOf(startIdx) }} />
          {rows.slice(startIdx, endIdx).map(row => {
            if (row.kind === 'sep') {
              return <SeparatorRow key={row.key} label={row.label} count={row.count} />
            }
            const { entry, pri } = row
            const selected = entry.name === selectedName
            return (
              <div
                key={row.key}
                className="fm-row"
                data-selected={selected ? 'true' : undefined}
                onClick={() => { onSelect(entry.name); }}
                title={entry.name}
                style={{
                  display: 'grid',
                  gridTemplateColumns: GRID,
                  alignItems: 'center',
                  gap: 10,
                  height: ROW_RECORD,
                  padding: '0 15px 0 0',
                  cursor: 'pointer',
                  overflow: 'hidden',
                  background: selected ? 'var(--sel-bg)' : undefined,
                  borderLeft: `${EDGE} ${selected ? 'var(--sel-edge)' : 'transparent'}`,
                }}
              >
                <span
                  className="tabular-nums"
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--fs-micro)',
                    color: selected ? 'var(--signal)' : 'var(--ink-5)',
                    textAlign: 'right',
                    paddingRight: 9,
                    alignSelf: 'stretch',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'flex-end',
                  }}
                >
                  {String(pri).padStart(2, '0')}
                </span>
                <span
                  style={{
                    fontSize: 'var(--fs-body)',
                    letterSpacing: '-0.01em',
                    color: selected ? 'var(--ink)' : 'var(--ink-3)',
                    fontWeight: selected ? 700 : 400,
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {entry.name}
                </span>
                <SignalMeter confidence={entry.confidence} exactMatch={entry.exactMatch} lit={selected} />
              </div>
            )
          })}
          <div style={{ height: Math.max(0, totalHeight(total) - offsetOf(endIdx)) }} />
        </div>
      )}
    </div>
  )
}

import SignalMeter from '../SignalMeter'
import { useVirtualScroll, LINE_HEIGHT } from '../../useVirtualScroll'
import type { FomodEntry } from '../../types'

interface RecordsListProps {
  fomods: FomodEntry[]
  selectedName: string | null
  onSelect: (name: string) => void
  loading: boolean
  totalCount: number
}

const GRID = '30px minmax(0, 1fr) 56px'

// Column 2 of the triptych: the priority-ordered record list. Rows are a fixed
// LINE_HEIGHT so the shared virtual-scroll windowing (from useVirtualScroll)
// keeps long libraries cheap. The selected row gets the field-manual treatment:
// a zebra fill, an inset ink left bar, and a bold name.
export default function RecordsList({
  fomods,
  selectedName,
  onSelect,
  loading,
  totalCount,
}: RecordsListProps) {
  const { scrollRef, handleScroll, startIdx: getStartIdx, endIdx: getEndIdx } = useVirtualScroll()
  const total = fomods.length
  const startIdx = getStartIdx(total)
  const endIdx = getEndIdx(total)

  return (
    <div
      style={{
        flex: '1 1 280px',
        minWidth: 250,
        maxWidth: 380,
        borderRight: '1px solid var(--rule-soft)',
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: GRID,
          alignItems: 'center',
          gap: 9,
          padding: '9px 14px 9px 0',
          borderBottom: '1px solid var(--rule-soft)',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: '0.1em',
          textTransform: 'uppercase',
          color: 'var(--ink-5)',
          flexShrink: 0,
        }}
      >
        <span style={{ textAlign: 'right', paddingRight: 8 }}>#</span>
        <span>Mod</span>
        <span style={{ textAlign: 'right' }}>Conf</span>
      </div>

      {loading && total === 0 ? (
        <div style={{ padding: '8px 0' }}>
          {[0, 1, 2, 3, 4].map(i => (
            <div
              key={i}
              style={{
                display: 'grid',
                gridTemplateColumns: GRID,
                alignItems: 'center',
                gap: 9,
                height: LINE_HEIGHT,
                padding: '0 14px 0 0',
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
            padding: '16px 14px',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            lineHeight: 1.7,
            color: 'var(--ink-6)',
          }}
        >
          {totalCount === 0 ? '// no records - run a scan' : '// no records match filter'}
        </div>
      ) : (
        <div
          ref={scrollRef}
          onScroll={handleScroll}
          className="scroll-pane"
          style={{ flex: 1, minHeight: 0, overflowY: 'auto', overflowX: 'hidden' }}
        >
          <div style={{ height: startIdx * LINE_HEIGHT }} />
          {fomods.slice(startIdx, endIdx).map((entry, i) => {
            const idx = startIdx + i
            const selected = entry.name === selectedName
            return (
              <div
                key={entry.name}
                className="fm-row"
                data-selected={selected ? 'true' : undefined}
                onClick={() => onSelect(entry.name)}
                title={entry.name}
                style={{
                  display: 'grid',
                  gridTemplateColumns: GRID,
                  alignItems: 'center',
                  gap: 9,
                  height: LINE_HEIGHT,
                  padding: '0 14px 0 0',
                  cursor: 'pointer',
                  background: selected ? 'var(--card-2)' : 'transparent',
                  boxShadow: selected ? 'inset 2.5px 0 0 var(--ink)' : 'none',
                  borderBottom: '1px solid var(--rule-faint)',
                }}
              >
                <span
                  className="tabular-nums"
                  style={{
                    fontFamily: 'var(--font-mono)',
                    fontSize: 'var(--fs-micro)',
                    color: 'var(--ink-6)',
                    textAlign: 'right',
                    paddingRight: 8,
                    alignSelf: 'stretch',
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'flex-end',
                    borderRight: '1px solid var(--rule-faint)',
                  }}
                >
                  {String(idx + 1).padStart(2, '0')}
                </span>
                <span
                  style={{
                    fontSize: 'var(--fs-body)',
                    color: selected ? 'var(--ink)' : 'var(--ink-2)',
                    fontWeight: selected ? 600 : 400,
                    whiteSpace: 'nowrap',
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                  }}
                >
                  {entry.name}
                </span>
                <span style={{ display: 'flex', justifyContent: 'flex-end' }}>
                  <SignalMeter confidence={entry.confidence} exactMatch={entry.exactMatch} />
                </span>
              </div>
            )
          })}
          <div style={{ height: Math.max(0, total - endIdx) * LINE_HEIGHT }} />
        </div>
      )}
    </div>
  )
}

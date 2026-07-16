import type { SubsystemFacet } from '../../logParse'

interface SubsystemFacetsProps {
  facets: SubsystemFacet[]
  active: string | null
  total: number
  onToggle: (tag: string) => void
  onClear: () => void
}

interface FacetRowProps {
  label: string
  count: number
  on: boolean
  onClick: () => void
}

function FacetRow({ label, count, on, onClick }: FacetRowProps) {
  return (
    <button
      type="button"
      className="fm-row"
      data-selected={on || undefined}
      aria-pressed={on}
      onClick={onClick}
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'space-between',
        gap: 8,
        width: '100%',
        padding: '6px 9px',
        border: 'none',
        borderRadius: 6,
        cursor: 'pointer',
        textAlign: 'left',
        background: on ? 'var(--card-2)' : 'transparent',
      }}
    >
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-label)',
          color: on ? 'var(--ink)' : 'var(--ink-3)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          color: on ? 'var(--ink)' : 'var(--ink-6)',
          flexShrink: 0,
        }}
      >
        {count}
      </span>
    </button>
  )
}

// The 158px left rail: an "All" row (clears the filter) plus the distinct
// subsystem tags with counts. Clicking a tag filters the stream to it; clicking
// "All" (or the active tag again) restores every subsystem.
export default function SubsystemFacets({
  facets,
  active,
  total,
  onToggle,
  onClear,
}: SubsystemFacetsProps) {
  return (
    <aside
      style={{
        width: 158,
        flexShrink: 0,
        borderRight: '1px solid var(--rule-soft)',
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
      }}
    >
      <div
        style={{
          flexShrink: 0,
          padding: '9px 14px',
          borderBottom: '1px solid var(--rule-soft)',
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: '0.12em',
          textTransform: 'uppercase',
          color: 'var(--ink-5)',
        }}
      >
        Subsystem
      </div>

      <div
        style={{
          flex: 1,
          minHeight: 0,
          overflowY: 'auto',
          overflowX: 'hidden',
          padding: 7,
          display: 'flex',
          flexDirection: 'column',
          gap: 2,
        }}
      >
        <FacetRow label="All" count={total} on={active === null} onClick={onClear} />

        {facets.length === 0 ? (
          <div
            style={{
              padding: '8px 9px',
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-micro)',
              color: 'var(--ink-5)',
            }}
          >
            no subsystems
          </div>
        ) : (
          facets.map((f) => (
            <FacetRow
              key={f.tag}
              label={f.tag}
              count={f.count}
              on={f.tag === active}
              onClick={() => onToggle(f.tag)}
            />
          ))
        )}
      </div>
    </aside>
  )
}

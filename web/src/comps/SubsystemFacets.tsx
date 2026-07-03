import type { SubsystemFacet } from '../logParse'

interface SubsystemFacetsProps {
  facets: SubsystemFacet[]
  active: string | null
  total: number
  onToggle: (tag: string) => void
  onClear: () => void
}

interface FacetChipProps {
  label: string
  count: number
  share: number
  on: boolean
  onClick: () => void
}

// omit an inline background so stylesheet hover state remains effective.
function FacetChip({ label, count, share, on, onClick }: FacetChipProps) {
  return (
    <button
      type="button"
      className="fm-row"
      data-selected={on ? 'true' : undefined}
      aria-pressed={on}
      onClick={onClick}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 7,
        flexShrink: 0,
        maxWidth: 200,
        height: 22,
        padding: '0 8px 0 6px',
        border: 'none',
        borderLeft: `2px solid ${on ? 'var(--sel-edge)' : 'transparent'}`,
        background: on ? 'var(--sel-bg)' : undefined,
        cursor: 'pointer',
        fontFamily: 'var(--font-mono)',
      }}
    >
      <span
        style={{
          minWidth: 0,
          fontSize: 'var(--fs-mono)',
          fontWeight: on ? 700 : 400,
          color: on ? 'var(--ink)' : 'var(--ink-4)',
          whiteSpace: 'nowrap',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
        }}
      >
        {label}
      </span>
      <span
        aria-hidden="true"
        style={{ width: 22, height: 3, flexShrink: 0, background: 'var(--mini-track)', overflow: 'hidden' }}
      >
        <span
          style={{
            display: 'block',
            height: '100%',
            width: `${Math.max(4, Math.round(share * 100))}%`,
            background: on ? 'var(--signal)' : 'var(--meter-bar)',
          }}
        />
      </span>
      <span
        className="tabular-nums"
        style={{
          flexShrink: 0,
          fontSize: 'var(--fs-micro)',
          color: on ? 'var(--signal)' : 'var(--ink-5)',
        }}
      >
        {count.toLocaleString()}
      </span>
    </button>
  )
}

export default function SubsystemFacets({ facets, active, total, onToggle, onClear }: SubsystemFacetsProps) {
  // normalize mini-bars against the largest subsystem, not the total.
  const facetMax = Math.max(1, ...facets.map(f => f.count))

  return (
    <div
      role="group"
      aria-label="Filter by subsystem"
      style={{
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        rowGap: 4,
        flexWrap: 'wrap',
        padding: '5px 18px 9px',
      }}
    >
      <span
        style={{
          flexShrink: 0,
          marginRight: 3,
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          letterSpacing: 'var(--tr-kicker)',
          textTransform: 'uppercase',
          color: 'var(--ink-faint)',
        }}
      >
        Subsystem
      </span>

      <FacetChip label="All" count={total} share={1} on={active === null} onClick={onClear} />

      {facets.length === 0 ? (
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-6)' }}>
          no subsystems
        </span>
      ) : (
        facets.map(f => (
          <FacetChip
            key={f.tag}
            label={f.tag}
            count={f.count}
            share={f.count / facetMax}
            on={active === f.tag}
            onClick={() => onToggle(f.tag)}
          />
        ))
      )}
    </div>
  )
}

export interface TabItem {
  id: string
  label: string
}

interface TabsProps {
  items: TabItem[]
  active: string
  onChange: (id: string) => void
  // 'underline' = inspector content tabs; 'segment' = the Logs switches.
  variant?: 'underline' | 'segment'
  /** Segment size: 'source' is the wider mixed-case row, 'level' the tracked one. */
  size?: 'source' | 'level'
  label?: string
}

/**
 * Two tab treatments sharing one keyboard-accessible button row.
 *
 * `segment` is a segmented control drawn flat: one shell fill inside a
 * hairline, with the active segment marked by a signal wash and signal ink
 * rather than raised out of a well. It keeps its edge because it is a control,
 * which is one of the cases index.css allows a border.
 *
 * `underline` is the inspector's content switch: the active tab carries a flat
 * 2px signal bar and the strip has no rule of its own. Neither state depends on
 * a shadow, so both read the same in either theme.
 */
export default function Tabs({
  items,
  active,
  onChange,
  variant = 'underline',
  size = 'source',
  label,
}: TabsProps) {
  if (variant === 'segment') {
    const level = size === 'level'
    return (
      <div
        role="tablist"
        aria-label={label}
        style={{
          display: 'inline-flex',
          flexShrink: 0,
          padding: 2,
          borderRadius: 'var(--radius-ctrl)',
          border: '1px solid var(--rule-soft)',
          background: 'var(--seg-bg)',
        }}
      >
        {items.map((it) => {
          const on = it.id === active
          return (
            <button
              key={it.id}
              type="button"
              role="tab"
              className="seg-item"
              aria-selected={on}
              onClick={() => { onChange(it.id); }}
              style={{
                padding: level ? '5px 10px' : '5px 12px',
                borderRadius: 'var(--radius-seg)',
                border: 'none',
                background: on ? 'var(--signal-wash-sel)' : 'transparent',
                color: on ? 'var(--signal-2)' : 'var(--ink-4)',
                fontFamily: 'var(--font-mono)',
                fontSize: level ? 'var(--fs-micro)' : 'var(--fs-label)',
                fontWeight: level ? 700 : 600,
                letterSpacing: level ? 'var(--tr-chip)' : 0,
              }}
            >
              {it.label}
            </button>
          )
        })}
      </div>
    )
  }

  return (
    <div role="tablist" aria-label={label} style={{ display: 'flex', flexShrink: 0 }}>
      {items.map((it) => {
        const on = it.id === active
        return (
          <button
            key={it.id}
            type="button"
            role="tab"
            className="seg-item"
            aria-selected={on}
            onClick={() => onChange(it.id)}
            style={{
              position: 'relative',
              padding: '11px 15px',
              background: 'transparent',
              border: 'none',
              color: on ? 'var(--ink)' : 'var(--ink-5)',
              fontFamily: 'var(--font-mono)',
              fontSize: 'var(--fs-label)',
              fontWeight: on ? 700 : 500,
              textTransform: 'uppercase',
              letterSpacing: 'var(--tr-chip)',
            }}
          >
            {it.label}
            {on && (
              <span
                aria-hidden="true"
                style={{
                  position: 'absolute',
                  left: 8,
                  right: 8,
                  // Flush with the strip. The strip has no bottom border for
                  // the marker to cover, so a negative offset would float it a
                  // pixel clear of the tab it belongs to.
                  bottom: 0,
                  height: 2,
                  background: 'var(--signal)',
                }}
              />
            )}
          </button>
        )
      })}
    </div>
  )
}

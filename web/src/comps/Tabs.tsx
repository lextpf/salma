export interface TabItem {
  id: string
  label: string
}

interface TabsProps {
  items: TabItem[]
  active: string
  onChange: (id: string) => void
  variant?: 'underline' | 'segment'
  size?: 'source' | 'level'
  label?: string
}

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
            onClick={() => { onChange(it.id); }}
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
                  // keep the marker flush with the borderless strip.
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

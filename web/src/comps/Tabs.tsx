export interface TabItem {
  id: string
  label: string
}

interface TabsProps {
  items: TabItem[]
  active: string
  onChange: (id: string) => void
  // 'underline' = inspector content tabs; 'pill' = the Logs level switch.
  variant?: 'underline' | 'pill'
}

// Two tab treatments sharing one keyboard-accessible button row: an underline
// bar for the inspector, and a bordered mono-pill group for log levels.
export default function Tabs({ items, active, onChange, variant = 'underline' }: TabsProps) {
  if (variant === 'pill') {
    return (
      <div
        role="tablist"
        style={{
          display: 'inline-flex',
          gap: 2,
          border: '1px solid var(--rule)',
          borderRadius: 8,
          padding: 2,
        }}
      >
        {items.map((it) => {
          const on = it.id === active
          return (
            <button
              key={it.id}
              type="button"
              role="tab"
              aria-selected={on}
              onClick={() => { onChange(it.id); }}
              style={{
                padding: '5px 12px',
                borderRadius: 6,
                border: 'none',
                background: on ? 'var(--ink)' : 'transparent',
                color: on ? 'var(--sheet)' : 'var(--ink-4)',
                fontFamily: 'var(--font-mono)',
                fontSize: 'var(--fs-micro)',
                fontWeight: 600,
                letterSpacing: '0.06em',
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
    <div role="tablist" style={{ display: 'flex', gap: 2 }}>
      {items.map((it) => {
        const on = it.id === active
        return (
          <button
            key={it.id}
            type="button"
            role="tab"
            aria-selected={on}
            onClick={() => onChange(it.id)}
            style={{
              padding: '9px 13px',
              background: 'transparent',
              border: 'none',
              borderBottom: `2px solid ${on ? 'var(--ink)' : 'transparent'}`,
              color: on ? 'var(--ink)' : 'var(--ink-4)',
              fontFamily: 'var(--font-body)',
              fontSize: 'var(--fs-body)',
              fontWeight: on ? 600 : 500,
            }}
          >
            {it.label}
          </button>
        )
      })}
    </div>
  )
}

interface StatusCardProps {
  label: string
  value: string | number
  detail?: string
  icon?: string
  color?: 'primary' | 'secondary' | 'tertiary' | 'success' | 'warning' | 'error'
}

const colorVar: Record<NonNullable<StatusCardProps['color']>, string> = {
  primary:   'var(--accent)',
  secondary: 'var(--ink-blue)',
  tertiary:  'var(--ink-2)',
  success:   'var(--moss)',
  warning:   'var(--ochre)',
  error:     'var(--danger)',
}

export default function StatusCard({ label, value, detail, icon, color = 'primary' }: StatusCardProps) {
  return (
    <div
      style={{
        position: 'relative',
        background: 'var(--card)',
        border: '1px solid var(--rule)',
        borderRadius: 'var(--radius-md)',
        padding: '18px 20px',
      }}
    >
      <div className="flex items-start justify-between" style={{ gap: 12 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          <p className="ui-label" style={{ marginBottom: 8, fontSize: 11 }}>{label}</p>
          <p
            className="display-serif-tight tabular-nums"
            style={{ fontSize: 26, color: colorVar[color], lineHeight: 1, margin: 0 }}
          >
            {value}
          </p>
          {detail && (
            <p
              className="timestamp-print"
              style={{ marginTop: 8, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}
              title={detail}
            >
              {detail}
            </p>
          )}
        </div>
        {icon && (
          <div
            style={{
              flexShrink: 0,
              width: 36,
              height: 36,
              borderRadius: 'var(--radius-md)',
              background: 'var(--paper-2)',
              border: '1px solid var(--rule-soft)',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
              color: colorVar[color],
            }}
          >
            <i className={icon} style={{ fontSize: 14 }} />
          </div>
        )}
      </div>
    </div>
  )
}

interface StatusCardProps {
  label: string
  value: string | number
  detail?: string
  icon?: string
  color?: 'primary' | 'secondary' | 'tertiary' | 'success' | 'warning' | 'error'
}

const gradientClass = {
  primary:   'icon-gradient-ocean',
  secondary: 'icon-gradient-forest',
  tertiary:  'icon-gradient-orchid',
  success:   'icon-gradient-spring',
  warning:   'icon-gradient-ember',
  error:     'icon-gradient-ember',
}

const colorConfig = {
  primary:   { text: 'text-primary' },
  secondary: { text: 'text-secondary' },
  tertiary:  { text: 'text-tertiary' },
  success:   { text: 'text-success' },
  warning:   { text: 'text-warning' },
  error:     { text: 'text-error' },
}

export default function StatusCard({ label, value, detail, icon, color = 'primary' }: StatusCardProps) {
  const c = colorConfig[color]
  const g = gradientClass[color]
  return (
    <div
      className="relative overflow-hidden rounded-2xl aurora-card p-5
                  status-card-soft group border-0"
    >
      <div className="flex items-start justify-between gap-3">
        <div className="flex-1 min-w-0">
          <p className="text-[0.65rem] font-semibold uppercase tracking-[0.12em] text-on-surface-variant mb-2">{label}</p>
          <p className={`text-2xl font-bold tracking-tight ${c.text}`}>{value}</p>
          {detail && <p className="text-[0.7rem] text-on-surface-variant mt-2 truncate leading-relaxed">{detail}</p>}
        </div>
        {icon && (
          <div className="shrink-0 w-10 h-10 rounded-xl bg-surface-container-high/55 border border-outline-variant/25 flex items-center justify-center shadow-[0_10px_20px_-16px_rgba(92,207,255,0.55)]">
            <i className={`${icon} icon-gradient ${g} text-lg`} />
          </div>
        )}
      </div>
    </div>
  )
}

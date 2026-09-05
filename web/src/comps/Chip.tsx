interface ChipProps {
  label: string
  color?: string
  title?: string
}

export default function Chip({ label, color = 'var(--ink-3)', title }: ChipProps) {
  return (
    <span
      title={title}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
        fontWeight: 600,
        letterSpacing: 'var(--tr-chip)',
        color,
        border: `1px solid color-mix(in srgb, ${color} 40%, transparent)`,
        background: `color-mix(in srgb, ${color} 11%, transparent)`,
        borderRadius: 'var(--radius-chip)',
        padding: '1.5px 7px',
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </span>
  )
}

interface ChipProps {
  label: string
  // Any CSS color or var; border/background derive from it via color-mix.
  color?: string
  title?: string
}

/**
 * Mono micro-chip: the label in `color`, a visible `color` border and a faint
 * `color` wash - one flat fill step inside one hairline, never a shadow. Used
 * for spec-rail badges, group-type tags, file badges.
 */
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

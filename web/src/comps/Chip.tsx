interface ChipProps {
  label: string
  // Any CSS color or var; border/background derive from it via color-mix.
  color?: string
  title?: string
}

// Mono micro-chip: the label in @color, a faint @color border and an even
// fainter @color wash. Used for spec-rail badges, group-type tags, file badges.
export default function Chip({ label, color = 'var(--ink-3)', title }: ChipProps) {
  return (
    <span
      title={title}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-micro)',
        letterSpacing: '0.03em',
        color,
        border: `1px solid color-mix(in srgb, ${color} 28%, transparent)`,
        background: `color-mix(in srgb, ${color} 7%, transparent)`,
        borderRadius: 4,
        padding: '1.5px 7px',
        whiteSpace: 'nowrap',
      }}
    >
      {label}
    </span>
  )
}

interface KickerProps {
  // Mono index like "01"; the square bullet renders even without it.
  num?: string
  label: string
}

// The field-manual section marker: a mono index, a small ink square bullet, and
// a tracked uppercase label. Used in page header bars and the module rail.
export default function Kicker({ num, label }: KickerProps) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 9 }}>
      {num && (
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-label)',
            fontWeight: 600,
            color: 'var(--ink)',
          }}
        >
          {num}
        </span>
      )}
      <span aria-hidden="true" style={{ width: 5, height: 5, background: 'var(--ink)' }} />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: '0.14em',
          color: 'var(--ink-3)',
        }}
      >
        {label}
      </span>
    </span>
  )
}

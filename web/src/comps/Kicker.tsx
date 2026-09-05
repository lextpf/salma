interface KickerProps {
  num?: string
  label: string
}

export default function Kicker({ num, label }: KickerProps) {
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 9, flexShrink: 0 }}>
      {num && (
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-sm)',
            fontWeight: 700,
            letterSpacing: '0.02em',
            color: 'var(--signal)',
          }}
        >
          {num}
        </span>
      )}
      <span
        aria-hidden="true"
        style={{
          width: 4,
          height: 4,
          background: 'var(--signal)',
        }}
      />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          fontWeight: 600,
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-kicker)',
          color: 'var(--ink-4)',
        }}
      >
        {label}
      </span>
    </span>
  )
}

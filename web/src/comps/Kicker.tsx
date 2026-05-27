interface KickerProps {
  num?: string
  label: string
}

/**
 * The module marker: mono numeral, a small solid signal square, then the
 * tracked uppercase label. The numeral and the square are the accent; the
 * label stays quiet so the pair reads as an instrument legend rather than a
 * title. The square is a flat mark, not a lit one.
 */
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

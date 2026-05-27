/**
 * Separator rules.
 *
 * A flat 1px line, drawn rather than lit: full strength end to end, since a
 * fade would read as a light source. VRule separates chrome groups; HRule fills
 * the remainder of a section-header row.
 */

export function VRule({ height = 20 }: { height?: number }) {
  return (
    <span
      aria-hidden="true"
      style={{
        width: 1,
        height,
        flexShrink: 0,
        background: 'var(--rule)',
      }}
    />
  )
}

export function HRule({ flex = true }: { flex?: boolean }) {
  return (
    <span
      aria-hidden="true"
      style={{
        height: 1,
        flex: flex ? 1 : undefined,
        minWidth: 12,
        background: 'var(--rule)',
      }}
    />
  )
}

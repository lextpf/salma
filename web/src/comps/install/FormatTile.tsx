import type { FormatSpec } from './formats'

// The tone-washed icon square used inline in job lines, the active card header,
// and the drag-over release card.
export function FormatTile({ spec, size = 18 }: { spec: FormatSpec; size?: number }) {
  return (
    <span
      aria-hidden="true"
      style={{
        width: size,
        height: size,
        flexShrink: 0,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        borderRadius: Math.round(size * 0.22),
        color: spec.tone,
        background: `color-mix(in srgb, ${spec.tone} 11%, transparent)`,
      }}
    >
      <i className={`fa-solid ${spec.icon}`} style={{ fontSize: Math.round(size * 0.5) }} />
    </span>
  )
}

// The bordered legend pill: a tile plus the extension label. Used in the idle
// feed legend and the empty-state invitation.
export function FormatChip({ spec }: { spec: FormatSpec }) {
  return (
    <span
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        padding: '3px 8px 3px 4px',
        border: '1px solid var(--rule)',
        borderRadius: 6,
        background: 'var(--sheet)',
      }}
    >
      <FormatTile spec={spec} size={18} />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          fontWeight: 600,
          color: spec.tone,
        }}
      >
        {spec.extension}
      </span>
    </span>
  )
}

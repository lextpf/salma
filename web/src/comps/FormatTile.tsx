import MIcon from './MIcon'
import type { FormatSpec } from './formats'

export function FormatTile({ spec, size = 22 }: { spec: FormatSpec; size?: number }) {
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
        borderRadius: 'var(--radius-chip)',
        color: spec.tone,
        background: `color-mix(in srgb, ${spec.tone} 16%, transparent)`,
      }}
    >
      <MIcon name={spec.icon} size={Math.round(size * 0.62)} />
    </span>
  )
}

export function QuietTile({ size = 22 }: { size?: number }) {
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
        borderRadius: 'var(--radius-chip)',
        color: 'var(--ink-5)',
        background: 'var(--chip-bg)',
      }}
    >
      <MIcon name="draft" size={Math.round(size * 0.62)} />
    </span>
  )
}

import MIcon from './MIcon'
import type { FormatSpec } from './formats'

/**
 * The format badge used in job rows and the active card header.
 *
 * A flat wash of the format's own colour with a square-ish corner and no edge:
 * at row scale an outline fights the row's hairline, and the tile only has to
 * say "which format", not "I am an object". The glyph box scales with the
 * tile so the 28px card-header tile and the 18px row tile read the same.
 *
 * The glyph's size is the one numeric size left in these files: it is an icon
 * box, not text, so it is off the type scale (the same exemption MIcon's `size`
 * prop has).
 */
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

/**
 * A neutral tile for a job with no archive of its own (a queued placeholder).
 * `draft` rather than `archive`: .zip owns the archive glyph, and a queued job
 * with nothing resolved yet should not wear a format's mark.
 */
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

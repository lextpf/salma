import type { CSSProperties } from 'react'

interface MIconProps {
  /** Material Symbols ligature name, e.g. "search" or "expand_more". */
  name: string
  /** Icon box in px; sets font-size and the opsz axis. Default 14. */
  size?: number
  /** Filled glyph variant (FILL axis). */
  fill?: boolean
  /** wght axis, 100-700. Default 400. */
  weight?: number
  className?: string
  style?: CSSProperties
  /** Accessible label; omitted = decorative (aria-hidden). */
  label?: string
}

// Material Symbols Outlined wrapper - the primary icon language of the UI.
// Color inherits currentColor; hover/active color changes on parents apply
// automatically (unlike FA duotone, which needs the --fa-* var plumbing).
export default function MIcon({
  name,
  size = 14,
  fill = false,
  weight = 400,
  className,
  style,
  label,
}: MIconProps) {
  const opsz = Math.min(Math.max(size, 20), 48)
  return (
    <span
      className={`material-symbols-outlined m-icon${className ? ` ${className}` : ''}`}
      style={{
        fontSize: size,
        fontVariationSettings: `'FILL' ${fill ? 1 : 0}, 'wght' ${weight}, 'GRAD' 0, 'opsz' ${opsz}`,
        ...style,
      }}
      aria-hidden={label ? undefined : true}
      aria-label={label}
      role={label ? 'img' : undefined}
    >
      {name}
    </span>
  )
}

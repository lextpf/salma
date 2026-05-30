import type { CSSProperties } from 'react'

interface MIconProps {
  /** Material Symbols ligature name, e.g. "search" or "expand_more". */
  name: string
  /**
   * Icon box in CSS pixels. Sets font-size directly. Default 14.
   *
   * The opsz axis follows it, clamped to 20-48, which is the range the variable
   * font supports. Every size below 20 therefore renders at opsz 20, the default
   * of 14 included: moving `size` on a small icon changes its box, not its
   * optical size.
   */
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

// Material Symbols Outlined wrapper, and the only icon language in the UI.
// Colour inherits from currentColor, so a parent's hover and active colours
// apply on their own with no per-icon plumbing.
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

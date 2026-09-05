import type { CSSProperties } from 'react'

/**
 * @interface MIconProps
 * @brief constrain variable icon axes and accessible labeling.
 * @author Alex (https://github.com/lextpf)
 *
 * `size` is in CSS pixels and optical size clamps to [20, 48]. `weight` is in
 * [100, 700] and defaults to 400. omit `label` for decorative icons.
 */
interface MIconProps {
  name: string
  size?: number
  fill?: boolean
  weight?: number
  className?: string
  style?: CSSProperties
  label?: string
}

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

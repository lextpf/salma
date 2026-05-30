import type { CSSProperties, ReactNode } from 'react'
import MIcon from './MIcon'

export type ButtonVariant = 'ghost' | 'primary' | 'danger'

interface ButtonProps {
  label: string
  icon?: string
  onClick?: () => void
  href?: string
  variant?: ButtonVariant
  disabled?: boolean
  /** Swaps the icon for a spinner and keeps the button interactive-looking. */
  running?: boolean
  /** Icon-only rendering for narrow docks; the label moves to title/aria. */
  compact?: boolean
  title?: string
  children?: ReactNode
}

/**
 * The one button in the app.
 *
 * Three variants, one geometry: same height, padding, radius and gap, so a
 * toolbar row aligns on every edge. Nothing here is lit - each variant is a
 * fill plus a visible 1px border, which is the only thing that has to say
 * "pressable". `ghost` is the default toolbar control, a quiet fill inside a
 * `--rule-ctrl` edge. `primary` is a flat `--signal` plate and there should be
 * at most one per screen - it is the screen's single "do the thing" affordance.
 * `danger` is outlined over a faint wash, never filled, so a destructive action
 * can sit in a toolbar without shouting. Hover and active states live in
 * index.css on `.btn` / `.btn-primary` / `.btn-danger`.
 */
export default function Button({
  label,
  icon,
  onClick,
  href,
  variant = 'ghost',
  disabled = false,
  running = false,
  compact = false,
  title,
  children,
}: ButtonProps) {
  const primary = variant === 'primary'

  const base: CSSProperties = {
    display: 'inline-flex',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 7,
    height: 30,
    flexShrink: 0,
    padding: compact ? 0 : '0 12px',
    width: compact ? 30 : undefined,
    borderRadius: 'var(--radius-ctrl)',
    fontFamily: 'inherit',
    letterSpacing: '-0.01em',
    whiteSpace: 'nowrap',
    textDecoration: 'none',
    cursor: disabled ? 'not-allowed' : 'pointer',
    // 0.6 and no lower. Below it a disabled ghost label falls under the
    // placeholder floor and dissolves into the plane; at 0.6 it reads at
    // roughly --ink-faint.
    opacity: disabled ? 0.6 : 1,
  }

  const skin: Record<ButtonVariant, CSSProperties> = {
    ghost: {
      background: 'var(--btn-bg)',
      border: '1px solid var(--rule-ctrl)',
      color: 'var(--ink-3)',
      fontSize: 'var(--fs-sm)',
      fontWeight: 500,
    },
    primary: {
      background: 'var(--signal)',
      border: '1px solid var(--signal)',
      color: 'var(--signal-ink)',
      fontSize: 'var(--fs-body)',
      fontWeight: 600,
    },
    danger: {
      background: 'var(--danger-wash)',
      border: '1px solid var(--danger-bd)',
      color: 'var(--danger)',
      fontSize: 'var(--fs-sm)',
      fontWeight: 500,
    },
  }

  // The glyph tracks its label rather than sitting a step dimmer: a toolbar
  // glyph is something the user acts on, so on a ghost control it holds
  // --ink-4 instead of fading toward the plane.
  const iconColor =
    primary ? 'var(--signal-ink)'
      : variant === 'danger' ? 'var(--danger)'
        : 'var(--ink-4)'

  const inner = (
    <>
      {running
        ? <MIcon name="progress_activity" className="m-spin" size={15} style={{ color: iconColor }} />
        : icon && <MIcon name={icon} size={15} style={{ color: iconColor }} />}
      {!compact && <span>{children ?? label}</span>}
    </>
  )

  const className = `btn btn-${variant}`
  const style = { ...base, ...skin[variant] }
  const resolvedTitle = title ?? (compact ? label : undefined)

  if (href) {
    return (
      <a
        className={className}
        href={href}
        target="_blank"
        rel="noreferrer"
        style={style}
        title={resolvedTitle}
        aria-label={compact ? label : undefined}
      >
        {inner}
      </a>
    )
  }

  return (
    <button
      type="button"
      className={className}
      onClick={onClick}
      disabled={disabled}
      style={style}
      title={resolvedTitle}
      aria-label={compact ? label : undefined}
    >
      {inner}
    </button>
  )
}

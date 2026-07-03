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
  running?: boolean
  compact?: boolean
  title?: string
  children?: ReactNode
}

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

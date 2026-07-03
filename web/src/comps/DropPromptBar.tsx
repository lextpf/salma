// show this inert status only while intake is blocked.

import MIcon from './MIcon'

export type DropPromptVariant = 'installing' | 'locked' | 'unavailable'

interface DropPromptBarProps {
  variant: DropPromptVariant
  compact?: boolean
}

interface VariantConfig {
  border: string
  background: string
  color: string
  glyph: string
  icon: string
  spin: boolean
  label: string
  short: string
}

const CONFIG: Record<DropPromptVariant, VariantConfig> = {
  installing: {
    border: 'var(--rule-ctrl)',
    background: 'transparent',
    color: 'var(--ink-5)',
    glyph: 'var(--ink-5)',
    icon: 'settings',
    spin: true,
    label: 'install in progress - intake reopens when done',
    short: 'installing',
  },
  locked: {
    border: 'var(--danger-bd)',
    background: 'var(--danger-wash)',
    color: 'var(--danger)',
    glyph: 'var(--danger)',
    icon: 'lock',
    spin: false,
    label: 'intake locked - plugin not deployed',
    short: 'locked',
  },
  unavailable: {
    border: 'var(--rule-ctrl)',
    background: 'transparent',
    color: 'var(--ink-5)',
    glyph: 'var(--ink-5)',
    icon: 'sync',
    spin: true,
    label: 'connecting to mo2-server - intake offline',
    short: 'offline',
  },
}

export default function DropPromptBar({ variant, compact = false }: DropPromptBarProps) {
  const cfg = CONFIG[variant]

  return (
    <div
      role="status"
      aria-label={cfg.label}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 9,
        flexShrink: 0,
        minWidth: 0,
        maxWidth: 420,
        height: 30,
        padding: '0 12px',
        border: `1px solid ${cfg.border}`,
        borderRadius: 'var(--radius-ctrl)',
        background: cfg.background,
        fontFamily: 'var(--font-mono)',
        fontSize: 'var(--fs-label)',
        color: cfg.color,
      }}
    >
      <span aria-hidden="true" style={{ color: cfg.glyph, display: 'inline-flex', flexShrink: 0 }}>
        {cfg.spin
          ? <MIcon name="progress_activity" className="m-spin" size={15} />
          : <MIcon name={cfg.icon} size={15} />}
      </span>
      <span style={{ minWidth: 0, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {compact ? cfg.short : cfg.label}
      </span>
    </div>
  )
}

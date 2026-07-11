// The dashed intake prompt that lives in the page header. It reflects intake
// state (idle / dragging / installing / locked / unavailable) but the actual
// drop target is the feed; the browse button opens the file picker.

import MIcon from '../MIcon'

export type DropPromptVariant = 'idle' | 'drag' | 'installing' | 'locked' | 'unavailable'

interface DropPromptBarProps {
  variant: DropPromptVariant
  onBrowse: () => void
}

interface VariantConfig {
  borderColor: string
  background: string
  color: string
  tileColor: string
  tileBg: string
  icon: string
  spin: boolean
  label: string
  browseBg: string
  browseColor: string
  browseDim: boolean
}

const CONFIG: Record<DropPromptVariant, VariantConfig> = {
  idle: {
    borderColor: 'var(--rule-strong)',
    background: 'transparent',
    color: 'var(--ink-4)',
    tileColor: 'var(--ink)',
    tileBg: 'color-mix(in srgb, var(--ink) 6%, transparent)',
    icon: 'upload',
    spin: false,
    label: 'drop archive anywhere on the feed',
    browseBg: 'var(--ink)',
    browseColor: 'var(--sheet)',
    browseDim: false,
  },
  drag: {
    borderColor: 'var(--ink)',
    background: 'transparent',
    color: 'var(--ink-3)',
    tileColor: 'var(--sheet)',
    tileBg: 'var(--ink)',
    icon: 'arrow_downward',
    spin: false,
    label: 'release to queue',
    browseBg: 'var(--ink)',
    browseColor: 'var(--sheet)',
    browseDim: true,
  },
  installing: {
    borderColor: 'var(--rule-strong)',
    background: 'transparent',
    color: 'var(--ink-4)',
    tileColor: 'var(--ink-4)',
    tileBg: 'color-mix(in srgb, var(--ink) 6%, transparent)',
    icon: 'settings',
    spin: true,
    label: 'install in progress - intake reopens when done',
    browseBg: 'var(--meter-empty)',
    browseColor: 'var(--ink-5)',
    browseDim: true,
  },
  locked: {
    borderColor: 'color-mix(in srgb, var(--danger) 35%, transparent)',
    background: 'color-mix(in srgb, var(--danger) 3%, transparent)',
    color: 'var(--tier-low-fg)',
    tileColor: 'var(--danger)',
    tileBg: 'color-mix(in srgb, var(--danger) 12%, transparent)',
    icon: 'lock',
    spin: false,
    label: 'intake locked - plugin not deployed',
    browseBg: 'var(--meter-empty)',
    browseColor: 'var(--ink-5)',
    browseDim: true,
  },
  unavailable: {
    borderColor: 'var(--rule-strong)',
    background: 'transparent',
    color: 'var(--ink-4)',
    tileColor: 'var(--ink-4)',
    tileBg: 'color-mix(in srgb, var(--ink) 6%, transparent)',
    icon: 'sync',
    spin: true,
    label: 'connecting to mo2-server - intake offline',
    browseBg: 'var(--meter-empty)',
    browseColor: 'var(--ink-5)',
    browseDim: true,
  },
}

export default function DropPromptBar({ variant, onBrowse }: DropPromptBarProps) {
  const cfg = CONFIG[variant]
  const interactive = variant === 'idle' || variant === 'drag'

  const barStyle: React.CSSProperties = {
    display: 'flex',
    alignItems: 'center',
    gap: 9,
    flex: 1,
    maxWidth: 500,
    height: 32,
    marginLeft: 8,
    padding: '0 6px 0 5px',
    border: `1.5px dashed ${cfg.borderColor}`,
    borderRadius: 8,
    background: cfg.background,
    fontFamily: 'var(--font-mono)',
    fontSize: 'var(--fs-micro)',
    color: cfg.color,
    textAlign: 'left',
    cursor: interactive ? 'pointer' : 'not-allowed',
  }

  const content = (
    <>
      <span
        aria-hidden="true"
        style={{
          width: 22,
          height: 22,
          flexShrink: 0,
          display: 'inline-flex',
          alignItems: 'center',
          justifyContent: 'center',
          borderRadius: 5,
          color: cfg.tileColor,
          background: cfg.tileBg,
        }}
      >
        {cfg.spin ? (
          <MIcon name="progress_activity" className="m-spin" size={13} />
        ) : (
          <MIcon name={cfg.icon} size={13} />
        )}
      </span>
      <span style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{cfg.label}</span>
      <span style={{ flex: 1 }} />
      <span
        aria-hidden="true"
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 6,
          padding: '4px 10px',
          borderRadius: 5,
          background: cfg.browseBg,
          color: cfg.browseColor,
          fontSize: 'var(--fs-micro)',
          fontWeight: 600,
          opacity: cfg.browseDim ? 0.4 : 1,
        }}
      >
        <MIcon name="folder_open" size={13} />
        browse
      </span>
    </>
  )

  if (interactive) {
    return (
      <button type="button" onClick={onBrowse} aria-label="Choose archive to install" style={barStyle}>
        {content}
      </button>
    )
  }

  return (
    <div role="status" aria-label={cfg.label} style={barStyle}>
      {content}
    </div>
  )
}

import MIcon from '../MIcon'
import { useTheme } from '../../theme'
import type { EngineState } from '../../useChromeStatus'

const ENGINE_LABEL: Record<EngineState, string> = {
  idle: 'idle',
  scanning: 'scanning',
  testing: 'testing',
  installing: 'installing',
}

interface TopBarProps {
  engineState: EngineState
  profile: string
}

// The 52px top bar: layered-square mark + wordmark, a static profile chip
// (derived from the configured mods path), and the live engine status LED. The
// dark/light toggle lives here.
export default function TopBar({ engineState, profile }: TopBarProps) {
  const { theme, toggleTheme } = useTheme()
  const running = engineState !== 'idle'

  const iconBtn: React.CSSProperties = {
    width: 32,
    height: 32,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    border: '1px solid var(--rule)',
    borderRadius: 6,
    background: 'var(--sheet)',
    color: 'var(--ink-3)',
    cursor: 'pointer',
    textDecoration: 'none',
  }

  return (
    <header
      style={{
        height: 52,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 15,
        padding: '0 18px',
        background: 'var(--sheet)',
        borderBottom: '1px solid var(--rule-soft)',
        boxShadow: 'var(--shadow-elevation-1)',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 11 }}>
        <span
          aria-hidden="true"
          style={{ position: 'relative', display: 'inline-block', width: 16, height: 16 }}
        >
          <span
            style={{
              position: 'absolute',
              left: 0,
              top: 0,
              width: 10,
              height: 10,
              border: '1.5px solid var(--ink)',
            }}
          />
          <span
            style={{ position: 'absolute', left: 5, top: 5, width: 10, height: 10, background: 'var(--ink)' }}
          />
        </span>
        <span style={{ fontSize: 'var(--fs-head)', fontWeight: 700, letterSpacing: '-0.025em', color: 'var(--ink)' }}>
          salma
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            textTransform: 'uppercase',
            letterSpacing: '0.2em',
            color: 'var(--ink-5)',
          }}
        >
          FOMOD engine
        </span>
      </div>

      <span aria-hidden="true" style={{ width: 1, height: 18, background: 'var(--rule-soft)' }} />

      <span
        title="Single MO2 instance (multi-instance support coming later)"
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 9,
          padding: '5px 11px',
          border: '1px solid var(--rule)',
          borderRadius: 6,
        }}
      >
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            textTransform: 'uppercase',
            letterSpacing: '0.08em',
            color: 'var(--ink-4)',
          }}
        >
          Profile
        </span>
        <span style={{ fontSize: 'var(--fs-body)', fontWeight: 500, color: 'var(--ink)' }}>{profile}</span>
      </span>

      <div style={{ flex: 1 }} />

      <span
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 8,
          padding: '5px 12px',
          border: '1px solid var(--rule)',
          borderRadius: 6,
        }}
      >
        <span
          aria-hidden="true"
          style={{
            width: 6,
            height: 6,
            borderRadius: '50%',
            background: running ? 'var(--ink)' : 'var(--ink-5)',
            animation: running ? 'salma-blink 1.4s ease-in-out infinite' : 'none',
          }}
        />
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            textTransform: 'uppercase',
            letterSpacing: '0.1em',
            color: 'var(--ink-4)',
          }}
        >
          Engine
        </span>
        <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink)' }}>
          {ENGINE_LABEL[engineState]}
        </span>
      </span>

      <button
        type="button"
        onClick={toggleTheme}
        aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`}
        style={iconBtn}
      >
        <MIcon name={theme === 'dark' ? 'dark_mode' : 'light_mode'} size={15} />
      </button>

      <a
        href="https://github.com/lextpf/salma"
        target="_blank"
        rel="noreferrer"
        aria-label="Help and documentation"
        style={iconBtn}
      >
        <MIcon name="help" size={15} />
      </a>
    </header>
  )
}

import { memo } from 'react'
import MIcon from './MIcon'
import { VRule } from './Rule'
import { useTheme } from '../theme'
import type { EngineState } from '../useChromeStatus'

const ENGINE_LABEL: Record<EngineState, string> = {
  idle: 'idle',
  scanning: 'scanning',
  testing: 'testing',
  installing: 'installing',
}

interface TopBarProps {
  engineState: EngineState
  instance: string
}

/**
 * The 56px top bar: layered-square mark and wordmark, the MO2 instance chip,
 * and the live engine LED.
 *
 * No fill and no rule of its own, so it sits on the same plane as the content
 * below it. The engine LED is a flat filled dot that blinks while the engine is
 * busy; there is no gradient, shadow or glow anywhere in the bar.
 *
 * "Instance" rather than "profile" is Mod Organizer 2's own word for what this
 * value names, and salma borrows MO2's vocabulary.
 *
 * The filled square in the mark is the only branded use of the accent in the
 * app. Everywhere else, signal means live, selected or primary.
 *
 * Memoized, because Layout re-renders every second for the status-bar clock and
 * nothing here changes at that rate.
 */
function TopBar({ engineState, instance }: TopBarProps) {
  const { theme, toggleTheme } = useTheme()
  const running = engineState !== 'idle'

  const iconBtn: React.CSSProperties = {
    width: 30,
    height: 30,
    flexShrink: 0,
    display: 'flex',
    alignItems: 'center',
    justifyContent: 'center',
    border: '1px solid var(--rule-ctrl)',
    borderRadius: 'var(--radius-ctrl)',
    background: 'var(--btn-bg)',
    color: 'var(--ink-4)',
    cursor: 'pointer',
    textDecoration: 'none',
  }

  return (
    <header
      style={{
        height: 56,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 15,
        padding: '0 18px',
        position: 'relative',
        zIndex: 3,
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 11, flexShrink: 0 }}>
        <span
          aria-hidden="true"
          style={{ position: 'relative', display: 'inline-block', width: 18, height: 18 }}
        >
          <span
            style={{
              position: 'absolute',
              left: 0,
              top: 0,
              width: 12,
              height: 12,
              border: '1.5px solid var(--ink-6)',
            }}
          />
          <span
            style={{
              position: 'absolute',
              left: 6,
              top: 6,
              width: 12,
              height: 12,
              background: 'var(--signal)',
            }}
          />
        </span>
        <span
          style={{
            fontSize: 'var(--fs-lede)',
            fontWeight: 800,
            letterSpacing: 'var(--tr-hero)',
            color: 'var(--ink)',
          }}
        >
          salma
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            fontWeight: 500,
            textTransform: 'uppercase',
            letterSpacing: 'var(--tr-lockup)',
            color: 'var(--ink-6)',
            lineHeight: 1.25,
            maxWidth: 58,
          }}
        >
          FOMOD engine
        </span>
      </div>

      <VRule />

      <span
        title="MO2 instance (multi-instance support comes later)"
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 9,
          height: 30,
          minWidth: 0,
          padding: '0 11px',
          border: '1px solid var(--rule-ctrl)',
          borderRadius: 'var(--radius-ctrl)',
          background: 'var(--chip-quiet)',
        }}
      >
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            textTransform: 'uppercase',
            letterSpacing: 'var(--tr-chip)',
            color: 'var(--ink-5)',
            flexShrink: 0,
          }}
        >
          Instance
        </span>
        <span
          style={{
            fontSize: 'var(--fs-body)',
            fontWeight: 600,
            color: 'var(--ink)',
            minWidth: 0,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
          }}
        >
          {instance}
        </span>
      </span>

      <div style={{ flex: 1 }} />

      {/* The engine readout. It only spends accent while the engine is actually
          doing something: an idle engine is the resting state of the app, and
          accenting it burns the signal budget on "nothing is happening". The
          dot, the border and the value all switch together, so the chip reads
          as live or quiet at a glance rather than always looking live. */}
      <span
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          gap: 8,
          height: 30,
          flexShrink: 0,
          padding: '0 12px',
          border: `1px solid ${running ? 'var(--signal-bd)' : 'var(--rule-ctrl)'}`,
          borderRadius: 'var(--radius-ctrl)',
          background: running ? 'var(--signal-wash-chip)' : 'transparent',
        }}
      >
        <span
          aria-hidden="true"
          className={running ? 'blink' : undefined}
          style={{
            width: 6,
            height: 6,
            borderRadius: 'var(--radius-full)',
            background: running ? 'var(--signal)' : 'var(--ink-5)',
          }}
        />
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            fontWeight: 600,
            textTransform: 'uppercase',
            letterSpacing: 'var(--tr-chip)',
            color: 'var(--ink-5)',
          }}
        >
          Engine
        </span>
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-micro)',
            fontWeight: 600,
            color: running ? 'var(--signal-2)' : 'var(--ink-3)',
          }}
        >
          {ENGINE_LABEL[engineState]}
        </span>
      </span>

      <button
        type="button"
        onClick={toggleTheme}
        aria-label={`Switch to ${theme === 'dark' ? 'light' : 'dark'} mode`}
        className="btn"
        style={iconBtn}
      >
        <MIcon name={theme === 'dark' ? 'dark_mode' : 'light_mode'} size={15} />
      </button>

      <a
        href="https://github.com/lextpf/salma"
        target="_blank"
        rel="noreferrer"
        aria-label="Help and documentation"
        className="btn"
        style={iconBtn}
      >
        <MIcon name="help" size={15} />
      </a>
    </header>
  )
}

export default memo(TopBar)

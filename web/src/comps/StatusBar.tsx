import { memo } from 'react'
import { VRule } from './Rule'

interface StatusBarProps {
  mo2On: boolean
  serverOn: boolean
  dllLoaded: boolean
  pluginPurged: boolean
  backendUp: boolean | null
  instance: string
  clock: string
}

interface IndicatorProps {
  label: string
  value: string
  tone: 'on' | 'off' | 'error'
}

function Indicator({ label, value, tone }: IndicatorProps) {
  const dot = tone === 'on' ? 'var(--moss)' : tone === 'error' ? 'var(--danger)' : 'var(--ink-5)'

  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 7 }}>
      <span
        aria-hidden="true"
        style={{ width: 6, height: 6, borderRadius: 'var(--radius-full)', background: dot }}
      />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-meta)',
          textTransform: 'uppercase',
          letterSpacing: 'var(--tr-chip)',
          color: 'var(--ink-5)',
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-meta)',
          color: tone === 'error' ? 'var(--danger)' : 'var(--ink-3)',
        }}
      >
        {value}
      </span>
    </span>
  )
}

function StatusBar({
  mo2On,
  serverOn,
  dllLoaded,
  pluginPurged,
  backendUp,
  instance,
  clock,
}: StatusBarProps) {
  const mo2Value = mo2On ? 'connected' : pluginPurged ? 'purged' : backendUp === false ? 'offline' : 'checking'
  const dllValue = dllLoaded ? 'loaded' : pluginPurged ? 'purged' : 'missing'

  return (
    <footer
      style={{
        height: 32,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '0 18px',
        position: 'relative',
        zIndex: 3,
      }}
    >
      <Indicator label="MO2" value={mo2Value} tone={mo2On ? 'on' : pluginPurged ? 'error' : 'off'} />
      <VRule height={13} />
      <Indicator label="SERVER" value={serverOn ? ':5000' : 'down'} tone={serverOn ? 'on' : 'off'} />
      <VRule height={13} />
      <Indicator label="DLL" value={dllValue} tone={dllLoaded ? 'on' : pluginPurged ? 'error' : 'off'} />

      <div style={{ flex: 1 }} />

      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-meta)',
          color: 'var(--ink-5)',
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {instance}
      </span>
      <VRule height={13} />
      <span
        className="tabular-nums"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-meta)',
          color: 'var(--ink-4)',
        }}
      >
        {clock}
      </span>
    </footer>
  )
}

export default memo(StatusBar)

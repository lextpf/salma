interface StatusBarProps {
  mo2On: boolean
  serverOn: boolean
  dllLoaded: boolean
  pluginPurged: boolean
  backendUp: boolean | null
  profile: string
  clock: string
}

interface IndicatorProps {
  label: string
  value: string
  tone: 'on' | 'off' | 'error'
}

function Indicator({ label, value, tone }: IndicatorProps) {
  const dot = tone === 'on' ? 'var(--ink)' : tone === 'error' ? 'var(--danger)' : 'var(--ink-5)'
  return (
    <span style={{ display: 'inline-flex', alignItems: 'center', gap: 7 }}>
      <span aria-hidden="true" style={{ width: 6, height: 6, borderRadius: '50%', background: dot }} />
      <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-4)' }}>{label}</span>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          color: tone === 'error' ? 'var(--danger)' : 'var(--ink)',
        }}
      >
        {value}
      </span>
    </span>
  )
}

function Divider() {
  return <span aria-hidden="true" style={{ width: 1, height: 13, background: 'var(--rule-soft)' }} />
}

// The 30px bottom status bar: connection LEDs for MO2 / server / DLL, the active
// profile, and a clock.
export default function StatusBar({
  mo2On,
  serverOn,
  dllLoaded,
  pluginPurged,
  backendUp,
  profile,
  clock,
}: StatusBarProps) {
  const mo2Value = mo2On ? 'connected' : pluginPurged ? 'purged' : backendUp === false ? 'offline' : 'checking'
  const dllValue = dllLoaded ? 'loaded' : pluginPurged ? 'purged' : 'missing'

  return (
    <footer
      style={{
        height: 30,
        flexShrink: 0,
        display: 'flex',
        alignItems: 'center',
        gap: 14,
        padding: '0 18px',
        background: 'var(--sheet)',
        borderTop: '1px solid var(--rule-soft)',
        boxShadow: 'var(--shadow-elevation-1)',
      }}
    >
      <Indicator label="MO2" value={mo2Value} tone={mo2On ? 'on' : pluginPurged ? 'error' : 'off'} />
      <Divider />
      <Indicator label="SERVER" value={serverOn ? ':5000' : 'down'} tone={serverOn ? 'on' : 'off'} />
      <Divider />
      <Indicator label="DLL" value={dllValue} tone={dllLoaded ? 'on' : pluginPurged ? 'error' : 'off'} />

      <div style={{ flex: 1 }} />

      <span style={{ fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: 'var(--ink-4)', padding: '0 14px' }}>
        {profile}
      </span>
      <span
        className="tabular-nums"
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 'var(--fs-micro)',
          color: 'var(--ink-3)',
          paddingLeft: 14,
          borderLeft: '1px solid var(--rule-soft)',
        }}
      >
        {clock}
      </span>
    </footer>
  )
}

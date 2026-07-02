import { useCallback, useState } from 'react'
import { Outlet, useMatch } from 'react-router-dom'
import ModuleRail from './ModuleRail'
import StatusBar from './StatusBar'
import TopBar from './TopBar'
import { deriveProfile, useChromeStatus } from '../useChromeStatus'
import { getRailCollapsed, setRailCollapsed } from '../prefs'

function pad(n: number) {
  return String(n).padStart(2, '0')
}

function formatClock(d: Date) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

export default function Layout() {
  const chrome = useChromeStatus()
  const instance = deriveProfile(chrome.config?.mo2ModsPath)
  const inferred = chrome.status?.jsonCount ?? 0
  const mods = chrome.status?.modCount ?? 0

  // update during render so record routes never show one frame at the expanded width.
  const onRecord = useMatch('/fomods/:name') !== null
  const [collapsed, setCollapsed] = useState(() => getRailCollapsed() || onRecord)
  const [wasOnRecord, setWasOnRecord] = useState(onRecord)

  if (onRecord !== wasOnRecord) {
    setWasOnRecord(onRecord)
    if (onRecord) setCollapsed(true)
  }

  const toggleRail = useCallback(() => {
    setCollapsed(prev => {
      const next = !prev
      setRailCollapsed(next)
      return next
    })
  }, [])

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100vh',
        overflow: 'hidden',
        background: 'var(--void)',
      }}
    >
      <TopBar engineState={chrome.engineState} instance={instance} />

      <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
        <ModuleRail
          inferredCount={inferred}
          modCount={mods}
          partialCount={chrome.rollup?.partial ?? null}
          modsPath={chrome.config?.mo2ModsPath}
          collapsed={collapsed}
          onToggle={toggleRail}
        />
        <main
          style={{
            flex: 1,
            minWidth: 0,
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
            background: 'var(--paper)',
          }}
        >
          <Outlet />
        </main>
      </div>

      <StatusBar
        mo2On={chrome.mo2On}
        serverOn={chrome.serverOn}
        dllLoaded={chrome.dllLoaded}
        pluginPurged={chrome.pluginPurged}
        backendUp={chrome.backendUp}
        instance={instance}
        clock={formatClock(chrome.now)}
      />
    </div>
  )
}

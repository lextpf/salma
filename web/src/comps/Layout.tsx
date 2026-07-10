import { Outlet } from 'react-router-dom'
import ModuleRail from './chrome/ModuleRail'
import StatusBar from './chrome/StatusBar'
import TopBar from './chrome/TopBar'
import { deriveProfile, useChromeStatus } from '../useChromeStatus'

function pad(n: number) {
  return String(n).padStart(2, '0')
}

function formatClock(d: Date) {
  return `${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`
}

// The v5 frame: a fixed top bar and bottom status bar sandwich a body row of
// [module rail | white content sheet]. Pages render their own 46px header inside
// <main>; this shell only owns the chrome and the single status poller.
export default function Layout() {
  const chrome = useChromeStatus()
  const profile = deriveProfile(chrome.config?.mo2ModsPath)
  const inferred = chrome.status?.jsonCount ?? 0

  return (
    <div
      style={{
        display: 'flex',
        flexDirection: 'column',
        height: '100vh',
        overflow: 'hidden',
        background: 'var(--paper)',
      }}
    >
      <TopBar engineState={chrome.engineState} profile={profile} />

      <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
        <ModuleRail inferredCount={inferred} modsPath={chrome.config?.mo2ModsPath} />
        <main
          style={{
            flex: 1,
            minWidth: 0,
            display: 'flex',
            flexDirection: 'column',
            overflow: 'hidden',
            background: 'var(--sheet)',
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
        profile={profile}
        clock={formatClock(chrome.now)}
      />
    </div>
  )
}

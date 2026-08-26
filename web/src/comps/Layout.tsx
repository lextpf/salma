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

// The frame: a fixed top bar and bottom status bar sandwich a body row of
// [module rail | content plane]. Pages render their own 52px ModuleHeader
// inside <main>; this shell only owns the chrome and the single status poller.
export default function Layout() {
  const chrome = useChromeStatus()
  const instance = deriveProfile(chrome.config?.mo2ModsPath)
  const inferred = chrome.status?.jsonCount ?? 0
  const mods = chrome.status?.modCount ?? 0

  // The rail folds to a 62px icon spine. The user's choice persists and wins,
  // but opening a Library record folds it once on their behalf: that route puts
  // three panes side by side and the rail is the cheapest 154px on screen.
  // Folding it here rather than in LibraryPage keeps the rail's width owned by
  // the frame that draws it.
  //
  // Adjusted during render off the previous route, not in an effect. This is
  // state reacting to other state, not synchronisation with anything outside
  // React, so an effect would only add a second render pass and one frame at
  // the wrong width. The initial value folds too, so landing straight on a
  // record URL gets the same layout as clicking through to one.
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

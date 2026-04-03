import { useEffect, useRef, useState } from 'react'
import { getConfig, getFomodScanStatus, getInstallStatus, getMo2Status, getTestStatus } from './api'
import type { AppConfig, Mo2Status } from './types'

export type EngineState = 'idle' | 'scanning' | 'testing' | 'installing'

export interface ChromeStatus {
  status: Mo2Status | null
  config: AppConfig | null
  backendUp: boolean | null
  engineState: EngineState
  now: Date
  mo2On: boolean
  serverOn: boolean
  dllLoaded: boolean
  pluginPurged: boolean
}

// Consolidates the chrome's background polling (status, scan/test/install
// activity, config) plus a 1s clock, and derives the engine state + the
// connection booleans the TopBar / StatusBar / ModuleRail render. One poller for
// the whole frame; pages keep their own hooks for their own data.
export function useChromeStatus(): ChromeStatus {
  const [status, setStatus] = useState<Mo2Status | null>(null)
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [backendUp, setBackendUp] = useState<boolean | null>(null)
  const [scanRunning, setScanRunning] = useState(false)
  const [testRunning, setTestRunning] = useState(false)
  const [installRunning, setInstallRunning] = useState(false)
  const [now, setNow] = useState(() => new Date())
  const inFlightRef = useRef(false)

  useEffect(() => {
    const check = () => {
      if (inFlightRef.current) {
        return
      }
      inFlightRef.current = true
      Promise.all([
        getMo2Status().catch(() => null),
        getFomodScanStatus().catch(() => null),
        getTestStatus().catch(() => null),
        getInstallStatus().catch(() => null),
        getConfig().catch(() => null),
      ])
        .then(([s, scan, test, install, cfg]) => {
          if (s) {
            setStatus(s)
            setBackendUp(true)
          } else {
            setBackendUp(false)
          }
          if (cfg) {
            setConfig(cfg)
          }
          setScanRunning(scan?.running === true)
          setTestRunning(test?.running === true)
          setInstallRunning(install?.running === true)
        })
        .finally(() => {
          inFlightRef.current = false
        })
    }
    check()
    const id = setInterval(check, 8000)
    return () => { clearInterval(id); }
  }, [])

  useEffect(() => {
    const id = setInterval(() => { setNow(new Date()); }, 1000)
    return () => clearInterval(id)
  }, [])

  const engineState: EngineState = installRunning
    ? 'installing'
    : scanRunning
      ? 'scanning'
      : testRunning
        ? 'testing'
        : 'idle'

  return {
    status,
    config,
    backendUp,
    engineState,
    now,
    mo2On: backendUp === true && status?.configured === true && status.pluginInstalled === true,
    serverOn: backendUp === true,
    dllLoaded: status?.pluginInstalled === true,
    pluginPurged: backendUp === true && status?.pluginInstalled === false,
  }
}

// Derive a human instance label from the MO2 mods path (the segment above
// "mods"). Used for the static PROFILE chip until multi-instance support lands.
export function deriveProfile(modsPath?: string): string {
  if (!modsPath) {
    return 'No instance'
  }
  const parts = modsPath.replace(/[\\/]+$/, '').split(/[\\/]+/).filter(Boolean)
  if (parts.length === 0) {
    return 'No instance'
  }
  const last = parts[parts.length - 1]
  if (last.toLowerCase() === 'mods' && parts.length >= 2) {
    return parts[parts.length - 2]
  }
  return last
}

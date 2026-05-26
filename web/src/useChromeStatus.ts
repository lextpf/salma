import { useEffect, useRef, useState } from 'react'
import { getConfig, getFomodScanStatus, getInstallStatus, getMo2Status, getTestStatus, listFomods } from './api'
import { tierFromEntry } from './confidence'
import type { AppConfig, Mo2Status } from './types'

export type EngineState = 'idle' | 'scanning' | 'testing' | 'installing'

/** Library-wide confidence roll-up, derived from the record list. */
export interface LibraryRollup {
  total: number
  /** Records whose inferred selection needs a human look (PARTIAL or LOW). */
  partial: number
  /** Mean composite confidence across records that have a score, 0..1. */
  meanConfidence: number | null
}

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
  rollup: LibraryRollup | null
}

// The chrome's single background poller: MO2 status, scan, test and install
// activity, config and the record list every 8 seconds, plus a 1 second clock.
// It derives the engine state and the connection booleans that TopBar,
// StatusBar and ModuleRail render. Pages keep their own hooks for page data.
//
// Every request is individually caught and turned into null, so one failing
// endpoint degrades its own field instead of blanking the whole frame.
export function useChromeStatus(): ChromeStatus {
  const [status, setStatus] = useState<Mo2Status | null>(null)
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [backendUp, setBackendUp] = useState<boolean | null>(null)
  const [scanRunning, setScanRunning] = useState(false)
  const [testRunning, setTestRunning] = useState(false)
  const [installRunning, setInstallRunning] = useState(false)
  const [now, setNow] = useState(() => new Date())
  const [rollup, setRollup] = useState<LibraryRollup | null>(null)
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
        listFomods().catch(() => null),
      ])
        .then(([s, scan, test, install, cfg, fomods]) => {
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
          // The rail footer and the install idle state both show a library
          // roll-up. Deriving it here costs one fetch for the whole frame
          // instead of one per screen.
          if (fomods) {
            let partial = 0
            let sum = 0
            let scored = 0
            for (const e of fomods) {
              const tier = tierFromEntry(e)
              if (tier.hasData && (tier.tier === 'PARTIAL' || tier.tier === 'LOW')) partial++
              if (tier.hasData) {
                sum += tier.pct
                scored++
              }
            }
            setRollup({
              total: fomods.length,
              partial,
              meanConfidence: scored > 0 ? sum / scored / 100 : null,
            })
          }
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
    rollup,
  }
}

// Derive a human instance label from the MO2 mods path (the segment above
// "mods"). MO2 calls this an instance, and so does the top-bar chip.
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

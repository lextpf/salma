import { useEffect, useRef, useState } from 'react'
import { getConfig, getFomodScanStatus, getInstallStatus, getMo2Status, getTestStatus, listFomods } from './api'
import { tierFromEntry } from './confidence'
import type { AppConfig, Mo2Status } from './types'

export type EngineState = 'idle' | 'scanning' | 'testing' | 'installing'

export interface LibraryRollup {
  total: number
  partial: number
  /**
   * @brief mean composite confidence.
   *
   * values are in [0, 1], or null when no records have confidence data.
   */
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

/**
 * @fn useChromeStatus(): ChromeStatus
 * @brief poll shared chrome state without coupling endpoint failures.
 * @author Alex (https://github.com/lextpf)
 *
 * polls every 8 seconds. the clock updates every second.
 */
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
      void Promise.all([
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
    return () => { clearInterval(id); }
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

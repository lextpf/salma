import { useState, useCallback, useRef, useEffect } from 'react'
import { scanFomods, getFomodScanStatus, isFetchUnavailableError } from '../api'
import { usePolling } from './usePolling'

const RETRY_DELAY_MS = 2000

export interface ScanJobState {
  scanRunning: boolean
  scanError: string | null
  handleScanFomods: () => Promise<void>
}

export function useScanJob(
  pluginInstalled: boolean,
  onComplete: (success: boolean) => void,
): ScanJobState {
  const [scanRunning, setScanRunning] = useState(false)
  const [scanError, setScanError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Initial status check with retry
  useEffect(() => {
    const loadScanStatus = () => {
      getFomodScanStatus()
        .then(s => {
          setScanRunning(Boolean(s.running))
          if (retryTimerRef.current) {
            clearTimeout(retryTimerRef.current)
            retryTimerRef.current = null
          }
        })
        .catch(e => {
          console.warn('[install] failed to get scan status, retrying', e)
          if (!retryTimerRef.current) {
            retryTimerRef.current = setTimeout(() => {
              retryTimerRef.current = null
              loadScanStatus()
            }, RETRY_DELAY_MS)
          }
        })
    }
    loadScanStatus()
    return () => {
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current)
    }
  }, [])

  // Poll while running
  const scanPoller = useCallback(async () => {
    try {
      const s = await getFomodScanStatus()
      if (!s.running) {
        setScanRunning(false)
        onComplete(Boolean(s.success))
        if (!s.success && s.error) {
          setScanError(s.error)
        }
      }
    } catch (e) {
      console.warn('[install] failed to poll scan status; will retry', e)
    }
  }, [onComplete])
  usePolling(scanPoller, 3000, scanRunning)

  const handleScanFomods = useCallback(async () => {
    if (!pluginInstalled) return
    setScanError(null)
    setScanRunning(true)
    try {
      const result = await scanFomods()
      setScanRunning(Boolean(result.running))
      if (!result.running && result.success) {
        onComplete(true)
      }
    } catch (e) {
      setScanRunning(false)
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to start FOMOD scan due to unavailable backend', e)
        return
      }
      setScanError(e instanceof Error ? e.message : 'Failed to scan FOMODs')
    }
  }, [pluginInstalled, onComplete])

  return { scanRunning, scanError, handleScanFomods }
}

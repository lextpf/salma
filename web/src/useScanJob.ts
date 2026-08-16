import { useState, useCallback, useRef, useEffect } from 'react'
import { scanFomods, getFomodScanStatus, isFetchUnavailableError } from './api'
import { usePolling } from './usePolling'

const RETRY_DELAY_MS = 2000

export interface ScanJobState {
  scanRunning: boolean
  scanError: string | null
  handleScanFomods: () => Promise<void>
}

/**
 * @fn useScanJob(pluginInstalled: boolean, onComplete: (success: boolean) => void): ScanJobState
 * @brief Resume observation of a shared scan and allow a new scan to start.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Initial status retrieval retries every two seconds on any failure.
 * A running scan is polled every three seconds. Poll failures keep observation active.
 *
 * @param pluginInstalled Whether the start handler may submit a scan.
 * @param onComplete Called when polling sees a stopped scan, or a start completes successfully.
 * @return Scan activity, the last reported error, and a start handler.
 */
export function useScanJob(
  pluginInstalled: boolean,
  onComplete: (success: boolean) => void,
): ScanJobState {
  const [scanRunning, setScanRunning] = useState(false)
  const [scanError, setScanError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    /**
     * @fn loadScanStatus(): void
     * @brief Retry initial status retrieval until the backend responds.
     * @author Alex (<https://github.com/lextpf>)
     */
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

  /**
   * @fn scanPoller(): Promise<void>
   * @brief Report completion when the shared scan stops.
   * @author Alex (<https://github.com/lextpf>)
   */
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

  /**
   * @fn handleScanFomods(): Promise<void>
   * @brief Start a scan when the plugin is available.
   * @author Alex (<https://github.com/lextpf>)
   *
   * Availability failures clear local activity without setting the displayed error.
   */
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

import { useState, useCallback, useRef, useEffect } from 'react'
import { getMo2Status, getConfig } from '../api'
import type { Mo2Status, AppConfig } from '../types'

const RETRY_DELAY_MS = 2000

/**
 * Manages system status (MO2 status + app config) with automatic
 * retry on failure. Cleans up timers on unmount.
 *
 * Exposes a stable `refreshRef` so callbacks/timers always call the
 * latest version without stale closures.
 */
export function useSystemStatus() {
  const [status, setStatus] = useState<Mo2Status | null>(null)
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [loading, setLoading] = useState(true)

  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)
  const mountedRef = useRef(true)

  const clearRetryTimer = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current)
      retryTimerRef.current = null
    }
  }, [])

  const refresh = useCallback((showOverlay = true, force = false) => {
    if (inFlightRef.current && !force) return
    if (showOverlay) setLoading(true)
    inFlightRef.current = true

    Promise.all([getMo2Status(), getConfig()])
      .then(([s, c]) => {
        if (!mountedRef.current) return
        setStatus(s)
        setConfig(c)
        setUpdatedAt(Date.now())
        setLoading(false)
        clearRetryTimer()
      })
      .catch(e => {
        if (!mountedRef.current) return
        console.warn('[status] failed to load, retrying', e)
        setLoading(true)
        if (!retryTimerRef.current) {
          retryTimerRef.current = setTimeout(() => {
            retryTimerRef.current = null
            // Call through ref to avoid stale closure
            refreshRef.current(true, true)
          }, RETRY_DELAY_MS)
        }
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }, [clearRetryTimer])

  // Always-current ref for use inside timers and external callbacks
  const refreshRef = useRef(refresh)
  useEffect(() => { refreshRef.current = refresh })

  // Initial fetch + cleanup
  useEffect(() => {
    mountedRef.current = true
    refresh(true)
    return () => {
      mountedRef.current = false
      clearRetryTimer()
    }
  }, [refresh, clearRetryTimer])

  return { status, config, updatedAt, loading, refresh, refreshRef }
}

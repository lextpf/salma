import { useState, useCallback, useRef, useEffect } from 'react'
import { getMo2Status, getConfig } from './api'
import type { Mo2Status, AppConfig } from './types'

const RETRY_DELAY_MS = 2000

/**
 * MO2 status and app config, fetched together and retried every
 * RETRY_DELAY_MS until both land. Timers are cleared on unmount, and a result
 * arriving after unmount is dropped.
 *
 * `refreshRef` is a stable ref onto the current `refresh`, for timers and
 * callbacks that would otherwise capture a stale one.
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

  // The fetch on its own. Nothing here touches state synchronously, which is
  // what lets the mount effect below call it directly.
  const runFetch = useCallback((force = false) => {
    if (inFlightRef.current && !force) return
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

  // The same fetch plus the loading overlay, for user-triggered refreshes.
  const refresh = useCallback((showOverlay = true, force = false) => {
    if (inFlightRef.current && !force) return
    if (showOverlay) setLoading(true)
    runFetch(force)
  }, [runFetch])

  // Kept current on every render, so timers and external callbacks reach the
  // live `refresh` without re-subscribing to it.
  const refreshRef = useRef(refresh)
  // eslint-disable-next-line react-hooks/immutability
  useEffect(() => { refreshRef.current = refresh })

  // Initial fetch and cleanup. Goes through runFetch, not refresh: `loading`
  // already starts true, so raising the overlay here would be a synchronous
  // setState in an effect body for a value that is already correct.
  useEffect(() => {
    mountedRef.current = true
    runFetch()
    return () => {
      mountedRef.current = false
      clearRetryTimer()
    }
  }, [runFetch, clearRetryTimer])

  return { status, config, updatedAt, loading, refresh, refreshRef }
}

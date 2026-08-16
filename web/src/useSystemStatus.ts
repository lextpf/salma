import { useState, useCallback, useRef, useEffect } from 'react'
import { getMo2Status, getConfig } from './api'
import type { Mo2Status, AppConfig } from './types'

const RETRY_DELAY_MS = 2000

/**
 * @fn useSystemStatus()
 * @brief Load configuration and MO2 status as one displayed snapshot.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Both requests must succeed before either displayed value changes.
 * Failures retain prior values and retry after two seconds without an attempt limit.
 * A forced refresh can overlap an earlier request; completion order determines the snapshot.
 *
 * @return The snapshot, its epoch-millisecond update time, loading state, and refresh callbacks.
 */
export function useSystemStatus() {
  const [status, setStatus] = useState<Mo2Status | null>(null)
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [updatedAt, setUpdatedAt] = useState<number | null>(null)
  const [loading, setLoading] = useState(true)

  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const inFlightRef = useRef(false)
  const mountedRef = useRef(true)

  /**
   * @fn clearRetryTimer(): void
   * @brief Remove the pending retry after success or unmount.
   * @author Alex (<https://github.com/lextpf>)
   */
  const clearRetryTimer = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current)
      retryTimerRef.current = null
    }
  }, [])

  /**
   * @fn runFetch(force = false): void
   * @brief Fetch both snapshot inputs and retry failures together.
   * @author Alex (<https://github.com/lextpf>)
   *
   * @param force Allow another request pair while a pair is in flight.
   */
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
            // Use the current callback without resubscribing.
            refreshRef.current(true, true)
          }, RETRY_DELAY_MS)
        }
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }, [clearRetryTimer])

  /**
   * @fn refresh(showOverlay = true, force = false): void
   * @brief Request a new snapshot with optional loading feedback.
   * @author Alex (<https://github.com/lextpf>)
   *
   * @param showOverlay Set loading before starting the request pair.
   * @param force Allow refresh while a request pair is in flight.
   */
  const refresh = useCallback((showOverlay = true, force = false) => {
    if (inFlightRef.current && !force) return
    if (showOverlay) setLoading(true)
    runFetch(force)
  }, [runFetch])

  // Keep timers on the current callback without resubscribing.
  const refreshRef = useRef(refresh)
  // eslint-disable-next-line react-hooks/immutability
  useEffect(() => { refreshRef.current = refresh })

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

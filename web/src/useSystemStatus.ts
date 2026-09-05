import { useState, useCallback, useRef, useEffect } from 'react'
import { getMo2Status, getConfig } from './api'
import type { Mo2Status, AppConfig } from './types'

const RETRY_DELAY_MS = 2000

export function useSystemStatus() {
  // retry status and configuration together until both succeed.
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
            // use the current callback without resubscribing.
            refreshRef.current(true, true)
          }, RETRY_DELAY_MS)
        }
      })
      .finally(() => {
        inFlightRef.current = false
      })
  }, [clearRetryTimer])

  const refresh = useCallback((showOverlay = true, force = false) => {
    if (inFlightRef.current && !force) return
    if (showOverlay) setLoading(true)
    runFetch(force)
  }, [runFetch])

  // keep timers on the current callback without resubscribing.
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

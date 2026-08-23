import { useCallback, useEffect, useRef, useState } from 'react'
import { getFomod } from './api'
import type { FomodDetail } from './types'

const RETRY_DELAY_MS = 2000
const MAX_RETRIES = 3

export interface RecordDetailState {
  detail: FomodDetail | null
  error: string | null
  retry: () => void
}

// Loads one FOMOD record's detail JSON for the selected name. It is lifted to
// LibraryPage so the VFS tree and the inspector share a single fetch. Callers
// derive `loading` as (name && !detail && !error).
//
// The AbortController does not cancel the HTTP request: getFomod takes no
// signal, so the request runs to its own 8 second timeout. The controller only
// marks the result stale, so a response landing after a name change is dropped
// instead of overwriting the new record.
//
// Every failure is retried, not only an unreachable backend, up to MAX_RETRIES
// attempts spaced RETRY_DELAY_MS apart. That multiplies getFomod's 8 second
// budget: the worst wait before an error appears is about MAX_RETRIES * 8s plus
// the delays. A success resets the counter, and so does the exported `retry()`.
export function useRecordDetail(name: string | null): RecordDetailState {
  const [detail, setDetail] = useState<FomodDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const retryCountRef = useRef(0)
  const loadRef = useRef<() => void>(() => {})

  useEffect(() => {
    const clearRetryTimer = () => {
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current)
        retryTimerRef.current = null
      }
    }

    // Cleared through a helper call rather than a direct in-effect setState, so
    // a name switch drops the previous record immediately.
    const reset = () => {
      setDetail(null)
      setError(null)
    }
    reset()
    retryCountRef.current = 0

    if (!name) {
      loadRef.current = () => {}
      return clearRetryTimer
    }

    const abortController = new AbortController()

    const load = () => {
      setError(null)
      getFomod(name)
        .then(d => {
          if (abortController.signal.aborted) return
          setDetail(d)
          retryCountRef.current = 0
          clearRetryTimer()
        })
        .catch(e => {
          if (abortController.signal.aborted) return
          retryCountRef.current += 1
          const msg = e instanceof Error ? e.message : 'Failed to load record'
          if (retryCountRef.current >= MAX_RETRIES) {
            console.warn(`[library] gave up loading "${name}" after ${MAX_RETRIES} attempts`, e)
            setError(msg)
            return
          }
          console.warn(
            `[library] failed to load "${name}", retrying (${retryCountRef.current}/${MAX_RETRIES})`,
            e,
          )
          if (!retryTimerRef.current) {
            retryTimerRef.current = setTimeout(() => {
              retryTimerRef.current = null
              load()
            }, RETRY_DELAY_MS)
          }
        })
    }

    loadRef.current = load
    load()

    return () => {
      abortController.abort()
      clearRetryTimer()
    }
  }, [name])

  const retry = useCallback(() => {
    retryCountRef.current = 0
    setError(null)
    loadRef.current()
  }, [])

  return { detail, error, retry }
}

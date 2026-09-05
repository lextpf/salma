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

/**
 * @fn useRecordDetail(name: string | null): RecordDetailState
 * @brief retry record loading while preventing stale state updates.
 * @author Alex (https://github.com/lextpf)
 *
 * ### :material-refresh: retries
 *
 * all failure types use the same limit of three attempts with a two-second delay between attempts.
 * manual retry resets the attempt count.
 *
 * ### :material-timer-outline: cancellation
 *
 * cleanup clears the retry timer and ignores stale results. it cannot cancel `getFomod`.
 */
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

    // clear the old record before a new name can resolve.
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

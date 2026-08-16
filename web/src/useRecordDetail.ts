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
 * @brief Ignore superseded record names while retrying detail loading.
 * @author Alex (<https://github.com/lextpf>)
 *
 * ### :material-refresh: Retries
 *
 * All failure types use the same limit of three attempts with a two-second delay between attempts.
 * Manual retry resets the attempt count.
 *
 * ### :material-timer-outline: Cancellation
 *
 * Cleanup clears the retry timer and ignores results for a previous name. It cannot cancel
 * getFomod requests already in flight. Manual retry does not cancel an earlier attempt.
 *
 * @param name Unencoded record stem, or null to clear the selected detail.
 * @return The loaded detail, terminal error, and a manual retry callback.
 */
export function useRecordDetail(name: string | null): RecordDetailState {
  const [detail, setDetail] = useState<FomodDetail | null>(null)
  const [error, setError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const retryCountRef = useRef(0)
  const loadRef = useRef<() => void>(() => {})

  useEffect(() => {
    /**
     * @fn clearRetryTimer(): void
     * @brief Remove the pending retry before cleanup or successful completion.
     * @author Alex (<https://github.com/lextpf>)
     */
    const clearRetryTimer = () => {
      if (retryTimerRef.current) {
        clearTimeout(retryTimerRef.current)
        retryTimerRef.current = null
      }
    }

    /**
     * @fn reset(): void
     * @brief Clear the displayed record when the selected name changes.
     * @author Alex (<https://github.com/lextpf>)
     */
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

    /**
     * @fn load(): void
     * @brief Load the selected record and schedule a retry within the attempt limit.
     * @author Alex (<https://github.com/lextpf>)
     */
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

  /**
   * @fn retry(): void
   * @brief Reset the attempt count and request the selected record again.
   * @author Alex (<https://github.com/lextpf>)
   */
  const retry = useCallback(() => {
    retryCountRef.current = 0
    setError(null)
    loadRef.current()
  }, [])

  return { detail, error, retry }
}

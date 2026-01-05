import { useState, useCallback, useRef, useEffect } from 'react'
import { runTests, getTestStatus, isFetchUnavailableError } from '../api'
import { usePolling } from './usePolling'

const RETRY_DELAY_MS = 2000

export interface TestRunnerState {
  testRunning: boolean
  testError: string | null
  handleRunTests: () => Promise<void>
}

export function useTestRunner(pluginInstalled: boolean): TestRunnerState {
  const [testRunning, setTestRunning] = useState(false)
  const [testError, setTestError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  // Initial status check with retry
  useEffect(() => {
    const loadTestStatus = () => {
      getTestStatus()
        .then(s => {
          setTestRunning(s.running)
          if (retryTimerRef.current) {
            clearTimeout(retryTimerRef.current)
            retryTimerRef.current = null
          }
        })
        .catch(e => {
          console.warn('[install] failed to get test status, retrying', e)
          if (!retryTimerRef.current) {
            retryTimerRef.current = setTimeout(() => {
              retryTimerRef.current = null
              loadTestStatus()
            }, RETRY_DELAY_MS)
          }
        })
    }
    loadTestStatus()
    return () => {
      if (retryTimerRef.current) clearTimeout(retryTimerRef.current)
    }
  }, [])

  // Poll while running
  const testPoller = useCallback(async () => {
    try {
      const s = await getTestStatus()
      if (!s.running) setTestRunning(false)
    } catch (e) {
      console.warn('[install] failed to poll test status; will retry', e)
    }
  }, [])
  usePolling(testPoller, 3000, testRunning)

  const handleRunTests = useCallback(async () => {
    if (!pluginInstalled) return
    setTestError(null)
    try {
      const args = localStorage.getItem('salma_test_args') || ''
      await runTests(args)
      setTestRunning(true)
    } catch (e) {
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to start tests due to unavailable backend', e)
        return
      }
      setTestError(e instanceof Error ? e.message : 'Failed to start tests')
    }
  }, [pluginInstalled])

  return { testRunning, testError, handleRunTests }
}

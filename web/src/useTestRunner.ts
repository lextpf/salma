import { useState, useCallback, useRef, useEffect } from 'react'
import { runTests, getTestStatus, isFetchUnavailableError } from './api'
import { usePolling } from './usePolling'

const RETRY_DELAY_MS = 2000

export interface TestRunnerState {
  testRunning: boolean
  testError: string | null
  handleRunTests: () => Promise<void>
}

/**
 * @fn useTestRunner(pluginInstalled: boolean): TestRunnerState
 * @brief Observe the shared harness process and expose its start action.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Initial status retrieval retries every two seconds. Running tests are polled every three.
 * The hook reports start errors; process exit diagnostics remain in the test log.
 *
 * @param pluginInstalled Whether the start handler may launch the harness.
 * @return Process activity, start errors, and a handler that reads saved harness arguments.
 */
export function useTestRunner(pluginInstalled: boolean): TestRunnerState {
  const [testRunning, setTestRunning] = useState(false)
  const [testError, setTestError] = useState<string | null>(null)
  const retryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    /**
     * @fn loadTestStatus(): void
     * @brief Retry initial process status retrieval until the backend responds.
     * @author Alex (<https://github.com/lextpf>)
     */
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

  /**
   * @fn testPoller(): Promise<void>
   * @brief Clear local activity when the harness process stops.
   * @author Alex (<https://github.com/lextpf>)
   */
  const testPoller = useCallback(async () => {
    try {
      const s = await getTestStatus()
      if (!s.running) setTestRunning(false)
    } catch (e) {
      console.warn('[install] failed to poll test status; will retry', e)
    }
  }, [])
  usePolling(testPoller, 3000, testRunning)

  /**
   * @fn handleRunTests(): Promise<void>
   * @brief Launch the harness with the saved raw argument string.
   * @author Alex (<https://github.com/lextpf>)
   *
   * The plugin must be available. Availability failures do not set the displayed error.
   */
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

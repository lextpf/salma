import { useState, useCallback, useRef } from 'react'
import { deployPlugin, purgePlugin, getPluginActionStatus, isFetchUnavailableError } from './api'
import { usePolling } from './usePolling'

export interface PluginActionState {
  pluginActionRunning: null | 'deploy' | 'purge'
  pluginActionError: string | null
  handleDeployPlugin: () => Promise<void>
  handlePurgePlugin: () => Promise<void>
}

/**
 * @fn usePluginAction(onComplete: (success: boolean) => void): PluginActionState
 * @brief Track plugin deployment or removal initiated by this hook.
 * @author Alex (<https://github.com/lextpf>)
 *
 * Poll every 1.5 seconds after an accepted start. An availability failure stops local
 * observation without cancelling the server action. Other poll failures leave polling active.
 *
 * @param onComplete Called with true after a successful observed action.
 * @return Local action state, displayed errors, and deploy and purge handlers.
 */
export function usePluginAction(onComplete: (success: boolean) => void): PluginActionState {
  const [pluginActionRunning, setPluginActionRunning] = useState<null | 'deploy' | 'purge'>(null)
  const [pluginActionError, setPluginActionError] = useState<string | null>(null)
  const [polling, setPolling] = useState(false)
  const actionRef = useRef<null | 'deploy' | 'purge'>(null)

  /**
   * @fn pluginActionPoller(): Promise<void>
   * @brief Settle local action state when shared status reports completion.
   * @author Alex (<https://github.com/lextpf>)
   */
  const pluginActionPoller = useCallback(async () => {
    try {
      const status = await getPluginActionStatus()
      if (!status.running) {
        setPolling(false)
        if (status.success) {
          onComplete(true)
        } else {
          setPluginActionError(status.error || `${actionRef.current} failed`)
        }
        actionRef.current = null
        setPluginActionRunning(null)
      }
    } catch (e) {
      if (isFetchUnavailableError(e)) {
        setPolling(false)
        actionRef.current = null
        setPluginActionRunning(null)
      } else {
        console.error('[install] error polling plugin action status', e)
      }
    }
  }, [onComplete])

  usePolling(pluginActionPoller, 1500, polling)

  /**
   * @fn handleDeployPlugin(): Promise<void>
   * @brief Submit deployment and begin observing its status.
   * @author Alex (<https://github.com/lextpf>)
   */
  const handleDeployPlugin = useCallback(async () => {
    setPluginActionError(null)
    actionRef.current = 'deploy'
    setPluginActionRunning('deploy')
    try {
      await deployPlugin()
      setPolling(true)
    } catch (e) {
      actionRef.current = null
      setPluginActionRunning(null)
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to deploy plugin due to unavailable backend', e)
        return
      }
      setPluginActionError(e instanceof Error ? e.message : 'Failed to deploy plugin')
    }
  }, [])

  /**
   * @fn handlePurgePlugin(): Promise<void>
   * @brief Submit plugin removal and begin observing its status.
   * @author Alex (<https://github.com/lextpf>)
   */
  const handlePurgePlugin = useCallback(async () => {
    setPluginActionError(null)
    actionRef.current = 'purge'
    setPluginActionRunning('purge')
    try {
      await purgePlugin()
      setPolling(true)
    } catch (e) {
      actionRef.current = null
      setPluginActionRunning(null)
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to purge plugin due to unavailable backend', e)
        return
      }
      setPluginActionError(e instanceof Error ? e.message : 'Failed to purge plugin')
    }
  }, [])

  return { pluginActionRunning, pluginActionError, handleDeployPlugin, handlePurgePlugin }
}

import { useState, useCallback, useRef } from 'react'
import { deployPlugin, purgePlugin, getPluginActionStatus, isFetchUnavailableError } from './api'
import { usePolling } from './usePolling'

export interface PluginActionState {
  pluginActionRunning: null | 'deploy' | 'purge'
  pluginActionError: string | null
  handleDeployPlugin: () => Promise<void>
  handlePurgePlugin: () => Promise<void>
}

export function usePluginAction(onComplete: (success: boolean) => void): PluginActionState {
  const [pluginActionRunning, setPluginActionRunning] = useState<null | 'deploy' | 'purge'>(null)
  const [pluginActionError, setPluginActionError] = useState<string | null>(null)
  const [polling, setPolling] = useState(false)
  const actionRef = useRef<null | 'deploy' | 'purge'>(null)

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

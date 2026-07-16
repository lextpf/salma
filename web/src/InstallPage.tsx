import { useCallback } from 'react'
import Button from './comps/Button'
import MIcon from './comps/MIcon'
import ModuleHeader from './comps/ModuleHeader'
import { VRule } from './comps/Rule'
import DropPromptBar, { type DropPromptVariant } from './comps/DropPromptBar'
import SessionFeed from './comps/SessionFeed'
import ProgressRibbon from './comps/ProgressRibbon'
import { ACCEPT } from './comps/formats'
import { useSystemStatus } from './useSystemStatus'
import { useInstallation } from './useInstallation'
import { useInstallConsole } from './useInstallConsole'
import { useFileDrop } from './useFileDrop'
import { useTestRunner } from './useTestRunner'
import { usePluginAction } from './usePluginAction'
import { useContentBreakpoints } from './useViewportNarrow'

export default function InstallPage() {
  // share one page poller across install views.
  const { status, config, loading: statusLoading, refreshRef } = useSystemStatus()

  const pluginInstalled = status?.pluginInstalled === true
  const systemUnavailable = statusLoading || !status
  const pluginPurged = !systemUnavailable && status.pluginInstalled === false

  const { jobs, isInstalling, handleFileSelect, cancel } = useInstallation(pluginInstalled)
  const { testRunning, testError, handleRunTests } = useTestRunner(pluginInstalled)

  const onPluginActionComplete = useCallback((success: boolean) => {
    if (success) refreshRef.current(false, true)
  }, [refreshRef])

  const { pluginActionRunning, pluginActionError, handleDeployPlugin, handlePurgePlugin } =
    usePluginAction(onPluginActionComplete)

  // lock intake while unavailable, purged, or installing.
  const locked = !pluginInstalled || systemUnavailable || isInstalling

  // useFileDrop expects a void callback, so drop the install promise here.
  const onFiles = useCallback((files: FileList) => { void handleFileSelect(files); }, [handleFileSelect])

  const { isDragging, inputRef, openPicker, onInputChange, onDragOver, onDragLeave, onDrop } =
    useFileDrop({ onFiles, disabled: locked })

  // installs run sequentially. the first non-terminal job is active.
  const active = (() => {
    for (let i = 0; i < jobs.length; i++) {
      const s = jobs[i].status
      if (s !== 'completed' && s !== 'error') {
        return { job: jobs[i], index: i + 1 }
      }
    }
    return null
  })()

  // share one console poll between the active card and ribbon.
  const { lines, rawLines } = useInstallConsole(active?.job.id ?? null, isInstalling)

  const queuedCount = jobs.filter(j => j.status === 'pending').length
  const errorBanner = testError || pluginActionError

  // unavailable state takes priority because plugin state is then unknown.
  const dropVariant: DropPromptVariant | null = systemUnavailable
    ? 'unavailable'
    : pluginPurged
      ? 'locked'
      : isInstalling
        ? 'installing'
        : null


  const { compactToolbar } = useContentBreakpoints()

  return (
    <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column' }}>
      <input
        ref={inputRef}
        type="file"
        multiple
        accept={ACCEPT}
        onChange={onInputChange}
        disabled={locked}
        style={{ display: 'none' }}
      />

      <ModuleHeader num="01" label="Install">
        {dropVariant && (
          <>
            <VRule height={18} />
            <DropPromptBar variant={dropVariant} compact={compactToolbar} />
          </>
        )}
        <div style={{ flex: 1 }} />
        <Button
          icon="science"
          label={testRunning ? 'Running...' : 'Run tests'}
          onClick={() => { void handleRunTests(); }}
          disabled={!pluginInstalled || testRunning || isInstalling}
          running={testRunning}
          compact={compactToolbar}
        />
        <Button
          icon="power"
          label={pluginActionRunning === 'deploy' ? 'Deploying...' : pluginPurged ? 'Deploy plugin' : 'Deploy'}
          onClick={() => { void handleDeployPlugin(); }}
          disabled={systemUnavailable || pluginActionRunning !== null || isInstalling}
          variant={pluginPurged ? 'primary' : 'ghost'}
          running={pluginActionRunning === 'deploy'}
          compact={compactToolbar}
        />
        <Button
          icon="delete"
          label={pluginActionRunning === 'purge' ? 'Purging...' : 'Purge'}
          onClick={() => { void handlePurgePlugin(); }}
          disabled={!pluginInstalled || pluginActionRunning !== null || isInstalling}
          variant="danger"
          running={pluginActionRunning === 'purge'}
          compact={compactToolbar}
        />
      </ModuleHeader>

      {errorBanner && (
        <div
          role="alert"
          style={{
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '6px 18px',
            background: 'var(--danger-wash)',
            fontFamily: 'var(--font-mono)',
            fontSize: 'var(--fs-label)',
            color: 'var(--danger)',
          }}
        >
          <MIcon name="warning" size={13} style={{ flexShrink: 0 }} />
          <span style={{ whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>{errorBanner}</span>
        </div>
      )}

      <SessionFeed
        jobs={jobs}
        active={active}
        isInstalling={isInstalling}
        locked={locked}
        purged={pluginPurged}
        consoleLines={lines}
        rawLines={rawLines}
        total={jobs.length}
        onCancel={cancel}
        onBrowse={openPicker}
        isDragging={isDragging}
        onDragOver={onDragOver}
        onDragLeave={onDragLeave}
        onDrop={onDrop}
        dragFileNames={[]}
        stats={status ? { inferred: status.jsonCount, mods: status.modCount } : undefined}
        destPath={config?.mo2ModsPath || undefined}
      />
      {isInstalling && active && (
        <ProgressRibbon job={active.job} rawLines={rawLines} index={active.index} total={jobs.length} queuedCount={queuedCount} />
      )}
    </div>
  )
}

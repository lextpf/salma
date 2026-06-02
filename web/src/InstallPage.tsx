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

// Module 01 - Install. A single monospace session feed: ambient intake (drop
// anywhere / header prompt bar), terminal and queued jobs as one-line rows, the
// active job as an inline card, and a docked progress ribbon.
export default function InstallPage() {
  // One poller for the page. `status` and `config` also feed the idle intake
  // panel (destination path, library tallies), so it prints real figures
  // without opening a second polling loop beside the chrome's.
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

  // Intake (drop and browse) is disabled while installing, while the system is
  // unavailable, and without the plugin, matching the hook's early return.
  const locked = !pluginInstalled || systemUnavailable || isInstalling

  const { isDragging, inputRef, openPicker, onInputChange, onDragOver, onDragLeave, onDrop } =
    useFileDrop({ onFiles: handleFileSelect, disabled: locked })

  // The active job is the first non-terminal one, which is the one installing.
  // The queue runs sequentially, so everything before it is terminal and
  // everything after it is queued.
  const active = (() => {
    for (let i = 0; i < jobs.length; i++) {
      const s = jobs[i].status
      if (s !== 'completed' && s !== 'error') {
        return { job: jobs[i], index: i + 1 }
      }
    }
    return null
  })()

  // Poll the install console once here and share it with the active card and the
  // ribbon (both render the same job).
  const { lines, rawLines } = useInstallConsole(active?.job.id ?? null, isInstalling)

  const queuedCount = jobs.filter(j => j.status === 'pending').length
  const errorBanner = testError || pluginActionError

  // systemUnavailable (initial connect or mo2-server unreachable) takes priority:
  // status is null so pluginPurged cannot be determined and no install can be
  // progressing. Its variant is non-interactive, so intake shows as offline
  // rather than a clickable-but-inert browse affordance (openPicker is a no-op
  // while locked === systemUnavailable).
  // Null when intake is ready: the rail below already carries the invitation,
  // next to the actual drop target.
  const dropVariant: DropPromptVariant | null = systemUnavailable
    ? 'unavailable'
    : pluginPurged
      ? 'locked'
      : isInstalling
        ? 'installing'
        : null


  // A ~936px window leaves 720px beside the 216px rail, which is not enough
  // for three labelled actions plus the drop prompt. Collapse to icons there.
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
          onClick={handleRunTests}
          disabled={!pluginInstalled || testRunning || isInstalling}
          running={testRunning}
          compact={compactToolbar}
        />
        <Button
          icon="power"
          label={pluginActionRunning === 'deploy' ? 'Deploying...' : pluginPurged ? 'Deploy plugin' : 'Deploy'}
          onClick={handleDeployPlugin}
          disabled={systemUnavailable || pluginActionRunning !== null || isInstalling}
          variant={pluginPurged ? 'primary' : 'ghost'}
          running={pluginActionRunning === 'deploy'}
          compact={compactToolbar}
        />
        <Button
          icon="delete"
          label={pluginActionRunning === 'purge' ? 'Purging...' : 'Purge'}
          onClick={handlePurgePlugin}
          disabled={!pluginInstalled || pluginActionRunning !== null || isInstalling}
          variant="danger"
          running={pluginActionRunning === 'purge'}
          compact={compactToolbar}
        />
      </ModuleHeader>

      {/* Error strip - a flat band, carried by its danger wash */}
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

      {/* Feed + docked ribbon */}
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

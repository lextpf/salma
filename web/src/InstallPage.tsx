import { useCallback } from 'react'
import Kicker from './comps/Kicker'
import MIcon from './comps/MIcon'
import DropPromptBar, { type DropPromptVariant } from './comps/install/DropPromptBar'
import SessionFeed from './comps/install/SessionFeed'
import ProgressRibbon from './comps/install/ProgressRibbon'
import { ACCEPT } from './comps/install/formats'
import { useSystemStatus } from './useSystemStatus'
import { useInstallation } from './useInstallation'
import { useInstallConsole } from './useInstallConsole'
import { useFileDrop } from './useFileDrop'
import { useTestRunner } from './useTestRunner'
import { usePluginAction } from './usePluginAction'
import { useViewportNarrow } from './useViewportNarrow'

const toolbarBtnBase: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  gap: 7,
  height: 30,
  padding: '6px 12px',
  border: '1px solid var(--rule)',
  borderRadius: 7,
  background: 'var(--sheet)',
  color: 'var(--ink-2)',
  fontSize: 'var(--fs-body)',
  cursor: 'pointer',
  fontFamily: 'inherit',
  whiteSpace: 'nowrap',
}

interface ToolbarButtonProps {
  icon: string
  label: string
  onClick: () => void
  disabled?: boolean
  danger?: boolean
  primary?: boolean
  running?: boolean
  // Icon-only rendering for narrow viewports; the label moves to title/aria.
  compact?: boolean
}

// One header-bar action. Spins its glyph while running; danger tints the border
// and label for purge; primary inverts to the ink button (used for Deploy when
// the plugin is purged).
function ToolbarButton({ icon, label, onClick, disabled = false, danger = false, primary = false, running = false, compact = false }: ToolbarButtonProps) {
  const iconColor = primary ? 'var(--sheet)' : danger ? 'var(--danger)' : 'var(--ink-4)'
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      aria-label={compact ? label : undefined}
      title={compact ? label : undefined}
      style={{
        ...toolbarBtnBase,
        ...(danger
          ? { border: '1px solid color-mix(in srgb, var(--danger) 35%, transparent)', color: 'var(--danger)' }
          : null),
        ...(primary
          ? { border: '1px solid var(--ink)', background: 'var(--ink)', color: 'var(--sheet)', fontWeight: 600 }
          : null),
        ...(disabled ? { opacity: 0.5, cursor: 'not-allowed' } : null),
      }}
    >
      {running ? (
        <MIcon name="progress_activity" className="m-spin" size={13} style={{ color: iconColor }} />
      ) : (
        <MIcon name={icon} size={13} style={{ color: iconColor }} />
      )}
      {!compact && <span>{label}</span>}
    </button>
  )
}

// Module 01 - Install. A single monospace session feed: ambient intake (drop
// anywhere / header prompt bar), terminal and queued jobs as one-line rows, the
// active job as an inline card, and a docked progress ribbon.
export default function InstallPage() {
  const { status, loading: statusLoading, refreshRef } = useSystemStatus()

  const pluginInstalled = status?.pluginInstalled === true
  const systemUnavailable = statusLoading || !status
  const pluginPurged = !systemUnavailable && status?.pluginInstalled === false

  const { jobs, isInstalling, handleFileSelect, cancel } = useInstallation(pluginInstalled)
  const { testRunning, testError, handleRunTests } = useTestRunner(pluginInstalled)

  const onPluginActionComplete = useCallback((success: boolean) => {
    if (success) refreshRef.current(false, true)
  }, [refreshRef])

  const { pluginActionRunning, pluginActionError, handleDeployPlugin, handlePurgePlugin } =
    usePluginAction(onPluginActionComplete)

  // Intake (drop + browse) is disabled while installing, unavailable, or without
  // the plugin - matching the hook's early return and the old dropzone.
  const locked = !pluginInstalled || systemUnavailable || isInstalling

  const { isDragging, inputRef, openPicker, onInputChange, onDragOver, onDragLeave, onDrop } =
    useFileDrop({ onFiles: handleFileSelect, disabled: locked })

  // Active job = the FIRST non-terminal job (the one actually installing). The
  // queue processes sequentially, so earlier jobs are terminal and later ones
  // are queued.
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
  const dropVariant: DropPromptVariant = systemUnavailable
    ? 'unavailable'
    : pluginPurged
      ? 'locked'
      : isInstalling
        ? 'installing'
        : isDragging
          ? 'drag'
          : 'idle'

  const ledColor = pluginPurged ? 'var(--danger)' : systemUnavailable ? 'var(--ink-5)' : 'var(--ink)'
  const ledText = pluginPurged ? 'purged' : systemUnavailable ? 'connecting' : 'deployed'

  // Phone-landscape widths (~930px) leave ~740px next to the module rail -
  // not enough for the full header. Collapse to icon-only actions there.
  const narrowHeader = useViewportNarrow()

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

      {/* Header (46px) */}
      <div
        style={{
          height: 46,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          gap: 12,
          padding: '0 18px',
          borderBottom: '1px solid var(--rule-soft)',
          boxShadow: 'var(--shadow-elevation-1)',
        }}
      >
        <Kicker num="01" label="Install" />
        <DropPromptBar variant={dropVariant} onBrowse={openPicker} />
        <div style={{ flex: 1 }} />
        <span
          title={ledText}
          style={{ display: 'inline-flex', alignItems: 'center', gap: 6, fontFamily: 'var(--font-mono)', fontSize: 'var(--fs-micro)', color: pluginPurged ? 'var(--tier-low-fg)' : 'var(--ink-3)' }}
        >
          <span
            aria-hidden="true"
            style={{
              width: 6,
              height: 6,
              borderRadius: '50%',
              background: ledColor,
              animation: isInstalling && pluginInstalled ? 'salma-blink 1.4s infinite' : undefined,
            }}
          />
          {!narrowHeader && ledText}
        </span>
        <ToolbarButton
          icon="science"
          label={testRunning ? 'Running...' : 'Run tests'}
          onClick={handleRunTests}
          disabled={!pluginInstalled || testRunning || isInstalling}
          running={testRunning}
          compact={narrowHeader}
        />
        <ToolbarButton
          icon="power"
          label={pluginActionRunning === 'deploy' ? 'Deploying...' : pluginPurged ? 'Deploy plugin' : 'Deploy'}
          onClick={handleDeployPlugin}
          disabled={systemUnavailable || pluginActionRunning !== null || isInstalling}
          primary={pluginPurged}
          running={pluginActionRunning === 'deploy'}
          compact={narrowHeader}
        />
        <ToolbarButton
          icon="delete"
          label={pluginActionRunning === 'purge' ? 'Purging...' : 'Purge'}
          onClick={handlePurgePlugin}
          disabled={!pluginInstalled || pluginActionRunning !== null || isInstalling}
          danger
          running={pluginActionRunning === 'purge'}
          compact={narrowHeader}
        />
      </div>

      {/* Error strip */}
      {errorBanner && (
        <div
          role="alert"
          style={{
            flexShrink: 0,
            display: 'flex',
            alignItems: 'center',
            gap: 8,
            padding: '6px 18px',
            borderBottom: '1px solid var(--rule-soft)',
            background: 'color-mix(in srgb, var(--danger) 7%, transparent)',
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
      />
      {isInstalling && active && (
        <ProgressRibbon job={active.job} rawLines={rawLines} index={active.index} total={jobs.length} queuedCount={queuedCount} />
      )}
    </div>
  )
}

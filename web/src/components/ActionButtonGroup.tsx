import { Link } from 'react-router-dom'

interface ActionButtonGroupProps {
  pluginInstalled: boolean
  systemUnavailable: boolean
  scanRunning: boolean
  testRunning: boolean
  pluginActionRunning: null | 'deploy' | 'purge'
  scanError: string | null
  testError: string | null
  pluginActionError: string | null
  actionChip: { key: string; text: string } | null
  handleDeployPlugin: () => void
  handlePurgePlugin: () => void
  handleScanFomods: () => void
  handleRunTests: () => void
}

export default function ActionButtonGroup({
  pluginInstalled,
  systemUnavailable,
  scanRunning,
  testRunning,
  pluginActionRunning,
  scanError,
  testError,
  pluginActionError,
  actionChip,
  handleDeployPlugin,
  handlePurgePlugin,
  handleScanFomods,
  handleRunTests,
}: ActionButtonGroupProps) {
  if (systemUnavailable) {
    return (
      <div className="flex flex-wrap gap-2">
        <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
        <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
        <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
      </div>
    )
  }

  return (
    <div className="flex flex-wrap items-center gap-3">
      <button
        onClick={handleDeployPlugin}
        disabled={pluginActionRunning !== null}
        className={`rounded-xl px-5 py-2.5 text-xs font-semibold transition-all flex items-center gap-2 action-btn action-btn-success
          ${pluginActionRunning === 'deploy'
            ? 'bg-surface-container-high text-on-surface-variant border border-outline-variant/40 cursor-not-allowed opacity-70'
            : 'bg-success/12 text-on-surface border border-success/20 hover:bg-success/24 hover:border-success/40'
          }`}
      >
        <i className={`fa-duotone fa-solid icon-sm ${pluginActionRunning === 'deploy' ? 'fa-spinner fa-spin' : 'fa-plug-circle-check icon-gradient icon-gradient-spring'}`} />
        {pluginActionRunning === 'deploy' ? 'Deploying...' : 'Deploy Plugin'}
      </button>

      <button
        onClick={handlePurgePlugin}
        disabled={pluginActionRunning !== null}
        className={`rounded-xl px-5 py-2.5 text-xs font-semibold transition-all flex items-center gap-2 action-btn action-btn-error
          ${pluginActionRunning === 'purge'
            ? 'bg-surface-container-high text-on-surface-variant border border-outline-variant/40 cursor-not-allowed opacity-70'
            : 'bg-error/10 text-on-surface border border-error/20 hover:bg-error/18 hover:border-error/35'
          }`}
      >
        <i className={`fa-duotone fa-solid icon-sm ${pluginActionRunning === 'purge' ? 'fa-spinner fa-spin' : 'fa-trash-can icon-gradient icon-gradient-ember'}`} />
        {pluginActionRunning === 'purge' ? 'Purging...' : 'Purge Plugin'}
      </button>

      <button
        onClick={handleScanFomods}
        disabled={scanRunning || !pluginInstalled}
        className={`rounded-xl px-5 py-2.5 text-xs font-semibold transition-all flex items-center gap-2 action-btn action-btn-primary
          ${!pluginInstalled
            ? 'bg-surface-container-high text-error/75 border border-outline-variant/45 cursor-not-allowed opacity-90'
            : scanRunning
            ? 'bg-surface-container-high text-on-surface-variant border border-outline-variant/40 cursor-not-allowed opacity-70'
            : 'bg-primary/12 text-on-surface border border-primary/20 hover:bg-primary/22 hover:border-primary/40'
          }`}
      >
        <i className={`fa-duotone fa-solid icon-sm ${
          !pluginInstalled
            ? 'fa-lock text-error/70'
            : (scanRunning ? 'fa-spinner fa-spin' : 'fa-arrows-rotate icon-gradient icon-gradient-atlas')
        }`} />
        {scanRunning ? 'Scanning...' : 'Scan FOMODs'}
      </button>
      {actionChip?.key === 'scan' && (
        <span className="inline-flex items-center gap-1 rounded-full border border-success/25 bg-success/10 px-2 py-1 text-[0.68rem] text-success">
          <i className="fa-duotone fa-solid fa-circle-check text-[0.64rem]" />
          {actionChip.text}
        </span>
      )}
      {scanRunning && (
        <Link
          to="/logs"
          className="inline whitespace-nowrap text-xs leading-tight text-primary hover:text-primary-light transition-colors border-b border-current"
        >
          <i className="fa-duotone fa-solid fa-arrow-right text-[0.6rem] align-[-0.04em]" />&nbsp;View salma.log
        </Link>
      )}

      <button
        onClick={handleRunTests}
        disabled={testRunning || !pluginInstalled}
        className={`rounded-xl px-5 py-2.5 text-xs font-semibold transition-all flex items-center gap-2 action-btn action-btn-warning
          ${!pluginInstalled
            ? 'bg-surface-container-high text-error/75 border border-outline-variant/45 cursor-not-allowed opacity-90'
            : testRunning
            ? 'bg-surface-container-high text-on-surface-variant border border-outline-variant/40 cursor-not-allowed opacity-70'
            : 'bg-warning/15 text-on-surface border border-warning/25 hover:bg-warning/30 hover:border-warning/45'
          }`}
      >
        <i className={`fa-duotone fa-solid icon-sm ${
          !pluginInstalled
            ? 'fa-lock text-error/70'
            : (testRunning ? 'fa-spinner fa-spin' : 'fa-flask-vial icon-gradient icon-gradient-ember')
        }`} />
        {testRunning ? 'Tests Running...' : 'Run Tests'}
      </button>
      {testRunning && (
        <Link
          to="/logs"
          className="inline whitespace-nowrap text-xs leading-tight text-primary hover:text-primary-light transition-colors border-b border-current"
        >
          <i className="fa-duotone fa-solid fa-arrow-right text-[0.6rem] align-[-0.04em]" />&nbsp;View test.log
        </Link>
      )}
      {pluginActionError && (
        <span className="text-xs text-error flex items-center gap-1.5">
          <i className="fa-duotone fa-solid fa-circle-exclamation" />{pluginActionError}
        </span>
      )}
      {testError && (
        <span className="text-xs text-error flex items-center gap-1.5">
          <i className="fa-duotone fa-solid fa-circle-exclamation" />{testError}
        </span>
      )}
      {scanError && (
        <span className="text-xs text-error flex items-center gap-1.5">
          <i className="fa-duotone fa-solid fa-circle-exclamation" />{scanError}
        </span>
      )}
    </div>
  )
}

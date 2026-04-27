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

function ToolGroup({ n, label, children }: { n: string; label: string; children: React.ReactNode }) {
  return (
    <div className="flex items-center" style={{ gap: 12 }}>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontStyle: 'italic',
          fontSize: 11,
          letterSpacing: '0.2em',
          textTransform: 'uppercase',
          color: 'var(--ink-4)',
          whiteSpace: 'nowrap',
        }}
      >
        {n} - {label}
      </span>
      <div className="flex" style={{ gap: 6, flexWrap: 'wrap' }}>{children}</div>
    </div>
  )
}

function ToolDivider() {
  return (
    <span
      aria-hidden="true"
      style={{
        width: 1,
        height: 32,
        background: 'var(--rule)',
      }}
    />
  )
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
      <div className="flex items-center" style={{ gap: 28, flexWrap: 'wrap' }}>
        <div className="flex items-center" style={{ gap: 12 }}>
          <div className="skeleton-line" style={{ width: 60, height: 10 }} />
          <div className="flex" style={{ gap: 6 }}>
            <div className="skeleton-line" style={{ width: 120, height: 30, borderRadius: 'var(--radius-sm)' }} />
            <div className="skeleton-line" style={{ width: 110, height: 30, borderRadius: 'var(--radius-sm)' }} />
          </div>
        </div>
        <ToolDivider />
        <div className="flex items-center" style={{ gap: 12 }}>
          <div className="skeleton-line" style={{ width: 60, height: 10 }} />
          <div className="flex" style={{ gap: 6 }}>
            <div className="skeleton-line" style={{ width: 130, height: 30, borderRadius: 'var(--radius-sm)' }} />
            <div className="skeleton-line" style={{ width: 100, height: 30, borderRadius: 'var(--radius-sm)' }} />
          </div>
        </div>
      </div>
    )
  }

  return (
    <div>
      <div className="flex items-center" style={{ gap: 28, flexWrap: 'wrap' }}>
        <ToolGroup n="i" label="Plugin">
          <button
            type="button"
            className="tool-btn"
            onClick={handleDeployPlugin}
            disabled={pluginActionRunning !== null}
          >
            <i
              className={`fa-duotone fa-solid fa-rocket-launch${pluginActionRunning === 'deploy' ? ' fa-bounce' : ''}`}
              style={{ fontSize: 14 }}
            />
            <span>{pluginActionRunning === 'deploy' ? 'Deploying...' : 'Deploy plugin'}</span>
          </button>
          <button
            type="button"
            className="tool-btn"
            onClick={handlePurgePlugin}
            disabled={pluginActionRunning !== null}
          >
            <i
              className={`fa-duotone fa-solid fa-fire-flame-curved${pluginActionRunning === 'purge' ? ' fa-beat-fade' : ''}`}
              style={{ fontSize: 14 }}
            />
            <span>{pluginActionRunning === 'purge' ? 'Purging...' : 'Purge plugin'}</span>
          </button>
        </ToolGroup>

        <ToolDivider />

        <ToolGroup n="ii" label="Fomods">
          <button
            type="button"
            className="tool-btn"
            data-purged={!pluginInstalled ? 'true' : 'false'}
            onClick={handleScanFomods}
            disabled={scanRunning || !pluginInstalled}
          >
            <i
              className={`fa-duotone fa-solid ${
                !pluginInstalled
                  ? 'fa-link-slash'
                  : `fa-radar${scanRunning ? ' fa-spin' : ''}`
              }`}
              style={{ fontSize: 14 }}
            />
            <span>{scanRunning ? 'Scanning...' : 'Scan FOMODs'}</span>
          </button>
          <button
            type="button"
            className="tool-btn tool-btn-primary"
            data-purged={!pluginInstalled ? 'true' : 'false'}
            onClick={handleRunTests}
            disabled={testRunning || !pluginInstalled}
          >
            <i
              className={`fa-duotone fa-solid ${
                !pluginInstalled
                  ? 'fa-ban'
                  : `fa-flask${testRunning ? ' fa-shake' : ''}`
              }`}
              style={{ fontSize: 14 }}
            />
            <span>{testRunning ? 'Tests running...' : 'Run tests'}</span>
          </button>
        </ToolGroup>

        {scanRunning && (
          <Link
            to="/logs"
            className="flex items-center"
            style={{
              gap: 6,
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
              color: 'var(--ink-blue)',
              textDecoration: 'none',
            }}
          >
            <span className="dot-status dot-status-warn dot-status-pulse" />
            // tail salma.log -&gt;
          </Link>
        )}
        {testRunning && (
          <Link
            to="/logs"
            className="flex items-center"
            style={{
              gap: 6,
              fontFamily: 'var(--font-mono)',
              fontSize: 12,
              color: 'var(--ink-blue)',
              textDecoration: 'none',
            }}
          >
            <span className="dot-status dot-status-warn dot-status-pulse" />
            // tail test.log -&gt;
          </Link>
        )}
      </div>

      {/* Status messages line */}
      {(actionChip?.key === 'scan' || scanError || testError || pluginActionError) && (
        <div
          className="flex items-center"
          style={{ gap: 14, flexWrap: 'wrap', marginTop: 12 }}
        >
          {actionChip?.key === 'scan' && (
            <span
              className="flex items-center"
              style={{
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 12,
                letterSpacing: '0.1em',
                textTransform: 'uppercase',
                color: 'var(--moss)',
              }}
            >
              <span className="dot-status dot-status-on" />
              {actionChip.text}
            </span>
          )}
          {pluginActionError && (
            <span
              className="flex items-center"
              style={{
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 12,
                color: 'var(--danger)',
              }}
            >
              <span className="dot-status dot-status-error" />
              {pluginActionError}
            </span>
          )}
          {scanError && (
            <span
              className="flex items-center"
              style={{
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 12,
                color: 'var(--danger)',
              }}
            >
              <span className="dot-status dot-status-error" />
              {scanError}
            </span>
          )}
          {testError && (
            <span
              className="flex items-center"
              style={{
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 12,
                color: 'var(--danger)',
              }}
            >
              <span className="dot-status dot-status-error" />
              {testError}
            </span>
          )}
        </div>
      )}
    </div>
  )
}

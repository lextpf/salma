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
          fontSize: 9,
          letterSpacing: '0.2em',
          textTransform: 'uppercase',
          color: 'var(--ink-4)',
          whiteSpace: 'nowrap',
        }}
      >
        {n} · {label}
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
              style={{ fontSize: 13 }}
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
              style={{ fontSize: 13 }}
            />
            <span>{pluginActionRunning === 'purge' ? 'Purging...' : 'Purge plugin'}</span>
          </button>
        </ToolGroup>

        <ToolDivider />

        <ToolGroup n="ii" label="Fomods">
          <button
            type="button"
            className="tool-btn"
            onClick={handleScanFomods}
            disabled={scanRunning || !pluginInstalled}
          >
            <i
              className={`fa-duotone fa-solid ${
                !pluginInstalled
                  ? 'fa-link-slash'
                  : `fa-radar${scanRunning ? ' fa-spin' : ''}`
              }`}
              style={{ fontSize: 13 }}
            />
            <span>{scanRunning ? 'Scanning...' : 'Scan FOMODs'}</span>
          </button>
          <button
            type="button"
            className="tool-btn tool-btn-primary"
            onClick={handleRunTests}
            disabled={testRunning || !pluginInstalled}
          >
            <i
              className={`fa-duotone fa-solid ${
                !pluginInstalled
                  ? 'fa-ban'
                  : `fa-flask${testRunning ? ' fa-shake' : ''}`
              }`}
              style={{ fontSize: 13 }}
            />
            <span>{testRunning ? 'Tests running...' : 'Run tests'}</span>
          </button>
        </ToolGroup>

        <div style={{ flex: 1 }} />

        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: 10.5,
            color: 'var(--ink-4)',
            letterSpacing: '0.08em',
            whiteSpace: 'nowrap',
          }}
        >
          <span style={{ color: 'var(--accent)' }}>⌘</span> + <span>U</span> to upload
        </span>
      </div>

      {/* Status messages line */}
      {(actionChip?.key === 'scan' || scanRunning || testRunning || scanError || testError || pluginActionError) && (
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
                fontSize: 10.5,
                letterSpacing: '0.1em',
                textTransform: 'uppercase',
                color: 'var(--moss)',
              }}
            >
              <span className="dot-status dot-status-on" />
              {actionChip.text}
            </span>
          )}
          {scanRunning && (
            <Link
              to="/logs"
              className="flex items-center"
              style={{
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 10.5,
                color: 'var(--ink-blue)',
                textDecoration: 'none',
              }}
            >
              <span className="dot-status dot-status-warn dot-status-pulse" />
              // tail salma.log →
            </Link>
          )}
          {testRunning && (
            <Link
              to="/logs"
              className="flex items-center"
              style={{
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 10.5,
                color: 'var(--ink-blue)',
                textDecoration: 'none',
              }}
            >
              <span className="dot-status dot-status-warn dot-status-pulse" />
              // tail test.log →
            </Link>
          )}
          {pluginActionError && (
            <span
              className="flex items-center"
              style={{
                gap: 6,
                fontFamily: 'var(--font-mono)',
                fontSize: 10.5,
                color: 'var(--accent)',
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
                fontSize: 10.5,
                color: 'var(--accent)',
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
                fontSize: 10.5,
                color: 'var(--accent)',
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

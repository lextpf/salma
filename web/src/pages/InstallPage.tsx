import { useEffect, useState, useCallback, useRef } from 'react'
import FileUpload from '../components/FileUpload'
import InstallationQueueShell from '../components/InstallationQueueShell'
import SystemStatusSection from '../components/SystemStatusSection'
import ActionButtonGroup from '../components/ActionButtonGroup'
import Section from '../components/Section'
import { useSystemStatus } from '../hooks/useSystemStatus'
import { useScanJob } from '../hooks/useScanJob'
import { useTestRunner } from '../hooks/useTestRunner'
import { usePluginAction } from '../hooks/usePluginAction'
import { useInstallation } from '../hooks/useInstallation'

type ActionChipKey = 'scan'

export default function InstallPage() {
  const { status, config, loading: statusLoading, refreshRef: refreshSystemStatusRef } = useSystemStatus()

  const [actionChip, setActionChip] = useState<{ key: ActionChipKey; text: string } | null>(null)
  const [readyPulse, setReadyPulse] = useState(false)
  const chipTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const prevReadyRef = useRef<number | null>(null)

  const pluginInstalled = status?.pluginInstalled === true
  const systemUnavailable = statusLoading || !status || !config
  const pluginPurged = !systemUnavailable && status?.pluginInstalled === false
  const { jobs, isInstalling, handleFileSelect } = useInstallation(pluginInstalled)

  const showActionChip = useCallback((key: ActionChipKey, text: string) => {
    if (chipTimerRef.current) clearTimeout(chipTimerRef.current)
    setActionChip({ key, text })
    chipTimerRef.current = setTimeout(() => setActionChip(null), 2500)
  }, [])

  const refreshStatus = useCallback((force: boolean) => {
    refreshSystemStatusRef.current(force)
  }, [refreshSystemStatusRef])

  const onScanComplete = useCallback((success: boolean) => {
    refreshStatus(false)
    if (success) showActionChip('scan', 'Scan complete')
  }, [refreshStatus, showActionChip])

  const onPluginActionComplete = useCallback((success: boolean) => {
    if (success) refreshStatus(false)
  }, [refreshStatus])

  const { scanRunning, scanError, handleScanFomods } = useScanJob(pluginInstalled, onScanComplete)
  const { testRunning, testError, handleRunTests } = useTestRunner(pluginInstalled)
  const { pluginActionRunning, pluginActionError, handleDeployPlugin, handlePurgePlugin } = usePluginAction(onPluginActionComplete)

  useEffect(() => () => { if (chipTimerRef.current) clearTimeout(chipTimerRef.current) }, [])

  const readyCount = status?.jsonCount ?? null
  useEffect(() => {
    if (readyCount === null) return
    if (prevReadyRef.current === null) { prevReadyRef.current = readyCount; return }
    if (prevReadyRef.current !== readyCount) {
      setReadyPulse(true)
      const id = setTimeout(() => setReadyPulse(false), 650)
      prevReadyRef.current = readyCount
      return () => clearTimeout(id)
    }
    prevReadyRef.current = readyCount
  }, [readyCount])

  // Mark isInstalling/scanRunning/testRunning as referenced - the page
  // itself doesn't need them anymore (Layout polls engine state) but the
  // hooks still need their callbacks fired.
  void isInstalling; void scanRunning; void testRunning

  return (
    <div className="page-fill">
      {/* ============================================================
          PAGE HEADER (compact for short viewports)
          ============================================================ */}
      <header style={{ marginBottom: 14, flexShrink: 0 }}>
        <p
          className="reveal reveal-delay-1"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 10,
            fontFamily: 'var(--font-mono)',
            fontSize: 13,
            letterSpacing: '0.18em',
            textTransform: 'uppercase',
            color: 'var(--ink-3)',
            marginBottom: 10,
          }}
        >
          <span
            className="display-serif-italic"
            style={{ fontSize: 14, color: 'var(--accent)', textTransform: 'none' }}
          >
            01.
          </span>
          <span>Sec. Upload and install mod archives</span>
        </p>

        <div className="flex items-end reveal reveal-delay-2" style={{ gap: 18, flexWrap: 'wrap' }}>
          <h1
            className="display-serif"
            style={{ fontSize: 64, lineHeight: 0.95, color: 'var(--ink)', margin: 0 }}
          >
            Install<span style={{ color: 'var(--accent)' }}>.</span>
          </h1>
          {!systemUnavailable && status && (
            <span className={`ready-pip ${readyPulse ? 'ring-pulse-once' : ''}`} style={{ marginBottom: 10, padding: '6px 12px 6px 10px' }}>
              <span className="dot-status dot-status-on dot-status-pulse" />
              <span
                className="tabular-nums"
                style={{ fontSize: 15, fontWeight: 500, color: 'var(--ink)' }}
              >
                {status.jsonCount}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 12,
                  letterSpacing: '0.1em',
                  textTransform: 'uppercase',
                  color: 'var(--ink-4)',
                }}
              >
                ready
              </span>
            </span>
          )}
        </div>
        <p
          className="reveal reveal-delay-3 display-serif-italic"
          style={{
            margin: '8px 0 0',
            fontSize: 15,
            color: 'var(--ink-3)',
            maxWidth: 620,
            lineHeight: 1.55,
          }}
        >
          Drop an archive - salma will extract, parse the FOMOD, and infer selections against your mod tree.
        </p>
      </header>

      {/* ============================================================
          SECTION 01 - UPLOAD (natural height)
          ============================================================ */}
      <div className="reveal reveal-delay-4" style={{ marginBottom: 16, flexShrink: 0 }}>
        <Section n="01" label="Upload" title="Select an archive" corner="01" bodyPadding="none">
          <div className="grid" style={{ gridTemplateColumns: 'minmax(0, 1fr) 280px', gap: 0 }}>
            <div style={{ borderRight: '1px solid var(--rule)', minWidth: 0 }}>
              {systemUnavailable ? (
                <div style={{ padding: '32px 28px' }}>
                  <div className="empty-state-card" style={{ gap: 18 }}>
                    <div
                      className="skeleton-line"
                      style={{
                        width: 38,
                        height: 38,
                        borderRadius: '50%',
                        flexShrink: 0,
                      }}
                    />
                    <div style={{ flex: 1, minWidth: 0 }}>
                      <div className="skeleton-line" style={{ height: 22, width: 280, marginBottom: 8 }} />
                      <div className="skeleton-line" style={{ height: 11, width: 220 }} />
                    </div>
                  </div>
                </div>
              ) : (
                <FileUpload
                  onFileSelect={handleFileSelect}
                  disabled={!pluginInstalled || isInstalling}
                  purged={pluginPurged}
                />
              )}
            </div>
            <SystemStatusSection status={status} config={config} />
          </div>
          <div className="atelier-section-toolbar">
            <ActionButtonGroup
              pluginInstalled={pluginInstalled}
              systemUnavailable={systemUnavailable}
              scanRunning={scanRunning}
              testRunning={testRunning}
              pluginActionRunning={pluginActionRunning}
              scanError={scanError}
              testError={testError}
              pluginActionError={pluginActionError}
              actionChip={actionChip}
              handleDeployPlugin={handleDeployPlugin}
              handlePurgePlugin={handlePurgePlugin}
              handleScanFomods={handleScanFomods}
              handleRunTests={handleRunTests}
            />
          </div>
        </Section>
      </div>

      {/* ============================================================
          SECTION 02 - ACTIVITY (collapsible; ledger bar in meta when
          collapsed, full queue table when expanded)
          ============================================================ */}
      <InstallationQueueShell jobs={jobs} systemUnavailable={systemUnavailable} />
    </div>
  )
}

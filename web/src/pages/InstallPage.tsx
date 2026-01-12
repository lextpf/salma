import { useEffect, useState, useCallback, useRef } from 'react'
import FileUpload from '../components/FileUpload'
import InstallationQueue from '../components/InstallationQueue'
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

  const activeCount    = jobs.filter(j => j.status === 'uploading' || j.status === 'processing').length
  const queuedCount    = jobs.filter(j => j.status === 'pending').length
  const completedCount = jobs.filter(j => j.status === 'completed').length

  return (
    <div className="page-fill">
      {/* ============================================================
          PAGE HEADER (compact)
          ============================================================ */}
      <header style={{ marginBottom: 28, flexShrink: 0 }}>
        <p
          className="reveal reveal-delay-1"
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 10,
            fontFamily: 'var(--font-mono)',
            fontSize: 11,
            letterSpacing: '0.18em',
            textTransform: 'uppercase',
            color: 'var(--ink-3)',
            marginBottom: 18,
          }}
        >
          <span
            className="display-serif-italic"
            style={{ fontSize: 12, color: 'var(--accent)', textTransform: 'none' }}
          >
            01.
          </span>
          <span>Sec. Upload and install mod archives</span>
        </p>

        <div className="flex items-end reveal reveal-delay-2" style={{ gap: 18, flexWrap: 'wrap' }}>
          <h1
            className="display-serif"
            style={{ fontSize: 92, lineHeight: 0.92, color: 'var(--ink)', margin: 0 }}
          >
            Install<span style={{ color: 'var(--accent)' }}>.</span>
          </h1>
          {!systemUnavailable && status && (
            <span className={`ready-pip ${readyPulse ? 'ring-pulse-once' : ''}`} style={{ marginBottom: 12, padding: '6px 12px 6px 10px' }}>
              <span className="dot-status dot-status-on dot-status-pulse" />
              <span
                className="tabular-nums"
                style={{ fontSize: 13, fontWeight: 500, color: 'var(--ink)' }}
              >
                {status.jsonCount}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 10,
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
            margin: '18px 0 0',
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
          SECTION 02 - ACTIVITY (natural height; queue caps + scrolls
          internally so it never dominates the page)
          ============================================================ */}
      <Section
        n="02"
        label="Activity"
        title="Installation queue"
        corner="02"
        bodyPadding="none"
        className="reveal reveal-delay-5"
        meta={
          <div className="flex items-center" style={{ gap: 16, flexWrap: 'wrap' }}>
            <Badge color="var(--moss)"  label={`${activeCount} active`} />
            <Badge color="var(--ink-4)" label={`${queuedCount} queued`} />
            <Badge color="var(--ink-5)" label={`${completedCount} complete`} dim />
          </div>
        }
      >
        <div style={systemUnavailable ? undefined : { maxHeight: 320, overflowY: 'auto' }}>
          {systemUnavailable ? (
            <div>
              {[0, 1].map(i => (
                <div
                  key={i}
                  className="grid"
                  style={{
                    gridTemplateColumns: '38px 1fr 130px 180px 70px 20px',
                    gap: 18,
                    padding: '16px 28px',
                    borderBottom: i === 1 ? 'none' : '1px solid var(--rule-soft)',
                    alignItems: 'center',
                  }}
                >
                  <div className="skeleton-line" style={{ width: 38, height: 38, borderRadius: '50%' }} />
                  <div>
                    <div className="skeleton-line" style={{ height: 14, width: ['72%', '58%'][i], marginBottom: 6 }} />
                    <div className="skeleton-line" style={{ height: 14, width: 110 }} />
                  </div>
                  <div className="skeleton-line" style={{ height: 14, width: 100 }} />
                  <div className="skeleton-line" style={{ height: 14, width: '100%' }} />
                  <div className="skeleton-line" style={{ height: 14, width: 40 }} />
                  <div />
                </div>
              ))}
            </div>
          ) : (
            <InstallationQueue jobs={jobs} />
          )}
        </div>
      </Section>
    </div>
  )
}

function Badge({ color, label, dim }: { color: string; label: string; dim?: boolean }) {
  return (
    <span className="flex items-center" style={{ gap: 6 }}>
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: color,
          opacity: dim ? 0.5 : 1,
        }}
      />
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontSize: 10.5,
          letterSpacing: '0.1em',
          textTransform: 'uppercase',
          color: dim ? 'var(--ink-4)' : 'var(--ink-3)',
        }}
      >
        {label}
      </span>
    </span>
  )
}

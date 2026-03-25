import { useEffect, useState, useCallback, useRef, useMemo } from 'react'
import FileUpload from '../components/FileUpload'
import InstallationQueue from '../components/InstallationQueue'
import SystemStatusSection from '../components/SystemStatusSection'
import ActionButtonGroup from '../components/ActionButtonGroup'
import { useSystemStatus } from '../hooks/useSystemStatus'
import { useScanJob } from '../hooks/useScanJob'
import { useTestRunner } from '../hooks/useTestRunner'
import { usePluginAction } from '../hooks/usePluginAction'
import { useInstallation } from '../hooks/useInstallation'
import { computeReadinessStyles } from '../utils/readinessStyles'

type ActionChipKey = 'scan'

function RelativeTime({ timestamp }: { timestamp: number | null }) {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const id = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(id)
  }, [])
  return <>{formatUpdatedAgo(timestamp, now)}</>
}

function formatUpdatedAgo(timestamp: number | null, now: number): string {
  if (!timestamp) return 'Updated just now'
  const sec = Math.max(0, Math.floor((now - timestamp) / 1000))
  if (sec < 2) return 'Updated just now'
  if (sec < 60) return `Updated ${sec}s ago`
  const min = Math.floor(sec / 60)
  if (min < 60) return `Updated ${min}m ago`
  const hrs = Math.floor(min / 60)
  return `Updated ${hrs}h ago`
}

export default function InstallPage() {
  const { status, config, updatedAt: statusUpdatedAt, loading: statusLoading, refreshRef: refreshSystemStatusRef } = useSystemStatus()

  const [ringPulse, setRingPulse] = useState(false)
  const [systemExpanded, setSystemExpanded] = useState(false)
  const [actionChip, setActionChip] = useState<{ key: ActionChipKey; text: string } | null>(null)
  const chipTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const prevReadinessRef = useRef<number | null>(null)

  const readiness = status
    ? Math.round((Number(status.pluginInstalled) + Number(status.configured) + Number(status.outputFolderExists)) / 3 * 100)
    : null
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
  }, [])

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

  useEffect(() => {
    return () => {
      if (chipTimerRef.current) clearTimeout(chipTimerRef.current)
    }
  }, [])

  useEffect(() => {
    if (readiness === null) return
    if (prevReadinessRef.current === null) {
      prevReadinessRef.current = readiness
      return
    }
    if (prevReadinessRef.current !== readiness) {
      setRingPulse(true)
      const id = setTimeout(() => setRingPulse(false), 650)
      prevReadinessRef.current = readiness
      return () => clearTimeout(id)
    }
    prevReadinessRef.current = readiness
  }, [readiness])

  const styles = useMemo(() => computeReadinessStyles(readiness), [readiness])

  return (
    <div className="animate-fade-in">
      <header className="page-header page-header-install mb-6">
        <h1 className="page-title text-2xl font-bold text-on-surface flex items-center gap-3">
          <span className="inline-flex w-6 shrink-0 items-center justify-center">
            <i className="fa-duotone fa-solid fa-cloud-arrow-up icon-gradient icon-gradient-nebula text-2xl" />
          </span>
          Install
          {!systemExpanded && status && (
            styles.isPerfectReadiness ? (
              <button
                onClick={() => setSystemExpanded(true)}
                className="w-2 h-2 rounded-full ml-1 hover:scale-125 transition-transform shadow-[0_0_5px_2px_rgba(52,211,153,0.35)]"
                style={{ background: 'conic-gradient(rgba(52,211,153,0.92) 0deg, rgba(56,189,248,0.88) 120deg, rgba(192,132,252,0.8) 240deg, rgba(52,211,153,0.92) 360deg)' }}
                title="Readiness: 100% - click to expand"
              />
            ) : (
              <button
                onClick={() => setSystemExpanded(true)}
                className="relative w-[1.1rem] h-[1.1rem] rounded-full ml-0.5 opacity-75 hover:opacity-100 hover:scale-115 transition-transform"
                title={`Readiness: ${styles.readinessValue}% - click to expand`}
              >
                <div className="absolute inset-0 rounded-full" style={styles.miniRingStyle} />
                <div className="absolute inset-[2px] rounded-full bg-surface-container" />
              </button>
            )
          )}
        </h1>
        <p className="page-subtitle mt-1 text-sm ml-9">Upload and install mod archives</p>
      </header>

      <SystemStatusSection
        status={status}
        config={config}
        readiness={readiness}
        styles={styles}
        ringPulse={ringPulse}
        systemExpanded={systemExpanded}
        setSystemExpanded={setSystemExpanded}
        statusUpdatedAt={statusUpdatedAt}
        relativeTimeNode={<RelativeTime timestamp={statusUpdatedAt} />}
      />

      <div className="flex flex-col gap-6">
        <div>
          {systemUnavailable ? (
            <div className="skeleton-line h-3 w-14 rounded mb-2" />
          ) : (
            <p className="ui-label text-primary/85 mb-2">Upload</p>
          )}
          {systemUnavailable ? (
            <div className="rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-5">
              <div className="skeleton-line h-3 w-48 mb-3" />
              <div className="skeleton-line h-3 w-64 mb-3" />
              <div className="skeleton-line h-3 w-56" />
            </div>
          ) : (
            <FileUpload onFileSelect={handleFileSelect} disabled={!pluginInstalled || isInstalling} />
          )}
        </div>

        <div>
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

        {systemUnavailable ? (
          <div>
            <div className="skeleton-line h-3 w-14 rounded mb-2" />
            <div className="rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-4">
              <div className="flex items-center gap-3">
                <div className="w-9 h-9 rounded-xl skeleton-line shrink-0" />
                <div className="flex-1">
                  <div className="skeleton-line h-3.5 w-48 mb-2" />
                  <div className="skeleton-line h-3 w-32" />
                </div>
                <div className="skeleton-line h-5 w-16 rounded-full" />
              </div>
            </div>
          </div>
        ) : (
          <InstallationQueue jobs={jobs} />
        )}
      </div>
    </div>
  )
}


import { useEffect, useState, useCallback, useRef } from 'react'
import { Link } from 'react-router-dom'
import FileUpload from '../components/FileUpload'
import InstallationQueue from '../components/InstallationQueue'
import { getMo2Status, getConfig, runTests, getTestStatus, scanFomods, getFomodScanStatus, deployPlugin, purgePlugin, getPluginActionStatus, getInstallStatus, isFetchUnavailableError } from '../api'
import type { InstallationJob, Mo2Status, AppConfig } from '../types'

type ActionChipKey = 'scan'
const RETRY_DELAY_MS = 2000

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
  const [jobs, setJobs] = useState<InstallationJob[]>([])
  const [status, setStatus] = useState<Mo2Status | null>(null)
  const [config, setConfig] = useState<AppConfig | null>(null)
  const [statusUpdatedAt, setStatusUpdatedAt] = useState<number | null>(null)
  const [nowMs, setNowMs] = useState(() => Date.now())
  const [statusLoading, setStatusLoading] = useState(true)
  const [scanRunning, setScanRunning] = useState(false)
  const [scanError, setScanError] = useState<string | null>(null)
  const [pluginActionRunning, setPluginActionRunning] = useState<null | 'deploy' | 'purge'>(null)
  const [pluginActionError, setPluginActionError] = useState<string | null>(null)
  const [testRunning, setTestRunning] = useState(false)
  const [testError, setTestError] = useState<string | null>(null)
  const [ringPulse, setRingPulse] = useState(false)
  const [systemExpanded, setSystemExpanded] = useState(false)
  const [actionChip, setActionChip] = useState<{ key: ActionChipKey; text: string } | null>(null)
  const chipTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const prevReadinessRef = useRef<number | null>(null)
  const statusRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const testStatusRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const scanStatusRetryTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null)
  const statusInFlightRef = useRef(false)

  const readiness = status
    ? Math.round((Number(status.pluginInstalled) + Number(status.configured) + Number(status.outputFolderExists)) / 3 * 100)
    : null
  const pluginInstalled = status?.pluginInstalled === true
  const systemUnavailable = statusLoading || !status || !config

  const showActionChip = useCallback((key: ActionChipKey, text: string) => {
    if (chipTimerRef.current) clearTimeout(chipTimerRef.current)
    setActionChip({ key, text })
    chipTimerRef.current = setTimeout(() => setActionChip(null), 2500)
  }, [])

  const clearStatusRetryTimer = () => {
    if (statusRetryTimerRef.current) {
      clearTimeout(statusRetryTimerRef.current)
      statusRetryTimerRef.current = null
    }
  }

  const refreshSystemStatus = (showOverlay = true, force = false) => {
    if (statusInFlightRef.current && !force) return
    if (showOverlay) setStatusLoading(true)
    statusInFlightRef.current = true

    Promise.all([getMo2Status(), getConfig()])
      .then(([s, c]) => {
        setStatus(s)
        setConfig(c)
        setStatusUpdatedAt(Date.now())
        setStatusLoading(false)
        clearStatusRetryTimer()
      })
      .catch(e => {
        console.warn('[install] failed to load system status, retrying', e)
        setStatusLoading(true)
        if (!statusRetryTimerRef.current) {
          statusRetryTimerRef.current = setTimeout(() => {
            statusRetryTimerRef.current = null
            refreshSystemStatus(true, true)
          }, RETRY_DELAY_MS)
        }
      })
      .finally(() => {
        statusInFlightRef.current = false
      })
  }

  useEffect(() => {
    refreshSystemStatus(true)

    const loadTestStatus = () => {
      getTestStatus()
        .then(s => {
          setTestRunning(s.running)
          if (testStatusRetryTimerRef.current) {
            clearTimeout(testStatusRetryTimerRef.current)
            testStatusRetryTimerRef.current = null
          }
        })
        .catch(e => {
          console.warn('[install] failed to get test status, retrying', e)
          if (!testStatusRetryTimerRef.current) {
            testStatusRetryTimerRef.current = setTimeout(() => {
              testStatusRetryTimerRef.current = null
              loadTestStatus()
            }, RETRY_DELAY_MS)
          }
        })
    }

    const loadScanStatus = () => {
      getFomodScanStatus()
        .then(s => {
          setScanRunning(Boolean(s.running))
          if (scanStatusRetryTimerRef.current) {
            clearTimeout(scanStatusRetryTimerRef.current)
            scanStatusRetryTimerRef.current = null
          }
        })
        .catch(e => {
          console.warn('[install] failed to get scan status, retrying', e)
          if (!scanStatusRetryTimerRef.current) {
            scanStatusRetryTimerRef.current = setTimeout(() => {
              scanStatusRetryTimerRef.current = null
              loadScanStatus()
            }, RETRY_DELAY_MS)
          }
        })
    }

    loadTestStatus()
    loadScanStatus()

    return () => {
      clearStatusRetryTimer()
      if (testStatusRetryTimerRef.current) clearTimeout(testStatusRetryTimerRef.current)
      if (scanStatusRetryTimerRef.current) clearTimeout(scanStatusRetryTimerRef.current)
    }
  }, [])

  useEffect(() => {
    const id = setInterval(() => setNowMs(Date.now()), 1000)
    return () => clearInterval(id)
  }, [])

  useEffect(() => {
    return () => {
      if (chipTimerRef.current) clearTimeout(chipTimerRef.current)
      clearStatusRetryTimer()
      if (testStatusRetryTimerRef.current) clearTimeout(testStatusRetryTimerRef.current)
      if (scanStatusRetryTimerRef.current) clearTimeout(scanStatusRetryTimerRef.current)
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

  // Poll test status while running
  useEffect(() => {
    if (!testRunning) return
    const id = setInterval(() => {
      getTestStatus()
        .then(s => { if (!s.running) setTestRunning(false) })
        .catch(e => {
          console.warn('[install] failed to poll test status; will retry', e)
        })
    }, 3000)
    return () => clearInterval(id)
  }, [testRunning])

  // Poll scan status while running
  useEffect(() => {
    if (!scanRunning) return
    const id = setInterval(() => {
      getFomodScanStatus()
        .then(async s => {
          if (!s.running) {
            setScanRunning(false)
            refreshSystemStatus(false)
            if (s.success) {
              showActionChip('scan', 'Scan complete')
              setScanError(null)
            } else if (s.error) {
              setScanError(s.error)
            }
          }
        })
        .catch(e => {
          console.warn('[install] failed to poll scan status; will retry', e)
        })
    }, 3000)
    return () => clearInterval(id)
  }, [scanRunning, showActionChip])

  const handleRunTests = useCallback(async () => {
    if (!pluginInstalled) return
    setTestError(null)
    try {
      await runTests()
      setTestRunning(true)
    } catch (e) {
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to start tests due to unavailable backend', e)
        return
      }
      setTestError(e instanceof Error ? e.message : 'Failed to start tests')
    }
  }, [pluginInstalled])

  const handleScanFomods = useCallback(async () => {
    if (!pluginInstalled) return
    setScanError(null)
    setScanRunning(true)
    try {
      const result = await scanFomods()
      setScanRunning(Boolean(result.running))
      if (!result.running && result.success) {
        refreshSystemStatus(false)
        showActionChip('scan', 'Scan complete')
      }
    } catch (e) {
      setScanRunning(false)
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to start FOMOD scan due to unavailable backend', e)
        return
      }
      setScanError(e instanceof Error ? e.message : 'Failed to scan FOMODs')
    }
  }, [pluginInstalled, showActionChip])

  const pollPluginAction = useCallback(async (action: 'deploy' | 'purge') => {
    const poll = () => new Promise<void>((resolve) => {
      const id = setInterval(async () => {
        try {
          const status = await getPluginActionStatus()
          if (!status.running) {
            clearInterval(id)
            if (status.success) {
              refreshSystemStatus(false)
            } else {
              setPluginActionError(status.error || `${action} failed`)
            }
            resolve()
          }
        } catch (e) {
          if (isFetchUnavailableError(e)) {
            clearInterval(id)
            resolve()
          }
        }
      }, 1500)
    })
    await poll()
  }, [])

  const handleDeployPlugin = useCallback(async () => {
    setPluginActionError(null)
    setPluginActionRunning('deploy')
    try {
      await deployPlugin()
      await pollPluginAction('deploy')
    } catch (e) {
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to deploy plugin due to unavailable backend', e)
        return
      }
      setPluginActionError(e instanceof Error ? e.message : 'Failed to deploy plugin')
    } finally {
      setPluginActionRunning(null)
    }
  }, [pollPluginAction])

  const handlePurgePlugin = useCallback(async () => {
    setPluginActionError(null)
    setPluginActionRunning('purge')
    try {
      await purgePlugin()
      await pollPluginAction('purge')
    } catch (e) {
      if (isFetchUnavailableError(e)) {
        console.warn('[install] failed to purge plugin due to unavailable backend', e)
        return
      }
      setPluginActionError(e instanceof Error ? e.message : 'Failed to purge plugin')
    } finally {
      setPluginActionRunning(null)
    }
  }, [pollPluginAction])

  const handleFileSelect = async (files: FileList) => {
    if (!pluginInstalled) return
    const fileArray = Array.from(files)

    const archiveFiles: File[] = []
    const jsonFiles: File[] = []

    fileArray.forEach(file => {
      const ext = file.name.toLowerCase().split('.').pop()
      if (ext === 'json') {
        jsonFiles.push(file)
      } else {
        archiveFiles.push(file)
      }
    })

    const newJobs: InstallationJob[] = archiveFiles.map(file => ({
      id: `job-${Date.now()}-${Math.random()}`,
      fileName: file.name,
      status: 'pending' as const,
    }))

    setJobs(prev => [...prev, ...newJobs])

    for (let i = 0; i < newJobs.length; i++) {
      const archiveFile = archiveFiles[i]
      const archiveNameWithoutExt = archiveFile.name.replace(/\.[^/.]+$/, '')
      const matchingJson = jsonFiles.find(json =>
        json.name.toLowerCase() === `${archiveNameWithoutExt.toLowerCase()}.json`
      )
      await processJob(newJobs[i], archiveFile, matchingJson)
    }
  }

  const processJob = async (job: InstallationJob, file: File, jsonFile?: File) => {
    setJobs(prev => prev.map(j => j.id === job.id ? { ...j, status: 'uploading', uploadProgress: 0 } : j))

    try {
      const formData = new FormData()
      formData.append('file', file)

      if (jsonFile) {
        const jsonText = await jsonFile.text()
        formData.append('fomodJson', jsonText)
        formData.append('jsonFileName', jsonFile.name)
      }

      const result = await new Promise<Record<string, string>>((resolve, reject) => {
        const xhr = new XMLHttpRequest()

        xhr.upload.addEventListener('progress', (e) => {
          if (e.lengthComputable) {
            const progress = (e.loaded / e.total) * 100
            setJobs(prev => prev.map(j =>
              j.id === job.id ? { ...j, uploadProgress: progress } : j
            ))
          }
        })

        xhr.addEventListener('load', () => {
          if (xhr.status >= 200 && xhr.status < 300) {
            try {
              resolve(JSON.parse(xhr.responseText))
            } catch {
              reject(new Error('Invalid JSON response'))
            }
          } else {
            let errorMessage = `HTTP ${xhr.status}: ${xhr.statusText}`
            try {
              const errorData = JSON.parse(xhr.responseText)
              errorMessage = errorData.error || errorMessage
            } catch {
              errorMessage = xhr.responseText || errorMessage
            }
            reject(new Error(errorMessage))
          }
        })

        xhr.addEventListener('error', () => reject(new Error('Network error during upload')))
        xhr.addEventListener('abort', () => reject(new Error('Upload aborted')))

        xhr.open('POST', '/api/installation/upload')
        xhr.send(formData)
      })

      setJobs(prev => prev.map(j =>
        j.id === job.id
          ? { ...j, status: 'processing', uploadProgress: 100, processingStatus: 'Installing mod...' }
          : j
      ))

      // Poll for completion
      await new Promise<void>((resolve) => {
        const id = setInterval(async () => {
          try {
            const status = await getInstallStatus()
            if (!status.running) {
              clearInterval(id)
              if (status.success) {
                setJobs(prev => prev.map(j =>
                  j.id === job.id
                    ? { ...j, status: 'completed', modPath: status.modPath || result.modPath, processingStatus: 'Installation complete', uploadProgress: 100 }
                    : j
                ))
              } else {
                setJobs(prev => prev.map(j =>
                  j.id === job.id
                    ? { ...j, status: 'error', error: status.error || 'Installation failed' }
                    : j
                ))
              }
              resolve()
            }
          } catch {
            // Keep polling on transient errors
          }
        }, 1500)
      })
    } catch (error) {
      setJobs(prev => prev.map(j =>
        j.id === job.id
          ? { ...j, status: 'error', error: error instanceof Error ? error.message : 'Unknown error' }
          : j
      ))
    }
  }

  const readinessValue = readiness ?? 0
  const clampedReadiness = Math.max(0, Math.min(100, readinessValue))
  const ringDeg = Math.round((clampedReadiness / 100) * 360)
  const isCriticalReadiness = clampedReadiness === 0
  const isWarningReadiness = !isCriticalReadiness && clampedReadiness <= 33
  const isPerfectReadiness = clampedReadiness === 100
  const ringPrimary = isCriticalReadiness
    ? 'rgba(239, 68, 68, 0.86)'
    : isWarningReadiness
      ? 'rgba(245, 158, 11, 0.86)'
      : isPerfectReadiness
        ? 'rgba(52, 211, 153, 0.92)'
        : 'rgba(56, 189, 248, 0.82)'
  const ringSecondary = isCriticalReadiness
    ? 'rgba(248, 113, 113, 0.78)'
    : isWarningReadiness
      ? 'rgba(251, 191, 36, 0.78)'
      : isPerfectReadiness
        ? 'rgba(56, 189, 248, 0.88)'
        : 'rgba(52, 211, 153, 0.78)'
  const readinessTextClass = isCriticalReadiness
    ? 'text-error'
    : isWarningReadiness
      ? 'text-warning'
      : isPerfectReadiness
        ? 'text-on-surface'
        : 'text-on-surface'
  const readinessLabelClass = isCriticalReadiness
    ? 'text-error-light/80'
    : isWarningReadiness
      ? 'text-warning/80'
      : isPerfectReadiness
        ? 'text-on-surface/80'
        : 'text-on-surface-variant'
  const readinessGlowColor = isCriticalReadiness
    ? 'rgba(239, 68, 68, 0.25)'
    : isWarningReadiness
      ? 'rgba(245, 158, 11, 0.2)'
      : isPerfectReadiness
        ? 'rgba(52, 211, 153, 0.35)'
        : 'rgba(56, 189, 248, 0.2)'

  const ringBackground = isPerfectReadiness
    ? `conic-gradient(${ringPrimary} 0deg, ${ringSecondary} 120deg, rgba(192, 132, 252, 0.8) 240deg, ${ringPrimary} 360deg)`
    : `conic-gradient(${ringPrimary} 0deg, ${ringSecondary} ${ringDeg}deg, rgba(79, 99, 127, 0.22) ${ringDeg}deg 360deg)`

  const miniRingStyle = {
    background: ringBackground,
    WebkitMaskImage: 'radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2.5px))',
    maskImage: 'radial-gradient(farthest-side, transparent calc(100% - 2.5px), #000 calc(100% - 2.5px))',
  }

  return (
    <div className="animate-fade-in">
      <header className="page-header page-header-install mb-6">
        <h1 className="page-title text-2xl font-bold text-on-surface flex items-center gap-3">
          <span className="inline-flex w-6 shrink-0 items-center justify-center">
            <i className="fa-duotone fa-solid fa-cloud-arrow-up icon-gradient icon-gradient-nebula text-2xl" />
          </span>
          Install
          {!systemExpanded && status && (
            isPerfectReadiness ? (
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
                title={`Readiness: ${readinessValue}% - click to expand`}
              >
                <div className="absolute inset-0 rounded-full" style={miniRingStyle} />
                <div className="absolute inset-[2px] rounded-full bg-surface-container" />
              </button>
            )
          )}
        </h1>
        <p className="page-subtitle mt-1 text-sm ml-9">Upload and install mod archives</p>
      </header>

      {/* Status cards - hidden entirely when collapsed */}
      {systemExpanded && (
        <>
        {status && config ? (
          <div className="mb-8">
            {!status.configured && (
              <div className="mb-6 rounded-2xl border border-warning/15 bg-warning/5 p-5 flex items-start gap-4 animate-slide-up">
                <div className="w-9 h-9 rounded-xl bg-warning/12 flex items-center justify-center shrink-0">
                  <i className="fa-duotone fa-solid fa-triangle-exclamation text-warning" />
                </div>
                <div>
                  <p className="text-warning font-semibold text-sm">MO2 paths not configured</p>
                  <p className="text-on-surface-variant text-xs mt-0.5">
                    Go to <Link to="/settings" className="text-warning underline underline-offset-2 font-medium hover:text-warning-light">Settings</Link> to set your MO2 mods path.
                  </p>
                </div>
              </div>
            )}

            {(() => {
            const healthChecks = [
              {
                label: 'MO2 Plugin',
                icon: 'fa-duotone fa-solid fa-puzzle-piece',
                ok: status.pluginInstalled,
                goodValue: 'Installed',
                badValue: 'Missing',
                detail: status.pluginDeployPath || 'No deploy path',
                grad: 'icon-gradient-spring',
              },
              {
                label: 'Configuration',
                icon: 'fa-duotone fa-solid fa-plug-circle-check',
                ok: status.configured,
                goodValue: 'Active',
                badValue: 'Not set',
                detail: config.mo2ModsPath || 'No mods path set',
                grad: 'icon-gradient-atlas',
              },
              {
                label: 'FOMOD Output',
                icon: 'fa-duotone fa-solid fa-folder-open',
                ok: status.outputFolderExists,
                goodValue: 'Ready',
                badValue: 'Missing',
                detail: status.fomodOutputDir || 'N/A',
                grad: 'icon-gradient-ocean',
              },
            ]

            const readyCount = healthChecks.filter(h => h.ok).length

            return (
              <div className="flex items-start gap-4 mb-8">
                <div className="shrink-0 flex items-center gap-4">
                  <button
                    onClick={() => setSystemExpanded(false)}
                    className="readiness-glow relative w-[5.5rem] h-[5.5rem] rounded-full hover:scale-105 transition-transform"
                    style={{ '--readiness-glow-color': readinessGlowColor } as React.CSSProperties}
                    title="Click to collapse system section"
                  >
                    <div
                      className={`absolute inset-0 rounded-full ${ringPulse ? 'ring-pulse-once' : ''}`}
                      style={{
                        background: ringBackground,
                        WebkitMaskImage: 'radial-gradient(farthest-side, transparent calc(100% - 7px), #000 calc(100% - 7px))',
                        maskImage: 'radial-gradient(farthest-side, transparent calc(100% - 7px), #000 calc(100% - 7px))',
                      }}
                    />
                    <div className="absolute inset-[7px] rounded-full bg-surface-container border border-outline-variant/35" />
                    <div className="absolute inset-0 flex flex-col items-center justify-center">
                      <p className={`text-[0.9rem] font-bold tabular-nums leading-none ${readinessTextClass}`}>{readinessValue}%</p>
                      <p className={`text-[0.55rem] uppercase tracking-[0.11em] mt-0.5 ${readinessLabelClass}`}>Readiness</p>
                    </div>
                  </button>

                  <div>
                    <p className="ui-micro uppercase tracking-[0.11em]">System</p>
                    <p className="text-xs font-semibold text-on-surface mt-1">
                      {readyCount} of {healthChecks.length} checks
                    </p>
                    <p className="ui-micro mt-1">
                      {formatUpdatedAgo(statusUpdatedAt, nowMs)}
                    </p>
                  </div>
                </div>

                <div className="flex flex-wrap gap-x-6 gap-y-1 pt-0.5">
                  <div>
                    <p className="ui-label text-primary/85 mb-1.5">Health Checks</p>
                    <div className="flex flex-wrap gap-1.5">
                      {healthChecks.map(item => (
                        <div
                          key={item.label}
                          title={`${item.label}: ${item.ok ? item.goodValue : item.badValue} - ${item.detail}`}
                          className="rounded-full px-2.5 py-1 bg-transparent inline-flex items-center gap-1.5 hover-intent"
                        >
                          <span className="inline-flex items-center justify-center w-4">
                            <i className={`${item.icon} icon-gradient ${item.grad} icon-sm`} />
                          </span>
                          <span className="text-[0.68rem] font-semibold uppercase tracking-[0.08em] text-on-surface-variant">
                            {item.label}
                          </span>
                          <span
                            className={`w-2 h-2 rounded-full ${
                              item.ok
                                ? 'bg-success shadow-[0_0_8px_rgba(34,197,94,0.55)]'
                                : 'bg-warning shadow-[0_0_8px_rgba(245,158,11,0.55)]'
                            }`}
                            aria-hidden="true"
                          />
                        </div>
                      ))}
                    </div>
                  </div>

                  <div>
                    <p className="ui-label text-secondary/85 mb-1.5">Library Metrics</p>
                    <div className="flex flex-wrap gap-1.5">
                      <div
                        title={status.fomodOutputDir || 'N/A'}
                        className="rounded-full px-2.5 py-1 bg-transparent inline-flex items-center gap-1.5 hover-intent"
                      >
                        <i className="fa-duotone fa-solid fa-file-code icon-gradient icon-gradient-atlas icon-sm" />
                        <span className="text-[0.68rem] font-semibold uppercase tracking-[0.08em] text-on-surface-variant">JSONs</span>
                        <span className="text-[0.74rem] font-bold tabular-nums text-primary metric-value-glow-primary">{status.jsonCount}</span>
                      </div>

                      <div
                        title={config.mo2ModsPath || 'No mods path set'}
                        className="rounded-full px-2.5 py-1 bg-transparent inline-flex items-center gap-1.5 hover-intent"
                      >
                        <i className="fa-duotone fa-solid fa-boxes-stacked icon-gradient icon-gradient-forest icon-sm" />
                        <span className="text-[0.68rem] font-semibold uppercase tracking-[0.08em] text-on-surface-variant">Folders</span>
                        <span className="text-[0.74rem] font-bold tabular-nums text-secondary metric-value-glow-secondary">{status.modCount}</span>
                      </div>
                    </div>
                  </div>
                </div>
              </div>
            )
            })()}
          </div>
        ) : (
          <div className="mb-8 rounded-2xl border border-outline-variant/30 bg-surface-container/35 p-5">
            <div className="skeleton-line h-4 w-40 mb-4" />
            <div className="skeleton-line h-3 w-64 mb-2" />
            <div className="skeleton-line h-3 w-52" />
          </div>
        )}
        {/* Aurora divider */}
        <div className="aurora-divider mb-8" />
        </>
      )}

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
            <FileUpload onFileSelect={handleFileSelect} disabled={!pluginInstalled} />
          )}
        </div>

        <div>
          {systemUnavailable ? (
            <div className="flex flex-wrap gap-2">
              <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
              <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
              <div className="skeleton-line h-[1.875rem] w-20 rounded-lg" />
            </div>
          ) : (
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
          )}
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

